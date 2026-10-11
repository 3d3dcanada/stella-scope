//! WhatsApp alerts through CallMeBot. The phone number and the CallMeBot key live in
//! `<database>.notify.json` beside the recording (mode 600), set from the app, never in the
//! repository or the config JSON. Messages go out from the recorder itself, so they arrive with
//! every window closed: a print finishing, failing or being cancelled, a BLTouch alarm, a stepper
//! driver warning, Klipper stopping, and the printer dropping off in the middle of a print. Sends
//! are spaced by 20 s, duplicate text is held for 10 minutes, and one worker drains a bounded
//! queue so the recording never waits on the provider.
use crate::{config::Config, Result};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    process::{Command, Output, Stdio},
    sync::{atomic::{AtomicU64, Ordering}, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const GAP: Duration = Duration::from_secs(20);
const REPEAT: Duration = Duration::from_secs(600);
const CALLMEBOT_URL: &str = "https://api.callmebot.com/whatsapp.php";
static SAVE_ID: AtomicU64 = AtomicU64::new(0);

fn callmebot_command(url: &str, phone: &str, text: &str) -> Command {
    let mut command = crate::remote::quiet(Command::new("curl"));
    command.args(["-s", "-m", "30", "-G", url, "--data-urlencode"])
        .arg(format!("phone={phone}"))
        .arg("--data-urlencode")
        .arg(format!("text={text}"))
        .args(["--data-urlencode", "apikey@-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn callmebot_to(url: &str, phone: &str, key: &str, text: &str) -> std::io::Result<Output> {
    let mut child = callmebot_command(url, phone, text).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(error) = stdin.write_all(key.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    child.wait_with_output()
}

struct State {
    cfg_path: Option<String>,
    sent: VecDeque<(Instant, String)>,
    pending: VecDeque<String>,
    worker: bool,
    last_send: Option<Instant>,
    last_result: Value,
    printer: String,
    job: String,
}
fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State { cfg_path: None, sent: VecDeque::new(), pending: VecDeque::new(), worker: false, last_send: None, last_result: Value::Null, printer: String::new(), job: String::new() })
    })
}
fn path(cfg: &Config) -> String {
    format!("{}.notify.json", cfg.database)
}
fn load(cfg: &Config) -> Value {
    fs::read_to_string(path(cfg)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}
/// Remember where the settings are, so the collector can send without passing the config around.
pub fn init(cfg: &Config) {
    if let Ok(mut s) = state().lock() {
        s.cfg_path = Some(path(cfg));
    }
}

/// `GET /api/notify`: what is set up, never the key itself.
pub fn settings(cfg: &Config) -> Value {
    let v = load(cfg);
    let (last, pending) = state().lock().map(|s| (s.last_result.clone(), s.pending.len())).unwrap_or((Value::Null, 0));
    json!({"phone":v["phone"],"enabled":v["enabled"].as_bool().unwrap_or(false),
        "has_key":v["apikey"].as_str().is_some_and(|k| !k.is_empty()),"last":last,"pending":pending,
        "bot_number":"+34 694 23 41 84","activation":"I allow callmebot to send me messages"})
}

/// `POST /api/notify`: {phone, apikey?, enabled}. An absent or empty key keeps the stored one.
pub fn save(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "send {\"phone\":\"+1...\",\"apikey\":\"...\",\"enabled\":true}")?;
    let old = load(cfg);
    let phone: String = v["phone"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    if phone.len() < 8 || phone.len() > 16 || !phone.starts_with('+') {
        return Err("the phone number needs its country code, like +15065551234".into());
    }
    let apikey = match v["apikey"].as_str().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) if k.len() <= 40 && k.chars().all(|c| c.is_ascii_alphanumeric()) => k.to_owned(),
        Some(_) => return Err("the CallMeBot key is letters and digits only".into()),
        None => old["apikey"].as_str().unwrap_or("").to_owned(),
    };
    let enabled = v["enabled"].as_bool().unwrap_or(true) && !apikey.is_empty();
    let stored = json!({"phone":phone,"apikey":apikey,"enabled":enabled});
    let file = path(cfg);
    let temp = format!("{file}.tmp-{}-{}", std::process::id(), SAVE_ID.fetch_add(1, Ordering::Relaxed));
    let mut options = OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write = (|| -> std::io::Result<()> {
        let mut f = options.open(&temp)?;
        f.write_all(stored.to_string().as_bytes())?;
        f.sync_all()?;
        fs::rename(&temp, &file)
    })();
    if let Err(e) = write { let _ = fs::remove_file(&temp); return Err(format!("cannot store the WhatsApp settings: {e}")); }
    Ok(settings(cfg))
}

fn enqueue(s: &mut State, text: &str, now: Instant) -> bool {
    let message: String = text.chars().take(900).collect();
    s.sent.retain(|(t, _)| now.duration_since(*t) < REPEAT);
    if s.sent.iter().any(|(_, m)| m == &message) {
        return false;
    }
    if s.pending.len() >= 32 {
        s.last_result = json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp queue full; alert retained in the recorder only"});
        return false;
    }
    s.sent.push_back((now, message.clone()));
    s.pending.push_back(message);
    true
}

fn drain() {
    loop {
        let (file, text, delay) = {
            let Ok(mut s) = state().lock() else { return };
            let Some(text) = s.pending.pop_front() else { s.worker = false; return };
            let delay = s.last_send.map(|at| GAP.saturating_sub(at.elapsed())).unwrap_or_default();
            (s.cfg_path.clone(), text, delay)
        };
        thread::sleep(delay);
        let v: Value = file.and_then(|p| fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let result = if let (Some(phone), Some(key)) = (v["phone"].as_str(), v["apikey"].as_str()) {
            if v["enabled"].as_bool() == Some(true) && !key.is_empty() {
                let out = callmebot_to(CALLMEBOT_URL, phone, key, &text);
                match out {
                    Ok(o) => {
                        let body = String::from_utf8_lossy(&o.stdout).to_string();
                        let plain: String = body.split('<').map(|p| p.split('>').last().unwrap_or("")).collect::<Vec<_>>().join(" ");
                        let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ").replace(key, "[redacted]");
                        let ok = o.status.success() && plain.to_lowercase().contains("queued");
                        json!({"ok":ok,"at":crate::model::now(),"reply":plain.chars().take(300).collect::<String>()})
                    }
                    Err(e) => json!({"ok":false,"at":crate::model::now(),"reply":format!("curl could not run: {e}")}),
                }
            } else { json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp disabled before queued alert was sent"}) }
        } else { json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp settings unavailable before queued alert was sent"}) };
        if let Ok(mut s) = state().lock() { s.last_send = Some(Instant::now()); s.last_result = result; }
    }
}

/// Queue one WhatsApp message. Returns false when off, repeated, or the bounded queue is full.
pub fn send(text: &str) -> bool {
    let Ok(mut s) = state().lock() else { return false };
    let Some(file) = s.cfg_path.clone() else { return false };
    let v: Value = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if v["phone"].as_str().is_none() || v["apikey"].as_str().is_none_or(str::is_empty) || !v["enabled"].as_bool().unwrap_or(false) {
        return false;
    }
    if !enqueue(&mut s, text, Instant::now()) { return false; }
    if !s.worker {
        s.worker = true;
        if let Err(e) = thread::Builder::new().name("stella-whatsapp".into()).spawn(drain) {
            s.worker = false;
            s.pending.pop_back();
            s.sent.pop_back();
            s.last_result = json!({"ok":false,"at":crate::model::now(),"reply":format!("WhatsApp worker could not start: {e}")});
            return false;
        }
    }
    true
}

/// The collector's view of the printer each cycle: says so when Klipper stops, and when the
/// printer drops off during a print. Only changes are announced.
pub fn on_state(printer: &str, job: Option<&str>, message: Option<&str>) {
    let (was_printer, was_job) = {
        let Ok(mut s) = state().lock() else { return };
        let before = (s.printer.clone(), s.job.clone());
        s.printer = printer.to_owned();
        if let Some(j) = job {
            s.job = j.to_owned();
        }
        before
    };
    if was_printer.is_empty() || was_printer == printer {
        return;
    }
    if printer == "not_ready" {
        send(&format!("Stella: Klipper has stopped. {}", message.unwrap_or("Open Stella Scope for the reason.")));
    } else if matches!(printer, "off" | "unreachable") && matches!(was_job.as_str(), "printing" | "paused") {
        send("Stella: the printer stopped answering in the middle of a print.");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn distinct_alerts_queue_while_duplicates_are_suppressed() {
        let mut state = super::State { cfg_path: None, sent: std::collections::VecDeque::new(), pending: std::collections::VecDeque::new(), worker: false, last_send: None, last_result: serde_json::Value::Null, printer: String::new(), job: String::new() };
        let now = std::time::Instant::now();
        assert!(super::enqueue(&mut state, "driver warning", now));
        assert!(super::enqueue(&mut state, "Klipper stopped", now));
        assert!(super::enqueue(&mut state, "print failed", now));
        assert!(!super::enqueue(&mut state, "driver warning", now));
        assert_eq!(state.pending.into_iter().collect::<Vec<_>>(), ["driver warning", "Klipper stopped", "print failed"]);
    }
    #[test]
    fn settings_keep_the_key_to_themselves_and_check_the_number() {
        let mut cfg = crate::config::Config::default();
        cfg.database = format!("/tmp/stella-notify-test-{}", std::process::id());
        assert!(super::save(&cfg, br#"{"phone":"5065550123","apikey":"123"}"#).is_err(), "country code needed");
        let v = super::save(&cfg, br#"{"phone":"+1 506 555 0123","apikey":"123456","enabled":true}"#).unwrap();
        assert_eq!(v["phone"], "+15065550123");
        assert_eq!(v["has_key"], true);
        assert!(!v.to_string().contains("123456"), "the key never leaves the store");
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(super::path(&cfg)).unwrap().permissions().mode() & 0o077, 0); }
        let v = super::save(&cfg, br#"{"phone":"+15065550123","enabled":false}"#).unwrap();
        assert_eq!(v["has_key"], true, "an empty key keeps the stored one");
        assert_eq!(v["enabled"], false);
        assert!(super::save(&cfg, br#"{"phone":"+15065550123","apikey":"bad key!"}"#).is_err());
        let _ = std::fs::remove_file(super::path(&cfg));
    }

    #[test]
    fn callmebot_key_stays_out_of_process_arguments() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/whatsapp.php", listener.local_addr().unwrap());
        let command = super::callmebot_command(&url, "+15065551234", "test message");
        let args = command.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ");
        assert!(args.contains("apikey@-"));
        assert!(!args.contains("apikey="));
        assert!(!args.contains("secret123"));
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline =>
                        std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(error) => panic!("fake CallMeBot did not receive curl: {error}"),
                }
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nqueued").unwrap();
            request
        });
        let result = super::callmebot_to(&url, "+15065551234", "secret123", "test message").unwrap();
        assert!(result.status.success());
        let request = server.join().unwrap();
        assert!(request.contains("apikey=secret123"));
        assert!(request.contains("text=test+message") || request.contains("text=test%20message"), "fake provider received: {request}");
    }
}
