//! Stella Scope for the computer next to the printer. The window opens on a one-screen setup the
//! first time: it finds Klipper printers on this network by asking every address for Moonraker's
//! /printer/info, you pick yours, and it writes Stella Scope's settings and starts the recorder
//! (which starts the phone link and, when switched on, the failure watch). After that it opens
//! straight into Stella Scope. Nothing leaves this computer except what Stella Scope itself does: no
//! account, no cloud copy.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
    time::Duration,
};
use tauri::{Manager, State};

const PORT: u16 = 7420;

#[derive(Default)]
struct Recorder(Mutex<Option<Child>>);

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("stella-scope.json"))
}

/// A plain HTTP GET with short timeouts; enough for Moonraker and Stella Scope on the home network.
fn http_get(addr: SocketAddr, path: &str, timeout: Duration) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout * 3)).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n").ok()?;
    let mut out = String::new();
    (&mut s).take(2 * 1024 * 1024 + 1).read_to_string(&mut out).ok()?;
    if out.len() > 2 * 1024 * 1024 { return None }
    let (head, body) = out.split_once("\r\n\r\n")?;
    head.lines().next().filter(|line| line.starts_with("HTTP/1.0 200 ") || line.starts_with("HTTP/1.1 200 "))?;
    Some(body.to_owned())
}
fn stella_running(cfg: &Value) -> bool {
    http_get(SocketAddr::from(([127, 0, 0, 1], PORT)), "/api/meta", Duration::from_millis(400))
        .is_some_and(|body| stella_meta(&body, cfg))
}
/// A Stella Scope answering on the port. With this app's own settings it must be that instance; with
/// none (a recorder installed as a service, as on the machine it was built for) any Stella Scope is
/// opened as it is, and anything that is not Stella Scope never is.
fn stella_meta(body: &str, cfg: &Value) -> bool {
    serde_json::from_str::<Value>(body).ok()
        .is_some_and(|body| body["service"] == "stella-scope" && body["collector"].is_object()
            && (cfg.is_null() || (cfg["moonraker_url"].is_string() && cfg["database"].is_string()
                && body["instance"]["moonraker_url"] == cfg["moonraker_url"]
                && body["instance"]["database"] == cfg["database"])))
}

/// An address pasted from Mainsail/Fluidd or a direct Moonraker host. An explicit port is tried
/// first; a web UI port falls back to Moonraker's usual 7125 after identity validation.
fn printer_candidates(input: &str) -> Result<(String, Vec<u16>), String> {
    let raw = input.trim();
    let (web, rest) = if let Some(rest) = raw.strip_prefix("http://") { (true, rest) }
        else if let Some(rest) = raw.strip_prefix("https://") { (true, rest) }
        else if raw.contains("://") { return Err("use an http:// or https:// printer address".into()) }
        else { (false, raw) };
    let authority = rest.split(&['/', '?', '#'][..]).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return Err("enter a printer IP, hostname or Mainsail/Fluidd URL without credentials".into());
    }
    let (host, explicit) = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, suffix) = ipv6.split_once(']').ok_or("close the IPv6 address with ]")?;
        if host.parse::<std::net::Ipv6Addr>().is_err() { return Err("invalid IPv6 printer address".into()) }
        let port = if suffix.is_empty() { None } else { Some(suffix.strip_prefix(':').ok_or("invalid printer port")?) };
        (host.to_owned(), port)
    } else {
        let (host, port) = authority.rsplit_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.is_empty() || host.contains(':') || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)) {
            return Err("invalid printer host; put IPv6 addresses in brackets".into());
        }
        (host.to_owned(), port)
    };
    let port = match explicit { Some(value) => value.parse::<u16>().ok().filter(|p| *p > 0).ok_or("printer port must be 1 to 65535")?, None => 7125 };
    let mut ports = vec![port];if web && port != 7125 { ports.push(7125); }
    Ok((host, ports))
}

fn find_moonraker(input: &str) -> Result<(String, SocketAddr), String> {
    let (host, ports) = printer_candidates(input)?;
    let mut resolved = false;
    for port in ports {
        let Ok(addresses) = (host.as_str(), port).to_socket_addrs() else { continue };
        for addr in addresses {
            resolved = true;
            let Some(body) = http_get(addr, "/printer/info", Duration::from_secs(2)) else { continue };
            if serde_json::from_str::<Value>(&body).ok().is_some_and(|v| v["result"]["state"].is_string()) {
                let url_host = if host.contains(':') { format!("[{host}]") } else { host.clone() };
                return Ok((format!("http://{url_host}:{port}"), addr));
            }
        }
    }
    Err(if resolved { "that address did not answer as Moonraker; check its direct HTTP port and printer power".into() }
        else { format!("{host} did not resolve; check the printer hostname or IP") })
}

#[tauri::command]
fn setup_state(app: tauri::AppHandle) -> Result<Value, String> {
    let path = config_path(&app)?;
    let cfg: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    Ok(json!({"configured":cfg.is_object(),"printer":cfg["moonraker_url"],"running":stella_running(&cfg),"url":format!("http://127.0.0.1:{PORT}/")}))
}

/// Every address on this computer's /24 that answers Moonraker's /printer/info on port 7125.
#[tauri::command]
async fn find_printers() -> Result<Vec<Value>, String> {
    let me = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .map_err(|e| format!("no network: {e}"))?;
    let IpAddr::V4(v4) = me.ip() else { return Err("only IPv4 home networks are searched".into()) };
    let [a, b, c, _] = v4.octets();
    let found = std::sync::Arc::new(Mutex::new(Vec::new()));
    let handles: Vec<_> = (1u8..=254)
        .map(|d| {
            let found = found.clone();
            std::thread::spawn(move || {
                let addr = SocketAddr::from(([a, b, c, d], 7125));
                if let Some(body) = http_get(addr, "/printer/info", Duration::from_millis(350)) {
                    if let Ok(v) = serde_json::from_str::<Value>(&body) {
                        let r = &v["result"];
                        if r.get("state").is_some() {
                            found.lock().unwrap().push(json!({"url":format!("http://{a}.{b}.{c}.{d}:7125"),
                                "host":r["hostname"],"state":r["state"],"version":r["software_version"]}));
                        }
                    }
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
    let list = found.lock().unwrap().clone();
    Ok(list)
}

/// Checks the printer answers, writes Stella Scope's settings, then starts it.
#[tauri::command]
async fn save_and_start(app: tauri::AppHandle, recorder: State<'_, Recorder>, printer: String, name: Option<String>) -> Result<String, String> {
    let (url, addr) = find_moonraker(&printer)?;
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    // The bed size comes from the printer's own config, so the mesh and probe maps are drawn to scale.
    let settings: Value = http_get(addr, "/printer/objects/query?configfile=settings", Duration::from_secs(3))
        .and_then(|b| serde_json::from_str(&b).ok())
        .unwrap_or(Value::Null);
    let s = &settings["result"]["status"]["configfile"]["settings"];
    let number = |v: &Value| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()).filter(|v| v.is_finite());
    let axis = |k: &str| -> Option<f64> { let max=number(&s[k]["position_max"])?;let min=number(&s[k]["position_min"]).unwrap_or(0.0);let travel=max-min;(10.0..=2000.0).contains(&travel).then_some(travel) };
    let mut cfg = json!({"moonraker_url":url,"port":PORT,"database":data.join("stella.sqlite3").to_string_lossy(),
        "controls_enabled":true,"lan_port":7422,"remote_link":true,"failure_watch":false,
        "probe_offset_mm":[0.0,0.0]});
    let (Some(x), Some(y)) = (axis("stepper_x"), axis("stepper_y")) else {
        return Err("Moonraker did not report valid X/Y travel; turn the printer on and verify its Klipper configuration before setup".into());
    };
    cfg["bed_size_mm"] = json!([x, y]);
    cfg["bed_size_confirmed"] = json!(true);
    for probe in ["bltouch", "probe"] {
        if let (Some(x), Some(y)) = (number(&s[probe]["x_offset"]), number(&s[probe]["y_offset"])) {
            cfg["probe_offset_mm"] = json!([x, y]);cfg["probe_offset_confirmed"] = json!(true);break;
        }
    }
    if let Some(n) = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()) {
        cfg["printer_name"] = json!(n);
    }
    let path = config_path(&app)?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).map_err(|e| e.to_string())?;
    start(app, recorder).await
}

/// Starts the recorder if nothing answers on the port yet, and waits until it does.
#[tauri::command]
async fn start(app: tauri::AppHandle, recorder: State<'_, Recorder>) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{PORT}/");
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(config_path(&app)?).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if stella_running(&cfg) {
        return Ok(url);
    }
    if TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], PORT)), Duration::from_millis(250)).is_ok() {
        return Err("port 7420 is occupied by another service; close that service before starting Stella Scope".into());
    }
    let bin = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(if cfg!(windows) { "stella-scope.exe" } else { "stella-scope" });
    let mut cmd = Command::new(&bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window beside the app
    }
    let child = cmd
        .args(["--config", &config_path(&app)?.to_string_lossy(), "--port", &PORT.to_string()])
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    *recorder.0.lock().unwrap() = Some(child);
    // A year of recordings can take a while to open on a cold disk (19 s seen on the 900 MB
    // database, 2026-09-26), so this waits a full minute before calling it a failure.
    for _ in 0..240 {
        std::thread::sleep(Duration::from_millis(250));
        if stella_running(&cfg) {
            return Ok(url);
        }
        if let Some(exit) = recorder.0.lock().unwrap().as_mut().ok_or("recorder process is missing")?
            .try_wait().map_err(|e| format!("cannot check the recorder process: {e}"))? {
            return Err(format!("Stella Scope exited before its identity appeared on port 7420 ({exit}); check its config and recorder log"));
        }
    }
    Err("Stella Scope did not start within a minute".into())
}

#[cfg(test)]
mod tests {
    use super::{printer_candidates, stella_meta};

    #[test]
    fn manual_address_forms_and_malformed_inputs() {
        for (input, host, ports) in [
            ("192.168.1.50", "192.168.1.50", vec![7125]),
            ("voron.local", "voron.local", vec![7125]),
            ("voron.local:8125", "voron.local", vec![8125]),
            ("http://voron.local/", "voron.local", vec![7125]),
            ("https://voron.local/#/printer", "voron.local", vec![7125]),
            ("http://voron.local:8080/printer", "voron.local", vec![8080, 7125]),
            ("[::1]:7125", "::1", vec![7125]),
        ] {
            assert_eq!(printer_candidates(input).unwrap(), (host.to_owned(), ports), "{input}");
        }
        for input in ["", "ftp://voron.local", "http://user:pass@voron.local", "voron.local:0", "voron.local:nope", "2001:db8::1", "http:///"] {
            assert!(printer_candidates(input).is_err(), "{input}");
        }
    }

    #[test]
    fn another_http_service_is_not_stella_scope() {
        let cfg = serde_json::json!({"moonraker_url":"http://127.0.0.1:7125","database":"/tmp/own.sqlite3"});
        assert!(!stella_meta("<html>ready</html>", &cfg));
        assert!(!stella_meta(r#"{"version":"0.4.0","collector":{}}"#, &cfg));
        assert!(!stella_meta(r#"{"service":"other","collector":{}}"#, &cfg));
        assert!(!stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"moonraker_url":"http://127.0.0.1:7125","database":"/tmp/other.sqlite3"}}"#, &cfg));
        assert!(stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"moonraker_url":"http://127.0.0.1:7125","database":"/tmp/own.sqlite3"}}"#, &cfg));
        // Not configured here: a running recorder service is opened, a stranger on the port is not.
        let none = serde_json::Value::Null;
        assert!(stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"moonraker_url":"http://192.168.1.50:7125","database":"/srv/other.sqlite3"}}"#, &none));
        assert!(!stella_meta(r#"{"service":"other","collector":{}}"#, &none));
        assert!(!stella_meta("<html>ready</html>", &none));
    }

    #[test]
    fn a_pasted_web_url_reaches_only_a_valid_moonraker_response() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            for status in ["500 Error", "200 OK"] {
                let (mut client, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(client.try_clone().unwrap());
                let mut line = String::new();reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("GET /printer/info "));
                loop { line.clear();reader.read_line(&mut line).unwrap();if line == "\r\n" { break; } }
                let body = r#"{"result":{"state":"ready"}}"#;
                write!(client, "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let url = format!("http://127.0.0.1:{port}/printer/");
        assert!(super::find_moonraker(&format!("127.0.0.1:{port}")).is_err());
        assert_eq!(super::find_moonraker(&url).unwrap().0, format!("http://127.0.0.1:{port}"));
        worker.join().unwrap();
    }
}

fn main() {
    tauri::Builder::default()
        .manage(Recorder::default())
        .invoke_handler(tauri::generate_handler![setup_state, find_printers, save_and_start, start])
        .build(tauri::generate_context!())
        .expect("Stella Scope could not start")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(mut c) = app.state::<Recorder>().0.lock().unwrap().take() {
                    let _ = c.kill();
                }
            }
        });
}
