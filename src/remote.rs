//! Anywhere access: when `remote_link` is on, Stella Scope starts `stella-link` (the link/ crate,
//! installed beside this binary). It holds an iroh endpoint whose key lives in
//! `<database>.link.key` (mode 600) and joins every stream a phone opens to the phone listener, so
//! a phone away from home meets exactly the same pairing code and device cookie as one on the home
//! network. iroh keeps its async runtime in that separate process; this service stays blocking.
use crate::config::Config;
use serde_json::{json, Value};
use std::{
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicU64, Ordering}, Mutex, OnceLock},
};

/// A helper started without a console window of its own on Windows (it would flash up otherwise).
#[allow(unused_mut)]
pub fn quiet(mut cmd: Command) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn child() -> &'static Mutex<Option<Child>> {
    static C: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}
fn status_path(cfg: &Config) -> String {
    format!("{}.link.json", cfg.database)
}

pub fn start(cfg: &Config) {
    if !cfg.remote_link {
        return;
    }
    let Some(port) = cfg.lan_port else { return };
    let Some(bin) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(format!("stella-link{}", std::env::consts::EXE_SUFFIX)))) else { return };
    if !bin.exists() {
        eprintln!("anywhere access is off: {} is not installed", bin.display());
        return;
    }
    let _ = std::fs::remove_file(status_path(cfg));
    // Paired phones only, from the first connection (src/lan.rs keeps this file current).
    if let Err(e) = crate::lan::write_allow(cfg) {
        eprintln!("anywhere access is off: {e}");
        return;
    }
    match quiet(Command::new(&bin))
        .args(["serve", "--key", &format!("{}.link.key", cfg.database), "--forward", &format!("127.0.0.1:{port}"), "--status", &status_path(cfg), "--allow", &crate::lan::allow_path(cfg)])
        .stdout(Stdio::null())
        .stdin(Stdio::piped())
        .env("STELLA_WATCH_STDIN", "1")
        .spawn()
    {
        Ok(c) => {
            println!("Anywhere access through stella-link (pid {})", c.id());
            if let Ok(mut g) = child().lock() {
                *g = Some(c);
            }
        }
        Err(e) => eprintln!("anywhere access is off: cannot start {}: {e}", bin.display()),
    }
}

/// `GET /api/pair/remote` (desktop only): whether it is on, the id a phone dials, and whether the
/// relay is reachable. The id is not a secret by itself; pairing still needs the desktop's code.
pub fn status(cfg: &Config) -> Value {
    let running = child().lock().ok().and_then(|mut g| g.as_mut().map(|c| c.try_wait().map(|s| s.is_none()).unwrap_or(false))).unwrap_or(false);
    let s: Value = std::fs::read_to_string(status_path(cfg)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let id = s["id"].as_str().unwrap_or("").to_owned();
    json!({"enabled":cfg.remote_link,"running":running,"id":id,"online":s["online"].as_bool().unwrap_or(false),
        "relay":s["relay"],"link":if id.is_empty(){Value::Null}else{json!(format!("stella://link/{id}"))},
        "qr_svg":if id.is_empty(){Value::Null}else{json!(qr_svg(&format!("stella://link/{id}")))},
        "app_url":crate::server::APP_PAGE,
        "app_qr_svg":qr_svg(crate::server::APP_PAGE)})
}

/// One scan sets a phone up: the code a new pairing window opens carries this computer's link id
/// and the six digits, and the Stella app dials the id and pairs with the digits by itself. Expires
/// with the window (five minutes).
pub fn with_pair_qr(cfg: &Config, mut v: Value) -> Value {
    let s = status(cfg);
    // Only a live helper: a saved id with stella-link gone would send the phone dialling nothing.
    let live = s["running"] == true;
    if let (Some(id), Some(code)) = (s["id"].as_str().filter(|i| live && !i.is_empty()), v["code"].as_str()) {
        let link = format!("stella://pair/{id}?code={code}");
        v["qr_svg"] = json!(qr_svg(&link));
        v["link"] = json!(link);
    }
    v
}

fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#07090C")).light_color(qrcode::render::svg::Color("#FFFFFF")).build(),
        Err(_) => String::new(),
    }
}

// ---------- failure watch (stella-eye) ----------
static EYE_GENERATION: AtomicU64 = AtomicU64::new(0);
fn eye_child() -> &'static Mutex<Option<Child>> {
    static C: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}
pub fn eye_status_path(cfg: &Config) -> String {
    format!("{}.eye.json", cfg.database)
}
/// The model file: `models/obico-failure-5a6b1be1fa.onnx` beside the recording.
fn eye_model(cfg: &Config) -> std::path::PathBuf {
    std::path::Path::new(&cfg.database).parent().unwrap_or(std::path::Path::new(".")).join("models/obico-failure-5a6b1be1fa.onnx")
}
fn eye_flag(cfg: &Config) -> String {
    format!("{}.eye.on", cfg.database)
}
/// On in the config, or switched on from the app (a flag file beside the recording). the failure
/// watch is an add-on the user installs, not part of the installers (Obico's model and scoring
/// are AGPL). It is found beside the recording, where its model lives, or, on a machine built
/// from source, beside this program.
fn eye_bin(cfg: &Config) -> Option<std::path::PathBuf> {
    let name = format!("stella-eye{}", std::env::consts::EXE_SUFFIX);
    let data = std::path::Path::new(&cfg.database).parent().unwrap_or(std::path::Path::new(".")).join(&name);
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&name)));
    [Some(data), beside].into_iter().flatten().find(|p| p.exists())
}
fn eye_on(cfg: &Config) -> bool {
    std::fs::read_to_string(eye_flag(cfg)).map(|v| v.trim() == "on").unwrap_or(cfg.failure_watch)
}
pub fn start_eye(cfg: &Config) {
    if !eye_on(cfg) {
        return;
    }
    let Ok(mut child_slot) = eye_child().lock() else { return };
    if child_slot.as_mut().is_some_and(|c| c.try_wait().map(|s| s.is_none()).unwrap_or(true)) { return; }
    let Some(bin) = eye_bin(cfg) else {
        eprintln!("failure watch is off: the stella-eye add-on is not installed");
        return;
    };
    let model = eye_model(cfg);
    if !model.exists() {
        eprintln!("failure watch is off: {} is missing", model.display());
        return;
    }
    if !eye_on(cfg) { return; }
    let _ = std::fs::remove_file(eye_status_path(cfg));
    let sens = cfg.failure_sensitivity.clamp(0.5, 2.0).to_string();
    match quiet(Command::new(&bin))
        .args(["--model", &model.to_string_lossy(), "--moonraker", cfg.moonraker_url.trim_end_matches('/'),
            "--snapshot", &format!("http://127.0.0.1:{}/api/camera/snapshot", cfg.port), "--status", &eye_status_path(cfg),
            "--state", &format!("{}.eye-state.json", cfg.database), "--sensitivity", &sens])
        .stdout(Stdio::null())
        .stdin(Stdio::piped())
        .env("STELLA_WATCH_STDIN", "1")
        .spawn()
    {
        Ok(c) => {
            println!("Failure watch through stella-eye (pid {})", c.id());
            let generation = EYE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
            let pid = c.id();
            *child_slot = Some(c);
            drop(child_slot);
            // WhatsApp on the way up to a warning or a failure, once per level per print.
            let path = eye_status_path(cfg);
            let _ = std::thread::Builder::new().name("stella-eye-watch".into()).spawn(move || {
                let mut said = (0u64, String::new());
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    if EYE_GENERATION.load(Ordering::SeqCst) != generation || !eye_child().lock().ok().is_some_and(|mut g| g.as_mut().is_some_and(|c| c.id() == pid && c.try_wait().ok().flatten().is_none())) { break; }
                    let Some(v) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
                    if v["watching"] != true || v["at"].as_f64().is_none_or(|at| crate::model::now() - at > 30.0 || at > crate::model::now() + 5.0) { continue; }
                    let (run, verdict) = (v["run"].as_u64().unwrap_or(0), v["verdict"].as_str().unwrap_or("").to_owned());
                    if run == 0 { continue; }
                    if run != said.0 {
                        said = (run, String::new());
                    }
                    let rank = |s: &str| match s { "warning" => 1, "failing" => 2, _ => 0 };
                    if rank(&verdict) > rank(&said.1) {
                        let pct = (v["normalized"].as_f64().unwrap_or(0.0) * 100.0).round();
                        crate::notify::send(&if verdict == "failing" {
                            format!("Stella: this print looks like it is FAILING (spaghetti detector at {pct}%). Check the camera now; the print is still running.")
                        } else {
                            format!("Stella: the spaghetti detector sees something odd ({pct}%). Worth a look at the camera.")
                        });
                        said.1 = verdict;
                    }
                }
            });
        }
        Err(e) => eprintln!("failure watch is off: cannot start {}: {e}", bin.display()),
    }
}
/// The newest verdict, or null when the watch is off or has not written one.
pub fn eye(cfg: &Config) -> Value {
    let downloading = EYE_DOWNLOAD.lock().map(|d| d.clone()).unwrap_or(Value::Null);
    if !eye_on(cfg) {
        return json!({"enabled":false,"installed":eye_bin(cfg).is_some(),"has_model":eye_model(cfg).exists(),"download":downloading});
    }
    let running = eye_child().lock().ok().is_some_and(|mut g| g.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_none()));
    let mut v: Value = std::fs::read_to_string(eye_status_path(cfg)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    if !running || v["at"].as_f64().is_none_or(|at| crate::model::now() - at > 30.0 || at > crate::model::now() + 5.0) { v = json!({}); }
    v["enabled"] = json!(true);
    v["has_model"] = json!(eye_model(cfg).exists());
    v["installed"] = json!(eye_bin(cfg).is_some());
    v["running"] = json!(running);
    v["download"] = downloading;
    v
}

static EYE_DOWNLOAD: Mutex<Value> = Mutex::new(Value::Null);
const EYE_MODEL_URL: &str = "https://tsd-pub-static.s3.amazonaws.com/ml-models/model-weights-5a6b1be1fa.onnx";
const EYE_MODEL_BYTES: u64 = 202_223_918;

/// `POST /api/eye/switch` {on}: the desktop turns the failure watch on or off. The first time on,
/// Obico's model (200 MB) is fetched in the background beside the recording; the watch starts when
/// it is complete. Off stops it; the model stays for next time.
pub fn eye_switch(cfg: &Config, body: &[u8]) -> crate::Result<Value> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "send {\"on\":true}")?;
    let on = v["on"].as_bool().ok_or("send {\"on\":true} or {\"on\":false}")?;
    if !on {
        std::fs::write(eye_flag(cfg), b"off").map_err(|e| format!("cannot switch the watch off: {e}"))?;
        EYE_GENERATION.fetch_add(1, Ordering::SeqCst);
        let _ = std::fs::remove_file(eye_status_path(cfg));
        if let Ok(mut g) = eye_child().lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        return Ok(eye(cfg));
    }
    if eye_bin(cfg).is_none() {
        return Err("the failure watch is an add-on: install stella-eye first (see the Stella Scope manual, Failure watch)".into());
    }
    std::fs::write(eye_flag(cfg), b"on").map_err(|e| format!("cannot switch the watch on: {e}"))?;
    let model = eye_model(cfg);
    if model.exists() {
        start_eye(cfg);
        return Ok(eye(cfg));
    }
    if EYE_DOWNLOAD.lock().map(|d| d["active"].as_bool() == Some(true)).unwrap_or(false) {
        return Ok(eye(cfg));
    }
    let owned = cfg.clone();
    *EYE_DOWNLOAD.lock().map_err(|_| "busy")? = json!({"active":true,"bytes":0,"total":EYE_MODEL_BYTES});
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(model.parent().unwrap_or(std::path::Path::new(".")));
        let part = model.with_extension("part");
        let mut child = match quiet(Command::new("curl")).args(["-sL", "--fail", "-o"]).arg(&part).arg(EYE_MODEL_URL).spawn() {
            Ok(c) => c,
            Err(e) => {
                *EYE_DOWNLOAD.lock().unwrap() = json!({"active":false,"error":format!("curl could not run: {e}")});
                return;
            }
        };
        let ok = loop {
            if let Ok(Some(st)) = child.try_wait() {
                break st.success();
            }
            let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            *EYE_DOWNLOAD.lock().unwrap() = json!({"active":true,"bytes":got,"total":EYE_MODEL_BYTES});
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        let size = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if ok && size == EYE_MODEL_BYTES && std::fs::rename(&part, &model).is_ok() {
            *EYE_DOWNLOAD.lock().unwrap() = json!({"active":false,"done":true});
            start_eye(&owned);
        } else {
            let _ = std::fs::remove_file(&part);
            *EYE_DOWNLOAD.lock().unwrap() = json!({"active":false,"error":format!("the model did not download completely ({size} of {EYE_MODEL_BYTES} bytes)")});
        }
    });
    Ok(eye(cfg))
}
