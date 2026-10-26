//! FlashForge Adventurer 5M, stock firmware 5.1.x (farm board WO-05).
//!
//! Written from our own measurements of his printer (`~/3d3d-printers/ad5m/`: probe.sh, ffsend.py,
//! RESEARCH.md and a real `/detail` reply captured 2026-08-30 on firmware 5.1.7, serial redacted) and
//! the published endpoint list of Parallel-7/flashforge-api-docs (read, not copied). OrcaSlicer's
//! Flashforge.cpp (AGPL) and ff-5mp-api-ts (licence unstated) were not copied.
//!
//! Two doors, one printer:
//! - HTTP :8898, JSON POST with `{"serialNumber","checkCode"}`: `/detail` (telemetry), `/gcodeList`,
//!   `/control`. Needs LAN mode on at the panel; answers `{"code":-2,"message":"Lan mode error"}`
//!   otherwise and `{"code":1,"message":"Access code is different"}` for a wrong (case-sensitive) code.
//! - TCP :8899, `~M601 S1` takes control, `~M602` releases, replies end in `ok`. Works with LAN mode off.
//!   the owner verified `~M104`, `~M83` + `~G1 E`, `~M28/~M29` upload and `~M23` start on his printer.
//!
//! What was never verified there is left off: homing, jogging, fans, speed, G-code console, emergency
//! stop. Bed temperature (`~M140`) is the same class of command as the nozzle's and is on; the live
//! pass confirms it. The 5M has no controller to report homing, and X/Y come from `~M114` when it can.
use crate::{
    config::Config,
    driver::{Action, CameraSource, Capabilities, Driver, StateSnapshot},
    Result,
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Mutex,
    time::{Duration, Instant},
};

const MIN_INTERVAL_S: f64 = 1.0;
const POSITION_EVERY_S: f64 = 2.0;
const RECOVER_EVERY_S: f64 = 60.0;

pub struct FlashForge {
    cfg: Config,
    http_port: u16,
    tcp_port: u16,
    agent: ureq::Agent,
    serial: Option<String>,
    check: Option<String>,
    st: Mutex<St>,
}

#[derive(Default)]
struct St {
    host: String,
    at: f64,
    sample: Option<Value>,
    fails: u32,
    last_recover: f64,
    machine_serial: Option<String>,
    mac: Option<String>,
    /// HTTP said LAN mode is off (or the code is wrong) at this moment; tried again every minute.
    lan_off: Option<(f64, String)>,
    last_file: String,
    pos: Option<([f64; 3], f64)>,
    camera_url: Option<String>,
    note: Option<String>,
}

/// What both doors are turned into before it becomes the object map.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Telemetry {
    pub state: String,
    pub nozzle: Option<f64>,
    pub nozzle_target: Option<f64>,
    pub bed: Option<f64>,
    pub bed_target: Option<f64>,
    /// 0..1.
    pub progress: Option<f64>,
    pub file: String,
    pub duration: Option<f64>,
    pub layer: Option<f64>,
    pub layers: Option<f64>,
    /// Part-cooling fan, 0..1.
    pub fan: Option<f64>,
    /// Speed adjust as a factor (1.0 = 100 %).
    pub speed_factor: Option<f64>,
    pub z_offset: Option<f64>,
    pub position: Option<[f64; 3]>,
    pub error: String,
}

fn num(v: Option<f64>) -> Value {
    v.filter(|n| n.is_finite())
        .map_or(Value::Null, |n| json!(n))
}

/// Print states in the Moonraker words the service records: (print_stats.state, idle state).
/// Both doors' vocabularies are covered, case-insensitively; a word not known is `standby`, never invented.
pub fn map_state(word: &str) -> (&'static str, &'static str) {
    match word.trim().to_ascii_lowercase().as_str() {
        "printing" | "building_from_sd" | "building" | "print" => ("printing", "Printing"),
        "pausing" | "paused" | "pause" => ("paused", "Ready"),
        "completed" | "complete" | "building_completed" | "finished" => ("complete", "Ready"),
        "cancel" | "cancelled" | "canceled" | "stopped" => ("cancelled", "Ready"),
        "error" | "fault" | "faulted" => ("error", "Ready"),
        // Heating, calibrating and busy are the printer working before or between prints.
        "heating" | "calibrate_doing" | "calibrating" | "busy" | "doing" | "leveling" => {
            ("standby", "Printing")
        }
        _ => ("standby", "Idle"),
    }
}

/// The pure part: telemetry into the object map the whole service reads.
pub fn normalise(t: &Telemetry, last_file: &str) -> Value {
    let (print_state, idle_state) = map_state(&t.state);
    let printing = print_state == "printing";
    let file = if !t.file.is_empty() {
        t.file.as_str()
    } else if matches!(print_state, "complete" | "cancelled" | "error" | "paused") {
        last_file
    } else {
        ""
    };
    let [x, y, z] = t
        .position
        .map_or([None, None, None], |p| [Some(p[0]), Some(p[1]), Some(p[2])]);
    let message = if t.error.is_empty() {
        String::new()
    } else {
        format!("The printer reports error {}.", t.error)
    };
    json!({
        "print_stats":{"state":print_state,"filename":file,"total_duration":num(t.duration),"print_duration":num(t.duration),
            "filament_used":Value::Null,"message":message,
            "info":{"total_layer":num(t.layers),"current_layer":num(t.layer)}},
        "virtual_sdcard":{"progress":num(t.progress),"is_active":printing,"file_path":file},
        "idle_timeout":{"state":idle_state},
        "extruder":{"temperature":num(t.nozzle),"target":num(t.nozzle_target),"power":Value::Null},
        "heater_bed":{"temperature":num(t.bed),"target":num(t.bed_target),"power":Value::Null},
        "toolhead":{"position":[num(x),num(y),num(z),Value::Null],"homed_axes":"","max_velocity":Value::Null,"max_accel":Value::Null},
        "motion_report":{"live_position":[num(x),num(y),num(z),Value::Null],"live_velocity":Value::Null,"live_extruder_velocity":Value::Null},
        "gcode_move":{"speed_factor":num(t.speed_factor),"extrude_factor":Value::Null,"speed":Value::Null,
            "homing_origin":[Value::Null,Value::Null,num(t.z_offset),Value::Null]},
        "fan":{"speed":num(t.fan),"rpm":Value::Null},
        "_scope_flashforge":{"state":t.state},
    })
}

/// `/detail`'s object (field names as captured from our printer).
pub fn from_detail(d: &Value) -> Telemetry {
    let f = |k: &str| d[k].as_f64().filter(|n| n.is_finite());
    Telemetry {
        state: d["status"].as_str().unwrap_or("").to_owned(),
        nozzle: f("rightTemp"),
        nozzle_target: f("rightTargetTemp"),
        bed: f("platTemp"),
        bed_target: f("platTargetTemp"),
        // printProgress is a fraction of the job (0.3231 at layer 62 of 240 in the capture).
        progress: f("printProgress").map(|p| p.clamp(0.0, 1.0)),
        file: d["printFileName"].as_str().unwrap_or("").to_owned(),
        duration: f("printDuration"),
        layer: f("printLayer"),
        layers: f("targetPrintLayer"),
        fan: f("coolingFanSpeed").map(|p| (p / 100.0).clamp(0.0, 1.0)),
        speed_factor: f("printSpeedAdjust").map(|p| p / 100.0),
        z_offset: f("zAxisCompensation"),
        position: None,
        error: d["errorCode"].as_str().unwrap_or("").to_owned(),
    }
}

fn after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.find(key).map(|i| text[i + key.len()..].trim_start())
}
fn first_number(s: &str) -> Option<f64> {
    let t: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    t.parse().ok()
}

/// The 8899 replies (formats from our probe.sh regexes and RESEARCH.md): M119 status, M105 temps,
/// M27 progress, M114 position.
pub fn from_tcp(m119: &str, m105: &str, m27: &str, m114: &str) -> Telemetry {
    let word = |t: &str, k: &str| {
        after(t, k)
            .and_then(|r| r.split_whitespace().next())
            .unwrap_or("")
            .to_owned()
    };
    let (nozzle, nozzle_target) = match after(m105, "T0:") {
        Some(r) => {
            let temp = first_number(r);
            let target = r.split('/').nth(1).and_then(first_number);
            (temp, target)
        }
        None => (None, None),
    };
    let (bed, bed_target) = match after(m105, "B:") {
        Some(r) => (first_number(r), r.split('/').nth(1).and_then(first_number)),
        None => (None, None),
    };
    // "SD printing byte 32/100": a percentage out of 100 in the stock firmware's words.
    let progress = after(m27, "SD printing byte").and_then(|r| {
        let (a, b) = r.split_whitespace().next()?.split_once('/')?;
        let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
        (b > 0.0).then(|| (a / b).clamp(0.0, 1.0))
    });
    let (layer, layers) = match after(m27, "Layer:").and_then(|r| r.split_whitespace().next()) {
        Some(s) => match s.split_once('/') {
            Some((a, b)) => (a.parse().ok(), b.parse().ok()),
            None => (None, None),
        },
        None => (None, None),
    };
    let position = {
        let get = |k: &str| after(m114, k).and_then(first_number);
        match (get("X:"), get("Y:"), get("Z:")) {
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => None,
        }
    };
    Telemetry {
        state: word(m119, "MachineStatus:"),
        nozzle,
        nozzle_target,
        bed,
        bed_target,
        progress,
        file: after(m119, "CurrentFile:")
            .map(|r| r.lines().next().unwrap_or("").trim().to_owned())
            .unwrap_or_default(),
        layer,
        layers,
        position,
        ..Telemetry::default()
    }
}

/// A file name the printer will take: our ffsend rule, anything outside [A-Za-z0-9._-] becomes `_`.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn printable(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".gcode") || l.ends_with(".gx") || l.ends_with(".3mf") || l.ends_with(".gcode.3mf")
}

/// `~M661`'s reply: binary framed, names appear as `/data/<name>` (our RESEARCH.md).
pub fn parse_m661(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let marker = b"/data/";
    let mut i = 0;
    while i + marker.len() < bytes.len() {
        if &bytes[i..i + marker.len()] == marker {
            let start = i + marker.len();
            let end = bytes[start..]
                .iter()
                .position(|b| *b == 0 || *b == b':' || *b == b'\r' || *b == b'\n' || *b < 0x20)
                .map_or(bytes.len(), |p| start + p);
            if let Ok(name) = std::str::from_utf8(&bytes[start..end]) {
                if !name.is_empty() && !out.contains(&name.to_owned()) {
                    out.push(name.to_owned());
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

impl FlashForge {
    pub fn new(cfg: &Config) -> Self {
        let rest = cfg
            .moonraker_url
            .trim_end_matches('/')
            .strip_prefix("http://")
            .unwrap_or("")
            .to_owned();
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
                (h.to_owned(), p.parse().unwrap_or(8898))
            }
            _ => (rest.clone(), 8898),
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(cfg.request_timeout_ms.max(500))))
            .max_redirects(0)
            .proxy(None)
            .http_status_as_error(false)
            .max_idle_connections(0)
            .build()
            .into();
        Self {
            http_port: port,
            tcp_port: cfg
                .secret
                .get("tcp_port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(8899),
            serial: cfg.secret.get("serial").map(str::to_owned),
            check: cfg.secret.get("check_code").map(str::to_owned),
            agent,
            st: Mutex::new(St {
                host,
                camera_url: cfg.camera_url.clone(),
                ..St::default()
            }),
            cfg: cfg.clone(),
        }
    }

    fn host(&self) -> String {
        self.st.lock().map(|s| s.host.clone()).unwrap_or_default()
    }

    // ---- HTTP 8898 -----------------------------------------------------------------------------
    fn creds(&self) -> Option<(String, String)> {
        let serial = self
            .serial
            .clone()
            .or_else(|| self.st.lock().ok().and_then(|s| s.machine_serial.clone()))?;
        Some((serial, self.check.clone()?))
    }

    fn http(&self, path: &str, extra: Option<Value>) -> Result<Value> {
        let (serial, check) = self.creds().ok_or("no check code saved for this printer")?;
        let mut body = json!({"serialNumber":serial,"checkCode":check});
        if let (Some(extra), Some(obj)) = (extra, body.as_object_mut()) {
            for (k, v) in extra.as_object().into_iter().flatten() {
                obj.insert(k.clone(), v.clone());
            }
        }
        let url = format!("http://{}:{}{path}", self.host(), self.http_port);
        let mut r = self
            .agent
            .post(&url)
            .content_type("application/json")
            .send(body.to_string().as_bytes())
            .map_err(|e| format!("{} at {url}: {e}", crate::driver::UNREACHABLE_GENERIC))?;
        let mut bytes = Vec::new();
        r.body_mut()
            .as_reader()
            .take(2 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("The printer's answer could not be read: {e}"))?;
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("The printer's answer could not be understood: {e}"))?;
        match v["code"].as_i64() {
            Some(0) => Ok(v),
            Some(-2) => Err("LAN mode is off on the printer's screen; turn it on there for live data (the printer can still be read without it)".into()),
            Some(1) => Err("the printer says the check code is different; it is case sensitive and shown on the panel".into()),
            Some(c) => Err(format!("The printer answered with an error ({c}): {}", v["message"].as_str().unwrap_or(""))),
            None => Err("The printer's answer had no status in it".into()),
        }
    }

    // ---- TCP 8899 ------------------------------------------------------------------------------
    /// A short control session: take control, run the commands, release. Each reply is read until its
    /// closing `ok` (or the read times out). One session at a time is a printer property: held briefly.
    pub fn tcp(&self, commands: &[String]) -> Result<Vec<Vec<u8>>> {
        let host = self.host();
        let addr = (host.as_str(), self.tcp_port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .ok_or_else(|| {
                format!(
                    "{} (the address {host} does not resolve)",
                    crate::driver::UNREACHABLE_GENERIC
                )
            })?;
        let timeout = Duration::from_millis(self.cfg.request_timeout_ms.max(500));
        let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
            format!(
                "{} at {host}:{}: {e}",
                crate::driver::UNREACHABLE_GENERIC,
                self.tcp_port
            )
        })?;
        let _ = s.set_read_timeout(Some(timeout));
        let _ = s.set_write_timeout(Some(timeout));
        let run = |s: &mut TcpStream, c: &str| -> Result<Vec<u8>> {
            s.write_all(format!("{c}\r\n").as_bytes())
                .map_err(|e| format!("printer write failed: {e}"))?;
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 4096];
            let deadline = Instant::now() + timeout;
            loop {
                match s.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        let tail = &buf[buf.len().saturating_sub(16)..];
                        if tail.windows(2).any(|w| w == b"ok") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
                if Instant::now() > deadline {
                    break;
                }
            }
            Ok(buf)
        };
        let mut out = Vec::new();
        let taken = run(&mut s, "~M601 S1")?;
        if !String::from_utf8_lossy(&taken)
            .to_ascii_lowercase()
            .contains("control success")
        {
            let _ = run(&mut s, "~M602");
            return Err("the printer would not give control (another program may be connected); try again in a moment".into());
        }
        for c in commands {
            match run(&mut s, c) {
                Ok(r) => out.push(r),
                Err(e) => {
                    let _ = run(&mut s, "~M602");
                    return Err(e);
                }
            }
        }
        let _ = run(&mut s, "~M602");
        Ok(out)
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    /// `~M115`: who this is. Cached; also what address recovery matches against.
    fn identify(&self) -> Result<(String, String)> {
        let r = self.tcp(&["~M115".into()])?;
        let t = Self::text(&r[0]);
        let serial = after(&t, "SN:")
            .and_then(|r| r.split_whitespace().next())
            .ok_or("the printer did not give its serial number")?
            .to_owned();
        let mac = after(&t, "Mac Address:")
            .and_then(|r| r.split_whitespace().next())
            .unwrap_or("")
            .to_owned();
        if let Ok(mut s) = self.st.lock() {
            s.machine_serial = Some(serial.clone());
            s.mac = Some(mac.clone()).filter(|m| !m.is_empty());
        }
        Ok((serial, mac))
    }

    /// The address stopped answering. Find the printer again, by serial: first the neighbour table
    /// (ARP) entry with its MAC, then a bounded sweep of the /24 for hosts with 8899 open that answer
    /// `~M115` with the same serial. Returns the new address.
    fn recover(&self) -> Option<String> {
        let (serial, mac, old) = {
            let s = self.st.lock().ok()?;
            (
                s.machine_serial.clone().or_else(|| self.serial.clone())?,
                s.mac.clone(),
                s.host.clone(),
            )
        };
        let mut candidates: Vec<String> = Vec::new();
        if let (Some(mac), Ok(arp)) = (mac.as_deref(), std::fs::read_to_string("/proc/net/arp")) {
            for line in arp.lines().skip(1) {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() >= 4 && f[3].eq_ignore_ascii_case(mac) && f[0] != old {
                    candidates.push(f[0].to_owned());
                }
            }
        }
        if candidates.is_empty() {
            if let Ok(ip) = old.parse::<std::net::Ipv4Addr>() {
                let [a, b, c, _] = ip.octets();
                let port = self.tcp_port;
                let open: Vec<String> = std::thread::scope(|scope| {
                    let handles: Vec<_> = (1..=254_u8)
                        .collect::<Vec<_>>()
                        .chunks(16)
                        .map(|chunk| {
                            let chunk = chunk.to_vec();
                            scope.spawn(move || {
                                chunk
                                    .into_iter()
                                    .filter_map(|d| {
                                        let ip = format!("{a}.{b}.{c}.{d}");
                                        let addr =
                                            (ip.as_str(), port).to_socket_addrs().ok()?.next()?;
                                        TcpStream::connect_timeout(
                                            &addr,
                                            Duration::from_millis(250),
                                        )
                                        .ok()
                                        .map(|_| ip)
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .flat_map(|h| h.join().unwrap_or_default())
                        .collect()
                });
                candidates.extend(open.into_iter().filter(|ip| *ip != old));
            }
        }
        for ip in candidates {
            let probe = FlashForge {
                cfg: self.cfg.clone(),
                http_port: self.http_port,
                tcp_port: self.tcp_port,
                agent: self.agent.clone(),
                serial: self.serial.clone(),
                check: self.check.clone(),
                st: Mutex::new(St {
                    host: ip.clone(),
                    ..St::default()
                }),
            };
            if let Ok((found, _)) = probe.identify() {
                if found == serial {
                    if let Ok(mut s) = self.st.lock() {
                        s.host = ip.clone();
                        s.note = Some(format!("Found again at {ip} (it was {old}); the address in settings is the old one."));
                    }
                    return Some(ip);
                }
            }
        }
        None
    }
}

impl Driver for FlashForge {
    /// WO-14 item 4: `~M115` carries the build volume (`X: 220 Y: 220 Z: 220` on the Adventurer
    /// 5M). Read-only; the printer says its own size, so no model table is needed here.
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        let r = self.tcp(&["~M115".into()]).ok()?;
        let t = Self::text(&r[0]);
        let x = after(&t, "X:").and_then(first_number);
        let y = after(&t, "Y:").and_then(first_number);
        match (x, y) {
            (Some(x), Some(y)) if x > 0.0 && y > 0.0 => Some([x, y]),
            _ => None,
        }
    }
    /// M115 names the machine ("Machine Type: Flashforge Adventurer 5M"); the brand is said apart.
    fn model_hint(&mut self) -> Option<String> {
        let r = self.tcp(&["~M115".into()]).ok()?;
        let t = Self::text(&r[0]);
        let line = t
            .lines()
            .find_map(|l| l.trim().strip_prefix("Machine Type:"))?
            .trim();
        let m = line
            .strip_prefix("Flashforge ")
            .or_else(|| line.strip_prefix("FlashForge "))
            .unwrap_or(line)
            .trim();
        (!m.is_empty() && m.len() <= 40).then(|| m.to_owned())
    }
    fn kind(&self) -> &'static str {
        "flashforge"
    }
    fn brand(&self) -> &'static str {
        "FlashForge"
    }
    fn capabilities(&self) -> Capabilities {
        // Only what the owner verified on his printer, plus the bed (same command class), and honest per door.
        Capabilities {
            heat: true,
            extrude: true,
            pause_resume: true,
            cancel: true,
            start_print: true,
            upload: true,
            camera: self.cfg.camera_url.is_some()
                || self
                    .st
                    .lock()
                    .map(|s| s.camera_url.is_some())
                    .unwrap_or(false),
            live_xy: true,
            xy: true,
            ..Capabilities::none()
        }
    }

    fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)> {
        {
            let s = self.st.lock().map_err(|_| "driver state poisoned")?;
            if let (Some(v), true) = (&s.sample, ts >= s.at && ts - s.at < MIN_INTERVAL_S) {
                return Ok((v.clone(), Vec::new()));
            }
        }
        let mut warnings = Vec::new();
        let http_wanted = self.creds().is_some()
            && self
                .st
                .lock()
                .map(|s| s.lan_off.as_ref().is_none_or(|(at, _)| ts - at > 60.0))
                .unwrap_or(true);
        let mut telemetry: Option<Telemetry> = None;
        let mut camera: Option<String> = None;
        let mut failure: Option<String> = None;
        if http_wanted {
            match self.http("/detail", None) {
                Ok(v) => {
                    let d = &v["detail"];
                    telemetry = Some(from_detail(d));
                    if let Some(u) = d["cameraStreamUrl"]
                        .as_str()
                        .filter(|u| u.starts_with("http://"))
                    {
                        camera = Some(u.to_owned());
                    }
                    if let (Ok(mut s), Some(sn)) = (self.st.lock(), d["name"].as_str()) {
                        s.machine_serial.get_or_insert_with(|| sn.to_owned());
                        s.lan_off = None;
                    }
                }
                Err(e) if e.contains("LAN mode") || e.contains("check code") => {
                    warnings.push(format!("Live data is limited: {e}; the printer is still read"));
                    if let Ok(mut s) = self.st.lock() {
                        s.lan_off = Some((ts, e));
                    }
                }
                Err(e) => failure = Some(e),
            }
        }
        // Live X/Y/Z only come from `~M114`, at most every couple of seconds.
        let want_position = self
            .st
            .lock()
            .map(|s| s.pos.is_none_or(|(_, at)| ts - at >= POSITION_EVERY_S))
            .unwrap_or(true);
        if telemetry.is_none() || want_position {
            let mut cmds: Vec<String> = Vec::new();
            if telemetry.is_none() {
                cmds.extend(["~M119", "~M105", "~M27"].map(String::from));
            }
            cmds.push("~M114".into());
            match self.tcp(&cmds) {
                Ok(r) => {
                    let t = |i: usize| r.get(i).map(|b| Self::text(b)).unwrap_or_default();
                    let from_tcp_full = telemetry.is_none();
                    let at = if from_tcp_full { 3 } else { 0 };
                    let pos = from_tcp("", "", "", &t(at)).position;
                    if from_tcp_full {
                        let mut tm = from_tcp(&t(0), &t(1), &t(2), "");
                        tm.position = pos;
                        telemetry = Some(tm);
                        failure = None;
                    } else if let Some(tm) = telemetry.as_mut() {
                        tm.position = pos;
                    }
                    if let (Ok(mut s), Some(p)) = (self.st.lock(), pos) {
                        s.pos = Some((p, ts));
                    }
                }
                Err(e) => {
                    if telemetry.is_none() {
                        failure = Some(failure.unwrap_or(e));
                    }
                }
            }
        }
        let Some(mut t) = telemetry else {
            let (fails, since) = {
                let mut s = self.st.lock().map_err(|_| "driver state poisoned")?;
                s.fails += 1;
                (s.fails, ts - s.last_recover)
            };
            if fails >= 2 && since >= RECOVER_EVERY_S {
                if let Ok(mut s) = self.st.lock() {
                    s.last_recover = ts;
                }
                if let Some(ip) = self.recover() {
                    if let Ok(mut s) = self.st.lock() {
                        s.fails = 0;
                    }
                    return self.sample(ts + 0.001).map(|(v, mut w)| {
                        w.push(format!(
                            "The printer was not at its saved address; found again at {ip}."
                        ));
                        (v, w)
                    });
                }
            }
            return Err(failure.unwrap_or_else(|| {
                format!("{} at {}", crate::driver::UNREACHABLE_GENERIC, self.host())
            }));
        };
        let mut s = self.st.lock().map_err(|_| "driver state poisoned")?;
        s.fails = 0;
        if let Some(u) = camera {
            s.camera_url = Some(u);
        }
        if let Some((p, at)) = s.pos {
            if t.position.is_none() && ts - at < 10.0 {
                t.position = Some(p);
            }
        }
        // While printing the head moves faster than a read a couple of seconds old: unknown, not stale.
        if map_state(&t.state).0 == "printing" && s.pos.is_none_or(|(_, at)| ts - at > 3.0) {
            t.position = None;
        }
        if !t.file.is_empty() {
            s.last_file = t.file.clone();
        }
        if let Some(n) = s.note.take() {
            warnings.push(n);
        }
        let sample = normalise(&t, &s.last_file);
        s.at = ts;
        s.sample = Some(sample.clone());
        Ok((sample, warnings))
    }

    fn files(&self) -> Result<Vec<Value>> {
        let names: Vec<String> = match self.http("/gcodeList", None) {
            Ok(v) => v["gcodeList"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            x.as_str()
                                .or_else(|| x["gcodeFileName"].as_str())
                                .map(|s| s.trim_start_matches("/data/").to_owned())
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => {
                let r = self.tcp(&["~M661".into()])?;
                parse_m661(&r[0])
            }
        };
        let mut rows: Vec<Value> = names
            .into_iter()
            .filter(|n| printable(n))
            .map(|n| json!({"path":n,"size":Value::Null,"modified":Value::Null}))
            .collect();
        rows.truncate(200);
        Ok(rows)
    }

    fn cameras(&self) -> Result<Vec<CameraSource>> {
        let mut url = self.st.lock().ok().and_then(|s| s.camera_url.clone());
        if url.is_none() && self.creds().is_some() {
            // Not seen yet: ask the printer. The non-Pro 5M answers an empty address (no camera).
            if let Ok(v) = self.http("/detail", None) {
                url = v["detail"]["cameraStreamUrl"]
                    .as_str()
                    .filter(|u| u.starts_with("http://"))
                    .map(str::to_owned);
                if let (Ok(mut s), Some(u)) = (self.st.lock(), url.clone()) {
                    s.camera_url = Some(u);
                }
            }
        }
        Ok(url
            .into_iter()
            .map(|u| CameraSource {
                name: "Camera".into(),
                kind: if u.starts_with("rtsp://") {
                    "rtsp"
                } else {
                    "mjpeg"
                },
                url: u,
            })
            .collect())
    }

    fn snapshot(&self) -> Result<StateSnapshot> {
        let t = match self.http("/detail", None) {
            Ok(v) => from_detail(&v["detail"]),
            Err(_) => {
                let r = self.tcp(&["~M119".into()])?;
                from_tcp(&Self::text(&r[0]), "", "", "")
            }
        };
        let (print_state, idle_state) = map_state(&t.state);
        Ok(StateSnapshot {
            state: if print_state == "error" {
                "error".into()
            } else {
                "ready".into()
            },
            homed_axes: String::new(),
            print_state: print_state.to_owned(),
            idle_state: idle_state.to_ascii_lowercase(),
            help: Default::default(),
        })
    }

    fn act(&self, action: &Action) -> Result<Value> {
        let ok = |replies: Vec<Vec<u8>>, what: &str| -> Result<Value> {
            let all: String = replies
                .iter()
                .map(|r| Self::text(r))
                .collect::<Vec<_>>()
                .join("\n");
            if all.to_ascii_lowercase().contains("error") && !all.contains("ok") {
                return Err(format!("the printer did not accept {what}: {}", all.trim()));
            }
            Ok(json!({"result":"ok"}))
        };
        let job_cmd = |verb: &str| -> Result<Value> {
            self.http(
                "/control",
                Some(json!({"payload":{"cmd":"jobCtl_cmd","args":{"jobID":"","action":verb}}})),
            )
            .map(|_| json!({"result":"ok"}))
        };
        match action {
            Action::SetTemperature { heater, celsius } => {
                let code = if *heater == crate::driver::Heater::Nozzle {
                    "~M104"
                } else {
                    "~M140"
                };
                ok(
                    self.tcp(&[format!("{code} S{celsius:.0}")])?,
                    "that temperature",
                )
            }
            Action::Extrude { mm, mm_per_s } => ok(
                self.tcp(&[
                    "~M83".into(),
                    format!("~G1 E{mm:.2} F{:.0}", mm_per_s * 60.0),
                ])?,
                "that extrusion",
            ),
            Action::Pause => job_cmd("pause").or_else(|_| ok(self.tcp(&["~M25".into()])?, "pause")),
            Action::Resume => {
                job_cmd("continue").or_else(|_| ok(self.tcp(&["~M24".into()])?, "resume"))
            }
            Action::Cancel => {
                job_cmd("cancel").or_else(|_| ok(self.tcp(&["~M26".into()])?, "cancel"))
            }
            Action::StartPrint { filename } => {
                if !printable(filename) || filename.contains(['/', '\\', '\r', '\n']) {
                    return Err("name a file from the printer's own list".into());
                }
                // F05: the HTTP door starts a file (`/printGcode`); `~M23` over TCP answers "File selected" and on 5.1.9
                // does not start it. TCP stays the way when LAN mode is off.
                self.http("/printGcode", Some(json!({"fileName":sanitize(filename),"levelingBeforePrint":false})))
                    .map(|_| json!({"result":"ok"}))
                    .or_else(|_| ok(self.tcp(&[format!("~M23 0:/user/{}", sanitize(filename))])?, "that file"))
            }
            other => Err(format!(
                "the stock FlashForge firmware has no verified way to do {other:?}"
            )),
        }
    }

    fn upload(&self, name: &str, bytes: &[u8], start: bool) -> Result<Value> {
        if !printable(name) || name.contains(['/', '\\', '\r', '\n']) || name.contains("..") {
            return Err("the file name must be a plain .gcode name".into());
        }
        let clean = sanitize(name);
        // Finish-2 F05 (2026-10-19's AD5M on firmware 5.1.9): `~M28`/`~M29` over TCP answers "Done saving file." but
        // saves an empty file ("File opened: ... Size: 0"), so a dry run was reported started while the printer stayed
        // ready. The HTTP door's `/uploadGcode` (multipart, the check code in headers) saves the file whole and can start
        // it; it is the only upload used.
        let (serial, check) = self.creds().ok_or("no check code saved for this printer")?;
        let boundary = "----stella-scope-upload-7f3a9c";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"gcodeFile\"; filename=\"{clean}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = format!("http://{}:{}/uploadGcode", self.host(), self.http_port);
        let mut r = self
            .agent
            .post(&url)
            // A whole file over 2.4 GHz Wi-Fi takes far longer than a status read: three minutes, as PrusaLink transfers get.
            .config()
            .timeout_global(Some(Duration::from_secs(180)))
            .build()
            .header("serialNumber", &serial)
            .header("checkCode", &check)
            .header("fileSize", &bytes.len().to_string())
            .header("printNow", if start { "true" } else { "false" })
            .header("levelingBeforePrint", "false")
            .content_type(&format!("multipart/form-data; boundary={boundary}"))
            .send(&body[..])
            .map_err(|e| format!("the file could not be sent: {e}"))?;
        let mut reply = Vec::new();
        let _ = r.body_mut().as_reader().take(64 * 1024).read_to_end(&mut reply);
        let v: Value = serde_json::from_slice(&reply).unwrap_or(Value::Null);
        match v["code"].as_i64() {
            Some(0) => Ok(json!({"result":"ok","name":clean,"started":start})),
            Some(-2) => Err("LAN mode is off on the printer's panel; turn it on to send files".into()),
            _ => Err(format!("the printer did not save the file: {}", String::from_utf8_lossy(&reply).trim())),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// our printer, 2026-08-30, firmware 5.1.7: the `detail` object as captured (serial, MAC and name
    /// replaced by placeholders; every number and word as it was).
    fn captured_detail() -> Value {
        serde_json::from_str(r#"{"autoShutdown":"close","autoShutdownTime":5,"cameraStreamUrl":"","chamberFanSpeed":100,"chamberTargetTemp":0,
            "chamberTemp":0,"coolingFanSpeed":100,"cumulativeFilament":598.864013671875,"cumulativePrintTime":7858,
            "currentPrintSpeed":122,"doorStatus":"close","errorCode":"","estimatedLeftLen":0,"estimatedLeftWeight":0.0,
            "estimatedRightLen":1524.3345947265625,"estimatedRightWeight":0.0,"estimatedTime":0,"externalFanStatus":"close",
            "fillAmount":0,"firmwareVersion":"5.1.7","internalFanStatus":"close","ipAddr":"192.168.1.126","leftFilamentType":"",
            "leftTargetTemp":0,"leftTemp":0,"lightStatus":"open","macAddr":"AA:BB:CC:DD:EE:FF","measure":"220X220X220",
            "name":"SNXXXXXXXXXXXX","nozzleCnt":1,"nozzleModel":"0.4mm","platTargetTemp":60.0,"platTemp":59.61000061035156,
            "printDuration":698,"printFileName":"3DBenchy_PLA_35m59s.gcode.3mf","printFileThumbUrl":"http://192.168.1.126:8898/getThum",
            "printLayer":62,"printProgress":0.3231707513332367,"printSpeedAdjust":100,"remainingDiskSpace":4.927837371826172,
            "rightFilamentType":"PLA","rightTargetTemp":220.0,"rightTemp":220.10000610351562,"status":"printing",
            "targetPrintLayer":240,"tvoc":0,"zAxisCompensation":0.05000000074505806}"#).unwrap()
    }

    #[test]
    fn the_bed_size_comes_from_m115() {
        // WO-14 item 4: the machine's own M115 X/Y, read-only, over the TCP session the driver
        // already holds for identification. The fake answers with the real 5M reply's wording.
        let f = crate::fakes::FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
        let mut cfg = crate::config::Config::default();
        cfg.moonraker_url = f.http_url();
        cfg.secret
            .0
            .insert("tcp_port".to_owned(), f.tcp_port().to_string());
        let mut d = FlashForge::new(&cfg);
        assert_eq!(
            crate::driver::Driver::bed_hint(&mut d),
            Some([220.0, 220.0])
        );
    }

    #[test]
    fn the_captured_detail_becomes_the_object_map_it_should() {
        let t = from_detail(&captured_detail());
        assert_eq!(t.state, "printing");
        assert!((t.progress.unwrap() - 0.3231707513).abs() < 1e-9);
        assert_eq!(t.layer, Some(62.0));
        assert_eq!(t.layers, Some(240.0));
        let o = normalise(&t, "");
        assert_eq!(o["print_stats"]["state"], "printing");
        assert_eq!(
            o["print_stats"]["filename"],
            "3DBenchy_PLA_35m59s.gcode.3mf"
        );
        assert_eq!(o["print_stats"]["print_duration"], 698.0);
        assert_eq!(o["print_stats"]["info"]["current_layer"], 62.0);
        assert!((o["extruder"]["temperature"].as_f64().unwrap() - 220.1000061).abs() < 1e-6);
        assert_eq!(o["extruder"]["target"], 220.0);
        assert_eq!(o["heater_bed"]["target"], 60.0);
        assert_eq!(o["fan"]["speed"], 1.0);
        assert_eq!(o["gcode_move"]["speed_factor"], 1.0);
        assert!(
            (o["gcode_move"]["homing_origin"][2].as_f64().unwrap() - 0.0500000007).abs() < 1e-8
        );
        // Not in /detail for this job (the lifetime filament figure is not the job's): unknown.
        assert!(o["print_stats"]["filament_used"].is_null());
        assert!(o["toolhead"]["position"][0].is_null());
        assert!(o["extruder"]["power"].is_null());
    }

    #[test]
    fn the_tcp_replies_parse_the_way_kens_probe_reads_them() {
        let m119 = "CMD M119 Received.\r\nEndstop: X-max:0 Y-max:0 Z-max:0\r\nMachineStatus: BUILDING_FROM_SD\r\nMoveMode: MOVING\r\nStatus: S:1 L:0 J:0 F:0\r\nLED: 1\r\nCurrentFile: 3DBenchy_PLA_35m59s.gcode.3mf\r\nok\r\n";
        let m105 = "CMD M105 Received.\r\nT0:220.1 /220.0 B:59.6 /60.0\r\nok\r\n";
        let m27 = "CMD M27 Received.\r\nSD printing byte 32/100\r\nLayer: 62/240\r\nok\r\n";
        let m114 = "CMD M114 Received.\r\nX:110.250 Y:98.000 Z:12.400 A:200.000 B:0.000\r\nok\r\n";
        let t = from_tcp(m119, m105, m27, m114);
        assert_eq!(t.state, "BUILDING_FROM_SD");
        assert_eq!(map_state(&t.state).0, "printing");
        assert_eq!(
            (t.nozzle, t.nozzle_target, t.bed, t.bed_target),
            (Some(220.1), Some(220.0), Some(59.6), Some(60.0))
        );
        assert_eq!(t.progress, Some(0.32));
        assert_eq!((t.layer, t.layers), (Some(62.0), Some(240.0)));
        assert_eq!(t.position, Some([110.25, 98.0, 12.4]));
        assert_eq!(t.file, "3DBenchy_PLA_35m59s.gcode.3mf");
        // A reply that is missing is unknown, not zero.
        let none = from_tcp("", "", "", "");
        assert_eq!(
            (none.nozzle, none.progress, none.position),
            (None, None, None)
        );
    }

    #[test]
    fn every_status_word_either_door_says_maps_to_a_print_state() {
        for (word, want) in [
            ("ready", "standby"),
            ("READY", "standby"),
            ("printing", "printing"),
            ("BUILDING_FROM_SD", "printing"),
            ("heating", "standby"),
            ("calibrate_doing", "standby"),
            ("pausing", "paused"),
            ("paused", "paused"),
            ("PAUSED", "paused"),
            ("completed", "complete"),
            ("BUILDING_COMPLETED", "complete"),
            ("cancel", "cancelled"),
            ("error", "error"),
            ("something_new", "standby"),
            ("", "standby"),
        ] {
            assert_eq!(map_state(word).0, want, "{word}");
        }
        assert_eq!(
            map_state("heating").1,
            "Printing",
            "heating is the printer working"
        );
    }

    #[test]
    fn file_names_are_made_safe_the_way_ken_s_own_sender_does_it() {
        assert_eq!(
            sanitize("bracket v2 (final).gcode"),
            "bracket_v2__final_.gcode"
        );
        assert_eq!(sanitize("../../etc/passwd"), ".._.._etc_passwd");
        let list = b"\x00\x00\x01/data/cube_20mm.gcode\x00\x00/data/Benchy.gcode.3mf:\x01junk/data/cube_20mm.gcode\x00";
        assert_eq!(
            parse_m661(list),
            vec!["cube_20mm.gcode".to_owned(), "Benchy.gcode.3mf".to_owned()]
        );
    }

    // ---- against the fake printer (src/fakes.rs): HTTP 8898 and TCP 8899 -----------------------

    use crate::fakes::FakeFlashForge;

    fn driver(f: &FakeFlashForge, with_code: bool) -> FlashForge {
        let mut cfg = Config {
            driver: "flashforge".into(),
            moonraker_url: f.http_url(),
            request_timeout_ms: 1500,
            ..Config::default()
        };
        let mut m = std::collections::BTreeMap::new();
        m.insert("tcp_port".to_owned(), f.tcp_port().to_string());
        m.insert("serial".to_owned(), "SNFAKE0000001".to_owned());
        if with_code {
            m.insert("check_code".to_owned(), "0fake000".to_owned());
        }
        cfg.secret = crate::config::Secret(m);
        FlashForge::new(&cfg)
    }
    fn fake() -> FakeFlashForge {
        let f = FakeFlashForge::start(0, 0, "SNFAKE0000001", "0fake000");
        let mut s = f.state.lock().unwrap();
        s.status = "printing".into();
        s.machine_status = "BUILDING_FROM_SD".into();
        s.file = "Benchy_PLA.gcode.3mf".into();
        s.progress = 0.32;
        s.layer = 62;
        s.layers = 240;
        s.nozzle = 219.5;
        s.nozzle_target = 220.0;
        s.bed = 59.8;
        s.bed_target = 60.0;
        drop(s);
        f
    }

    #[test]
    fn http_gives_the_telemetry_and_the_position_comes_from_tcp() {
        let f = fake();
        let mut d = driver(&f, true);
        let (o, w) = d.sample(1000.0).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(o["print_stats"]["state"], "printing");
        assert_eq!(o["print_stats"]["filename"], "Benchy_PLA.gcode.3mf");
        assert_eq!(o["virtual_sdcard"]["progress"], 0.32);
        assert_eq!(o["extruder"]["temperature"], 219.5);
        // /detail has no XYZ; ~M114 does, and the head is moving, so a fresh read is shown.
        assert_eq!(o["toolhead"]["position"], json!([110.0, 110.0, 5.0, null]));
        let log = f.state.lock().unwrap().tcp_log.clone();
        assert_eq!(
            log,
            vec!["~M601 S1", "~M114", "~M602"],
            "one short session, released"
        );
        assert!(!f.state.lock().unwrap().controlled);
    }

    #[test]
    fn with_lan_mode_off_the_tcp_door_carries_everything() {
        let f = fake();
        f.state.lock().unwrap().lan_mode = false;
        let mut d = driver(&f, true);
        let (o, w) = d.sample(1000.0).unwrap();
        assert!(w.iter().any(|m| m.contains("LAN mode")), "{w:?}");
        assert_eq!(o["print_stats"]["state"], "printing");
        assert_eq!(o["print_stats"]["filename"], "Benchy_PLA.gcode.3mf");
        assert_eq!(o["virtual_sdcard"]["progress"], 0.32);
        assert_eq!(o["heater_bed"]["target"], 60.0);
        assert_eq!(o["print_stats"]["info"]["current_layer"], 62.0);
        assert_eq!(o["toolhead"]["position"][2], 5.0);
        // No check code at all: the same, with no HTTP attempt and no warning.
        let f2 = fake();
        let mut d2 = driver(&f2, false);
        let (_, w) = d2.sample(1000.0).unwrap();
        assert!(w.is_empty());
        assert!(f2.state.lock().unwrap().http_log.is_empty());
    }

    #[test]
    fn a_check_code_in_the_wrong_case_is_refused_in_words() {
        let f = fake();
        let mut cfg = Config {
            driver: "flashforge".into(),
            moonraker_url: f.http_url(),
            request_timeout_ms: 1500,
            ..Config::default()
        };
        let m: std::collections::BTreeMap<String, String> = [
            ("tcp_port", f.tcp_port().to_string()),
            ("serial", "SNFAKE0000001".into()),
            ("check_code", "0FAKE000".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        cfg.secret = crate::config::Secret(m);
        let d = FlashForge::new(&cfg);
        let e = d.http("/detail", None).unwrap_err();
        assert!(e.contains("case sensitive"), "{e}");
        assert!(!e.contains("0FAKE000") && !e.contains("0fake000"));
    }

    #[test]
    fn actions_send_exactly_these_lines_and_bodies() {
        let f = fake();
        let d = driver(&f, true);
        use crate::driver::Heater;
        d.act(&Action::SetTemperature {
            heater: Heater::Nozzle,
            celsius: 215.0,
        })
        .unwrap();
        d.act(&Action::SetTemperature {
            heater: Heater::Bed,
            celsius: 60.0,
        })
        .unwrap();
        d.act(&Action::Extrude {
            mm: 5.0,
            mm_per_s: 3.0,
        })
        .unwrap();
        d.act(&Action::Pause).unwrap();
        assert_eq!(f.state.lock().unwrap().status, "paused");
        d.act(&Action::Resume).unwrap();
        assert_eq!(f.state.lock().unwrap().status, "printing");
        d.act(&Action::Cancel).unwrap();
        assert_eq!(f.state.lock().unwrap().status, "cancel");
        d.act(&Action::StartPrint {
            filename: "cube 20mm.gcode".into(),
        })
        .unwrap();
        let tcp: Vec<String> = f
            .state
            .lock()
            .unwrap()
            .tcp_log
            .iter()
            .filter(|l| !l.starts_with("~M60"))
            .cloned()
            .collect();
        assert_eq!(
            tcp,
            vec![
                "~M104 S215",
                "~M140 S60",
                "~M83",
                "~G1 E5.00 F180",
                "~M23 0:/user/cube_20mm.gcode"
            ]
        );
        let controls: Vec<Value> = f
            .state
            .lock()
            .unwrap()
            .http_log
            .iter()
            .filter(|e| e["path"] == "/control")
            .map(|e| e["body"]["payload"].clone())
            .collect();
        assert_eq!(
            controls,
            vec![
                json!({"cmd":"jobCtl_cmd","args":{"jobID":"","action":"pause"}}),
                json!({"cmd":"jobCtl_cmd","args":{"jobID":"","action":"continue"}}),
                json!({"cmd":"jobCtl_cmd","args":{"jobID":"","action":"cancel"}}),
            ]
        );
        // Names are checked; things nobody verified on this printer are refused, not guessed.
        assert!(d
            .act(&Action::StartPrint {
                filename: "../x.gcode".into()
            })
            .is_err());
        assert!(d
            .act(&Action::StartPrint {
                filename: "notes.txt".into()
            })
            .is_err());
        assert!(d.act(&Action::Home).is_err());
        assert!(d.act(&Action::Jog { axis: 'X', mm: 5.0 }).is_err());
        assert!(d.act(&Action::Gcode("M112".into())).is_err());
    }

    #[test]
    fn pause_falls_back_to_the_tcp_codes_when_http_is_off() {
        let f = fake();
        f.state.lock().unwrap().lan_mode = false;
        let d = driver(&f, true);
        d.act(&Action::Pause).unwrap();
        d.act(&Action::Resume).unwrap();
        d.act(&Action::Cancel).unwrap();
        let tcp: Vec<String> = f
            .state
            .lock()
            .unwrap()
            .tcp_log
            .iter()
            .filter(|l| !l.starts_with("~M60"))
            .cloned()
            .collect();
        assert_eq!(tcp, vec!["~M25", "~M24", "~M26"]);
    }

    #[test]
    /// Finish-2 F05: the upload goes through the HTTP door (TCP `~M28` saved an empty file on our 5.1.9), whole, and
    /// starts the file only when asked; a start of a file already there uses `/printGcode`.
    fn upload_goes_over_http_whole_and_starts_only_when_asked() {
        let f = fake();
        let d = driver(&f, true);
        let data: Vec<u8> = (0..10_000_u32).map(|i| (i % 251) as u8).collect();
        d.upload("plate one.gcode", &data, false).unwrap();
        {
            let s = f.state.lock().unwrap();
            assert_eq!(s.uploads, vec![("plate_one.gcode".to_owned(), 10_000)]);
            assert!(s.files.contains(&"plate_one.gcode".to_owned()));
            assert_ne!(s.file, "plate_one.gcode", "printNow false does not start it");
            assert!(s.tcp_log.iter().all(|l| !l.starts_with("~M28")), "no TCP upload: {:?}", s.tcp_log);
        }
        d.act(&Action::StartPrint { filename: "plate one.gcode".into() }).unwrap();
        let s = f.state.lock().unwrap();
        assert_eq!((s.status.as_str(), s.file.as_str()), ("printing", "plate_one.gcode"));
        drop(s);
        let f2 = fake();
        let d2 = driver(&f2, true);
        d2.upload("cube.gcode", b"G1 X1\n", true).unwrap();
        assert_eq!(f2.state.lock().unwrap().file, "cube.gcode", "printNow true starts it");
        assert!(d.upload("../x.gcode", b"", false).is_err());
        assert!(d.upload("notes.txt", b"", false).is_err());
    }

    #[test]
    fn files_come_from_http_or_from_the_tcp_listing() {
        let f = fake();
        let d = driver(&f, true);
        let names: Vec<String> = d
            .files()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, vec!["cube_20mm_PLA.gcode", "Benchy_PLA.gcode.3mf"]);
        f.state.lock().unwrap().lan_mode = false;
        let names: Vec<String> = d
            .files()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            vec!["cube_20mm_PLA.gcode", "Benchy_PLA.gcode.3mf"],
            "the binary ~M661 list reads the same"
        );
    }

    /// The printer has no static address. When the saved one stops answering the driver finds it again
    /// by serial number: here the saved address is 127.0.0.77 (nothing there) and the printer is at
    /// 127.0.0.1; the sweep of that /24 finds the open port and ~M115 names the serial.
    #[test]
    fn the_printer_is_found_again_by_serial_when_its_address_moves() {
        let f = fake();
        let mut cfg = Config {
            driver: "flashforge".into(),
            moonraker_url: format!(
                "http://127.0.0.77:{}",
                f.http_addr.rsplit(':').next().unwrap()
            ),
            request_timeout_ms: 600,
            ..Config::default()
        };
        let m: std::collections::BTreeMap<String, String> = [
            ("tcp_port", f.tcp_port().to_string()),
            ("serial", "SNFAKE0000001".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        cfg.secret = crate::config::Secret(m);
        let mut d = FlashForge::new(&cfg);
        assert!(d.sample(1000.0).is_err(), "the first miss is only a miss");
        let (o, w) = d.sample(1010.0).unwrap();
        assert_eq!(o["print_stats"]["state"], "printing");
        assert!(
            w.iter().any(|m| m.contains("found again at 127.0.0.1")),
            "{w:?}"
        );
        assert_eq!(d.host(), "127.0.0.1");
        // A printer with a different serial at that address is not mistaken for it.
        let other = FakeFlashForge::start(0, 0, "SNSOMEONEELSE", "x");
        let mut cfg2 = cfg.clone();
        cfg2.secret = crate::config::Secret(
            [
                ("tcp_port", other.tcp_port().to_string()),
                ("serial", "SNFAKE0000001".into()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        );
        cfg2.moonraker_url = format!(
            "http://127.0.0.77:{}",
            other.http_addr.rsplit(':').next().unwrap()
        );
        let mut d2 = FlashForge::new(&cfg2);
        let _ = d2.sample(1000.0);
        assert!(d2.sample(1010.0).is_err());
        assert_eq!(d2.host(), "127.0.0.77");
    }

    #[test]
    fn capabilities_are_the_verified_ones() {
        let d = FlashForge::new(&Config {
            driver: "flashforge".into(),
            moonraker_url: "http://192.168.1.126:8898".into(),
            ..Config::default()
        });
        let c = d.capabilities();
        for (id, want) in [
            ("set_heater", true),
            ("extrude", true),
            ("pause", true),
            ("resume", true),
            ("cancel", true),
            ("print_start", true),
            ("jog", false),
            ("home", false),
            ("set_fan", false),
            ("console", false),
            ("emergency_stop", false),
            ("qgl", false),
            ("speed_factor", false),
            ("macro", false),
        ] {
            assert_eq!(c.allows(id, "FlashForge").is_ok(), want, "{id}");
        }
        assert!(c.live_xy && c.upload && !c.camera);
    }
}
