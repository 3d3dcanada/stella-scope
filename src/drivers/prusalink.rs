//! PrusaLink (Prusa CORE One, MK4, XL...), farm board WO-04. Written from Prusa's public OpenAPI
//! (`Prusa-Link-Web/spec/openapi.yaml`, 1.0.0-draft) and from our own slicer-upload (which already
//! uploads to his CORE One L); no code from any other project. Everything it reports is what
//! `/api/v1/status`, `/info`, `/job`, `/storage` and `/files` say; a value PrusaLink does not give is
//! `null`, and the UI shows it as unknown.
//!
//! Spec facts relied on (verified against the spec text, NOT yet against our printer: it was off
//! when this was written, so a live capture is owed in WO-04's STOP-REPORT):
//! - HTTP digest auth (MD5, qop=auth) or `X-Api-Key`.
//! - `GET /api/v1/status`: printer.state IDLE|BUSY|PRINTING|PAUSED|FINISHED|STOPPED|ERROR|ATTENTION|READY,
//!   temp/target nozzle and bed, axis_x/y (only when the head is still) and axis_z, flow and speed in
//!   percent, fan RPMs; job.progress in percent, time_remaining and time_printing in seconds.
//! - `GET /api/v1/job` (204 when none), `PUT /api/v1/job/{id}/pause|resume`, `DELETE /api/v1/job/{id}`.
//! - `GET /api/v1/storage`, `GET|PUT|POST|DELETE /api/v1/files/{storage}/{path}`; PUT with
//!   `Print-After-Upload: ?0|?1`; POST starts the file.
use super::digest::Session;
use crate::{
    config::Config,
    driver::{Action, CameraSource, Capabilities, Driver, StateSnapshot},
    Result,
};
use serde_json::{json, Value};
use std::{
    io::Read,
    sync::Mutex,
    time::{Duration, Instant},
};

/// PrusaLink is a small embedded web server. One status read a second is plenty and kind to it.
const MIN_INTERVAL_S: f64 = 1.0;
/// How long a file transfer or a print start may take (a 5 MB file over the printer's own web server takes seconds).
const TRANSFER_TIMEOUT_S: u64 = 180;

/// 2026-10-25 ("the Prusa is on, but I can't do anything ... I can't change the nozzle"). PrusaLink's v1 API has no temperature, fan or
/// homing endpoint, but it starts a file it is given (`Print-After-Upload: ?1`). A one-line file is how heat, the part fan, homing and motors
/// off reach a CORE One: the route Prusa users run themselves (Prusa-Link issue #832, a working example on a Core One). The file starts with
/// `M862.6 P"Input shaper"`, which an input-shaper printer wants before it runs a file by itself. It only runs while the printer is idle:
/// a file cannot start during a print, so there is no way to change these mid-print, and the refusal says so.
pub const COMMAND_FILE: &str = "stella-command.gcode";

/// The G-code of one command file, or None for an action this route does not carry.
pub fn command_gcode(action: &Action) -> Option<String> {
    use crate::driver::Heater;
    let body = match action {
        Action::SetTemperature { heater: Heater::Nozzle, celsius } => format!("M104 S{:.0}", celsius.max(0.0)),
        Action::SetTemperature { heater: Heater::Bed, celsius } => format!("M140 S{:.0}", celsius.max(0.0)),
        Action::Fan { fraction } if *fraction <= 0.0 => "M107".to_owned(),
        Action::Fan { fraction } => format!("M106 S{:.0}", (fraction.clamp(0.0, 1.0) * 255.0).round()),
        Action::Home => "G28".to_owned(),
        Action::MotorsOff => "M84".to_owned(),
        _ => return None,
    };
    Some(format!("; Stella Scope: one command, sent as a file because PrusaLink has no command endpoint\nM862.6 P\"Input shaper\"\n{body}\n"))
}

pub struct PrusaLink {
    cfg: Config,
    base: String,
    agent: ureq::Agent,
    /// Polish WO-07 (live pass 2026-10-19): moving a whole print file, and starting one, takes the printer far longer than
    /// a status read. With the status timeout (1.5 s) a dry run's start timed out on the real CORE One L.
    slow: ureq::Agent,
    digest: Mutex<Option<Session>>,
    api_key: Option<String>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    at: f64,
    sample: Option<Value>,
    /// The file of the job last seen printing: PrusaLink forgets it when the job ends, and the
    /// finished card still wants to say what finished. It is what the printer last said, not a guess.
    last_file: String,
    storage: Option<String>,
    /// The last job the printer ran was one of Stella's command files (COMMAND_FILE): its FINISHED is not a print that finished.
    last_was_command: bool,
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl PrusaLink {
    pub fn new(cfg: &Config) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(cfg.request_timeout_ms.max(500))))
            .max_redirects(0)
            .proxy(None)
            .http_status_as_error(false)
            .max_idle_connections(1)
            .build()
            .into();
        let slow: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(TRANSFER_TIMEOUT_S)))
            .max_redirects(0)
            .proxy(None)
            .http_status_as_error(false)
            .max_idle_connections(1)
            .build()
            .into();
        let user = cfg.secret.get("user").unwrap_or("maker");
        let digest = cfg.secret.get("password").map(|p| Session::new(user, p));
        Self {
            cfg: cfg.clone(),
            base: cfg.moonraker_url.trim_end_matches('/').to_owned(),
            agent,
            slow,
            digest: Mutex::new(digest),
            api_key: cfg.secret.get("api_key").map(str::to_owned),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// One request, authorised with the printer's own credentials. A 401 teaches the digest session
    /// the printer's nonce and the request is repeated once. `limit` bounds the body read.
    pub fn call(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(&[u8], &str)>,
        limit: usize,
    ) -> Result<Reply> {
        self.call_on(&self.agent, method, path, headers, body, limit)
    }

    /// The same request with the long timeout: a file down or up, or a print started.
    fn call_slow(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(&[u8], &str)>,
        limit: usize,
    ) -> Result<Reply> {
        self.call_on(&self.slow, method, path, headers, body, limit)
    }

    fn call_on(
        &self,
        agent: &ureq::Agent,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(&[u8], &str)>,
        limit: usize,
    ) -> Result<Reply> {
        let url = format!("{}{path}", self.base);
        for attempt in 0..2 {
            let auth = if let Some(key) = &self.api_key {
                Some(("X-Api-Key", key.clone()))
            } else {
                self.digest
                    .lock()
                    .ok()
                    .and_then(|mut d| d.as_mut().and_then(|s| s.authorize(method, path)))
                    .map(|h| ("Authorization", h))
            };
            let mut response = match (method, body) {
                ("GET", _) => {
                    let mut r = agent.get(&url);
                    for (k, v) in headers {
                        r = r.header(*k, *v);
                    }
                    if let Some((k, v)) = &auth {
                        r = r.header(*k, v);
                    }
                    r.call()
                }
                ("DELETE", _) => {
                    let mut r = agent.delete(&url);
                    for (k, v) in headers {
                        r = r.header(*k, *v);
                    }
                    if let Some((k, v)) = &auth {
                        r = r.header(*k, v);
                    }
                    r.call()
                }
                (m, b) => {
                    let mut r = if m == "PUT" {
                        agent.put(&url)
                    } else {
                        agent.post(&url)
                    };
                    for (k, v) in headers {
                        r = r.header(*k, *v);
                    }
                    if let Some((k, v)) = &auth {
                        r = r.header(*k, v);
                    }
                    match b {
                        Some((bytes, ctype)) => r.content_type(ctype).send(bytes),
                        None => r.send_empty(),
                    }
                }
            }
            .map_err(|e| {
                format!(
                    "{} at {}: {e}; is the printer on and PrusaLink reachable on this network?",
                    crate::driver::UNREACHABLE_GENERIC,
                    self.base
                )
            })?;
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 && self.api_key.is_none() {
                let challenge = response
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let learned = match (challenge, self.digest.lock().ok()) {
                    (Some(h), Some(mut d)) => d.as_mut().is_some_and(|s| s.learn(&h)),
                    _ => false,
                };
                if learned {
                    continue;
                }
                return Err("PrusaLink asked for a login and none is saved (or it is refused); add the user and password in this printer's settings".into());
            }
            let mut bytes = Vec::new();
            response
                .body_mut()
                .as_reader()
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("PrusaLink response could not be read: {e}"))?;
            if bytes.len() > limit {
                return Err(format!(
                    "PrusaLink response is larger than the {limit} byte limit"
                ));
            }
            return Ok(Reply {
                status,
                body: bytes,
            });
        }
        Err("PrusaLink refused the login".into())
    }

    fn get_json(&self, path: &str) -> Result<Option<Value>> {
        let r = self.call("GET", path, &[], None, 2 * 1024 * 1024)?;
        match r.status {
            204 => Ok(None),
            200 => serde_json::from_slice(&r.body)
                .map(Some)
                .map_err(|e| format!("PrusaLink's answer for {path} could not be understood: {e}")),
            401 => Err("PrusaLink refused the login; check the user and password".into()),
            404 => Err(format!("this printer's PrusaLink has no {path}; its firmware may be too old")),
            code => Err(format!("PrusaLink answered {path} with an error ({code})")),
        }
    }

    fn storage(&self) -> String {
        if let Some(s) = self.cache.lock().ok().and_then(|c| c.storage.clone()) {
            return s;
        }
        let found = self
            .get_json("/api/v1/storage")
            .ok()
            .flatten()
            .and_then(|v| {
                v["storage_list"].as_array().and_then(|l| {
                    l.iter()
                        .find(|s| {
                            s["available"].as_bool() != Some(false)
                                && s["read_only"].as_bool() != Some(true)
                        })
                        .and_then(|s| s["name"].as_str().map(str::to_owned))
                })
            })
            .unwrap_or_else(|| "usb".to_owned());
        if let Ok(mut c) = self.cache.lock() {
            c.storage = Some(found.clone());
        }
        found
    }

    fn file_path(&self, storage: &str, name: &str) -> String {
        let encoded: String = name
            .split('/')
            .map(crate::moonraker::encode)
            .collect::<Vec<_>>()
            .join("/");
        format!("/api/v1/files/{storage}/{encoded}")
    }

    /// Print files below `dir` (a PrusaLink path such as `/usb/`), two folders deep.
    fn list_files(&self, dir: &str, depth: u8, out: &mut Vec<Value>, root: &str) -> Result<()> {
        let Some(v) = self.get_json(&format!("/api/v1/files{dir}"))? else {
            return Ok(());
        };
        for child in v["children"].as_array().cloned().unwrap_or_default() {
            let name = child["name"].as_str().unwrap_or("");
            let ty = child["type"].as_str().unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if ty.eq_ignore_ascii_case("folder") && depth > 0 {
                let _ = self.list_files(&format!("{dir}{name}/"), depth - 1, out, root);
            } else {
                // Polish WO-07 (live pass 2026-10-19): on a FAT stick `name` is the 8.3 short name ("TWISTB~1.GCO"), which
                // neither ends in .gcode nor reads as anything; the real name is `display_name`, and PrusaLink answers to it.
                let long = child["display_name"].as_str().filter(|s| !s.is_empty() && !s.contains('/')).unwrap_or(name);
                if printable(long) {
                    let rel = format!("{}{long}", dir.strip_prefix(root).unwrap_or(""));
                    out.push(json!({"path":rel,"size":child["size"],"modified":child["m_timestamp"],"display_name":long}));
                }
            }
        }
        Ok(())
    }
}

/// Files PrusaLink can print: ASCII and binary G-code.
fn printable(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".gcode") || l.ends_with(".bgcode")
}

/// PrusaLink's printer state words, in the Moonraker print states the service reads.
/// Returns (print_stats.state, idle_timeout.state, message).
pub fn map_state(word: &str) -> (&'static str, &'static str, &'static str) {
    match word.to_ascii_uppercase().as_str() {
        "PRINTING" => ("printing", "Printing", ""),
        "PAUSED" => ("paused", "Ready", ""),
        "ATTENTION" => (
            "paused",
            "Ready",
            "The printer is asking a question on its screen. Answer it there to go on.",
        ),
        "FINISHED" => ("complete", "Ready", ""),
        "STOPPED" => ("cancelled", "Ready", ""),
        "ERROR" => (
            "error",
            "Ready",
            "The printer reports an error (check its screen).",
        ),
        "BUSY" => ("standby", "Printing", ""),
        // IDLE, READY and anything not named: not printing.
        _ => ("standby", "Idle", ""),
    }
}

fn num(v: &Value) -> Value {
    v.as_f64()
        .filter(|n| n.is_finite())
        .map_or(Value::Null, |n| json!(n))
}

/// The pure part: PrusaLink's `status`, `job` and `info` into the object map the service records.
/// `last_file` is the file last seen printing (kept only for a finished or stopped job).
pub fn normalise(status: &Value, job: &Value, last_file: &str) -> Value {
    let p = &status["printer"];
    let state_word = p["state"].as_str().unwrap_or("");
    let (print_state, idle_state, message) = map_state(state_word);
    let printing = print_state == "printing";
    // X and Y come only while the head is still; while printing they are unknown, never the last value.
    let xy = |v: &Value| if printing { Value::Null } else { num(v) };
    let z = num(&p["axis_z"]);
    let (x, y) = (xy(&p["axis_x"]), xy(&p["axis_y"]));
    let j = if job.is_object() { job } else { &status["job"] };
    let progress = j["progress"].as_f64().map(|v| (v / 100.0).clamp(0.0, 1.0));
    let file = job["file"]["display_name"]
        .as_str()
        .or_else(|| job["file"]["name"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(
            if matches!(print_state, "complete" | "cancelled" | "error" | "paused") {
                last_file
            } else {
                ""
            },
        );
    let printed = num(&j["time_printing"]);
    let fan = |v: &Value| json!({"speed":Value::Null,"rpm":num(v)});
    let percent = |v: &Value| v.as_f64().map_or(Value::Null, |n| json!(n / 100.0));
    json!({
        "print_stats":{"state":print_state,"filename":file,"total_duration":printed,"print_duration":printed,
            "filament_used":Value::Null,"message":message,"info":{"total_layer":Value::Null,"current_layer":Value::Null}},
        "virtual_sdcard":{"progress":progress,"is_active":printing,"file_path":file},
        "idle_timeout":{"state":idle_state},
        "extruder":{"temperature":num(&p["temp_nozzle"]),"target":num(&p["target_nozzle"]),"power":Value::Null},
        "heater_bed":{"temperature":num(&p["temp_bed"]),"target":num(&p["target_bed"]),"power":Value::Null},
        "toolhead":{"position":[x,y,z,Value::Null],"homed_axes":"","max_velocity":Value::Null,"max_accel":Value::Null},
        "motion_report":{"live_position":[x,y,z,Value::Null],"live_velocity":Value::Null,"live_extruder_velocity":Value::Null},
        "gcode_move":{"speed_factor":percent(&p["speed"]),"extrude_factor":percent(&p["flow"]),"speed":Value::Null,
            "homing_origin":[Value::Null,Value::Null,Value::Null,Value::Null]},
        "fan":fan(&p["fan_print"]),
        "heater_fan hotend_fan":fan(&p["fan_hotend"]),
        "_scope_prusalink":{"state":state_word,"time_remaining":num(&j["time_remaining"]),"job_id":j["id"]},
    })
}

impl PrusaLink {
    /// Send one command file (COMMAND_FILE) and start it, only while the printer is idle.
    fn run_command(&self, gcode: &str) -> Result<Value> {
        let status = self
            .get_json("/api/v1/status")?
            .ok_or("PrusaLink returned no status")?;
        let word = status["printer"]["state"].as_str().unwrap_or("").to_ascii_uppercase();
        if !matches!(word.as_str(), "IDLE" | "READY" | "FINISHED" | "STOPPED") {
            return Err(format!(
                "PrusaLink takes heat, fan and homing only while the printer is idle (it is {}): it has no way to change them during a print, so nothing was sent",
                word.to_ascii_lowercase()
            ));
        }
        let mut r = self.upload(COMMAND_FILE, gcode.as_bytes(), true)?;
        r["via"] = json!("command file");
        Ok(r)
    }
}

impl Driver for PrusaLink {
    fn kind(&self) -> &'static str {
        "prusalink"
    }
    fn brand(&self) -> &'static str {
        "PrusaLink"
    }
    fn capabilities(&self) -> Capabilities {
        // Only what the v1 API provides: no temperature, move, home, fan or G-code endpoint exists in
        // the spec, so none is claimed. Whether this firmware offers more is a question for the live
        // pass; until a capture proves it, it stays off.
        Capabilities {
            pause_resume: true,
            cancel: true,
            start_print: true,
            upload: true,
            thumbnails: true,
            gcode_file: true,
            camera: self.cfg.camera_url.is_some(),
            xy: true,
            // through a one-line file while idle (COMMAND_FILE); refused, with the reason, during a print
            heat: true,
            fan: true,
            home: true,
            motors_off: true,
            ..Capabilities::none()
        }
    }
    /// WO-14 item 4: PrusaLink does not report the bed size, but `/api/v1/info` carries the hostname, which a Prusa derives from the model
    /// ("prusa-core-one-l"). Finish-2 F09 (2026-10-19): the model and the plate come from one table of every Prusa machine
    /// (`prusa_machines.rs`: the published print volumes, cited there) instead of two lists kept here. An unknown hostname is None, never a guess.
    fn model_hint(&mut self) -> Option<String> {
        let info = self.get_json("/api/v1/info").ok().flatten()?;
        crate::prusa_machines::for_hostname(info["hostname"].as_str()?).map(|m| m.short.to_owned())
    }
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        let info = self.get_json("/api/v1/info").ok().flatten()?;
        crate::prusa_machines::for_hostname(info["hostname"].as_str()?).and_then(|m| m.bed)
    }
    fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)> {
        if let Ok(c) = self.cache.lock() {
            if let (Some(v), true) = (&c.sample, ts - c.at < MIN_INTERVAL_S && ts >= c.at) {
                return Ok((v.clone(), Vec::new()));
            }
        }
        let started = Instant::now();
        let status = self
            .get_json("/api/v1/status")?
            .ok_or("PrusaLink returned no status")?;
        let word = status["printer"]["state"]
            .as_str()
            .unwrap_or("")
            .to_ascii_uppercase();
        // The job (file name, thumbnail) only while there is one.
        let job = if matches!(word.as_str(), "PRINTING" | "PAUSED" | "ATTENTION" | "BUSY") {
            self.get_json("/api/v1/job")
                .ok()
                .flatten()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let mut cache = self.cache.lock().map_err(|_| "driver state poisoned")?;
        let mut status = status;
        let mut job = job;
        if let Some(name) = job["file"]["display_name"]
            .as_str()
            .or_else(|| job["file"]["name"].as_str())
        {
            if name == COMMAND_FILE || name.ends_with(&format!("/{COMMAND_FILE}")) {
                cache.last_was_command = true;
            } else {
                cache.last_was_command = false;
                cache.last_file = name.to_owned();
            }
        }
        // A command file is not a print: while it runs the printer is busy, and when it is done it is ready, not "finished".
        if cache.last_was_command {
            let busy = matches!(word.as_str(), "PRINTING" | "BUSY");
            let done = matches!(word.as_str(), "FINISHED" | "STOPPED");
            if busy || done {
                status["printer"]["state"] = json!(if busy { "BUSY" } else { "IDLE" });
                status["job"] = Value::Null;
                job = Value::Null;
            }
        }
        let sample = normalise(&status, &job, &cache.last_file);
        cache.at = ts;
        cache.sample = Some(sample.clone());
        let _ = started;
        Ok((sample, Vec::new()))
    }
    fn files(&self) -> Result<Vec<Value>> {
        let storage = self.storage();
        let root = format!("/{storage}/");
        let mut out = Vec::new();
        self.list_files(&root, 2, &mut out, &root)?;
        out.sort_by(|a, b| {
            b["modified"]
                .as_f64()
                .partial_cmp(&a["modified"].as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(200);
        Ok(out)
    }
    fn file_metadata(&self, filename: &str) -> Result<Value> {
        let path = self.file_path(&self.storage(), filename);
        let v = self
            .get_json(&path)?
            .ok_or("PrusaLink returned no file information")?;
        Ok(
            json!({"size":v["size"],"modified":v["m_timestamp"],"slicer":Value::Null,
            "estimated_time":v["meta"]["estimated_print_time"],"filament_type":v["meta"]["filament_type"],
            "nozzle_diameter":v["meta"]["nozzle_diameter"],"layer_height":v["meta"]["layer_height"],
            // what the printer itself reported about the file, whole (finish-2 F06: its keys are not the ones the fake of an earlier board guessed)
            "printer_meta":v["meta"],
            "source":"GET /api/v1/files (PrusaLink)"}),
        )
    }
    /// WO-01: the file's size from its own metadata, read-only.
    fn file_size(&self, filename: &str) -> Result<usize> {
        let v = self.file_metadata(filename)?;
        v["size"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as usize)
            .ok_or_else(|| format!("PrusaLink did not report a size for {filename}"))
    }
    /// WO-01: the file's bytes, one read-only GET. The body is the gcode text the printer holds.
    fn file_bytes(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        // Polish WO-07 (live pass 2026-10-19): GET /api/v1/files/<storage>/<path> answers with the file's metadata as JSON,
        // not its bytes; the bytes are at the file's own download reference ("/usb/TWISTB~1.GCO"). Reading the metadata as
        // the file made a dry run upload 242 bytes of transformed JSON to the real CORE One L as its "dry run" G-code.
        let meta = self
            .get_json(&self.file_path(&self.storage(), filename))?
            .ok_or("PrusaLink returned no file information")?;
        let path = meta["refs"]["download"]
            .as_str()
            .filter(|p| p.starts_with('/') && !p.contains("..") && !p.contains(['\r', '\n']))
            .ok_or("PrusaLink gave no download address for that file")?
            .to_owned();
        let r = self.call_slow("GET", &path, &[], None, max_bytes)?;
        if r.status != 200 {
            return Err(format!(
                "PrusaLink returned HTTP {} for {filename}",
                r.status
            ));
        }
        if r.body.len() > max_bytes {
            return Err(format!(
                "{filename} is larger than the {max_bytes} byte gcode limit; slice a smaller file"
            ));
        }
        Ok(r.body)
    }
    fn file_thumbnail(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let path = self.file_path(&self.storage(), filename);
        // Polish WO-06 (2026-10-19): the printer answering 404 for this file means it is not on the storage asked (a print started from
        // another drive, or from the printer's own screen), not that the firmware is old: say so, so the picture route can answer 404.
        let v = match self.get_json(&path) {
            Err(e) if e.contains("firmware may be too old") => {
                return Err("no such file on the printer's storage".into())
            }
            other => other?,
        }
        .ok_or("PrusaLink returned no file information")?;
        let thumb = v["refs"]["thumbnail"]
            .as_str()
            .or_else(|| v["refs"]["icon"].as_str())
            .ok_or("that file has no thumbnail")?;
        if !thumb.starts_with('/') || thumb.contains("..") || thumb.contains(['\r', '\n']) {
            return Err("thumbnail path is not a plain path on the printer".into());
        }
        // Finish-2 F02: a slicer picture took more than the 1.5 s status timeout on the CORE One L (502 on its file card). It is
        // a transfer, so it takes the transfer timeout; transfers run one at a time beside the polls (drivers.rs `transfer`).
        // ponytail: one stuck picture holds the next transfer up to the transfer timeout; a picture timeout if that bites.
        let r = self.call_slow("GET", thumb, &[], None, max_bytes)?;
        // Finish-2 F02 (CORE One L): a file sliced without a picture lists a thumbnail address that answers 404; that is no
        // picture, not a failing printer (it showed as a 502 in the browser on every page that listed the file).
        if r.status == 404 {
            return Err("that file has no thumbnail".into());
        }
        if r.status != 200 {
            return Err(format!(
                "PrusaLink returned HTTP {} for the thumbnail",
                r.status
            ));
        }
        if r.body.len() < 8 || r.body[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
            return Err("the thumbnail is not a PNG; refusing to serve it as one".into());
        }
        Ok(r.body)
    }
    fn cameras(&self) -> Result<Vec<CameraSource>> {
        // The CORE One's Buddy3D camera is its own device with its own address; PrusaLink does not
        // list it (2026-08-31, CAMERA.md). The address is set per printer.
        Ok(self
            .cfg
            .camera_url
            .iter()
            .map(|u| CameraSource {
                name: "Camera".into(),
                kind: if u.starts_with("rtsp://") {
                    "rtsp"
                } else {
                    "mjpeg"
                },
                url: u.clone(),
            })
            .collect())
    }
    fn snapshot(&self) -> Result<StateSnapshot> {
        let status = self
            .get_json("/api/v1/status")?
            .ok_or("PrusaLink returned no status")?;
        let (print_state, idle_state, _) =
            map_state(status["printer"]["state"].as_str().unwrap_or(""));
        Ok(StateSnapshot {
            state: if print_state == "error" {
                "error".into()
            } else {
                "ready".into()
            },
            // PrusaLink does not report homing; commands that need it are not offered for this brand.
            homed_axes: String::new(),
            print_state: print_state.to_owned(),
            idle_state: idle_state.to_ascii_lowercase(),
            help: Default::default(),
        })
    }
    fn act(&self, action: &Action) -> Result<Value> {
        // The job id is read fresh: pause, resume and stop name the job they act on.
        let job_path = |verb: &str| -> Result<String> {
            let job = self
                .get_json("/api/v1/job")?
                .ok_or("there is no job to act on")?;
            let id = job["id"]
                .as_i64()
                .ok_or("PrusaLink did not name the running job")?;
            Ok(if verb.is_empty() {
                format!("/api/v1/job/{id}")
            } else {
                format!("/api/v1/job/{id}/{verb}")
            })
        };
        let check = |r: Reply, what: &str| -> Result<Value> {
            match r.status {
                200 | 201 | 202 | 204 => Ok(json!({"result":"ok","status":r.status})),
                401 => Err("PrusaLink refused the login; check the user and password".into()),
                409 => Err(format!(
                    "PrusaLink {} {what} in its current state",
                    crate::driver::PRINTER_REFUSED
                )),
                code => Err(format!("PrusaLink answered with an error ({code}) when asked to {what}")),
            }
        };
        match action {
            Action::Pause => check(
                self.call(
                    "PUT",
                    &job_path("pause")?,
                    &[],
                    Some((b"", "text/plain")),
                    4096,
                )?,
                "pause",
            ),
            Action::Resume => check(
                self.call(
                    "PUT",
                    &job_path("resume")?,
                    &[],
                    Some((b"", "text/plain")),
                    4096,
                )?,
                "resume",
            ),
            Action::Cancel => check(
                self.call("DELETE", &job_path("")?, &[], None, 4096)?,
                "cancel the job",
            ),
            Action::StartPrint { filename } => {
                if !printable(filename) || filename.contains("..") || filename.starts_with('/') {
                    return Err("name a .gcode or .bgcode file from the printer's own list".into());
                }
                check(
                    self.call_slow(
                        "POST",
                        &self.file_path(&self.storage(), filename),
                        &[],
                        None,
                        4096,
                    )?,
                    "start that file",
                )
            }
            other => match command_gcode(other) {
                Some(gcode) => self.run_command(&gcode),
                None => Err(format!("PrusaLink cannot do that from here ({other:?})")),
            },
        }
    }
    fn upload(&self, name: &str, bytes: &[u8], start: bool) -> Result<Value> {
        if !printable(name)
            || name.contains("..")
            || name.starts_with('/')
            || name.contains(['\r', '\n'])
        {
            return Err("the file name must be a plain .gcode or .bgcode name".into());
        }
        let path = self.file_path(&self.storage(), name);
        let after = if start { "?1" } else { "?0" };
        let r = self.call_slow(
            "PUT",
            &path,
            &[("Print-After-Upload", after), ("Overwrite", "?1")],
            Some((bytes, "application/octet-stream")),
            4096,
        )?;
        match r.status {
            200 | 201 | 202 | 204 => Ok(json!({"result":"ok","status":r.status,"started":start})),
            409 => Err(
                "PrusaLink refused the upload: the printer is busy or the file already exists"
                    .into(),
            ),
            code => Err(format!("PrusaLink answered the upload with an error ({code})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes from the OpenAPI spec (SPEC, not a capture from our printer).
    fn status(state: &str, with_xy: bool) -> Value {
        let mut printer = json!({"state":state,"temp_bed":60.0,"target_bed":60.0,"temp_nozzle":214.7,"target_nozzle":215.0,
            "axis_z":12.4,"flow":100,"speed":100,"fan_hotend":5200,"fan_print":3100});
        if with_xy {
            printer["axis_x"] = json!(120.5);
            printer["axis_y"] = json!(88.0);
        }
        json!({"storage":{"path":"/usb/","name":"usb","read_only":false},"printer":printer,
            "job":{"id":42,"progress":43.5,"time_remaining":1800,"time_printing":1400}})
    }
    fn job() -> Value {
        json!({"id":42,"state":"PRINTING","progress":43.5,"time_remaining":1800,"time_printing":1400,
            "file":{"name":"BRACKET~1.GCO","display_name":"bracket_PETG.gcode","path":"/usb","size":123456}})
    }

    #[test]
    fn every_printer_state_maps_to_a_print_state_and_nothing_is_invented() {
        for (word, print, idle) in [
            ("IDLE", "standby", "Idle"),
            ("READY", "standby", "Idle"),
            ("BUSY", "standby", "Printing"),
            ("PRINTING", "printing", "Printing"),
            ("PAUSED", "paused", "Ready"),
            ("ATTENTION", "paused", "Ready"),
            ("FINISHED", "complete", "Ready"),
            ("STOPPED", "cancelled", "Ready"),
            ("ERROR", "error", "Ready"),
            ("printing", "printing", "Printing"),
            ("SOMETHING_NEW", "standby", "Idle"),
        ] {
            let o = normalise(&status(word, true), &Value::Null, "");
            assert_eq!(o["print_stats"]["state"], print, "{word}");
            assert_eq!(o["idle_timeout"]["state"], idle, "{word}");
        }
        assert!(
            !map_state("ATTENTION").2.is_empty(),
            "attention says so in words"
        );
    }

    #[test]
    fn a_printing_job_is_read_whole_and_xy_are_unknown_while_it_prints() {
        let o = normalise(&status("PRINTING", true), &job(), "");
        assert_eq!(o["print_stats"]["filename"], "bracket_PETG.gcode");
        assert_eq!(o["print_stats"]["print_duration"], 1400.0);
        assert_eq!(o["virtual_sdcard"]["progress"], 0.435);
        assert_eq!(o["extruder"]["temperature"], 214.7);
        assert_eq!(o["heater_bed"]["target"], 60.0);
        // Even though the status carried axis_x/y, a printing head's X and Y are not shown as live.
        assert_eq!(o["toolhead"]["position"], json!([null, null, 12.4, null]));
        assert_eq!(
            o["motion_report"]["live_position"],
            json!([null, null, 12.4, null])
        );
        assert_eq!(o["gcode_move"]["speed_factor"], 1.0);
        assert_eq!(o["fan"]["rpm"], 3100.0);
        assert!(o["fan"]["speed"].is_null(), "an RPM is not a duty cycle");
        // Not reported by PrusaLink: unknown, never zero.
        assert!(o["print_stats"]["filament_used"].is_null());
        assert!(o["extruder"]["power"].is_null());
        assert!(o["toolhead"]["max_velocity"].is_null());
        assert_eq!(o["_scope_prusalink"]["time_remaining"], 1800.0);
    }

    #[test]
    fn a_still_head_reports_xy_and_a_missing_axis_is_null() {
        let o = normalise(&status("IDLE", true), &Value::Null, "");
        assert_eq!(o["toolhead"]["position"], json!([120.5, 88.0, 12.4, null]));
        let mut s = status("IDLE", false);
        s["printer"].as_object_mut().unwrap().remove("axis_z");
        let o = normalise(&s, &Value::Null, "");
        assert_eq!(o["toolhead"]["position"], json!([null, null, null, null]));
    }

    #[test]
    fn a_finished_job_keeps_the_name_of_what_finished() {
        let o = normalise(
            &status("FINISHED", true),
            &Value::Null,
            "bracket_PETG.gcode",
        );
        assert_eq!(o["print_stats"]["state"], "complete");
        assert_eq!(o["print_stats"]["filename"], "bracket_PETG.gcode");
        // An idle printer with an old name on file does not claim to have a file.
        assert_eq!(
            normalise(&status("IDLE", true), &Value::Null, "old.gcode")["print_stats"]["filename"],
            ""
        );
    }

    // ---- against the fake PrusaLink (src/fakes.rs): digest login enforced, spec shapes ----------

    fn live(fake: &crate::fakes::FakePrusaLink, pass: &str) -> PrusaLink {
        let mut cfg = Config {
            moonraker_url: fake.url(),
            request_timeout_ms: 1500,
            ..Config::default()
        };
        cfg.secret = crate::config::Secret(
            [
                ("user".to_owned(), "maker".to_owned()),
                ("password".to_owned(), pass.to_owned()),
            ]
            .into(),
        );
        PrusaLink::new(&cfg)
    }
    fn post_log(fake: &crate::fakes::FakePrusaLink) -> Vec<(String, String)> {
        fake.log()
            .iter()
            .filter(|e| e["method"] != "GET")
            .map(|e| {
                (
                    e["method"].as_str().unwrap().to_owned(),
                    e["path"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn the_login_is_digest_and_a_wrong_password_reads_nothing() {
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let mut ok = live(&fake, "secret");
        let (sample, _) = ok.sample(100.0).unwrap();
        assert_eq!(sample["print_stats"]["state"], "standby");
        assert!(sample["extruder"]["temperature"]
            .as_f64()
            .is_some_and(|t| (20.0..=25.0).contains(&t)));
        // Next call reuses the printer's nonce (no new challenge round trip): the counter moves.
        assert!(ok.sample(102.5).is_ok());
        assert!(fake.state.lock().unwrap().logins >= 2);
        let mut bad = live(&fake, "wrong");
        let e = bad.sample(100.0).unwrap_err();
        assert!(e.contains("login") || e.contains("refused"), "{e}");
        assert!(!e.contains("wrong") && !e.contains("secret"));
        // No credentials at all: told what to do.
        let mut none = PrusaLink::new(&Config {
            moonraker_url: fake.url(),
            ..Config::default()
        });
        assert!(none
            .sample(1.0)
            .unwrap_err()
            .contains("add the user and password"));
    }

    #[test]
    fn an_unreachable_printer_says_it_cannot_be_reached_in_the_words_the_link_state_reads() {
        let mut d = PrusaLink::new(&Config {
            moonraker_url: "http://127.0.0.1:9".into(),
            request_timeout_ms: 500,
            ..Config::default()
        });
        let e = d.sample(1.0).unwrap_err();
        assert!(e.contains(crate::driver::UNREACHABLE_GENERIC), "{e}");
        let (state, _) = crate::model::printer_link(
            false,
            Some(&format!("Printer not answering. {e}")),
            &Value::Null,
            false,
        );
        assert_eq!(state, "unreachable");
    }

    #[test]
    fn every_state_the_printer_can_say_reaches_the_recording_as_a_print_state() {
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let mut d = live(&fake, "secret");
        let mut t = 1000.0;
        for (word, want) in [
            ("IDLE", "standby"),
            ("READY", "standby"),
            ("BUSY", "standby"),
            ("PRINTING", "printing"),
            ("PAUSED", "paused"),
            ("ATTENTION", "paused"),
            ("FINISHED", "complete"),
            ("STOPPED", "cancelled"),
            ("ERROR", "error"),
        ] {
            {
                let mut s = fake.state.lock().unwrap();
                s.state = word.into();
                s.job_id = (word == "PRINTING").then_some(5);
                s.file = "bracket_PETG.gcode".into();
            }
            t += 10.0;
            let (o, _) = d.sample(t).unwrap();
            assert_eq!(o["print_stats"]["state"], want, "{word}");
            assert_eq!(o["_scope_prusalink"]["state"], word);
        }
    }

    #[test]
    fn pause_resume_stop_start_and_upload_send_exactly_these_requests() {
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let d = live(&fake, "secret");
        {
            let mut s = fake.state.lock().unwrap();
            s.state = "PRINTING".into();
            s.job_id = Some(42);
            s.file = "bracket_PETG.gcode".into();
        }
        d.act(&Action::Pause).unwrap();
        assert_eq!(fake.state.lock().unwrap().state, "PAUSED");
        d.act(&Action::Resume).unwrap();
        assert_eq!(fake.state.lock().unwrap().state, "PRINTING");
        d.act(&Action::Cancel).unwrap();
        assert_eq!(fake.state.lock().unwrap().state, "STOPPED");
        // Conflicts are reported in words, not as success.
        let e = d.act(&Action::Pause).unwrap_err();
        assert!(e.contains("no job") || e.contains("cannot"), "{e}");
        d.act(&Action::StartPrint {
            filename: "cube_20mm_PETG.bgcode".into(),
        })
        .unwrap();
        assert_eq!(fake.state.lock().unwrap().state, "PRINTING");
        assert!(d
            .act(&Action::StartPrint {
                filename: "../etc/passwd".into()
            })
            .is_err());
        assert!(d
            .act(&Action::StartPrint {
                filename: "notes.txt".into()
            })
            .is_err());
        // Upload: queued only, then started.
        d.upload(
            "plate one.gcode",
            b"G28
",
            false,
        )
        .unwrap();
        fake.state.lock().unwrap().state = "IDLE".into();
        d.upload(
            "plate2.gcode",
            b"G28
G1 X1
",
            true,
        )
        .unwrap();
        assert!(d.upload("../x.gcode", b"", false).is_err());
        assert_eq!(
            post_log(&fake),
            vec![
                ("PUT".to_owned(), "/api/v1/job/42/pause".to_owned()),
                ("PUT".to_owned(), "/api/v1/job/42/resume".to_owned()),
                ("DELETE".to_owned(), "/api/v1/job/42".to_owned()),
                (
                    "POST".to_owned(),
                    "/api/v1/files/usb/cube_20mm_PETG.bgcode".to_owned()
                ),
                (
                    "PUT".to_owned(),
                    "/api/v1/files/usb/plate%20one.gcode".to_owned()
                ),
                (
                    "PUT".to_owned(),
                    "/api/v1/files/usb/plate2.gcode".to_owned()
                ),
            ]
        );
        let up = fake.state.lock().unwrap().uploads.clone();
        assert_eq!(
            up,
            vec![
                ("plate one.gcode".to_owned(), 4, false),
                ("plate2.gcode".to_owned(), 10, true)
            ]
        );
        let headers: Vec<Value> = fake
            .log()
            .iter()
            .filter(|e| e["method"] == "PUT" && e["path"].as_str().unwrap().contains("plate"))
            .map(|e| e["headers"].clone())
            .collect();
        assert_eq!(headers[0]["print-after-upload"], "?0");
        assert_eq!(headers[1]["print-after-upload"], "?1");
        // Things PrusaLink v1 has no endpoint for are refused, not faked.
        assert!(d.act(&Action::Home).is_err());
        assert!(d
            .act(&Action::SetTemperature {
                heater: crate::driver::Heater::Nozzle,
                celsius: 215.0
            })
            .is_err());
    }

    /// Polish WO-07 (live pass): a file's bytes come from its download reference, not the metadata JSON its API path answers with.
    #[test]
    fn file_bytes_are_the_file_and_not_its_metadata() {
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let d = live(&fake, "secret");
        let body = b"; generated by PrusaSlicer\nG28\nG1 X10 Y10 Z0.2 E0.5 F1200\n";
        d.upload("long name plate.gcode", body, false).unwrap();
        assert_eq!(d.file_bytes("long name plate.gcode", 1 << 20).unwrap(), body.to_vec());
        assert!(d.files().unwrap().iter().any(|f| f["path"] == "long name plate.gcode"), "listed by its long name");
    }

    #[test]
    fn files_and_thumbnails_come_from_the_printer() {
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let d = live(&fake, "secret");
        let files = d.files().unwrap();
        assert_eq!(files.len(), 3, "two with pictures and one without (finish-2 F02)");
        assert_eq!(
            files[0]["path"], "cube_20mm_PETG.bgcode",
            "newest first, .bgcode counts as printable"
        );
        let png = d.file_thumbnail("cube_20mm_PETG.bgcode", 100_000).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
        assert!(d.file_thumbnail("nope.gcode", 1000).is_err());
        assert_eq!(
            d.file_metadata("cube_20mm_PETG.bgcode").unwrap()["estimated_time"],
            1260
        );
    }

    /// Through the whole service: the page's control list for a PrusaLink printer hides what v1 cannot
    /// do and keeps pause, resume, cancel and start, and a pause reaches the printer once.
    #[test]
    fn through_the_service_only_what_the_printer_can_do_is_offered() {
        use crate::commands::Service;
        use std::{collections::BTreeMap, net::IpAddr};
        let fake = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        {
            let mut s = fake.state.lock().unwrap();
            s.state = "PRINTING".into();
            s.job_id = Some(42);
        }
        let mut cfg = Config {
            driver: "prusalink".into(),
            moonraker_url: fake.url(),
            database: format!("/tmp/stella-prusa-test-{}.sqlite3", std::process::id()),
            controls_enabled: true,
            request_timeout_ms: 1500,
            ..Config::default()
        };
        cfg.secret = crate::config::Secret(
            [
                ("user".to_owned(), "maker".to_owned()),
                ("password".to_owned(), "secret".to_owned()),
            ]
            .into(),
        );
        let service = Service::new(&cfg, "live").unwrap();
        let get = |p: &str| {
            let r = service.handle(
                "GET",
                p,
                &BTreeMap::from([("host".into(), "127.0.0.1:7420".into())]),
                &[],
                IpAddr::from([127, 0, 0, 1]),
            );
            serde_json::from_slice::<Value>(&r.body).unwrap()
        };
        let controls = get("/api/controls");
        assert_eq!(controls["brand"], "PrusaLink");
        let cmd = |id: &str| {
            controls["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap()
                .clone()
        };
        for id in ["jog", "console", "qgl", "filament_load"] {
            assert_eq!(cmd(id)["supported"], false, "{id}");
        }
        // heat, fan, home and motors off go as a one-line file while idle (COMMAND_FILE, 2026-10-25)
        for id in ["pause", "resume", "cancel", "print_start", "set_heater", "set_fan", "home", "motors_off"] {
            assert_eq!(cmd(id)["supported"], true, "{id}");
        }
        assert_eq!(cmd("pause")["allowed"], true);
        let headers = BTreeMap::from([
            ("host".to_owned(), "127.0.0.1:7420".to_owned()),
            ("origin".to_owned(), "http://127.0.0.1:7420".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            (
                "x-stella-control-token".to_owned(),
                controls["token"].as_str().unwrap().to_owned(),
            ),
        ]);
        let r = service.handle(
            "POST",
            "/api/commands",
            &headers,
            br#"{"id":"pause"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(r.code, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(
            post_log(&fake),
            vec![("PUT".to_owned(), "/api/v1/job/42/pause".to_owned())]
        );
        for suffix in [
            "",
            ".commands.sqlite3",
            ".commands.sqlite3-wal",
            ".commands.sqlite3-shm",
        ] {
            let _ = std::fs::remove_file(format!("{}{suffix}", cfg.database));
        }
    }

    #[test]
    fn capabilities_claim_only_what_the_v1_api_provides() {
        let d = PrusaLink::new(&Config::default());
        let c = d.capabilities();
        assert!(c.pause_resume && c.cancel && c.start_print && c.upload && c.thumbnails);
        // heat, fan, home and motors off go through a one-line file while idle (COMMAND_FILE, 2026-10-25); a hand jog and typed G-code still do not
        for (id, want) in [
            ("set_heater", true),
            ("set_fan", true),
            ("home", true),
            ("motors_off", true),
            ("jog", false),
            ("speed_factor", false),
            ("console", false),
            ("emergency_stop", false),
            ("qgl", false),
        ] {
            assert_eq!(c.allows(id, "PrusaLink").is_ok(), want, "{id}");
        }
        assert!(!c.camera, "no camera until an address is set");
        let with = PrusaLink::new(&Config {
            camera_url: Some("rtsp://192.168.1.132:554/live".into()),
            ..Config::default()
        });
        assert!(with.capabilities().camera);
        assert_eq!(with.cameras().unwrap()[0].kind, "rtsp");
    }

    #[test]
    fn a_command_is_a_one_line_file_that_an_input_shaper_printer_runs_by_itself() {
        use crate::driver::Heater;
        let g = command_gcode(&Action::SetTemperature { heater: Heater::Nozzle, celsius: 215.4 }).unwrap();
        assert!(g.contains("M862.6 P\"Input shaper\"\nM104 S215\n"), "{g}");
        assert!(command_gcode(&Action::SetTemperature { heater: Heater::Bed, celsius: 0.0 }).unwrap().contains("\nM140 S0\n"));
        assert!(command_gcode(&Action::Fan { fraction: 0.5 }).unwrap().contains("\nM106 S128\n"));
        assert!(command_gcode(&Action::Fan { fraction: 0.0 }).unwrap().contains("\nM107\n"));
        assert!(command_gcode(&Action::Home).unwrap().contains("\nG28\n"));
        assert!(command_gcode(&Action::MotorsOff).unwrap().contains("\nM84\n"));
        assert!(command_gcode(&Action::Jog { axis: 'z', mm: 1.0 }).is_none(), "no hand jog: PrusaLink does not say whether it is homed");
        assert!(command_gcode(&Action::Gcode("M104 S200".into())).is_none(), "typed G-code is not carried");
    }
}
