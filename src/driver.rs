//! The seam between the service and a printer brand (farm board WO-02, 2026-10-12).
//!
//! Until this file everything printer-facing assumed Moonraker. A `Driver` is the one place a brand
//! plugs in: it reports state in the shape the whole service already reads (the Moonraker object map:
//! `print_stats`, `toolhead`, `extruder`, `heater_bed`, `virtual_sdcard`, `webhooks`, ...), says what
//! the printer can do (`Capabilities`), and carries out a driver-neutral `Action`. Moonraker stays
//! first-class: `MoonrakerDriver` is the old `Moonraker` client unchanged behind the trait, and the
//! Klipper command path in commands.rs (preview, review tiers, fresh-state gates) still renders the
//! same G-code it always did. PrusaLink and FlashForge are written against their own public specs in
//! WO-04 and WO-05; nothing here copies code from KX-Bridge, Bambuddy, OctoEverywhere or Klipper.
//!
//! Rule that outranks the rest (2026-10-12): never present a value a printer did not report.
//! A driver that cannot read something returns `null` for it, and the UI shows it as unknown.
use crate::{
    config::Config,
    demo::Demo,
    moonraker::Moonraker,
    store::Event,
    ws_source::{self, WsEvent},
    Result,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    sync::{atomic::AtomicBool, mpsc::Receiver, Arc},
};

/// What one printer can do. The UI hides or disables whatever is false, with a plain reason, instead of
/// letting a press fail. Each field is a thing the user would name, not a protocol feature.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Capabilities {
    /// Set nozzle and bed temperature targets.
    pub heat: bool,
    pub fan: bool,
    /// Speed and flow factors (M220 / M221).
    pub speed: bool,
    pub flow: bool,
    /// Move the head by hand (the jog pad).
    pub jog: bool,
    pub home: bool,
    pub motors_off: bool,
    /// Push or pull filament by a length.
    pub extrude: bool,
    pub filament: bool,
    pub pause_resume: bool,
    pub cancel: bool,
    pub start_print: bool,
    /// Send a file to the printer.
    pub upload: bool,
    /// Type G-code.
    pub console: bool,
    /// Run named macros.
    pub macros: bool,
    /// Gantry level, bed mesh, probe, Z offset: Klipper's own service routines.
    pub level: bool,
    pub mesh: bool,
    pub z_offset: bool,
    /// Klipper-only tuning and diagnostics: velocity limits, pressure advance, GET_POSITION,
    /// QUERY_PROBE, resonance and shaper runs, printer.cfg edits, object exclusion, the job queue.
    pub klipper: bool,
    pub restart: bool,
    pub emergency_stop: bool,
    pub camera: bool,
    pub thumbnails: bool,
    /// The service can read the print file itself (the G-code follower and the layer picture). A FlashForge's stock
    /// HTTP door does not serve file contents (polish WO-06, 2026-10-18): the page must not keep asking.
    pub gcode_file: bool,
    /// X and Y are reported while the head moves (PrusaLink reports them only when it is still).
    pub live_xy: bool,
    /// X and Y are reported at all. A bridge that fills them with zeros has false: unknown, not 0.
    pub xy: bool,
    /// The printer pushes state; false means the service polls it.
    pub push: bool,
}

impl Capabilities {
    /// Klipper through Moonraker: everything the service has ever done.
    pub fn klipper() -> Self {
        Self {
            heat: true,
            fan: true,
            speed: true,
            flow: true,
            jog: true,
            home: true,
            motors_off: true,
            extrude: true,
            filament: true,
            pause_resume: true,
            cancel: true,
            start_print: true,
            upload: true,
            console: true,
            macros: true,
            level: true,
            mesh: true,
            z_offset: true,
            klipper: true,
            restart: true,
            emergency_stop: true,
            camera: true,
            thumbnails: true,
            gcode_file: true,
            live_xy: true,
            xy: true,
            push: true,
        }
    }
    /// A driver starts from nothing and claims only what it has verified.
    pub fn none() -> Self {
        Self {
            heat: false,
            fan: false,
            speed: false,
            flow: false,
            jog: false,
            home: false,
            motors_off: false,
            extrude: false,
            filament: false,
            pause_resume: false,
            cancel: false,
            start_print: false,
            upload: false,
            console: false,
            macros: false,
            level: false,
            mesh: false,
            z_offset: false,
            klipper: false,
            restart: false,
            emergency_stop: false,
            camera: false,
            thumbnails: false,
            gcode_file: false,
            live_xy: false,
            xy: false,
            push: false,
        }
    }
    /// Can this command run on this printer? `Err` carries the plain-words reason the UI shows.
    /// The ids are src/commands.rs COMMANDS; an id this table does not know is refused.
    pub fn allows(&self, command_id: &str, brand: &str) -> std::result::Result<(), String> {
        let (ok, what) = match command_id {
            "set_heater" => (self.heat, "set temperatures"),
            "set_fan" => (self.fan, "set the part fan"),
            "speed_factor" => (self.speed, "change the speed"),
            "extrude_factor" => (self.flow, "change the flow"),
            "jog" => (self.jog, "move the head by hand"),
            "home" => (self.home, "home the axes"),
            "motors_off" => (self.motors_off, "switch the motors off"),
            "extrude" | "load_filament" | "unload_filament" => {
                (self.extrude, "move filament by a length")
            }
            "filament_load" | "filament_unload" | "filament_shake" => {
                (self.filament, "run a filament load, unload or shake")
            }
            "pause" | "resume" => (self.pause_resume, "pause or resume a print"),
            "cancel" => (self.cancel, "cancel a print"),
            "print_start" => (self.start_print, "start a print"),
            "dry_run" => (self.start_print, "run a dry run of a file"),
            "console" => (self.console, "take typed G-code"),
            "macro" => (self.macros, "run macros"),
            "qgl" => (self.level, "level the gantry"),
            "mesh" | "probe_accuracy" => (self.mesh, "probe the bed"),
            "z_offset" | "z_offset_save" => (self.z_offset, "change the Z offset"),
            "limit" | "pressure_advance" | "position" | "probe_query" | "exclude_object"
            | "print_queue_add" | "print_queue_remove" | "resonance_x" | "resonance_y"
            | "shaper_x" | "shaper_y" | "resonance_capture" | "config_heater_fan" => {
                (self.klipper, "use tuning and diagnostics tools")
            }
            "klipper_restart" | "firmware_restart" | "reconnect" => {
                (self.restart, "restart its firmware")
            }
            "pid_calibrate" | "probe_calibrate" | "z_endstop_calibrate" | "testz" | "manual_accept"
            | "manual_abort" | "tuning_tower" | "save_config" => (self.klipper, "run Klipper's calibrations from here"),
            "emergency_stop" => (self.emergency_stop, "emergency-stop from here"),
            _ => (false, "do that"),
        };
        if ok {
            Ok(())
        } else {
            Err(format!("{brand} cannot {what}"))
        }
    }
}

/// What a command needs to know about the printer right now (src/commands.rs FreshState reads it).
pub struct StateSnapshot {
    /// `ready`, or the brand's word for not ready (Klipper: startup, shutdown, error).
    pub state: String,
    pub homed_axes: String,
    pub print_state: String,
    pub idle_state: String,
    /// Extended commands the printer reports (Klipper's gcode/help). Empty for other brands.
    pub help: BTreeSet<String>,
}

/// A temperature target's heater.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heater {
    Nozzle,
    Bed,
}

/// A driver-neutral thing to do, already bounded by src/commands.rs. A driver maps it to its own
/// protocol or returns an honest error; it never invents an effect.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SetTemperature { heater: Heater, celsius: f64 },
    Fan { fraction: f64 },
    SpeedFactor { percent: f64 },
    FlowFactor { percent: f64 },
    Jog { axis: char, mm: f64 },
    Home,
    MotorsOff,
    Extrude { mm: f64, mm_per_s: f64 },
    Pause,
    Resume,
    Cancel,
    StartPrint { filename: String },
    Gcode(String),
}

/// One camera a driver offers.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct CameraSource {
    pub name: String,
    /// `mjpeg` (a stream) or `snapshot` (a still fetched repeatedly).
    pub kind: &'static str,
    /// Absolute http:// URL on the printer's own host.
    pub url: String,
}

// `kind`, `cameras` and `upload` are read by the drivers and the farm queue of WO-04, 05 and 08.
#[allow(dead_code)]
pub trait Driver: Send {
    /// `moonraker`, `prusalink`, `flashforge`.
    fn kind(&self) -> &'static str;
    /// The brand in words a person reads.
    fn brand(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    // ---- the recorder ------------------------------------------------------------------------
    /// One state sample in the normalised object map, plus warnings. Errors start with
    /// `moonraker::UNREACHABLE` when the printer does not answer at all.
    fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)>;
    /// Klipper's configfile settings; other brands have none (an empty object, not a fabrication).
    fn fetch_config(&mut self) -> Result<Value> {
        Ok(json!({}))
    }
    /// The bed size in mm when the printer reports it, or the model it identifies as is in the
    /// known-model table (WO-14 item 4). None leaves the configured size: queue matching must not
    /// run on the 350 mm default for a machine nobody measured.
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        None
    }
    /// The machine's model when the printer names it (read-only, at add time). None leaves it unset.
    fn model_hint(&mut self) -> Option<String> {
        None
    }
    fn discover(&mut self) {}
    fn manifest(&self) -> Value {
        Value::Null
    }
    fn host_tick(&mut self, _ts: f64) -> Value {
        Value::Null
    }
    /// New console lines, and whether the window is full.
    fn console(&self) -> Result<(Vec<Event>, bool)> {
        Ok((Vec::new(), false))
    }
    /// Slicer metadata for the file being printed, and any tuned settings found in its tail.
    fn print_context(&self, _filename: &str) -> Result<(Value, Value)> {
        Err("this printer does not report print metadata".into())
    }
    /// A push source, when the printer has one. The collector polls `sample` when this is `None`.
    fn push(&self, _stop: &Arc<AtomicBool>) -> Option<Receiver<WsEvent>> {
        None
    }

    // ---- reads -------------------------------------------------------------------------------
    /// `[{path, size, modified}]`, printable files only.
    fn files(&self) -> Result<Vec<Value>> {
        Err("this printer cannot list files".into())
    }
    /// The size in bytes of one printable file, for the gcode follower's size gate (WO-01).
    fn file_size(&self, _filename: &str) -> Result<usize> {
        Err("this printer cannot report a file size".into())
    }
    /// The bytes of one printable file the printer holds, for the G-code follower (WO-01). Read-only,
    /// one request, capped so a 100 MB file never holds the service whole. Moonraker serves it from
    /// its gcodes root; PrusaLink from /api/v1/files/{storage}/{path}; FlashForge's stock HTTP door
    /// does not serve file contents, so it errors and the follower follows by layer and time instead.
    /// The Kobra X bridge answers as Moonraker.
    fn file_bytes(&self, _filename: &str, _max_bytes: usize) -> Result<Vec<u8>> {
        Err("this printer cannot serve a print file; follow by layer and time instead".into())
    }
    fn file_metadata(&self, _filename: &str) -> Result<Value> {
        Err("this printer does not report file metadata".into())
    }
    fn file_thumbnail(&self, _filename: &str, _max_bytes: usize) -> Result<Vec<u8>> {
        Err("this printer has no thumbnails".into())
    }
    fn cameras(&self) -> Result<Vec<CameraSource>> {
        Ok(Vec::new())
    }

    // ---- control -----------------------------------------------------------------------------
    fn snapshot(&self) -> Result<StateSnapshot>;
    fn act(&self, action: &Action) -> Result<Value>;
    /// Send a file and, when `start`, begin printing it.
    fn upload(&self, _name: &str, _bytes: &[u8], _start: bool) -> Result<Value> {
        Err("this printer cannot take a file from here".into())
    }
    /// Stop the printer at once (the emergency stop). Default: not offered.
    fn emergency_stop(&self) -> Result<Value> {
        Err("this printer has no emergency stop from here".into())
    }
}

/// The phrase a driver uses when the printer answered and said it will not do what was asked (PrusaLink's 409). The command is then
/// known not to have happened: "refused", not the "unknown" of a request that may or may not have arrived (polish WO-06).
pub const PRINTER_REFUSED: &str = "says it cannot";

/// Every driver's "the printer did not answer at all" error carries this phrase (Moonraker's own is
/// `moonraker::UNREACHABLE`), so the collector can tell a printer that is off from one that answers
/// badly. The link state then reads "off", not "not ready".
pub const UNREACHABLE_GENERIC: &str = "cannot reach the printer";

/// The brand names `Config.driver` may hold.
pub const KINDS: [&str; 4] = ["moonraker", "prusalink", "flashforge", "demo"];

/// Open the driver a configuration names. Moonraker is built here; the others are added by WO-04/05
/// and report themselves unavailable until then rather than quietly falling back to Moonraker.
pub fn open(cfg: &Config) -> Result<Box<dyn Driver>> {
    match cfg.driver.as_str() {
        "moonraker" => Ok(Box::new(MoonrakerDriver::new(cfg))),
        "demo" => Ok(Box::new(Demo::open(cfg))),
        other => crate::drivers::open_other(other, cfg),
    }
}

// ------------------------------------------------------------------------------------------------
// Klipper through Moonraker: the old client, behind the trait, behaving exactly as before.
// ------------------------------------------------------------------------------------------------

/// A printer that is not Klipper but is presented as Moonraker by a bridge on this computer (the
/// Anycubic Kobra X through KX-Bridge). The bridge answers every Klipper object, but fills what the
/// machine never told it with zeros: X and Y, velocity, flow. Those must show as unknown, never as 0
/// (README rule 2), and Klipper's own service routines do not exist on the machine.
pub struct Bridge {
    pub model: &'static str,
    pub brand: &'static str,
    /// Lower-case pieces of printer/info cpu_info or hostname that identify the bridge.
    pub signs: &'static [&'static str],
    pub caps: fn() -> Capabilities,
    /// The printable area, from the maker's own slicer profile (the bridge does not report it).
    pub bed: [f64; 2],
}

pub const BRIDGES: &[Bridge] = &[Bridge {
    model: "Kobra X",
    brand: "Anycubic",
    signs: &["kobra x", "kobrax"],
    // Anycubic's OrcaSlicer profile "Anycubic Kobra X 0.4 nozzle": printable_area 260 x 260, height 260
    bed: [260.0, 260.0],
    caps: || Capabilities {
        heat: true,
        fan: true,
        pause_resume: true,
        cancel: true,
        camera: true,
        thumbnails: true,
        // unchanged by polish WO-06: the bridge answers as Moonraker, which serves its files; not checked against the machine
        gcode_file: true,
        push: true,
        ..Capabilities::none()
    },
}];

/// The bridge a configured model names, if any.
pub fn bridged(model: Option<&str>) -> Option<&'static Bridge> {
    let m = model?.to_ascii_lowercase();
    BRIDGES
        .iter()
        .find(|b| m.contains(&b.model.to_ascii_lowercase()))
}

pub struct MoonrakerDriver {
    cfg: Config,
    client: Moonraker,
}

impl MoonrakerDriver {
    pub fn new(cfg: &Config) -> Self {
        Self {
            cfg: cfg.clone(),
            client: Moonraker::new(cfg),
        }
    }
    /// The Klipper G-code a driver-neutral action means. commands.rs `render` produces the same text
    /// for the same request (a test holds the two together); this is what a caller with only an
    /// `Action` in hand (the farm queue) sends.
    pub fn script_for(action: &Action) -> Option<String> {
        Some(match action {
            Action::SetTemperature { heater, celsius } => format!(
                "SET_HEATER_TEMPERATURE HEATER={} TARGET={:.0}",
                if *heater == Heater::Nozzle {
                    "extruder"
                } else {
                    "heater_bed"
                },
                celsius
            ),
            Action::Fan { fraction } => format!("M106 S{:.0}", fraction * 255.0),
            Action::SpeedFactor { percent } => format!("M220 S{percent:.0}"),
            Action::FlowFactor { percent } => format!("M221 S{percent:.0}"),
            Action::Jog { axis, mm } => {
                let feed = if *axis == 'Z' { 900 } else { 6000 };
                format!("G91\nG1 {axis}{mm:.2} F{feed}\nG90")
            }
            Action::Home => "G28".into(),
            Action::MotorsOff => "M84".into(),
            Action::Extrude { mm, mm_per_s } => format!("M83\nG1 E{mm:.2} F{:.0}", mm_per_s * 60.0),
            Action::Pause => "PAUSE".into(),
            Action::Resume => "RESUME".into(),
            Action::Cancel => "CANCEL_PRINT".into(),
            Action::Gcode(line) => line.clone(),
            Action::StartPrint { .. } => return None,
        })
    }
}

impl Driver for MoonrakerDriver {
    fn kind(&self) -> &'static str {
        "moonraker"
    }
    fn brand(&self) -> &'static str {
        bridged(self.cfg.printer_model.as_deref()).map_or("Klipper", |b| b.brand)
    }
    fn capabilities(&self) -> Capabilities {
        bridged(self.cfg.printer_model.as_deref())
            .map_or_else(Capabilities::klipper, |b| (b.caps)())
    }
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        bridged(self.cfg.printer_model.as_deref()).map(|b| b.bed)
    }
    fn model_hint(&mut self) -> Option<String> {
        // Moonraker::get already unwraps "result"
        let r = self.client.get("/printer/info").ok()?;
        let named = [r["cpu_info"].as_str(), r["hostname"].as_str()];
        BRIDGES.iter().find_map(|b| {
            named
                .iter()
                .flatten()
                .any(|n| b.signs.iter().any(|s| n.to_ascii_lowercase().contains(s)))
                .then(|| b.model.to_owned())
        })
    }
    fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)> {
        self.client.sample(ts)
    }
    fn fetch_config(&mut self) -> Result<Value> {
        self.client.fetch_config()
    }
    fn discover(&mut self) {
        self.client.discover()
    }
    fn manifest(&self) -> Value {
        self.client.manifest()
    }
    fn host_tick(&mut self, ts: f64) -> Value {
        self.client.host_tick(ts)
    }
    fn console(&self) -> Result<(Vec<Event>, bool)> {
        self.client.console()
    }
    fn print_context(&self, filename: &str) -> Result<(Value, Value)> {
        self.client.print_context(filename)
    }
    fn push(&self, stop: &Arc<AtomicBool>) -> Option<Receiver<WsEvent>> {
        Some(ws_source::spawn(&self.cfg, stop))
    }
    fn files(&self) -> Result<Vec<Value>> {
        let list = self.client.get("/server/files/list?root=gcodes")?;
        let mut files: Vec<Value> = list["result"]
            .as_array()
            .or_else(|| list.as_array())
            .map(|rows| {
                rows.iter()
                    .filter(|row| {
                        row["path"].as_str().is_some_and(|p| p.to_ascii_lowercase().ends_with(".gcode"))
                    })
                    .map(|row| json!({"path":row["path"],"size":row["size"],"modified":row["modified"]}))
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| {
            b["modified"]
                .as_f64()
                .partial_cmp(&a["modified"].as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        files.truncate(200);
        Ok(files)
    }
    fn file_size(&self, filename: &str) -> Result<usize> {
        let meta = self.client.file_metadata(filename)?;
        meta["size"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as usize)
            .ok_or_else(|| format!("Moonraker did not report a size for {filename}"))
    }
    fn file_bytes(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.client.gcode_file(filename, max_bytes)
    }
    fn file_metadata(&self, filename: &str) -> Result<Value> {
        self.client.file_metadata(filename)
    }
    fn file_thumbnail(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.client.file_thumbnail(filename, max_bytes)
    }
    fn snapshot(&self) -> Result<StateSnapshot> {
        let info = self.client.get("/printer/info")?;
        let state = info["state"]
            .as_str()
            .ok_or("printer info did not report a state; command was not sent")?;
        if state != "ready" {
            return Ok(StateSnapshot {
                state: state.to_owned(),
                homed_axes: String::new(),
                print_state: String::new(),
                idle_state: String::new(),
                help: BTreeSet::new(),
            });
        }
        let status = self.client.get(
            "/printer/objects/query?toolhead=homed_axes&print_stats=state&idle_timeout=state",
        )?;
        let s = status["status"]
            .as_object()
            .ok_or("printer status is missing; command was not sent")?;
        let text = |o: &str, f: &str| -> Result<String> {
            s.get(o)
                .and_then(|v| v.get(f))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("fresh {o}.{f} state is missing; command was not sent"))
        };
        let help = self.client.get("/printer/gcode/help")?;
        Ok(StateSnapshot {
            state: "ready".into(),
            homed_axes: text("toolhead", "homed_axes")?,
            print_state: text("print_stats", "state")?,
            idle_state: text("idle_timeout", "state")?,
            help: help
                .as_object()
                .ok_or("G-code help is missing; command was not sent")?
                .keys()
                .cloned()
                .collect(),
        })
    }
    fn upload(&self, name: &str, bytes: &[u8], start: bool) -> Result<Value> {
        let v = self.client.upload_gcode(name, bytes)?;
        if start {
            self.act(&Action::StartPrint {
                filename: name.to_owned(),
            })?;
        }
        Ok(v)
    }
    fn act(&self, action: &Action) -> Result<Value> {
        match action {
            Action::StartPrint { filename } => {
                crate::moonraker::check_gcode_name(filename)?;
                self.client.post(
                    &format!(
                        "/printer/print/start?filename={}",
                        crate::moonraker::encode(filename)
                    ),
                    None,
                )
            }
            other => {
                let script = Self::script_for(other).ok_or("that action has no Klipper script")?;
                self.client
                    .post("/printer/gcode/script", Some(&json!({"script": script})))
            }
        }
    }
    fn emergency_stop(&self) -> Result<Value> {
        self.client.post("/printer/emergency_stop", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klipper_allows_every_command_and_an_empty_driver_allows_none() {
        let all = Capabilities::klipper();
        let none = Capabilities::none();
        for id in [
            "set_heater",
            "set_fan",
            "speed_factor",
            "extrude_factor",
            "jog",
            "extrude",
            "z_offset",
            "z_offset_save",
            "limit",
            "pressure_advance",
            "macro",
            "console",
            "pause",
            "resume",
            "cancel",
            "firmware_restart",
            "motors_off",
            "position",
            "probe_query",
            "home",
            "qgl",
            "mesh",
            "probe_accuracy",
            "resonance_x",
            "resonance_y",
            "shaper_x",
            "shaper_y",
            "print_start",
            "print_queue_add",
            "print_queue_remove",
            "exclude_object",
            "klipper_restart",
            "reconnect",
            "load_filament",
            "unload_filament",
            "filament_load",
            "filament_unload",
            "filament_shake",
            "resonance_capture",
            "emergency_stop",
            "config_heater_fan",
        ] {
            assert_eq!(all.allows(id, "Klipper"), Ok(()), "{id}");
            assert!(none.allows(id, "X").is_err(), "{id}");
        }
        assert!(all.allows("not_a_command", "Klipper").is_err());
    }

    /// 2026-10-13: the Kobra X through KX-Bridge answers as Klipper but fills X, Y, velocity and
    /// flow with zeros. Named as itself, it claims none of them, so they show as unknown.
    #[test]
    fn a_bridged_printer_is_named_as_itself_and_claims_no_zeros() {
        let cfg = Config {
            printer_model: Some("Kobra X".into()),
            ..Config::default()
        };
        let d = MoonrakerDriver::new(&cfg);
        assert_eq!(d.brand(), "Anycubic");
        let c = d.capabilities();
        assert!(!c.xy && !c.live_xy && !c.speed && !c.flow && !c.klipper && !c.console);
        assert!(c.pause_resume && c.cancel && c.heat && c.camera);
        let voron = MoonrakerDriver::new(&Config {
            printer_model: Some("Voron 2.4 R2".into()),
            ..Config::default()
        });
        assert_eq!(voron.brand(), "Klipper");
        assert!(voron.capabilities().xy);
    }

    #[test]
    fn a_refusal_names_the_brand_and_the_thing() {
        let caps = Capabilities {
            jog: false,
            ..Capabilities::klipper()
        };
        assert_eq!(
            caps.allows("jog", "PrusaLink"),
            Err("PrusaLink cannot move the head by hand".into())
        );
        assert_eq!(caps.allows("home", "PrusaLink"), Ok(()));
    }

    #[test]
    fn the_neutral_actions_make_the_scripts_commands_rs_makes() {
        assert_eq!(
            MoonrakerDriver::script_for(&Action::SetTemperature {
                heater: Heater::Bed,
                celsius: 55.0
            })
            .unwrap(),
            "SET_HEATER_TEMPERATURE HEATER=heater_bed TARGET=55"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::Fan { fraction: 0.5 }).unwrap(),
            "M106 S128"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::SpeedFactor { percent: 120.0 }).unwrap(),
            "M220 S120"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::FlowFactor { percent: 95.0 }).unwrap(),
            "M221 S95"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::Jog {
                axis: 'X',
                mm: 10.0
            })
            .unwrap(),
            "G91\nG1 X10.00 F6000\nG90"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::Jog {
                axis: 'Z',
                mm: -0.5
            })
            .unwrap(),
            "G91\nG1 Z-0.50 F900\nG90"
        );
        assert_eq!(MoonrakerDriver::script_for(&Action::Home).unwrap(), "G28");
        assert_eq!(
            MoonrakerDriver::script_for(&Action::MotorsOff).unwrap(),
            "M84"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::Extrude {
                mm: 5.0,
                mm_per_s: 3.0
            })
            .unwrap(),
            "M83\nG1 E5.00 F180"
        );
        assert_eq!(
            MoonrakerDriver::script_for(&Action::Cancel).unwrap(),
            "CANCEL_PRINT"
        );
        assert!(MoonrakerDriver::script_for(&Action::StartPrint {
            filename: "a.gcode".into()
        })
        .is_none());
    }
}
