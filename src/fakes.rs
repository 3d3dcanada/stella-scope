// Fake printers for tests and for the scratch instance (tools/fake-printers.rs includes this file).
// Compiled only under `cargo test` and into the example; the shipped binary never contains it.
// Every response shape is copied from a real reply or from the published spec of that brand, and
// each fake says which. The fakes record every request they get so a test can assert the exact
// bytes that left the service, which is the proof the work orders ask for.
#![allow(dead_code)]
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Req {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
impl Req {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }
    pub fn query(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, q)| q)
    }
    pub fn param(&self, key: &str) -> Option<String> {
        self.query().split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == key).then(|| pct_decode(v))
        })
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}
pub struct Rsp {
    pub code: u16,
    pub ctype: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}
impl Rsp {
    pub fn json(v: Value) -> Self {
        Self {
            code: 200,
            ctype: "application/json".into(),
            body: v.to_string().into_bytes(),
            headers: vec![],
        }
    }
    pub fn status(code: u16, v: Value) -> Self {
        Self {
            code,
            ..Self::json(v)
        }
    }
    pub fn text(s: &str) -> Self {
        Self {
            code: 200,
            ctype: "text/plain".into(),
            body: s.as_bytes().to_vec(),
            headers: vec![],
        }
    }
    pub fn bytes(ctype: &str, body: Vec<u8>) -> Self {
        Self {
            code: 200,
            ctype: ctype.into(),
            body,
            headers: vec![],
        }
    }
}
pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_req(stream: &mut TcpStream) -> Option<Req> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end;
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            head_end = p;
            break;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let mut headers = BTreeMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(Req {
        method,
        target,
        headers,
        body,
    })
}

fn write_rsp(stream: &mut TcpStream, r: Rsp) {
    let reason = match r.code {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.code,
        r.ctype,
        r.body.len()
    );
    for (k, v) in &r.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&r.body);
}

/// One thread accepts, one thread per connection answers; `stop` ends the accept loop.
pub fn serve_http(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    handler: Arc<dyn Fn(Req, &mut TcpStream) -> Option<Rsp> + Send + Sync>,
) {
    listener.set_nonblocking(true).ok();
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).ok();
                    let h = handler.clone();
                    thread::spawn(move || {
                        if let Some(req) = read_req(&mut s) {
                            if let Some(r) = h(req, &mut s) {
                                write_rsp(&mut s, r);
                            }
                        }
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(15)),
            }
        }
    });
}

// ------------------------------------------------------------------------------------------
// Moonraker + Klipper. Shapes: /server/info, /printer/info, /printer/objects/*, /printer/gcode/*,
// /server/gcode_store, /server/files/*, /server/job_queue/status as Moonraker's API documents them
// and as recorded from Stella on 2026-09-17 (see src/probe.rs and src/commands.rs fixtures).
// ------------------------------------------------------------------------------------------

pub struct MoonState {
    pub born: Instant,
    pub klippy: String,
    pub nozzle: f64,
    pub nozzle_target: f64,
    pub bed: f64,
    pub bed_target: f64,
    pub homed: String,
    pub pos: [f64; 3],
    pub print_state: String,
    pub filename: String,
    pub progress: f64,
    pub speed_factor: f64,
    pub extrude_factor: f64,
    pub fan: f64,
    pub z_offset: f64,
    pub busy_until: Option<Instant>,
    pub extruder_velocity_until: Option<Instant>,
    pub qgl_applied: bool,
    pub console: Vec<(f64, String, &'static str)>,
    pub log: Vec<Value>,
    /// Per-motor drv_status overrides, e.g. {"stepper_x": {"olb":1,"stst":1}}.
    pub tmc: BTreeMap<String, Value>,
    pub moving_until: Option<Instant>,
    pub qgl_secs: f64,
    pub load_secs: f64,
    pub last_temp_at: Instant,
    /// Every request waits this long before it is answered: a printer that hangs.
    pub stall_ms: u64,
    /// (file name, byte count) of every G-code file uploaded to `gcodes`, oldest first.
    pub uploads: Vec<(String, usize)>,
    /// UI board U06: a saved 7 x 7 bed mesh (a shallow bowl, 0.12 mm across, taken as a real Klipper reports it) for a browser to look at; off, the matrix is empty as it always was.
    pub mesh: bool,
}
impl MoonState {
    pub fn new() -> Self {
        Self {
            born: Instant::now(),
            klippy: "ready".into(),
            nozzle: 24.0,
            nozzle_target: 0.0,
            bed: 23.0,
            bed_target: 0.0,
            homed: "xyz".into(),
            pos: [175.0, 175.0, 10.0],
            print_state: "standby".into(),
            filename: String::new(),
            progress: 0.0,
            speed_factor: 1.0,
            extrude_factor: 1.0,
            fan: 0.0,
            z_offset: -0.035,
            busy_until: None,
            extruder_velocity_until: None,
            qgl_applied: false,
            console: Vec::new(),
            log: Vec::new(),
            tmc: BTreeMap::new(),
            moving_until: None,
            qgl_secs: 6.0,
            load_secs: 4.0,
            last_temp_at: Instant::now(),
            stall_ms: 0,
            uploads: Vec::new(),
            mesh: false,
        }
    }
    fn step(&mut self) {
        // Temperatures approach their targets: nozzle 8 C/s, bed 2 C/s, cooling at half that.
        let dt = self.last_temp_at.elapsed().as_secs_f64();
        self.last_temp_at = Instant::now();
        let approach = |cur: f64, target: f64, up: f64| {
            let goal = if target > 0.0 { target } else { 22.0 };
            let rate = if goal > cur { up } else { up / 2.0 };
            if (goal - cur).abs() <= rate * dt {
                goal
            } else {
                cur + (goal - cur).signum() * rate * dt
            }
        };
        self.nozzle = approach(self.nozzle, self.nozzle_target, 8.0);
        self.bed = approach(self.bed, self.bed_target, 2.0);
        if self.busy_until.is_some_and(|t| Instant::now() >= t) {
            self.busy_until = None;
        }
        if self.print_state == "printing" {
            self.progress = (self.progress + dt / 600.0).min(1.0);
            if self.progress >= 1.0 {
                self.print_state = "complete".into();
            }
        }
    }
    pub fn say(&mut self, kind: &'static str, msg: &str) {
        let t = unix_now();
        self.console.push((t, msg.to_owned(), kind));
        if self.console.len() > 300 {
            self.console.remove(0);
        }
    }
    fn busy(&self) -> bool {
        self.busy_until.is_some()
    }
}
fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub struct FakeMoonraker {
    pub addr: String,
    pub state: Arc<Mutex<MoonState>>,
    stop: Arc<AtomicBool>,
}
impl Drop for FakeMoonraker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl FakeMoonraker {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    /// Every request the service made, oldest first: {method, path, body}.
    pub fn log(&self) -> Vec<Value> {
        self.state.lock().unwrap().log.clone()
    }
    pub fn posts(&self, path_prefix: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|e| {
                e["method"] == "POST"
                    && e["path"]
                        .as_str()
                        .is_some_and(|p| p.starts_with(path_prefix))
            })
            .collect()
    }
    pub fn scripts(&self) -> Vec<String> {
        self.posts("/printer/gcode/script")
            .iter()
            .filter_map(|e| e["body"]["script"].as_str().map(str::to_owned))
            .collect()
    }
    pub fn start(port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("fake moonraker port");
        let addr = listener.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(MoonState::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let st = state.clone();
        serve_http(
            listener,
            stop.clone(),
            Arc::new(move |req, _s| Some(moon_route(&st, req))),
        );
        Self { addr, state, stop }
    }
}

const MOTORS: [&str; 7] = [
    "stepper_x",
    "stepper_y",
    "stepper_z",
    "stepper_z1",
    "stepper_z2",
    "stepper_z3",
    "extruder",
];

/// A 7 x 7 shallow bowl with a small twist, in the shape Klipper's `bed_mesh` object has (probed_matrix, mesh_min, mesh_max and the saved profile with its points and mesh_params).
fn mesh_object() -> Value {
    let rows: Vec<Vec<f64>> = (0..7)
        .map(|y| {
            (0..7)
                .map(|x| {
                    let (fx, fy) = (x as f64 / 6.0 - 0.5, y as f64 / 6.0 - 0.5);
                    ((0.16 * (fx * fx + fy * fy) + 0.03 * fx * fy * 4.0 - 0.02) * 10000.0).round() / 10000.0
                })
                .collect()
        })
        .collect();
    json!({"profile_name":"default","probed_matrix":rows,"mesh_min":[20,20],"mesh_max":[330,330],
        "profiles":{"default":{"points":rows,"mesh_params":{"min_x":20.0,"min_y":20.0,"max_x":330.0,"max_y":330.0,"x_count":7,"y_count":7}}}})
}

fn moon_status(s: &mut MoonState) -> Value {
    s.step();
    let moving = s.moving_until.is_some_and(|t| Instant::now() < t) || s.busy();
    let ev = s
        .extruder_velocity_until
        .is_some_and(|t| Instant::now() < t);
    let mut o = json!({
        "webhooks":{"state":s.klippy,"state_message":if s.klippy=="ready"{"Printer is ready"}else{"Lost communication with MCU 'mcu'\nPrinter is shutdown"}},
        "motion_report":{"live_position":[s.pos[0],s.pos[1],s.pos[2],0.0],"live_velocity":if moving {120.0} else {0.0},"live_extruder_velocity":if ev {5.0} else {0.0}},
        "toolhead":{"position":[s.pos[0],s.pos[1],s.pos[2],0.0],"homed_axes":s.homed,"max_velocity":300.0,"max_accel":3000.0,"square_corner_velocity":5.0,"axis_minimum":[0,0,-2,0],"axis_maximum":[350,350,330,0]},
        "gcode_move":{"homing_origin":[0,0,s.z_offset,0],"speed_factor":s.speed_factor,"extrude_factor":s.extrude_factor,"speed":1000.0},
        "idle_timeout":{"state":if s.busy()||s.print_state=="printing" {"Printing"} else if s.nozzle_target>0.0 {"Ready"} else {"Idle"}},
        "print_stats":{"state":s.print_state,"filename":s.filename,"total_duration":s.progress*600.0,"print_duration":s.progress*600.0,"filament_used":s.progress*3000.0,"message":"","info":{"total_layer":null,"current_layer":null}},
        "virtual_sdcard":{"progress":s.progress,"is_active":s.print_state=="printing","file_path":s.filename},
        "extruder":{"temperature":s.nozzle,"target":s.nozzle_target,"power":if s.nozzle<s.nozzle_target {0.8} else {0.2},"pressure_advance":0.05},
        "heater_bed":{"temperature":s.bed,"target":s.bed_target,"power":if s.bed<s.bed_target {0.7} else {0.1}},
        "fan":{"speed":s.fan,"rpm":null},
        "heater_fan hotend_fan":{"speed":if s.nozzle>50.0 {1.0} else {0.0},"rpm":null},
        "controller_fan controller_fan":{"speed":0.4,"rpm":null},
        "probe":{"last_query":false,"last_z_result":0.0},
        "bed_mesh":if s.mesh {mesh_object()} else {json!({"profile_name":"","probed_matrix":[[]],"mesh_min":[20,20],"mesh_max":[330,330]})},
        "quad_gantry_level":{"applied":s.qgl_applied},
        "stepper_enable":{"steppers":{"stepper_x":true,"stepper_y":true,"stepper_z":true,"stepper_z1":true,"stepper_z2":true,"stepper_z3":true,"extruder":true}},
        "configfile":{"settings":{"printer":{"kinematics":"corexy","max_velocity":300.0},"gcode_macro load_filament":{"gcode":"{% set temp = params.TEMP|default(220)|float %}\nG1 E50 F300"},"gcode_macro unload_filament":{"gcode":"{% set temp = params.TEMP|default(220)|float %}\nG1 E-100 F300"},"gcode_macro quad_gantry_level":{"gcode":"G28"},"bltouch":{"z_offset":1.9}}}
    });
    for name in MOTORS {
        let mut drv = json!({"cs_actual":if moving {18} else {8},"stst":if moving {0} else {1}});
        if let Some(extra) = s.tmc.get(name) {
            for (k, v) in extra.as_object().into_iter().flatten() {
                drv[k] = v.clone();
            }
        }
        o[format!("tmc2209 {name}")] =
            json!({"run_current":0.9,"hold_current":0.5,"drv_status":drv});
    }
    o
}

fn moon_script(st: &Arc<Mutex<MoonState>>, script: &str) {
    for line in script.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let upper = line.to_ascii_uppercase();
        let word = upper.split_whitespace().next().unwrap_or("");
        let arg = |key: &str| {
            upper
                .split_whitespace()
                .find_map(|t| t.strip_prefix(key).map(str::to_owned))
        };
        let mut s = st.lock().unwrap();
        s.say("command", line);
        match word {
            "SET_HEATER_TEMPERATURE" => {
                let target = arg("TARGET=").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                if arg("HEATER=").as_deref() == Some("EXTRUDER") {
                    s.nozzle_target = target;
                } else {
                    s.bed_target = target;
                }
            }
            "M104" | "M109" => {
                s.nozzle_target = upper
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse().ok()))
                    .unwrap_or(0.0)
            }
            "M140" | "M190" => {
                s.bed_target = upper
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse().ok()))
                    .unwrap_or(0.0)
            }
            "M106" => {
                s.fan = upper
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse::<f64>().ok()))
                    .unwrap_or(0.0)
                    / 255.0
            }
            "M220" => {
                s.speed_factor = upper
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse::<f64>().ok()))
                    .unwrap_or(100.0)
                    / 100.0
            }
            "M221" => {
                s.extrude_factor = upper
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse::<f64>().ok()))
                    .unwrap_or(100.0)
                    / 100.0
            }
            "G28" => {
                s.homed = "xyz".into();
                s.pos = [175.0, 175.0, 10.0];
                s.moving_until = Some(Instant::now() + Duration::from_secs(2));
            }
            "G1" => {
                for t in upper.split_whitespace() {
                    for (i, p) in ["X", "Y", "Z"].iter().enumerate() {
                        if let Some(v) = t.strip_prefix(p).and_then(|v| v.parse::<f64>().ok()) {
                            s.pos[i] += v;
                        }
                    }
                }
            }
            "PAUSE" => s.print_state = "paused".into(),
            // Mainsail's RESUME macro, as on Stella (finish-2 F05): it aborts with a cold nozzle and still answers "ok"
            "RESUME" if s.nozzle < 170.0 => s.say(
                "response",
                "!! Resume aborted !!! \"extruder\" not hot enough, please heat up again and press RESUME",
            ),
            "RESUME" => s.print_state = "printing".into(),
            // Mainsail's CANCEL_PRINT macro, as on Stella: it cancels the print and turns both heaters off
            "CANCEL_PRINT" => {
                s.print_state = "cancelled".into();
                s.nozzle_target = 0.0;
                s.bed_target = 0.0;
            }
            "QUAD_GANTRY_LEVEL" => {
                let total = s.qgl_secs;
                s.busy_until = Some(Instant::now() + Duration::from_secs_f64(total));
                s.moving_until = s.busy_until;
                drop(s);
                // Klipper prints each probe, then the round's range, as it goes. The POST does not
                // return until the whole macro has run, exactly like Moonraker.
                let rounds = [(0.24, 0), (0.08, 1), (0.034, 2), (0.012, 3)];
                let pts = [
                    (27.05, 24.28),
                    (27.05, 319.28),
                    (297.05, 319.28),
                    (297.05, 24.28),
                ];
                let each = total / rounds.len() as f64;
                for (range, n) in rounds {
                    for (i, (x, y)) in pts.iter().enumerate() {
                        thread::sleep(Duration::from_secs_f64(each * 0.7 / 4.0));
                        st.lock().unwrap().say(
                            "response",
                            &format!(
                                "// probe: at {x:.3},{y:.3} bed will contact at z={:.6}",
                                1.5 + 0.01 * i as f64 * (3 - n) as f64
                            ),
                        );
                    }
                    thread::sleep(Duration::from_secs_f64(each * 0.3));
                    st.lock().unwrap().say("response", &format!("// Gantry-relative probe points:\n// 0: 15.4 1: 15.5 2: 15.5 3: 15.4\n// Making the following Z adjustments:\n// stepper_z = 0.010297\n// stepper_z1 = -0.035458\n// stepper_z2 = -0.002368\n// stepper_z3 = 0.027528\n// Retries: {n}/5 Probed points range: {range:.6} tolerance: 0.020000"));
                }
                let mut s = st.lock().unwrap();
                s.qgl_applied = true;
                s.busy_until = None;
                continue;
            }
            // the shake after a load (filament_shake) is about a second of small fast moves on a real printer
            "SAVE_GCODE_STATE" => {
                drop(s);
                thread::sleep(Duration::from_millis(1200));
                continue;
            }
            "LOAD_FILAMENT" | "UNLOAD_FILAMENT" => {
                let t = arg("TEMP=").and_then(|v| v.parse().ok()).unwrap_or(220.0);
                s.nozzle_target = t;
                let secs = s.load_secs;
                s.busy_until = Some(Instant::now() + Duration::from_secs_f64(secs));
                s.extruder_velocity_until = Some(Instant::now() + Duration::from_secs_f64(secs));
                drop(s);
                thread::sleep(Duration::from_secs_f64(secs));
                st.lock()
                    .unwrap()
                    .say("response", &format!("// {} finished", word));
                continue;
            }
            _ => {}
        }
    }
}

fn moon_route(st: &Arc<Mutex<MoonState>>, req: Req) -> Rsp {
    let path = req.path().to_owned();
    let stall = st.lock().unwrap().stall_ms;
    if stall > 0 && !path.starts_with("/__") {
        thread::sleep(Duration::from_millis(stall));
    }
    let body = req.json();
    if !path.starts_with("/__")
        && !path.starts_with("/server/gcode_store")
        && !path.starts_with("/printer/objects/query")
    {
        st.lock()
            .unwrap()
            .log
            .push(json!({"method":req.method,"path":req.target,"body":body}));
    }
    if path.starts_with("/printer/objects/query") {
        st.lock()
            .unwrap()
            .log
            .push(json!({"method":req.method,"path":"/printer/objects/query","body":null}));
    }
    match (req.method.as_str(), path.as_str()) {
        ("GET", "/server/info") => Rsp::json(
            json!({"result":{"klippy_connected":true,"klippy_state":st.lock().unwrap().klippy,"components":[],"failed_components":[],"registered_directories":["config","gcodes"],"warnings":[],"websocket_count":0,"moonraker_version":"v0.9.3-fake","api_version":[1,5,0],"api_version_string":"1.5.0"}}),
        ),
        ("GET", "/printer/info") => {
            let s = st.lock().unwrap();
            Rsp::json(
                json!({"result":{"state":s.klippy,"state_message":if s.klippy=="ready"{"Printer is ready"}else{"Printer is shutdown"},"hostname":"fake-voron","software_version":"v0.12.0-fake","klipper_path":"/home/pi/klipper","config_file":"/home/pi/printer_data/config/printer.cfg"}}),
            )
        }
        ("GET", "/printer/objects/list") => {
            let mut names: Vec<String> = [
                "webhooks",
                "motion_report",
                "toolhead",
                "gcode_move",
                "idle_timeout",
                "print_stats",
                "virtual_sdcard",
                "extruder",
                "heater_bed",
                "fan",
                "heater_fan hotend_fan",
                "controller_fan controller_fan",
                "probe",
                "bed_mesh",
                "quad_gantry_level",
                "stepper_enable",
                "configfile",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            names.extend(MOTORS.iter().map(|m| format!("tmc2209 {m}")));
            Rsp::json(json!({"result":{"objects":names}}))
        }
        ("GET", "/printer/objects/query") => {
            let mut s = st.lock().unwrap();
            if s.klippy != "ready" && req.query().is_empty() {
                return Rsp::status(503, json!({"error":{"message":"Klippy not ready"}}));
            }
            let all = moon_status(&mut s);
            let mut out = serde_json::Map::new();
            for kv in req.query().split('&').filter(|k| !k.is_empty()) {
                let name = pct_decode(kv.split('=').next().unwrap_or(""));
                if let Some(v) = all.get(&name) {
                    out.insert(name, v.clone());
                }
            }
            Rsp::json(json!({"result":{"eventtime":unix_now(),"status":out}}))
        }
        ("GET", "/printer/gcode/help") => Rsp::json(json!({"result":{
            "QUAD_GANTRY_LEVEL":"Conform a moving, twistable gantry to the shape of a stationary bed",
            "LOAD_FILAMENT":"Park front right, heat, purge 50 mm","UNLOAD_FILAMENT":"Park front right, heat, retract 100 mm",
            "SET_HEATER_TEMPERATURE":"Sets a heater temperature","BED_MESH_CALIBRATE":"Perform Mesh Bed Leveling","PROBE_ACCURACY":"Probe Z-height accuracy at current XY position",
            "SET_GCODE_OFFSET":"Set a virtual offset to g-code positions","PAUSE":"Pauses the current print","RESUME":"Resumes the print from a pause","CANCEL_PRINT":"Cancel the current print",
            "FIRMWARE_RESTART":"Firmware restart","RESTART":"Reload config file and restart host software","EXCLUDE_OBJECT":"Excludes an object","SET_VELOCITY_LIMIT":"Set printer velocity limits","SET_PRESSURE_ADVANCE":"Set pressure advance parameters",
            "TEST_RESONANCES":"Runs the resonance test","Z_OFFSET_APPLY_PROBE":"Adjust the probe's z_offset","SAVE_CONFIG":"Overwrite config file and restart"}})),
        ("POST", "/printer/gcode/script") => {
            let script = body["script"].as_str().unwrap_or("").to_owned();
            moon_script(st, &script);
            Rsp::json(json!({"result":"ok"}))
        }
        ("GET", "/server/gcode_store") => {
            let s = st.lock().unwrap();
            let count: usize = req
                .param("count")
                .and_then(|c| c.parse().ok())
                .unwrap_or(100);
            let rows: Vec<Value> = s
                .console
                .iter()
                .rev()
                .take(count)
                .rev()
                .map(|(t, m, k)| json!({"message":m,"time":t,"type":k}))
                .collect();
            Rsp::json(json!({"result":{"gcode_store":rows}}))
        }
        ("GET", "/server/files/list") => Rsp::json(json!({"result":[
            {"path":"cube_20mm_PETG.gcode","modified":unix_now()-3600.0,"size":412345,"permissions":"rw"},
            {"path":"camera_cap_PETG.gcode","modified":unix_now()-86400.0,"size":2214400,"permissions":"rw"}]})),
        ("GET", "/server/files/metadata") => Rsp::json(
            json!({"result":{"size":412345,"modified":unix_now()-3600.0,"slicer":"OrcaSlicer","slicer_version":"2.2.0","layer_height":0.2,"first_layer_height":0.2,"object_height":20.0,"estimated_time":1260.0,"filament_name":"Generic PETG","filament_type":"PETG","filament_total":3200.0,"filament_weight_total":9.6,"layer_count":100,"first_layer_extr_temp":245.0,"first_layer_bed_temp":55.0,"nozzle_diameter":0.4,"thumbnails":[]}}),
        ),
        ("GET", "/server/job_queue/status") => {
            Rsp::json(json!({"result":{"queued_jobs":[],"queue_state":"ready"}}))
        }
        ("GET", "/machine/proc_stats") => Rsp::json(
            json!({"result":{"moonraker_stats":{"cpu_usage":1.0,"memory":40000},"cpu_temp":48.0,"network":{},"system_cpu_usage":{"cpu":12.0},"system_memory":{"total":4000000,"available":2300000,"used":1700000},"websocket_connections":0}}),
        ),
        ("GET", "/server/webcams/list") => Rsp::json(json!({"result":{"webcams":[]}})),
        ("GET", p) if p.starts_with("/server/files/config/") => {
            Rsp::text("[printer]\nkinematics: corexy\n")
        }
        ("POST", "/server/files/upload") => {
            // Multipart: the file part's name and how many bytes it carries.
            let body = &req.body;
            let text = String::from_utf8_lossy(&body[..body.len().min(2048)]).into_owned();
            let name = text
                .split("filename=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or("")
                .to_owned();
            let start = body.windows(4).position(|w| w == b"\r\n\r\n").map(|_| {
                // The file starts after the header of the part that names it.
                let at = text.find("filename=").unwrap_or(0);
                text[at..].find("\r\n\r\n").map(|i| at + i + 4).unwrap_or(0)
            });
            let tail = body
                .windows(2)
                .rposition(|w| w == b"\r\n")
                .unwrap_or(body.len());
            let end = body
                .windows(4)
                .rposition(|w| w == b"\r\n--")
                .unwrap_or(tail);
            let size = end.saturating_sub(start.unwrap_or(0));
            st.lock().unwrap().uploads.push((name.clone(), size));
            Rsp::json(
                json!({"item":{"path":name,"root":"gcodes","size":size},"print_queued":false,"action":"create_file"}),
            )
        }
        ("POST", "/printer/print/start") => {
            let mut s = st.lock().unwrap();
            s.filename = req.param("filename").unwrap_or_default();
            s.print_state = "printing".into();
            s.progress = 0.0;
            Rsp::json(json!({"result":"ok"}))
        }
        ("POST", "/printer/print/pause") => {
            st.lock().unwrap().print_state = "paused".into();
            Rsp::json(json!({"result":"ok"}))
        }
        ("POST", "/printer/print/resume") => {
            st.lock().unwrap().print_state = "printing".into();
            Rsp::json(json!({"result":"ok"}))
        }
        ("POST", "/printer/print/cancel") => {
            {
                let mut s = st.lock().unwrap();
                s.print_state = "cancelled".into();
                s.nozzle_target = 0.0;
                s.bed_target = 0.0;
            }
            Rsp::json(json!({"result":"ok"}))
        }
        ("POST", "/printer/emergency_stop") => {
            let mut s = st.lock().unwrap();
            s.klippy = "shutdown".into();
            s.nozzle_target = 0.0;
            s.bed_target = 0.0;
            Rsp::json(json!({"result":"ok"}))
        }
        ("POST", "/printer/firmware_restart") | ("POST", "/printer/restart") => {
            st.lock().unwrap().klippy = "ready".into();
            Rsp::json(json!({"result":"ok"}))
        }
        // Test control surface, not part of Moonraker.
        ("GET", "/__log") => Rsp::json(Value::Array(st.lock().unwrap().log.clone())),
        ("POST", "/__reset") => {
            let mut s = st.lock().unwrap();
            s.log.clear();
            s.console.clear();
            Rsp::json(json!({"ok":true}))
        }
        ("POST", "/__set") => {
            let mut s = st.lock().unwrap();
            for (k, v) in body.as_object().into_iter().flatten() {
                match (k.as_str(), v) {
                    ("nozzle", v) => s.nozzle = v.as_f64().unwrap_or(s.nozzle),
                    ("nozzle_target", v) => s.nozzle_target = v.as_f64().unwrap_or(0.0),
                    ("bed", v) => s.bed = v.as_f64().unwrap_or(s.bed),
                    ("bed_target", v) => s.bed_target = v.as_f64().unwrap_or(0.0),
                    ("klippy", v) => s.klippy = v.as_str().unwrap_or("ready").into(),
                    ("print_state", v) => s.print_state = v.as_str().unwrap_or("standby").into(),
                    ("filename", v) => s.filename = v.as_str().unwrap_or("").into(),
                    ("progress", v) => s.progress = v.as_f64().unwrap_or(0.0),
                    ("homed", v) => s.homed = v.as_str().unwrap_or("").into(),
                    ("pos", v) => {
                        for (i, n) in v.as_array().into_iter().flatten().take(3).enumerate() {
                            s.pos[i] = n.as_f64().unwrap_or(s.pos[i]);
                        }
                    }
                    ("qgl_secs", v) => s.qgl_secs = v.as_f64().unwrap_or(6.0),
                    ("load_secs", v) => s.load_secs = v.as_f64().unwrap_or(4.0),
                    ("tmc", v) => {
                        for (m, f) in v.as_object().into_iter().flatten() {
                            s.tmc.insert(m.clone(), f.clone());
                        }
                    }
                    _ => {}
                }
            }
            Rsp::json(json!({"ok":true}))
        }
        _ => Rsp::status(
            404,
            json!({"error":{"code":404,"message":format!("Not Found {path}")}}),
        ),
    }
}

// ------------------------------------------------------------------------------------------
// PrusaLink v1. Shapes: Prusa-Link-Web/spec/openapi.yaml (SPEC, not a capture from a printer), digest
// auth MD5 qop=auth as the real one challenges. The fake enforces the login, so a driver that never
// answers the challenge fails against it the way it would against the printer.
// ------------------------------------------------------------------------------------------
#[path = "drivers/digest.rs"]
mod digest_impl;

pub struct PrusaState {
    pub state: String,
    pub nozzle: f64,
    pub nozzle_target: f64,
    pub bed: f64,
    pub bed_target: f64,
    pub z: f64,
    pub x: f64,
    pub y: f64,
    pub moving: bool,
    pub job_id: Option<i64>,
    pub file: String,
    pub progress: f64,
    pub printing_s: f64,
    pub files: Vec<(String, usize, f64)>,
    pub uploads: Vec<(String, usize, bool)>,
    /// What each uploaded file holds, served at its download reference like the printer does.
    pub contents: std::collections::HashMap<String, Vec<u8>>,
    pub log: Vec<Value>,
    pub born: Instant,
    pub last: Instant,
    pub logins: u64,
    /// Every request waits this long before it is answered: a PrusaLink that is on but slow (finish-2 F04: the wall's first paint with a slow printer).
    pub stall_ms: u64,
}

pub struct FakePrusaLink {
    pub addr: String,
    pub state: Arc<Mutex<PrusaState>>,
    pub user: String,
    pub pass: String,
    stop: Arc<AtomicBool>,
}
impl Drop for FakePrusaLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

const PRUSA_REALM: &str = "Printer API";
const PRUSA_NONCE: &str = "9b4c0d8e1f7a3c52";

fn tiny_png() -> Vec<u8> {
    // A 1x1 PNG.
    vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, b'I', b'D',
        b'A', b'T', 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0, 0x05, 0xfe, 0x02, 0xfe, 0xa7,
        0x35, 0x81, 0x84, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ]
}

impl FakePrusaLink {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub fn log(&self) -> Vec<Value> {
        self.state.lock().unwrap().log.clone()
    }
    pub fn start(port: u16, user: &str, pass: &str) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("fake prusalink port");
        let addr = listener.local_addr().unwrap().to_string();
        let now = Instant::now();
        let state = Arc::new(Mutex::new(PrusaState {
            state: "IDLE".into(),
            nozzle: 24.0,
            nozzle_target: 0.0,
            bed: 23.0,
            bed_target: 0.0,
            z: 0.0,
            x: 125.0,
            y: 105.0,
            moving: false,
            job_id: None,
            file: String::new(),
            progress: 0.0,
            printing_s: 0.0,
            files: vec![
                ("cube_20mm_PETG.bgcode".into(), 412345, unix_now() - 3600.0),
                ("camera_cap.gcode".into(), 2214400, unix_now() - 86400.0),
                ("nopic.gcode".into(), 1604375, unix_now() - 90000.0),
            ],
            uploads: Vec::new(),
            contents: Default::default(),
            log: Vec::new(),
            born: now,
            last: now,
            logins: 0,
            stall_ms: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, u, p) = (state.clone(), user.to_owned(), pass.to_owned());
        serve_http(
            listener,
            stop.clone(),
            Arc::new(move |req, _s| Some(prusa_route(&st, &u, &p, req))),
        );
        Self {
            addr,
            state,
            user: user.into(),
            pass: pass.into(),
            stop,
        }
    }
}

fn prusa_step(s: &mut PrusaState) {
    let dt = s.last.elapsed().as_secs_f64();
    s.last = Instant::now();
    let approach = |cur: f64, target: f64, rate: f64| {
        let goal = if target > 0.0 { target } else { 22.0 };
        if (goal - cur).abs() <= rate * dt {
            goal
        } else {
            cur + (goal - cur).signum() * rate * dt
        }
    };
    s.nozzle = approach(s.nozzle, s.nozzle_target, 6.0);
    s.bed = approach(s.bed, s.bed_target, 1.5);
    if s.state == "PRINTING" {
        s.printing_s += dt;
        s.progress = (s.progress + dt / 6.0).min(100.0); // a 10-minute print
        s.moving = true;
        if s.progress >= 100.0 {
            s.state = "FINISHED".into();
            s.job_id = None;
            s.moving = false;
        }
    }
}

fn prusa_route(st: &Arc<Mutex<PrusaState>>, user: &str, pass: &str, req: Req) -> Rsp {
    let path = req.path().to_owned();
    let stall = st.lock().unwrap().stall_ms;
    if stall > 0 && !path.starts_with("/__") {
        thread::sleep(Duration::from_millis(stall));
    }
    if path.starts_with("/__") {
        let mut s = st.lock().unwrap();
        return match (req.method.as_str(), path.as_str()) {
            ("GET", "/__log") => Rsp::json(Value::Array(s.log.clone())),
            ("POST", "/__reset") => {
                s.log.clear();
                Rsp::json(json!({"ok":true}))
            }
            ("POST", "/__set") => {
                let b = req.json();
                for (k, v) in b.as_object().into_iter().flatten() {
                    match k.as_str() {
                        "state" => s.state = v.as_str().unwrap_or("IDLE").into(),
                        "nozzle" => s.nozzle = v.as_f64().unwrap_or(s.nozzle),
                        "nozzle_target" => s.nozzle_target = v.as_f64().unwrap_or(0.0),
                        "bed" => s.bed = v.as_f64().unwrap_or(s.bed),
                        "bed_target" => s.bed_target = v.as_f64().unwrap_or(0.0),
                        "file" => s.file = v.as_str().unwrap_or("").into(),
                        "progress" => s.progress = v.as_f64().unwrap_or(0.0),
                        "job_id" => s.job_id = v.as_i64(),
                        "moving" => s.moving = v.as_bool().unwrap_or(false),
                        "x" => s.x = v.as_f64().unwrap_or(s.x),
                        "y" => s.y = v.as_f64().unwrap_or(s.y),
                        "z" => s.z = v.as_f64().unwrap_or(s.z),
                        _ => {}
                    }
                }
                Rsp::json(json!({"ok":true}))
            }
            _ => Rsp::status(404, json!({})),
        };
    }
    // Digest login, enforced.
    let ok = req
        .headers
        .get("authorization")
        .map(|h| {
            let field = |k: &str| -> Option<String> {
                let i = h.find(&format!("{k}=\""))? + k.len() + 2;
                Some(h[i..].split('"').next()?.to_owned())
            };
            let nc = h
                .split("nc=")
                .nth(1)
                .map(|t| t.split([',', ' ']).next().unwrap_or(""))
                .unwrap_or("");
            let (u, uri, cn, resp) = (
                field("username"),
                field("uri"),
                field("cnonce"),
                field("response"),
            );
            match (u, uri, cn, resp) {
                (Some(u), Some(uri), Some(cn), Some(resp)) if u == user && uri == req.target => {
                    let ha1 = digest_impl::md5_hex(&format!("{user}:{PRUSA_REALM}:{pass}"));
                    let ha2 = digest_impl::md5_hex(&format!("{}:{}", req.method, uri));
                    digest_impl::md5_hex(&format!("{ha1}:{PRUSA_NONCE}:{nc}:{cn}:auth:{ha2}"))
                        == resp
                }
                _ => false,
            }
        })
        .unwrap_or(false);
    if !ok {
        let mut r = Rsp::status(401, json!({"message":"Unauthorized"}));
        r.headers.push((
            "WWW-Authenticate".into(),
            format!("Digest realm=\"{PRUSA_REALM}\", nonce=\"{PRUSA_NONCE}\", qop=\"auth\", algorithm=MD5"),
        ));
        return r;
    }
    let mut s = st.lock().unwrap();
    s.logins += 1;
    prusa_step(&mut s);
    if !path.starts_with("/api/v1/status") && !path.starts_with("/api/v1/job")
        || req.method != "GET"
    {
        let (m, t) = (req.method.clone(), req.target.clone());
        s.log.push(json!({"method":m,"path":t,"headers":{"print-after-upload":req.headers.get("print-after-upload"),"overwrite":req.headers.get("overwrite")},"bytes":req.body.len()}));
    }
    match (req.method.as_str(), path.as_str()) {
        ("GET", "/api/version") => Rsp::json(
            json!({"api":"0.1","server":"2.1.2","text":"PrusaLink 0.7.2","hostname":"prusa-core-one-l"}),
        ),
        ("GET", "/api/v1/info") => Rsp::json(
            json!({"nozzle_diameter":0.4,"mmu":false,"serial":"13052-4742441653609084","hostname":"prusa-core-one-l","min_extrusion_temp":170}),
        ),
        ("GET", "/api/v1/status") => {
            let printer = {
                let mut p = json!({"state":s.state,"temp_bed":s.bed,"target_bed":s.bed_target,"temp_nozzle":s.nozzle,"target_nozzle":s.nozzle_target,
                    "axis_z":s.z,"flow":100,"speed":100,"fan_hotend":5200,"fan_print":if s.state=="PRINTING" {3100} else {0}});
                if !s.moving {
                    p["axis_x"] = json!(s.x);
                    p["axis_y"] = json!(s.y);
                }
                p
            };
            let mut out = json!({"storage":{"path":"/usb/","name":"usb","read_only":false},"printer":printer});
            if let Some(id) = s.job_id {
                out["job"] = json!({"id":id,"progress":s.progress,"time_remaining":((100.0-s.progress)*6.0) as i64,"time_printing":s.printing_s as i64});
            }
            Rsp::json(out)
        }
        ("GET", "/api/v1/job") => match s.job_id {
            Some(id) => Rsp::json(
                json!({"id":id,"state":s.state,"progress":s.progress,"time_remaining":((100.0-s.progress)*6.0) as i64,"time_printing":s.printing_s as i64,
                "file":{"name":"BRACKET.BGC","display_name":s.file,"path":"/usb","size":412345,"refs":{"thumbnail":format!("/thumb/l/usb/{}", s.file),"icon":format!("/thumb/s/usb/{}", s.file)}}}),
            ),
            None => Rsp {
                code: 204,
                ctype: "application/json".into(),
                body: vec![],
                headers: vec![],
            },
        },
        ("GET", "/api/v1/storage") => Rsp::json(
            json!({"storage_list":[{"path":"/usb/","name":"usb","type":"USB","read_only":false,"available":true,"free_space":8000000000_i64,"total_space":16000000000_i64,"print_files":2}]}),
        ),
        ("GET", "/api/v1/files/usb/") | ("GET", "/api/v1/files/usb") => Rsp::json(
            json!({"type":"FOLDER","name":"usb","path":"/usb","read_only":false,
            // Polish WO-07 (live pass 2026-10-19): a CORE One L's USB stick is FAT, and PrusaLink lists each file by its 8.3 short name
            // ("TWISTB~1.GCO") with the real name only in display_name. The fake once used the long name for both, which hid
            // that the app listed no files at all on the real printer.
            "children": s.files.iter().map(|(n,sz,t)| {let sn=short_name(n);json!({"name":sn,"display_name":n,"type":"PRINT_FILE","size":sz,"m_timestamp":t,
                "refs":{"thumbnail":format!("/thumb/l/usb/{sn}"),"icon":format!("/thumb/s/usb/{sn}"),"download":format!("/usb/{sn}")}})}).collect::<Vec<_>>()}),
        ),
        ("GET", p) if p.starts_with("/api/v1/files/usb/") => {
            let name = pct_decode(&p["/api/v1/files/usb/".len()..]);
            // the printer answers to the long name and to the short one
            match s.files.iter().find(|(n, _, _)| *n == name || short_name(n) == name) {
                Some((n, sz, t)) => {
                    // Finish-2 F06 (2026-10-19): the real CORE One L (firmware 6.5.7) answers no `meta` at all for a plain .gcode (its time, filament and layer height
                    // are in the file's own comments); only a binary .bgcode, which carries them in blocks, is described. The fake once described every file.
                    let mut v = json!({"type":"PRINT_FILE","name":short_name(n),"display_name":n,"size":sz,"m_timestamp":t,
                    "refs":{"thumbnail":format!("/thumb/l/usb/{}", short_name(n)),"icon":format!("/thumb/s/usb/{}", short_name(n)),"download":format!("/usb/{}", short_name(n))}});
                    if n.to_ascii_lowercase().ends_with(".bgcode") {
                        v["meta"] = json!({"estimated_print_time":1260,"filament_type":"PETG","nozzle_diameter":0.4,"layer_height":0.2});
                    }
                    Rsp::json(v)
                }
                None => Rsp::status(404, json!({"message":"Not Found"})),
            }
        }
        // the file itself, at its download reference (short name), as the printer serves it
        ("GET", p) if p.starts_with("/usb/") => {
            let name = pct_decode(&p["/usb/".len()..]);
            match s.files.iter().find(|(n, _, _)| *n == name || short_name(n) == name) {
                Some((n, _, _)) => Rsp::bytes("application/octet-stream", s.contents.get(n).cloned().unwrap_or_else(|| b"G28\nG1 X10 Y10 Z0.2 E1 F1200\n".to_vec())),
                None => Rsp::status(404, json!({"message":"Not Found"})),
            }
        }
        // A file sliced without pictures (PrusaSlicer's command line) lists a thumbnail address that answers 404 (finish-2 F02).
        ("GET", p) if p.starts_with("/thumb/") && p.to_ascii_uppercase().contains("NOPIC") => Rsp::status(404, json!({"title":"Not Found"})),
        ("GET", p) if p.starts_with("/thumb/") => Rsp::bytes("image/png", tiny_png()),
        ("PUT", p) if p.starts_with("/api/v1/job/") && p.ends_with("/pause") => {
            if s.state == "PRINTING" {
                s.state = "PAUSED".into();
                s.moving = false;
                Rsp {
                    code: 204,
                    ctype: "text/plain".into(),
                    body: vec![],
                    headers: vec![],
                }
            } else {
                Rsp::status(409, json!({"message":"Conflict"}))
            }
        }
        ("PUT", p) if p.starts_with("/api/v1/job/") && p.ends_with("/resume") => {
            if s.state == "PAUSED" {
                s.state = "PRINTING".into();
                Rsp {
                    code: 204,
                    ctype: "text/plain".into(),
                    body: vec![],
                    headers: vec![],
                }
            } else {
                Rsp::status(409, json!({"message":"Conflict"}))
            }
        }
        ("DELETE", p) if p.starts_with("/api/v1/job/") => {
            if matches!(s.state.as_str(), "PRINTING" | "PAUSED") {
                s.state = "STOPPED".into();
                s.job_id = None;
                s.moving = false;
                Rsp {
                    code: 204,
                    ctype: "text/plain".into(),
                    body: vec![],
                    headers: vec![],
                }
            } else {
                Rsp::status(409, json!({"message":"Conflict"}))
            }
        }
        ("PUT", p) if p.starts_with("/api/v1/files/usb/") => {
            let name = pct_decode(&p["/api/v1/files/usb/".len()..]);
            let start = req
                .headers
                .get("print-after-upload")
                .is_some_and(|v| v == "?1");
            s.files.push((name.clone(), req.body.len(), unix_now()));
            s.uploads.push((name.clone(), req.body.len(), start));
            s.contents.insert(name.clone(), req.body.clone());
            if start {
                s.state = "PRINTING".into();
                s.job_id = Some(77);
                s.file = name;
                s.progress = 0.0;
                s.printing_s = 0.0;
            }
            Rsp {
                code: 201,
                ctype: "text/plain".into(),
                body: vec![],
                headers: vec![],
            }
        }
        ("POST", p) if p.starts_with("/api/v1/files/usb/") => {
            let name = pct_decode(&p["/api/v1/files/usb/".len()..]);
            if s.state == "PRINTING" {
                return Rsp::status(409, json!({"message":"Conflict"}));
            }
            s.state = "PRINTING".into();
            s.job_id = Some(78);
            s.file = name;
            s.progress = 0.0;
            s.printing_s = 0.0;
            Rsp {
                code: 204,
                ctype: "text/plain".into(),
                body: vec![],
                headers: vec![],
            }
        }
        _ => Rsp::status(404, json!({"message":"Not Found"})),
    }
}

// ------------------------------------------------------------------------------------------
// FlashForge Adventurer 5M. HTTP 8898 field names are from a real /detail capture of our printer
// (firmware 5.1.7, 2026-08-30); the 8899 reply texts follow the regexes in our probe.sh. The fake
// enforces LAN mode and the case-sensitive check code, takes one control session at a time, and keeps
// every TCP line it was sent so a test sees the exact bytes.
// ------------------------------------------------------------------------------------------
pub struct FlashState {
    pub status: String,
    pub machine_status: String,
    pub nozzle: f64,
    pub nozzle_target: f64,
    pub bed: f64,
    pub bed_target: f64,
    pub file: String,
    pub progress: f64,
    pub layer: u32,
    pub layers: u32,
    pub pos: [f64; 3],
    pub lan_mode: bool,
    pub serial: String,
    pub check: String,
    pub mac: String,
    pub files: Vec<String>,
    pub uploads: Vec<(String, usize)>,
    pub tcp_log: Vec<String>,
    pub http_log: Vec<Value>,
    pub controlled: bool,
    pub camera_url: String,
}

pub struct FakeFlashForge {
    pub http_addr: String,
    pub tcp_addr: String,
    pub state: Arc<Mutex<FlashState>>,
    stop: Arc<AtomicBool>,
}
impl Drop for FakeFlashForge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl FakeFlashForge {
    pub fn http_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }
    pub fn tcp_port(&self) -> u16 {
        self.tcp_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0)
    }
    pub fn start(http_port: u16, tcp_port: u16, serial: &str, check: &str) -> Self {
        let state = Arc::new(Mutex::new(FlashState {
            status: "ready".into(),
            machine_status: "READY".into(),
            nozzle: 24.0,
            nozzle_target: 0.0,
            bed: 23.0,
            bed_target: 0.0,
            file: String::new(),
            progress: 0.0,
            layer: 0,
            layers: 0,
            pos: [110.0, 110.0, 5.0],
            lan_mode: true,
            serial: serial.into(),
            check: check.into(),
            mac: "24:A1:0D:B1:E1:ED".into(),
            files: vec!["cube_20mm_PLA.gcode".into(), "Benchy_PLA.gcode.3mf".into()],
            uploads: Vec::new(),
            tcp_log: Vec::new(),
            http_log: Vec::new(),
            controlled: false,
            camera_url: String::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        // HTTP 8898
        let hl = TcpListener::bind(("127.0.0.1", http_port)).expect("fake flashforge http port");
        let http_addr = hl.local_addr().unwrap().to_string();
        let st = state.clone();
        serve_http(
            hl,
            stop.clone(),
            Arc::new(move |req, _s| Some(flash_http(&st, req))),
        );
        // TCP 8899
        let tl = TcpListener::bind(("127.0.0.1", tcp_port)).expect("fake flashforge tcp port");
        let tcp_addr = tl.local_addr().unwrap().to_string();
        tl.set_nonblocking(true).ok();
        let (st, sp) = (state.clone(), stop.clone());
        thread::spawn(move || {
            while !sp.load(Ordering::Relaxed) {
                match tl.accept() {
                    Ok((s, _)) => {
                        let st = st.clone();
                        thread::spawn(move || flash_tcp(s, st));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(15)),
                }
            }
        });
        Self {
            http_addr,
            tcp_addr,
            state,
            stop,
        }
    }
}

fn flash_detail(s: &FlashState) -> Value {
    json!({"autoShutdown":"close","cameraStreamUrl":s.camera_url,"chamberFanSpeed":100,"coolingFanSpeed":100,
        "cumulativeFilament":598.86,"cumulativePrintTime":7858,"currentPrintSpeed":122,"errorCode":"","estimatedTime":0,
        "firmwareVersion":"5.1.7","ipAddr":"192.168.1.126","macAddr":s.mac,"name":s.serial,"nozzleCnt":1,"nozzleModel":"0.4mm",
        "platTargetTemp":s.bed_target,"platTemp":s.bed,"printDuration":(s.progress*2160.0) as i64,"printFileName":s.file,
        "printFileThumbUrl":"","printLayer":s.layer,"printProgress":s.progress,"printSpeedAdjust":100,"rightFilamentType":"PLA",
        "rightTargetTemp":s.nozzle_target,"rightTemp":s.nozzle,"status":s.status,"targetPrintLayer":s.layers,
        "zAxisCompensation":0.05})
}

fn flash_http(st: &Arc<Mutex<FlashState>>, req: Req) -> Rsp {
    let mut s = st.lock().unwrap();
    // Finish-2 F05: the HTTP upload as the 5M's firmware takes it: multipart `gcodeFile`, the check code in headers.
    if req.path() == "/uploadGcode" {
        s.http_log.push(json!({"path":"/uploadGcode","print_now":req.headers.get("printnow")}));
        if req.headers.get("serialnumber") != Some(&s.serial) || req.headers.get("checkcode") != Some(&s.check) {
            return Rsp::json(json!({"code":1,"message":"Access code is different"}));
        }
        let text = String::from_utf8_lossy(&req.body).into_owned();
        let name = text.split("filename=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_owned();
        let start = text.find("\r\n\r\n").map_or(0, |i| i + 4);
        let end = req.body.windows(4).rposition(|w| w == b"\r\n--").unwrap_or(req.body.len());
        let size = end.saturating_sub(start);
        if req.headers.get("filesize").and_then(|v| v.parse::<usize>().ok()) != Some(size) {
            return Rsp::json(json!({"code":2,"message":"file size mismatch"}));
        }
        s.uploads.push((name.clone(), size));
        s.files.insert(0, name.clone());
        if req.headers.get("printnow").map(String::as_str) == Some("true") {
            s.file = name;
            s.status = "printing".into();
            s.machine_status = "BUILDING_FROM_SD".into();
        }
        return Rsp::json(json!({"code":0,"message":"Success"}));
    }
    let body = req.json();
    s.http_log
        .push(json!({"path":req.path(),"body":{"payload":body["payload"]}}));
    if !s.lan_mode {
        return Rsp::json(json!({"code":-2,"message":"Lan mode error"}));
    }
    if body["serialNumber"] != json!(s.serial) || body["checkCode"] != json!(s.check) {
        return Rsp::json(json!({"code":1,"message":"Access code is different"}));
    }
    match req.path() {
        "/detail" => Rsp::json(json!({"code":0,"message":"Success","detail":flash_detail(&s)})),
        "/printGcode" => {
            let name = body["fileName"].as_str().unwrap_or("").to_owned();
            if !s.files.contains(&name) {
                return Rsp::json(json!({"code":2,"message":"file not found"}));
            }
            s.file = name;
            s.status = "printing".into();
            s.machine_status = "BUILDING_FROM_SD".into();
            Rsp::json(json!({"code":0,"message":"Success"}))
        }
        "/gcodeList" => {
            Rsp::json(json!({"code":0,"message":"Success","gcodeList":s.files.clone()}))
        }
        "/control" => {
            let cmd = body["payload"]["cmd"].as_str().unwrap_or("").to_owned();
            let action = body["payload"]["args"]["action"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            if cmd == "jobCtl_cmd" {
                match action.as_str() {
                    "pause" => s.status = "paused".into(),
                    "continue" => s.status = "printing".into(),
                    "cancel" => s.status = "cancel".into(),
                    _ => return Rsp::json(json!({"code":2,"message":"bad action"})),
                }
                s.machine_status = match s.status.as_str() {
                    "paused" => "PAUSED",
                    "printing" => "BUILDING_FROM_SD",
                    _ => "READY",
                }
                .into();
            }
            Rsp::json(json!({"code":0,"message":"Success"}))
        }
        _ => Rsp::status(404, json!({"code":404})),
    }
}

fn flash_tcp(mut sock: TcpStream, st: Arc<Mutex<FlashState>>) {
    use std::io::{BufRead, BufReader};
    let _ = sock.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(sock.try_clone().unwrap());
    let reply = |sock: &mut TcpStream, cmd: &str, text: &str| {
        let _ = sock.write_all(format!("CMD {cmd} Received.\r\n{text}ok\r\n").as_bytes());
    };
    let mut mine = false;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let l = line.trim().to_owned();
        let code = l
            .trim_start_matches('~')
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_owned();
        let mut s = st.lock().unwrap();
        s.tcp_log.push(l.clone());
        match code.as_str() {
            "M601" => {
                if s.controlled && !mine {
                    let _ = sock.write_all(b"CMD M601 Received.\r\nControl failed.\r\nok\r\n");
                } else {
                    s.controlled = true;
                    mine = true;
                    reply(&mut sock, "M601", "Control Success V2.1.\r\n");
                }
            }
            "M602" => {
                if mine {
                    s.controlled = false;
                    mine = false;
                }
                reply(&mut sock, "M602", "Control Release.\r\n");
            }
            "M115" => {
                let text = format!("Machine Type: Flashforge Adventurer 5M\r\nMachine Name: {}\r\nFirmware: v5.1.7\r\nSN: {}\r\nX: 220 Y: 220 Z: 220\r\nTool Count: 1\r\nMac Address: {}\r\n", s.serial, s.serial, s.mac);
                reply(&mut sock, "M115", &text);
            }
            "M119" => {
                let text = format!("Endstop: X-max:0 Y-max:0 Z-max:0\r\nMachineStatus: {}\r\nMoveMode: READY\r\nStatus: S:0 L:0 J:0 F:0\r\nLED: 1\r\nCurrentFile: {}\r\n", s.machine_status, s.file);
                reply(&mut sock, "M119", &text);
            }
            "M105" => {
                let text = format!(
                    "T0:{:.1} /{:.1} B:{:.1} /{:.1}\r\n",
                    s.nozzle, s.nozzle_target, s.bed, s.bed_target
                );
                reply(&mut sock, "M105", &text);
            }
            "M27" => {
                let text = format!(
                    "SD printing byte {}/100\r\nLayer: {}/{}\r\n",
                    (s.progress * 100.0) as u32,
                    s.layer,
                    s.layers
                );
                reply(&mut sock, "M27", &text);
            }
            "M114" => {
                let text = format!(
                    "X:{:.3} Y:{:.3} Z:{:.3} A:200.000 B:0.000\r\n",
                    s.pos[0], s.pos[1], s.pos[2]
                );
                reply(&mut sock, "M114", &text);
            }
            "M661" => {
                let mut bytes = b"CMD M661 Received.\r\n".to_vec();
                bytes.extend_from_slice(&[0, 0, 0, 1]);
                for f in &s.files {
                    bytes.extend_from_slice(format!("/data/{f}").as_bytes());
                    bytes.extend_from_slice(&[0, 0]);
                }
                bytes.extend_from_slice(b"\r\nok\r\n");
                let _ = sock.write_all(&bytes);
            }
            "M104" => {
                s.nozzle_target = l
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse().ok()))
                    .unwrap_or(0.0);
                reply(&mut sock, "M104", "");
            }
            "M140" => {
                s.bed_target = l
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix('S').and_then(|v| v.parse().ok()))
                    .unwrap_or(0.0);
                reply(&mut sock, "M140", "");
            }
            "M83" | "G1" | "M25" | "M24" | "M26" => {
                match code.as_str() {
                    "M25" => {
                        s.status = "paused".into();
                        s.machine_status = "PAUSED".into();
                    }
                    "M24" => {
                        s.status = "printing".into();
                        s.machine_status = "BUILDING_FROM_SD".into();
                    }
                    "M26" => {
                        s.status = "ready".into();
                        s.machine_status = "READY".into();
                    }
                    _ => {}
                }
                reply(&mut sock, &code, "");
            }
            "M23" => {
                let name = l
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .trim_start_matches("0:/user/")
                    .to_owned();
                s.file = name;
                s.status = "printing".into();
                s.machine_status = "BUILDING_FROM_SD".into();
                s.progress = 0.0;
                reply(&mut sock, "M23", "File opened\r\n");
            }
            "M28" => {
                let size: usize = l
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let name = l
                    .split_whitespace()
                    .nth(2)
                    .unwrap_or("")
                    .trim_start_matches("0:/user/")
                    .to_owned();
                let _ = sock.write_all(b"CMD M28 Received.\r\nWriting to file: ok\r\n");
                drop(s);
                let mut got = 0;
                let mut buf = vec![0_u8; 4096];
                while got < size {
                    match std::io::Read::read(&mut reader, &mut buf[..(size - got).min(4096)]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got += n,
                    }
                }
                let mut s = st.lock().unwrap();
                s.uploads.push((name, got));
                continue;
            }
            "M29" => {
                reply(&mut sock, "M29", "Done saving file.\r\n");
                if let Some((name, _)) = s.uploads.last().cloned() {
                    if !s.files.contains(&name) {
                        s.files.push(name);
                    }
                }
            }
            _ => reply(&mut sock, &code, ""),
        }
    }
    let mut s = st.lock().unwrap();
    if mine {
        s.controlled = false;
    }
}

// ------------------------------------------------------------------------------------------
// An MJPEG camera (mjpg-streamer style, `/?action=stream` and `/?action=snapshot`), for the printers
// whose cameras are plain HTTP. Frames are made once by ffmpeg's test source and looped; with no ffmpeg
// there is one marker-only frame.
// ------------------------------------------------------------------------------------------
pub struct FakeMjpeg {
    pub addr: String,
    pub clients: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<AtomicBool>,
}
impl Drop for FakeMjpeg {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl FakeMjpeg {
    pub fn start(port: u16, fps: u64) -> Self {
        let mut frames: Vec<Vec<u8>> = Vec::new();
        if let Ok(out) = std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x360:rate=5",
                "-frames:v",
                "25",
                "-q:v",
                "7",
                "-f",
                "image2pipe",
                "-c:v",
                "mjpeg",
                "pipe:1",
            ])
            .output()
        {
            let b = out.stdout;
            let mut i = 0;
            while i + 1 < b.len() {
                if b[i] == 0xFF && b[i + 1] == 0xD8 {
                    if let Some(e) = b[i + 2..].windows(2).position(|w| w == [0xFF, 0xD9]) {
                        frames.push(b[i..i + 2 + e + 2].to_vec());
                        i += 2 + e + 2;
                        continue;
                    }
                }
                i += 1;
            }
        }
        if frames.is_empty() {
            frames.push(vec![0xFF, 0xD8, 0xFF, 0xD9]);
        }
        let frames = Arc::new(frames);
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("fake mjpeg port");
        let addr = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (sp, cl) = (stop.clone(), clients.clone());
        serve_http(
            listener,
            stop.clone(),
            Arc::new(move |req, s| {
                let frames = frames.clone();
                cl.fetch_add(1, Ordering::Relaxed);
                if req.target.contains("action=snapshot") {
                    return Some(Rsp::bytes("image/jpeg", frames[0].clone()));
                }
                let _ = write!(s, "HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=boundarydonotcross\r\nConnection: close\r\n\r\n");
                let mut i = 0;
                while !sp.load(Ordering::Relaxed) {
                    let f = &frames[i % frames.len()];
                    let ok = write!(s, "--boundarydonotcross\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", f.len()).is_ok()
                    && s.write_all(f).is_ok()
                    && s.write_all(b"\r\n").is_ok();
                    if !ok {
                        break;
                    }
                    i += 1;
                    thread::sleep(Duration::from_millis(1000 / fps.max(1)));
                }
                None
            }),
        );
        Self {
            addr,
            clients,
            stop,
        }
    }
    pub fn stream_url(&self) -> String {
        format!("http://{}/?action=stream", self.addr)
    }
}

/// The FAT 8.3 name PrusaLink lists a long file name under ("twistbowl.gcode" -> "TWISTB~1.GCO").
fn short_name(long: &str) -> String {
    let (stem, ext) = long.rsplit_once('.').unwrap_or((long, ""));
    let up = |s: &str, n: usize| -> String { s.chars().filter(char::is_ascii_alphanumeric).take(n).collect::<String>().to_ascii_uppercase() };
    let (s, e) = (up(stem, 8), up(ext, 3));
    if s.len() <= 8 && stem.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 3 && stem.len() <= 8 {
        return if e.is_empty() { s } else { format!("{s}.{e}") };
    }
    format!("{}~1.{e}", up(stem, 6))
}
