//! Stella Scope for the computer next to the printer (2026-09-25: "builds for Windows, Mac,
//! Linux, very easy to install"). The first time, one button writes Stella Scope's settings with no printer and starts
//! the recorder (which starts the phone link and, when switched on, the failure watch); printers of every brand are
//! added in Stella Scope itself. After that it opens straight into Stella Scope.
//! Nothing leaves this computer except what Stella Scope itself does: no account, no cloud copy.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
    time::Duration,
};
use tauri::{Manager, State};

/// 7420, or `STELLA_SCOPE_PORT` (finish-2 F10, 2026-10-19: the window can be tested on a scratch port beside a running
/// Stella Scope instead of only on 7420).
fn port() -> u16 {
    std::env::var("STELLA_SCOPE_PORT").ok().and_then(|p| p.parse().ok()).filter(|p| *p > 1024).unwrap_or(7420)
}

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
    http_get(SocketAddr::from(([127, 0, 0, 1], port())), "/api/meta", Duration::from_millis(400))
        .is_some_and(|body| stella_meta(&body, cfg))
}
/// A Stella Scope answering on the port. With this app's own settings it must be that instance; with
/// none (a recorder installed as a service, as on the machine it was built for) any Stella Scope is
/// opened as it is, and anything that is not Stella Scope never is.
/// Finish-2 F10 (2026-10-19): settings this app writes now name no printer (printers of any brand are added in Stella Scope
/// itself), and a farm answers with its first printer's own recording, `<database>.printers/<id>.sqlite3`: the instance is
/// this app's when its recording is the settings' database or lives beside it.
fn stella_meta(body: &str, cfg: &Value) -> bool {
    serde_json::from_str::<Value>(body).ok()
        .is_some_and(|body| body["service"] == "stella-scope" && body["collector"].is_object()
            && (cfg.is_null() || cfg["database"].as_str().is_some_and(|db| {
                body["instance"]["database"].as_str().is_some_and(|got| got == db || got.starts_with(&format!("{db}.printers/")))
            })))
}

#[tauri::command]
fn setup_state(app: tauri::AppHandle) -> Result<Value, String> {
    let path = config_path(&app)?;
    let cfg: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    Ok(json!({"configured":cfg.is_object(),"running":stella_running(&cfg),"url":format!("http://127.0.0.1:{}/", port())}))
}

/// Finish-2 F10 (2026-10-19): the first run writes settings that name no printer and starts Stella Scope, whose own
/// Add a printer takes every brand and searches the network only when asked. The old first run scanned the whole /24
/// for anything answering on 7125 by itself, which on our network includes a printer bridge that must never be asked.
fn first_settings(port: u16, database: &str) -> Value {
    json!({"port":port,"database":database,"controls_enabled":true,"lan_port":port + 2,"remote_link":true,"failure_watch":false})
}

/// Writes Stella Scope's settings (only when there are none yet), then starts it.
#[tauri::command]
async fn create_and_start(app: tauri::AppHandle, recorder: State<'_, Recorder>) -> Result<String, String> {
    let path = config_path(&app)?;
    if !path.exists() {
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
        let cfg = first_settings(port(), &data.join("stella.sqlite3").to_string_lossy());
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).map_err(|e| e.to_string())?;
    }
    start(app, recorder).await
}

/// Starts the recorder if nothing answers on the port yet, and waits until it does.
#[tauri::command]
async fn start(app: tauri::AppHandle, recorder: State<'_, Recorder>) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/", port());
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(config_path(&app)?).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if stella_running(&cfg) {
        return Ok(url);
    }
    if TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port())), Duration::from_millis(250)).is_ok() {
        return Err(format!("port {} is occupied by another service; close that service before starting Stella Scope", port()));
    }
    let bin = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(if cfg!(windows) { "stella-scope.exe" } else { "stella-scope" });
    let mut cmd = Command::new(&bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window beside the app
    }
    let child = cmd
        .args(["--config", &config_path(&app)?.to_string_lossy(), "--port", &port().to_string()])
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    *recorder.0.lock().unwrap() = Some(child);
    // A year of recordings can take a while to open on a cold disk (19 s seen on our 900 MB
    // database, 2026-09-26), so this waits a full minute before calling it a failure.
    for _ in 0..240 {
        std::thread::sleep(Duration::from_millis(250));
        if stella_running(&cfg) {
            return Ok(url);
        }
        if let Some(exit) = recorder.0.lock().unwrap().as_mut().ok_or("recorder process is missing")?
            .try_wait().map_err(|e| format!("cannot check the recorder process: {e}"))? {
            return Err(format!("Stella Scope exited before its identity appeared on port {} ({exit}); check its config and recorder log", port()));
        }
    }
    Err("Stella Scope did not start within a minute".into())
}

/// Finish-2 F10 (2026-10-19): the window comes back at the size it was closed at, and never larger than the screen it
/// opens on (our screens are 1920x1080 and a 768x1366 portrait one, where the old fixed 1440x900 did not fit).
fn window_size(saved: Option<(f64, f64)>, screen: (f64, f64)) -> (f64, f64) {
    let (w, h) = saved.filter(|(w, h)| w.is_finite() && h.is_finite()).unwrap_or((1440.0, 900.0));
    (w.min(screen.0 - 40.0).max(360.0), h.min(screen.1 - 80.0).max(480.0))
}
fn window_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("window.json"))
}

#[cfg(test)]
mod tests {
    use super::{first_settings, stella_meta};

    #[test]
    fn another_http_service_is_not_stella_scope() {
        let cfg = serde_json::json!({"database":"/tmp/own.sqlite3"});
        assert!(!stella_meta("<html>ready</html>", &cfg));
        assert!(!stella_meta(r#"{"version":"0.4.0","collector":{}}"#, &cfg));
        assert!(!stella_meta(r#"{"service":"other","collector":{}}"#, &cfg));
        assert!(!stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"database":"/tmp/other.sqlite3"}}"#, &cfg));
        assert!(!stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"database":"/tmp/own.sqlite3.other/x.sqlite3"}}"#, &cfg));
        assert!(stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"database":"/tmp/own.sqlite3"}}"#, &cfg));
        // A farm answers with its first printer's recording beside the settings' database.
        assert!(stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"database":"/tmp/own.sqlite3.printers/core-one-l.sqlite3"}}"#, &cfg));
        // Not configured here: a running recorder service is opened, a stranger on the port is not.
        let none = serde_json::Value::Null;
        assert!(stella_meta(r#"{"service":"stella-scope","collector":{},"instance":{"moonraker_url":"http://192.168.1.50:7125","database":"/srv/other.sqlite3"}}"#, &none));
        assert!(!stella_meta(r#"{"service":"other","collector":{}}"#, &none));
        assert!(!stella_meta("<html>ready</html>", &none));
    }

    #[test]
    fn the_first_settings_name_no_printer() {
        let cfg = first_settings(7432, "/tmp/own.sqlite3");
        assert!(cfg.get("moonraker_url").is_none() && cfg.get("printers").is_none());
        assert_eq!((cfg["port"].as_u64(), cfg["lan_port"].as_u64()), (Some(7432), Some(7434)));
    }

    #[test]
    fn the_window_fits_the_screen_and_keeps_its_size() {
        use super::window_size;
        assert_eq!(window_size(None, (1920.0, 1080.0)), (1440.0, 900.0));
        assert_eq!(window_size(Some((1100.0, 700.0)), (1920.0, 1080.0)), (1100.0, 700.0));
        assert_eq!(window_size(None, (768.0, 1366.0)), (728.0, 900.0));
        assert_eq!(window_size(Some((3000.0, 100.0)), (1920.0, 1080.0)), (1880.0, 480.0));
    }
}

fn main() {
    tauri::Builder::default()
        .manage(Recorder::default())
        .invoke_handler(tauri::generate_handler![setup_state, create_and_start, start])
        .setup(|app| {
            let Some(window) = app.get_webview_window("main") else { return Ok(()) };
            let saved = window_file(app.handle()).and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| Some((v["width"].as_f64()?, v["height"].as_f64()?)));
            // 2026-10-20: the window opened on the screen at the left edge of the desktop (his small side screen, switched off), not
            // his main one. It opens in the middle of the primary screen, sized to it.
            if let Ok(Some(m)) = window.primary_monitor().or_else(|_| window.current_monitor()) {
                let scale = m.scale_factor();
                let screen = m.size().to_logical::<f64>(scale);
                let origin = m.position().to_logical::<f64>(scale);
                let (w, h) = window_size(saved, (screen.width, screen.height));
                let _ = window.set_size(tauri::LogicalSize::new(w, h));
                let _ = window.set_position(tauri::LogicalPosition::new(origin.x + (screen.width - w) / 2.0, origin.y + (screen.height - h) / 2.0));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let (Ok(size), Ok(scale), Some(path)) = (window.inner_size(), window.scale_factor(), window_file(window.app_handle())) {
                    let size = size.to_logical::<f64>(scale);
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let _ = std::fs::write(path, json!({"width":size.width,"height":size.height}).to_string());
                }
            }
        })
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
