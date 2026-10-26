//! Timelapse and print reports (2026-10-26, "build those five"). While a printer prints, one picture from its camera at each new
//! layer (at most one every MIN_GAP_S; one every EVERY_S when the printer does not report its layer), kept per print beside the recording:
//! `<database>.timelapse/<started>-<file>/000001.jpg` and `print.json` (the file, when it started and ended, how it ended, the frames).
//! The page plays the pictures back in order. Bounded: FRAMES_MAX a print, the newest KEEP_PRINTS prints, TOTAL_MAX_BYTES on disk.
//! Nothing is sent anywhere; a printer with no camera records nothing and is asked again only once a minute.
use crate::{collector::Shared, config::Config};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FRAMES_MAX: usize = 3000;
const KEEP_PRINTS: usize = 40;
const TOTAL_MAX_BYTES: u64 = 3 * 1024 * 1024 * 1024;
const MIN_GAP_S: f64 = 4.0;
const EVERY_S: f64 = 30.0;
const NO_CAMERA_RETRY_S: f64 = 60.0;
const FRAME_MAX_BYTES: usize = 4 * 1024 * 1024;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The folder every print's pictures go under, beside this printer's recording.
pub fn root(cfg: &Config) -> PathBuf {
    PathBuf::from(format!("{}.timelapse", cfg.database))
}

/// A folder name a person can read and nothing can escape from: the start time and the file's own name, letters and digits only.
pub fn session_name(started: f64, file: &str) -> String {
    let stem = file.rsplit('/').next().unwrap_or(file);
    let stem = stem.strip_suffix(".gcode").or_else(|| stem.strip_suffix(".bgcode")).unwrap_or(stem);
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(60)
        .collect();
    format!("{:.0}-{}", started.max(0.0), if safe.is_empty() { "print".into() } else { safe })
}

/// A session name as `session_name` makes them; anything else (a path, dots) is refused before it touches the disk.
pub fn valid_session(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && name.split('-').next().is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

/// What one tick says about the print: its state, the file, and the layer when the printer reports it.
fn job_of(tick: &Value) -> (String, String, Option<f64>) {
    let job = &tick["job"];
    (
        job["state"].as_str().unwrap_or("").to_owned(),
        job["name"].as_str().or_else(|| job["file"].as_str()).unwrap_or("").to_owned(),
        job["layer"].as_f64(),
    )
}

/// The next frame is due: a new layer (and the last frame at least MIN_GAP_S ago), or EVERY_S since the last when no layer is known.
pub fn frame_due(layer: Option<f64>, last_layer: Option<f64>, since_last_s: f64) -> bool {
    match layer {
        Some(l) => last_layer != Some(l) && since_last_s >= MIN_GAP_S,
        None => since_last_s >= EVERY_S,
    }
}

struct Session {
    dir: PathBuf,
    meta: Value,
    frames: usize,
    last_layer: Option<f64>,
    last_at: f64,
}

impl Session {
    fn write_meta(&self) {
        let _ = fs::write(self.dir.join("print.json"), serde_json::to_vec_pretty(&self.meta).unwrap_or_default());
    }
}

/// Start the recorder for one printer; it ends when `stop` is set.
pub fn spawn(cfg: Config, state: Shared, stop: Arc<AtomicBool>, id: String) {
    let _ = thread::Builder::new()
        .name(format!("timelapse-{id}"))
        .stack_size(256 * 1024)
        .spawn(move || run(&cfg, &state, &stop));
}

fn run(cfg: &Config, state: &Shared, stop: &AtomicBool) {
    let mut session: Option<Session> = None;
    let mut no_camera_until = 0.0;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(2));
        let tick = state.lock().ok().and_then(|s| s.last_tick.clone()).unwrap_or(Value::Null);
        let (job_state, file, layer) = job_of(&tick);
        let t = now();
        let printing = job_state == "printing" && !file.is_empty();
        // a print that ended, or another file started: close the one being recorded
        if let Some(s) = session.as_mut() {
            if !printing || s.meta["file"].as_str() != Some(file.as_str()) {
                s.meta["ended"] = json!(t);
                s.meta["outcome"] = json!(if job_state.is_empty() { "unknown".to_owned() } else { job_state.clone() });
                s.write_meta();
                session = None;
                prune(&root(cfg));
            }
        }
        if !printing || t < no_camera_until {
            continue;
        }
        if session.is_none() {
            let started = t;
            let dir = root(cfg).join(session_name(started, &file));
            if fs::create_dir_all(&dir).is_err() {
                no_camera_until = t + NO_CAMERA_RETRY_S;
                continue;
            }
            let s = Session { dir, meta: json!({"file":file,"started":started,"ended":null,"outcome":"printing","frames":0}), frames: 0, last_layer: None, last_at: 0.0 };
            s.write_meta();
            session = Some(s);
        }
        let Some(s) = session.as_mut() else { continue };
        if s.frames >= FRAMES_MAX || !frame_due(layer, s.last_layer, t - s.last_at) {
            continue;
        }
        match crate::camera::still(cfg) {
            Ok(jpeg) if !jpeg.is_empty() && jpeg.len() <= FRAME_MAX_BYTES => {
                s.frames += 1;
                if fs::write(s.dir.join(format!("{:06}.jpg", s.frames)), &jpeg).is_ok() {
                    s.last_layer = layer;
                    s.last_at = t;
                    s.meta["frames"] = json!(s.frames);
                    if s.frames % 10 == 1 {
                        s.write_meta();
                    }
                } else {
                    s.frames -= 1;
                }
            }
            _ => no_camera_until = t + NO_CAMERA_RETRY_S,
        }
    }
    if let Some(mut s) = session {
        s.meta["ended"] = json!(now());
        s.meta["outcome"] = json!("stopped");
        s.write_meta();
    }
}

fn dir_bytes(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

/// Keep the newest KEEP_PRINTS prints and at most TOTAL_MAX_BYTES; the oldest go first.
pub fn prune(root: &Path) {
    let mut sessions: Vec<(String, PathBuf)> = fs::read_dir(root)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    valid_session(&name).then(|| (name, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    sessions.sort_by_key(|(n, _)| n.split('-').next().and_then(|t| t.parse::<u64>().ok()).unwrap_or(0));
    let mut total: u64 = sessions.iter().map(|(_, p)| dir_bytes(p)).sum();
    while sessions.len() > KEEP_PRINTS || (total > TOTAL_MAX_BYTES && sessions.len() > 1) {
        let (_, p) = sessions.remove(0);
        total = total.saturating_sub(dir_bytes(&p));
        let _ = fs::remove_dir_all(&p);
    }
}

/// `GET /api/timelapse` (every print recorded, newest first) and `GET /api/timelapse/<print>/<n>.jpg` (one picture).
pub fn handle(method: &str, path: &str, cfg: &Config) -> crate::api::Response {
    use crate::api::Response;
    if method != "GET" {
        return Response::json(405, json!({"error":"read-only; use GET"}));
    }
    let root = root(cfg);
    let rest = path.trim_start_matches("/api/timelapse").trim_start_matches('/');
    if rest.is_empty() {
        let mut out: Vec<Value> = fs::read_dir(&root)
            .map(|it| {
                it.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !valid_session(&name) {
                            return None;
                        }
                        let meta: Value = fs::read(e.path().join("print.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
                        Some(json!({"id":name,"file":meta["file"],"started":meta["started"],"ended":meta["ended"],"outcome":meta["outcome"],"frames":meta["frames"],"bytes":dir_bytes(&e.path())}))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b["started"].as_f64().unwrap_or(0.0).total_cmp(&a["started"].as_f64().unwrap_or(0.0)));
        return Response::json(200, json!({"prints":out,"keep":KEEP_PRINTS,"frames_max":FRAMES_MAX}));
    }
    let Some((session, frame)) = rest.split_once('/') else {
        return Response::json(404, json!({"error":"ask for /api/timelapse/<print>/<n>.jpg"}));
    };
    let n = frame.strip_suffix(".jpg").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=FRAMES_MAX).contains(n));
    let (true, Some(n)) = (valid_session(session), n) else {
        return Response::json(400, json!({"error":"not a timelapse picture"}));
    };
    match fs::read(root.join(session).join(format!("{n:06}.jpg"))) {
        Ok(body) => Response { code: 200, mime: "image/jpeg", body },
        Err(_) => Response::json(404, json!({"error":"no such picture"})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_and_only_safe_names_are_read() {
        assert_eq!(session_name(1792000000.4, "folder/3D3D twist vase.gcode"), "1792000000-3D3D_twist_vase");
        assert_eq!(session_name(5.0, ""), "5-print");
        assert!(valid_session("1792000000-3D3D_twist_vase"));
        for bad in ["", "../etc", "x-1", "1792/../2", "1792.0-a", "1792-a b"] {
            assert!(!valid_session(bad), "{bad}");
        }
    }

    #[test]
    fn a_frame_is_taken_at_each_new_layer_or_every_half_minute_without_one() {
        assert!(frame_due(Some(2.0), Some(1.0), 10.0), "a new layer");
        assert!(!frame_due(Some(2.0), Some(2.0), 100.0), "the same layer");
        assert!(!frame_due(Some(3.0), Some(2.0), 1.0), "layers faster than the gap");
        assert!(frame_due(None, None, 31.0) && !frame_due(None, None, 10.0), "no layer: every 30 s");
    }

    #[test]
    fn the_api_lists_prints_serves_their_pictures_and_refuses_paths() {
        let db = std::env::temp_dir().join(format!("stella-tl-{}", std::process::id()));
        let cfg = Config { database: db.to_string_lossy().into_owned(), ..Config::default() };
        let dir = root(&cfg).join("1792000000-vase");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("print.json"), br#"{"file":"vase.gcode","started":1792000000,"ended":1792003600,"outcome":"complete","frames":1}"#).unwrap();
        fs::write(dir.join("000001.jpg"), b"\xff\xd8jpeg").unwrap();
        let list = handle("GET", "/api/timelapse", &cfg);
        let v: Value = serde_json::from_slice(&list.body).unwrap();
        assert_eq!(v["prints"][0]["id"], "1792000000-vase");
        assert_eq!(v["prints"][0]["outcome"], "complete");
        assert_eq!(handle("GET", "/api/timelapse/1792000000-vase/1.jpg", &cfg).body, b"\xff\xd8jpeg");
        assert_eq!(handle("GET", "/api/timelapse/../1.jpg", &cfg).code, 400);
        assert_eq!(handle("GET", "/api/timelapse/1792000000-vase/../../x.jpg", &cfg).code, 400);
        assert_eq!(handle("POST", "/api/timelapse", &cfg).code, 405);
        let _ = fs::remove_dir_all(root(&cfg));
    }

    #[test]
    fn pruning_keeps_the_newest_prints() {
        let base = std::env::temp_dir().join(format!("stella-tl-prune-{}", std::process::id()));
        for i in 0..(KEEP_PRINTS + 3) {
            fs::create_dir_all(base.join(format!("{}-p", 1000 + i))).unwrap();
        }
        prune(&base);
        let left = fs::read_dir(&base).unwrap().count();
        assert_eq!(left, KEEP_PRINTS);
        assert!(!base.join("1000-p").exists() && base.join(format!("{}-p", 1000 + KEEP_PRINTS + 2)).exists());
        let _ = fs::remove_dir_all(&base);
    }
}
