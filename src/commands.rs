use crate::{
    api::print::MAX_GCODE_BYTES,
    api::Response,
    config::Config,
    driver::{self, Action, Capabilities, Heater},
    dry_run,
    moonraker::Moonraker,
    store::db_error,
    Result,
};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Shape of the operator-supplied parameters. Fixed commands carry their whole script;
/// the others render a bounded script from `CommandInput` (see `render`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Params {
    Fixed,
    /// `target` = extruder | heater_bed, `value` = °C
    Heater,
    /// `value` = 0..1 of the part-cooling fan
    Fan,
    /// `value` = percent 50..200 (M220 speed or M221 flow)
    Factor,
    /// `axis` = X | Y | Z, `value` = signed millimetres
    Jog,
    /// `value` = signed millimetres of filament
    Extrude,
    /// `value` = signed millimetres applied to the live Z offset
    ZOffset,
    /// `value` = mm/s velocity, `target` = "velocity" | "accel" | "scv"
    Limit,
    /// `value` = pressure advance in seconds
    Advance,
    /// `target` = extruder | heater_bed, `value` = °C the heater is tuned at (PID_CALIBRATE)
    Pid,
    /// `value` = signed millimetres for the manual Z probe (TESTZ), within ±5
    TestZ,
    /// `params` start and factor (per mm of height) of a pressure-advance tower (TUNING_TOWER)
    Tower,
    /// `target` = an exact macro name reported by gcode/help
    Macro,
    /// `script` = whatever the operator typed in the console
    Console,
    /// `filename` = a gcode file Moonraker lists under the gcodes root
    Filename,
    /// `job_id` = an id reported by /server/job_queue/status
    JobId,
    /// `name` = an object name reported by the `exclude_object` status
    ObjectName,
    /// `speed` = mm/s of filament; the length is fixed per command
    Filament,
    /// `value` = nozzle °C. Runs the printer's own LOAD_FILAMENT / UNLOAD_FILAMENT macro with it
    /// (2026-09-28: the macros park, heat and purge; the raw G1 above is only a hand feed).
    FilamentMacro,
    /// Handled inside Stella Scope; nothing is sent to the printer
    Local,
    /// `axis` = X | Y | belts. Runs TEST_RESONANCES while this service records the ADXL345
    /// stream over Moonraker's Klippy bridge, then computes the PSD and shaper table itself.
    Resonance,
}

/// What a command actually sends. Rendering produces this once, so the preview, the audit line
/// and the request on the wire are all built from the same value and cannot drift apart.
#[derive(Clone, Debug, PartialEq)]
enum Sent {
    /// A G-code script posted to /printer/gcode/script.
    Gcode(String),
    /// A Moonraker API call that is not G-code.
    Api {
        method: &'static str,
        path: String,
        body: Option<Value>,
    },
    /// Stella Scope acts on itself; the printer is not contacted.
    Local(&'static str),
    /// A resonance sweep this service runs itself while recording the accelerometer. The
    /// preview text lists every TEST_RESONANCES leg; the axis drives `resonance::start`.
    Resonance { axis: String, preview: String },
}

impl Sent {
    /// The exact G-code, when there is any. Non-G-code commands have none, and say so.
    fn script(&self) -> Option<&str> {
        match self {
            Sent::Gcode(script) => Some(script),
            Sent::Resonance { preview, .. } => Some(preview),
            _ => None,
        }
    }
    /// One line naming precisely what will happen, for the preview pane and the audit row.
    fn describe(&self) -> String {
        match self {
            Sent::Gcode(script) => script.clone(),
            Sent::Api { method, path, body } => match body {
                Some(body) => format!("{method} {path} {body}"),
                None => format!("{method} {path} (no G-code)"),
            },
            Sent::Local(what) => format!("{what} (local to Stella Scope; nothing is sent)"),
            Sent::Resonance { preview, .. } => preview.clone(),
        }
    }
}

#[derive(Clone, Copy)]
struct Command {
    id: &'static str,
    script: Option<&'static str>,
    capability: Option<&'static str>,
    tier: &'static str,
    requires_homed: bool,
    params: Params,
}

const NOZZLE_MAX_C: f64 = 290.0;
const EXTRUDE_MAX_MM: f64 = 200.0;
const ZOFFSET_STEP_MAX_MM: f64 = 1.0;
const CONSOLE_MAX_CHARS: usize = 256;
const BED_MAX_C: f64 = 120.0;
const JOG_MAX_MM: f64 = 50.0;
/// Filament feed in mm/s. The existing defaults are F180 and F300, i.e. 3 and 5 mm/s; 25 mm/s is
/// faster than any sane hand feed and well under what a Clockwork2 will skip at.
const FEED_MIN_MMS: f64 = 1.0;
const FEED_MAX_MMS: f64 = 25.0;
/// Lengths taken from Stella's own LOAD_FILAMENT / UNLOAD_FILAMENT macros: purge 50, retract 100.
const LOAD_MM: f64 = 50.0;
const UNLOAD_MM: f64 = 100.0;
/// Below this no filament moves: Klipper's own min_extrude_temp is 170 by default.
const FILAMENT_MIN_C: f64 = 150.0;
const FILENAME_MAX_CHARS: usize = 200;
const MACRO_PARAMS_MAX: usize = 12;

/// Commands that skip the review modal and go straight out. Their tier is unchanged, so every
/// safety gate still applies: blocked while printing or paused, `confirm: true` still required on
/// the wire, preflight still refuses QGL until the axes are homed. The only thing dropped is the
/// dialog. Owner rule 2026-09-21, after being asked to confirm a Z jog: "I should not have to do
/// this on any of it." The emergency stop is always on the header, so a dialog in front of a
/// named, fixed action buys nothing.
///
/// What is deliberately NOT here, and why: `console` and `macro` send free text, so the preview is
/// the only place the exact G-code is ever seen; `cancel` throws away a running print;
/// `firmware_restart` and `restart` restart Klipper; `z_offset_save` rewrites printer.cfg. Those
/// five are not "press the button again", they are one-way, so they still ask once.
const NO_REVIEW: [&str; 12] = [
    "home",
    "qgl",
    "jog",
    "motors_off",
    "mesh",
    "probe_accuracy",
    "resonance_x",
    "resonance_y",
    "resonance_capture",
    "filament_load",
    "filament_unload",
    "filament_shake",
];
/// Tuning commands that never wait for a running macro. 2026-09-28: bed heat was refused while a
/// quad gantry level ran, because the level's own request holds the "one ordinary command" slot until
/// Klipper finishes the macro (minutes). Klipper takes a heater, fan, speed or flow change at any
/// time, so these use their own slot: two of them still never overlap, but a long macro cannot
/// lock them out.
const LIGHT: [&str; 4] = ["set_heater", "set_fan", "speed_factor", "extrude_factor"];
/// 2026-10-22 (filament board): after a load the head gives a quick shake to flick the filament tail off the nozzle. Small fast moves along X, in place: +2 -4 +4 -4 +4 -4 +2 is
/// net zero, relative moves, and the printer's own G-code state (absolute or relative, units) is saved first and put back after.
const SHAKE_SCRIPT: &str = "SAVE_GCODE_STATE NAME=STELLA_SHAKE\nG91\nG1 X2 F12000\nG1 X-4 F12000\nG1 X4 F12000\nG1 X-4 F12000\nG1 X4 F12000\nG1 X-4 F12000\nG1 X2 F12000\nRESTORE_GCODE_STATE NAME=STELLA_SHAKE";
/// The shake's moves for a head at `x` on travel `min..max`: SHAKE_SCRIPT when there are 2 mm (and a half) each side, else six 4 mm moves on the side
/// that has room, still net zero. None when neither side has room.
fn shake_script(x: f64, min: f64, max: f64) -> Option<String> {
    let moves: &[i32] = if x - min >= 2.5 && max - x >= 2.5 {
        &[2, -4, 4, -4, 4, -4, 2]
    } else if max - x >= 4.5 {
        &[4, -4, 4, -4, 4, -4]
    } else if x - min >= 4.5 {
        &[-4, 4, -4, 4, -4, 4]
    } else {
        return None;
    };
    let body: String = moves.iter().map(|m| format!("G1 X{m} F12000\n")).collect();
    Some(format!("SAVE_GCODE_STATE NAME=STELLA_SHAKE\nG91\n{body}RESTORE_GCODE_STATE NAME=STELLA_SHAKE"))
}
const COMMANDS: &[Command] = &[
    Command {
        id: "set_heater",
        script: None,
        capability: Some("SET_HEATER_TEMPERATURE"),
        tier: "T2",
        requires_homed: false,
        params: Params::Heater,
    },
    Command {
        id: "set_fan",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Fan,
    },
    Command {
        id: "speed_factor",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Factor,
    },
    Command {
        id: "extrude_factor",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Factor,
    },
    Command {
        id: "jog",
        script: None,
        capability: None,
        tier: "T3",
        requires_homed: true,
        params: Params::Jog,
    },
    Command {
        id: "extrude",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Extrude,
    },
    Command {
        id: "z_offset",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::ZOffset,
    },
    Command {
        id: "z_offset_save",
        script: Some("Z_OFFSET_APPLY_PROBE\nSAVE_CONFIG"),
        capability: Some("Z_OFFSET_APPLY_PROBE"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "limit",
        script: None,
        capability: Some("SET_VELOCITY_LIMIT"),
        tier: "T2",
        requires_homed: false,
        params: Params::Limit,
    },
    Command {
        id: "pressure_advance",
        script: None,
        capability: Some("SET_PRESSURE_ADVANCE"),
        tier: "T2",
        requires_homed: false,
        params: Params::Advance,
    },
    // Calibration (2026-10-26). Each heats or moves for minutes, so each is T3: shown first, never while printing.
    Command {
        id: "pid_calibrate",
        script: None,
        capability: Some("PID_CALIBRATE"),
        tier: "T3",
        requires_homed: false,
        params: Params::Pid,
    },
    Command {
        id: "probe_calibrate",
        script: Some("PROBE_CALIBRATE"),
        capability: Some("PROBE_CALIBRATE"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "z_endstop_calibrate",
        script: Some("Z_ENDSTOP_CALIBRATE"),
        capability: Some("Z_ENDSTOP_CALIBRATE"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    // the manual probe's own words exist only while a probe session is open, so they are not looked up in gcode/help
    Command {
        id: "testz",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::TestZ,
    },
    Command {
        id: "manual_accept",
        script: Some("ACCEPT"),
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "manual_abort",
        script: Some("ABORT"),
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "tuning_tower",
        script: None,
        capability: Some("TUNING_TOWER"),
        tier: "T3",
        requires_homed: false,
        params: Params::Tower,
    },
    // writes the results above into printer.cfg and restarts Klipper
    Command {
        id: "save_config",
        script: Some("SAVE_CONFIG"),
        capability: Some("SAVE_CONFIG"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "macro",
        script: None,
        capability: None,
        tier: "T3",
        requires_homed: false,
        params: Params::Macro,
    },
    Command {
        id: "console",
        script: None,
        capability: None,
        tier: "T3",
        requires_homed: false,
        params: Params::Console,
    },
    Command {
        id: "pause",
        script: Some("PAUSE"),
        capability: Some("PAUSE"),
        tier: "T2",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "resume",
        script: Some("RESUME"),
        capability: Some("RESUME"),
        tier: "T2",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "cancel",
        script: Some("CANCEL_PRINT"),
        capability: Some("CANCEL_PRINT"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "firmware_restart",
        script: Some("FIRMWARE_RESTART"),
        capability: Some("FIRMWARE_RESTART"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "motors_off",
        script: Some("M84"),
        capability: None,
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "position",
        script: Some("GET_POSITION"),
        capability: Some("GET_POSITION"),
        tier: "T1",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "probe_query",
        script: Some("QUERY_PROBE"),
        capability: Some("QUERY_PROBE"),
        tier: "T1",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "home",
        script: Some("G28"),
        capability: Some("G28"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "qgl",
        script: Some("QUAD_GANTRY_LEVEL"),
        capability: Some("QUAD_GANTRY_LEVEL"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "mesh",
        script: Some("BED_MESH_CALIBRATE"),
        capability: Some("BED_MESH_CALIBRATE"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "probe_accuracy",
        script: Some("PROBE_ACCURACY"),
        capability: Some("PROBE_ACCURACY"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "resonance_x",
        script: Some("TEST_RESONANCES AXIS=X"),
        capability: Some("TEST_RESONANCES"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "resonance_y",
        script: Some("TEST_RESONANCES AXIS=Y"),
        capability: Some("TEST_RESONANCES"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "shaper_x",
        script: Some("SHAPER_CALIBRATE AXIS=X"),
        capability: Some("SHAPER_CALIBRATE"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "shaper_y",
        script: Some("SHAPER_CALIBRATE AXIS=Y"),
        capability: Some("SHAPER_CALIBRATE"),
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    // L3 print control.
    Command {
        id: "print_start",
        script: None,
        capability: None,
        tier: "T3",
        requires_homed: false,
        params: Params::Filename,
    },
    // WO-02: a dry run uploads a no-extrusion copy of the file and starts it, so the printer moves
    // every move without printing anything. Refused while printing (T3) and on a printer that cannot
    // start a file (start_print capability).
    Command {
        id: "dry_run",
        script: None,
        capability: None,
        tier: "T3",
        requires_homed: false,
        params: Params::Filename,
    },
    Command {
        id: "print_queue_add",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Filename,
    },
    Command {
        id: "print_queue_remove",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::JobId,
    },
    // T2 on purpose: cancelling an object is only ever useful while the print is running, and the
    // T3 gate blocks everything while printing.
    Command {
        id: "exclude_object",
        script: None,
        capability: Some("EXCLUDE_OBJECT"),
        tier: "T2",
        requires_homed: false,
        params: Params::ObjectName,
    },
    // L4 restart, reconnect and filament.
    Command {
        id: "klipper_restart",
        script: Some("RESTART"),
        capability: Some("RESTART"),
        tier: "T3",
        requires_homed: false,
        params: Params::Fixed,
    },
    Command {
        id: "reconnect",
        script: None,
        capability: None,
        tier: "T1",
        requires_homed: false,
        params: Params::Local,
    },
    Command {
        id: "load_filament",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Filament,
    },
    Command {
        id: "unload_filament",
        script: None,
        capability: None,
        tier: "T2",
        requires_homed: false,
        params: Params::Filament,
    },
    // 2026-09-28: Load filament never reached the printer. These are the named, fixed actions
    // the dock's Load and Unload buttons and the filament page send: LOAD_FILAMENT TEMP=<n>. T3, so
    // they are refused mid-print, and in NO_REVIEW like Home: the macro name is fixed, the only
    // operand is a bounded temperature, and a dialog in front of it buys nothing.
    Command {
        id: "filament_load",
        script: None,
        capability: Some("LOAD_FILAMENT"),
        tier: "T3",
        requires_homed: false,
        params: Params::FilamentMacro,
    },
    Command {
        id: "filament_unload",
        script: None,
        capability: Some("UNLOAD_FILAMENT"),
        tier: "T3",
        requires_homed: false,
        params: Params::FilamentMacro,
    },
    // The shake after a load. T3 (refused while a print runs or is paused) and homed only (the head must know where it is), no review: fixed moves, no operand.
    Command {
        id: "filament_shake",
        script: Some(SHAKE_SCRIPT),
        capability: None,
        tier: "T3",
        requires_homed: true,
        params: Params::Fixed,
    },
    Command {
        id: "resonance_capture",
        script: None,
        capability: Some("TEST_RESONANCES"),
        tier: "T3",
        requires_homed: true,
        params: Params::Resonance,
    },
    Command {
        id: "emergency_stop",
        script: None,
        capability: None,
        tier: "EMERGENCY",
        requires_homed: false,
        params: Params::Fixed,
    },
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandInput {
    id: String,
    #[serde(default)]
    confirm: bool,
    /// Heater name for `set_heater`.
    #[serde(default)]
    target: Option<String>,
    /// Numeric parameter: °C, fan fraction, percent or millimetres.
    #[serde(default)]
    value: Option<f64>,
    /// Jog axis.
    #[serde(default)]
    axis: Option<String>,
    /// Raw console line, or a macro's parameters as one already-formatted line.
    #[serde(default)]
    script: Option<String>,
    /// Filament or extrusion feed in mm/s.
    #[serde(default)]
    speed: Option<f64>,
    /// A gcode file under Moonraker's gcodes root.
    #[serde(default)]
    filename: Option<String>,
    /// Heat the printer as normal during the dry run (default off: no M104/M140/M109/M190).
    #[serde(default)]
    heat: bool,
    /// A job id from /server/job_queue/status.
    #[serde(default)]
    job_id: Option<String>,
    /// An object name from the `exclude_object` status.
    #[serde(default)]
    name: Option<String>,
    /// A macro's parameters as name/value pairs; rendered `NAME K=V` in sorted order.
    #[serde(default)]
    params: Option<BTreeMap<String, String>>,
}

/// A value Klipper will accept after `KEY=`. Anything that could start a second command, a
/// comment or a Jinja expression is refused rather than escaped.
fn macro_value_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+:/".contains(&b))
}

/// A gcode filename as Moonraker lists it: a relative path under the gcodes root.
fn filename_ok(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= FILENAME_MAX_CHARS
        && !filename.contains("..")
        && !filename.starts_with('/')
        && !filename.contains(['?', '#', '\r', '\n', '\\', '"'])
        && !filename.chars().any(char::is_control)
        && (filename.to_ascii_lowercase().ends_with(".gcode")
            || filename.to_ascii_lowercase().ends_with(".bgcode"))
}

/// WO-02: the dry-run copy's name shows "dry run" on the printer's screen and in the history so it
/// is never mistaken for a print. `cube.gcode` becomes `cube (dry run).gcode`.
///
/// Finish-2 F07 (2026-10-19): the copy keeps the file's own name, capitals and all. It was lowercased ("voron cube v7 pla stella fast (dry run)")
/// because the extension was cut from a lowercased copy of the whole name.
pub(crate) fn dry_run_name(filename: &str) -> String {
    let cut = |ext: &str| {
        let at = filename.len().checked_sub(ext.len())?;
        (filename.is_char_boundary(at) && filename[at..].eq_ignore_ascii_case(ext)).then(|| &filename[..at])
    };
    let (stem, ext) = if let Some(rest) = cut(".gcode") {
        (rest, ".gcode")
    } else if let Some(rest) = cut(".bgcode") {
        (rest, ".bgcode")
    } else {
        return format!("{filename} (dry run)");
    };
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe} (dry run){ext}")
}

/// The file a dry run's copy was made from, by name: `cube (dry run).gcode` is `cube.gcode`. None for a name that is not a dry run's copy.
/// A copy has no picture of its own; the page and the farm wall ask for the original's (finish-2 F07, 2026-10-19).
pub fn dry_run_original(filename: &str) -> Option<String> {
    let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
    let base = stem.strip_suffix(" (dry run)")?;
    Some(if ext.is_empty() { base.to_owned() } else { format!("{base}.{ext}") })
}

/// What the Calibrate page needs of printer.cfg: the values each calibration writes, whether a SAVE_CONFIG is waiting, and which
/// optional sections exist (a probe, a gantry level, a mesh, an input shaper). Only those values; never the rest of the file.
fn calibration_view(configfile: &Value) -> Value {
    let s = &configfile["settings"];
    let num = |sec: &str, key: &str| s[sec][key].as_f64();
    let probe = ["probe", "bltouch", "beacon", "cartographer", "scanner", "smart_effector", "eddy_probe"]
        .iter()
        .find(|k| s.get(**k).is_some())
        .copied();
    json!({
        "extruder": {"rotation_distance": num("extruder", "rotation_distance"), "gear_ratio": s["extruder"]["gear_ratio"].clone(),
            "pressure_advance": num("extruder", "pressure_advance"), "control": s["extruder"]["control"].clone(),
            "pid_kp": num("extruder", "pid_kp"), "pid_ki": num("extruder", "pid_ki"), "pid_kd": num("extruder", "pid_kd"),
            "nozzle_diameter": num("extruder", "nozzle_diameter")},
        "heater_bed": {"control": s["heater_bed"]["control"].clone(), "pid_kp": num("heater_bed", "pid_kp"), "pid_ki": num("heater_bed", "pid_ki"), "pid_kd": num("heater_bed", "pid_kd")},
        "probe": probe.map(|p| json!({"kind": p, "z_offset": num(p, "z_offset")})),
        "z_endstop": num("stepper_z", "position_endstop"),
        "quad_gantry_level": s.get("quad_gantry_level").is_some(),
        "z_tilt": s.get("z_tilt").is_some(),
        "bed_mesh": s.get("bed_mesh").is_some(),
        "input_shaper": s.get("input_shaper").map(|i| json!({"x": i["shaper_type_x"], "fx": i["shaper_freq_x"], "y": i["shaper_type_y"], "fy": i["shaper_freq_y"]})),
        "save_config_pending": configfile["save_config_pending"].as_bool().unwrap_or(false),
        "pending": configfile["save_config_pending_items"].clone(),
    })
}

/// Render the exact script for a command from operator input, refusing anything out of bounds.
/// Fixed commands ignore the input. Nothing here touches the network.
fn render(command: Command, input: &CommandInput) -> Result<Sent> {
    let value = || {
        input
            .value
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{} needs a finite numeric value", command.id))
    };
    // Feed in mm/s -> the F word in mm/min. Absent means the command's own default.
    let feed = |default_mms: f64| -> Result<f64> {
        let mms = input.speed.unwrap_or(default_mms);
        if !mms.is_finite() || !(FEED_MIN_MMS..=FEED_MAX_MMS).contains(&mms) {
            return Err(format!(
                "{} speed must be {FEED_MIN_MMS} to {FEED_MAX_MMS} mm/s",
                command.id
            ));
        }
        Ok(mms * 60.0)
    };
    Ok(match command.params {
        Params::Fixed => Sent::Gcode(command.script.ok_or("command has no script")?.to_owned()),
        Params::Heater => {
            let (heater, max) = match input.target.as_deref() {
                Some("extruder") => ("extruder", NOZZLE_MAX_C),
                Some("heater_bed") => ("heater_bed", BED_MAX_C),
                _ => return Err("set_heater target must be extruder or heater_bed".into()),
            };
            let celsius = value()?;
            if !(0.0..=max).contains(&celsius) {
                return Err(format!("{heater} target must be 0 to {max} °C"));
            }
            Sent::Gcode(format!(
                "SET_HEATER_TEMPERATURE HEATER={heater} TARGET={:.0}",
                celsius
            ))
        }
        Params::Fan => {
            let fraction = value()?;
            if !(0.0..=1.0).contains(&fraction) {
                return Err("set_fan value must be 0 to 1".into());
            }
            Sent::Gcode(format!("M106 S{:.0}", fraction * 255.0))
        }
        Params::Factor => {
            let percent = value()?;
            if !(50.0..=200.0).contains(&percent) {
                return Err(format!("{} must be 50 to 200 percent", command.id));
            }
            let code = if command.id == "speed_factor" {
                "M220"
            } else {
                "M221"
            };
            Sent::Gcode(format!("{code} S{:.0}", percent))
        }
        Params::Extrude => {
            let mm = value()?;
            if mm == 0.0 || mm.abs() > EXTRUDE_MAX_MM {
                return Err(format!(
                    "extrude amount must be non-zero and within ±{EXTRUDE_MAX_MM} mm"
                ));
            }
            // Klipper refuses below min_extrude_temp itself; this only bounds the amount and feed.
            // Without an explicit speed the original amount-derived feeds stand: 3 mm/s, or
            // 5 mm/s past 20 mm, which is what the HUD sent before speed was a control.
            let feed = match input.speed {
                Some(mms) => feed(mms)?,
                None if mm.abs() > 20.0 => 300.0,
                None => 180.0,
            };
            Sent::Gcode(format!("M83\nG1 E{mm:.2} F{feed:.0}"))
        }
        Params::Filament => {
            let mm = if command.id == "load_filament" {
                LOAD_MM
            } else {
                -UNLOAD_MM
            };
            let feed = feed(5.0)?;
            Sent::Gcode(format!("M83\nG1 E{mm:.2} F{feed:.0}"))
        }
        Params::FilamentMacro => {
            let celsius = value()?;
            if !(FILAMENT_MIN_C..=NOZZLE_MAX_C).contains(&celsius) {
                return Err(format!(
                    "{} needs a nozzle temperature of {FILAMENT_MIN_C} to {NOZZLE_MAX_C} °C",
                    command.id
                ));
            }
            let macro_name = if command.id == "filament_load" {
                "LOAD_FILAMENT"
            } else {
                "UNLOAD_FILAMENT"
            };
            Sent::Gcode(format!("{macro_name} TEMP={celsius:.0}"))
        }
        Params::ZOffset => {
            let mm = value()?;
            if mm == 0.0 || mm.abs() > ZOFFSET_STEP_MAX_MM {
                return Err(format!(
                    "z offset step must be non-zero and within ±{ZOFFSET_STEP_MAX_MM} mm"
                ));
            }
            Sent::Gcode(format!("SET_GCODE_OFFSET Z_ADJUST={mm:.3} MOVE=1"))
        }
        Params::Limit => {
            let v = value()?;
            Sent::Gcode(match input.target.as_deref() {
                Some("velocity") if (1.0..=1000.0).contains(&v) => {
                    format!("SET_VELOCITY_LIMIT VELOCITY={v:.0}")
                }
                Some("accel") if (100.0..=25000.0).contains(&v) => {
                    format!("SET_VELOCITY_LIMIT ACCEL={v:.0}")
                }
                Some("scv") if (0.5..=30.0).contains(&v) => {
                    format!("SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY={v:.1}")
                }
                _ => return Err("limit target must be velocity, accel or scv, and in range".into()),
            })
        }
        Params::Advance => {
            let v = value()?;
            if !(0.0..=1.0).contains(&v) {
                return Err("pressure advance must be 0 to 1 second".into());
            }
            Sent::Gcode(format!("SET_PRESSURE_ADVANCE ADVANCE={v:.4}"))
        }
        // Calibration (2026-10-26: "you likely don't even have calibration ... pre-calibration setup").
        Params::Pid => {
            let (heater, lo, hi) = match input.target.as_deref() {
                Some("extruder") => ("extruder", 150.0, NOZZLE_MAX_C),
                Some("heater_bed") => ("heater_bed", 40.0, BED_MAX_C),
                _ => return Err("pid_calibrate target must be extruder or heater_bed".into()),
            };
            let celsius = value()?;
            if !(lo..=hi).contains(&celsius) {
                return Err(format!("{heater} is tuned at {lo:.0} to {hi:.0} °C"));
            }
            Sent::Gcode(format!("PID_CALIBRATE HEATER={heater} TARGET={celsius:.0}"))
        }
        Params::TestZ => {
            let mm = value()?;
            if mm == 0.0 || mm.abs() > 5.0 {
                return Err("a probe step must be non-zero and within ±5 mm".into());
            }
            Sent::Gcode(format!("TESTZ Z={mm:+.3}"))
        }
        Params::Tower => {
            let num = |k: &str| -> Result<f64> {
                input
                    .params
                    .as_ref()
                    .and_then(|m| m.get(k))
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("tuning_tower needs a numeric {k}"))
            };
            let (start, factor) = (num("start")?, num("factor")?);
            if !(0.0..=0.5).contains(&start) || !(0.0005..=0.05).contains(&factor) {
                return Err("a pressure advance tower starts at 0 to 0.5 and rises 0.0005 to 0.05 a millimetre".into());
            }
            Sent::Gcode(format!("TUNING_TOWER COMMAND=SET_PRESSURE_ADVANCE PARAMETER=ADVANCE START={start:.4} FACTOR={factor:.5}"))
        }
        Params::Macro => {
            let name = input
                .target
                .as_deref()
                .ok_or("macro needs a name reported by gcode/help")?;
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err("macro name must be the exact uppercase name from gcode/help".into());
            }
            // Two ways in: `params` as a map, which is what the HUD sends now, or the older
            // pre-formatted `script` line. A request may use one or the other, never both, so
            // there is never a question of which one produced the script that was sent.
            let args = match (&input.params, input.script.as_deref()) {
                (Some(_), Some(_)) => {
                    return Err("send macro parameters as params or script, not both".into())
                }
                (Some(map), None) => {
                    if map.len() > MACRO_PARAMS_MAX {
                        return Err(format!(
                            "a macro call carries at most {MACRO_PARAMS_MAX} parameters"
                        ));
                    }
                    let mut pairs = Vec::with_capacity(map.len());
                    for (key, value) in map {
                        if key.is_empty()
                            || key.len() > 32
                            || !key
                                .bytes()
                                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                        {
                            return Err(format!(
                                "macro parameter name {key:?} must be uppercase letters, digits or underscore"
                            ));
                        }
                        if !macro_value_ok(value) {
                            return Err(format!(
                                "macro parameter {key} has a value Klipper would not accept verbatim"
                            ));
                        }
                        pairs.push(format!("{key}={value}"));
                    }
                    // BTreeMap iterates sorted, so the same request always renders the same line.
                    pairs.join(" ")
                }
                (None, Some(script)) => script.to_owned(),
                (None, None) => String::new(),
            };
            if args.len() > CONSOLE_MAX_CHARS || args.contains('\n') {
                return Err("macro parameters must be one short line".into());
            }
            Sent::Gcode(if args.is_empty() {
                name.to_owned()
            } else {
                format!("{name} {args}")
            })
        }
        Params::Console => {
            let line = input
                .script
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .ok_or("type a G-code line first")?;
            if line.len() > CONSOLE_MAX_CHARS {
                return Err(format!(
                    "console lines are limited to {CONSOLE_MAX_CHARS} characters"
                ));
            }
            if line.contains('\n') || line.contains('\r') {
                return Err("send one line at a time".into());
            }
            Sent::Gcode(line.to_owned())
        }
        // Moonraker's own print and queue APIs; none of these is G-code.
        Params::Filename => {
            let filename = input
                .filename
                .as_deref()
                .map(str::trim)
                .ok_or("a .gcode filename as Moonraker lists it is required")?;
            if !filename_ok(filename) {
                return Err(format!(
                    "{filename:?} is not a .gcode path under the gcodes root; pick one from /api/files"
                ));
            }
            if command.id == "print_start" {
                Sent::Api {
                    method: "POST",
                    path: format!(
                        "/printer/print/start?filename={}",
                        crate::moonraker::encode(filename)
                    ),
                    body: None,
                }
            } else {
                Sent::Api {
                    method: "POST",
                    path: "/server/job_queue/job".into(),
                    body: Some(json!({"filenames":[filename]})),
                }
            }
        }
        Params::JobId => {
            let job_id = input
                .job_id
                .as_deref()
                .map(str::trim)
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 64
                        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
                .ok_or("job_id must be an id reported by GET /api/print/queue")?;
            Sent::Api {
                method: "DELETE",
                path: format!(
                    "/server/job_queue/job?job_ids={}",
                    crate::moonraker::encode(job_id)
                ),
                body: None,
            }
        }
        Params::ObjectName => {
            // Object names come from the slicer via EXCLUDE_OBJECT_DEFINE and can carry spaces
            // and dots. Klipper reads NAME= to the end of the line, so a newline is the only
            // character that could smuggle a second command, and quotes are refused with it.
            let name = input
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| {
                    !n.is_empty()
                        && n.len() <= 128
                        && !n.chars().any(char::is_control)
                        && !n.contains('"')
                })
                .ok_or("name must be an object reported by GET /api/print/objects")?;
            Sent::Gcode(format!("EXCLUDE_OBJECT NAME={name}"))
        }
        Params::Local => Sent::Local("reconnect the Moonraker websocket source"),
        Params::Resonance => {
            // The rendered text is the preview: every sweep this capture will run, in order.
            // It is never POSTed as one script — `resonance::start` runs each leg itself, with
            // the accelerometer already subscribed, so the samples and the motion line up.
            let axis = input
                .axis
                .as_deref()
                .ok_or("resonance_capture needs axis X, Y or belts")?;
            let preview = crate::resonance::legs(axis)?
                .iter()
                .map(|(_, script)| *script)
                .collect::<Vec<_>>()
                .join("\n");
            Sent::Resonance {
                axis: axis.to_owned(),
                preview,
            }
        }
        Params::Jog => {
            let axis = match input.axis.as_deref() {
                Some("X") => "X",
                Some("Y") => "Y",
                Some("Z") => "Z",
                _ => return Err("jog axis must be X, Y or Z".into()),
            };
            let mm = value()?;
            if mm == 0.0 || mm.abs() > JOG_MAX_MM {
                return Err(format!(
                    "jog distance must be non-zero and within ±{JOG_MAX_MM} mm"
                ));
            }
            let feed = if axis == "Z" { 900 } else { 6000 };
            Sent::Gcode(format!("G91\nG1 {axis}{mm:.2} F{feed}\nG90"))
        }
    })
}

/// The driver-neutral action a rendered command means (farm board WO-02). `render` has already
/// bounded every operand and produced Klipper's own text; this reads that text back, so a printer
/// of another brand gets exactly the validation Klipper does. `None` is a command with no neutral
/// meaning: the service then reports that this printer cannot do it.

/// What an action does, in the words the review shows on a printer that is not Klipper (finish-2 F02, CORE One L,
/// 2026-10-19): Pause, Resume and Cancel showed Klipper's own macros (PAUSE, RESUME, CANCEL_PRINT), which a PrusaLink or a
/// FlashForge is never sent; it is sent the action.
/// The "!!" error a macro printed after the last `command` line of the console, if it printed one (see `execute_with`).
fn klipper_refusal(moonraker: &Moonraker, command: &str) -> Option<String> {
    let store = moonraker.get("/server/gcode_store?count=30").ok()?;
    let lines = store["gcode_store"].as_array()?;
    let start = lines
        .iter()
        .rposition(|l| l["type"] == "command" && l["message"].as_str().is_some_and(|m| m.trim().eq_ignore_ascii_case(command)))?;
    lines[start + 1..].iter().find_map(|l| {
        let m = l["message"].as_str()?.trim();
        m.strip_prefix("!!").map(|rest| rest.trim_matches(|c: char| c == '!' || c.is_whitespace()).to_owned())
    })
}

/// How long a command may take to answer (see `execute_with`): Klipper's long procedures get half an hour.
fn command_timeout_ms(id: &str, normal: u64) -> u64 {
    if matches!(id, "mesh" | "qgl" | "probe_accuracy" | "macro" | "home") {
        normal.max(30 * 60 * 1000)
    } else {
        normal
    }
}

fn action_words(a: &Action) -> String {
    match a {
        Action::SetTemperature { heater, celsius } => {
            let what = if matches!(heater, Heater::Nozzle) { "nozzle" } else { "bed" };
            if *celsius <= 0.0 { format!("Turn the {what} heater off.") } else { format!("Heat the {what} to {celsius:.0} °C.") }
        }
        Action::Fan { fraction } => format!("Set the part fan to {:.0} %.", fraction * 100.0),
        Action::SpeedFactor { percent } => format!("Set the print speed to {percent:.0} %."),
        Action::FlowFactor { percent } => format!("Set the flow to {percent:.0} %."),
        Action::Jog { axis, mm } => format!("Move {axis} by {mm:+.1} mm."),
        Action::Home => "Home every axis.".into(),
        Action::MotorsOff => "Turn the motors off.".into(),
        Action::Extrude { mm, .. } => format!("Push {mm:.0} mm of filament through the nozzle."),
        Action::Pause => "Pause the print.".into(),
        Action::Resume => "Resume the print.".into(),
        Action::Cancel => "Cancel the print. It cannot be resumed.".into(),
        Action::StartPrint { filename } => format!("Start \"{filename}\" on the printer, once."),
        Action::Gcode(g) => g.clone(),
    }
}

fn action_from_sent(command: Command, sent: &Sent, input: &CommandInput) -> Option<Action> {
    let script = match sent {
        Sent::Gcode(script) => script.as_str(),
        Sent::Api { .. } if command.id == "print_start" => {
            return input.filename.as_deref().map(|f| Action::StartPrint {
                filename: f.trim().to_owned(),
            });
        }
        _ => return None,
    };
    let after = |text: &str, key: &str| -> Option<f64> {
        text.split_whitespace()
            .find_map(|t| t.strip_prefix(key))
            .and_then(|v| v.parse::<f64>().ok())
    };
    Some(match command.id {
        "set_heater" => {
            let celsius = after(script, "TARGET=")?;
            let heater = if script.contains("HEATER=extruder") {
                Heater::Nozzle
            } else {
                Heater::Bed
            };
            Action::SetTemperature { heater, celsius }
        }
        "set_fan" => Action::Fan {
            fraction: after(script, "S")? / 255.0,
        },
        "speed_factor" => Action::SpeedFactor {
            percent: after(script, "S")?,
        },
        "extrude_factor" => Action::FlowFactor {
            percent: after(script, "S")?,
        },
        "jog" => {
            let line = script.lines().nth(1)?;
            let token = line
                .split_whitespace()
                .find(|t| t.starts_with(['X', 'Y', 'Z']))?;
            Action::Jog {
                axis: token.chars().next()?,
                mm: token[1..].parse().ok()?,
            }
        }
        "home" => Action::Home,
        "motors_off" => Action::MotorsOff,
        "extrude" | "load_filament" | "unload_filament" => {
            let line = script.lines().nth(1)?;
            Action::Extrude {
                mm: after(line, "E")?,
                mm_per_s: after(line, "F")? / 60.0,
            }
        }
        "pause" => Action::Pause,
        "resume" => Action::Resume,
        "cancel" => Action::Cancel,
        "console" => Action::Gcode(script.to_owned()),
        _ => return None,
    })
}

struct FreshState {
    help: BTreeSet<String>,
    homed_axes: String,
    print_state: String,
    idle_state: String,
}

/// Reviewer's decision on WO-20 (2026-09-23): while Moonraker answers and Klipper is in shutdown or
/// error, RESTART and FIRMWARE_RESTART still go, after the same one review. They are how a user gets
/// out of "Lost communication with MCU", and Mainsail and Fluidd offer them in that state. Klipper
/// registers both before it reads its config, so they work there. Everything else stays refused.
const RECOVERY: [&str; 2] = ["firmware_restart", "klipper_restart"];

/// What the printer can take right now, read fresh for every decision.
enum Fresh {
    Ready(FreshState),
    /// Klipper reports shutdown or error: only the RECOVERY commands may go.
    Halted(String),
}

impl Fresh {
    fn refusal(&self, command: Command) -> Option<String> {
        match self {
            Fresh::Ready(state) => state.refusal(command),
            Fresh::Halted(_) if RECOVERY.contains(&command.id) => None,
            Fresh::Halted(state) => Some(format!(
                "Klippy state is {state}, not ready; command was not sent"
            )),
        }
    }
}

impl FreshState {
    fn fetch(moonraker: &Moonraker) -> Result<Fresh> {
        let info = moonraker.get("/printer/info")?;
        let state = info["state"]
            .as_str()
            .ok_or("printer info did not report a state; command was not sent")?;
        if ["shutdown", "error"].contains(&state) {
            return Ok(Fresh::Halted(state.to_owned()));
        }
        if state != "ready" {
            return Err(format!(
                "Klippy state is {state}, not ready; command was not sent"
            ));
        }
        let status = moonraker.get(
            "/printer/objects/query?toolhead=homed_axes&print_stats=state&idle_timeout=state",
        )?;
        let status = status["status"]
            .as_object()
            .ok_or("printer status is missing; command was not sent")?;
        let text = |object: &str, field: &str| -> Result<String> {
            status
                .get(object)
                .and_then(|value| value.get(field))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!("fresh {object}.{field} state is missing; command was not sent")
                })
        };
        let help = moonraker.get("/printer/gcode/help")?;
        let help = help
            .as_object()
            .ok_or("G-code help is missing; command was not sent")?
            .keys()
            .cloned()
            .collect();
        Ok(Fresh::Ready(Self {
            help,
            homed_axes: text("toolhead", "homed_axes")?,
            print_state: text("print_stats", "state")?,
            idle_state: text("idle_timeout", "state")?,
        }))
    }

    /// Klipper's `gcode/help` lists only extended and macro commands. Base G-codes such as G28,
    /// M84 and G1 are built into the firmware and never appear there, so checking one against help
    /// refuses a command that is in fact always available. the owner hit this on 2026-09-21: HOME opened
    /// the review modal, Confirm reported "G28 is not reported by current G-code help" and nothing
    /// moved. Read off Stella the same morning: 112 help entries, QUAD_GANTRY_LEVEL present, G28
    /// and M84 absent. The tests that passed through this gate fed a fake help dict containing
    /// G28, which no real Klipper returns.
    fn is_base_gcode(name: &str) -> bool {
        let bytes = name.as_bytes();
        matches!(bytes.first(), Some(b'G' | b'M'))
            && bytes.len() > 1
            && bytes[1..].iter().all(|b| b.is_ascii_digit() || *b == b'.')
    }

    fn refusal(&self, command: Command) -> Option<String> {
        if command.capability.is_some_and(|capability| {
            !Self::is_base_gcode(capability) && !self.help.contains(capability)
        }) {
            return Some(format!(
                "{} is not reported by current G-code help; command was not sent",
                command.capability.unwrap_or("command")
            ));
        }
        let print_state = self.print_state.to_ascii_lowercase();
        let idle_state = self.idle_state.to_ascii_lowercase();
        if ![
            "standby",
            "printing",
            "paused",
            "complete",
            "cancelled",
            "error",
        ]
        .contains(&print_state.as_str())
            || !["ready", "printing", "idle"].contains(&idle_state.as_str())
        {
            return Some(format!(
                "printer reported unknown print/idle state {}/{}; command was not sent",
                self.print_state, self.idle_state
            ));
        }
        // NEXT review 2026-09-24: CANCEL_PRINT only means anything during a print, so it is the one
        // T3 command this block must not stop (it had made Cancel impossible in both HUDs). It is
        // still T3: it still asks for review before it goes.
        if command.tier == "T3"
            && command.id != "cancel"
            && (["printing", "paused"].contains(&print_state.as_str())
                || ["printing", "paused"].contains(&idle_state.as_str()))
        {
            // Finish-2 F05 (Stella, 2026-10-19): with no print running, Klipper's idle state reads "Printing" only while it is
            // still carrying out the last command (a move, a heat-up); say that instead of "printing".
            if !["printing", "paused"].contains(&print_state.as_str()) {
                return Some("The printer is still busy with the last command. Send this when it has finished.".into());
            }
            return Some(format!(
                "printer state is {}/{}; T3 commands are blocked while printing or paused",
                self.print_state, self.idle_state
            ));
        }
        if command.requires_homed
            && !['x', 'y', 'z']
                .iter()
                .all(|axis| self.homed_axes.contains(*axis))
        {
            return Some(format!(
                "all axes must be homed before {}; current homed_axes is {:?}",
                command.id, self.homed_axes
            ));
        }
        None
    }
}

/// Every control route and the methods it answers. Anything not listed here never reaches the
/// service, and a listed route that gets the wrong verb is a 405 before any state is touched.
pub const ROUTES: &[(&str, &[&str])] = &[
    ("/api/controls", &["GET"]),
    ("/api/macros", &["GET"]),
    ("/api/files", &["GET"]),
    ("/api/commands", &["POST"]),
    ("/api/command-preview", &["POST"]),
    ("/api/config/heater-fan", &["GET", "POST"]),
    ("/api/config/heater-fan/apply", &["POST"]),
    ("/api/config/calibration", &["GET"]),
    ("/api/print/queue", &["GET"]),
    ("/api/print/objects", &["GET"]),
    ("/api/presets", &["GET", "PUT"]),
    ("/api/presets/standard", &["GET"]),
    ("/api/console/help", &["GET"]),
    ("/api/console/history", &["GET", "POST"]),
    ("/api/filament/guard", &["POST"]),
];

pub fn routed(path: &str) -> Option<&'static [&'static str]> {
    ROUTES
        .iter()
        .find(|(route, _)| *route == path)
        .map(|(_, methods)| *methods)
}

pub struct Service {
    cfg: Config,
    mode: String,
    token: Option<String>,
    busy: AtomicBool,
    /// The slot for LIGHT commands, apart from `busy` so a running macro cannot lock them out.
    busy_light: AtomicBool,
    /// Whether this printer's controls are locked because the farm is past its trial and this printer
    /// is beyond the free five. Set by the farm from the licence state; a running print is never
    /// interrupted by it.
    licence_locked: AtomicBool,
    /// The plain-words reason shown when locked.
    licence_reason: Mutex<Option<String>>,
    /// Preview tokens handed out by POST /api/config/heater-fan, each holding the exact bytes
    /// that apply will upload. Apply consumes the token, so the file that was previewed is the
    /// file that is written, and a stale preview cannot be replayed.
    pending_edit: std::sync::Mutex<Option<PendingEdit>>,
    /// gcode/help is 112 entries on Stella and changes only across a restart.
    help_cache: std::sync::Mutex<Option<(f64, Value)>>,
    /// What this printer's driver can do, and its brand in words (farm board WO-02). A driver that
    /// cannot be opened (a brand not built yet) can do nothing rather than fall back to Moonraker.
    caps: Capabilities,
    brand: &'static str,
    /// When the nozzle heater goes off if no page says otherwise (unix seconds). A filament job in a page keeps pushing it on; closing the page
    /// (or the phone going to sleep) no longer leaves the nozzle hot for ever (Opus review 2026-10-22).
    heat_guard: Mutex<Option<f64>>,
}

/// The longest a page may hold the nozzle hot without renewing: the reload wait's own top (600 s) plus a margin.
const HEAT_GUARD_MAX_S: f64 = 900.0;

struct PendingEdit {
    token: String,
    at: f64,
    celsius: f64,
    original: String,
    edited: String,
}

/// A preview older than this is refused: the config could have been edited on the Pi since.
const EDIT_TOKEN_TTL_S: f64 = 300.0;
const HELP_CACHE_TTL_S: f64 = 300.0;

impl Service {
    pub fn new(cfg: &Config, mode: &str) -> Result<Self> {
        // A demonstration runs a simulated printer locally, so its controls are on the same
        // loopback machine and may be used without controls_enabled: demo is always eligible.
        let eligible = mode == "demo"
            || (cfg.controls_enabled
                && mode == "live"
                && cfg
                    .bind
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback()));
        let (caps, brand) = match driver::open(cfg) {
            Ok(d) => (d.capabilities(), d.brand()),
            Err(_) => (Capabilities::none(), "This printer"),
        };
        Ok(Self {
            caps,
            brand,
            cfg: cfg.clone(),
            mode: mode.to_owned(),
            token: eligible.then(random_token).transpose()?,
            busy: AtomicBool::new(false),
            busy_light: AtomicBool::new(false),
            licence_locked: AtomicBool::new(false),
            licence_reason: Mutex::new(None),
            pending_edit: std::sync::Mutex::new(None),
            help_cache: std::sync::Mutex::new(None),
            heat_guard: Mutex::new(None),
        })
    }

    /// POST /api/filament/guard {"off_in_s": n}: the nozzle heater goes off in n seconds unless renewed; 0 takes the guard away.
    fn set_heat_guard(&self, body: &[u8]) -> Response {
        let n = serde_json::from_slice::<Value>(body).ok().and_then(|v| v["off_in_s"].as_f64());
        let Some(n) = n.filter(|n| (0.0..=HEAT_GUARD_MAX_S).contains(n)) else {
            return failure(400, "off_in_s must be a number of seconds from 0 to 900", "not_sent", None);
        };
        let at = (n > 0.0).then(|| now() + n);
        if let Ok(mut g) = self.heat_guard.lock() {
            *g = at;
        }
        Response::json(200, json!({"result":"ok","off_at":at}))
    }

    /// Takes the guard if it has run out. The caller turns the heater off.
    fn heat_guard_due(&self, at: f64) -> bool {
        let Ok(mut g) = self.heat_guard.lock() else { return false };
        if g.is_some_and(|deadline| at >= deadline) {
            *g = None;
            return true;
        }
        false
    }

    /// One thread per printer: once a second, a guard that has run out turns the nozzle heater off, through the same door as a page.
    pub fn spawn_heat_guard(service: &std::sync::Arc<Self>, stop: std::sync::Arc<AtomicBool>) {
        let weak = std::sync::Arc::downgrade(service);
        let _ = std::thread::Builder::new().name("heat-guard".into()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                let Some(s) = weak.upgrade() else { return };
                if s.heat_guard_due(now()) {
                    let off = json!({"id":"set_heater","target":"extruder","value":0});
                    if let Err(e) = s.run(&off) {
                        eprintln!("heat guard: the nozzle heater could not be turned off: {e}");
                    }
                }
            }
        });
    }

    /// Run a command the service itself decided to send (the farm queue starting a file), through
    /// exactly the door a page uses: the Host and Origin checks, the control token, the tier rules,
    /// the fresh-state check and the audit. Nothing about safety is skipped because the caller is
    /// in-process. Returns the command's JSON reply, or the refusal in words.
    pub fn run(&self, body: &Value) -> Result<Value> {
        let token = self.token.clone().ok_or_else(|| self.disabled_reason())?;
        let host = self.expected_host();
        let mut headers = BTreeMap::new();
        headers.insert("host".to_owned(), host.clone());
        headers.insert("origin".to_owned(), format!("http://{host}"));
        headers.insert("content-type".to_owned(), "application/json".to_owned());
        headers.insert("x-stella-control-token".to_owned(), token);
        let r = self.handle(
            "POST",
            "/api/commands",
            &headers,
            body.to_string().as_bytes(),
            IpAddr::from([127, 0, 0, 1]),
        );
        let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if r.code == 200 {
            Ok(v)
        } else {
            Err(v["error"]
                .as_str()
                .unwrap_or("the printer refused the command")
                .to_owned())
        }
    }

    pub fn handle(
        &self,
        method: &str,
        path: &str,
        headers: &BTreeMap<String, String>,
        body: &[u8],
        peer: IpAddr,
    ) -> Response {
        let preview = path == "/api/command-preview";
        let Some(allowed) = routed(path) else {
            return failure(404, "not a control route", "not_sent", None);
        };
        if !allowed.contains(&method) {
            return failure(
                405,
                "method not allowed for this control route",
                "not_sent",
                None,
            );
        }
        if let Err(error) = self.validate_origin(headers, peer, method != "GET") {
            return failure(403, &error, "not_sent", None);
        }
        if path == "/api/controls" {
            return self.controls();
        }
        if path == "/api/macros" || path == "/api/files" {
            return self.printer_list(path);
        }
        // Read-only views of printer and operator state: loopback and Host checked above, but no
        // control token, exactly like /api/macros and /api/files.
        if method == "GET" {
            return match path {
                "/api/config/heater-fan" => self.enabled_then(|| {
                    if !self.caps.klipper {
                        return Err(format!(
                            "{} has no printer.cfg to edit from here",
                            self.brand
                        ));
                    }
                    crate::printer_config::read(&Moonraker::new(&self.cfg))
                }),
                // the Calibrate page: what printer.cfg holds now for each calibration, read-only (2026-10-26)
                "/api/config/calibration" => self.enabled_then(|| {
                    if !self.caps.klipper {
                        return Err(format!("{} keeps its calibration inside the printer", self.brand));
                    }
                    let v = Moonraker::new(&self.cfg).get("/printer/objects/query?configfile=settings,save_config_pending,save_config_pending_items")?;
                    Ok(calibration_view(&v["result"]["status"]["configfile"]))
                }),
                "/api/print/queue" => self.enabled_then(|| self.print_queue()),
                "/api/print/objects" => self.enabled_then(|| self.print_objects()),
                "/api/console/help" => self.enabled_then(|| self.console_help()),
                "/api/presets/standard" => Response::json(
                    200,
                    json!({"materials":crate::operator::standard_materials(),"sheets":crate::operator::standard_sheets(),"maintenance":crate::operator::standard_maintenance()}),
                ),
                "/api/presets" => match self.operator().presets() {
                    Ok(value) => Response::json(200, value),
                    Err(error) => failure(500, &error, "not_sent", None),
                },
                "/api/console/history" => match self.operator().history() {
                    Ok(rows) => Response::json(
                        200,
                        json!({"history":rows,"limit":crate::operator::HISTORY_MAX}),
                    ),
                    Err(error) => failure(500, &error, "not_sent", None),
                },
                _ => failure(404, "not a control route", "not_sent", None),
            };
        }
        if !self.enabled() && !preview {
            return failure(503, &self.disabled_reason(), "not_sent", None);
        }
        if headers
            .get("content-type")
            .is_none_or(|value| value != "application/json")
        {
            return failure(
                415,
                "Content-Type must be exactly application/json",
                "not_sent",
                None,
            );
        }
        if self.enabled() && !self.valid_token(headers.get("x-stella-control-token")) {
            return failure(
                401,
                "this page is not allowed to control the printer; reload it and try again",
                "not_sent",
                None,
            );
        }
        match path {
            "/api/config/heater-fan" => return self.heater_fan_preview(body),
            "/api/config/heater-fan/apply" => return self.heater_fan_apply(body),
            "/api/presets" => return self.put_presets(body),
            "/api/console/history" => return self.push_history(body),
            "/api/filament/guard" => return self.set_heat_guard(body),
            _ => {}
        }
        let input: CommandInput = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    400,
                    &format!("invalid command JSON: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        let Some(command) = COMMANDS
            .iter()
            .copied()
            .find(|command| command.id == input.id)
        else {
            return failure(
                400,
                "command id is not in the fixed allowlist",
                "not_sent",
                None,
            );
        };
        // A printer that cannot do this says so, in words, before anything is rendered or sent.
        if let Err(reason) = self.caps.allows(command.id, self.brand) {
            return failure(409, &reason, "refused", None);
        }
        if preview {
            let sent = if command.id == "emergency_stop" {
                None
            } else {
                match render(command, &input) {
                    Ok(sent) => Some(sent),
                    Err(error) => return failure(400, &error, "not_sent", None),
                }
            };
            // A G-code command previews as `script`; anything else previews as `action`, which
            // is the shape emergency_stop has always used.
            let script = sent
                .as_ref()
                .and_then(Sent::script)
                .map_or(Value::Null, |script| Value::String(script.into()));
            let action = match &sent {
                None => Value::String("POST /printer/emergency_stop (no G-code)".into()),
                // WO-02: say what a dry run really sends, not the queue call its filename renders to.
                Some(_) if command.id == "dry_run" => {
                    let file = input.filename.as_deref().unwrap_or("").trim();
                    Value::String(format!(
                        "Upload \"{}\": every move of {file} with nothing extruded, {}, Z 30 mm higher. Then start it.",
                        dry_run_name(file),
                        if input.heat { "heaters as in the file" } else { "heaters off" }
                    ))
                }
                Some(Sent::Gcode(_)) => Value::Null,
                // Finish-2 F02 (CORE One L, 2026-10-19): the review showed Klipper's own call ("POST /printer/print/start?
                // filename=... (no G-code)") on a PrusaLink printer, which is neither what is sent there nor words a person
                // reads. The audit row keeps the exact call; the person reads what will happen.
                Some(Sent::Api { .. }) if command.id == "print_start" => Value::String(format!(
                    "Start \"{}\" on the printer, once.",
                    input.filename.as_deref().unwrap_or("").trim()
                )),
                Some(other) => Value::String(other.describe()),
            };
            // Another brand is sent the action, not the Klipper G-code it was rendered from: the review says the action.
            let other_brand = self.cfg.driver != "moonraker" && command.id != "dry_run";
            let (script, action) = match sent
                .as_ref()
                .filter(|_| other_brand)
                .and_then(|s| action_from_sent(command, s, &input))
            {
                Some(a) => (Value::Null, Value::String(action_words(&a))),
                None => (script, action),
            };
            return Response::json(
                200,
                json!({
                    "status": "preview",
                    "id": command.id,
                    "script": script,
                    "action": action,
                    "tier": command.tier,
                    "review": !NO_REVIEW.contains(&command.id),
                    "enabled": self.enabled(),
                    "reason": if self.enabled() { "Execution eligibility is checked again immediately before send.".into() } else { self.disabled_reason() }
                }),
            );
        }
        if command.tier == "T3" && !input.confirm && !matches!(command.params, Params::Console) {
            return failure(400, "T3 command requires confirm:true", "not_sent", None);
        }
        // WO-02: a dry run uploads a no-extrusion copy of the file and starts it, so the printer
        // runs every move without printing anything. Refused while printing and on a printer that
        // cannot start a file (both gated above: T3 + start_print capability).
        if command.id == "dry_run" {
            let Some(filename) = input.filename.as_deref().filter(|f| filename_ok(f)) else {
                return failure(
                    400,
                    "name a .gcode or .bgcode file from GET /api/files",
                    "not_sent",
                    None,
                );
            };
            let fresh = if self.cfg.driver == "moonraker" {
                FreshState::fetch(&Moonraker::new(&self.cfg))
            } else {
                self.fresh_other()
            };
            if let Ok(state) = fresh {
                if let Some(reason) = state.refusal(command) {
                    return failure(409, &reason, "refused", None);
                }
            }
            let driver = match driver::open(&self.cfg) {
                Ok(d) => d,
                Err(error) => return failure(502, &error, "not_sent", None),
            };
            let bytes = match driver.file_bytes(filename, MAX_GCODE_BYTES) {
                Ok(b) => b,
                Err(error) => return failure(502, &error, "not_sent", None),
            };
            if dry_run::is_binary_gcode(&bytes) {
                return failure(
                    400,
                    "This is a binary G-code file (.bgcode). A dry run needs plain G-code: slice it as .gcode to dry-run it.",
                    "not_sent",
                    None,
                );
            }
            let text = String::from_utf8_lossy(&bytes);
            let transformed = match dry_run::transform(
                &text,
                &dry_run::Recipe {
                    heat: input.heat,
                    ..Default::default()
                },
            ) {
                Ok(t) => t,
                Err(error) => return failure(400, &error, "not_sent", None),
            };
            let dry_name = dry_run_name(filename);
            if let Err(error) = driver.upload(&dry_name, transformed.as_bytes(), false) {
                return failure(502, &error, "not_sent", None);
            }
            if let Err(error) = driver.act(&Action::StartPrint {
                filename: dry_name.clone(),
            }) {
                return failure(502, &error, "not_sent", None);
            }
            return Response::json(
                200,
                json!({"result":"ok","dry_run":dry_name,"filename":filename}),
            );
        }
        let sent = if command.id == "emergency_stop" {
            Sent::Api {
                method: "POST",
                path: "/printer/emergency_stop".into(),
                body: None,
            }
        } else if command.id == "filament_shake" && self.cfg.driver == "moonraker" {
            match self.shake_here() {
                Ok(script) => Sent::Gcode(script),
                Err(error) => return failure(409, &error, "refused", None),
            }
        } else {
            match render(command, &input) {
                Ok(sent) => sent,
                Err(error) => return failure(400, &error, "not_sent", None),
            }
        };
        // Another brand does not take Klipper G-code: the bounded request is carried as a neutral
        // action. Built from the rendered script, so every driver gets the same validation.
        let action = (self.cfg.driver != "moonraker")
            .then(|| action_from_sent(command, &sent, &input))
            .flatten();
        self.execute_with(command, &sent, action)
    }

    /// The shake for where the head is now: centred when there is room both ways, else all on the side away from the end of travel
    /// (a head parked at X0 shook to X-2 and Klipper refused it "out of range").
    fn shake_here(&self) -> Result<String> {
        let v = Moonraker::new(&self.cfg)
            .get("/printer/objects/query?toolhead=position,axis_minimum,axis_maximum")?;
        let at = |field: &str| v["status"]["toolhead"][field][0].as_f64();
        let (Some(x), Some(min), Some(max)) = (at("position"), at("axis_minimum"), at("axis_maximum")) else {
            return Err("the printer did not report where the head is; the shake was not sent".into());
        };
        shake_script(x, min, max).ok_or_else(|| {
            format!("no room to shake the head at X {x:.1} (travel {min:.0} to {max:.0}); the shake was not sent")
        })
    }

    fn operator(&self) -> crate::operator::Operator {
        crate::operator::Operator::new(&self.cfg.database)
    }

    /// Run a read that needs the printer, or say plainly why the printer is not being asked.
    fn enabled_then(&self, read: impl FnOnce() -> Result<Value>) -> Response {
        if !self.enabled() {
            return failure(503, &self.disabled_reason(), "not_sent", None);
        }
        match read() {
            Ok(value) => Response::json(200, value),
            Err(error) => failure(502, &error, "not_sent", None),
        }
    }

    fn print_queue(&self) -> Result<Value> {
        if !self.caps.klipper {
            return Ok(
                json!({"queued_jobs":[],"queue_state":"unavailable","source":"this printer has no job queue of its own"}),
            );
        }
        let status = Moonraker::new(&self.cfg).get("/server/job_queue/status")?;
        let jobs: Vec<Value> = status["queued_jobs"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        json!({"job_id":row["job_id"],"filename":row["filename"],
                               "time_added":row["time_added"],"time_in_queue":row["time_in_queue"]})
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "queued_jobs": jobs,
            "queue_state": status["queue_state"],
            "source": "GET /server/job_queue/status"
        }))
    }

    /// Objects the slicer declared for the running print, with the polygon and centre Klipper
    /// reports, plus which of them have already been excluded.
    fn print_objects(&self) -> Result<Value> {
        if !self.caps.klipper {
            return Ok(
                json!({"objects":[],"current_object":Value::Null,"excluded":[],"source":"this printer does not report printed objects"}),
            );
        }
        let status = Moonraker::new(&self.cfg).get("/printer/objects/query?exclude_object")?;
        let exclude = &status["status"]["exclude_object"];
        let objects: Vec<Value> = exclude["objects"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        let name = row["name"].clone();
                        let excluded = exclude["excluded_objects"]
                            .as_array()
                            .is_some_and(|done| done.iter().any(|d| *d == name));
                        json!({"name":name,"center":row["center"],"polygon":row["polygon"],
                               "excluded":excluded})
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "objects": objects,
            "current_object": exclude["current_object"],
            "excluded": exclude["excluded_objects"],
            "source": "GET /printer/objects/query?exclude_object"
        }))
    }

    fn console_help(&self) -> Result<Value> {
        if !self.caps.klipper {
            return Ok(json!({"commands":[],"source":"this printer has no G-code help"}));
        }
        let clock = now();
        if let Ok(cache) = self.help_cache.lock() {
            if let Some((at, value)) = cache.as_ref() {
                if clock - at < HELP_CACHE_TTL_S {
                    return Ok(value.clone());
                }
            }
        }
        let help = Moonraker::new(&self.cfg).get("/printer/gcode/help")?;
        let mut commands: Vec<Value> = help
            .as_object()
            .ok_or("Klipper returned no gcode/help; is Klippy ready?")?
            .iter()
            .map(|(name, help)| json!({"name":name,"help":help}))
            .collect();
        commands.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let value = json!({
            "commands": commands,
            "cached_at": clock,
            "ttl_s": HELP_CACHE_TTL_S,
            "source": "GET /printer/gcode/help"
        });
        if let Ok(mut cache) = self.help_cache.lock() {
            *cache = Some((clock, value.clone()));
        }
        Ok(value)
    }

    /// Preview only: download printer.cfg, bound the value, and hand back the diff plus a token.
    /// Nothing is written and nothing is sent to the printer.
    fn heater_fan_preview(&self, body: &[u8]) -> Response {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            heater_temp: f64,
        }
        let input: Input = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    400,
                    &format!("invalid edit JSON: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        if let Some(error) = crate::printer_config::bounds_error(input.heater_temp) {
            return failure(400, &error, "not_sent", None);
        }
        let moonraker = Moonraker::new(&self.cfg);
        let (diff, original, edited) =
            match crate::printer_config::preview(&moonraker, input.heater_temp) {
                Ok(parts) => parts,
                Err(error) => return failure(502, &error, "not_sent", None),
            };
        let token = match random_token() {
            Ok(token) => token,
            Err(error) => return failure(500, &error, "not_sent", None),
        };
        if let Ok(mut pending) = self.pending_edit.lock() {
            *pending = Some(PendingEdit {
                token: token.clone(),
                at: now(),
                celsius: input.heater_temp,
                original,
                edited,
            });
        }
        Response::json(
            200,
            json!({
                "status": "preview",
                "diff": diff,
                "token": token,
                "heater_temp": input.heater_temp,
                "file": crate::printer_config::FILE,
                "expires_in_s": EDIT_TOKEN_TTL_S,
                "reason": "Nothing has been written. POST the token to /api/config/heater-fan/apply to save a dated backup, upload the edit and FIRMWARE_RESTART."
            }),
        )
    }

    /// Apply the previewed edit: backup, upload, FIRMWARE_RESTART. The token is consumed whether
    /// or not the upload succeeds, so one preview can never be applied twice.
    fn heater_fan_apply(&self, body: &[u8]) -> Response {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            token: String,
        }
        let input: Input = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    400,
                    &format!("invalid apply JSON: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        let Ok(mut pending) = self.pending_edit.lock() else {
            return failure(500, "edit state is unavailable", "not_sent", None);
        };
        let Some(edit) = pending.take() else {
            return failure(
                409,
                "no previewed edit is waiting; POST /api/config/heater-fan first",
                "not_sent",
                None,
            );
        };
        if edit.token != input.token {
            return failure(
                409,
                "that token does not match the waiting preview; preview the edit again",
                "not_sent",
                None,
            );
        }
        if now() - edit.at > EDIT_TOKEN_TTL_S {
            return failure(
                409,
                "the previewed edit is older than five minutes; preview it again so the diff matches the file on the Pi",
                "not_sent",
                None,
            );
        }
        drop(pending);
        let command = Command {
            id: "config_heater_fan",
            script: None,
            capability: None,
            tier: "T3",
            requires_homed: false,
            params: Params::Fixed,
        };
        let audit = Audit::new(&self.cfg.database);
        let intent = format!(
            "operator applied printer.cfg [heater_fan hotend_fan] heater_temp={:.1}",
            edit.celsius
        );
        let audit_id = match audit.intent(command, &intent) {
            Ok(id) => id,
            Err(error) => {
                return failure(
                    500,
                    &format!("config edit intent could not be saved: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        let moonraker = Moonraker::new(&self.cfg);
        match crate::printer_config::apply(
            &moonraker,
            &edit.original,
            &edit.edited,
            &crate::printer_config::stamp(now()),
        ) {
            Ok(mut value) => {
                let message = value["message"].as_str().unwrap_or("applied").to_owned();
                if let Err(error) = audit.result(&audit_id, command, "completed", &message) {
                    return failure(
                        500,
                        &format!(
                            "the edit was uploaded but its result could not be saved: {error}"
                        ),
                        "unknown",
                        Some(&audit_id),
                    );
                }
                value["audit_id"] = json!(audit_id);
                Response::json(200, value)
            }
            Err(error) => {
                // A failed backup means nothing was edited; a failed upload leaves the outcome
                // unknown. The message from printer_config::apply says which of the two it is.
                let outcome = if error.contains("nothing was edited") {
                    "not_sent"
                } else {
                    "unknown"
                };
                let _ = audit.result(&audit_id, command, outcome, &error);
                failure(502, &error, outcome, Some(&audit_id))
            }
        }
    }

    fn put_presets(&self, body: &[u8]) -> Response {
        let incoming: Value = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    400,
                    &format!("invalid presets JSON: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        match self.operator().put_presets(&incoming) {
            Ok(value) => Response::json(200, value),
            Err(error) => failure(400, &error, "not_sent", None),
        }
    }

    fn push_history(&self, body: &[u8]) -> Response {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            line: String,
        }
        let input: Input = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    400,
                    &format!("invalid console history JSON: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        match self.operator().push_history(now(), &input.line) {
            Ok(rows) => Response::json(
                200,
                json!({"history":rows,"limit":crate::operator::HISTORY_MAX}),
            ),
            Err(error) => failure(400, &error, "not_sent", None),
        }
    }

    /// Read-only passthrough for the two lists the HUD needs from the printer. Never writes.
    fn printer_list(&self, path: &str) -> Response {
        if !self.enabled() {
            return failure(503, &self.disabled_reason(), "not_sent", None);
        }
        let moonraker = Moonraker::new(&self.cfg);
        if path == "/api/macros" {
            // Macros are Klipper's: another brand reports none rather than an error.
            if !self.caps.macros {
                return Response::json(200, json!({"macros":[]}));
            }
            return match moonraker.get("/printer/gcode/help") {
                Ok(help) => {
                    // The macro bodies live in configfile settings, not in gcode/help. A failure
                    // there costs the parameter lists, not the macro list, so it degrades to an
                    // empty params array rather than a 502.
                    let settings = Moonraker::new(&self.cfg)
                        .fetch_config()
                        .unwrap_or(Value::Null);
                    let mut macros: Vec<Value> = help
                        .as_object()
                        .map(|map| {
                            map.iter()
                                .filter(|(name, _)| {
                                    name.len() > 1
                                        && name.bytes().all(|b| {
                                            b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
                                        })
                                })
                                .map(|(name, help)| {
                                    // Klipper lowercases section names: `gcode_macro print_start`.
                                    let section =
                                        &settings[format!("gcode_macro {}", name.to_ascii_lowercase())];
                                    let body = section["gcode"].as_str().unwrap_or_default();
                                    // WO-19 (2026-09-23): the HUD's Macros list is the gcode_macro
                                    // sections; gcode/help also lists every registered command.
                                    json!({"name":name,"help":help,"params":macro_params(body),"macro":section.is_object()})
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    macros.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    Response::json(200, json!({"macros":macros}))
                }
                Err(error) => failure(502, &error, "not_sent", None),
            };
        }
        // The printable files, through the driver: Klipper's list as it always was, another
        // brand's own.
        match driver::open(&self.cfg).and_then(|d| d.files()) {
            Ok(files) => Response::json(200, json!({"files":files})),
            Err(error) => failure(502, &error, "not_sent", None),
        }
    }

    fn controls(&self) -> Response {
        if !self.enabled() {
            let reason = self.disabled_reason();
            return Response::json(
                200,
                json!({
                    "enabled": false,
                    "reason": reason,
                    "token": Value::Null,
                    "driver": self.cfg.driver, "brand": self.brand, "capabilities": self.caps,
                    "commands": COMMANDS.iter().map(|command| json!({
                        "id": command.id, "allowed": false, "reason": reason, "tier": command.tier, "params": params_name(command.params),
                        "supported": self.caps.allows(command.id, self.brand).is_ok()
                    })).collect::<Vec<_>>()
                }),
            );
        }
        let fresh = if self.cfg.driver == "moonraker" {
            FreshState::fetch(&Moonraker::new(&self.cfg))
        } else {
            self.fresh_other()
        };
        let commands = COMMANDS
            .iter()
            .map(|command| {
                // What the printer cannot do at all comes first, in the brand's words; then the fresh
                // state of what it can. `supported: false` is for the UI to hide, `allowed: false` to lock.
                let unsupported = self.caps.allows(command.id, self.brand).err();
                let refusal = if let Some(reason) = unsupported.clone() {
                    Some(reason)
                } else if command.id == "emergency_stop" {
                    None
                } else {
                    fresh
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|state| state.refusal(*command).map_or(Ok(()), Err))
                        .err()
                };
                json!({
                    "id": command.id,
                    "allowed": refusal.is_none(),
                    "supported": unsupported.is_none(),
                    "reason": refusal.unwrap_or_else(|| "available from fresh printer state".into()),
                    "tier": command.tier,
                    "params": params_name(command.params)
                })
            })
            .collect::<Vec<_>>();
        Response::json(
            200,
            json!({
                "enabled": true,
                "reason": "live loopback controls enabled",
                "token": self.token,
                "driver": self.cfg.driver,
                "brand": self.brand,
                "capabilities": self.caps,
                "commands": commands
            }),
        )
    }

    fn execute_with(&self, command: Command, sent: &Sent, action: Option<Action>) -> Response {
        let other = self.cfg.driver != "moonraker";
        let script = sent.describe();
        let script = script.as_str();
        let _permit = if command.id == "emergency_stop" {
            None
        } else {
            let slot = if LIGHT.contains(&command.id) {
                &self.busy_light
            } else {
                &self.busy
            };
            match BusyPermit::acquire(slot) {
                Some(permit) => Some(permit),
                None => {
                    return failure(
                        409,
                        "another ordinary command is active; no command was queued",
                        "not_sent",
                        None,
                    )
                }
            }
        };
        let audit = Audit::new(&self.cfg.database);
        let (audit_id, audit_error) = match audit.intent(command, script) {
            Ok(id) => (Some(id), None),
            Err(error) if command.id == "emergency_stop" => (None, Some(error)),
            Err(error) => {
                return failure(
                    500,
                    &format!("command intent could not be saved: {error}"),
                    "not_sent",
                    None,
                )
            }
        };
        // Finish-2 F05 (Stella, 2026-10-19): Moonraker answers a G-code script when it is done, and Stella's full bed mesh
        // (each point probed twice) takes about eight minutes; at the 180 s command limit the page said "did not confirm
        // an outcome" while the printer meshed normally. Klipper's long procedures get half an hour; a macro may heat-soak.
        let mut timed = self.cfg.clone();
        timed.command_timeout_ms = command_timeout_ms(command.id, self.cfg.command_timeout_ms);
        let moonraker = Moonraker::new(&timed);
        if command.id != "emergency_stop" {
            let fresh = match if other {
                self.fresh_other()
            } else {
                FreshState::fetch(&moonraker)
            } {
                Ok(state) => state,
                Err(error) => {
                    return self.finish_failure(
                        &audit,
                        audit_id.as_deref().unwrap(),
                        command,
                        "not_sent",
                        503,
                        error,
                    )
                }
            };
            if let Some(error) = fresh.refusal(command) {
                // A halted Klipper keeps its old answer for everything but the restarts: not sent,
                // 503, exactly as before the recovery path existed.
                let (outcome, code) = if matches!(fresh, Fresh::Halted(_)) {
                    ("not_sent", 503)
                } else {
                    ("refused", 409)
                };
                return self.finish_failure(
                    &audit,
                    audit_id.as_deref().unwrap(),
                    command,
                    outcome,
                    code,
                    error,
                );
            }
        }
        let started = matches!(sent, Sent::Resonance { .. });
        let result = if other {
            self.act_other(command, action.as_ref())
        } else {
            match sent {
                Sent::Gcode(script) => {
                    moonraker.post("/printer/gcode/script", Some(&json!({"script": script})))
                }
                Sent::Api {
                    method: "DELETE",
                    path,
                    ..
                } => moonraker.delete(path),
                Sent::Api { path, body, .. } => moonraker.post(path, body.as_ref()),
                // Nothing leaves this process: the websocket source is asked to drop its connection
                // and its own retry loop brings it back within one 500 ms read tick. In poll mode
                // there is no such source, and saying "reconnected" there would be a lie.
                // The capture owns the sweep so the accelerometer is subscribed before the toolhead
                // moves; it returns a run id and finishes on its own thread (~130 s a leg).
                Sent::Resonance { axis, .. } => crate::resonance::start(&self.cfg, axis)
                    .map(|run_id| json!({ "run_id": run_id })),
                Sent::Local(_) => {
                    if crate::ws_source::request_reconnect(&self.cfg.moonraker_url) {
                        Ok(json!({"reconnect":"requested"}))
                    } else {
                        Err("this recording is not using the websocket push source, so there is no connection to drop; the HTTP poll reconnects on its own each interval".into())
                    }
                }
            }
        };
        // Finish-2 F05 (Stella, 2026-10-19): Klipper's RESUME macro answers "ok" even when it aborts ("!! Resume aborted !!!
        // "extruder" not hot enough", always so after a heaters-off dry run), and the page said the resume was done. The
        // console after the RESUME line tells: an aborted resume is reported as refused, in the printer's own words.
        let result = match result {
            Ok(v) if !other && command.id == "resume" => match klipper_refusal(&moonraker, "RESUME") {
                Some(why) => Err(format!("Klipper {}: {why}", crate::driver::PRINTER_REFUSED)),
                None => Ok(v),
            },
            r => r,
        };
        // Plain words on a printer that is not Klipper (README rule 10): name what answered.
        let who = if self.cfg.driver == "moonraker" { "Moonraker" } else { self.brand };
        match result {
            Ok(_) if audit_id.is_none() => Response::json(
                200,
                json!({
                    "status":"completed",
                    "message":format!("emergency_stop: {who} returned success, but the physical stop was not independently observed; audit was unavailable: {}", audit_error.unwrap_or_else(|| "unknown audit error".into())),
                    "audit_id":Value::Null
                }),
            ),
            Ok(value) => {
                let run_id = value.get("run_id").cloned().unwrap_or(Value::Null);
                let status = if started { "started" } else { "completed" };
                let message = if started {
                    format!(
                        "{}: the sweep is running on the printer; poll GET /api/resonance/{} for the result",
                        command.id, run_id
                    )
                } else {
                    format!(
                        "{}: {who} returned success; the physical result was not independently observed",
                        command.id
                    )
                };
                let audit_id = audit_id.as_deref().unwrap();
                if let Err(error) = audit.result(audit_id, command, "completed", &message) {
                    return failure(
                        500,
                        &format!("{who} returned success but the reported result could not be saved: {error}"),
                        "unknown",
                        Some(audit_id),
                    );
                }
                Response::json(
                    200,
                    json!({"status":status,"message":message,"audit_id":audit_id,"run_id":run_id}),
                )
            }
            Err(error) if audit_id.is_none() => failure(
                502,
                &format!(
                    "{error}; emergency audit was also unavailable: {}",
                    audit_error.unwrap_or_else(|| "unknown audit error".into())
                ),
                "unknown",
                None,
            ),
            Err(error) => {
                // A local action either happened or it did not; there is no uncertain middle,
                // so it reports "refused" rather than borrowing the transport's "unknown".
                let (outcome, code) = match sent {
                    Sent::Local(_) => ("refused", 409),
                    // the printer answered and said no: the command did not happen (a PrusaLink 409)
                    _ if error.contains(crate::driver::PRINTER_REFUSED) => ("refused", 409),
                    _ => ("unknown", 502),
                };
                self.finish_failure(
                    &audit,
                    audit_id.as_deref().unwrap(),
                    command,
                    outcome,
                    code,
                    error,
                )
            }
        }
    }

    /// The fresh-state read for a printer that is not Klipper: the driver's own snapshot in the same
    /// shape. Klipper's `help` is the list of extended commands a printer reports; here every named
    /// capability counts as reported, because `Capabilities::allows` has already refused what the
    /// printer cannot do.
    fn fresh_other(&self) -> Result<Fresh> {
        let snapshot = driver::open(&self.cfg)?.snapshot()?;
        if snapshot.state != "ready" {
            return if ["error", "faulted"].contains(&snapshot.state.as_str()) {
                Ok(Fresh::Halted(snapshot.state))
            } else {
                Err(format!(
                    "Printer state is {}, not ready; command was not sent",
                    snapshot.state
                ))
            };
        }
        Ok(Fresh::Ready(FreshState {
            // What the driver reports (nothing, for these brands today) plus every capability it was allowed.
            help: snapshot
                .help
                .into_iter()
                .chain(
                    COMMANDS
                        .iter()
                        .filter_map(|c| c.capability)
                        .map(str::to_owned),
                )
                .collect(),
            homed_axes: snapshot.homed_axes,
            print_state: snapshot.print_state,
            idle_state: snapshot.idle_state,
        }))
    }

    /// Carry out a command on a printer that is not Klipper.
    fn act_other(&self, command: Command, action: Option<&Action>) -> Result<Value> {
        let driver = driver::open(&self.cfg)?;
        if command.id == "emergency_stop" {
            return driver.emergency_stop();
        }
        let action = action.ok_or_else(|| {
            format!(
                "{} cannot do {} from here",
                self.brand,
                command.id.replace('_', " ")
            )
        })?;
        driver.act(action)
    }

    fn finish_failure(
        &self,
        audit: &Audit,
        audit_id: &str,
        command: Command,
        outcome: &str,
        code: u16,
        message: String,
    ) -> Response {
        if let Err(error) = audit.result(audit_id, command, outcome, &message) {
            return failure(
                500,
                &format!(
                    "command outcome is {outcome}, but its result could not be saved: {error}"
                ),
                "unknown",
                Some(audit_id),
            );
        }
        failure(code, &message, outcome, Some(audit_id))
    }

    fn enabled(&self) -> bool {
        self.token.is_some() && !self.licence_locked.load(Ordering::Relaxed)
    }

    fn disabled_reason(&self) -> String {
        if self.licence_locked.load(Ordering::Relaxed) {
            return self
                .licence_reason
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .unwrap_or_else(|| "this printer is locked by the farm licence".into());
        }
        if !self.cfg.controls_enabled {
            "controls_enabled is false".into()
        } else if self.mode != "live" {
            format!("controls are disabled in {} mode", self.mode)
        } else {
            "controls only work when Stella Scope listens on this computer alone (127.0.0.1)".into()
        }
    }

    /// Set by the farm: whether this printer's controls are locked by the farm licence (past trial,
    /// beyond the free five) and the plain-words reason. A running print is never interrupted.
    pub fn set_licence_locked(&self, locked: bool, reason: String) {
        self.licence_locked.store(locked, Ordering::Relaxed);
        let mut r = self
            .licence_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *r = Some(reason);
    }

    fn expected_host(&self) -> String {
        self.cfg.host()
    }

    fn validate_origin(
        &self,
        headers: &BTreeMap<String, String>,
        peer: IpAddr,
        require_origin: bool,
    ) -> Result<()> {
        if !peer.is_loopback() {
            return Err("printer controls only work from the computer Stella Scope runs on, or a paired phone".into());
        }
        let host = self.expected_host();
        if headers.get("host") != Some(&host) {
            return Err("printer controls were refused: this page was opened at a different address than the one Stella Scope listens on".into());
        }
        let origin = format!("http://{host}");
        match headers.get("origin") {
            Some(value) if value == &origin => Ok(()),
            None if !require_origin => Ok(()),
            _ => Err("printer controls were refused: this page was opened at a different address than the one Stella Scope listens on".into()),
        }
    }

    /// Whether `supplied` is this printer's control token (the queue's plate and start routes carry it).
    pub fn token_ok(&self, supplied: Option<&String>) -> bool {
        self.valid_token(supplied)
    }

    fn valid_token(&self, supplied: Option<&String>) -> bool {
        let (Some(expected), Some(supplied)) = (self.token.as_ref(), supplied) else {
            return false;
        };
        expected.len() == supplied.len()
            && expected
                .bytes()
                .zip(supplied.bytes())
                .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
                == 0
    }
}

struct BusyPermit<'a>(&'a AtomicBool);
impl<'a> BusyPermit<'a> {
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self(flag))
    }
}
impl Drop for BusyPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Audit {
    path: String,
}
impl Audit {
    fn new(database: &str) -> Self {
        Self {
            path: format!("{database}.commands.sqlite3"),
        }
    }

    fn open(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path).map_err(db_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(db_error)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS command_audit (
               row_id INTEGER PRIMARY KEY,
               audit_id TEXT NOT NULL,
               at REAL NOT NULL,
               command_id TEXT NOT NULL,
               tier TEXT NOT NULL,
               phase TEXT NOT NULL CHECK(phase IN ('intent','result')),
               outcome TEXT NOT NULL CHECK(outcome IN ('intent','refused','not_sent','completed','unknown')),
               message TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS command_audit_id ON command_audit(audit_id,row_id);",
        ).map_err(db_error)?;
        Ok(connection)
    }

    fn intent(&self, command: Command, script: &str) -> Result<String> {
        let audit_id = random_token()?;
        self.append(
            &audit_id,
            command,
            "intent",
            "intent",
            &format!("operator requested allowlisted command: {script}"),
        )?;
        Ok(audit_id)
    }

    fn result(&self, audit_id: &str, command: Command, outcome: &str, message: &str) -> Result<()> {
        self.append(audit_id, command, "result", outcome, message)
    }

    fn append(
        &self,
        audit_id: &str,
        command: Command,
        phase: &str,
        outcome: &str,
        message: &str,
    ) -> Result<()> {
        let connection = self.open()?;
        connection.execute(
            "INSERT INTO command_audit(audit_id,at,command_id,tier,phase,outcome,message) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![audit_id, now(), command.id, command.tier, phase, outcome, message],
        ).map_err(db_error)?;
        Ok(())
    }
}

/// Parameters a macro reads, taken from its own Jinja body: every `params.NAME`, with the literal
/// from `|default(...)` when there is one. Hand-scanned rather than regexed — there is no regex
/// crate here, and the grammar is two tokens wide.
///
/// Stella's `pause` macro, for example, contains `params.RESTORE|default(1)|int`, which reads as
/// `{"name":"RESTORE","default":"1"}`. A bare `params.EXTRUDE` reads as a null default.
fn macro_params(gcode: &str) -> Vec<Value> {
    let bytes = gcode.as_bytes();
    let mut out: Vec<Value> = Vec::new();
    let mut at = 0;
    while let Some(found) = gcode[at..].find("params.") {
        let start = at + found + "params.".len();
        let end = start
            + gcode[start..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(gcode.len() - start);
        at = end.max(start + 1);
        if end == start {
            continue;
        }
        let name = &gcode[start..end];
        // `|default(` may be separated from the name by spaces: `params.X | default(1)`.
        let mut cursor = end;
        while bytes.get(cursor) == Some(&b' ') {
            cursor += 1;
        }
        let default = gcode[cursor..]
            .strip_prefix('|')
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix("default("))
            .and_then(|rest| {
                // Balance the parentheses so `default({'a':1})` and `default(0)` both close right.
                let mut depth = 1_i32;
                let mut inner = String::new();
                for c in rest.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(inner.trim().trim_matches(['\'', '"']).to_owned());
                            }
                        }
                        _ => {}
                    }
                    inner.push(c);
                    if inner.len() > 120 {
                        return None;
                    }
                }
                None
            });
        if out.iter().any(|existing| existing["name"] == name) {
            continue;
        }
        out.push(json!({"name":name,"default":default}));
        if out.len() >= MACRO_PARAMS_MAX {
            break;
        }
    }
    out
}

fn params_name(params: Params) -> &'static str {
    match params {
        Params::Fixed => "fixed",
        Params::Heater => "heater",
        Params::Fan => "fan",
        Params::Extrude => "extrude",
        Params::ZOffset => "zoffset",
        Params::Limit => "limit",
        Params::Advance => "advance",
        Params::Pid => "pid",
        Params::TestZ => "testz",
        Params::Tower => "tower",
        Params::Macro => "macro",
        Params::Console => "console",
        Params::Factor => "factor",
        Params::Jog => "jog",
        Params::Filename => "filename",
        Params::JobId => "job_id",
        Params::ObjectName => "object_name",
        Params::Filament => "filament",
        Params::FilamentMacro => "filament_macro",
        Params::Local => "local",
        Params::Resonance => "resonance",
    }
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    // The operating system's random source on every platform (it was /dev/urandom, Linux only).
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("cannot create local control token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn failure(code: u16, error: &str, status: &str, audit_id: Option<&str>) -> Response {
    Response::json(
        code,
        json!({"error":error,"status":status,"audit_id":audit_id}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
        thread,
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn path(label: &str) -> String {
        format!(
            "/tmp/stella-command-test-{}-{}-{label}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn cleanup(database: &str) {
        for suffix in [
            ".commands.sqlite3",
            ".commands.sqlite3-wal",
            ".commands.sqlite3-shm",
        ] {
            let _ = fs::remove_file(format!("{database}{suffix}"));
        }
    }

    /// JSON replies, one per expected request.
    fn fake(replies: Vec<Option<Value>>) -> (String, thread::JoinHandle<Vec<String>>) {
        fake_text(
            replies
                .into_iter()
                .map(|reply| reply.map(|value| value.to_string()))
                .collect(),
        )
    }

    /// Raw-body replies, for the endpoints that return a file rather than JSON
    /// (`GET /server/files/config/printer.cfg`).
    fn fake_text(replies: Vec<Option<String>>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                let header_end = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .map(str::to_owned)
                    })
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                if let Some(body) = reply {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            }
            requests
        });
        (format!("http://{address}"), worker)
    }

    fn one_response(status: &'static str, delay: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            thread::sleep(delay);
            let body = r#"{"result":"ok"}"#;
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}"), worker)
    }

    fn info() -> Value {
        json!({"result":{"state":"ready"}})
    }
    fn status(print: &str, idle: &str, homed: &str) -> Value {
        json!({"result":{"status":{
            "toolhead":{"homed_axes":homed},
            "print_stats":{"state":print},
            "idle_timeout":{"state":idle}
        }}})
    }
    /// Shaped like a real Klipper `gcode/help`: extended and macro commands only. G28 is
    /// deliberately absent, because Klipper does not report base G-codes here and this fixture
    /// claiming otherwise is what let the HOME gate ship broken.
    fn help() -> Value {
        json!({"result":{
            "GET_POSITION":"", "QUERY_PROBE":"", "QUAD_GANTRY_LEVEL":"",
            "BED_MESH_CALIBRATE":"", "PROBE_ACCURACY":"", "TEST_RESONANCES":"",
            "SHAPER_CALIBRATE":"", "SET_HEATER_TEMPERATURE":"", "SET_VELOCITY_LIMIT":"",
            "SET_PRESSURE_ADVANCE":"", "Z_OFFSET_APPLY_PROBE":"", "PAUSE":"", "RESUME":"",
            "CANCEL_PRINT":"", "FIRMWARE_RESTART":"", "RESTART":"", "EXCLUDE_OBJECT":"",
            // Stella's own macros (2026-09-28); filament_load / filament_unload are gated on them.
            "LOAD_FILAMENT":"", "UNLOAD_FILAMENT":"",
            // the calibrations Klipper offers (2026-10-26)
            "PID_CALIBRATE":"", "PROBE_CALIBRATE":"", "Z_ENDSTOP_CALIBRATE":"", "TUNING_TOWER":"", "SAVE_CONFIG":""
        }})
    }
    fn service(url: String, database: String, mode: &str) -> Service {
        let cfg = Config {
            moonraker_url: url,
            database,
            controls_enabled: true,
            request_timeout_ms: 500,
            ..Config::default()
        };
        Service::new(&cfg, mode).unwrap()
    }
    fn headers(service: &Service) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("host".into(), "127.0.0.1:7420".into()),
            ("origin".into(), "http://127.0.0.1:7420".into()),
            ("content-type".into(), "application/json".into()),
            (
                "x-stella-control-token".into(),
                service.token.clone().unwrap(),
            ),
        ])
    }
    fn body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn fixed_allowlist_and_payload_are_exact_and_audited() {
        let expected = [
            ("z_offset_save", "Z_OFFSET_APPLY_PROBE\nSAVE_CONFIG"),
            // calibration (2026-10-26): the probe and endstop paper tests, their accept and abort, and the save
            ("probe_calibrate", "PROBE_CALIBRATE"),
            ("z_endstop_calibrate", "Z_ENDSTOP_CALIBRATE"),
            ("manual_accept", "ACCEPT"),
            ("manual_abort", "ABORT"),
            ("save_config", "SAVE_CONFIG"),
            ("pause", "PAUSE"),
            ("resume", "RESUME"),
            ("cancel", "CANCEL_PRINT"),
            ("firmware_restart", "FIRMWARE_RESTART"),
            ("motors_off", "M84"),
            ("position", "GET_POSITION"),
            ("probe_query", "QUERY_PROBE"),
            ("home", "G28"),
            ("qgl", "QUAD_GANTRY_LEVEL"),
            ("mesh", "BED_MESH_CALIBRATE"),
            ("probe_accuracy", "PROBE_ACCURACY"),
            ("resonance_x", "TEST_RESONANCES AXIS=X"),
            ("resonance_y", "TEST_RESONANCES AXIS=Y"),
            ("shaper_x", "SHAPER_CALIBRATE AXIS=X"),
            ("shaper_y", "SHAPER_CALIBRATE AXIS=Y"),
            ("klipper_restart", "RESTART"),
            // filament board 2026-10-22: the shake after a load, a new fixed command (exactly this text; the allowlist test names every fixed script)
            ("filament_shake", SHAKE_SCRIPT),
        ];
        assert_eq!(
            COMMANDS
                .iter()
                .filter_map(|command| command.script.map(|script| (command.id, script)))
                .collect::<Vec<_>>(),
            expected
        );
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
            Some(json!({"result":"ok"})),
        ]);
        let database = path("success");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"resonance_x","confirm":true}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 200);
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[3].starts_with("POST /printer/gcode/script "));
        assert!(requests[3].ends_with(r#"{"script":"TEST_RESONANCES AXIS=X"}"#));
        let audit = Connection::open(format!("{database}.commands.sqlite3")).unwrap();
        let entries = audit
            .prepare("SELECT phase,outcome,message FROM command_audit ORDER BY row_id")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "intent");
        assert_eq!(entries[1].1, "completed");
        assert!(entries
            .iter()
            .all(|entry| !entry.2.contains(service.token.as_ref().unwrap())));
        drop(audit);
        cleanup(&database);
    }

    /// Klipper reports only extended commands in gcode/help, so a base G-code must not be checked
    /// against it. Home was refused on Stella for exactly this reason on 2026-09-21.
    #[test]
    fn base_gcodes_are_not_checked_against_gcode_help() {
        let home = *COMMANDS
            .iter()
            .find(|command| command.id == "home")
            .unwrap();
        let qgl = *COMMANDS.iter().find(|command| command.id == "qgl").unwrap();
        // Real shape: QUAD_GANTRY_LEVEL is reported, G28 is not.
        let state = FreshState {
            help: BTreeSet::from(["QUAD_GANTRY_LEVEL".into()]),
            homed_axes: "xyz".into(),
            print_state: "standby".into(),
            idle_state: "ready".into(),
        };
        assert_eq!(state.refusal(home), None);
        assert_eq!(state.refusal(qgl), None);
        // An extended command missing from help is still refused.
        let empty = FreshState {
            help: BTreeSet::new(),
            ..state
        };
        assert_eq!(empty.refusal(home), None);
        assert!(empty.refusal(qgl).unwrap().contains("not reported"));
        assert!(FreshState::is_base_gcode("G28"));
        assert!(FreshState::is_base_gcode("M84"));
        assert!(!FreshState::is_base_gcode("QUAD_GANTRY_LEVEL"));
        assert!(!FreshState::is_base_gcode("GET_POSITION"));
        assert!(!FreshState::is_base_gcode("G"));
    }

    /// HOME and QGL keep their T3 safety gates but are marked review:false so the HUD sends them
    /// without a dialog. Every other T3 command still asks.
    #[test]
    fn home_and_qgl_skip_the_review_without_losing_their_tier() {
        for id in NO_REVIEW {
            let command = *COMMANDS.iter().find(|c| c.id == id).unwrap();
            assert_eq!(command.tier, "T3", "{id} must keep its T3 gates");
        }
        for id in ["home", "qgl", "jog", "motors_off", "mesh"] {
            assert!(NO_REVIEW.contains(&id), "{id} should send without a dialog");
        }
        // The one-way five keep their single confirmation: free-text G-code, a thrown-away print,
        // two Klipper restarts and a printer.cfg rewrite.
        for id in [
            "console",
            "macro",
            "cancel",
            "firmware_restart",
            "restart",
            "z_offset_save",
        ] {
            assert!(!NO_REVIEW.contains(&id), "{id} must still ask once");
        }
        // T3 still refuses mid-print, review flag or not.
        let printing = FreshState {
            help: BTreeSet::from(["QUAD_GANTRY_LEVEL".into()]),
            homed_axes: "xyz".into(),
            print_state: "printing".into(),
            idle_state: "printing".into(),
        };
        let home = *COMMANDS.iter().find(|c| c.id == "home").unwrap();
        assert!(printing.refusal(home).unwrap().contains("blocked while"));
    }

    #[test]
    fn state_and_capability_gates_fail_closed() {
        let command = *COMMANDS.iter().find(|command| command.id == "qgl").unwrap();
        let base = FreshState {
            help: BTreeSet::from(["QUAD_GANTRY_LEVEL".into()]),
            homed_axes: "xyz".into(),
            print_state: "standby".into(),
            idle_state: "ready".into(),
        };
        assert!(base.refusal(command).is_none());
        for print in ["printing", "paused", "future-state"] {
            let state = FreshState {
                print_state: print.into(),
                ..FreshState {
                    help: base.help.clone(),
                    homed_axes: base.homed_axes.clone(),
                    print_state: base.print_state.clone(),
                    idle_state: base.idle_state.clone(),
                }
            };
            assert!(state.refusal(command).is_some());
        }
        let unhomed = FreshState {
            homed_axes: "xy".into(),
            ..base
        };
        assert!(unhomed.refusal(command).unwrap().contains("homed"));
        let missing = FreshState {
            help: BTreeSet::new(),
            ..unhomed
        };
        assert!(missing.refusal(command).unwrap().contains("not reported"));
    }

    #[test]
    fn fake_printer_print_homing_and_capability_gates_never_post() {
        for (label, printer_status, gcode_help) in [
            ("printing", status("printing", "printing", "xyz"), help()),
            ("paused", status("paused", "ready", "xyz"), help()),
            ("unhomed", status("standby", "ready", "xy"), help()),
            (
                "missing-capability",
                status("standby", "ready", "xyz"),
                json!({"result":{}}),
            ),
        ] {
            let (url, worker) = fake(vec![Some(info()), Some(printer_status), Some(gcode_help)]);
            let database = path(label);
            let service = service(url, database.clone(), "live");
            let response = service.handle(
                "POST",
                "/api/commands",
                &headers(&service),
                br#"{"id":"qgl","confirm":true}"#,
                IpAddr::from([127, 0, 0, 1]),
            );
            assert_eq!(response.code, 409, "{label}");
            assert_eq!(body(&response)["status"], "refused", "{label}");
            let requests = worker.join().unwrap();
            assert_eq!(requests.len(), 3, "{label}");
            assert!(
                requests.iter().all(|request| request.starts_with("GET ")),
                "{label}"
            );
            cleanup(&database);
        }
    }

    #[test]
    fn unknown_fresh_state_refuses_without_command_post() {
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("new-state", "ready", "xyz")),
            Some(help()),
        ]);
        let database = path("unknown-state");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"qgl","confirm":true}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 409);
        assert_eq!(body(&response)["status"], "refused");
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.starts_with("GET ")));
        let audit = Connection::open(format!("{database}.commands.sqlite3")).unwrap();
        let outcome: String = audit
            .query_row(
                "SELECT outcome FROM command_audit WHERE phase='result'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "refused");
        drop(audit);
        cleanup(&database);
    }

    #[test]
    fn offline_and_not_ready_are_not_sent() {
        let database = path("offline");
        let offline_service = service("http://127.0.0.1:9".into(), database.clone(), "live");
        let response = offline_service.handle(
            "POST",
            "/api/commands",
            &headers(&offline_service),
            br#"{"id":"position"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 503);
        assert_eq!(body(&response)["status"], "not_sent");
        cleanup(&database);

        let (url, worker) = fake(vec![Some(json!({"result":{"state":"startup"}}))]);
        let database = path("not-ready");
        let not_ready_service = service(url, database.clone(), "live");
        let response = not_ready_service.handle(
            "POST",
            "/api/commands",
            &headers(&not_ready_service),
            br#"{"id":"position"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 503);
        assert_eq!(body(&response)["status"], "not_sent");
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);
    }

    #[test]
    fn enabled_capabilities_get_uses_fresh_state_and_returns_token() {
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
        ]);
        let database = path("capabilities");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "GET",
            "/api/controls",
            &BTreeMap::from([("host".into(), "127.0.0.1:7420".into())]),
            &[],
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 200);
        let value = body(&response);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["token"].as_str().unwrap().len(), 64);
        assert_eq!(value["commands"].as_array().unwrap().len(), COMMANDS.len());
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| command["allowed"] == true));
        assert_eq!(worker.join().unwrap().len(), 3);
        cleanup(&database);
    }

    #[test]
    fn uncertain_transport_is_unknown_and_never_retried() {
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
            None,
        ]);
        let database = path("unknown-outcome");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"home","confirm":true}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 502);
        assert_eq!(body(&response)["status"], "unknown");
        assert_eq!(worker.join().unwrap().len(), 4);
        let audit = Connection::open(format!("{database}.commands.sqlite3")).unwrap();
        let result: String = audit
            .query_row(
                "SELECT outcome FROM command_audit WHERE phase='result'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(result, "unknown");
        drop(audit);
        cleanup(&database);
    }

    #[test]
    fn emergency_bypasses_confirmation_busy_lock_and_preflight() {
        let (url, worker) = fake(vec![Some(json!({"result":"ok"}))]);
        let database = path("emergency");
        let service = service(url, database.clone(), "live");
        service.busy.store(true, Ordering::Release);
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"emergency_stop"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 200);
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("POST /printer/emergency_stop "));
        cleanup(&database);
    }

    #[test]
    fn emergency_is_attempted_when_audit_storage_is_unavailable() {
        let (url, worker) = fake(vec![Some(json!({"result":"ok"}))]);
        let service = service(url, "/proc/stella-command-audit-unavailable".into(), "live");
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"emergency_stop"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 200);
        assert_eq!(body(&response)["status"], "completed");
        assert!(body(&response)["message"]
            .as_str()
            .unwrap()
            .contains("audit was unavailable"));
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("POST /printer/emergency_stop "));
    }

    #[test]
    fn ordinary_command_is_not_sent_when_intent_audit_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let service = service(
            format!("http://{}", listener.local_addr().unwrap()),
            "/proc/stella-command-audit-unavailable".into(),
            "live",
        );
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"position"}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 500);
        assert_eq!(body(&response)["status"], "not_sent");
        assert!(listener.accept().is_err());
    }

    #[test]
    fn disabled_replay_and_request_security_make_no_outbound_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let database = path("disabled");
        let mut cfg = Config {
            moonraker_url: format!("http://{}", listener.local_addr().unwrap()),
            database: database.clone(),
            controls_enabled: true,
            ..Config::default()
        };
        let replay = Service::new(&cfg, "replay").unwrap();
        let replay_headers = BTreeMap::from([("host".into(), "127.0.0.1:7420".into())]);
        let response = replay.handle(
            "GET",
            "/api/controls",
            &replay_headers,
            &[],
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(body(&response)["enabled"], false);
        assert!(listener.accept().is_err());
        cfg.controls_enabled = false;
        let disabled = Service::new(&cfg, "live").unwrap();
        assert_eq!(
            disabled
                .handle(
                    "POST",
                    "/api/commands",
                    &BTreeMap::new(),
                    b"{}",
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            403
        );
        assert!(listener.accept().is_err());
        assert!(!std::path::Path::new(&format!("{database}.commands.sqlite3")).exists());
    }

    #[test]
    fn replay_preview_renders_exact_scripts_without_network_or_audit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let database = path("preview");
        let cfg = Config {
            moonraker_url: format!("http://{}", listener.local_addr().unwrap()),
            database: database.clone(),
            controls_enabled: true,
            ..Config::default()
        };
        let service = Service::new(&cfg, "replay").unwrap();
        let headers = BTreeMap::from([
            ("host".into(), "127.0.0.1:7420".into()),
            ("origin".into(), "http://127.0.0.1:7420".into()),
            ("content-type".into(), "application/json".into()),
        ]);
        for (request, expected) in [
            (r#"{"id":"set_heater","target":"extruder","value":215}"#, "SET_HEATER_TEMPERATURE HEATER=extruder TARGET=215"),
            (r#"{"id":"set_fan","value":0.5}"#, "M106 S128"),
            (r#"{"id":"jog","axis":"Z","value":-0.1}"#, "G91\nG1 Z-0.10 F900\nG90"),
            (r#"{"id":"extrude","value":25}"#, "M83\nG1 E25.00 F300"),
            (r#"{"id":"limit","target":"scv","value":7.5}"#, "SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY=7.5"),
            (r#"{"id":"pressure_advance","value":0.0425}"#, "SET_PRESSURE_ADVANCE ADVANCE=0.0425"),
            (r#"{"id":"macro","target":"CLEAN_NOZZLE","script":"PASSES=2"}"#, "CLEAN_NOZZLE PASSES=2"),
            (r#"{"id":"console","script":"QUERY_ENDSTOPS"}"#, "QUERY_ENDSTOPS"),
            (r#"{"id":"qgl"}"#, "QUAD_GANTRY_LEVEL"),
            (r#"{"id":"resonance_capture","axis":"X"}"#, "TEST_RESONANCES AXIS=X OUTPUT=raw_data"),
            (r#"{"id":"resonance_capture","axis":"belts"}"#, "TEST_RESONANCES AXIS=1,1 OUTPUT=raw_data\nTEST_RESONANCES AXIS=1,-1 OUTPUT=raw_data"),
            // calibration (2026-10-26)
            (r#"{"id":"pid_calibrate","target":"extruder","value":245}"#, "PID_CALIBRATE HEATER=extruder TARGET=245"),
            (r#"{"id":"pid_calibrate","target":"heater_bed","value":100}"#, "PID_CALIBRATE HEATER=heater_bed TARGET=100"),
            (r#"{"id":"probe_calibrate"}"#, "PROBE_CALIBRATE"),
            (r#"{"id":"testz","value":-0.025}"#, "TESTZ Z=-0.025"),
            (r#"{"id":"manual_accept"}"#, "ACCEPT"),
            (r#"{"id":"tuning_tower","params":{"start":"0","factor":"0.005"}}"#, "TUNING_TOWER COMMAND=SET_PRESSURE_ADVANCE PARAMETER=ADVANCE START=0.0000 FACTOR=0.00500"),
            (r#"{"id":"save_config"}"#, "SAVE_CONFIG"),
        ] {
            let response = service.handle(
                "POST",
                "/api/command-preview",
                &headers,
                request.as_bytes(),
                IpAddr::from([127, 0, 0, 1]),
            );
            assert_eq!(response.code, 200, "{request}");
            let value = body(&response);
            assert_eq!(value["script"], expected, "{request}");
            assert_eq!(value["enabled"], false, "{request}");
            assert!(value["reason"].as_str().unwrap().contains("replay"));
        }
        assert!(listener.accept().is_err());
        assert!(!std::path::Path::new(&format!("{database}.commands.sqlite3")).exists());
    }

    #[test]
    fn calibration_commands_refuse_out_of_range_values() {
        let cmd = |id: &str| COMMANDS.iter().copied().find(|c| c.id == id).unwrap();
        let inp = |j: &str| serde_json::from_str::<CommandInput>(j).unwrap();
        assert!(render(cmd("pid_calibrate"), &inp(r#"{"id":"pid_calibrate","target":"extruder","value":320}"#)).is_err(), "over the nozzle limit");
        assert!(render(cmd("pid_calibrate"), &inp(r#"{"id":"pid_calibrate","target":"heater_bed","value":20}"#)).is_err(), "a bed below 40");
        assert!(render(cmd("pid_calibrate"), &inp(r#"{"id":"pid_calibrate","target":"chamber","value":60}"#)).is_err(), "only the two heaters");
        assert!(render(cmd("testz"), &inp(r#"{"id":"testz","value":6}"#)).is_err(), "a probe step over 5 mm");
        assert!(render(cmd("testz"), &inp(r#"{"id":"testz","value":0}"#)).is_err(), "a zero step");
        assert!(render(cmd("tuning_tower"), &inp(r#"{"id":"tuning_tower","params":{"start":"0","factor":"0.2"}}"#)).is_err(), "a factor too steep");
        assert!(render(cmd("tuning_tower"), &inp(r#"{"id":"tuning_tower","params":{"start":"x","factor":"0.005"}}"#)).is_err(), "a start that is not a number");
        for id in ["pid_calibrate", "probe_calibrate", "z_endstop_calibrate", "tuning_tower", "save_config"] {
            assert_eq!(cmd(id).tier, "T3", "{id} is shown first and never runs during a print");
        }
    }

    #[test]
    fn resonance_capture_starts_a_run_instead_of_posting_one_script() {
        // The three preflight GETs and nothing else: the sweep is NOT sent as a gcode POST from
        // this path, because the capture has to subscribe to the accelerometer first.
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
        ]);
        let database = path("resonance");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"resonance_capture","axis":"X","confirm":true}"#,
            IpAddr::from([127, 0, 0, 1]),
        );
        assert_eq!(response.code, 200);
        let value = body(&response);
        assert_eq!(
            value["status"], "started",
            "a 130 s sweep must not be reported as completed: {value}"
        );
        let run_id = value["run_id"].as_i64().expect("a run id to poll");
        assert!(value["message"]
            .as_str()
            .unwrap()
            .contains("/api/resonance/"));
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 3, "only the preflight GETs");
        assert!(requests.iter().all(|r| r.starts_with("GET ")));

        // The run exists and is honest about being in flight.
        let cfg = Config {
            database: database.clone(),
            ..Config::default()
        };
        let detail = crate::resonance::detail(&cfg, run_id).unwrap().unwrap();
        assert_eq!(detail["axis"], "X");
        assert!(["running", "failed"].contains(&detail["status"].as_str().unwrap()));
        // It is audited exactly like every other allowlisted command.
        let audit = Connection::open(format!("{database}.commands.sqlite3")).unwrap();
        let outcome: String = audit
            .query_row(
                "SELECT outcome FROM command_audit WHERE phase='result'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "completed");
        drop(audit);
        cleanup(&database);
        for suffix in [
            ".resonance.sqlite3",
            ".resonance.sqlite3-wal",
            ".resonance.sqlite3-shm",
        ] {
            let _ = fs::remove_file(format!("{database}{suffix}"));
        }
    }

    #[test]
    fn resonance_capture_is_gated_like_every_other_t3() {
        let command = *COMMANDS
            .iter()
            .find(|c| c.id == "resonance_capture")
            .unwrap();
        assert_eq!(command.tier, "T3");
        assert!(command.requires_homed);
        assert_eq!(command.capability, Some("TEST_RESONANCES"));
        assert!(command.script.is_none(), "it is never one fixed script");
        assert!(matches!(command.params, Params::Resonance));

        let database = path("resonance-gate");
        let service = service("http://127.0.0.1:9".into(), database.clone(), "live");
        // Missing confirm, and an axis the printer does not have, both refused before any I/O.
        for (request, code) in [
            (r#"{"id":"resonance_capture","axis":"X"}"#, 400),
            (
                r#"{"id":"resonance_capture","axis":"Z","confirm":true}"#,
                400,
            ),
            (r#"{"id":"resonance_capture","confirm":true}"#, 400),
        ] {
            let response = service.handle(
                "POST",
                "/api/commands",
                &headers(&service),
                request.as_bytes(),
                IpAddr::from([127, 0, 0, 1]),
            );
            assert_eq!(response.code, code, "{request}");
            assert_eq!(body(&response)["status"], "not_sent", "{request}");
        }
        assert!(
            !std::path::Path::new(&format!("{database}.resonance.sqlite3")).exists(),
            "a refused request must not create a run"
        );
        cleanup(&database);
    }

    #[test]
    fn token_origin_content_type_confirmation_and_busy_are_enforced_before_network() {
        let database = path("security");
        let service = service("http://127.0.0.1:9".into(), database.clone(), "live");
        let mut valid = headers(&service);
        let mut bad = valid.clone();
        bad.insert("origin".into(), "http://localhost:7420".into());
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &bad,
                    br#"{"id":"position"}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            403
        );
        bad = valid.clone();
        bad.insert("x-stella-control-token".into(), "wrong".into());
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &bad,
                    br#"{"id":"position"}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            401
        );
        bad = valid.clone();
        bad.insert("content-type".into(), "text/plain".into());
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &bad,
                    br#"{"id":"position"}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            415
        );
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &valid,
                    b"{",
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            400
        );
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &valid,
                    br#"{"id":"position","extra":true}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            400
        );
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &valid,
                    br#"{"id":"home"}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            400
        );
        service.busy.store(true, Ordering::Release);
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &valid,
                    br#"{"id":"home","confirm":true}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            409
        );
        valid.insert("host".into(), "localhost:7420".into());
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &valid,
                    br#"{"id":"position"}"#,
                    IpAddr::from([127, 0, 0, 1])
                )
                .code,
            403
        );
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/commands",
                    &headers(&service),
                    br#"{"id":"position"}"#,
                    IpAddr::from([192, 168, 1, 20]),
                )
                .code,
            403
        );
        cleanup(&database);
    }

    #[test]
    fn controls_default_off_and_non_loopback_opt_in_is_invalid() {
        assert!(!Config::default().controls_enabled);
        assert_eq!(Config::default().command_timeout_ms, 180_000);
        let cfg = Config {
            controls_enabled: true,
            bind: "0.0.0.0".into(),
            moonraker_url: "http://192.168.1.131:7125".into(),
            ..Config::default()
        };
        assert!(cfg.validate().unwrap_err().contains("loopback"));
        let cfg = Config {
            command_timeout_ms: 999,
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    fn get_headers() -> BTreeMap<String, String> {
        BTreeMap::from([("host".into(), "127.0.0.1:7420".into())])
    }
    const LOOPBACK: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));

    /// The live section, an unrelated section that also has a heater_temp, and the commented-out
    /// reference copy — the exact shape of Stella's printer.cfg.
    const CFG: &str = "[heater_fan hotend_fan]\npin: EBBCan: PA1\nheater: extruder\nheater_temp: 50.0    # 2026-09-15: was 30.0\n\n[controller_fan controller_fan]\nheater_temp: 45.0\n\n#[heater_fan hotend_fan]\n#heater_temp: 50.0\n";

    // ---- L3 read endpoints ------------------------------------------------------------------

    #[test]
    fn print_queue_and_objects_read_moonraker_and_never_post() {
        let (url, worker) = fake(vec![
            Some(json!({"result":{"queued_jobs":[
                {"filename":"tray.gcode","job_id":"0000000066D99C70","time_added":1690,"time_in_queue":12.3}
            ],"queue_state":"ready"}})),
            Some(json!({"result":{"status":{"exclude_object":{
                "objects":[
                    {"name":"Cube_id_0","center":[100.0,100.0],"polygon":[[90.0,90.0],[110.0,110.0]]},
                    {"name":"Cube_id_1","center":[200.0,100.0],"polygon":[[190.0,90.0],[210.0,110.0]]}
                ],
                "excluded_objects":["Cube_id_1"],
                "current_object":"Cube_id_0"
            }}}})),
        ]);
        let database = path("print-read");
        let service = service(url, database.clone(), "live");

        let queue = service.handle("GET", "/api/print/queue", &get_headers(), &[], LOOPBACK);
        assert_eq!(queue.code, 200);
        let value = body(&queue);
        assert_eq!(value["queue_state"], "ready");
        assert_eq!(value["queued_jobs"][0]["job_id"], "0000000066D99C70");
        assert_eq!(value["queued_jobs"][0]["filename"], "tray.gcode");
        assert_eq!(value["source"], "GET /server/job_queue/status");

        let objects = service.handle("GET", "/api/print/objects", &get_headers(), &[], LOOPBACK);
        assert_eq!(objects.code, 200);
        let value = body(&objects);
        assert_eq!(value["objects"].as_array().unwrap().len(), 2);
        assert_eq!(value["objects"][0]["name"], "Cube_id_0");
        assert_eq!(value["objects"][0]["center"], json!([100.0, 100.0]));
        assert_eq!(value["objects"][0]["polygon"][0], json!([90.0, 90.0]));
        // The excluded flag is derived, so the UI does not have to cross-reference two lists.
        assert_eq!(value["objects"][0]["excluded"], false);
        assert_eq!(value["objects"][1]["excluded"], true);
        assert_eq!(value["current_object"], "Cube_id_0");

        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.starts_with("GET ")));
        assert!(requests[0].starts_with("GET /server/job_queue/status "));
        assert!(requests[1].starts_with("GET /printer/objects/query?exclude_object "));
        cleanup(&database);
    }

    #[test]
    fn print_start_and_queue_remove_use_moonrakers_own_apis_not_gcode() {
        // print_start is T3: info, status, help, then the print API itself.
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
            Some(json!({"result":"ok"})),
        ]);
        let database = path("print-start");
        let starter = service(url, database.clone(), "live");
        let response = starter.handle(
            "POST",
            "/api/commands",
            &headers(&starter),
            br#"{"id":"print_start","confirm":true,"filename":"tray r8.gcode"}"#,
            LOOPBACK,
        );
        assert_eq!(response.code, 200, "{:?}", body(&response));
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(
            requests[3].starts_with("POST /printer/print/start?filename=tray%20r8.gcode "),
            "{}",
            requests[3].lines().next().unwrap()
        );
        assert!(!requests[3].contains("/printer/gcode/script"));
        cleanup(&database);

        // print_queue_remove is a DELETE, and the verb has to reach the wire as one.
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("standby", "ready", "xyz")),
            Some(help()),
            Some(json!({"result":{"queued_jobs":[]}})),
        ]);
        let database = path("queue-remove");
        let remover = service(url, database.clone(), "live");
        let response = remover.handle(
            "POST",
            "/api/commands",
            &headers(&remover),
            br#"{"id":"print_queue_remove","job_id":"0000000066D99C70"}"#,
            LOOPBACK,
        );
        assert_eq!(response.code, 200, "{:?}", body(&response));
        let requests = worker.join().unwrap();
        assert!(requests[3].starts_with("DELETE /server/job_queue/job?job_ids=0000000066D99C70 "));
        cleanup(&database);
    }

    #[test]
    fn a_refused_bound_never_reaches_the_printer() {
        for request in [
            br#"{"id":"print_start","confirm":true,"filename":"../escape.gcode"}"#.as_slice(),
            br#"{"id":"print_start","confirm":true,"filename":"notgcode.txt"}"#.as_slice(),
            br#"{"id":"print_queue_remove","job_id":"bad id"}"#.as_slice(),
            br#"{"id":"exclude_object","name":"a\nM112"}"#.as_slice(),
            br#"{"id":"load_filament","speed":99}"#.as_slice(),
            br#"{"id":"unload_filament","speed":0}"#.as_slice(),
            br#"{"id":"extrude","value":5,"speed":100}"#.as_slice(),
            br#"{"id":"macro","target":"PRINT_START","params":{"A":"1 2"}}"#.as_slice(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let database = path("bounds");
            let service = service(
                format!("http://{}", listener.local_addr().unwrap()),
                database.clone(),
                "live",
            );
            let response = service.handle(
                "POST",
                "/api/commands",
                &headers(&service),
                request,
                LOOPBACK,
            );
            assert_eq!(response.code, 400, "{}", String::from_utf8_lossy(request));
            assert_eq!(body(&response)["status"], "not_sent");
            // Nothing was dialled: the bound is checked before any socket is opened.
            assert!(listener.accept().is_err());
            cleanup(&database);
        }
    }

    #[test]
    fn reconnect_sends_nothing_to_the_printer() {
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("printing", "printing", "xyz")),
            Some(help()),
        ]);
        let database = path("reconnect");
        let service = service(url, database.clone(), "live");
        // T1, so it is allowed even mid-print — and it still only does the three preflight GETs.
        let response = service.handle(
            "POST",
            "/api/commands",
            &headers(&service),
            br#"{"id":"reconnect"}"#,
            LOOPBACK,
        );
        // No websocket source runs under test, and that is reported as a refusal rather than as
        // a reconnect that silently did nothing.
        assert_eq!(response.code, 409);
        assert_eq!(body(&response)["status"], "refused");
        assert!(body(&response)["error"]
            .as_str()
            .unwrap()
            .contains("no connection to drop"));
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.starts_with("GET ")));
        cleanup(&database);
    }

    // ---- L2 config edit ---------------------------------------------------------------------

    #[test]
    fn heater_fan_edit_previews_a_one_line_diff_then_backs_up_before_uploading() {
        let (url, worker) = fake_text(vec![
            Some(CFG.to_owned()),                                         // GET printer.cfg
            Some(json!({"result":{"action":"create_file"}}).to_string()), // upload backup
            Some(json!({"result":{"action":"create_file"}}).to_string()), // upload printer.cfg
            Some(json!({"result":"ok"}).to_string()),                     // FIRMWARE_RESTART
        ]);
        let database = path("heater-fan");
        let service = service(url, database.clone(), "live");

        let preview = service.handle(
            "POST",
            "/api/config/heater-fan",
            &headers(&service),
            br#"{"heater_temp":45}"#,
            LOOPBACK,
        );
        assert_eq!(preview.code, 200, "{:?}", body(&preview));
        let value = body(&preview);
        let diff = value["diff"].as_str().unwrap();
        // The diff touches exactly one line, and it is the heater_temp inside the live section.
        let minus: Vec<&str> = diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .collect();
        let plus: Vec<&str> = diff
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .collect();
        assert_eq!(minus, vec!["-heater_temp: 50.0    # 2026-09-15: was 30.0"]);
        assert_eq!(plus, vec!["+heater_temp: 45.0    # 2026-09-15: was 30.0"]);
        assert!(diff.contains("[heater_fan hotend_fan]"));
        // The controller_fan heater_temp and the commented copy are context, never changed.
        assert!(!diff.contains("-#heater_temp"));
        assert!(!diff.contains("-[controller_fan"));
        let token = value["token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 64);
        assert_eq!(value["heater_temp"], 45.0);
        assert_eq!(value["file"], "printer.cfg");

        let apply = service.handle(
            "POST",
            "/api/config/heater-fan/apply",
            &headers(&service),
            format!(r#"{{"token":"{token}"}}"#).as_bytes(),
            LOOPBACK,
        );
        assert_eq!(apply.code, 200, "{:?}", body(&apply));
        let value = body(&apply);
        assert!(value["backup"]
            .as_str()
            .unwrap()
            .starts_with("printer.cfg.before-"));
        assert!(value["firmware_restart"]
            .as_str()
            .unwrap()
            .contains("FIRMWARE_RESTART"));

        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("GET /server/files/config/printer.cfg "));
        // Order is the whole point: the backup is written before the file it backs up is replaced.
        assert!(requests[1].starts_with("POST /server/files/upload "));
        assert!(requests[1].contains("name=\"path\"\r\n\r\nprinter.cfg.before-"));
        assert!(
            requests[1].contains("heater_temp: 50.0"),
            "backup holds the ORIGINAL"
        );
        assert!(requests[2].starts_with("POST /server/files/upload "));
        assert!(requests[2].contains("name=\"path\"\r\n\r\nprinter.cfg\r\n"));
        assert!(
            requests[2].contains("heater_temp: 45.0"),
            "upload holds the EDIT"
        );
        assert!(requests[2].contains("name=\"root\"\r\n\r\nconfig"));
        // The commented reference copy rode along untouched in the uploaded bytes.
        assert!(requests[2].contains("#heater_temp: 50.0"));
        assert!(requests[3].starts_with("POST /printer/gcode/script "));
        assert!(requests[3].ends_with(r#"{"script":"FIRMWARE_RESTART"}"#));

        // The token is spent: replaying it changes nothing and opens no connection.
        let replay = service.handle(
            "POST",
            "/api/config/heater-fan/apply",
            &headers(&service),
            format!(r#"{{"token":"{token}"}}"#).as_bytes(),
            LOOPBACK,
        );
        assert_eq!(replay.code, 409);
        cleanup(&database);
    }

    #[test]
    fn heater_fan_refuses_out_of_bounds_before_touching_the_printer() {
        for value in ["29.9", "60.1", "0", "-5", "1000"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let database = path("heater-bounds");
            let service = service(
                format!("http://{}", listener.local_addr().unwrap()),
                database.clone(),
                "live",
            );
            let response = service.handle(
                "POST",
                "/api/config/heater-fan",
                &headers(&service),
                format!(r#"{{"heater_temp":{value}}}"#).as_bytes(),
                LOOPBACK,
            );
            assert_eq!(response.code, 400, "{value}");
            assert!(body(&response)["error"]
                .as_str()
                .unwrap()
                .contains("30 to 60"));
            assert!(listener.accept().is_err(), "{value} reached the printer");
            cleanup(&database);
        }

        // A bad token is refused, and an apply with no waiting preview is a 409, not a write.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let database = path("heater-token");
        let service = service(
            format!("http://{}", listener.local_addr().unwrap()),
            database.clone(),
            "live",
        );
        let response = service.handle(
            "POST",
            "/api/config/heater-fan/apply",
            &headers(&service),
            br#"{"token":"nothing-was-previewed"}"#,
            LOOPBACK,
        );
        assert_eq!(response.code, 409);
        assert_eq!(body(&response)["status"], "not_sent");
        assert!(listener.accept().is_err());
        cleanup(&database);
    }

    #[test]
    fn heater_fan_read_reports_the_live_line_and_its_bounds() {
        let (url, worker) = fake_text(vec![Some(CFG.to_owned())]);
        let database = path("heater-read");
        let service = service(url, database.clone(), "live");
        let response = service.handle(
            "GET",
            "/api/config/heater-fan",
            &get_headers(),
            &[],
            LOOPBACK,
        );
        assert_eq!(response.code, 200, "{:?}", body(&response));
        let value = body(&response);
        assert_eq!(value["heater_temp"], 50.0);
        assert_eq!(value["min"], 30.0);
        assert_eq!(value["max"], 60.0);
        assert_eq!(value["file"], "printer.cfg");
        // Line 4, one-based: the live one, not the commented copy at line 9.
        assert_eq!(value["line"], 4);
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);
    }

    // ---- L5 presets and L8 console ----------------------------------------------------------

    #[test]
    fn presets_and_console_history_never_touch_the_printer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let database = path("presets");
        let service = service(
            format!("http://{}", listener.local_addr().unwrap()),
            database.clone(),
            "live",
        );

        let seeded = body(&service.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK));
        assert_eq!(seeded["materials"], crate::operator::standard_materials());
        assert_eq!(seeded["steps"], json!([0.1, 1, 10, 50]));

        let put = service.handle(
            "PUT",
            "/api/presets",
            &headers(&service),
            br#"{"materials":[{"name":"ASA","nozzle":260,"bed":100}],"steps":[0.1,10],"fan":[0,100]}"#,
            LOOPBACK,
        );
        assert_eq!(put.code, 200, "{:?}", body(&put));
        assert_eq!(body(&put)["materials"].as_array().unwrap().len(), 1);
        assert_eq!(
            body(&service.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK)),
            body(&put)
        );

        // Bounds are the contract's: 0..290 nozzle, 0..120 bed.
        for bad in [
            r#"{"materials":[{"name":"X","nozzle":290.5,"bed":60}],"steps":[1],"fan":[50]}"#,
            r#"{"materials":[{"name":"X","nozzle":250,"bed":121}],"steps":[1],"fan":[50]}"#,
        ] {
            let response = service.handle(
                "PUT",
                "/api/presets",
                &headers(&service),
                bad.as_bytes(),
                LOOPBACK,
            );
            assert_eq!(response.code, 400, "{bad}");
        }
        // The refusal left the stored presets alone.
        assert_eq!(
            body(&service.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK))
                ["materials"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let history =
            body(&service.handle("GET", "/api/console/history", &get_headers(), &[], LOOPBACK));
        assert_eq!(history["history"].as_array().unwrap().len(), 0);
        assert_eq!(history["limit"], 200);
        let pushed = service.handle(
            "POST",
            "/api/console/history",
            &headers(&service),
            br#"{"line":"QUERY_ENDSTOPS"}"#,
            LOOPBACK,
        );
        assert_eq!(pushed.code, 200, "{:?}", body(&pushed));
        assert_eq!(body(&pushed)["history"][0]["line"], "QUERY_ENDSTOPS");
        assert!(body(&pushed)["history"][0]["at"].as_f64().unwrap() > 0.0);
        assert_eq!(
            service
                .handle(
                    "POST",
                    "/api/console/history",
                    &headers(&service),
                    br#"{"line":"M115\nM112"}"#,
                    LOOPBACK
                )
                .code,
            400
        );

        assert!(listener.accept().is_err(), "the printer was contacted");
        cleanup(&database);
    }

    /// WO-20 (2026-09-23), the owner: "Anybody can change their preheat presets, and they should save
    /// within the system." Through the route: no assumed temperatures on first read, a saved list read back by
    /// a new service on the same database (a restart), and every bad list refused in plain words
    /// with nothing stored. None of it reaches the printer.
    #[test]
    fn presets_route_saves_reloads_and_refuses_in_plain_words() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let database = path("presets-route");
        let service = service(url.clone(), database.clone(), "live");
        let first = body(&service.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK));
        assert_eq!(first["materials"], crate::operator::standard_materials());
        let saved = br#"{"materials":[{"name":"PLA","nozzle":215,"bed":60},{"name":"PETG Sunlu","nozzle":245,"bed":85},{"name":"ASA","nozzle":260,"bed":100}],"steps":[0.1,1,10,50],"fan":[0,25,50,75,100]}"#;
        let put = service.handle("PUT", "/api/presets", &headers(&service), saved, LOOPBACK);
        assert_eq!(put.code, 200, "{:?}", body(&put));
        drop(service);
        let restarted = super::tests::service(url, database.clone(), "live");
        let read = body(&restarted.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK));
        assert_eq!(
            read["materials"][1],
            json!({"name":"PETG Sunlu","nozzle":245.0,"bed":85.0})
        );
        assert_eq!(read["materials"].as_array().unwrap().len(), 3);
        for (bad, says) in [
            (
                r#"{"materials":[{"name":"","nozzle":218,"bed":60}],"steps":[1],"fan":[0]}"#,
                "needs a name",
            ),
            (
                r#"{"materials":[{"name":"PLA","nozzle":291,"bed":60}],"steps":[1],"fan":[0]}"#,
                "PLA nozzle must be 0 to 290 °C",
            ),
            (
                r#"{"materials":[{"name":"PLA","nozzle":218,"bed":121}],"steps":[1],"fan":[0]}"#,
                "PLA bed must be 0 to 120 °C",
            ),
            (
                r#"{"materials":[{"name":"PLA","nozzle":218,"bed":60},{"name":"PLA","nozzle":210,"bed":55}],"steps":[1],"fan":[0]}"#,
                "two presets are named PLA",
            ),
            (
                r#"{"materials":[{"name":"cool down","nozzle":0,"bed":0}],"steps":[1],"fan":[0]}"#,
                "Cool down is built in",
            ),
            (
                r#"{"materials":[{"name":"PLA","nozzle":"hot","bed":60}],"steps":[1],"fan":[0]}"#,
                "needs a numeric nozzle",
            ),
        ] {
            let response = restarted.handle(
                "PUT",
                "/api/presets",
                &headers(&restarted),
                bad.as_bytes(),
                LOOPBACK,
            );
            assert_eq!(response.code, 400, "{bad}");
            let error = body(&response)["error"].as_str().unwrap().to_owned();
            assert!(error.contains(says), "{bad}: {error}");
        }
        assert_eq!(
            body(&restarted.handle("GET", "/api/presets", &get_headers(), &[], LOOPBACK)),
            read
        );
        assert!(listener.accept().is_err(), "the printer was contacted");
        cleanup(&database);
    }

    /// Reviewer's decision on WO-20 (2026-09-23): the service gate for the two restarts. With
    /// Klipper in shutdown or error they are listed as allowed and go out after their review, with
    /// no other printer request than the state read; every other command keeps its refusal. With
    /// Klipper ready the ordinary rules apply, and with Moonraker silent or Klipper starting up
    /// nothing goes. The state messages are Klipper's own shapes.
    #[test]
    fn restarts_recover_a_halted_klipper_and_nothing_else_goes() {
        // Klipper's own shapes: the cause line, its FIRMWARE_RESTART advice, and the closing line.
        let halted = |state: &str| {
            let (cause, last) = if state == "error" {
                (
                    "mcu 'EBBCan': Unable to connect",
                    "Error configuring printer",
                )
            } else {
                (
                    "Lost communication with MCU 'EBBCan'",
                    "Printer is shutdown",
                )
            };
            json!({"result":{"state":state,"state_message":format!("{cause}\nOnce the underlying issue is corrected, use the\n\"FIRMWARE_RESTART\" command to reset the firmware, reload the\nconfig, and restart the host software.\n{last}\n")}})
        };
        let post = |service: &Service, body: &[u8]| {
            service.handle("POST", "/api/commands", &headers(service), body, LOOPBACK)
        };

        // Shutdown: the capability list, read with one request.
        let (url, worker) = fake(vec![Some(halted("shutdown"))]);
        let database = path("recover-controls");
        let caps_service = service(url, database.clone(), "live");
        let caps =
            body(&caps_service.handle("GET", "/api/controls", &get_headers(), &[], LOOPBACK));
        let allowed = |id: &str| {
            caps["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(allowed("firmware_restart")["allowed"], true);
        assert_eq!(allowed("klipper_restart")["allowed"], true);
        assert_eq!(allowed("emergency_stop")["allowed"], true);
        assert_eq!(allowed("home")["allowed"], false);
        assert_eq!(
            allowed("home")["reason"],
            "Klippy state is shutdown, not ready; command was not sent"
        );
        assert_eq!(allowed("reconnect")["allowed"], false);
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);

        // Shutdown and error: each restart goes out once, after the state read and nothing else.
        for (state, id, script) in [
            ("shutdown", "firmware_restart", "FIRMWARE_RESTART"),
            ("error", "klipper_restart", "RESTART"),
            ("error", "firmware_restart", "FIRMWARE_RESTART"),
        ] {
            let (url, worker) = fake(vec![Some(halted(state)), Some(json!({"result":"ok"}))]);
            let database = path("recover-send");
            let sender = service(url, database.clone(), "live");
            let unconfirmed = post(&sender, format!(r#"{{"id":"{id}"}}"#).as_bytes());
            assert_eq!(
                unconfirmed.code, 400,
                "{state} {id}: the review is still required"
            );
            let response = post(
                &sender,
                format!(r#"{{"id":"{id}","confirm":true}}"#).as_bytes(),
            );
            assert_eq!(response.code, 200, "{state} {id}: {:?}", body(&response));
            let requests = worker.join().unwrap();
            assert_eq!(requests.len(), 2, "{state} {id}");
            assert!(requests[0].starts_with("GET /printer/info "));
            assert!(requests[1].starts_with("POST /printer/gcode/script "));
            assert!(requests[1].ends_with(&format!(r#"{{"script":"{script}"}}"#)));
            cleanup(&database);
        }

        // Shutdown: anything else is refused as before, not sent, after the state read alone.
        let (url, worker) = fake(vec![Some(halted("shutdown"))]);
        let database = path("recover-home");
        let home_service = service(url, database.clone(), "live");
        let response = post(&home_service, br#"{"id":"home","confirm":true}"#);
        assert_eq!(response.code, 503);
        assert_eq!(body(&response)["status"], "not_sent");
        assert!(body(&response)["error"]
            .as_str()
            .unwrap()
            .contains("Klippy state is shutdown"));
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);

        // Ready: the ordinary rules still decide, so a restart during a print is refused.
        let (url, worker) = fake(vec![
            Some(info()),
            Some(status("printing", "printing", "xyz")),
            Some(help()),
        ]);
        let database = path("recover-ready");
        let ready_service = service(url, database.clone(), "live");
        let response = post(
            &ready_service,
            br#"{"id":"firmware_restart","confirm":true}"#,
        );
        assert_eq!(response.code, 409);
        assert!(body(&response)["error"]
            .as_str()
            .unwrap()
            .contains("T3 commands are blocked while printing"));
        assert_eq!(worker.join().unwrap().len(), 3);
        cleanup(&database);

        // NEXT review 2026-09-24: CANCEL_PRINT is the one T3 command a print does not block, in
        // both printing and paused, and it goes out as exactly CANCEL_PRINT.
        for print in ["printing", "paused"] {
            let (url, worker) = fake(vec![
                Some(info()),
                Some(status(print, "printing", "xyz")),
                Some(help()),
                Some(json!({"result":"ok"})),
            ]);
            let database = path(&format!("cancel-{print}"));
            let cancel_service = service(url, database.clone(), "live");
            let response = post(&cancel_service, br#"{"id":"cancel","confirm":true}"#);
            assert_eq!(
                response.code,
                200,
                "cancel while {print}: {:?}",
                body(&response)
            );
            let requests = worker.join().unwrap();
            assert!(
                requests[3].ends_with(r#"{"script":"CANCEL_PRINT"}"#),
                "{:?}",
                requests[3]
            );
            cleanup(&database);
        }

        // Moonraker silent, or Klipper still starting: nothing goes.
        let database = path("recover-unreachable");
        let silent = service("http://127.0.0.1:9".into(), database.clone(), "live");
        let response = post(&silent, br#"{"id":"firmware_restart","confirm":true}"#);
        assert_eq!(response.code, 503);
        assert_eq!(body(&response)["status"], "not_sent");
        cleanup(&database);
        let (url, worker) = fake(vec![Some(
            json!({"result":{"state":"startup","state_message":"\nPrinter is not ready\nThe klippy host software is attempting to connect.  Please\nretry in a few moments.\n"}}),
        )]);
        let database = path("recover-startup");
        let startup_service = service(url, database.clone(), "live");
        let response = post(
            &startup_service,
            br#"{"id":"klipper_restart","confirm":true}"#,
        );
        assert_eq!(response.code, 503);
        assert_eq!(body(&response)["status"], "not_sent");
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);
    }

    #[test]
    fn console_help_is_fetched_once_and_then_served_from_cache() {
        // One upstream reply for two requests: a second fetch would hang and fail the test.
        let (url, worker) = fake(vec![Some(
            json!({"result":{"QUERY_ENDSTOPS":"Report on the state of each endstop","G28":"Home"}}),
        )]);
        let database = path("console-help");
        let service = service(url, database.clone(), "live");
        let first = service.handle("GET", "/api/console/help", &get_headers(), &[], LOOPBACK);
        assert_eq!(first.code, 200, "{:?}", body(&first));
        let value = body(&first);
        // Sorted by name, so the autocomplete list is stable between calls.
        assert_eq!(value["commands"][0]["name"], "G28");
        assert_eq!(value["commands"][1]["name"], "QUERY_ENDSTOPS");
        assert_eq!(
            value["commands"][1]["help"],
            "Report on the state of each endstop"
        );
        let second = service.handle("GET", "/api/console/help", &get_headers(), &[], LOOPBACK);
        assert_eq!(body(&second)["commands"], value["commands"]);
        assert_eq!(body(&second)["cached_at"], value["cached_at"]);
        assert_eq!(worker.join().unwrap().len(), 1);
        cleanup(&database);
    }

    #[test]
    fn macros_carry_the_parameters_read_from_their_own_bodies() {
        let (url, worker) = fake(vec![
            Some(
                json!({"result":{"LOAD_FILAMENT":"Park front right, heat, purge 50 mm","G28":"Home"}}),
            ),
            Some(json!({"result":{"status":{"configfile":{"settings":{
                "gcode_macro load_filament":{"gcode":"{% set temp = params.TEMP|default(220)|int %}\nG1 E50 F300"}
            }}}}})),
        ]);
        let database = path("macros");
        let service = service(url, database.clone(), "live");
        let response = service.handle("GET", "/api/macros", &get_headers(), &[], LOOPBACK);
        assert_eq!(response.code, 200, "{:?}", body(&response));
        let macros = body(&response);
        let macros = macros["macros"].as_array().unwrap();
        assert_eq!(macros[0]["name"], "G28");
        // G28 is not a gcode_macro, so it has no body and therefore no parameters.
        assert_eq!(macros[0]["params"], json!([]));
        assert_eq!(macros[1]["name"], "LOAD_FILAMENT");
        assert_eq!(
            macros[1]["params"],
            json!([{"name":"TEMP","default":"220"}])
        );
        assert_eq!(worker.join().unwrap().len(), 2);
        cleanup(&database);
    }

    /// WO-19 (2026-09-23): the HUD's Macros list is the printer's gcode_macro sections, so the
    /// service flags which gcode/help entries have one. The fixture is the shape Klipper returns:
    /// gcode/help lists registered commands and macros alike (never base G-codes such as G1), and
    /// macro bodies sit under lowercased `gcode_macro <name>` sections in configfile settings.
    #[test]
    fn macros_are_flagged_from_their_gcode_macro_sections() {
        let (url, worker) = fake(vec![
            Some(
                json!({"result":{"LOAD_FILAMENT":"Park front right, heat, purge 50 mm","BED_MESH_CALIBRATE":"Perform Mesh Bed Leveling"}}),
            ),
            Some(json!({"result":{"status":{"configfile":{"settings":{
                "gcode_macro load_filament":{"gcode":"G1 E50 F300"}
            }}}}})),
        ]);
        let database = path("macro-flag");
        let service = service(url, database.clone(), "live");
        let macros = body(&service.handle("GET", "/api/macros", &get_headers(), &[], LOOPBACK));
        let macros = macros["macros"].as_array().unwrap();
        assert_eq!(macros[0]["name"], "BED_MESH_CALIBRATE");
        assert_eq!(macros[0]["macro"], false);
        assert_eq!(macros[1]["name"], "LOAD_FILAMENT");
        assert_eq!(macros[1]["macro"], true);
        assert_eq!(worker.join().unwrap().len(), 2);
        cleanup(&database);
    }

    #[test]
    fn control_routes_reject_the_wrong_verb_and_unknown_paths() {
        let database = path("routes");
        let service = service("http://127.0.0.1:9".into(), database.clone(), "live");
        for (method, path) in [
            ("POST", "/api/presets"),
            ("PUT", "/api/console/history"),
            ("GET", "/api/config/heater-fan/apply"),
            ("PUT", "/api/config/heater-fan"),
            ("POST", "/api/print/queue"),
            ("POST", "/api/console/help"),
            ("GET", "/api/commands"),
        ] {
            let response = service.handle(method, path, &headers(&service), b"{}", LOOPBACK);
            assert_eq!(response.code, 405, "{method} {path}");
        }
        assert_eq!(routed("/api/nope"), None);
        assert_eq!(routed("/api/presets"), Some(["GET", "PUT"].as_slice()));
        cleanup(&database);
    }

    #[test]
    fn command_post_rejects_redirect_and_uses_separate_timeout() {
        let (url, worker) = one_response("302 Found", Duration::ZERO);
        let cfg = Config {
            moonraker_url: url,
            ..Config::default()
        };
        assert!(Moonraker::new(&cfg)
            .post("/printer/emergency_stop", None)
            .unwrap_err()
            .contains("HTTP 302"));
        worker.join().unwrap();

        let (url, worker) = one_response("200 OK", Duration::from_millis(250));
        let cfg = Config {
            moonraker_url: url,
            request_timeout_ms: 100,
            command_timeout_ms: 1_000,
            ..Config::default()
        };
        assert_eq!(
            Moonraker::new(&cfg)
                .post("/printer/emergency_stop", None)
                .unwrap(),
            "ok"
        );
        worker.join().unwrap();
    }

    // ---- farm board WO-01 (2026-10-12): our 2026-09-28 list, items 1-3 and 11 --------------------
    // These run against the fake Moonraker in src/fakes.rs, which records every request it gets, so
    // each assertion is on the exact bytes that left the service.

    fn fake_service(label: &str) -> (crate::fakes::FakeMoonraker, std::sync::Arc<Service>, String) {
        let fake = crate::fakes::FakeMoonraker::start(0);
        let database = path(label);
        let service = std::sync::Arc::new(service(fake.url(), database.clone(), "live"));
        (fake, service, database)
    }
    fn post(service: &Service, route: &str, payload: &str) -> Response {
        service.handle(
            "POST",
            route,
            &headers(service),
            payload.as_bytes(),
            IpAddr::from([127, 0, 0, 1]),
        )
    }

    // WO-04: when the farm is past its trial and this printer is beyond the free five, controls are
    // refused with a plain reason; a running print is never interrupted.
    #[test]
    fn locked_controls_are_refused_with_a_plain_reason() {
        let (fake, service, database) = fake_service("licence-lock");
        fake.state.lock().unwrap().load_secs = 0.2;
        // Unlocked: a control is accepted (it reaches the fake).
        let r = post(
            &service,
            "/api/commands",
            r#"{"id":"set_heater","target":"heater_bed","value":60,"confirm":true}"#,
        );
        assert_eq!(r.code, 200, "unlocked: {}", body(&r));
        assert!(service.enabled());
        // Lock it.
        service.set_licence_locked(
            true,
            "The trial has ended. A farm licence turns printers beyond the free five back on."
                .into(),
        );
        assert!(!service.enabled());
        assert!(service
            .disabled_reason()
            .contains("farm licence turns printers beyond the free five back on"));
        let r = post(
            &service,
            "/api/commands",
            r#"{"id":"set_heater","target":"heater_bed","value":60,"confirm":true}"#,
        );
        assert_eq!(r.code, 503, "locked: {}", body(&r));
        assert_eq!(body(&r)["status"], "not_sent");
        cleanup(&database);
    }

    /// 2026-10-22 (filament board): the shake after a load is a named command with fixed moves. It goes out only when the printer is homed and not printing or paused, with no
    /// review dialog; it is net zero (+2 -4 +4 -4 +4 -4 +2 mm along X), and the printer's own G-code state is saved and put back around it.
    #[test]
    fn the_filament_shake_is_homed_only_never_mid_print_and_leaves_the_head_where_it_was() {
        let (fake, service, database) = fake_service("filament-shake");
        let go = |s: &Service| post(s, "/api/commands", r#"{"id":"filament_shake","confirm":true}"#);
        let preview = body(&post(&service, "/api/command-preview", r#"{"id":"filament_shake"}"#));
        assert_eq!(preview["tier"], "T3");
        assert_eq!(preview["review"], false, "a fixed action: no dialog in front of it");
        // not homed: refused, nothing sent
        fake.state.lock().unwrap().homed = "".into();
        let r = go(&service);
        assert_ne!(r.code, 200, "{:?}", body(&r));
        assert!(fake.scripts().is_empty(), "{:?}", fake.scripts());
        // printing or paused: refused, nothing sent
        for state in ["printing", "paused"] {
            {
                let mut f = fake.state.lock().unwrap();
                f.homed = "xyz".into();
                f.print_state = state.into();
            }
            let r = go(&service);
            assert_ne!(r.code, 200, "shake while {state}: {:?}", body(&r));
        }
        assert!(fake.scripts().is_empty(), "{:?}", fake.scripts());
        // homed and idle: it goes, exactly this, and the head is where it was
        {
            let mut f = fake.state.lock().unwrap();
            f.print_state = "standby".into();
            f.pos = [175.0, 175.0, 10.0];
        }
        let r = go(&service);
        assert_eq!(r.code, 200, "{:?}", body(&r));
        let sent = fake.scripts();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], SHAKE_SCRIPT);
        assert!(sent[0].starts_with("SAVE_GCODE_STATE") && sent[0].ends_with("RESTORE_GCODE_STATE NAME=STELLA_SHAKE"));
        let net: f64 = sent[0]
            .lines()
            .filter_map(|l| l.strip_prefix("G1 X"))
            .filter_map(|l| l.split_whitespace().next()?.parse::<f64>().ok())
            .sum();
        assert_eq!(net, 0.0, "the moves cancel out");
        assert_eq!(fake.state.lock().unwrap().pos, [175.0, 175.0, 10.0]);
        // A head at the end of travel (the fake's X is 0..350) shakes only on the side that has room, still net zero.
        let sum = |s: &str| s.lines().filter_map(|l| l.strip_prefix("G1 X")?.split_whitespace().next()?.parse::<f64>().ok()).sum::<f64>();
        for (x, first) in [(0.5, "G1 X4 F12000"), (349.0, "G1 X-4 F12000")] {
            fake.state.lock().unwrap().pos = [x, 175.0, 10.0];
            let r = go(&service);
            assert_eq!(r.code, 200, "{:?}", body(&r));
            let last = fake.scripts().last().cloned().unwrap();
            assert_eq!(last.lines().nth(2), Some(first), "at X {x}: {last}");
            assert_eq!(sum(&last), 0.0);
        }
        assert!(shake_script(1.0, 0.0, 3.0).is_none(), "no room either way: not sent");
        assert_eq!(shake_script(175.0, 0.0, 350.0).as_deref(), Some(SHAKE_SCRIPT));
        cleanup(&database);
    }

    /// Opus review 2026-10-22: a filament job's timers ran in the page, so closing the page in the reload wait left the nozzle hot for ever. The page now holds a guard in
    /// the service; if it is not renewed the service turns the nozzle heater off by itself. Taken away (0), nothing is sent.
    #[test]
    fn a_heat_guard_that_runs_out_turns_the_nozzle_off_and_a_cleared_one_sends_nothing() {
        let (fake, service, database) = fake_service("heat-guard");
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        Service::spawn_heat_guard(&service, stop.clone());
        let r = post(&service, "/api/filament/guard", r#"{"off_in_s":901}"#);
        assert_eq!(r.code, 400, "past the top is refused");
        let r = post(&service, "/api/filament/guard", r#"{"off_in_s":30}"#);
        assert_eq!(r.code, 200, "{:?}", body(&r));
        let r = post(&service, "/api/filament/guard", r#"{"off_in_s":0}"#);
        assert_eq!(r.code, 200);
        std::thread::sleep(Duration::from_millis(1500));
        assert!(fake.scripts().is_empty(), "cleared: {:?}", fake.scripts());
        let r = post(&service, "/api/filament/guard", r#"{"off_in_s":1}"#);
        assert_eq!(r.code, 200);
        let t0 = std::time::Instant::now();
        while fake.scripts().is_empty() && t0.elapsed() < Duration::from_secs(6) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let sent = fake.scripts();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("SET_HEATER_TEMPERATURE HEATER=extruder TARGET=0"), "{sent:?}");
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(fake.scripts().len(), 1, "once, not every second");
        stop.store(true, Ordering::Relaxed);
        cleanup(&database);
    }


    /// 2026-10-22 (Stella): CANCEL in the paused bar sent nothing, so Klipper got PAUSE and never CANCEL_PRINT. The page's completed hold posts exactly this, and it reaches the
    /// printer as CANCEL_PRINT, which (as Mainsail's macro on Stella) cancels the print and turns both heaters off. Without confirm it is refused and nothing is sent.
    #[test]
    fn a_confirmed_cancel_while_paused_reaches_klipper_as_cancel_print_and_the_heaters_go_off() {
        let (fake, service, database) = fake_service("cancel-paused");
        {
            let mut s = fake.state.lock().unwrap();
            s.print_state = "paused".into();
            s.filename = "ring.gcode".into();
            s.nozzle_target = 250.0;
            s.nozzle = 250.0;
            s.bed_target = 60.0;
            s.bed = 60.0;
        }
        let unconfirmed = post(&service, "/api/commands", r#"{"id":"cancel"}"#);
        assert_ne!(unconfirmed.code, 200, "a cancel without confirm is not sent");
        assert!(fake.scripts().is_empty(), "nothing reached the printer: {:?}", fake.scripts());
        let r = post(&service, "/api/commands", r#"{"id":"cancel","confirm":true}"#);
        assert_eq!(r.code, 200, "{:?}", body(&r));
        assert_eq!(fake.scripts(), vec!["CANCEL_PRINT"]);
        let s = fake.state.lock().unwrap();
        assert_eq!(s.print_state, "cancelled");
        assert_eq!((s.nozzle_target, s.bed_target), (0.0, 0.0), "both heaters off");
        drop(s);
        cleanup(&database);
    }

    /// Item 1-3: Load and Unload send the printer's own macro with the chosen temperature, in one
    /// press (no review), and are refused while a print runs. Before this the wizard sent `macro`,
    /// a T3 free-text command that stopped at a review dialog nobody could see, and nothing reached
    /// Klipper (gcode_store held no LOAD_FILAMENT).
    #[test]
    fn load_and_unload_send_the_named_macro_with_the_nozzle_temperature() {
        let (fake, service, database) = fake_service("filament-macro");
        fake.state.lock().unwrap().load_secs = 0.2;
        let preview = post(
            &service,
            "/api/command-preview",
            r#"{"id":"filament_load","value":250}"#,
        );
        let preview = body(&preview);
        assert_eq!(preview["script"], "LOAD_FILAMENT TEMP=250");
        assert_eq!(preview["tier"], "T3");
        assert_eq!(
            preview["review"], false,
            "one press: no dialog in front of a named action"
        );
        assert_eq!(
            post(
                &service,
                "/api/commands",
                r#"{"id":"filament_load","value":250,"confirm":true}"#
            )
            .code,
            200
        );
        assert_eq!(
            post(
                &service,
                "/api/commands",
                r#"{"id":"filament_unload","value":240,"confirm":true}"#
            )
            .code,
            200
        );
        assert_eq!(
            fake.scripts(),
            vec!["LOAD_FILAMENT TEMP=250", "UNLOAD_FILAMENT TEMP=240"]
        );
        // Out-of-range or missing temperatures never reach the printer.
        for bad in [
            r#"{"id":"filament_load","value":100,"confirm":true}"#,
            r#"{"id":"filament_load","value":400,"confirm":true}"#,
            r#"{"id":"filament_load","confirm":true}"#,
        ] {
            assert_eq!(post(&service, "/api/commands", bad).code, 400, "{bad}");
        }
        assert_eq!(fake.scripts().len(), 2);
        // T3: refused while printing, nothing sent.
        fake.state.lock().unwrap().print_state = "printing".into();
        let refused = post(
            &service,
            "/api/commands",
            r#"{"id":"filament_load","value":250,"confirm":true}"#,
        );
        assert_eq!(refused.code, 409);
        assert_eq!(fake.scripts().len(), 2);
        cleanup(&database);
    }

    /// Item 11: a quad gantry level holds the "one ordinary command" slot for its whole run, and bed
    /// heat was refused for that time. Heater, fan, speed and flow have their own slot now.
    #[test]
    fn heat_is_accepted_while_a_gantry_level_is_running() {
        let (fake, service, database) = fake_service("heat-during-qgl");
        fake.state.lock().unwrap().qgl_secs = 2.5;
        let level = {
            let service = service.clone();
            thread::spawn(move || {
                post(&service, "/api/commands", r#"{"id":"qgl","confirm":true}"#).code
            })
        };
        // Wait until the macro is really running on the printer.
        let started = std::time::Instant::now();
        while !fake.scripts().iter().any(|s| s == "QUAD_GANTRY_LEVEL") {
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "the level never started"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let bed = post(
            &service,
            "/api/commands",
            r#"{"id":"set_heater","target":"heater_bed","value":55,"confirm":false}"#,
        );
        assert_eq!(bed.code, 200, "{}", String::from_utf8_lossy(&bed.body));
        let nozzle = post(
            &service,
            "/api/commands",
            r#"{"id":"set_heater","target":"extruder","value":250,"confirm":false}"#,
        );
        assert_eq!(nozzle.code, 200);
        assert!(
            !level.is_finished(),
            "the heat went out while the level was still running"
        );
        // Another heavy command still waits its turn, as before.
        let home = post(&service, "/api/commands", r#"{"id":"home","confirm":true}"#);
        assert_eq!(home.code, 409);
        assert_eq!(level.join().unwrap(), 200);
        let scripts = fake.scripts();
        assert!(
            scripts.contains(&"SET_HEATER_TEMPERATURE HEATER=heater_bed TARGET=55".to_string()),
            "{scripts:?}"
        );
        assert!(scripts.contains(&"SET_HEATER_TEMPERATURE HEATER=extruder TARGET=250".to_string()));
        assert!(!scripts.contains(&"G28".to_string()));
        cleanup(&database);
    }

    // ---- farm board WO-02 (2026-10-12): the driver seam -------------------------------------------

    fn other_service(label: &str) -> (Service, String) {
        let database = path(label);
        let cfg = Config {
            driver: "testbrand".into(),
            database: database.clone(),
            controls_enabled: true,
            request_timeout_ms: 500,
            ..Config::default()
        };
        (Service::new(&cfg, "live").unwrap(), database)
    }
    fn get(service: &Service, route: &str) -> Response {
        service.handle(
            "GET",
            route,
            &BTreeMap::from([("host".into(), "127.0.0.1:7420".into())]),
            &[],
            IpAddr::from([127, 0, 0, 1]),
        )
    }

    /// The WO's case: a printer with no move capability. What it cannot do is reported as
    /// `supported:false` with the brand's own words (the page hides it), the request is refused before
    /// anything is rendered, and nothing reaches the driver. What it can do arrives as a neutral action.
    #[test]
    fn a_printer_that_cannot_move_says_so_and_nothing_reaches_it() {
        let (service, database) = other_service("testbrand");
        let value = body(&get(&service, "/api/controls"));
        assert_eq!(value["driver"], "testbrand");
        assert_eq!(value["brand"], "Testbrand");
        assert_eq!(value["capabilities"]["jog"], false);
        assert_eq!(value["capabilities"]["heat"], true);
        let command = |id: &str| {
            value["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap()
                .clone()
        };
        let jog = command("jog");
        assert_eq!(jog["supported"], false);
        assert_eq!(jog["allowed"], false);
        assert_eq!(jog["reason"], "Testbrand cannot move the head by hand");
        for id in [
            "home",
            "macro",
            "console",
            "qgl",
            "mesh",
            "filament_load",
            "set_fan",
        ] {
            assert_eq!(command(id)["supported"], false, "{id}");
        }
        let heat = command("set_heater");
        assert_eq!(
            (heat["supported"].clone(), heat["allowed"].clone()),
            (json!(true), json!(true))
        );

        // Refused before it is rendered or sent, in the brand's words.
        let refused = post(
            &service,
            "/api/commands",
            r#"{"id":"jog","axis":"X","value":5,"confirm":true}"#,
        );
        assert_eq!(refused.code, 409);
        assert!(body(&refused)["error"]
            .as_str()
            .unwrap()
            .contains("Testbrand cannot move the head by hand"));
        assert_eq!(
            post(&service, "/api/command-preview", r#"{"id":"home"}"#).code,
            409
        );
        assert!(crate::drivers::testing::asked(&database).is_empty());

        // What it can do reaches it as the neutral action, validated exactly as for Klipper.
        assert_eq!(
            post(
                &service,
                "/api/commands",
                r#"{"id":"set_heater","target":"extruder","value":210}"#
            )
            .code,
            200
        );
        assert_eq!(
            post(
                &service,
                "/api/commands",
                r#"{"id":"set_heater","target":"extruder","value":999}"#
            )
            .code,
            400
        );
        assert_eq!(
            post(
                &service,
                "/api/commands",
                r#"{"id":"print_start","filename":"plate.gcode","confirm":true}"#
            )
            .code,
            200
        );
        assert_eq!(
            post(&service, "/api/commands", r#"{"id":"pause"}"#).code,
            200
        );
        assert_eq!(
            post(&service, "/api/commands", r#"{"id":"emergency_stop"}"#).code,
            200
        );
        assert_eq!(
            crate::drivers::testing::asked(&database),
            vec![
                "SetTemperature { heater: Nozzle, celsius: 210.0 }".to_string(),
                "StartPrint { filename: \"plate.gcode\" }".into(),
                "Pause".into(),
                "emergency_stop".into(),
            ]
        );
        // Files come from the driver; Klipper-only reads answer empty, not with an error.
        assert_eq!(
            body(&get(&service, "/api/files"))["files"][0]["path"],
            "plate.gcode"
        );
        assert_eq!(body(&get(&service, "/api/macros"))["macros"], json!([]));
        assert_eq!(
            body(&get(&service, "/api/print/queue"))["queue_state"],
            "unavailable"
        );
        cleanup(&database);
    }

    /// A brand that is configured but not built answers plainly and does nothing: it is never quietly
    /// treated as Moonraker.
    #[test]
    fn a_brand_that_is_not_built_is_refused_not_treated_as_moonraker() {
        let database = path("unbuilt");
        let cfg = Config {
            driver: "nonesuch".into(),
            database: database.clone(),
            controls_enabled: true,
            ..Config::default()
        };
        assert!(crate::driver::open(&cfg).is_err());
        let service = Service::new(&cfg, "live").unwrap();
        let refused = post(
            &service,
            "/api/commands",
            r#"{"id":"set_heater","target":"extruder","value":210}"#,
        );
        assert_eq!(refused.code, 409);
        assert!(body(&refused)["error"].as_str().unwrap().contains("cannot"));
        cleanup(&database);
    }

    /// The neutral action is read back from the rendered Klipper text, so a request is bounded once.
    /// For every command with a neutral meaning, render -> action -> script is the text rendered.
    #[test]
    fn the_neutral_action_round_trips_through_the_rendered_script() {
        let cases = [
            r#"{"id":"set_heater","target":"extruder","value":210}"#,
            r#"{"id":"set_heater","target":"heater_bed","value":60}"#,
            r#"{"id":"set_fan","value":0.5}"#,
            r#"{"id":"speed_factor","value":120}"#,
            r#"{"id":"extrude_factor","value":95}"#,
            r#"{"id":"jog","axis":"X","value":10}"#,
            r#"{"id":"jog","axis":"Z","value":-0.5}"#,
            r#"{"id":"home"}"#,
            r#"{"id":"motors_off"}"#,
            r#"{"id":"extrude","value":5}"#,
            r#"{"id":"extrude","value":30}"#,
            r#"{"id":"load_filament"}"#,
            r#"{"id":"unload_filament","speed":8}"#,
            r#"{"id":"pause"}"#,
            r#"{"id":"resume"}"#,
            r#"{"id":"cancel"}"#,
            r#"{"id":"console","script":"M115"}"#,
        ];
        for case in cases {
            let input: CommandInput = serde_json::from_str(case).unwrap();
            let command = *COMMANDS.iter().find(|c| c.id == input.id).unwrap();
            let sent = render(command, &input).unwrap();
            let action = action_from_sent(command, &sent, &input)
                .unwrap_or_else(|| panic!("no action for {case}"));
            assert_eq!(
                crate::driver::MoonrakerDriver::script_for(&action).as_deref(),
                sent.script(),
                "{case}"
            );
        }
        // No neutral meaning: Klipper-only commands give None, and the printer is told so.
        for case in [
            r#"{"id":"qgl"}"#,
            r#"{"id":"mesh"}"#,
            r#"{"id":"macro","target":"PRINT_START"}"#,
        ] {
            let input: CommandInput = serde_json::from_str(case).unwrap();
            let command = *COMMANDS.iter().find(|c| c.id == input.id).unwrap();
            let sent = render(command, &input).unwrap();
            assert!(action_from_sent(command, &sent, &input).is_none(), "{case}");
        }
    }

    /// The driver's own snapshot of a Klipper printer is the read FreshState::fetch has always made.
    #[test]
    fn the_moonraker_driver_snapshot_is_the_fresh_state_commands_read() {
        let fake = crate::fakes::FakeMoonraker::start(0);
        let cfg = Config {
            moonraker_url: fake.url(),
            request_timeout_ms: 800,
            ..Config::default()
        };
        use crate::driver::Driver as _;
        let snap = crate::driver::MoonrakerDriver::new(&cfg)
            .snapshot()
            .unwrap();
        let Fresh::Ready(fresh) = FreshState::fetch(&Moonraker::new(&cfg)).unwrap() else {
            panic!("not ready")
        };
        assert_eq!(snap.state, "ready");
        assert_eq!(
            (
                snap.homed_axes,
                snap.print_state,
                snap.idle_state,
                snap.help
            ),
            (
                fresh.homed_axes,
                fresh.print_state,
                fresh.idle_state,
                fresh.help
            )
        );
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    fn blank(id: &str) -> CommandInput {
        CommandInput {
            id: id.into(),
            confirm: true,
            target: None,
            value: None,
            axis: None,
            script: None,
            speed: None,
            filename: None,
            heat: false,
            job_id: None,
            name: None,
            params: None,
        }
    }
    fn input(
        id: &str,
        target: Option<&str>,
        value: Option<f64>,
        axis: Option<&str>,
    ) -> CommandInput {
        CommandInput {
            target: target.map(str::to_owned),
            value,
            axis: axis.map(str::to_owned),
            ..blank(id)
        }
    }
    fn typed(id: &str, target: Option<&str>, script: Option<&str>) -> CommandInput {
        CommandInput {
            target: target.map(str::to_owned),
            script: script.map(str::to_owned),
            ..blank(id)
        }
    }
    fn command(id: &str) -> Command {
        *COMMANDS.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn parametric_scripts_are_exact_and_bounded() {
        let ok = |id, t, v, a| render(command(id), &input(id, t, v, a)).unwrap().describe();
        let refused = |id, t, v, a| render(command(id), &input(id, t, v, a)).is_err();
        assert_eq!(
            ok("set_heater", Some("extruder"), Some(215.0), None),
            "SET_HEATER_TEMPERATURE HEATER=extruder TARGET=215"
        );
        assert_eq!(
            ok("set_heater", Some("heater_bed"), Some(0.0), None),
            "SET_HEATER_TEMPERATURE HEATER=heater_bed TARGET=0"
        );
        assert_eq!(
            ok("set_heater", Some("extruder"), Some(290.0), None),
            "SET_HEATER_TEMPERATURE HEATER=extruder TARGET=290"
        );
        assert!(refused("set_heater", Some("extruder"), Some(290.1), None));
        assert!(refused("set_heater", Some("heater_bed"), Some(121.0), None));
        assert!(refused("set_heater", Some("chamber"), Some(40.0), None));
        assert!(refused(
            "set_heater",
            Some("extruder"),
            Some(f64::NAN),
            None
        ));
        assert_eq!(ok("set_fan", None, Some(0.5), None), "M106 S128");
        assert_eq!(ok("set_fan", None, Some(1.0), None), "M106 S255");
        assert!(refused("set_fan", None, Some(1.5), None));
        assert_eq!(ok("speed_factor", None, Some(120.0), None), "M220 S120");
        assert_eq!(ok("extrude_factor", None, Some(95.0), None), "M221 S95");
        assert!(refused("speed_factor", None, Some(49.0), None));
        assert!(refused("extrude_factor", None, Some(201.0), None));
        assert_eq!(
            ok("jog", None, Some(-10.0), Some("X")),
            "G91\nG1 X-10.00 F6000\nG90"
        );
        assert_eq!(
            ok("jog", None, Some(0.1), Some("Z")),
            "G91\nG1 Z0.10 F900\nG90"
        );
        assert!(refused("jog", None, Some(0.0), Some("X")));
        assert!(refused("jog", None, Some(51.0), Some("Y")));
        assert!(refused("jog", None, Some(5.0), Some("E")));
        assert_eq!(ok("motors_off", None, None, None), "M84");
        assert!(command("jog").requires_homed && command("jog").tier == "T3");
        assert!(["set_heater", "set_fan", "speed_factor", "extrude_factor"]
            .iter()
            .all(|id| command(id).tier == "T2"));
    }

    #[test]
    fn operator_text_and_tuning_are_bounded() {
        let ok = |id, t, v, a| render(command(id), &input(id, t, v, a)).unwrap().describe();
        let refused = |id, t, v, a| render(command(id), &input(id, t, v, a)).is_err();
        assert_eq!(ok("extrude", None, Some(5.0), None), "M83\nG1 E5.00 F180");
        assert_eq!(
            ok("extrude", None, Some(-50.0), None),
            "M83\nG1 E-50.00 F300"
        );
        assert!(refused("extrude", None, Some(0.0), None));
        assert!(refused("extrude", None, Some(201.0), None));
        assert_eq!(
            ok("z_offset", None, Some(-0.025), None),
            "SET_GCODE_OFFSET Z_ADJUST=-0.025 MOVE=1"
        );
        assert!(refused("z_offset", None, Some(1.5), None));
        assert_eq!(
            ok("limit", Some("velocity"), Some(300.0), None),
            "SET_VELOCITY_LIMIT VELOCITY=300"
        );
        assert_eq!(
            ok("limit", Some("accel"), Some(4000.0), None),
            "SET_VELOCITY_LIMIT ACCEL=4000"
        );
        assert_eq!(
            ok("limit", Some("scv"), Some(5.0), None),
            "SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY=5.0"
        );
        assert!(refused("limit", Some("jerk"), Some(5.0), None));
        assert!(refused("limit", Some("accel"), Some(99.0), None));
        assert_eq!(
            ok("pressure_advance", None, Some(0.0425), None),
            "SET_PRESSURE_ADVANCE ADVANCE=0.0425"
        );
        assert!(refused("pressure_advance", None, Some(1.5), None));

        let m = |t, a| render(command("macro"), &typed("macro", Some(t), a)).map(|s| s.describe());
        assert_eq!(m("PRINT_START", None).unwrap(), "PRINT_START");
        assert_eq!(
            m("PURGE_LINE", Some("EXTRUDE=5")).unwrap(),
            "PURGE_LINE EXTRUDE=5"
        );
        assert!(m("rm -rf /", None).is_err());
        assert!(m("PRINT_START", Some("A=1\nB=2")).is_err());

        let c = |line| {
            render(command("console"), &typed("console", None, Some(line))).map(|s| s.describe())
        };
        assert_eq!(c("QUERY_ENDSTOPS").unwrap(), "QUERY_ENDSTOPS");
        assert_eq!(c("  M115  ").unwrap(), "M115");
        assert!(c("").is_err());
        assert!(c("M115\nM112").is_err());
        assert!(c(&"X".repeat(257)).is_err());
        assert!(matches!(command("console").params, Params::Console));
    }

    // ---- L3 print control -------------------------------------------------------------------

    #[test]
    fn print_control_renders_exact_moonraker_calls_and_refuses_bad_paths() {
        let start = |f: &str| {
            render(
                command("print_start"),
                &CommandInput {
                    filename: Some(f.into()),
                    ..blank("print_start")
                },
            )
        };
        assert_eq!(
            start("Voron_Design_Cube_v7.gcode").unwrap(),
            Sent::Api {
                method: "POST",
                path: "/printer/print/start?filename=Voron_Design_Cube_v7.gcode".into(),
                body: None
            }
        );
        // Spaces and subdirectories are percent-encoded exactly once.
        assert_eq!(
            start("parts/tray r8.gcode").unwrap(),
            Sent::Api {
                method: "POST",
                path: "/printer/print/start?filename=parts%2Ftray%20r8.gcode".into(),
                body: None
            }
        );
        assert_eq!(
            start("a.gcode").unwrap().describe(),
            "POST /printer/print/start?filename=a.gcode (no G-code)"
        );
        for bad in [
            "",
            "../../etc/passwd.gcode",
            "/absolute.gcode",
            "notgcode.txt",
            "has?query.gcode",
            "has\nnewline.gcode",
            "quote\".gcode",
            "back\\slash.gcode",
        ] {
            assert!(start(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(start(&format!("{}.gcode", "x".repeat(250))).is_err());

        assert_eq!(
            render(
                command("print_queue_add"),
                &CommandInput {
                    filename: Some("b.gcode".into()),
                    ..blank("print_queue_add")
                }
            )
            .unwrap(),
            Sent::Api {
                method: "POST",
                path: "/server/job_queue/job".into(),
                body: Some(json!({"filenames":["b.gcode"]}))
            }
        );

        let remove = |id: &str| {
            render(
                command("print_queue_remove"),
                &CommandInput {
                    job_id: Some(id.into()),
                    ..blank("print_queue_remove")
                },
            )
        };
        assert_eq!(
            remove("0000000066D99C70").unwrap(),
            Sent::Api {
                method: "DELETE",
                path: "/server/job_queue/job?job_ids=0000000066D99C70".into(),
                body: None
            }
        );
        for bad in ["", "id with space", "id;rm", "../x", &"a".repeat(65)] {
            assert!(remove(bad).is_err(), "{bad:?} should be refused");
        }

        let exclude = |name: &str| {
            render(
                command("exclude_object"),
                &CommandInput {
                    name: Some(name.into()),
                    ..blank("exclude_object")
                },
            )
            .map(|s| s.describe())
        };
        assert_eq!(
            exclude("Cube_v7_id_0").unwrap(),
            "EXCLUDE_OBJECT NAME=Cube_v7_id_0"
        );
        // Slicer names carry spaces and dots; those are legal, a newline never is.
        assert_eq!(
            exclude("tray r8.stl id 2").unwrap(),
            "EXCLUDE_OBJECT NAME=tray r8.stl id 2"
        );
        for bad in [
            "",
            "   ",
            "a\nEMERGENCY",
            "a\rb",
            "say\"hi\"",
            &"x".repeat(129),
        ] {
            assert!(exclude(bad).is_err(), "{bad:?} should be refused");
        }
        // Tier matters: exclude_object must be T2 or the printing gate would block it.
        assert_eq!(command("exclude_object").tier, "T2");
        assert_eq!(command("print_start").tier, "T3");
        assert_eq!(command("print_queue_add").tier, "T2");
        assert_eq!(command("print_queue_remove").tier, "T2");
    }

    // ---- L4 restart, reconnect, macro params, filament --------------------------------------

    #[test]
    fn filament_and_extrude_speed_are_exact_and_bounded() {
        let filament = |id: &str, speed: Option<f64>| {
            render(command(id), &CommandInput { speed, ..blank(id) }).map(|s| s.describe())
        };
        // Default 5 mm/s -> F300, the feed Stella's own LOAD_FILAMENT macro uses.
        assert_eq!(
            filament("load_filament", None).unwrap(),
            "M83\nG1 E50.00 F300"
        );
        assert_eq!(
            filament("unload_filament", None).unwrap(),
            "M83\nG1 E-100.00 F300"
        );
        assert_eq!(
            filament("load_filament", Some(10.0)).unwrap(),
            "M83\nG1 E50.00 F600"
        );
        assert_eq!(
            filament("unload_filament", Some(1.0)).unwrap(),
            "M83\nG1 E-100.00 F60"
        );
        assert_eq!(
            filament("load_filament", Some(25.0)).unwrap(),
            "M83\nG1 E50.00 F1500"
        );
        for bad in [0.9, 25.1, 0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(
                filament("load_filament", Some(bad)).is_err(),
                "{bad} mm/s should be refused"
            );
            assert!(filament("unload_filament", Some(bad)).is_err());
        }

        // extrude keeps its old amount-derived feeds when no speed is given, and honours one
        // when it is. Both bounds still apply.
        let extrude = |mm: f64, speed: Option<f64>| {
            render(
                command("extrude"),
                &CommandInput {
                    value: Some(mm),
                    speed,
                    ..blank("extrude")
                },
            )
            .map(|s| s.describe())
        };
        assert_eq!(extrude(5.0, None).unwrap(), "M83\nG1 E5.00 F180");
        assert_eq!(extrude(25.0, None).unwrap(), "M83\nG1 E25.00 F300");
        assert_eq!(extrude(5.0, Some(2.0)).unwrap(), "M83\nG1 E5.00 F120");
        assert_eq!(extrude(-50.0, Some(20.0)).unwrap(), "M83\nG1 E-50.00 F1200");
        assert!(extrude(0.0, Some(5.0)).is_err());
        assert!(extrude(201.0, Some(5.0)).is_err());
        assert!(extrude(5.0, Some(25.1)).is_err());
        assert!(extrude(5.0, Some(0.5)).is_err());
    }

    #[test]
    fn macro_params_render_sorted_and_refuse_injection() {
        let call = |name: &str, params: &[(&str, &str)]| {
            render(
                command("macro"),
                &CommandInput {
                    target: Some(name.into()),
                    params: Some(
                        params
                            .iter()
                            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                            .collect(),
                    ),
                    ..blank("macro")
                },
            )
            .map(|s| s.describe())
        };
        assert_eq!(call("PRINT_START", &[]).unwrap(), "PRINT_START");
        assert_eq!(
            call("LOAD_FILAMENT", &[("TEMP", "240")]).unwrap(),
            "LOAD_FILAMENT TEMP=240"
        );
        // BTreeMap ordering makes the rendered line deterministic whatever order they arrive in.
        assert_eq!(
            call(
                "PRINT_START",
                &[("TOOL", "0"), ("BED", "60"), ("EXTRUDER", "215")]
            )
            .unwrap(),
            "PRINT_START BED=60 EXTRUDER=215 TOOL=0"
        );
        assert_eq!(
            call("MESH", &[("PROFILE", "kamp-1.2")]).unwrap(),
            "MESH PROFILE=kamp-1.2"
        );
        for bad_value in ["", "1 2", "1\nM112", "a;b", "{x}", "\"q\"", &"v".repeat(65)] {
            assert!(
                call("PRINT_START", &[("A", bad_value)]).is_err(),
                "value {bad_value:?} should be refused"
            );
        }
        for bad_key in ["", "lower", "has space", "A-B", &"K".repeat(33)] {
            assert!(
                call("PRINT_START", &[(bad_key, "1")]).is_err(),
                "key {bad_key:?} should be refused"
            );
        }
        // params and script together is ambiguous, so it is a refusal rather than a precedence.
        assert!(render(
            command("macro"),
            &CommandInput {
                target: Some("PRINT_START".into()),
                script: Some("A=1".into()),
                params: Some(BTreeMap::from([("B".to_owned(), "2".to_owned())])),
                ..blank("macro")
            }
        )
        .is_err());
        let many: Vec<(String, String)> =
            (0..13).map(|i| (format!("K{i}"), i.to_string())).collect();
        assert!(render(
            command("macro"),
            &CommandInput {
                target: Some("PRINT_START".into()),
                params: Some(many.into_iter().collect()),
                ..blank("macro")
            }
        )
        .is_err());
    }

    #[test]
    fn restart_and_reconnect_are_exact() {
        assert_eq!(
            render(command("klipper_restart"), &blank("klipper_restart"))
                .unwrap()
                .describe(),
            "RESTART"
        );
        assert_eq!(command("klipper_restart").tier, "T3");
        assert_eq!(command("klipper_restart").capability, Some("RESTART"));
        // RESTART is not FIRMWARE_RESTART: they are separate rows with separate scripts.
        assert_eq!(
            render(command("firmware_restart"), &blank("firmware_restart"))
                .unwrap()
                .describe(),
            "FIRMWARE_RESTART"
        );

        let reconnect = render(command("reconnect"), &blank("reconnect")).unwrap();
        assert_eq!(
            reconnect,
            Sent::Local("reconnect the Moonraker websocket source")
        );
        assert_eq!(reconnect.script(), None);
        assert!(reconnect.describe().contains("nothing is sent"));
        assert_eq!(command("reconnect").tier, "T1");
    }

    #[test]
    fn macro_parameters_are_read_out_of_the_macro_body() {
        // Stella's real `pause` macro body, trimmed to the parameter line.
        let pause = "{% set restore = False if printer.toolhead.extruder == ''\n  else True if params.RESTORE|default(1)|int == 1 else False %}";
        assert_eq!(
            macro_params(pause),
            vec![json!({"name":"RESTORE","default":"1"})]
        );
        // Stella's real `load_filament` body.
        assert_eq!(
            macro_params("{% set temp = params.TEMP|default(220)|int %}\nG1 E50 F300"),
            vec![json!({"name":"TEMP","default":"220"})]
        );
        // No default, spaces around the pipe, quoted default, nested parentheses, duplicates.
        assert_eq!(
            macro_params("params.BARE and params.SPACED | default(7) and params.QUOTED|default('abc') and params.NESTED|default({'a':1}) and params.BARE"),
            vec![
                json!({"name":"BARE","default":null}),
                json!({"name":"SPACED","default":"7"}),
                json!({"name":"QUOTED","default":"abc"}),
                json!({"name":"NESTED","default":"{'a':1}"}),
            ]
        );
        assert!(macro_params("no parameters here at all").is_empty());
        assert!(macro_params("").is_empty());
        // A body full of parameters is capped, not unbounded.
        let huge: String = (0..40).map(|i| format!("params.P{i} ")).collect();
        assert_eq!(macro_params(&huge).len(), MACRO_PARAMS_MAX);
    }

    // WO-02: the dry-run copy's name shows "dry run" on the printer's screen and in the history.
    #[test]
    fn dry_run_name_shows_dry_run() {
        assert_eq!(super::dry_run_name("cube.gcode"), "cube (dry run).gcode");
        assert_eq!(
            super::dry_run_name("plate one.bgcode"),
            "plate_one (dry run).bgcode"
        );
        assert_eq!(
            super::dry_run_name("weird__x.gcode"),
            "weird__x (dry run).gcode"
        );
        assert_eq!(super::dry_run_name("noextension"), "noextension (dry run)");
    }

    // Finish-2 F07 (2026-10-19): the copy keeps the file's own name. It was shown lowercased.
    #[test]
    fn the_dry_run_copy_keeps_the_files_own_name() {
        assert_eq!(
            super::dry_run_name("Voron_Cube_v7_PLA_Stella_fast.gcode"),
            "Voron_Cube_v7_PLA_Stella_fast (dry run).gcode"
        );
        assert_eq!(
            super::dry_run_name("Plate One.BGCODE"),
            "Plate_One (dry run).bgcode",
            "the extension is normalised, the name is not"
        );
        assert_eq!(super::dry_run_name("Ünï.gcode"), "_n_ (dry run).gcode", "a name that is not plain ASCII is still made safe");
        // and the original is found again from the copy
        assert_eq!(
            super::dry_run_original("Voron_Cube_v7 (dry run).gcode").as_deref(),
            Some("Voron_Cube_v7.gcode")
        );
        assert_eq!(super::dry_run_original("sub/cube (dry run).bgcode").as_deref(), Some("sub/cube.bgcode"));
        assert_eq!(super::dry_run_original("cube.gcode"), None);
        assert_eq!(super::dry_run_original("a (dry run)").as_deref(), Some("a"));
    }

    // WO-02: dry run is a T3, start_print-gated command — refused while printing, refused on a
    // printer that cannot start a file.
    #[test]
    fn dry_run_is_t3_and_start_print_gated() {
        let all = super::Capabilities::klipper();
        assert_eq!(all.allows("dry_run", "Klipper"), Ok(()));
        let none = super::Capabilities::none();
        assert!(none.allows("dry_run", "X").is_err());
        // T3 requires confirm.
        let command = *super::COMMANDS.iter().find(|c| c.id == "dry_run").unwrap();
        assert_eq!(command.tier, "T3");
    }
}

#[cfg(test)]
mod timeout_tests {
    /// Finish-2 F05: with no print running, a busy Klipper (idle state "Printing") is said to be busy, not printing.
    #[test]
    fn a_busy_printer_with_no_print_says_busy() {
        let busy = super::FreshState { help: Default::default(), homed_axes: "xyz".into(), print_state: "cancelled".into(), idle_state: "Printing".into() };
        let console = *super::COMMANDS.iter().find(|c| c.id == "console").unwrap();
        assert!(busy.refusal(console).unwrap().contains("still busy with the last command"));
        let printing = super::FreshState { print_state: "printing".into(), ..busy };
        assert!(printing.refusal(console).unwrap().contains("blocked while printing"));
    }
    /// Finish-2 F05: a full bed mesh on Stella takes about eight minutes; the long procedures may run half an hour.
    #[test]
    fn long_klipper_procedures_have_half_an_hour_and_the_rest_keep_the_normal_limit() {
        for id in ["mesh", "qgl", "probe_accuracy", "macro", "home"] {
            assert_eq!(super::command_timeout_ms(id, 180_000), 1_800_000, "{id}");
        }
        assert_eq!(super::command_timeout_ms("pause", 180_000), 180_000);
        assert_eq!(super::command_timeout_ms("mesh", 3_600_000), 3_600_000, "a longer configured limit stays");
    }
}
