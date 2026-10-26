//! A simulated printer (finish board WO-03, widened by polish board WO-02): a driver that runs on this
//! machine with no network, so someone without a printer can operate one and play with Stella Scope.
//! A demo printer is any machine in the model catalogue (`cad_model`): its bed, Z travel and what moves
//! come from the catalogue and from the machine's registered motion, and it plays a real sliced sample
//! print that fits its bed (demo/README.md).
//!
//! The demo heats with a warm-up / cool-down curve, homes, jogs, moves fans, speed and flow, pauses,
//! resumes, cancels, takes uploads and starts prints. A started print plays its file in time: every
//! move takes its length over its feed rate (scaled to the slicer's own estimate), the head is where
//! the file puts it, and temperatures, the fan and the active tool follow the file's own commands. The
//! service reports all of it exactly like a Klipper printer, so the WO-01 follower draws it. Nothing
//! leaves the machine and nothing is printed. Every view badges a demo printer DEMO.
//!
//! The collector and every command open their own driver, so one printer's state lives in a shared
//! registry keyed by its recording path: a command sent from the page changes what the page sees.

use crate::{
    config::{Config, PrinterEntry},
    driver::*,
    printer_models, Result,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

/// Ambient temperature, °C.
const AMBIENT: f64 = 22.0;
/// The largest file a demo printer takes, the same limit the G-code proxy uses.
const MAX_FILE: usize = 64 * 1024 * 1024;
/// How long a finished demo print sits on the plate before the demo prints it again, seconds.
const REPEAT_AFTER_S: f64 = 90.0;

static VASE: &[u8] = include_bytes!("../demo/stella-twist-vase.gcode.gz");
static BOWL: &[u8] = include_bytes!("../demo/core-one-l-twist-bowl.gcode.gz");
static SMALL: &[u8] = include_bytes!("../demo/small-twist-vase.gcode.gz");
static DUO: &[u8] = include_bytes!("../demo/duo-twist.gcode.gz");

// ------------------------------------------------------------------------------------------------
// The machine: what a catalogue entry says a demo printer is.
// ------------------------------------------------------------------------------------------------

/// What moves when the nozzle goes up or forward, read from the machine's registered motion
/// (`web/printer-motion*.json`): the X beam on its Z rails, the bed on its Z screws, or the bed in Y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Frame {
    /// CoreXY with a gantry that climbs (Voron 2.4, Micron, SV08): the beam moves in Z.
    GantryZ,
    /// CoreXY with a bed that drops (Trident, V0, Satsuma): the bed moves in Z.
    BedZ,
    /// A bed slinger (Ender-3): the bed moves in Y, the beam in Z.
    BedY,
    /// No motion is registered for this model, so nothing is claimed about what moves.
    Unregistered,
}

impl Frame {
    /// Where the nozzle is after G28: the centre of the bed (Voron and Prusa homing macros park there),
    /// or the front left corner for a slinger, whose bed homes against its own endstop.
    fn home(self, bed: [f64; 2]) -> [f64; 3] {
        match self {
            Frame::BedY => [0.0, 0.0, 10.0],
            _ => [bed[0] / 2.0, bed[1] / 2.0, 10.0],
        }
    }

    /// The kind a machine's registered motion describes: a `bed` group that really holds parts and moves
    /// in Y is a slinger, one that moves in Z is a bed that drops, anything else with motion is a gantry
    /// that climbs. A machine with no registered motion is `Unregistered`.
    pub fn of_profile(profile: Option<&Value>) -> Frame {
        let Some(p) = profile else {
            return Frame::Unregistered;
        };
        let named = p["groups"].as_object().is_some_and(|g| !g.is_empty());
        let real = |name: &str| {
            p["motions"].get(name).is_some()
                && (!named || p["groups"][name].as_array().is_some_and(|a| !a.is_empty()))
        };
        if real("bed") {
            let moves_y = p["motions"]["bed"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c == "y" || c.get("y").is_some()));
            return if moves_y { Frame::BedY } else { Frame::BedZ };
        }
        Frame::GantryZ
    }
}

/// How many tools a printer has and where an idle one waits. `Changer` is a machine with a tool bank chosen for it
/// (polish WO-01 draws the banks, WO-03 plays one in the demo farm): each tool waits at the dock the bank puts it at.
#[derive(Clone, Debug, PartialEq)]
pub enum Tools {
    One,
    /// Two carriages on one beam: the idle one parks at its end of the beam (X 0 and the far end).
    Idex,
    /// A toolchanger: every tool has a dock (X, Y, Z in machine coordinates).
    Changer(Vec<[f64; 3]>),
}

impl Tools {
    pub fn count(&self) -> usize {
        match self {
            Tools::One => 1,
            Tools::Idex => 2,
            Tools::Changer(d) => d.len().max(1),
        }
    }
}

/// A demo printer's machine: the numbers the driver reports and the motion the model follows.
#[derive(Clone, Debug)]
pub struct Spec {
    pub id: String,
    pub brand: &'static str,
    pub model: String,
    pub bed: [f64; 2],
    pub max_z: f64,
    pub frame: Frame,
    pub tools: Tools,
    nozzle_tau: f64,
    bed_tau: f64,
    max_velocity: f64,
}

/// The maker's name for a model family of the catalogue. Unknown families give the first word of the
/// label, kept once for the life of the process (the farm summary opens a driver every few seconds, so a
/// name must not be allocated afresh each time).
fn brand_of(family: &str, label: &str) -> &'static str {
    match family {
        "v24" | "trident" | "v0" | "micron" => "Voron",
        "sovol_sv08" => "Sovol",
        "ender3" => "Creality",
        "prusa-core-one" | "prusa-i3" | "prusa-mini" => "Prusa",
        "doomcube" => "DoomCube",
        "vcore3" | "vcore4" | "vminion" => "Rat Rig",
        _ => {
            static NAMES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
            let first = label.split([' ', '·']).next().unwrap_or("Demo");
            let mut all = NAMES
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *all.entry(first.to_owned())
                .or_insert_with(|| Box::leak(first.to_owned().into_boxed_str()))
        }
    }
}

impl Spec {
    /// The machine a configuration names: its `cad_model`, or (for the first two demos, which only had a
    /// model name) a CORE One L when the name says CORE and a Voron 2.4 350 otherwise.
    fn of(cfg: &Config) -> Spec {
        let name = cfg.printer_model.as_deref().unwrap_or("");
        let id = cfg
            .cad_model
            .as_deref()
            .filter(|m| printer_models::machine(m).is_some())
            .unwrap_or(if name.to_ascii_uppercase().contains("CORE") {
                "prusa_core_one_l"
            } else {
                "voron_v24_350_printed"
            });
        let mut spec = Spec::catalogue(id);
        if !name.is_empty() {
            spec.model = name.to_owned();
        }
        // A tool bank chosen for the machine gives it that many tools, each waiting at the dock the bank puts it at
        // (the head's printer coordinates there): the data is the catalogue's, so the demo plays with the bank's files
        // not installed too.
        if let Some(tools) = cfg
            .cad_options
            .as_ref()
            .and_then(|sel| printer_models::bank_tools(id, sel))
        {
            spec.tools = Tools::Changer(tools.iter().map(|t| t.dock).collect());
        }
        spec
    }

    /// A catalogue machine as a demo printer. `id` must be in the catalogue (`printer_models::machine`).
    pub fn catalogue(id: &str) -> Spec {
        let row = printer_models::machine(id);
        let profile = printer_models::profile(id);
        let label = row.and_then(|r| r["label"].as_str()).unwrap_or(id);
        let short = label
            .split(['·', '/'])
            .next()
            .unwrap_or(label)
            .trim()
            .to_owned();
        let family = row.and_then(|r| r["family"].as_str()).unwrap_or("");
        let size = row.and_then(|r| r["size_mm"].as_f64());
        // A Prusa machine has no single size in the catalogue (the CORE One L, the CORE One, the MK3S+ and the MINI come from their makers' own files, finish-2 F09):
        // its plate and height are the published ones in the table of every Prusa machine.
        let prusa = crate::prusa_machines::by_id(id).filter(|m| m.bed.is_some() && m.height.is_some());
        let (bed, mut max_z) = match (prusa, size) {
            (Some(m), _) => (m.bed.unwrap_or([250.0, 250.0]), m.height.unwrap_or(250.0)),
            (None, Some(s)) => ([s, s], s),
            _ => ([250.0, 250.0], 250.0),
        };
        // Z travel is what the machine's registered motion allows when there is one.
        if let Some(z) =
            profile.and_then(|p| Some(p["axes"]["z"][1].as_f64()? - p["axes"]["z"][0].as_f64()?))
        {
            if prusa.is_none() {
                max_z = z.round();
            }
        }
        let core = id == "prusa_core_one_l";
        Spec {
            id: id.to_owned(),
            brand: brand_of(family, label),
            model: prusa.map_or(short, |m| m.short.to_owned()),
            bed,
            max_z,
            frame: Frame::of_profile(profile),
            tools: if id.ends_with("_idex") {
                Tools::Idex
            } else {
                Tools::One
            },
            nozzle_tau: if core { 10.0 } else { 12.0 },
            // A bigger bed takes longer to warm: 45 s for a 350 mm bed, as the first demo always had.
            bed_tau: if core {
                60.0
            } else {
                10.0 + 35.0 * (bed[0] * bed[1]) / (350.0 * 350.0)
            },
            max_velocity: if core {
                400.0
            } else if family == "ender3" || family == "prusa-i3" || family == "prusa-mini" {
                // a bed slinger's bed is the heavy thing that moves: the Ender-3's and the Prusa i3's and MINI's top speeds are about 200 mm/s
                200.0
            } else {
                500.0
            },
        }
    }

    /// The sample that fits this machine: two colours for a printer with tools, the Prusa bowl for a
    /// Prusa, the tall vase when the bed and Z travel are generous, the small vase otherwise.
    fn sample_kind(&self) -> Kind {
        if self.tools.count() > 1 {
            Kind::Duo
        } else if self.id.starts_with("prusa_core_one") {
            // the bowl was sliced for a CORE One L; the Prusa bed slingers print the twist vase like any other 250 mm machine
            Kind::Bowl
        } else if self.bed[0].min(self.bed[1]) >= 200.0 && self.max_z >= 150.0 {
            Kind::Vase
        } else {
            Kind::Small
        }
    }

    /// Where tool `tool` waits when it is not the active one; `head` is where the active tool is.
    pub fn park(&self, tool: usize, head: [f64; 3]) -> [f64; 3] {
        match &self.tools {
            Tools::Changer(docks) => docks.get(tool).copied().unwrap_or(head),
            Tools::Idex => [if tool == 0 { 0.0 } else { self.bed[0] }, head[1], head[2]],
            Tools::One => head,
        }
    }
}

/// The farm one press adds (polish WO-02): id, name, catalogue machine, model shown, how far into its
/// sample print it starts (0 heating, 1 finished) and whether it starts paused. Every model is GPL except the CORE One
/// L's, and the toolchanger's tool files are open (GPL and CC BY-SA), so no non-commercial licence is asked for. The
/// DoomCube 2 was here until polish WO-03: its model has no part records and upstream ships it static, so it never moved.
/// Finish-2 F03 (2026-10-19): all eight start printing, at staggered places, so every machine moves the moment it is pressed
/// (the CORE One L heating, the V0.2 finished and the SV08 paused stood still). Paused, finished and heating are one press
/// away on any of them.
const DEMO_FARM: [(&str, &str, &str, &str, f64, bool); 9] = [
    (
        "demo-voron-24",
        "Voron 2.4 350",
        "voron_v24_350_printed",
        "Voron 2.4 350",
        0.62,
        false,
    ),
    (
        "demo-core-one-l",
        "CORE One L",
        "prusa_core_one_l",
        "CORE One L",
        0.3,
        false,
    ),
    (
        "demo-trident",
        "Trident 300",
        "voron_trident_300",
        "Trident 300",
        0.18,
        false,
    ),
    (
        "demo-v0",
        "Voron V0.2",
        "voron_v02_120",
        "V0.2 120",
        0.25,
        false,
    ),
    (
        "demo-micron",
        "Micron+ 180",
        "micron_plus_r1_180",
        "Micron+ 180",
        0.85,
        false,
    ),
    (
        "demo-sv08",
        "Sovol SV08",
        "sovol_sv08_350",
        "SV08 350",
        0.45,
        false,
    ),
    // A toolchanger (polish WO-03, Opus supplement S1): a Voron 2.4 300 with a StealthChanger bank of three docks, playing
    // the two-colour sample; the tool on the gantry changes at T0 and T1 and the third waits at its dock.
    (
        "demo-toolchanger",
        "Toolchanger",
        "voron_v24_300_printed",
        "V2.4 Changer",
        0.4,
        false,
    ),
    (
        "demo-ender-3",
        "Ender-3",
        "ender3_stock_220",
        "Ender-3",
        0.5,
        false,
    ),
    // Finish-2 F09 (2026-10-19): the Prusa i3 MK3S+, a bed slinger drawn from its community CAD that the build carries (the bed slides in Y, the X axis climbs);
    // it prints, like every other demo printer, from the moment of the press, at a place of its own.
    (
        "demo-mk3s",
        "Prusa MK3S+",
        "prusa_mk3s",
        "MK3S+",
        0.35,
        false,
    ),
];

/// The bank the toolchanger demo has: three docks of JabberWocky tools, the first on the gantry. The JabberWocky head is
/// CC BY-SA 4.0 and every other file of this selection (docks, shuttle, backplates, fasteners) is GPL-3.0, all open and
/// shippable with credit; the bank's other tool, Xol, is non-commercial and is left out so the demo farm never asks for a
/// non-commercial licence.
fn demo_bank() -> Value {
    json!([{"option":"stealthchanger_bank","tools":["jabberwocky_jabberwocky_drive_conch","jabberwocky_jabberwocky_drive_conch","jabberwocky_jabberwocky_drive_conch"],"active":0}])
}

const DEMO_ACCENTS: [&str; 6] = [
    "#04D9C4", "#FF6A00", "#FF2D92", "#18E36B", "#3D8BFF", "#B58CFF",
];

/// The printers "Try the demo farm" adds, as farm entries.
pub fn demo_farm() -> Vec<PrinterEntry> {
    DEMO_FARM
        .iter()
        .enumerate()
        .map(|(i, (id, name, cad, model, start, paused))| {
            let spec = Spec::catalogue(cad);
            PrinterEntry {
                id: (*id).into(),
                name: (*name).into(),
                driver: "demo".into(),
                model: Some((*model).into()),
                accent: Some(DEMO_ACCENTS[i % DEMO_ACCENTS.len()].into()),
                bed_size_mm: Some(spec.bed),
                // F06 (2026-10-19): every demo machine has the usual 0.4 mm nozzle, so a sliced file offered to the queue is matched to it without the
                // note "tell the app which nozzle it has", which a person trying the demo cannot act on.
                nozzle_mm: Some(0.4),
                cad_model: Some((*cad).into()),
                // F03 (2026-10-19): the Trident's base model has no gantry; its R2 gantry and X carriage (GPL-3.0, VoronDesign) are the option that gives
                // it a head that moves in X and Y.
                cad_options: match *id {
                    "demo-toolchanger" => Some(demo_bank()),
                    "demo-trident" => Some(json!([{"option":"trident_r2_gantry"}])),
                    _ => None,
                },
                demo_start_at: Some(*start),
                demo_paused: *paused,
                demo_since: Some(crate::model::now()),
                ..PrinterEntry::default()
            }
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------
// The sample files: sliced files, each centred where its slicer put it, moved to the bed's centre.
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Vase,
    Bowl,
    Small,
    Duo,
}

impl Kind {
    /// The name the printer shows, the compressed file, and the point the slicer centred it on.
    fn file(self) -> (&'static str, &'static [u8], [f64; 2]) {
        match self {
            Kind::Vase => ("3D3D twist vase.gcode", VASE, [175.0, 175.0]),
            Kind::Bowl => ("3D3D twist bowl.gcode", BOWL, [150.0, 150.0]),
            Kind::Small => ("3D3D small twist vase.gcode", SMALL, [60.0, 60.0]),
            Kind::Duo => ("3D3D duo twist.gcode", DUO, [125.0, 125.0]),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// The plan: a sliced file turned into timed moves.
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Move {
    /// Print-clock time at the end of this move, seconds.
    t: f64,
    /// Byte offset just past this move's line.
    byte: u32,
    /// Where the move ends, mm.
    x: f32,
    y: f32,
    z: f32,
    /// Layer number (1-based) this move belongs to; 0 before the first layer change.
    layer: u32,
    /// Head speed during the move, mm/s.
    speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Set {
    Nozzle(f64),
    Bed(f64),
    Fan(f64),
    /// `T0`, `T1`...: this tool is now the active one.
    Tool(usize),
}

/// A file played in time.
#[derive(Debug, Default)]
struct Plan {
    moves: Vec<Move>,
    /// Temperature, fan and tool commands in file order, with the print-clock time they run at.
    sets: Vec<(f64, Set)>,
    total_s: f64,
    layers: u32,
    /// The slicer's own estimate, when the file carries one.
    estimate_s: Option<f64>,
    first_nozzle: f64,
    first_bed: f64,
}

/// `;estimated printing time (normal mode) = 1h 46m 20s` (PrusaSlicer) or `;TIME:6380` (Cura-style).
fn parse_estimate(line: &str) -> Option<f64> {
    if let Some(rest) = line.strip_prefix(";TIME:") {
        return rest.trim().parse().ok();
    }
    let rest = line
        .strip_prefix("; estimated printing time (normal mode) =")
        .or_else(|| line.strip_prefix("; total estimated time:"))?;
    let mut s = 0.0;
    for part in rest.split_whitespace() {
        let (num, unit) = part.split_at(part.find(|c: char| c.is_ascii_alphabetic())?);
        let n: f64 = num.parse().ok()?;
        s += n * match unit {
            "d" => 86400.0,
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            _ => return None,
        };
    }
    Some(s)
}

/// The value after `letter` in a G-code line's words, e.g. `S` in `M104 S215`.
fn word(line: &str, letter: char) -> Option<f64> {
    line.split_ascii_whitespace()
        .skip(1)
        .find(|w| w.starts_with(letter))
        .and_then(|w| w[1..].parse().ok())
}

/// `PRINT_START BED=85 EXTRUDER=250` (Klipper macros carry the temperatures as parameters).
fn macro_param(line: &str, key: &str) -> Option<f64> {
    line.split_ascii_whitespace().find_map(|w| {
        let (k, v) = w.split_once('=')?;
        k.eq_ignore_ascii_case(key)
            .then(|| v.parse().ok())
            .flatten()
    })
}

/// `T0`, `T12`: a tool change (a macro named T... with letters is not one).
fn tool_change(head: &str) -> Option<usize> {
    head.strip_prefix('T')
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse().ok())
}

impl Plan {
    fn parse(bytes: &[u8]) -> Plan {
        let mut plan = Plan::default();
        let (mut abs, mut feed) = (true, 50.0f64); // mm/s; slicers set F before the first move
        let mut pos = [0.0f64; 3];
        let (mut t, mut layer) = (0.0f64, 0u32);
        let mut offset = 0usize;
        for raw in bytes.split(|&b| b == b'\n') {
            offset += raw.len() + 1;
            let Ok(line) = std::str::from_utf8(raw) else {
                continue;
            };
            let line = line.trim();
            if let Some(comment) = line.strip_prefix(';') {
                if comment.starts_with("LAYER_CHANGE") {
                    layer += 1;
                } else if plan.estimate_s.is_none() {
                    plan.estimate_s = parse_estimate(line);
                }
                continue;
            }
            let code = line.split(';').next().unwrap_or("").trim();
            let head = code.split_ascii_whitespace().next().unwrap_or("");
            let at = |plan: &mut Plan, set: Set| plan.sets.push((t, set));
            match head {
                "G0" | "G1" | "G2" | "G3" => {
                    if let Some(f) = word(code, 'F') {
                        feed = (f / 60.0).max(1.0);
                    }
                    let mut to = pos;
                    for (i, l) in ['X', 'Y', 'Z'].into_iter().enumerate() {
                        if let Some(v) = word(code, l) {
                            to[i] = if abs { v } else { pos[i] + v };
                        }
                    }
                    let d = ((to[0] - pos[0]).powi(2)
                        + (to[1] - pos[1]).powi(2)
                        + (to[2] - pos[2]).powi(2))
                    .sqrt();
                    // A move with no travel (a retract, a prime) still takes its filament length.
                    let len = if d > 0.0 {
                        d
                    } else {
                        word(code, 'E').map_or(0.0, f64::abs)
                    };
                    if len <= 0.0 {
                        continue;
                    }
                    t += len / feed;
                    pos = to;
                    plan.moves.push(Move {
                        t,
                        byte: offset.min(u32::MAX as usize) as u32,
                        x: pos[0] as f32,
                        y: pos[1] as f32,
                        z: pos[2] as f32,
                        layer,
                        speed: feed as f32,
                    });
                }
                "G90" => abs = true,
                "G91" => abs = false,
                "G28" => pos = [0.0, 0.0, 0.0],
                "G4" => {
                    t += word(code, 'P').map_or(0.0, |ms| ms / 1000.0)
                        + word(code, 'S').unwrap_or(0.0)
                }
                "M104" | "M109" => {
                    if let Some(s) = word(code, 'S') {
                        at(&mut plan, Set::Nozzle(s));
                    }
                }
                "M140" | "M190" => {
                    if let Some(s) = word(code, 'S') {
                        at(&mut plan, Set::Bed(s));
                    }
                }
                "M106" => at(
                    &mut plan,
                    Set::Fan(word(code, 'S').unwrap_or(255.0) / 255.0),
                ),
                "M107" => at(&mut plan, Set::Fan(0.0)),
                _ if tool_change(head).is_some() => {
                    at(&mut plan, Set::Tool(tool_change(head).unwrap_or(0)))
                }
                _ if head.eq_ignore_ascii_case("PRINT_START") => {
                    if let Some(b) = macro_param(code, "BED") {
                        at(&mut plan, Set::Bed(b));
                    }
                    if let Some(e) = macro_param(code, "EXTRUDER") {
                        at(&mut plan, Set::Nozzle(e));
                    }
                }
                _ => {}
            }
        }
        plan.layers = layer;
        // Moves at raw feed rates ignore acceleration, so they run fast; stretch the clock to the
        // slicer's own estimate when the file has one.
        if let Some(est) = plan.estimate_s.filter(|e| *e > 0.0 && t > 0.0) {
            let k = est / t;
            plan.moves.iter_mut().for_each(|m| {
                m.t *= k;
                m.speed /= k as f32;
            });
            plan.sets.iter_mut().for_each(|s| s.0 *= k);
            t = est;
        }
        plan.total_s = t;
        let first = |want: fn(&Set) -> Option<f64>| {
            plan.sets
                .iter()
                .find_map(|(_, s)| want(s).filter(|v| *v > 0.0))
        };
        plan.first_nozzle = first(|s| match s {
            Set::Nozzle(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(0.0);
        plan.first_bed = first(|s| match s {
            Set::Bed(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(0.0);
        plan
    }

    /// Index of the move running at print-clock `t`.
    fn index_at(&self, t: f64) -> usize {
        self.moves
            .partition_point(|m| m.t < t)
            .min(self.moves.len().saturating_sub(1))
    }

    /// The head position at print-clock `t`, between the previous move's end and this one's.
    fn position_at(&self, t: f64) -> Option<([f64; 3], &Move)> {
        let i = self.index_at(t);
        let m = self.moves.get(i)?;
        let (from, t0) = match i.checked_sub(1).and_then(|p| self.moves.get(p)) {
            Some(p) => ([p.x, p.y, p.z], p.t),
            None => ([m.x, m.y, m.z], 0.0),
        };
        let f = if m.t > t0 {
            ((t - t0) / (m.t - t0)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let lerp = |a: f32, b: f32| a as f64 + (b as f64 - a as f64) * f;
        Some((
            [lerp(from[0], m.x), lerp(from[1], m.y), lerp(from[2], m.z)],
            m,
        ))
    }
}

/// One file a demo printer holds: its name, bytes and plan.
struct DemoFile {
    name: String,
    bytes: Arc<Vec<u8>>,
    plan: Arc<Plan>,
    modified: f64,
}

fn unpack(gz: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // The samples are compiled in and checked by a test; a failure here is a broken build.
    let _ = crate::gzip::gunzip(gz, |chunk| {
        out.extend_from_slice(chunk);
        Ok(())
    });
    out
}

type Loaded = (String, Arc<Vec<u8>>, Arc<Plan>);

/// A sample file as sliced, unpacked and planned once per process, only when a demo printer needs it.
fn base(kind: Kind) -> &'static Loaded {
    static CELLS: [OnceLock<Loaded>; 4] = [const { OnceLock::new() }; 4];
    CELLS[kind as usize].get_or_init(|| {
        let (name, gz, _) = kind.file();
        let bytes = unpack(gz);
        let plan = Plan::parse(&bytes);
        (name.to_owned(), Arc::new(bytes), Arc::new(plan))
    })
}

/// The first two samples, as the tests of the first demo name them.
#[cfg(test)]
fn sample(core: bool) -> &'static Loaded {
    base(if core { Kind::Bowl } else { Kind::Vase })
}

/// `v + d` as G-code writes a coordinate: three decimals at most, no trailing zeros.
fn coordinate(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The G-code of a sample with every X and Y of its moves moved by (dx, dy), so the print sits at the
/// centre of a bed the slicer did not know. Only the copy a printer holds is moved; the shipped file is
/// never edited. Arc centres (I, J) are relative and stay as they are.
fn shifted(bytes: &[u8], dx: f64, dy: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    for (i, raw) in bytes.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        let moves = matches!(
            raw.split(|&b| b == b' ').next(),
            Some(b"G0" | b"G1" | b"G2" | b"G3")
        );
        let Ok(line) = std::str::from_utf8(raw) else {
            out.extend_from_slice(raw);
            continue;
        };
        if !moves {
            out.extend_from_slice(raw);
            continue;
        }
        let (code, comment) = line.split_at(line.find(';').unwrap_or(line.len()));
        let words: Vec<String> = code
            .split(' ')
            .map(|w| {
                let v = w.get(1..).and_then(|n| n.parse::<f64>().ok());
                match (w.as_bytes().first(), v) {
                    (Some(b'X'), Some(v)) => format!("X{}", coordinate(v + dx)),
                    (Some(b'Y'), Some(v)) => format!("Y{}", coordinate(v + dy)),
                    _ => w.to_owned(),
                }
            })
            .collect();
        out.extend_from_slice(words.join(" ").as_bytes());
        out.extend_from_slice(comment.as_bytes());
    }
    out
}

/// The sample for a bed: the file as sliced when the bed's centre is where the slicer centred it, else a
/// copy moved to the bed's centre (kept once per distance, shared by every printer that needs it).
fn for_bed(kind: Kind, bed: [f64; 2]) -> Loaded {
    let b = base(kind);
    let c = kind.file().2;
    let (dx, dy) = (bed[0] / 2.0 - c[0], bed[1] / 2.0 - c[1]);
    if dx.abs() < 0.05 && dy.abs() < 0.05 {
        return (b.0.clone(), b.1.clone(), b.2.clone());
    }
    static MOVED: OnceLock<Mutex<HashMap<(usize, i64, i64), Loaded>>> = OnceLock::new();
    let key = (
        kind as usize,
        (dx * 10.0).round() as i64,
        (dy * 10.0).round() as i64,
    );
    let mut all = MOVED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let l = all.entry(key).or_insert_with(|| {
        let bytes = shifted(&b.1, dx, dy);
        let plan = Plan::parse(&bytes);
        (b.0.clone(), Arc::new(bytes), Arc::new(plan))
    });
    (l.0.clone(), l.1.clone(), l.2.clone())
}

/// The PNG pictures a sliced file carries as `; thumbnail begin WxH N` comment blocks, the largest first.
fn thumbnails(bytes: &[u8]) -> Vec<(usize, Vec<u8>)> {
    // They sit at the top of the file; the rest is moves.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512 * 1024)]);
    let (mut out, mut lines) = (Vec::new(), head.lines());
    while let Some(l) = lines.next() {
        let Some(rest) = l.strip_prefix("; thumbnail begin ") else {
            continue;
        };
        let size = rest.split_whitespace().next().unwrap_or("");
        let area = size
            .split_once('x')
            .and_then(|(w, h)| Some(w.parse::<usize>().ok()? * h.parse::<usize>().ok()?))
            .unwrap_or(0);
        let text: String = lines
            .by_ref()
            .take_while(|l| !l.starts_with("; thumbnail end"))
            .map(|l| l.trim_start_matches("; ").trim())
            .collect();
        if let Some(png) = base64(&text).filter(|p| p.starts_with(b"\x89PNG")) {
            out.push((area, png));
        }
    }
    out.sort_by_key(|(area, _)| std::cmp::Reverse(*area));
    out
}

/// Standard base64 (the alphabet G-code thumbnails use), padding optional.
fn base64(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(s.len() * 3 / 4), 0u32, 0);
    for c in s.bytes().take_while(|&c| c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ------------------------------------------------------------------------------------------------
// The printer.
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Job {
    Idle,
    /// Started, waiting for the heaters to reach the file's first temperatures.
    Heating,
    Printing,
    Paused,
    Complete,
    Cancelled,
    Error,
}

impl Job {
    fn word(self) -> &'static str {
        match self {
            Job::Idle => "standby",
            Job::Heating | Job::Printing => "printing",
            Job::Paused => "paused",
            Job::Complete => "complete",
            Job::Cancelled => "cancelled",
            Job::Error => "error",
        }
    }
    fn active(self) -> bool {
        matches!(self, Job::Heating | Job::Printing | Job::Paused)
    }
}

struct State {
    job: Job,
    files: Vec<DemoFile>,
    /// Index into `files` of the file loaded (printing, or the last one printed).
    current: Option<usize>,
    /// Print-clock seconds into the current file.
    clock: f64,
    /// Next entry of the plan's `sets` to apply.
    next_set: usize,
    /// Wall-clock time the job started and finished (sample timestamps).
    started_at: f64,
    finished_at: f64,
    /// Print the sample again after it finishes; off once someone cancels or starts their own file.
    repeat: bool,
    pos: [f64; 3],
    homed: bool,
    nozzle: (f64, f64),
    bed: (f64, f64),
    fan: f64,
    speed: f64,
    flow: f64,
    /// The active tool (`T0`, `T1`...), changed by the file's own tool changes.
    tool: usize,
    last_ts: Option<f64>,
    /// Where to start the sample on the first sample (0..1; 1 is finished), from the configuration.
    start_at: Option<f64>,
    /// Start part way but paused.
    start_paused: bool,
}

/// Where a demo that joins part way is in its sample print: `at` when it was added (0 to 1), moved on by the time since
/// then and wrapped round as the sample repeats. A paused demo, or one added before this was kept, is where it was put.
fn joined_at(at: f64, paused: bool, since: Option<f64>, now: f64, total_s: f64) -> f64 {
    match since {
        Some(t) if !paused && total_s > 0.0 && at < 1.0 => (at + (now - t).max(0.0) / total_s).rem_euclid(1.0),
        _ => at,
    }
}

struct Machine {
    spec: Spec,
    brand: &'static str,
    model: String,
    bed: [f64; 2],
    max_z: f64,
    nozzle_tau: f64,
    bed_tau: f64,
    state: Mutex<State>,
}

/// One printer's machine, shared by the collector and every command (see the module comment).
fn machine(cfg: &Config) -> Arc<Machine> {
    static ALL: OnceLock<Mutex<HashMap<String, Arc<Machine>>>> = OnceLock::new();
    let spec = Spec::of(cfg);
    let key = format!("{}|{}|{}", cfg.database, cfg.printer_id, spec.id);
    let mut all = ALL
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    all.entry(key)
        .or_insert_with(|| Arc::new(Machine::new(spec, cfg.demo_start_at, cfg.demo_paused, cfg.demo_since)))
        .clone()
}

impl Machine {
    fn new(spec: Spec, start_at: Option<f64>, paused: bool, since: Option<f64>) -> Machine {
        let (name, bytes, plan) = for_bed(spec.sample_kind(), spec.bed);
        let start_at = start_at.map(|at| joined_at(at, paused, since, crate::model::now(), plan.total_s));
        let bed = spec.bed;
        Machine {
            brand: spec.brand,
            model: spec.model.clone(),
            bed,
            max_z: spec.max_z,
            nozzle_tau: spec.nozzle_tau,
            bed_tau: spec.bed_tau,
            spec,
            state: Mutex::new(State {
                job: Job::Idle,
                files: vec![DemoFile {
                    name,
                    bytes,
                    plan,
                    modified: 1_760_745_600.0, // 2026-10-18, the day the samples were sliced
                }],
                current: None,
                clock: 0.0,
                next_set: 0,
                started_at: 0.0,
                finished_at: 0.0,
                repeat: true,
                pos: [bed[0] / 2.0, bed[1] / 2.0, 30.0],
                homed: false,
                nozzle: (AMBIENT, 0.0),
                bed: (AMBIENT, 0.0),
                fan: 0.0,
                speed: 100.0,
                flow: 100.0,
                tool: 0,
                last_ts: None,
                start_at: start_at.map(|f| f.clamp(0.0, 1.0)),
                start_paused: paused,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Load `index` and start it: homed, heaters to the file's first temperatures, waiting for heat.
    fn start(&self, s: &mut State, index: usize, ts: f64, at: f64) {
        let plan = s.files[index].plan.clone();
        s.current = Some(index);
        s.clock = plan.total_s * at;
        s.next_set = 0;
        s.tool = 0;
        // A demo that joins part way has been printing that long: the elapsed time, the estimate and
        // the "finished after" line all agree with the progress.
        s.started_at = ts - s.clock;
        s.homed = true;
        // The file's own first temperatures; a file that sets none (a dry run) runs cold at once.
        s.nozzle.1 = plan.first_nozzle;
        s.bed.1 = plan.first_bed;
        let cold = plan.first_nozzle <= 0.0 && plan.first_bed <= 0.0;
        if at > 0.0 || cold {
            // Joined mid-print: already hot, every command up to here applied.
            self.apply_sets(s, &plan);
            if !cold {
                s.nozzle.0 = s.nozzle.1;
                s.bed.0 = s.bed.1;
            }
            if let Some((p, _)) = plan.position_at(s.clock) {
                s.pos = p;
            }
            s.job = Job::Printing;
        } else {
            s.job = Job::Heating;
        }
    }

    /// The print is done and has been sitting there: the part is on the plate, the heaters are off and
    /// still cooling, the head is lifted clear. It stays finished until someone starts a print.
    fn finish(&self, s: &mut State, ts: f64) {
        let plan = s.files[0].plan.clone();
        s.current = Some(0);
        s.clock = plan.total_s;
        s.next_set = plan.sets.len();
        s.started_at = ts - plan.total_s;
        s.finished_at = ts;
        s.homed = true;
        s.job = Job::Complete;
        s.repeat = false;
        s.nozzle = (80.0, 0.0);
        s.bed = (45.0, 0.0);
        s.fan = 0.0;
        s.tool = 0;
        if let Some((p, _)) = plan.position_at(plan.total_s) {
            s.pos = p;
        }
        s.pos[2] = (s.pos[2] + 20.0).min(self.max_z);
    }

    fn apply_sets(&self, s: &mut State, plan: &Plan) {
        while let Some((t, set)) = plan.sets.get(s.next_set) {
            if *t > s.clock {
                break;
            }
            match *set {
                Set::Nozzle(v) => s.nozzle.1 = v,
                Set::Bed(v) => s.bed.1 = v,
                Set::Fan(v) => s.fan = v.clamp(0.0, 1.0),
                Set::Tool(n) => s.tool = n.min(self.spec.tools.count() - 1),
            }
            s.next_set += 1;
        }
    }

    /// Advance to sample time `ts`: heaters, the print clock, the repeat.
    fn advance(&self, s: &mut State, ts: f64) {
        if let Some(at) = s.start_at.take() {
            if s.job == Job::Idle {
                if at >= 1.0 {
                    self.finish(s, ts);
                } else {
                    self.start(s, 0, ts, at);
                    if s.start_paused && s.job == Job::Printing {
                        s.job = Job::Paused;
                    }
                }
            }
        }
        let dt = s.last_ts.map_or(0.0, |l| (ts - l).clamp(0.0, 10.0));
        s.last_ts = Some(ts);
        let approach = |(t, target): (f64, f64), tau: f64| {
            let goal = if target > 0.0 { target } else { AMBIENT };
            t + (goal - t) * (dt / tau).min(1.0)
        };
        s.nozzle.0 = approach(s.nozzle, self.nozzle_tau);
        s.bed.0 = approach(s.bed, self.bed_tau);
        let Some(index) = s.current else {
            return;
        };
        let plan = s.files[index].plan.clone();
        match s.job {
            Job::Heating
                if (s.nozzle.0 - s.nozzle.1).abs() < 3.0 && (s.bed.0 - s.bed.1).abs() < 2.0 =>
            {
                s.job = Job::Printing
            }
            Job::Printing => {
                s.clock += dt * s.speed / 100.0;
                self.apply_sets(s, &plan);
                if let Some((p, _)) = plan.position_at(s.clock) {
                    s.pos = p;
                }
                if s.clock >= plan.total_s {
                    s.clock = plan.total_s;
                    s.job = Job::Complete;
                    s.finished_at = ts;
                    // PRINT_END: heaters and fan off, head up and out of the way.
                    s.nozzle.1 = 0.0;
                    s.bed.1 = 0.0;
                    s.fan = 0.0;
                    s.pos[2] = (s.pos[2] + 20.0).min(self.max_z);
                }
            }
            Job::Complete if s.repeat && ts - s.finished_at > REPEAT_AFTER_S => {
                self.start(s, index, ts, 0.0)
            }
            _ => {}
        }
    }

    /// Where every tool is: the active one at the head, each idle one parked.
    fn tool_positions(&self, s: &State) -> Vec<[f64; 3]> {
        (0..self.spec.tools.count())
            .map(|t| {
                if t == s.tool {
                    s.pos
                } else {
                    self.spec.park(t, s.pos)
                }
            })
            .collect()
    }

    fn report(&self, s: &State, ts: f64) -> Value {
        let file = s.current.map(|i| &s.files[i]);
        let name = file.map_or("", |f| f.name.as_str());
        let (progress, byte, size, layer, total_layers, velocity) = match file {
            Some(f) => {
                let plan = &f.plan;
                let total = plan.total_s.max(1e-9);
                let (byte, layer, speed) = match (s.job, plan.moves.get(plan.index_at(s.clock))) {
                    (Job::Heating, _) | (_, None) => (0, 0, 0.0),
                    (_, Some(m)) => (m.byte as usize, m.layer, m.speed as f64),
                };
                let moving = s.job == Job::Printing;
                (
                    (s.clock / total).clamp(0.0, 1.0),
                    byte.min(f.bytes.len()),
                    f.bytes.len(),
                    layer,
                    plan.layers,
                    // A sample is played at its own pace, but never faster than this machine's limit.
                    if moving {
                        (speed * s.speed / 100.0).min(self.spec.max_velocity)
                    } else {
                        0.0
                    },
                )
            }
            None => (0.0, 0, 0, 0, 0, 0.0),
        };
        let active = s.job.active();
        let homed = if s.homed { "xyz" } else { "" };
        let p = [s.pos[0], s.pos[1], s.pos[2], 0.0];
        let idle = match s.job {
            Job::Heating | Job::Printing => "Printing",
            _ if s.homed => "Ready",
            _ => "Idle",
        };
        let total_duration = if s.current.is_some() {
            (ts - s.started_at).max(0.0)
        } else {
            0.0
        };
        let mut v = json!({
            "print_stats":{"state":s.job.word(),"filename":name,"total_duration":total_duration,
                "print_duration":if s.current.is_some() { s.clock } else { 0.0 },"filament_used":Value::Null,
                "message":if s.job == Job::Heating {"Heating up"} else {""},
                "info":{"total_layer":if total_layers > 0 { json!(total_layers) } else { Value::Null },
                        "current_layer":if layer > 0 { json!(layer) } else { Value::Null }}},
            "virtual_sdcard":{"progress":progress,"file_position":byte,"file_size":size,"is_active":active,"file_path":name},
            "display_status":{"progress":progress,"message":Value::Null},
            "idle_timeout":{"state":idle},
            "webhooks":{"state":if s.job == Job::Error {"shutdown"} else {"ready"}},
            "motion_report":{"live_position":p,"live_velocity":velocity,"live_extruder_velocity":Value::Null},
            "toolhead":{"position":p,"homed_axes":homed,"max_velocity":self.spec.max_velocity,
                "extruder":if s.tool == 0 { "extruder".to_owned() } else { format!("extruder{}", s.tool) },
                "axis_minimum":[0.0,0.0,0.0,0.0],"axis_maximum":[self.bed[0],self.bed[1],self.max_z,0.0]},
            "gcode_move":{"homing_origin":[0.0,0.0,0.0,0.0],"speed_factor":s.speed/100.0,"extrude_factor":s.flow/100.0,
                "speed":velocity},
            "extruder":{"temperature":s.nozzle.0,"target":s.nozzle.1,
                "power":if s.nozzle.1 > 0.0 { ((s.nozzle.1 - s.nozzle.0) / 40.0 + 0.35).clamp(0.0, 1.0) } else { 0.0 }},
            "heater_bed":{"temperature":s.bed.0,"target":s.bed.1,
                "power":if s.bed.1 > 0.0 { ((s.bed.1 - s.bed.0) / 15.0 + 0.25).clamp(0.0, 1.0) } else { 0.0 }},
            "fan":{"speed":s.fan},
            "heater_fan hotend_fan":{"speed":if s.nozzle.0 > 50.0 {1.0} else {0.0}},
        });
        // A toolchanger names its tools the way the Klipper toolchanger module does.
        if matches!(self.spec.tools, Tools::Changer(_)) {
            let numbers: Vec<usize> = (0..self.spec.tools.count()).collect();
            v["toolchanger"] =
                json!({"tool_number":s.tool,"tool_numbers":numbers,"status":"ready"});
        }
        v
    }
}

/// The report a demo printer gives right now (polish WO-03, 2026-10-18). The model view asks where the head is between the
/// collector's recordings, which on a busy disk are seconds apart; a demo plays its file in real time in memory, so asking is
/// free and is the same machine the collector reads, not another one.
pub fn peek(cfg: &Config) -> Option<Value> {
    Demo::open(cfg)
        .sample(Demo::now())
        .ok()
        .map(|(report, _)| report)
}

/// The driver the collector and the commands open. Cheap: it only holds the shared machine.
pub struct Demo {
    m: Arc<Machine>,
}

impl Demo {
    pub fn open(cfg: &Config) -> Demo {
        Demo { m: machine(cfg) }
    }

    fn find(s: &State, filename: &str) -> Option<usize> {
        let base = filename.rsplit('/').next().unwrap_or(filename);
        s.files
            .iter()
            .position(|f| f.name == filename || f.name == base)
    }

    fn now() -> f64 {
        crate::model::now()
    }

    /// Where every tool is right now: the active one at the head, the others parked.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn tool_positions(&self) -> Vec<[f64; 3]> {
        let s = self.m.lock();
        self.m.tool_positions(&s)
    }
}

impl Driver for Demo {
    fn kind(&self) -> &'static str {
        "demo"
    }
    fn brand(&self) -> &'static str {
        self.m.brand
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            heat: true,
            fan: true,
            speed: true,
            flow: true,
            jog: true,
            home: true,
            motors_off: true,
            extrude: true,
            filament: false,
            pause_resume: true,
            cancel: true,
            start_print: true,
            upload: true,
            console: false,
            macros: false,
            level: false,
            mesh: false,
            z_offset: false,
            klipper: false,
            restart: false,
            emergency_stop: true,
            camera: false,
            thumbnails: true,
            gcode_file: true,
            live_xy: true,
            xy: true,
            push: false,
        }
    }
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        Some(self.m.bed)
    }
    fn model_hint(&mut self) -> Option<String> {
        Some(self.m.model.to_owned())
    }

    fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)> {
        let mut s = self.m.lock();
        self.m.advance(&mut s, ts);
        Ok((self.m.report(&s, ts), Vec::new()))
    }

    fn snapshot(&self) -> Result<StateSnapshot> {
        let s = self.m.lock();
        Ok(StateSnapshot {
            state: if s.job == Job::Error {
                "shutdown"
            } else {
                "ready"
            }
            .into(),
            homed_axes: if s.homed { "xyz" } else { "" }.into(),
            print_state: s.job.word().into(),
            idle_state: if matches!(s.job, Job::Heating | Job::Printing) {
                "printing"
            } else if s.homed {
                "ready"
            } else {
                "idle"
            }
            .into(),
            help: Default::default(),
        })
    }

    fn act(&self, action: &Action) -> Result<Value> {
        let ts = Demo::now();
        let mut s = self.m.lock();
        self.m.advance(&mut s, ts);
        let busy = matches!(s.job, Job::Heating | Job::Printing);
        match action {
            Action::SetTemperature { heater, celsius } => {
                let target = if *celsius > 0.0 {
                    celsius.clamp(0.0, 300.0)
                } else {
                    0.0
                };
                match heater {
                    Heater::Nozzle => s.nozzle.1 = target,
                    Heater::Bed => s.bed.1 = target.min(120.0),
                }
            }
            Action::Fan { fraction } => s.fan = fraction.clamp(0.0, 1.0),
            Action::SpeedFactor { percent } => s.speed = percent.clamp(10.0, 500.0),
            Action::FlowFactor { percent } => s.flow = percent.clamp(10.0, 500.0),
            Action::Jog { axis, mm } => {
                if busy {
                    return Err("the demo printer is printing; pause it to move the head".into());
                }
                if !s.homed {
                    return Err("home the demo printer before moving it".into());
                }
                let (i, max) = match axis {
                    'X' => (0, self.m.bed[0]),
                    'Y' => (1, self.m.bed[1]),
                    'Z' => (2, self.m.max_z),
                    _ => return Err("jog axis must be X, Y or Z".into()),
                };
                s.pos[i] = (s.pos[i] + mm).clamp(0.0, max);
            }
            Action::Home => {
                if busy {
                    return Err("the demo printer is printing".into());
                }
                s.pos = self.m.spec.frame.home(self.m.bed);
                s.homed = true;
            }
            Action::MotorsOff => {
                if busy {
                    return Err("the demo printer is printing".into());
                }
                s.homed = false;
            }
            Action::Extrude { .. } => {
                if s.nozzle.0 < 170.0 {
                    return Err("the demo nozzle is below 170 °C; heat it first".into());
                }
            }
            Action::Pause => {
                if busy {
                    s.job = Job::Paused;
                }
            }
            Action::Resume => {
                if s.job == Job::Paused {
                    s.job = Job::Printing;
                }
            }
            Action::Cancel => {
                if s.job.active() {
                    s.job = Job::Cancelled;
                    s.repeat = false;
                    s.nozzle.1 = 0.0;
                    s.bed.1 = 0.0;
                    s.fan = 0.0;
                }
            }
            Action::StartPrint { filename } => {
                if s.job.active() {
                    return Err("the demo printer is already printing".into());
                }
                let index = Demo::find(&s, filename)
                    .ok_or_else(|| format!("the demo printer has no file {filename}"))?;
                s.repeat = index == 0;
                self.m.start(&mut s, index, ts, 0.0);
            }
            Action::Gcode(_) => return Err("the demo printer takes no typed G-code".into()),
        }
        Ok(json!({"result":"ok"}))
    }

    fn upload(&self, name: &str, bytes: &[u8], start: bool) -> Result<Value> {
        if bytes.len() > MAX_FILE {
            return Err(format!("{name} is over the {MAX_FILE} byte limit"));
        }
        let plan = Plan::parse(bytes);
        if plan.moves.is_empty() {
            return Err(format!("{name} has no moves to play"));
        }
        {
            let mut s = self.m.lock();
            let file = DemoFile {
                name: name.to_owned(),
                bytes: Arc::new(bytes.to_vec()),
                plan: Arc::new(plan),
                modified: Demo::now(),
            };
            match Demo::find(&s, name) {
                // The sample stays; anything else with the same name is replaced.
                Some(i) if i > 0 && s.current != Some(i) => s.files[i] = file,
                Some(_) => return Err(format!("{name} is in use on the demo printer")),
                None => s.files.push(file),
            }
        }
        if start {
            self.act(&Action::StartPrint {
                filename: name.to_owned(),
            })?;
        }
        Ok(json!({"result":"ok","file":name}))
    }

    fn files(&self) -> Result<Vec<Value>> {
        let s = self.m.lock();
        Ok(s.files
            .iter()
            .map(|f| json!({"path":f.name,"size":f.bytes.len(),"modified":f.modified}))
            .collect())
    }

    fn file_metadata(&self, filename: &str) -> Result<Value> {
        let s = self.m.lock();
        let f = &s.files[Demo::find(&s, filename).ok_or("no such file on the demo printer")?];
        Ok(
            json!({"size":f.bytes.len(),"modified":f.modified,"slicer":"PrusaSlicer",
            "estimated_time":f.plan.estimate_s.unwrap_or(f.plan.total_s),"layer_count":f.plan.layers,
            "filament_type":"PETG","nozzle_diameter":0.4,"source":"demo printer"}),
        )
    }

    fn file_size(&self, filename: &str) -> Result<usize> {
        let s = self.m.lock();
        let i = Demo::find(&s, filename).ok_or("no such file on the demo printer")?;
        Ok(s.files[i].bytes.len())
    }

    fn file_bytes(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let s = self.m.lock();
        let i = Demo::find(&s, filename).ok_or("no such file on the demo printer")?;
        let b = &s.files[i].bytes;
        if b.len() > max_bytes {
            return Err(format!(
                "{filename} is over the {max_bytes} byte gcode limit"
            ));
        }
        Ok(b.as_ref().clone())
    }

    /// The largest picture the file carries (the samples carry three; a file someone uploads may not).
    fn file_thumbnail(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let bytes = {
            let s = self.m.lock();
            let i = Demo::find(&s, filename).ok_or("no such file on the demo printer")?;
            s.files[i].bytes.clone()
        };
        let (_, png) = thumbnails(&bytes)
            .into_iter()
            .next()
            .ok_or("that file has no thumbnail")?;
        if png.len() > max_bytes {
            return Err("the thumbnail is too large to serve".into());
        }
        Ok(png)
    }

    fn emergency_stop(&self) -> Result<Value> {
        let mut s = self.m.lock();
        s.job = Job::Error;
        s.repeat = false;
        s.homed = false;
        s.nozzle.1 = 0.0;
        s.bed.1 = 0.0;
        s.fan = 0.0;
        Ok(json!({"result":"ok"}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str, model: &str, start_at: Option<f64>) -> Config {
        let mut c = Config::default();
        c.driver = "demo".into();
        c.printer_id = id.into();
        c.database = format!("demo-test-{id}.sqlite3");
        c.printer_model = Some(model.into());
        c.demo_start_at = start_at;
        c
    }

    /// Finish-2 F10: a restart of the service continues a demo farm's prints from where the time since it was added puts
    /// them (it started them over before), wrapping as the sample repeats; a paused one stays where it was put.
    #[test]
    fn a_restart_continues_a_demo_print_instead_of_starting_it_over() {
        assert_eq!(joined_at(0.3, false, Some(1000.0), 1000.0, 600.0), 0.3);
        assert!((joined_at(0.3, false, Some(1000.0), 1060.0, 600.0) - 0.4).abs() < 1e-9);
        assert!((joined_at(0.9, false, Some(1000.0), 1120.0, 600.0) - 0.1).abs() < 1e-9);
        assert_eq!(joined_at(0.3, true, Some(1000.0), 1300.0, 600.0), 0.3);
        assert_eq!(joined_at(0.3, false, None, 1300.0, 600.0), 0.3);
        assert!(demo_farm().iter().all(|e| e.demo_since.is_some()), "the farm keeps when it was added");
    }

    const SMALL: &[u8] = b"; estimated printing time (normal mode) = 1m 40s\nM140 S60\nM104 S215\nG28\nG90\n;LAYER_CHANGE\nG1 Z0.2 F600\nM106 S255\nG1 X10 Y0 F6000 E1\nG1 X10 Y10 E1\n;LAYER_CHANGE\nG1 Z0.4\nG1 X0 Y10 E1\nG1 X0 Y0 E1\nM107\nM104 S0\n";

    #[test]
    fn the_samples_unpack_and_plan() {
        for core in [false, true] {
            let (name, bytes, plan) = sample(core);
            assert!(name.ends_with(".gcode"));
            assert!(bytes.len() > 1_000_000, "{name}: {} bytes", bytes.len());
            assert!(
                plan.moves.len() > 10_000,
                "{name}: {} moves",
                plan.moves.len()
            );
            assert!(plan.layers > 100, "{name}: {} layers", plan.layers);
            assert_eq!(
                Some(plan.total_s),
                plan.estimate_s,
                "clock stretched to the estimate"
            );
            assert!(plan.first_nozzle >= 200.0 && plan.first_bed >= 60.0);
            // Every move ends on the bed, give or take the CORE One's purge line just off its front.
            let bed = if core { 300.0 } else { 350.0 };
            assert!(plan
                .moves
                .iter()
                .all(|m| (-5.0..=bed).contains(&(m.x as f64))
                    && (-5.0..=bed).contains(&(m.y as f64))));
        }
    }

    #[test]
    fn a_plan_follows_the_file_in_time() {
        let plan = Plan::parse(SMALL);
        assert_eq!(plan.estimate_s, Some(100.0));
        assert_eq!(plan.total_s, 100.0);
        assert_eq!(plan.layers, 2);
        assert_eq!((plan.first_nozzle, plan.first_bed), (215.0, 60.0));
        // Starts at the origin, ends back at the origin on layer 2.
        let (p, m) = plan.position_at(100.0).unwrap();
        assert_eq!((p[0], p[1], m.layer), (0.0, 0.0, 2));
        assert!((p[2] - 0.4).abs() < 1e-6);
        // Halfway along the first 10 mm side the head is at X 5.
        let first_side = plan
            .moves
            .iter()
            .position(|m| m.x == 10.0 && m.y == 0.0)
            .unwrap();
        let (t0, t1) = (plan.moves[first_side - 1].t, plan.moves[first_side].t);
        let (p, _) = plan.position_at((t0 + t1) / 2.0).unwrap();
        assert!((p[0] - 5.0).abs() < 1e-6 && p[1] == 0.0, "{p:?}");
        // Bytes rise with time.
        assert!(plan
            .moves
            .windows(2)
            .all(|w| w[0].byte < w[1].byte && w[0].t <= w[1].t));
        // The fan comes on and goes off in file order.
        let fans: Vec<_> = plan
            .sets
            .iter()
            .filter(|s| matches!(s.1, Set::Fan(_)))
            .collect();
        assert_eq!(fans.len(), 2);
    }

    #[test]
    fn estimates_in_both_slicer_forms_are_read() {
        assert_eq!(
            parse_estimate("; estimated printing time (normal mode) = 2h 48m 40s"),
            Some(10120.0)
        );
        assert_eq!(parse_estimate(";TIME:6380"), Some(6380.0));
        assert_eq!(
            parse_estimate("; estimated printing time (normal mode) = soon"),
            None
        );
        assert_eq!(
            macro_param("PRINT_START BED=85 EXTRUDER=250", "extruder"),
            Some(250.0)
        );
    }

    #[test]
    fn a_command_reaches_the_printer_the_collector_reads() {
        // The bug this replaces: every command opened a fresh demo, so nothing changed on screen.
        let c = cfg("shared", "Voron 2.4 350", None);
        let mut reader = Demo::open(&c);
        Demo::open(&c)
            .act(&Action::SetTemperature {
                heater: Heater::Nozzle,
                celsius: 240.0,
            })
            .unwrap();
        Demo::open(&c).act(&Action::Home).unwrap();
        let (v, _) = reader.sample(1000.0).unwrap();
        assert_eq!(v["extruder"]["target"], 240.0);
        assert_eq!(v["toolhead"]["homed_axes"], "xyz");
        // Two printers do not share a machine.
        let (other, _) = Demo::open(&cfg("other", "Voron 2.4 350", None))
            .sample(1000.0)
            .unwrap();
        assert_eq!(other["extruder"]["target"], 0.0);
        // The command gate needs "ready" to send anything.
        assert_eq!(reader.snapshot().unwrap().state, "ready");
    }

    #[test]
    fn a_started_print_heats_then_moves_then_finishes_and_repeats() {
        let c = cfg("cycle", "CORE One L", None);
        let mut d = Demo::open(&c);
        d.upload("square.gcode", SMALL, false).unwrap();
        // Start the sample at 0 by hand through the machine, with a clock we control.
        {
            let mut s = d.m.lock();
            s.last_ts = Some(0.0);
            d.m.start(&mut s, 0, 0.0, 0.0);
            assert!(s.repeat);
        }
        let (v, _) = d.sample(1.0).unwrap();
        assert_eq!(v["print_stats"]["state"], "printing");
        assert_eq!(v["print_stats"]["message"], "Heating up");
        assert_eq!(v["virtual_sdcard"]["file_position"], 0);
        // Heat until hot (bed tau 60 s: a few minutes of sample time in 10 s steps).
        let mut ts = 1.0;
        while d.m.lock().job == Job::Heating {
            ts += 10.0;
            d.sample(ts).unwrap();
            assert!(ts < 2000.0, "never got hot");
        }
        let total = d.m.lock().files[0].plan.total_s;
        let (a, _) = d.sample(ts + 10.0).unwrap();
        let (b, _) = d.sample(ts + 20.0).unwrap();
        assert!(
            b["virtual_sdcard"]["file_position"].as_u64()
                > a["virtual_sdcard"]["file_position"].as_u64()
        );
        assert!(b["print_stats"]["info"]["current_layer"].as_u64() >= Some(1));
        assert_ne!(
            a["motion_report"]["live_position"],
            b["motion_report"]["live_position"]
        );
        // Run to the end in 10 s steps (the clock never jumps more than 10 s a sample).
        ts += 20.0;
        let mut finished = None;
        for _ in 0..((total / 10.0) as usize + 5) {
            ts += 10.0;
            let (v, _) = d.sample(ts).unwrap();
            if v["print_stats"]["state"] == "complete" {
                finished = Some(v);
                break;
            }
        }
        let v = finished.expect("the sample finishes");
        assert_eq!(v["virtual_sdcard"]["progress"], 1.0);
        assert_eq!(v["extruder"]["target"], 0.0);
        // The sample prints again after it has sat on the plate.
        ts += REPEAT_AFTER_S + 1.0;
        let (v, _) = d.sample(ts).unwrap();
        assert_eq!(v["print_stats"]["state"], "printing");
    }

    #[test]
    fn joining_mid_print_is_hot_and_part_way() {
        let mut d = Demo::open(&cfg("mid", "Voron 2.4 350", Some(0.4)));
        let (v, _) = d.sample(50.0).unwrap();
        assert_eq!(v["print_stats"]["state"], "printing");
        let p = v["virtual_sdcard"]["progress"].as_f64().unwrap();
        assert!((0.39..0.41).contains(&p), "{p}");
        assert!(v["extruder"]["temperature"].as_f64().unwrap() > 200.0);
        assert!(v["print_stats"]["info"]["current_layer"].as_u64().unwrap() > 50);
    }

    #[test]
    fn controls_pause_resume_cancel_and_guard_motion() {
        let c = cfg("controls", "Voron 2.4 350", Some(0.2));
        let mut d = Demo::open(&c);
        d.sample(10.0).unwrap();
        assert!(
            d.act(&Action::Jog { axis: 'X', mm: 5.0 }).is_err(),
            "no jogging mid-print"
        );
        assert!(d
            .act(&Action::StartPrint {
                filename: "3D3D twist vase.gcode".into()
            })
            .is_err());
        d.act(&Action::Pause).unwrap();
        assert_eq!(d.snapshot().unwrap().print_state, "paused");
        d.act(&Action::Jog { axis: 'Z', mm: 5.0 }).unwrap();
        d.act(&Action::Resume).unwrap();
        assert_eq!(d.snapshot().unwrap().print_state, "printing");
        d.act(&Action::Cancel).unwrap();
        assert_eq!(d.snapshot().unwrap().print_state, "cancelled");
        assert!(
            !d.m.lock().repeat,
            "a cancelled demo does not start itself again"
        );
        // A file that is not on the printer, and a path, are refused; nothing is read from disk.
        assert!(d
            .act(&Action::StartPrint {
                filename: "/etc/passwd".into()
            })
            .is_err());
        assert!(d.act(&Action::Gcode("G28".into())).is_err());
        d.emergency_stop().unwrap();
        assert_eq!(d.snapshot().unwrap().state, "shutdown");
    }

    #[test]
    fn a_dry_run_on_the_demo_moves_cold() {
        let d = Demo::open(&cfg("dry", "Voron 2.4 350", None));
        let (_, bytes, _) = sample(false);
        let text = String::from_utf8_lossy(bytes);
        let dry = crate::dry_run::transform(&text, &crate::dry_run::Recipe::default()).unwrap();
        d.upload("vase (dry run).gcode", dry.as_bytes(), true)
            .unwrap();
        let mut d = d;
        let (a, _) = d.sample(Demo::now() + 1.0).unwrap();
        let (b, _) = d.sample(Demo::now() + 30.0).unwrap();
        assert_eq!(a["print_stats"]["state"], "printing");
        assert_eq!(b["extruder"]["target"], 0.0, "no heat in a dry run");
        assert!(
            b["virtual_sdcard"]["file_position"].as_u64() > Some(0),
            "moving at once"
        );
        // every Z is 30 mm up: the head never comes below 30 mm while the layers run
        assert!(b["toolhead"]["position"][2].as_f64().unwrap() >= 30.0);
    }

    #[test]
    fn a_demo_printer_needs_no_address() {
        // WO-03's "Add a demo printer" sent no address and the farm refused it.
        let mut c = Config::default();
        c.printers.push(crate::config::PrinterEntry {
            id: "demo-1".into(),
            name: "Demo".into(),
            driver: "demo".into(),
            ..Default::default()
        });
        assert_eq!(c.validate(), Ok(()));
        let own = c
            .for_printer(&c.printers[0].clone(), std::path::Path::new("."))
            .unwrap();
        assert_eq!(own.validate(), Ok(()));
        c.printers[0].address = "http://192.168.1.2".into();
        assert!(c.validate().is_err(), "a demo printer takes no address");
    }

    #[test]
    fn files_upload_and_serve_for_the_follower() {
        let d = Demo::open(&cfg("files", "CORE One L", None));
        d.upload("square.gcode", SMALL, false).unwrap();
        let names: Vec<String> = d
            .files()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["3D3D twist bowl.gcode", "square.gcode"]);
        assert_eq!(d.file_bytes("square.gcode", SMALL.len()).unwrap(), SMALL);
        assert!(d.file_bytes("square.gcode", SMALL.len() - 1).is_err());
        assert_eq!(d.file_size("square.gcode").unwrap(), SMALL.len());
        assert_eq!(d.file_metadata("square.gcode").unwrap()["layer_count"], 2);
        assert!(d.upload("empty.gcode", b"; nothing\n", false).is_err());
        assert!(
            d.upload("3D3D twist bowl.gcode", SMALL, false).is_err(),
            "the sample stays"
        );
        assert_eq!(d.kind(), "demo");
        assert_eq!(d.brand(), "Prusa");
        let caps = d.capabilities();
        assert!(caps.start_print && caps.upload && caps.live_xy && !caps.klipper && !caps.camera);
    }

    // ---- polish WO-02: a demo printer for any catalogue machine ----------------------------------

    fn machine_cfg(id: &str, cad: &str, start_at: Option<f64>, paused: bool) -> Config {
        let mut c = cfg(id, "", start_at);
        c.printer_model = None;
        c.cad_model = Some(cad.into());
        c.demo_paused = paused;
        c
    }

    /// What a plan draws: [min x, max x, min y, max y, max z] over the layers between the first and the
    /// last. The first layer starts from the origin G28 leaves (and may hold a purge line); the last
    /// ends with the head parked at a corner.
    fn extent(plan: &Plan) -> [f64; 5] {
        let mut e = [f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MIN];
        for m in plan
            .moves
            .iter()
            .filter(|m| m.layer >= 2 && m.layer < plan.layers)
        {
            e[0] = e[0].min(m.x as f64);
            e[1] = e[1].max(m.x as f64);
            e[2] = e[2].min(m.y as f64);
            e[3] = e[3].max(m.y as f64);
            e[4] = e[4].max(m.z as f64);
        }
        e
    }

    #[test]
    fn a_demo_printer_takes_its_bed_travel_and_kinematics_from_its_machine() {
        // (catalogue id, bed, Z travel, what moves). The V2.4's gantry climbs, the Trident's, V0's and CORE's
        // beds drop, the Ender's bed slides in Y; the DoomCube has no registered motion, so nothing is claimed.
        let table = [
            ("voron_v24_350_printed", 350.0, 330.0, Frame::GantryZ),
            ("voron_v24_300_printed", 300.0, 280.0, Frame::GantryZ),
            ("prusa_core_one_l", 300.0, 330.0, Frame::BedZ),
            ("voron_trident_300", 300.0, 250.0, Frame::BedZ),
            ("voron_v02_120", 120.0, 120.0, Frame::BedZ),
            ("micron_plus_r1_180", 180.0, 165.0, Frame::GantryZ),
            // Polish WO-03 (2026-10-18): the SV08's travel now reaches its plate, from the nozzle and plate measured in its model
            // (printer-print-frames.json): Z runs from -8.0 to the registered 300, so 308 (it was 300 from the pose).
            ("sovol_sv08_350", 350.0, 308.0, Frame::GantryZ),
            ("doomcube2_350_reference", 350.0, 350.0, Frame::Unregistered),
            ("ender3_stock_220", 220.0, 244.0, Frame::BedY),
        ];
        for (id, bed, z, frame) in table {
            let s = Spec::catalogue(id);
            assert_eq!((s.bed, s.max_z, s.frame), ([bed, bed], z, frame), "{id}");
            // The driver reports exactly that: the bed and the travel the model page and the queue read.
            let mut d = Demo::open(&machine_cfg(&format!("spec-{id}"), id, None, false));
            assert_eq!(d.bed_hint(), Some([bed, bed]));
            let (v, _) = d.sample(1000.0).unwrap();
            assert_eq!(
                v["toolhead"]["axis_maximum"],
                json!([bed, bed, z, 0.0]),
                "{id}"
            );
        }
        // The same reading over machines registered before this work, from 3D Print Rig's own profiles: a bed
        // group that holds parts decides (it slides in Y, or drops in Z); a machine with no such group climbs.
        for (id, frame) in [
            ("tictac_21_120", Frame::BedY),
            ("satsuma180_v10", Frame::BedZ),
            ("rook_mk2_120", Frame::BedZ),
            ("the100_v11_165", Frame::BedZ),
            ("annex_k2_assembly", Frame::BedZ),
            ("annex_k1_assembly", Frame::GantryZ),
            ("fysetc_v24_250_pro", Frame::GantryZ),
        ] {
            assert_eq!(Spec::catalogue(id).frame, frame, "{id}");
        }
        assert_eq!(Frame::of_profile(None), Frame::Unregistered);
        assert_eq!(Spec::catalogue("voron_trident_300").brand, "Voron");
        assert_eq!(Spec::catalogue("ender3_stock_220").brand, "Creality");
        // Homing leaves a Voron or Prusa at the centre and a bed slinger at its front left corner.
        for (id, at) in [
            ("voron_trident_300", [150.0, 150.0, 10.0]),
            ("ender3_stock_220", [0.0, 0.0, 10.0]),
        ] {
            let d = Demo::open(&machine_cfg(&format!("home-{id}"), id, None, false));
            d.act(&Action::Home).unwrap();
            assert_eq!(d.m.lock().pos, at, "{id}");
        }
    }

    #[test]
    fn every_catalogue_machine_is_a_demo_printer_with_a_sample_that_fits_its_bed() {
        let mut seen = 0;
        for m in printer_models::catalogue()["machines"].as_array().unwrap() {
            let id = m["id"].as_str().unwrap();
            let s = Spec::catalogue(id);
            let kind = s.sample_kind();
            let e = extent(&base(kind).2);
            let (w, d, h) = (e[1] - e[0], e[3] - e[2], e[4]);
            assert!(
                w <= s.bed[0] - 20.0 && d <= s.bed[1] - 20.0 && h <= s.max_z - 10.0,
                "{id}: {kind:?} is {w:.0} x {d:.0} x {h:.0} on a {:?} bed with {} mm of Z",
                s.bed,
                s.max_z
            );
            seen += 1;
        }
        assert_eq!(
            seen,
            printer_models::catalogue()["machines"]
                .as_array()
                .unwrap()
                .len()
        );
    }

    #[test]
    fn a_sample_is_centred_on_the_machines_own_bed_and_its_bytes_are_never_edited() {
        for e in demo_farm() {
            let id = e.cad_model.as_deref().unwrap();
            let s = Spec::catalogue(id);
            let kind = s.sample_kind();
            let (_, bytes, plan) = for_bed(kind, s.bed);
            let x = extent(&plan);
            let (cx, cy) = ((x[0] + x[1]) / 2.0, (x[2] + x[3]) / 2.0);
            assert!(
                (cx - s.bed[0] / 2.0).abs() < 1.5 && (cy - s.bed[1] / 2.0).abs() < 1.5,
                "{id}: {kind:?} is centred at {cx:.1},{cy:.1} on a {:?} bed",
                s.bed
            );
            // The moved copy plays the same print: same layers, same time, same number of moves.
            let b = base(kind);
            assert_eq!(
                (plan.layers, plan.moves.len()),
                (b.2.layers, b.2.moves.len()),
                "{id}"
            );
            assert_eq!(plan.total_s, b.2.total_s, "{id}");
            // The shipped file is as it was unpacked: where the bed centre is the slicer's, the very same bytes.
            if (s.bed[0] / 2.0, s.bed[1] / 2.0) == (kind.file().2[0], kind.file().2[1]) {
                assert!(
                    Arc::ptr_eq(&bytes, &b.1),
                    "{id}: no copy when nothing moves"
                );
            } else {
                assert!(
                    !Arc::ptr_eq(&bytes, &b.1) && bytes.len() < b.1.len() * 11 / 10,
                    "{id}"
                );
            }
        }
    }

    #[test]
    fn shifting_a_file_changes_only_the_x_and_y_of_its_moves() {
        let src = b"; layer\nG28\nG90\nG1 Z.2 F600\nG1 X10 Y0 F6000 E1 ; comment X1\nG3 X20.5 Y-3 I-4.5 J2 E.2\nM104 S215\nEXCLUDE_OBJECT_DEFINE NAME='a' CENTER=1,2\nG1 X-1.234 Y99.9996\n";
        let out = String::from_utf8(shifted(src, 5.0, -2.0)).unwrap();
        assert_eq!(
            out,
            "; layer\nG28\nG90\nG1 Z.2 F600\nG1 X15 Y-2 F6000 E1 ; comment X1\nG3 X25.5 Y-5 I-4.5 J2 E.2\nM104 S215\nEXCLUDE_OBJECT_DEFINE NAME='a' CENTER=1,2\nG1 X3.766 Y98\n"
        );
        // A file's own numbers come back as they were when there is nothing to move.
        let plain = b"G90\nG1 X10.5 Y-3.25 E.1 ; x\nM104 S200\n";
        assert_eq!(
            shifted(plain, 0.0, 0.0),
            plain.to_vec(),
            "no distance, no change"
        );
    }

    #[test]
    fn each_kinematics_family_moves_its_own_parts_at_a_known_file_time() {
        // The pose the model page draws is the machine's registered motion applied to the position the
        // demo reports. Two file times of the same print show what moves and what does not.
        let pose_at = |id: &str, at: f64| -> (Value, Value) {
            let mut d = Demo::open(&machine_cfg(
                &format!("pose-{id}-{at}"),
                id,
                Some(at),
                false,
            ));
            let (v, _) = d.sample(1000.0).unwrap();
            let p = &v["toolhead"]["position"];
            let pos = [p[0].as_f64(), p[1].as_f64(), p[2].as_f64()];
            let profile = printer_models::profile(id).expect("registered motion");
            let cal = printer_models::Calibration::starting(profile);
            let pose = printer_models::pose_for_tool(profile, &cal, pos, Some(0));
            assert_eq!(
                pose["clamped"],
                json!([]),
                "{id}: inside the registered travel"
            );
            (v, pose)
        };
        let g = |pose: &Value, name: &str| -> [f64; 3] {
            let a = pose["groups"][name]
                .as_array()
                .unwrap_or_else(|| panic!("no {name} in {pose}"));
            [
                a[0].as_f64().unwrap(),
                a[1].as_f64().unwrap(),
                a[2].as_f64().unwrap(),
            ]
        };
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} vs {b}");

        // Voron 2.4: the gantry climbs with Z, the X beam also moves with Y, the head with X, Y and Z.
        let (v0, p0) = pose_at("voron_v24_350_printed", 0.3);
        let (v1, p1) = pose_at("voron_v24_350_printed", 0.7);
        let z = |v: &Value| v["toolhead"]["position"][2].as_f64().unwrap();
        let (dz, dy) = (
            z(&v1) - z(&v0),
            v1["toolhead"]["position"][1].as_f64().unwrap()
                - v0["toolhead"]["position"][1].as_f64().unwrap(),
        );
        assert!(dz > 10.0, "the print climbed {dz} mm between 30% and 70%");
        near(g(&p1, "z_gantry")[2] - g(&p0, "z_gantry")[2], dz);
        assert_eq!(
            (g(&p0, "z_gantry")[0], g(&p0, "z_gantry")[1]),
            (0.0, 0.0),
            "the gantry moves in Z only"
        );
        near(g(&p1, "x_beam")[2] - g(&p0, "x_beam")[2], dz);
        near(g(&p1, "x_beam")[1] - g(&p0, "x_beam")[1], dy);
        assert_eq!(g(&p0, "x_beam")[0], 0.0, "the beam does not move in X");
        near(g(&p1, "head")[2] - g(&p0, "head")[2], dz);
        assert!(
            p0["groups"].get("bed").is_none(),
            "the bed of a Voron 2.4 does not move"
        );

        // Trident: the bed drops by exactly the Z that rose. The base model has no gantry or head; the R2 gantry option the demo Trident carries
        // (F03, 2026-10-19) gives it a beam that moves with Y and a carriage that moves with X and Y, and the bed still moves in Z only.
        let (v0, p0) = pose_at("voron_trident_300", 0.2);
        let (v1, p1) = pose_at("voron_trident_300", 0.8);
        let dz = z(&v1) - z(&v0);
        assert!(dz > 10.0);
        assert_eq!(
            (g(&p0, "bed")[0], g(&p0, "bed")[1]),
            (0.0, 0.0),
            "the bed moves in Z only"
        );
        near(g(&p1, "bed")[2] - g(&p0, "bed")[2], -dz);
        let mut moving: Vec<&str> = p0["groups"].as_object().unwrap().keys().map(String::as_str).filter(|k| *k != "fixed").collect();
        moving.sort_unstable();
        assert_eq!(moving, ["bed", "gantry", "head"], "the bed, the X beam and the carriage move");
        let at = |v: &Value, i: usize| v["toolhead"]["position"][i].as_f64().unwrap();
        let (tx, ty) = (at(&v1, 0) - at(&v0, 0), at(&v1, 1) - at(&v0, 1));
        assert!(tx.abs() > 1.0 || ty.abs() > 1.0, "the head went somewhere between 20% and 80%: {tx} {ty}");
        near(g(&p1, "gantry")[1] - g(&p0, "gantry")[1], ty);
        assert_eq!((g(&p0, "gantry")[0], g(&p0, "gantry")[2]), (0.0, 0.0), "the beam moves in Y only");
        near(g(&p1, "head")[0] - g(&p0, "head")[0], tx);
        near(g(&p1, "head")[1] - g(&p0, "head")[1], ty);
        assert_eq!(g(&p0, "head")[2], 0.0, "the carriage does not climb: the bed drops");

        // Ender-3: the bed slides in Y, the beam climbs with Z, the head slides along it with X.
        let (v0, p0) = pose_at("ender3_stock_220", 0.30);
        let (v1, p1) = pose_at("ender3_stock_220", 0.31);
        let pos = |v: &Value, i: usize| v["toolhead"]["position"][i].as_f64().unwrap();
        let (dx, dy) = (pos(&v1, 0) - pos(&v0, 0), pos(&v1, 1) - pos(&v0, 1));
        assert!(dy.abs() > 1.0, "the head moved {dy} mm in Y a moment later");
        near(g(&p1, "bed")[2] - g(&p0, "bed")[2], dy);
        assert_eq!(
            (g(&p0, "bed")[0], g(&p0, "bed")[1]),
            (0.0, 0.0),
            "the bed moves in Y only (the model's Z)"
        );
        near(g(&p1, "head")[0] - g(&p0, "head")[0], dx);
        near(
            g(&p1, "beam")[1] - g(&p0, "beam")[1],
            pos(&v1, 2) - pos(&v0, 2),
        );
        assert_eq!(g(&p0, "beam")[0], 0.0);
    }

    #[test]
    fn the_four_samples_carry_pictures_the_tiles_can_show() {
        // A machine that takes each sample, and the driver serving the largest picture its file carries.
        let takes = [
            (Kind::Vase, "voron_trident_300"),
            (Kind::Bowl, "prusa_core_one_l"),
            (Kind::Small, "voron_v02_120"),
            (Kind::Duo, "ratrig_vcore_40_300_idex"),
        ];
        for (i, (kind, id)) in takes.into_iter().enumerate() {
            let t = thumbnails(&base(kind).1);
            let sizes: Vec<usize> = t.iter().map(|(a, _)| *a).collect();
            assert_eq!(sizes, [440 * 240, 313 * 173, 16 * 16], "{kind:?}");
            assert!(
                t.iter()
                    .all(|(_, png)| png.starts_with(b"\x89PNG\r\n\x1a\n")),
                "{kind:?}"
            );
            let d = Demo::open(&machine_cfg(&format!("thumb{i}"), id, None, false));
            assert_eq!(d.m.spec.sample_kind(), kind, "{id}");
            let name = d.files().unwrap()[0]["path"].as_str().unwrap().to_owned();
            assert_eq!(name, base(kind).0);
            assert_eq!(d.file_thumbnail(&name, 512 * 1024).unwrap(), t[0].1);
            assert!(
                d.file_thumbnail(&name, 100).is_err(),
                "too large for the limit it was given"
            );
        }
        let d = Demo::open(&machine_cfg("thumb-none", "voron_trident_300", None, false));
        d.upload("square.gcode", SMALL, false).unwrap();
        assert!(d
            .file_thumbnail("square.gcode", 512 * 1024)
            .unwrap_err()
            .contains("no thumbnail"));
        assert!(d.file_thumbnail("nope.gcode", 512 * 1024).is_err());
        assert!(d.capabilities().thumbnails);
    }

    // Polish WO-03 (2026-10-18, Opus supplement S1): the DoomCube 2 (static upstream, so it never moved) gave its place to a
    // toolchanger, a Voron 2.4 300 with an open-licence StealthChanger bank; the farm is still eight machines.
    #[test]
    fn the_demo_farm_is_eight_machines_staggered_to_show_every_state() {
        // Finish-2 F09 (2026-10-19): eight machines and the Prusa MK3S+, the farm's bed slinger from Prusa (the test keeps its name; the count is nine).
        let farm = demo_farm();
        assert_eq!(farm.len(), 9);
        let ids: std::collections::BTreeSet<&str> = farm.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), 9, "every id is its own");
        for e in &farm {
            let cad = e.cad_model.as_deref().unwrap();
            let row = printer_models::machine(cad)
                .unwrap_or_else(|| panic!("{cad} is not in the catalogue"));
            // Every model is an open licence a build may carry (finish-2 F09: Prusa's Open Community License is one of them, the CORE One L's CAD ships in the build).
            let class = row["licence"]["class"].as_str().unwrap();
            assert!(printer_models::may_bundle(class), "{cad}: {class}");
            assert_eq!(e.driver, "demo");
            assert!(e.address.is_empty(), "a demo printer takes no address");
            assert_eq!(e.bed_size_mm, Some(Spec::catalogue(cad).bed), "{cad}");
        }
        let names: Vec<&str> = farm
            .iter()
            .map(|e| e.cad_model.as_deref().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "voron_v24_350_printed",
                "prusa_core_one_l",
                "voron_trident_300",
                "voron_v02_120",
                "micron_plus_r1_180",
                "sovol_sv08_350",
                "voron_v24_300_printed",
                "ender3_stock_220",
                "prusa_mk3s"
            ]
        );
        // Finish-2 F03: all eight printing, so every machine moves at the press (polish WO-02 had one heating, one finished
        // and one paused, which stood still); each at its own place in its print.
        let starts: Vec<f64> = farm.iter().map(|e| e.demo_start_at.unwrap()).collect();
        assert!(starts.iter().all(|s| *s > 0.0 && *s < 1.0), "{starts:?}");
        assert_eq!(farm.iter().filter(|e| e.demo_paused).count(), 0);
        let mut printing: Vec<f64> = starts
            .iter()
            .copied()
            .filter(|s| *s > 0.0 && *s < 1.0)
            .collect();
        printing.sort_by(f64::total_cmp);
        printing.dedup();
        assert_eq!(printing.len(), 9, "nine different points in their prints");
        // The farm validates as a configuration (no address, valid ids and names).
        let mut c = Config::default();
        c.printers = farm.clone();
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn a_demo_that_joins_a_print_can_be_heating_printing_paused_or_finished() {
        let state = |name: &str, at: f64, paused: bool| {
            let mut d = Demo::open(&machine_cfg(name, "voron_trident_300", Some(at), paused));
            let (a, _) = d.sample(1000.0).unwrap();
            let (b, _) = d.sample(1030.0).unwrap();
            (a, b)
        };
        let (a, _) = state("st-heat", 0.0, false);
        assert_eq!(
            (
                a["print_stats"]["state"].as_str(),
                a["print_stats"]["message"].as_str()
            ),
            (Some("printing"), Some("Heating up"))
        );
        assert!(
            a["extruder"]["temperature"].as_f64().unwrap() < 100.0
                && a["extruder"]["target"].as_f64().unwrap() > 200.0
        );
        let (a, b) = state("st-mid", 0.62, false);
        assert!((a["virtual_sdcard"]["progress"].as_f64().unwrap() - 0.62).abs() < 0.01);
        assert_ne!(
            a["motion_report"]["live_position"], b["motion_report"]["live_position"],
            "it moves"
        );
        let (a, b) = state("st-paused", 0.45, true);
        assert_eq!(b["print_stats"]["state"], "paused");
        assert_eq!(
            a["motion_report"]["live_position"], b["motion_report"]["live_position"],
            "a paused head stays put"
        );
        assert_eq!(
            a["virtual_sdcard"]["file_position"],
            b["virtual_sdcard"]["file_position"]
        );
        assert!(
            a["extruder"]["target"].as_f64().unwrap() > 200.0,
            "paused is still hot"
        );
        let mut d = Demo::open(&machine_cfg(
            "st-done",
            "voron_trident_300",
            Some(1.0),
            false,
        ));
        let (a, _) = d.sample(1000.0).unwrap();
        assert_eq!(a["print_stats"]["state"], "complete");
        assert_eq!(a["virtual_sdcard"]["progress"], 1.0);
        assert_eq!(a["extruder"]["target"], 0.0);
        // A finished demo stays finished: it does not start itself again after the usual repeat time.
        let (b, _) = d.sample(1000.0 + REPEAT_AFTER_S * 20.0).unwrap();
        assert_eq!(b["print_stats"]["state"], "complete");
        assert!(
            a["print_stats"]["total_duration"].as_f64().unwrap() > 3600.0,
            "it has been printing the whole print"
        );
        // Someone can print it again.
        d.act(&Action::StartPrint {
            filename: a["print_stats"]["filename"].as_str().unwrap().into(),
        })
        .unwrap();
        assert_eq!(d.snapshot().unwrap().print_state, "printing");
    }

    #[test]
    fn tool_changes_in_the_file_move_the_active_tool_and_idle_tools_park() {
        // A two-colour sample on an IDEX machine: the file says T0 and T1 and the printer follows.
        let id = "ratrig_vcore_40_300_idex";
        let spec = Spec::catalogue(id);
        assert_eq!(
            (spec.tools.clone(), spec.sample_kind(), spec.bed),
            (Tools::Idex, Kind::Duo, [300.0, 300.0])
        );
        let (_, bytes, plan) = for_bed(Kind::Duo, spec.bed);
        assert!(
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|l| *l == "T0" || *l == "T1")
                .count()
                > 100
        );
        let changes: Vec<(f64, usize)> = plan
            .sets
            .iter()
            .filter_map(|(t, s)| {
                if let Set::Tool(n) = s {
                    Some((*t, *n))
                } else {
                    None
                }
            })
            .collect();
        // A change after the first layers, to a tool that was not active a moment before.
        let k = (10..changes.len())
            .find(|&k| changes[k].1 != changes[k - 1].1)
            .unwrap();
        let (t, to, from) = (changes[k].0, changes[k].1, changes[k - 1].1);
        for (n, (clock, active)) in [(t - 0.5, from), (t + 0.5, to)].into_iter().enumerate() {
            let mut d = Demo::open(&machine_cfg(&format!("idex-{n}"), id, None, false));
            {
                let mut s = d.m.lock();
                s.last_ts = Some(0.0);
                d.m.start(&mut s, 0, 0.0, clock / plan.total_s);
            }
            let (v, _) = d.sample(0.0).unwrap();
            assert_eq!(
                v["toolhead"]["extruder"],
                if active == 0 {
                    "extruder".to_owned()
                } else {
                    format!("extruder{active}")
                }
            );
            let at = d.tool_positions();
            let head = [
                v["toolhead"]["position"][0].as_f64().unwrap(),
                v["toolhead"]["position"][1].as_f64().unwrap(),
                v["toolhead"]["position"][2].as_f64().unwrap(),
            ];
            assert_eq!(at[active], head, "the active tool is at the head");
            let idle = 1 - active;
            assert_eq!(
                at[idle],
                [if idle == 0 { 0.0 } else { 300.0 }, head[1], head[2]],
                "the idle carriage waits at its end of the beam"
            );
        }
        // The profile's tools place each tool's group: the active one follows the head, the idle one is at its park pose.
        let profile = json!({"axes":{"x":[-150,150],"y":[-150,150],"z":[0,300]},
            "motions":{"t0":["x","y","z"],"t1":["x","y","z"]},
            "tools":[{"group":"t0","park":{"x":-150}},{"group":"t1","park":{"x":150}}]});
        let cal = printer_models::Calibration {
            offset_mm: [-150.0, -150.0, 0.0],
            sign: [1.0; 3],
            source: [0, 1, 2],
        };
        let p = printer_models::pose_for_tool(
            &profile,
            &cal,
            [Some(200.0), Some(120.0), Some(40.0)],
            Some(1),
        );
        assert_eq!(
            p["groups"]["t1"],
            json!([50.0, -30.0, 40.0]),
            "the active tool follows the head"
        );
        assert_eq!(
            p["groups"]["t0"],
            json!([-150.0, -30.0, 40.0]),
            "the idle tool is at its dock"
        );
        let p = printer_models::pose_for_tool(
            &profile,
            &cal,
            [Some(200.0), Some(120.0), Some(40.0)],
            Some(0),
        );
        assert_eq!(p["groups"]["t0"], json!([50.0, -30.0, 40.0]));
        assert_eq!(p["groups"]["t1"], json!([150.0, -30.0, 40.0]));
        assert_eq!(p["tool"], 0);
    }

    #[test]
    fn a_toolchanger_names_its_tools_and_every_idle_tool_sits_in_its_dock() {
        let docks = vec![
            [50.0, 360.0, 20.0],
            [100.0, 360.0, 20.0],
            [150.0, 360.0, 20.0],
        ];
        let spec = Spec {
            tools: Tools::Changer(docks.clone()),
            ..Spec::catalogue("voron_trident_300")
        };
        assert_eq!(spec.tools.count(), 3);
        let m = Arc::new(Machine::new(spec, Some(0.4), false, None));
        let mut d = Demo { m };
        let (v, _) = d.sample(1000.0).unwrap();
        assert_eq!(v["toolchanger"]["tool_numbers"], json!([0, 1, 2]));
        let tool = v["toolchanger"]["tool_number"].as_u64().unwrap() as usize;
        assert_eq!(
            v["toolhead"]["extruder"],
            if tool == 0 {
                "extruder".to_owned()
            } else {
                format!("extruder{tool}")
            }
        );
        let at = d.tool_positions();
        for (t, dock) in docks.iter().enumerate() {
            if t != tool {
                assert_eq!(at[t], *dock, "tool {t} waits in its dock");
            }
        }
        assert_eq!(at[tool][2], v["toolhead"]["position"][2].as_f64().unwrap());
        // A printer with one tool says nothing about a changer.
        let mut one = Demo::open(&machine_cfg(
            "one-tool",
            "voron_trident_300",
            Some(0.4),
            false,
        ));
        assert!(one.sample(1000.0).unwrap().0.get("toolchanger").is_none());
    }

    #[test]
    fn the_head_never_moves_faster_than_its_machine_can() {
        // The vase was sliced for a quick printer: its travel moves play at about 290 mm/s. An Ender-3 (200 mm/s)
        // shows no more than its limit; a Voron (500 mm/s) shows the pace as sliced.
        let fastest = |cad: &str| -> f64 {
            let mut d = Demo::open(&machine_cfg(&format!("speed-{cad}"), cad, Some(0.5), false));
            (0..300)
                .map(|i| {
                    let (v, _) = d.sample(1000.0 + f64::from(i) * 0.7).unwrap();
                    v["motion_report"]["live_velocity"].as_f64().unwrap()
                })
                .fold(0.0, f64::max)
        };
        let ender = fastest("ender3_stock_220");
        assert!(ender <= 200.0 + 1e-9, "{ender}");
        assert!(ender > 100.0, "the Ender still moves at pace: {ender}");
        assert!(
            fastest("voron_v24_350_printed") > 200.0,
            "the Voron is not held to the Ender's limit"
        );
    }

    #[test]
    fn a_brand_is_named_once_however_often_a_printer_is_opened() {
        let a = Spec::catalogue("snakeoil_xy_180").brand;
        assert_eq!(a, "SnakeOil");
        assert!(
            std::ptr::eq(a, Spec::catalogue("snakeoil_xy_180").brand),
            "the same name, not a new one each time"
        );
        assert_eq!(Spec::catalogue("voron_trident_300").brand, "Voron");
    }

    #[test]
    fn the_first_two_demos_are_what_they_were_without_a_catalogue_id() {
        // A config written before WO-02 names only a model; it must still be a Voron 2.4 350 or a CORE One L.
        let mut d = Demo::open(&cfg("legacy-a", "Voron 2.4 350", None));
        assert_eq!(
            (d.brand(), d.model_hint().as_deref(), d.bed_hint()),
            ("Voron", Some("Voron 2.4 350"), Some([350.0, 350.0]))
        );
        let mut d = Demo::open(&cfg("legacy-b", "CORE One L", None));
        assert_eq!(
            (d.brand(), d.model_hint().as_deref(), d.bed_hint()),
            ("Prusa", Some("CORE One L"), Some([300.0, 300.0]))
        );
    }
}

#[cfg(test)]
mod toolchanger_tests {
    use super::*;

    fn bank_entry() -> PrinterEntry {
        demo_farm()
            .into_iter()
            .find(|e| e.id == "demo-toolchanger")
            .expect("the toolchanger is in the demo farm")
    }

    fn config_of(e: &PrinterEntry) -> Config {
        let mut c = Config::default();
        c.printer_id = e.id.clone();
        c.database = format!("/tmp/stella-toolchanger-test-{}-{}.sqlite3", e.id, std::process::id());
        c.cad_model = e.cad_model.clone();
        c.cad_options = e.cad_options.clone();
        c.demo_start_at = e.demo_start_at;
        c
    }

    /// Polish WO-03: the toolchanger demo printer is a Voron 2.4 300 with a StealthChanger bank of three docks. It has three
    /// tools, each waiting at the dock the bank puts it at, plays the two-colour sample, and says which tool is active the
    /// way the Klipper toolchanger module does.
    #[test]
    fn the_toolchanger_demo_has_three_tools_each_at_the_dock_the_bank_puts_it_at() {
        let e = bank_entry();
        let c = config_of(&e);
        let spec = Spec::of(&c);
        let tools = printer_models::bank_tools("voron_v24_300_printed", c.cad_options.as_ref().unwrap()).unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(spec.tools, Tools::Changer(tools.iter().map(|t| t.dock).collect()));
        assert_eq!((spec.tools.count(), spec.sample_kind(), spec.bed), (3, Kind::Duo, [300.0, 300.0]));
        // Every dock is a place the head can stand: inside the bed's width and the machine's height, in front of the
        // middle of the bed (the docks hang off the front of the frame, 190 to 215 mm in front of the plate's corner...).
        for (i, t) in tools.iter().enumerate() {
            assert!(t.dock[0] > 0.0 && t.dock[0] < 300.0, "dock {i}: {:?}", t.dock);
            assert!(t.dock[1] >= 0.0 && t.dock[1] < 150.0, "dock {i} is toward the front: {:?}", t.dock);
            assert!(t.dock[2] > 100.0 && t.dock[2] <= spec.max_z, "dock {i} is up high: {:?}", t.dock);
        }
        assert!(
            tools[0].dock[0] < tools[1].dock[0] && tools[1].dock[0] < tools[2].dock[0],
            "the docks run left to right"
        );
        // 90 mm apart (the bank's pitch), give or take the few hundredths between one tool and the next.
        assert!((tools[1].dock[0] - tools[0].dock[0] - 90.0).abs() < 0.5);
        // The printer reports its tools like the toolchanger module and plays the duo file.
        let mut d = Demo::open(&c);
        let (v, _) = d.sample(1000.0).unwrap();
        assert_eq!(v["toolchanger"]["tool_numbers"], json!([0, 1, 2]));
        assert!(v["print_stats"]["filename"].as_str().unwrap().contains("duo"));
        // The two idle tools wait in their docks.
        let at = d.tool_positions();
        let tool = v["toolchanger"]["tool_number"].as_u64().unwrap() as usize;
        for (t, tl) in tools.iter().enumerate() {
            if t != tool {
                assert_eq!(at[t], tl.dock, "tool {t} waits in its dock");
            }
        }
    }

    /// Opus supplement S1: the demo farm asks for no non-commercial file. Every machine is open (the CORE One L is Prusa's own,
    /// added by hand) and every file of the toolchanger's selection (docks, shuttle, backplates, fasteners and the JabberWocky
    /// head) is GPL or CC BY-SA, which a build may carry with credit; Xol, the bank's non-commercial tool, is not in it.
    #[test]
    fn the_demo_farm_needs_no_file_that_is_not_open() {
        let none = printer_models::Library::new(std::env::temp_dir().join(format!("stella-demo-open-{}", std::process::id())));
        let mut bank_files = 0;
        for e in demo_farm() {
            let cad = e.cad_model.as_deref().unwrap();
            let class = printer_models::machine(cad).unwrap()["licence"]["class"].as_str().unwrap().to_owned();
            assert!(printer_models::may_bundle(&class) || cad == "prusa_core_one_l", "{cad}: {class}");
            let Some(sel) = &e.cad_options else { continue };
            let scene = printer_models::scene_swapping(cad, sel, &none);
            for pl in scene["placements"].as_array().unwrap() {
                let id = pl["part"].as_str().unwrap();
                let c = printer_models::part(id).unwrap()["licence"]["class"].as_str().unwrap().to_owned();
                assert!(printer_models::may_bundle(&c), "{id} is {c}: the demo farm would need a licence prompt");
                assert_ne!(id, "xol", "the non-commercial tool is not in the demo");
                bank_files += 1;
            }
        }
        assert!(bank_files >= 10, "the toolchanger has docks, backplates and a shuttle: {bank_files}");
    }

    /// The tool the file asks for changes while the part grows, and the third never comes out of its dock (the sample
    /// only ever calls T0 and T1).
    #[test]
    fn the_file_changes_the_tool_and_the_third_stays_docked() {
        let c = config_of(&bank_entry());
        let (_, _, plan) = for_bed(Spec::of(&c).sample_kind(), Spec::of(&c).bed);
        let seen: std::collections::BTreeSet<usize> = plan
            .sets
            .iter()
            .filter_map(|(_, s)| if let Set::Tool(n) = s { Some(*n) } else { None })
            .collect();
        assert_eq!(seen, [0, 1].into_iter().collect(), "the duo file calls T0 and T1 only");
        let changes = plan.sets.iter().filter(|(_, s)| matches!(s, Set::Tool(_))).count();
        assert!(changes > 100, "many changes over one print: {changes}");
        // Walk the print: the active tool takes both values, never the third.
        let mut d = Demo::open(&config_of(&PrinterEntry { id: "toolchanger-walk".into(), ..bank_entry() }));
        let mut tools_seen = std::collections::BTreeSet::new();
        for i in 0..400 {
            let (v, _) = d.sample(1000.0 + f64::from(i) * 10.0).unwrap();
            tools_seen.insert(v["toolchanger"]["tool_number"].as_u64().unwrap());
        }
        assert!(tools_seen.contains(&0) && tools_seen.contains(&1), "{tools_seen:?}");
        assert!(!tools_seen.contains(&2));
    }

    /// F03 item 5: the SnakeOil XY IDEX (the one IDEX machine of the catalogue whose file carries motion) is a demo printer of two tools that plays the two-colour sample:
    /// its tool changes at every T0 and T1 of the file, and each tool's carriage waits at its own end of the beam while the other prints. It is a mixed licence and so is
    /// never one of the demo farm's eight.
    #[test]
    fn an_idex_machine_is_a_demo_of_two_tools_playing_the_duo_and_is_not_in_the_farm() {
        let spec = Spec::catalogue("snakeoil_xy_idex");
        assert_eq!((spec.tools.clone(), spec.bed, spec.frame), (Tools::Idex, [250.0, 250.0], Frame::BedZ));
        assert!(matches!(spec.sample_kind(), Kind::Duo));
        // The idle tool waits at its end of the beam: X 0 for the first, the bed's far edge for the second.
        assert_eq!(spec.park(0, [100.0, 50.0, 10.0]), [0.0, 50.0, 10.0]);
        assert_eq!(spec.park(1, [100.0, 50.0, 10.0]), [250.0, 50.0, 10.0]);
        assert!(!demo_farm().iter().any(|e| e.cad_model.as_deref() == Some("snakeoil_xy_idex")), "a mixed licence is never asked for on the one press");
        assert!(!printer_models::may_bundle(printer_models::machine("snakeoil_xy_idex").unwrap()["licence"]["class"].as_str().unwrap()));
    }
}
