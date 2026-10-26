//! Alerts: WhatsApp through CallMeBot (2026-09-25: "Use CallMeBot") and, off by default, ntfy.
//!
//! The settings live in `<database>.notify.json` beside the recording (mode 600), set from the app,
//! never in the repository or the config JSON: the phone number and CallMeBot key, which events to
//! tell about, and the address of an ntfy topic. Messages go out from the recorder itself, so they
//! arrive with every window closed: a print finishing, failing or being cancelled, a plate that needs
//! clearing, a plate check that found something, a printer dropping off in the middle of a print, the
//! failure watch, a BLTouch alarm or a stepper driver warning, the queue running empty. WhatsApp sends
//! are spaced by 20 s and one worker drains a bounded queue so the recording never waits on the
//! provider; duplicate text is held for 10 minutes on every channel.
//!
//! ntfy (farm board WO-11) is a plain HTTP POST to a topic the person runs themselves (self-hosted, on
//! their network): `http://` only, no new crate, off until an address is set. Each message carries a
//! `Click` address, `stella://p/<printer id>`, so tapping it on the phone opens that printer in the app.
use crate::{config::Config, Result};
use serde_json::{json, Map, Value};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

const GAP: Duration = Duration::from_secs(20);
const REPEAT: Duration = Duration::from_secs(600);
const CALLMEBOT_URL: &str = "https://api.callmebot.com/whatsapp.php";
static SAVE_ID: AtomicU64 = AtomicU64::new(0);

/// What an alert can be about, in the words the settings page shows. All are on until the person turns one off.
pub const KINDS: [(&str, &str); 8] = [
    ("done", "A print finishes, fails or is cancelled"),
    ("plate_clear", "A plate needs clearing before the next file"),
    (
        "plate_failed",
        "The plate check finds something on the plate",
    ),
    (
        "offline",
        "A printer stops answering in the middle of a print",
    ),
    ("failure_watch", "The failure watch sees a problem"),
    (
        "attention",
        "A printer needs attention: an alarm, a motor driver warning, a printer that stopped",
    ),
    ("queue_empty", "The queue runs empty"),
    ("spool_low", "A spool runs low"),
];

/// A note for the alarm feed and the in-app pop-ups only, never a phone channel: a printer switched
/// off at night is worth a pop-up, not a WhatsApp message.
pub fn note(kind: &str, printer: &str, name: &str, text: &str) {
    remember(kind, printer, name, text, &[], &Value::Null);
}

fn callmebot_command(url: &str, phone: &str, text: &str) -> Command {
    let mut command = crate::remote::quiet(Command::new("curl"));
    command
        .args(["-s", "-m", "30", "-G", url, "--data-urlencode"])
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
    /// The recording's path: the link status file beside it says which id a phone dials.
    database: Option<String>,
    sent: VecDeque<(Instant, String)>,
    ntfy_sent: VecDeque<(Instant, String)>,
    pending: VecDeque<String>,
    worker: bool,
    last_send: Option<Instant>,
    last_result: Value,
    last_ntfy: Value,
    ntfy_inflight: usize,
    /// The newest alerts and where each went, so the settings page can show what was said.
    recent: VecDeque<Value>,
    /// The farm-wide alarm feed (2026-10-13): every alert, whether or not a channel took it, newest
    /// last, numbered so a page asks only for what it has not seen. Kept beside the settings file.
    feed: VecDeque<Value>,
    seq: u64,
    hooks_sent: VecDeque<(Instant, String)>,
    hooks_inflight: usize,
    last_hooks: Value,
    /// Per printer id: (printer state, job state), so one printer's change is announced once and
    /// another's reports never read as a change of the first (farm board WO-03).
    seen: std::collections::HashMap<String, (String, String)>,
}
impl State {
    fn new() -> Self {
        State {
            cfg_path: None,
            database: None,
            sent: VecDeque::new(),
            ntfy_sent: VecDeque::new(),
            pending: VecDeque::new(),
            worker: false,
            last_send: None,
            last_result: Value::Null,
            last_ntfy: Value::Null,
            ntfy_inflight: 0,
            recent: VecDeque::new(),
            feed: VecDeque::new(),
            seq: 0,
            hooks_sent: VecDeque::new(),
            hooks_inflight: 0,
            last_hooks: Value::Null,
            seen: std::collections::HashMap::new(),
        }
    }
}
fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::new()))
}
fn path(cfg: &Config) -> String {
    format!("{}.notify.json", cfg.database)
}
fn load(cfg: &Config) -> Value {
    fs::read_to_string(path(cfg))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}
/// Remember where the settings are, so the collector can send without passing the config around.
pub fn init(cfg: &Config) {
    if let Ok(mut s) = state().lock() {
        s.cfg_path = Some(path(cfg));
        s.database = Some(cfg.database.clone());
        if s.feed.is_empty() {
            let kept: Vec<Value> = fs::read_to_string(feed_path(&path(cfg)))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            s.seq = kept
                .iter()
                .filter_map(|e| e["seq"].as_u64())
                .max()
                .unwrap_or(0);
            s.feed = kept.into_iter().collect();
        }
    }
}

const FEED_KEEP: usize = 500;
fn feed_path(settings: &str) -> String {
    format!("{settings}.feed.json")
}

/// How loud an alert is, for its colour and whether its pop-up stays until it is closed.
pub fn severity(kind: &str, text: &str) -> &'static str {
    let t = text.to_ascii_lowercase();
    match kind {
        "failure_watch" | "attention" | "plate_failed" => "alarm",
        "offline" => "warn",
        "done" if t.contains("fail") || t.contains("error") => "alarm",
        "done" if t.contains("cancel") => "warn",
        "done" => "ok",
        "spool_low" => "warn",
        "online" => "ok",
        _ => "info",
    }
}

/// `GET /api/alerts/feed?after=N&printer=ID&limit=M`: the farm-wide alarm feed, oldest first after
/// `after`, or the newest `limit` when `after` is 0.
pub fn feed(after: u64, printer: Option<&str>, limit: usize) -> Value {
    let (rows, last) = state()
        .lock()
        .map(|s| {
            let rows: Vec<Value> = s
                .feed
                .iter()
                .filter(|e| e["seq"].as_u64().unwrap_or(0) > after)
                .filter(|e| printer.is_none_or(|p| e["printer"] == p))
                .cloned()
                .collect();
            (rows, s.seq)
        })
        .unwrap_or_default();
    let limit = limit.clamp(1, FEED_KEEP);
    let skip = rows.len().saturating_sub(limit);
    json!({"events":rows[skip..],"last_seq":last})
}

/// Discord and Slack incoming webhooks: https only, on their own hosts, nothing else accepted, so a
/// pasted address cannot turn the service into a way to post anywhere.
pub fn webhook_address(service: &str, text: &str) -> Result<String> {
    let t = text.trim();
    let prefixes: &[&str] = match service {
        "discord" => &[
            "https://discord.com/api/webhooks/",
            "https://discordapp.com/api/webhooks/",
            "https://ptb.discord.com/api/webhooks/",
            "https://canary.discord.com/api/webhooks/",
        ],
        "slack" => &[
            "https://hooks.slack.com/services/",
            "https://hooks.slack.com/triggers/",
            "https://hooks.slack.com/workflows/",
        ],
        _ => return Err(format!("unknown service {service}: discord or slack")),
    };
    let rest = prefixes
        .iter()
        .find_map(|p| t.strip_prefix(p))
        .ok_or_else(|| format!("a {service} webhook address starts {}", prefixes[0]))?;
    if rest.is_empty()
        || rest.len() > 200
        || !rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
    {
        return Err(format!(
            "that {service} webhook address has characters a webhook never has"
        ));
    }
    Ok(t.to_owned())
}

/// The JSON each service expects: one line, the printer's name first.
pub fn webhook_body(service: &str, name: &str, text: &str) -> Value {
    let who = if name.is_empty() {
        "Stella Scope"
    } else {
        name
    };
    match service {
        "slack" => json!({"text":format!("*{who}*: {text}")}),
        _ => json!({"content":format!("**{who}**: {text}"),"allowed_mentions":{"parse":[]}}),
    }
}

/// `POST /api/notify/webhook`: {service, url, enabled}. An empty address turns it off and forgets it.
pub fn save_webhook(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|_| "send {\"service\":\"discord\",\"url\":\"https://...\",\"enabled\":true}")?;
    let service = v["service"].as_str().unwrap_or("");
    if !matches!(service, "discord" | "slack") {
        return Err("service is discord or slack".into());
    }
    let mut stored = load(cfg);
    if !stored.is_object() {
        stored = json!({});
    }
    let url = v["url"].as_str().unwrap_or("").trim();
    let keep = stored["webhooks"][service]["url"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let entry = if url.is_empty() && v["enabled"].as_bool() == Some(true) && !keep.is_empty() {
        json!({"url":keep,"enabled":true})
    } else if url.is_empty() {
        json!({"url":"","enabled":false})
    } else {
        json!({"url":webhook_address(service, url)?,"enabled":v["enabled"].as_bool().unwrap_or(true)})
    };
    if !stored["webhooks"].is_object() {
        stored["webhooks"] = json!({});
    }
    stored["webhooks"][service] = entry;
    write_settings(cfg, &stored)?;
    Ok(settings(cfg))
}

fn webhook_post(url: &str, body: &Value) -> std::result::Result<u16, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    agent
        .post(url)
        .header("Content-Type", "application/json")
        .send(body.to_string().as_bytes())
        .map(|r| r.status().as_u16())
        .map_err(|e| e.to_string())
}

fn events_of(v: &Value) -> Map<String, Value> {
    KINDS
        .iter()
        .map(|(k, _)| {
            (
                (*k).to_owned(),
                json!(v["events"][*k].as_bool().unwrap_or(true)),
            )
        })
        .collect()
}

/// `GET /api/notify`: what is set up, never the key itself.
pub fn settings(cfg: &Config) -> Value {
    let v = load(cfg);
    let (last, pending, last_ntfy, recent) = state()
        .lock()
        .map(|s| {
            (
                s.last_result.clone(),
                s.pending.len(),
                s.last_ntfy.clone(),
                s.recent.iter().cloned().collect::<Vec<_>>(),
            )
        })
        .unwrap_or((Value::Null, 0, Value::Null, Vec::new()));
    let on = events_of(&v);
    json!({"phone":v["phone"],"enabled":v["enabled"].as_bool().unwrap_or(false),
        "has_key":v["apikey"].as_str().is_some_and(|k| !k.is_empty()),"last":last,"pending":pending,
        "bot_number":"+34 694 23 41 84","activation":"I allow callmebot to send me messages",
        "events":KINDS.iter().map(|(k, label)| json!({"kind":k,"label":label,"on":on[*k]})).collect::<Vec<_>>(),
        "webhooks":(["discord","slack"]).iter().map(|k| {
            let u = v["webhooks"][*k]["url"].as_str().unwrap_or("");
            // the address is a secret (it carries the webhook's token): only that one is set
            ((*k).to_owned(), json!({"set":!u.is_empty(),"enabled":!u.is_empty()&&v["webhooks"][*k]["enabled"].as_bool().unwrap_or(false)}))
        }).collect::<Map<String, Value>>(),
        "webhooks_last":state().lock().map(|s| s.last_hooks.clone()).unwrap_or(Value::Null),
        "ntfy":{"url":v["ntfy"]["url"],"enabled":v["ntfy"]["enabled"].as_bool().unwrap_or(false)&&v["ntfy"]["url"].as_str().is_some_and(|u| !u.is_empty()),"last":last_ntfy},
        "recent":recent})
}

/// Write the settings file whole: mode 600, a temporary file renamed over the old one.
fn write_settings(cfg: &Config, stored: &Value) -> Result<()> {
    let file = path(cfg);
    let temp = format!(
        "{file}.tmp-{}-{}",
        std::process::id(),
        SAVE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
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
    if let Err(e) = write {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot store the alert settings: {e}"));
    }
    Ok(())
}

/// `POST /api/notify`: {phone, apikey?, enabled}. An absent or empty key keeps the stored one. The
/// events and the ntfy address already saved are kept.
pub fn save(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|_| "send {\"phone\":\"+1...\",\"apikey\":\"...\",\"enabled\":true}")?;
    let old = load(cfg);
    let phone: String = v["phone"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if phone.len() < 8 || phone.len() > 16 || !phone.starts_with('+') {
        return Err("the phone number needs its country code, like +15065551234".into());
    }
    let apikey = match v["apikey"]
        .as_str()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(k) if k.len() <= 40 && k.chars().all(|c| c.is_ascii_alphanumeric()) => k.to_owned(),
        Some(_) => return Err("the CallMeBot key is letters and digits only".into()),
        None => old["apikey"].as_str().unwrap_or("").to_owned(),
    };
    let enabled = v["enabled"].as_bool().unwrap_or(true) && !apikey.is_empty();
    let mut stored = if old.is_object() { old } else { json!({}) };
    stored["phone"] = json!(phone);
    stored["apikey"] = json!(apikey);
    stored["enabled"] = json!(enabled);
    write_settings(cfg, &stored)?;
    Ok(settings(cfg))
}

/// `POST /api/notify/events`: {events: {kind: bool}}. Only the kinds named change.
pub fn save_events(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| "send {\"events\":{\"done\":true,...}}")?;
    let given = v["events"]
        .as_object()
        .ok_or("send {\"events\":{\"done\":true,...}}")?;
    for k in given.keys() {
        if !KINDS.iter().any(|(kind, _)| kind == k) {
            return Err(format!("{k} is not something Stella Scope can tell about"));
        }
    }
    let mut stored = load(cfg);
    if !stored.is_object() {
        stored = json!({});
    }
    let mut events = events_of(&stored);
    for (k, on) in given {
        events.insert(
            k.clone(),
            json!(on.as_bool().ok_or("each event is true or false")?),
        );
    }
    stored["events"] = Value::Object(events);
    write_settings(cfg, &stored)?;
    Ok(settings(cfg))
}

/// An ntfy topic address: `http://host[:port]/topic` for a server on the local network, or
/// `https://host/topic` for ntfy.sh or a self-hosted server with TLS. ureq (rustls) posts to either.
fn ntfy_address(text: &str) -> Result<String> {
    let t = text.trim();
    let (scheme, rest) = t
        .strip_prefix("https://")
        .map(|r| ("https", r))
        .or_else(|| t.strip_prefix("http://").map(|r| ("http", r)))
        .ok_or("the ntfy address starts http:// or https://; a local server uses http, ntfy.sh and a self-hosted server may use https")?;
    let (host, topic) = rest
        .split_once('/')
        .ok_or("the ntfy address needs a topic, like https://ntfy.sh/stella or http://192.168.1.10:8080/stella")?;
    let host_ok = !host.is_empty()
        && host.len() <= 100
        && host.split(':').count() <= 2
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        && host
            .split(':')
            .nth(1)
            .is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !host_ok {
        return Err("the ntfy address has a host that cannot be right (letters, digits, dots and dashes, and an optional :port)".into());
    }
    if topic.is_empty()
        || topic.len() > 64
        || !topic
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err("the ntfy topic is letters, digits, dashes and underscores, up to 64".into());
    }
    Ok(format!("{scheme}://{host}/{topic}"))
}

/// `POST /api/notify/ntfy`: {url, enabled}. An empty address turns it off and forgets it.
pub fn save_ntfy(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|_| "send {\"url\":\"http://host:8080/topic\",\"enabled\":true}")?;
    let mut stored = load(cfg);
    if !stored.is_object() {
        stored = json!({});
    }
    let url = v["url"].as_str().unwrap_or("").trim();
    stored["ntfy"] = if url.is_empty() {
        json!({"url":"","enabled":false})
    } else {
        json!({"url":ntfy_address(url)?,"enabled":v["enabled"].as_bool().unwrap_or(true)})
    };
    write_settings(cfg, &stored)?;
    Ok(settings(cfg))
}

fn enqueue(s: &mut State, text: &str, now: Instant) -> bool {
    let message: String = text.chars().take(900).collect();
    s.sent.retain(|(t, _)| now.duration_since(*t) < REPEAT);
    if s.sent.iter().any(|(_, m)| m == &message) {
        return false;
    }
    if s.pending.len() >= 32 {
        s.last_result = json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp queue full; alert kept in Alerts only"});
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
            let Some(text) = s.pending.pop_front() else {
                s.worker = false;
                return;
            };
            let delay = s
                .last_send
                .map(|at| GAP.saturating_sub(at.elapsed()))
                .unwrap_or_default();
            (s.cfg_path.clone(), text, delay)
        };
        thread::sleep(delay);
        let v: Value = file
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let result = if let (Some(phone), Some(key)) = (v["phone"].as_str(), v["apikey"].as_str()) {
            if v["enabled"].as_bool() == Some(true) && !key.is_empty() {
                let out = callmebot_to(CALLMEBOT_URL, phone, key, &text);
                match out {
                    Ok(o) => {
                        let body = String::from_utf8_lossy(&o.stdout).to_string();
                        let plain: String = body
                            .split('<')
                            .map(|p| p.split('>').last().unwrap_or(""))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let plain = plain
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .replace(key, "[redacted]");
                        let ok = o.status.success() && plain.to_lowercase().contains("queued");
                        json!({"ok":ok,"at":crate::model::now(),"reply":plain.chars().take(300).collect::<String>()})
                    }
                    Err(e) => {
                        json!({"ok":false,"at":crate::model::now(),"reply":format!("curl could not run: {e}")})
                    }
                }
            } else {
                json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp disabled before queued alert was sent"})
            }
        } else {
            json!({"ok":false,"at":crate::model::now(),"reply":"WhatsApp settings unavailable before queued alert was sent"})
        };
        if let Ok(mut s) = state().lock() {
            s.last_send = Some(Instant::now());
            s.last_result = result;
        }
    }
}

/// Queue one WhatsApp message. Returns false when off, repeated, or the bounded queue is full.
fn send_whatsapp(text: &str) -> bool {
    let Ok(mut s) = state().lock() else {
        return false;
    };
    let Some(file) = s.cfg_path.clone() else {
        return false;
    };
    let v: Value = fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if v["phone"].as_str().is_none()
        || v["apikey"].as_str().is_none_or(str::is_empty)
        || !v["enabled"].as_bool().unwrap_or(false)
    {
        return false;
    }
    if !enqueue(&mut s, text, Instant::now()) {
        return false;
    }
    if !s.worker {
        s.worker = true;
        if let Err(e) = thread::Builder::new()
            .name("stella-whatsapp".into())
            .spawn(drain)
        {
            s.worker = false;
            s.pending.pop_back();
            s.sent.pop_back();
            s.last_result = json!({"ok":false,"at":crate::model::now(),"reply":format!("WhatsApp worker could not start: {e}")});
            return false;
        }
    }
    true
}

/// The text made safe for an HTTP header value: ASCII only, no control characters.
fn header_safe(t: &str) -> String {
    t.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .take(80)
        .collect()
}

/// POST one message to an ntfy topic. `Title` names the printer, `Click` opens it in the app.
fn ntfy_post(url: &str, title: &str, click: Option<&str>, urgent: bool, text: &str) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    let mut req = agent
        .post(url)
        .header("Title", header_safe(title))
        .header("Tags", "printer")
        .header("Priority", if urgent { "high" } else { "default" });
    if let Some(c) = click {
        req = req.header("Click", c);
    }
    match req.send(text.as_bytes()) {
        Ok(r) if (200..300).contains(&r.status().as_u16()) => {
            json!({"ok":true,"at":crate::model::now(),"reply":format!("ntfy took it ({})", r.status().as_u16())})
        }
        Ok(r) => {
            json!({"ok":false,"at":crate::model::now(),"reply":format!("ntfy answered {}", r.status().as_u16())})
        }
        Err(e) => {
            json!({"ok":false,"at":crate::model::now(),"reply":format!("ntfy could not be reached: {e}")})
        }
    }
}

/// The address a tapped alert opens in the app: that printer, on this computer's link.
fn click_address(database: Option<&str>, printer: &str) -> Option<String> {
    if printer.is_empty()
        || !printer
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    let link = database
        .and_then(|d| fs::read_to_string(format!("{d}.link.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["id"].as_str().map(str::to_owned))
        .filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()));
    Some(match link {
        Some(id) => format!("stella://p/{printer}?link={id}"),
        None => format!("stella://p/{printer}"),
    })
}

fn remember(kind: &str, printer: &str, name: &str, text: &str, sent: &[&str], detail: &Value) {
    let Ok(mut s) = state().lock() else { return };
    let at = crate::model::now();
    s.recent.push_front(
        json!({"at":at,"kind":kind,"printer":printer,"name":name,"text":text,"sent":sent}),
    );
    s.recent.truncate(20);
    s.seq += 1;
    let seq = s.seq;
    let mut row = json!({"seq":seq,"at":at,"kind":kind,"severity":severity(kind, text),"printer":printer,"name":name,"text":text,"sent":sent});
    // UI board U07 (2026-10-20): what the page needs to draw the alarm (the failure watch's score and where it saw it), beside the words
    if !detail.is_null() {
        row["detail"] = detail.clone();
    }
    s.feed.push_back(row);
    while s.feed.len() > FEED_KEEP {
        s.feed.pop_front();
    }
    if let Some(settings) = s.cfg_path.clone() {
        let rows: Vec<&Value> = s.feed.iter().collect();
        let file = feed_path(&settings);
        let temp = format!("{file}.tmp-{}", std::process::id());
        if fs::write(&temp, serde_json::to_vec(&rows).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&temp, &file);
        }
    }
}

/// One alert, to every channel that is on, unless the person turned this kind off. `printer` is the
/// printer's id and `name` how it is called ("" for the service as a whole). Returns whether any channel took it.
pub fn event(kind: &str, printer: &str, name: &str, text: &str) -> bool {
    event_with(kind, printer, name, text, Value::Null)
}

/// `event` with `detail`: a small object kept beside the words in the alarm feed (the failure watch's verdict, its score and the boxes it drew).
pub fn event_with(kind: &str, printer: &str, name: &str, text: &str, detail: Value) -> bool {
    let (file, database) = match state().lock() {
        Ok(s) => (s.cfg_path.clone(), s.database.clone()),
        Err(_) => return false,
    };
    let v: Value = file
        .and_then(|f| fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if kind != "test" && !events_of(&v)[kind].as_bool().unwrap_or(true) {
        remember(kind, printer, name, text, &[], &detail);
        return false;
    }
    let mut sent: Vec<&str> = Vec::new();
    if send_whatsapp(text) {
        sent.push("whatsapp");
    }
    if let Some(url) = v["ntfy"]["url"]
        .as_str()
        .filter(|u| !u.is_empty() && v["ntfy"]["enabled"].as_bool().unwrap_or(false))
    {
        let now = Instant::now();
        let fresh = state().lock().map(|mut s| {
            s.ntfy_sent.retain(|(t, _)| now.duration_since(*t) < REPEAT);
            if s.ntfy_sent.iter().any(|(_, m)| m == text) || s.ntfy_inflight >= 4 {
                false
            } else {
                s.ntfy_sent.push_back((now, text.to_owned()));
                s.ntfy_inflight += 1;
                true
            }
        });
        if fresh.unwrap_or(false) {
            let (url, text_owned) = (url.to_owned(), text.to_owned());
            let title = if name.is_empty() {
                "Stella Scope".to_owned()
            } else {
                name.to_owned()
            };
            let click = click_address(database.as_deref(), printer);
            let urgent = matches!(kind, "failure_watch" | "attention" | "offline");
            let spawned = thread::Builder::new()
                .name("stella-ntfy".into())
                .spawn(move || {
                    let result = ntfy_post(&url, &title, click.as_deref(), urgent, &text_owned);
                    if let Ok(mut s) = state().lock() {
                        s.ntfy_inflight = s.ntfy_inflight.saturating_sub(1);
                        s.last_ntfy = result;
                    }
                });
            if spawned.is_ok() {
                sent.push("ntfy");
            } else if let Ok(mut s) = state().lock() {
                s.ntfy_inflight = s.ntfy_inflight.saturating_sub(1);
            }
        }
    }
    for service in ["discord", "slack"] {
        let Some(url) = v["webhooks"][service]["url"].as_str().filter(|u| {
            !u.is_empty() && v["webhooks"][service]["enabled"].as_bool().unwrap_or(false)
        }) else {
            continue;
        };
        let now = Instant::now();
        let key = format!("{service}|{text}");
        let fresh = state().lock().map(|mut s| {
            s.hooks_sent
                .retain(|(t, _)| now.duration_since(*t) < REPEAT);
            if s.hooks_sent.iter().any(|(_, m)| m == &key) || s.hooks_inflight >= 4 {
                false
            } else {
                s.hooks_sent.push_back((now, key.clone()));
                s.hooks_inflight += 1;
                true
            }
        });
        if !fresh.unwrap_or(false) {
            continue;
        }
        let (url, body) = (url.to_owned(), webhook_body(service, name, text));
        let spawned = thread::Builder::new().name("stella-webhook".into()).spawn(move || {
            let result = match webhook_post(&url, &body) {
                Ok(c) if (200..300).contains(&c) => json!({"ok":true,"service":service,"at":crate::model::now(),"reply":format!("{service} took it ({c})")}),
                Ok(c) => json!({"ok":false,"service":service,"at":crate::model::now(),"reply":format!("{service} answered {c}")}),
                Err(e) => json!({"ok":false,"service":service,"at":crate::model::now(),"reply":format!("{service} could not be reached: {e}")}),
            };
            if let Ok(mut s) = state().lock() {
                s.hooks_inflight = s.hooks_inflight.saturating_sub(1);
                s.last_hooks = result;
            }
        });
        if spawned.is_ok() {
            sent.push(service);
        } else if let Ok(mut s) = state().lock() {
            s.hooks_inflight = s.hooks_inflight.saturating_sub(1);
        }
    }
    remember(kind, printer, name, text, &sent, &detail);
    !sent.is_empty()
}

/// What a printer that stopped is announced as. Klipper's own word only for a Klipper printer (polish WO-06,
/// 2026-10-18, README rule 8): a demo, a Prusa or a FlashForge "has stopped".
fn stopped_text(name: &str, message: Option<&str>, klipper: bool) -> String {
    format!(
        "{name}: {} has stopped. {}",
        if klipper { "Klipper" } else { "the printer" },
        message.unwrap_or("Open Stella Scope for the reason.")
    )
}

/// The collector's view of one printer each cycle: says so when Klipper stops, and when the printer
/// drops off during a print. Only changes are announced, per printer, and the message names it.
pub fn on_state(
    id: &str,
    name: &str,
    printer: &str,
    job: Option<&str>,
    message: Option<&str>,
    klipper: bool,
) {
    let (was_printer, was_job) = {
        let Ok(mut s) = state().lock() else { return };
        let entry = s.seen.entry(id.to_owned()).or_default();
        let before = entry.clone();
        entry.0 = printer.to_owned();
        if let Some(j) = job {
            entry.1 = j.to_owned();
        }
        before
    };
    if was_printer.is_empty() || was_printer == printer {
        return;
    }
    if printer == "not_ready" {
        event("attention", id, name, &stopped_text(name, message, klipper));
    } else if matches!(printer, "off" | "unreachable")
        && matches!(was_job.as_str(), "printing" | "paused")
    {
        event(
            "offline",
            id,
            name,
            &format!("{name}: the printer stopped answering in the middle of a print."),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_klipper_printer_is_announced_as_klipper_stopping() {
        assert_eq!(
            super::stopped_text("Stella", Some("MCU lost"), true),
            "Stella: Klipper has stopped. MCU lost"
        );
        let plain = super::stopped_text("Demo", Some("Stopped (emergency stop)"), false);
        assert_eq!(
            plain,
            "Demo: the printer has stopped. Stopped (emergency stop)"
        );
        assert!(!super::stopped_text("CORE One L", None, false).contains("Klipper"));
    }

    #[test]
    fn distinct_alerts_queue_while_duplicates_are_suppressed() {
        let mut state = super::State::new();
        let now = std::time::Instant::now();
        assert!(super::enqueue(&mut state, "driver warning", now));
        assert!(super::enqueue(&mut state, "Klipper stopped", now));
        assert!(super::enqueue(&mut state, "print failed", now));
        assert!(!super::enqueue(&mut state, "driver warning", now));
        assert_eq!(
            state.pending.into_iter().collect::<Vec<_>>(),
            ["driver warning", "Klipper stopped", "print failed"]
        );
    }
    #[test]
    fn settings_keep_the_key_to_themselves_and_check_the_number() {
        let mut cfg = crate::config::Config::default();
        cfg.database = format!("/tmp/stella-notify-test-{}", std::process::id());
        assert!(
            super::save(&cfg, br#"{"phone":"5069532678","apikey":"123"}"#).is_err(),
            "country code needed"
        );
        let v = super::save(
            &cfg,
            br#"{"phone":"+1 506 953 2678","apikey":"123456","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(v["phone"], "+15069532678");
        assert_eq!(v["has_key"], true);
        assert!(
            !v.to_string().contains("123456"),
            "the key never leaves the store"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(super::path(&cfg))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
        let v = super::save(&cfg, br#"{"phone":"+15069532678","enabled":false}"#).unwrap();
        assert_eq!(v["has_key"], true, "an empty key keeps the stored one");
        assert_eq!(v["enabled"], false);
        assert!(super::save(&cfg, br#"{"phone":"+15069532678","apikey":"bad key!"}"#).is_err());
        let _ = std::fs::remove_file(super::path(&cfg));
    }

    /// A stand-in for an ntfy server: it keeps every request it is sent (request line, headers, body).
    fn fake_ntfy() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let keep = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 2048];
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                keep.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).to_string());
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                );
            }
        });
        (format!("http://{addr}/stella"), seen)
    }

    fn scratch_cfg(tag: &str) -> crate::config::Config {
        let mut cfg = crate::config::Config::default();
        cfg.database = format!("/tmp/stella-notify-{tag}-{}", std::process::id());
        cfg
    }

    #[test]
    fn ntfy_addresses_are_checked_and_http_and_https_are_taken() {
        assert_eq!(
            super::ntfy_address(" http://192.168.1.10:8080/stella ").unwrap(),
            "http://192.168.1.10:8080/stella"
        );
        assert_eq!(
            super::ntfy_address("https://ntfy.sh/stella").unwrap(),
            "https://ntfy.sh/stella"
        );
        assert_eq!(
            super::ntfy_address("https://192.168.1.10:8443/topic").unwrap(),
            "https://192.168.1.10:8443/topic"
        );
        assert!(super::ntfy_address("ftp://host/t").is_err(), "no ftp");
        assert!(super::ntfy_address("http://host")
            .unwrap_err()
            .contains("topic"));
        assert!(
            super::ntfy_address("http://user:pw@host/t").is_err(),
            "no credentials in the address"
        );
        assert!(super::ntfy_address("http://host/with space").is_err());
        assert!(super::ntfy_address("http://host:/t").is_err());
        assert!(
            super::ntfy_address("http://host/a/b").is_err(),
            "one topic, no path"
        );
    }

    #[test]
    fn events_can_be_turned_off_one_by_one_and_the_other_settings_are_kept() {
        let cfg = scratch_cfg("events");
        // WhatsApp saved first; then events and ntfy must not lose it, and it must not lose them.
        super::save(
            &cfg,
            br#"{"phone":"+15065551234","apikey":"abc123","enabled":true}"#,
        )
        .unwrap();
        let v =
            super::save_events(&cfg, br#"{"events":{"queue_empty":false,"done":true}}"#).unwrap();
        let on = |v: &serde_json::Value, k: &str| {
            v["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["kind"] == k)
                .unwrap()["on"]
                .as_bool()
                .unwrap()
        };
        assert!(
            !on(&v, "queue_empty") && on(&v, "done") && on(&v, "plate_clear"),
            "only the named one changed: {v}"
        );
        assert_eq!(v["has_key"], true, "the WhatsApp key survived");
        assert!(super::save_events(&cfg, br#"{"events":{"made_up":true}}"#).is_err());
        let v = super::save_ntfy(
            &cfg,
            br#"{"url":"http://127.0.0.1:9/stella","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(
            (v["ntfy"]["url"].clone(), v["ntfy"]["enabled"].clone()),
            (
                serde_json::json!("http://127.0.0.1:9/stella"),
                serde_json::json!(true)
            )
        );
        let v = super::save(&cfg, br#"{"phone":"+15065551234","enabled":false}"#).unwrap();
        assert!(
            !on(&v, "queue_empty"),
            "saving WhatsApp again kept the events"
        );
        assert_eq!(
            v["ntfy"]["url"], "http://127.0.0.1:9/stella",
            "and the ntfy address"
        );
        let v = super::save_ntfy(&cfg, br#"{"url":""}"#).unwrap();
        assert_eq!(v["ntfy"]["enabled"], false);
        let _ = std::fs::remove_file(super::path(&cfg));
    }

    #[test]
    fn an_alert_reaches_ntfy_with_the_printer_to_open_and_a_muted_kind_does_not() {
        let cfg = scratch_cfg("ntfy");
        let (url, seen) = fake_ntfy();
        super::init(&cfg);
        super::save_ntfy(
            &cfg,
            format!("{{\"url\":\"{url}\",\"enabled\":true}}").as_bytes(),
        )
        .unwrap();
        super::save_events(&cfg, br#"{"events":{"queue_empty":false}}"#).unwrap();
        // A link id on file, as the helper writes it: the tapped alert names this computer too.
        let link = "ab".repeat(32);
        std::fs::write(
            format!("{}.link.json", cfg.database),
            format!("{{\"id\":\"{link}\"}}"),
        )
        .unwrap();
        assert!(super::event(
            "plate_clear",
            "voron-left",
            "Voron Left",
            "Voron Left: the plate needs clearing. (ntfy test)"
        ));
        assert!(
            !super::event(
                "queue_empty",
                "",
                "",
                "The queue is empty. (ntfy test, muted)"
            ),
            "a muted kind goes nowhere"
        );
        // Same text again within ten minutes: said once.
        assert!(!super::event(
            "plate_clear",
            "voron-left",
            "Voron Left",
            "Voron Left: the plate needs clearing. (ntfy test)"
        ));
        // Polish WO-06 (2026-10-18): the notifier is one per process, so a demo printer finishing in another test (the demo farm's)
        // reaches this fake too, and arrived first under load. Wait for this test's own alert, not for any alert.
        let start = std::time::Instant::now();
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("(ntfy test)"))
            && start.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = seen.lock().unwrap().clone();
        let mine: Vec<&String> = got.iter().filter(|r| r.contains("(ntfy test)")).collect();
        assert_eq!(mine.len(), 1, "{got:?}");
        let r = mine[0].to_ascii_lowercase();
        assert!(r.starts_with("post /stella "), "{r}");
        assert!(r.contains("title: voron left"), "{r}");
        assert!(
            r.contains(&format!("click: stella://p/voron-left?link={link}")),
            "{r}"
        );
        assert!(r.contains("the plate needs clearing"), "{r}");
        assert!(
            !got.iter().any(|r| r.contains("muted")),
            "the muted kind was not posted"
        );
        // What was said lately is on the settings page.
        let s = super::settings(&cfg);
        assert!(
            s["recent"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "plate_clear" && e["sent"].to_string().contains("ntfy")),
            "{s}"
        );
        assert!(s["recent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "queue_empty" && e["sent"].as_array().unwrap().is_empty()));
        // The printer's id is a path-safe word or nothing: anything else makes no address.
        assert_eq!(super::click_address(None, "a/../b"), None);
        assert_eq!(
            super::click_address(None, "core-one-l"),
            Some("stella://p/core-one-l".into())
        );
        let _ = std::fs::remove_file(super::path(&cfg));
        let _ = std::fs::remove_file(format!("{}.link.json", cfg.database));
        if let Ok(mut st) = super::state().lock() {
            st.cfg_path = None;
            st.database = None;
        }
    }

    #[test]
    fn callmebot_key_stays_out_of_process_arguments() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/whatsapp.php", listener.local_addr().unwrap());
        let command = super::callmebot_command(&url, "+15065551234", "test message");
        let args = command
            .get_args()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("apikey@-"));
        assert!(!args.contains("apikey="));
        assert!(!args.contains("secret123"));
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(error) => panic!("fake CallMeBot did not receive curl: {error}"),
                }
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nqueued",
                )
                .unwrap();
            request
        });
        let result =
            super::callmebot_to(&url, "+15065551234", "secret123", "test message").unwrap();
        assert!(result.status.success());
        let request = server.join().unwrap();
        assert!(request.contains("apikey=secret123"));
        assert!(
            request.contains("text=test+message") || request.contains("text=test%20message"),
            "fake provider received: {request}"
        );
    }
}
