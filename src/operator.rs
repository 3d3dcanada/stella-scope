//! L5 presets and L8 console history: the two pieces of operator state that belong to Stella
//! Scope rather than to the printer. Both live in the same `<database>.commands.sqlite3` sidecar
//! as the command audit, so the recording itself stays a pure read-only capture of the machine.

use crate::{store::db_error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::time::Duration;

/// Klipper refuses above this itself; the same ceiling as `set_heater`.
const NOZZLE_MAX_C: f64 = 290.0;
const BED_MAX_C: f64 = 120.0;
/// The jog rail tops out at 50 mm (`JOG_MAX_MM`); a step preset may not exceed what jog allows.
const STEP_MAX_MM: f64 = 50.0;
/// Board contract: the console keeps the operator's last 200 entries and no more.
pub const HISTORY_MAX: usize = 200;
const LINE_MAX_CHARS: usize = 256;

pub struct Operator {
    path: String,
}

impl Operator {
    pub fn new(database: &str) -> Self {
        Self {
            path: format!("{database}.commands.sqlite3"),
        }
    }

    fn open(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path).map_err(db_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(db_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
                 CREATE TABLE IF NOT EXISTS presets (kind TEXT PRIMARY KEY, payload TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS console_history (
                   row_id INTEGER PRIMARY KEY,
                   at REAL NOT NULL,
                   line TEXT NOT NULL
                 );",
            )
            .map_err(db_error)?;
        Ok(connection)
    }

    /// Presets as stored, seeding the defaults the first time anything asks. Seeding is an
    /// INSERT OR IGNORE per kind, so an operator who empties one list keeps it empty.
    pub fn presets(&self) -> Result<Value> {
        let connection = self.open()?;
        for (kind, payload) in seed() {
            connection
                .execute(
                    "INSERT OR IGNORE INTO presets(kind,payload) VALUES(?1,?2)",
                    params![kind, payload.to_string()],
                )
                .map_err(db_error)?;
        }
        // WO-20 (2026-09-23), the owner: "Preheat numbers should be set up so they're proper." A store
        // still holding the first seed exactly as shipped (PLA 215, PETG 240, ABS, ASA, TPU) moves
        // once: PLA and PETG take his measured profiles in place, and ABS, ASA and TPU stay as they
        // are (reviewer's decision, same day: the owner prints ASA, so a saved preset is never dropped).
        // The move is recorded, so a store that has moved, or that anyone has saved, is never
        // rewritten again.
        let moved: Option<String> = connection
            .query_row(
                "SELECT payload FROM presets WHERE kind='seed_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if moved.is_none() {
            let transaction = connection.unchecked_transaction().map_err(db_error)?;
            let stored: String = transaction
                .query_row(
                    "SELECT payload FROM presets WHERE kind='materials'",
                    [],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            // Every name stays, so a loaded material (kept by name) stays valid through the move.
            if serde_json::from_str::<Value>(&stored).ok() == Some(first_seed()) {
                transaction
                    .execute(
                        "UPDATE presets SET payload=?1 WHERE kind='materials'",
                        params![ken_materials().to_string()],
                    )
                    .map_err(db_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO presets(kind,payload) VALUES('seed_version','2')",
                    [],
                )
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
        }
        // rc9 (2026-10-23): a store made by an older version holds the empty lists it was seeded with. Once, each list
        // that is still empty is given the standard one; a list anyone filled is never touched, and a list emptied after
        // this is kept empty.
        let filled: bool = connection
            .query_row("SELECT 1 FROM presets WHERE kind='standard_lists'", [], |_| Ok(true))
            .optional()
            .map_err(db_error)?
            .is_some();
        if !filled {
            let transaction = connection.unchecked_transaction().map_err(db_error)?;
            for (kind, standard) in [("materials", standard_materials()), ("sheets", standard_sheets()), ("maintenance", standard_maintenance())] {
                transaction
                    .execute(
                        "UPDATE presets SET payload=?2 WHERE kind=?1 AND payload='[]'",
                        params![kind, standard.to_string()],
                    )
                    .map_err(db_error)?;
            }
            transaction
                .execute("INSERT INTO presets(kind,payload) VALUES('standard_lists','1')", [])
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
        }
        let mut out = serde_json::Map::new();
        for (kind, _) in seed() {
            let stored: String = connection
                .query_row(
                    "SELECT payload FROM presets WHERE kind=?1",
                    params![kind],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            out.insert(
                kind.to_owned(),
                serde_json::from_str(&stored).map_err(|e| {
                    format!("stored {kind} presets are not valid JSON: {e}; PUT /api/presets to reset them")
                })?,
            );
        }
        Ok(Value::Object(out))
    }

    /// Replace every list at once. Validated before a single row is written, so a bad `fan` list
    /// cannot leave `materials` half-updated.
    pub fn put_presets(&self, incoming: &Value) -> Result<Value> {
        let checked = validate(incoming)?;
        // Read (and so seed, and move) first: a save must never be followed by a seed move.
        self.presets()?;
        let connection = self.open()?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        for (kind, payload) in &checked {
            transaction
                .execute(
                    "INSERT INTO presets(kind,payload) VALUES(?1,?2)
                     ON CONFLICT(kind) DO UPDATE SET payload=excluded.payload",
                    params![kind, payload.to_string()],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        self.presets()
    }

    pub fn history(&self) -> Result<Vec<Value>> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT at,line FROM console_history ORDER BY row_id DESC LIMIT ?1")
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![HISTORY_MAX as i64], |row| {
                Ok(json!({"at":row.get::<_,f64>(0)?,"line":row.get::<_,String>(1)?}))
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        Ok(rows)
    }

    /// Append one operator entry and prune to the newest `HISTORY_MAX`. The prune runs on every
    /// append, not on a timer: a cap that only applies on the next read is not a cap.
    pub fn push_history(&self, at: f64, line: &str) -> Result<Vec<Value>> {
        let line = line.trim();
        if line.is_empty() {
            return Err("console history entries cannot be blank".into());
        }
        if line.chars().count() > LINE_MAX_CHARS {
            return Err(format!(
                "console history entries are limited to {LINE_MAX_CHARS} characters"
            ));
        }
        if line.contains(['\n', '\r']) {
            return Err("console history stores one line at a time".into());
        }
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO console_history(at,line) VALUES(?1,?2)",
                params![at, line],
            )
            .map_err(db_error)?;
        connection
            .execute(
                "DELETE FROM console_history WHERE row_id NOT IN
                 (SELECT row_id FROM console_history ORDER BY row_id DESC LIMIT ?1)",
                params![HISTORY_MAX as i64],
            )
            .map_err(db_error)?;
        drop(connection);
        self.history()
    }
}

/// The standard lists a fresh installation starts with (2026-10-23: "Fresh install should have everything done properly.
/// All things listed, you should be able to choose what to add, what to take away."). Every entry is editable and removable;
/// `GET /api/presets/standard` serves the same lists so an editor can add back anything taken away. Temperatures are common
/// starting points from the materials' own ranges, for the person to tune. No PA-CF (our standing rule).
pub fn standard_materials() -> Value {
    json!([
        {"name":"PLA","nozzle":210,"bed":60},
        {"name":"PLA+","nozzle":220,"bed":60},
        {"name":"Silk PLA","nozzle":220,"bed":60},
        {"name":"Matte PLA","nozzle":215,"bed":60},
        {"name":"PLA-CF","nozzle":220,"bed":60},
        {"name":"PETG","nozzle":240,"bed":80},
        {"name":"PETG-CF","nozzle":250,"bed":80},
        {"name":"PCTG","nozzle":250,"bed":80},
        {"name":"ABS","nozzle":250,"bed":100},
        {"name":"ASA","nozzle":260,"bed":100},
        {"name":"ASA-CF","nozzle":265,"bed":100},
        {"name":"TPU 95A","nozzle":225,"bed":45},
        {"name":"TPU 85A","nozzle":220,"bed":40},
        {"name":"PC","nozzle":270,"bed":110},
        {"name":"PC-ABS","nozzle":260,"bed":105},
        {"name":"Nylon PA12","nozzle":255,"bed":80},
        {"name":"HIPS","nozzle":235,"bed":100},
        {"name":"PVA","nozzle":200,"bed":60},
        {"name":"PVB","nozzle":215,"bed":60},
        {"name":"PP","nozzle":235,"bed":85}
    ])
}
/// Common build plates. Their Z offsets are not known until measured, so each starts `measured: false` and the page never
/// offers to apply an offset nobody set.
pub fn standard_sheets() -> Value {
    json!([
        {"name":"Smooth PEI","z_offset":0.0,"measured":false},
        {"name":"Textured PEI","z_offset":0.0,"measured":false},
        {"name":"Cool plate","z_offset":0.0,"measured":false},
        {"name":"G10 garolite","z_offset":0.0,"measured":false},
        {"name":"Glass","z_offset":0.0,"measured":false}
    ])
}
/// The common jobs by print hours. `last_h` null means "start counting when the page first knows the hours".
pub fn standard_maintenance() -> Value {
    let jobs = [
        ("Clean the nozzle and check for plastic build-up", 50),
        ("Clean the build sheet (IPA or dish soap)", 20),
        ("Check belt tension", 100),
        ("Lube the linear rails", 200),
        ("Clean the extruder gears", 250),
        ("Check the Z drive (belts, leadscrews or pulleys)", 300),
        ("Check frame and gantry bolts", 300),
        ("Check the cables and cable chains for wear", 300),
        ("Replace the nozzle", 500),
    ];
    Value::Array(jobs.iter().map(|(n, h)| json!({"name":n,"every_h":h,"last_h":null,"last_at":null,"note":null})).collect())
}
fn seed_materials() -> Value {
    standard_materials()
}

/// Preserve the old the owner-specific migration for existing stores that still have the exact first seed.
fn ken_materials() -> Value {
    // Beds are our purple Conweb plate standard (2026-09-28): PETG 55 first layer, PLA 40. The 80/60
    // this seed carried before is what he had to correct by hand through PUT /api/presets.
    json!([
        {"name":"PLA","nozzle":218,"bed":40},
        {"name":"PETG","nozzle":250,"bed":55},
        {"name":"ABS","nozzle":255,"bed":100},
        {"name":"ASA","nozzle":260,"bed":100},
        {"name":"TPU","nozzle":230,"bed":50}
    ])
}

/// The first seed, as it shipped before WO-20. Only a store still holding exactly this is moved.
fn first_seed() -> Value {
    json!([
        {"name":"PLA","nozzle":215,"bed":60},
        {"name":"PETG","nozzle":240,"bed":80},
        {"name":"ABS","nozzle":255,"bed":100},
        {"name":"ASA","nozzle":260,"bed":100},
        {"name":"TPU","nozzle":230,"bed":50}
    ])
}

fn seed() -> [(&'static str, Value); 10] {
    [
        ("materials", seed_materials()),
        // Jog steps in millimetres, and fan speeds in percent: the rails Fable draws.
        ("steps", json!([0.1, 1, 10, 50])),
        ("fan", json!([0, 25, 50, 75, 100])),
        // What is in the extruder right now, by material name, with when it was loaded. Null until
        // the operator loads something; the filament panel heats to this material's nozzle preset.
        ("loaded", Value::Null),
        // Measured hotend max volumetric flow in mm³/s (Stella/docs/10, method A). Null until the owner
        // measures it; the HUD's speed-to-flow arithmetic judges nothing without it.
        ("hotend_max_flow", Value::Null),
        // NEXT review 2026-09-24 (the owner: "flex sheet loaded"): the build sheets he owns, each with the
        // Z offset it prints at, and which one is on the bed. Klipper cannot know which sheet is
        // fitted, so this is the operator's record; empty until he adds one.
        ("sheets", standard_sheets()),
        ("sheet", Value::Null),
        // Maintenance reminders (2026-09-24): each task, how many print hours apart, and the
        // print-hour count and date it was last done. Empty until he adopts a schedule.
        ("maintenance", standard_maintenance()),
        // 2026-09-25 ("a mesh history ... this one's a new build plate"): each time a plate is
        // marked as on the bed, oldest first, so a recorded mesh can say which plate it was taken on.
        ("sheet_log", json!([])),
        // Calibration records (2026-10-26): what each calibration found and when, by its id (pid_nozzle, z_offset, pressure_advance...).
        // Empty until one is run; the Calibrate page fills it.
        ("calibrations", json!({})),
    ]
}

/// Every list is checked before anything is stored. An unknown key is a refusal, not a silent drop.
fn validate(incoming: &Value) -> Result<Vec<(&'static str, Value)>> {
    let object = incoming
        .as_object()
        .ok_or("presets must be an object with materials, steps and fan")?;
    for key in object.keys() {
        if ![
            "materials",
            "steps",
            "fan",
            "loaded",
            "hotend_max_flow",
            "sheets",
            "sheet",
            "maintenance",
            "sheet_log",
            "calibrations",
        ]
        .contains(&key.as_str())
        {
            return Err(format!(
                "{key} is not a preset list; send materials, steps, fan, loaded, hotend_max_flow, sheets, sheet, maintenance, sheet_log and calibrations only"
            ));
        }
    }
    let list = |key: &str| -> Result<&Vec<Value>> {
        object
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("presets.{key} must be an array"))
    };
    let materials = list("materials")?;
    // The presets PUT carries a larger body than a command (PRESETS_MAX_BODY, 16 KiB, server.rs): a worst-case material row is
    // 57 bytes, so 40 rows, the sheets, the maintenance jobs and the sheet log stay well inside it.
    if materials.len() > 40 {
        return Err("at most 40 material presets".into());
    }
    let mut checked_materials: Vec<Value> = Vec::with_capacity(materials.len());
    for entry in materials {
        let name = entry["name"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.len() <= 24)
            .ok_or("every material preset needs a name of 1 to 24 characters")?;
        let number = |field: &str, max: f64| -> Result<f64> {
            let value = entry[field]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("{name} preset needs a numeric {field}"))?;
            if !(0.0..=max).contains(&value) {
                return Err(format!("{name} {field} must be 0 to {max:.0} °C"));
            }
            Ok(value)
        };
        // WO-20: Cool down is the HUD's own fixed entry, and a name must say which preset it is.
        if name.eq_ignore_ascii_case("cool down") {
            return Err(
                "Cool down is built in and always there; give this preset another name".into(),
            );
        }
        if checked_materials.iter().any(|m| {
            m["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        }) {
            return Err(format!(
                "two presets are named {name}; each name must be different"
            ));
        }
        let nozzle = number("nozzle", NOZZLE_MAX_C)?;
        let bed = number("bed", BED_MAX_C)?;
        // Material profiles (2026-10-26): what this material costs and weighs, and what Calibrate measured for it. Each is optional;
        // a value given is checked, an absent one stays absent (never a made-up default).
        let optional = |field: &str, lo: f64, hi: f64, what: &str| -> Result<Option<f64>> {
            match entry.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|x| x.is_finite() && (lo..=hi).contains(x))
                    .map(Some)
                    .ok_or_else(|| format!("{name} {what} must be {lo} to {hi}")),
            }
        };
        let mut m = json!({"name":name,"nozzle":nozzle,"bed":bed});
        for (field, lo, hi, what) in [
            ("price_kg", 0.0, 1000.0, "price per kg"),
            ("density", 0.5, 3.0, "density (g/cm³)"),
            ("pa", 0.0, 1.0, "pressure advance"),
            ("flow", 0.5, 1.5, "flow ratio"),
            ("fan", 0.0, 100.0, "part fan %"),
        ] {
            if let Some(v) = optional(field, lo, hi, what)? {
                m[field] = json!(v);
            }
        }
        checked_materials.push(m);
    }
    let numbers = |key: &str, max: f64| -> Result<Value> {
        let values = list(key)?;
        if values.len() > 12 {
            return Err(format!("at most 12 {key} presets"));
        }
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            let n = value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("presets.{key} must contain numbers"))?;
            if !(0.0..=max).contains(&n) {
                return Err(format!("presets.{key} values must be 0 to {max:.0}"));
            }
            out.push(json!(n));
        }
        Ok(Value::Array(out))
    };
    // `loaded` is optional: absent or null clears it; otherwise it must name a listed material.
    let loaded = match object.get("loaded") {
        None | Some(Value::Null) => Value::Null,
        Some(entry) => {
            let name = entry["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or("presets.loaded needs the name of a listed material")?;
            if !checked_materials.iter().any(|m| m["name"] == name) {
                return Err(format!("{name} is not one of the material presets"));
            }
            let at = entry["at"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(0.0);
            json!({"name":name,"at":at})
        }
    };
    let hotend_max_flow = match object.get("hotend_max_flow") {
        None | Some(Value::Null) => Value::Null,
        Some(value) => {
            let flow = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("presets.hotend_max_flow must be a number of mm³/s or null")?;
            if !(1.0..=80.0).contains(&flow) {
                return Err("presets.hotend_max_flow must be 1 to 80 mm³/s".into());
            }
            json!(flow)
        }
    };
    let mut out = vec![
        ("materials", Value::Array(checked_materials)),
        ("steps", numbers("steps", STEP_MAX_MM)?),
        ("fan", numbers("fan", 100.0)?),
        ("loaded", loaded),
        ("hotend_max_flow", hotend_max_flow),
    ];
    // Sheets travel together with the sheet on the bed. A save that sends neither (the classic
    // HUD's presets editor) leaves both as stored.
    match (object.get("sheets"), object.get("sheet")) {
        (None, None) => {}
        (None, Some(_)) => return Err("presets.sheet needs presets.sheets beside it".into()),
        (Some(sheets), fitted) => {
            let sheets = sheets.as_array().ok_or("presets.sheets must be an array")?;
            if sheets.len() > 8 {
                return Err("at most 8 build sheets".into());
            }
            let mut checked = Vec::with_capacity(sheets.len());
            for entry in sheets {
                let name = entry["name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|n| !n.is_empty() && n.len() <= 24)
                    .ok_or("every build sheet needs a name of 1 to 24 characters")?;
                if checked.iter().any(|m: &Value| {
                    m["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                }) {
                    return Err(format!(
                        "two sheets are named {name}; each name must be different"
                    ));
                }
                let z = entry["z_offset"]
                    .as_f64()
                    .filter(|v| v.is_finite() && (-2.0..=2.0).contains(v))
                    .ok_or_else(|| format!("{name} needs a Z offset from -2 to 2 mm"))?;
                // measured: false is a standard plate whose offset nobody has set yet (absent = measured, as every older store)
                let measured = entry["measured"].as_bool().unwrap_or(true);
                checked.push(json!({"name":name,"z_offset":(z * 1000.0).round() / 1000.0,"measured":measured}));
            }
            let fitted = match fitted {
                None | Some(Value::Null) => Value::Null,
                Some(entry) => {
                    let name = entry["name"]
                        .as_str()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .ok_or("presets.sheet needs the name of a listed sheet")?;
                    if !checked.iter().any(|m| m["name"] == name) {
                        return Err(format!("{name} is not one of the build sheets"));
                    }
                    let at = entry["at"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .unwrap_or(0.0);
                    json!({"name":name,"at":at})
                }
            };
            out.push(("sheets", Value::Array(checked)));
            out.push(("sheet", fitted));
        }
    }
    if let Some(log) = object.get("sheet_log") {
        let log = log.as_array().ok_or("presets.sheet_log must be an array")?;
        let mut checked = Vec::with_capacity(log.len().min(200));
        for entry in log.iter().rev().take(200).rev() {
            let name = entry["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.len() <= 24)
                .ok_or("every plate record needs a name of 1 to 24 characters")?;
            let at = entry["at"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or("every plate record needs a time")?;
            checked.push(json!({"name":name,"at":at}));
        }
        out.push(("sheet_log", Value::Array(checked)));
    }
    // Maintenance tasks travel alone and are kept as stored when absent, like the sheets.
    if let Some(tasks) = object.get("maintenance") {
        let tasks = tasks
            .as_array()
            .ok_or("presets.maintenance must be an array")?;
        if tasks.len() > 24 {
            return Err("at most 24 maintenance tasks".into());
        }
        let mut checked = Vec::with_capacity(tasks.len());
        for t in tasks {
            let name = t["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.len() <= 60)
                .ok_or("every maintenance task needs a name of 1 to 60 characters")?;
            let every = t["every_h"]
                .as_f64()
                .filter(|v| v.is_finite() && (1.0..=10000.0).contains(v))
                .ok_or_else(|| format!("{name} needs a schedule of 1 to 10000 print hours"))?;
            let last_h = t["last_h"].as_f64().filter(|v| v.is_finite() && *v >= 0.0);
            let last_at = t["last_at"].as_f64().filter(|v| v.is_finite() && *v >= 0.0);
            let note = t["note"]
                .as_str()
                .map(|n| n.chars().take(200).collect::<String>());
            checked.push(
                json!({"name":name,"every_h":every,"last_h":last_h,"last_at":last_at,"note":note}),
            );
        }
        out.push(("maintenance", Value::Array(checked)));
    }
    // Calibration records travel alone and are kept as stored when absent. Each is the time it was done, its figures (numbers or short
    // words) and a note; nothing else is stored, and a record that is not that shape is refused.
    if let Some(cal) = object.get("calibrations") {
        let cal = cal.as_object().ok_or("presets.calibrations must be an object")?;
        if cal.len() > 40 {
            return Err("at most 40 calibration records".into());
        }
        let mut checked = serde_json::Map::new();
        for (id, rec) in cal {
            if id.is_empty() || id.len() > 40 || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                return Err(format!("{id} is not a calibration name"));
            }
            let at = rec["at"]
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.0)
                .ok_or_else(|| format!("calibration {id} needs the time it was done"))?;
            let mut values = serde_json::Map::new();
            if let Some(v) = rec.get("values") {
                let v = v.as_object().ok_or_else(|| format!("calibration {id}: values must be an object"))?;
                if v.len() > 16 {
                    return Err(format!("calibration {id}: at most 16 values"));
                }
                for (k, x) in v {
                    if k.is_empty() || k.len() > 32 {
                        return Err(format!("calibration {id}: a value name of 1 to 32 characters"));
                    }
                    let ok = match x {
                        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
                        Value::String(t) => t.chars().count() <= 80,
                        Value::Bool(_) | Value::Null => true,
                        _ => false,
                    };
                    if !ok {
                        return Err(format!("calibration {id}: {k} must be a number or a short word"));
                    }
                    values.insert(k.clone(), x.clone());
                }
            }
            let note = rec["note"].as_str().map(|n| n.chars().take(300).collect::<String>());
            checked.insert(id.clone(), json!({"at":at,"values":values,"note":note}));
        }
        out.push(("calibrations", Value::Object(checked)));
    }
    Ok(out)
}

#[cfg(test)]
mod material_profile_tests {
    #[test]
    fn a_material_carries_its_profile_and_bad_values_are_refused() {
        let p = |m: serde_json::Value| super::validate(&serde_json::json!({"materials":[m],"steps":[1],"fan":[0]}));
        let out = p(serde_json::json!({"name":"PETG","nozzle":245,"bed":80,"price_kg":32.5,"density":1.27,"pa":0.045,"flow":0.97,"fan":40})).unwrap();
        let (_, mats) = out.iter().find(|(k, _)| *k == "materials").unwrap();
        assert_eq!(mats[0]["pa"], 0.045);
        assert_eq!(mats[0]["price_kg"], 32.5);
        let plain = p(serde_json::json!({"name":"PLA","nozzle":210,"bed":60})).unwrap();
        let (_, mats) = plain.iter().find(|(k, _)| *k == "materials").unwrap();
        assert!(mats[0].get("pa").is_none(), "an absent value stays absent");
        for bad in [serde_json::json!({"name":"X","nozzle":200,"bed":60,"pa":2}), serde_json::json!({"name":"X","nozzle":200,"bed":60,"density":9}), serde_json::json!({"name":"X","nozzle":200,"bed":60,"flow":"high"})] {
            assert!(p(bad.clone()).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
mod calibration_record_tests {
    #[test]
    fn calibration_records_are_checked_and_kept_when_absent() {
        let base = serde_json::json!({"materials":[{"name":"PETG","nozzle":250,"bed":80}],"steps":[1],"fan":[0]});
        let mut with = base.clone();
        with["calibrations"] = serde_json::json!({"pid_nozzle":{"at":1.7e9,"values":{"kp":22.8,"ki":1.29,"kd":101.2,"target":245},"note":"saved"}});
        let out = super::validate(&with).unwrap();
        let (_, c) = out.iter().find(|(k, _)| *k == "calibrations").unwrap();
        assert_eq!(c["pid_nozzle"]["values"]["kp"], 22.8);
        assert!(super::validate(&base).unwrap().iter().all(|(k, _)| *k != "calibrations"), "absent means kept as stored");
        for bad in [
            serde_json::json!({"Bad Name":{"at":1.0}}),
            serde_json::json!({"pid_nozzle":{"values":{}}}),
            serde_json::json!({"pid_nozzle":{"at":1.0,"values":{"kp":[1,2]}}}),
        ] {
            let mut b = base.clone();
            b["calibrations"] = bad.clone();
            assert!(super::validate(&b).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
mod sheet_tests {
    #[test]
    fn sheets_are_checked_and_kept_when_absent() {
        let base = serde_json::json!({"materials":[{"name":"PETG","nozzle":250,"bed":80}],"steps":[1],"fan":[0]});
        let mut with = base.clone();
        with["sheets"] = serde_json::json!([{"name":"Textured PEI","z_offset":-0.0504},{"name":"Smooth","z_offset":0.02}]);
        with["sheet"] = serde_json::json!({"name":"Smooth","at":5});
        let v = super::validate(&with).unwrap();
        assert!(v
            .iter()
            .any(|(k, x)| *k == "sheets" && x[0]["z_offset"] == -0.05));
        assert!(v
            .iter()
            .any(|(k, x)| *k == "sheet" && x["name"] == "Smooth"));
        assert!(
            !super::validate(&base)
                .unwrap()
                .iter()
                .any(|(k, _)| *k == "sheets"),
            "absent sheets are kept as stored"
        );
        let mut bad = with.clone();
        bad["sheet"] = serde_json::json!({"name":"Glass"});
        assert!(super::validate(&bad).is_err());
        bad = with.clone();
        bad["sheets"][0]["z_offset"] = serde_json::json!(3.0);
        assert!(super::validate(&bad).is_err());
        bad = base.clone();
        bad["sheet"] = serde_json::json!({"name":"Smooth"});
        assert!(super::validate(&bad).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn operator(label: &str) -> (Operator, String) {
        let database = format!(
            "/tmp/stella-operator-test-{}-{}-{label}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        (Operator::new(&database), database)
    }
    /// rc9 WO-01 (2026-10-23: "Fresh install should have everything"): a new store starts with the standard lists; an
    /// older store's still-empty lists are filled once; a list filled by the person is kept; a list emptied later stays empty.
    #[test]
    fn a_fresh_store_has_every_standard_list_and_an_old_empty_one_is_filled_once() {
        let (op, database) = operator("standard-lists");
        let p = op.presets().unwrap();
        assert_eq!(p["materials"].as_array().unwrap().len(), 20);
        assert!(!p["materials"].as_array().unwrap().iter().any(|m| m["name"] == "PA-CF"));
        assert_eq!(p["sheets"].as_array().unwrap().len(), 5);
        assert_eq!(p["sheets"][0]["measured"], false);
        assert_eq!(p["maintenance"].as_array().unwrap().len(), 9);
        // emptied by the person: stays empty
        let mut cleared = p.clone();
        cleared["maintenance"] = json!([]);
        op.put_presets(&cleared).unwrap();
        assert_eq!(op.presets().unwrap()["maintenance"], json!([]));
        cleanup(&database);
        // an older store: empty lists as it was seeded, the person's own materials kept
        let (op, database) = operator("standard-lists-old");
        let c = op.open().unwrap();
        c.execute_batch(&format!(
            "INSERT INTO presets(kind,payload) VALUES('materials','{}'),('sheets','[]'),('maintenance','[]'),('seed_version','2')",
            ken_materials()
        ))
        .unwrap();
        drop(c);
        let p = op.presets().unwrap();
        assert_eq!(p["materials"], ken_materials(), "his own five are kept");
        assert_eq!(p["sheets"].as_array().unwrap().len(), 5);
        assert_eq!(p["maintenance"].as_array().unwrap().len(), 9);
        // a sheet saved without the flag is a measured one
        let mut own = p.clone();
        own["sheets"] = json!([{"name":"Purple Conweb","z_offset":-0.02}]);
        op.put_presets(&own).unwrap();
        assert_eq!(op.presets().unwrap()["sheets"][0]["measured"], true);
        cleanup(&database);
    }
    fn cleanup(database: &str) {
        for suffix in [
            ".commands.sqlite3",
            ".commands.sqlite3-wal",
            ".commands.sqlite3-shm",
        ] {
            let _ = std::fs::remove_file(format!("{database}{suffix}"));
        }
    }

    #[test]
    fn presets_seed_once_and_survive_replacement() {
        let (operator, database) = operator("presets");
        let seeded = operator.presets().unwrap();
        assert_eq!(
            seeded["materials"],
            standard_materials(),
            "a fresh install starts with the standard list (2026-10-23)"
        );
        assert_eq!(seeded["steps"], json!([0.1, 1, 10, 50]));
        assert_eq!(seeded["fan"], json!([0, 25, 50, 75, 100]));
        // Reading twice does not duplicate the seed.
        assert_eq!(operator.presets().unwrap(), seeded);

        let replaced = operator
            .put_presets(&json!({
                "materials":[{"name":"PLA+","nozzle":220,"bed":62}],
                "steps":[0.05,5],
                "fan":[0,100]
            }))
            .unwrap();
        assert_eq!(replaced["materials"].as_array().unwrap().len(), 1);
        assert_eq!(replaced["materials"][0]["name"], "PLA+");
        assert_eq!(replaced["steps"], json!([0.05, 5.0]));
        assert_eq!(replaced["loaded"], Value::Null);
        assert_eq!(operator.presets().unwrap(), replaced);
        // The loaded material must be one of the listed presets; then it is kept with its time.
        let refused = operator.put_presets(&json!({
            "materials":[{"name":"PLA+","nozzle":220,"bed":62}],"steps":[1],"fan":[0],
            "loaded":{"name":"ABS","at":10.0}
        }));
        assert!(refused
            .unwrap_err()
            .contains("not one of the material presets"));
        let loaded = operator
            .put_presets(&json!({
                "materials":[{"name":"PLA+","nozzle":220,"bed":62}],"steps":[1],"fan":[0],
                "loaded":{"name":"PLA+","at":12.5}
            }))
            .unwrap();
        assert_eq!(loaded["loaded"], json!({"name":"PLA+","at":12.5}));
        assert_eq!(loaded["hotend_max_flow"], Value::Null);
        let flow = operator
            .put_presets(&json!({
                "materials":[{"name":"PLA+","nozzle":220,"bed":62}],"steps":[1],"fan":[0],
                "hotend_max_flow":21.5
            }))
            .unwrap();
        assert_eq!(flow["hotend_max_flow"], json!(21.5));
        assert!(operator
            .put_presets(&json!({"materials":[],"steps":[1],"fan":[0],"hotend_max_flow":200}))
            .unwrap_err()
            .contains("1 to 80"));
        cleanup(&database);
    }

    #[test]
    fn preset_bounds_refuse_and_change_nothing() {
        let (operator, database) = operator("preset-bounds");
        let before = operator.presets().unwrap();
        for bad in [
            json!({"materials":[{"name":"X","nozzle":290.1,"bed":60}],"steps":[1],"fan":[50]}),
            json!({"materials":[{"name":"X","nozzle":250,"bed":120.5}],"steps":[1],"fan":[50]}),
            json!({"materials":[{"name":"X","nozzle":-1,"bed":60}],"steps":[1],"fan":[50]}),
            json!({"materials":[{"name":"","nozzle":250,"bed":60}],"steps":[1],"fan":[50]}),
            json!({"materials":[{"nozzle":250,"bed":60}],"steps":[1],"fan":[50]}),
            json!({"materials":[],"steps":[51],"fan":[50]}),
            json!({"materials":[],"steps":[1],"fan":[101]}),
            json!({"materials":[],"steps":[1],"fan":[50],"extra":[]}),
            json!({"materials":[],"steps":[1]}),
            json!([]),
            // One past the material cap.
            json!({
                "materials":(0..41).map(|i|json!({"name":format!("M{i}"),"nozzle":215,"bed":60}))
                    .collect::<Vec<_>>(),
                "steps":[1],"fan":[50]
            }),
        ] {
            assert!(operator.put_presets(&bad).is_err(), "{bad} should refuse");
        }
        assert_eq!(operator.presets().unwrap(), before);
        cleanup(&database);
    }

    /// A store as the first seed left it: the five rows the old service wrote on first read, and no
    /// record of a seed move.
    fn old_store(database: &str, materials: Value, loaded: Value) {
        let connection = Connection::open(format!("{database}.commands.sqlite3")).unwrap();
        connection
            .execute_batch("CREATE TABLE presets (kind TEXT PRIMARY KEY, payload TEXT NOT NULL);")
            .unwrap();
        for (kind, payload) in [
            ("materials", materials),
            ("steps", json!([0.1, 1, 10, 50])),
            ("fan", json!([0, 25, 50, 75, 100])),
            ("loaded", loaded),
            ("hotend_max_flow", Value::Null),
        ] {
            connection
                .execute(
                    "INSERT INTO presets(kind,payload) VALUES(?1,?2)",
                    params![kind, payload.to_string()],
                )
                .unwrap();
        }
    }

    /// WO-20 (2026-09-23): our live store held the first seed untouched (read off it read-only on
    /// 2026-09-23). It moves once: PLA and PETG to his measured profiles in place, ABS, ASA and TPU
    /// exactly as they were. An edited store never moves, a loaded material stays loaded, and once
    /// moved, even saving the old five back is kept as saved.
    #[test]
    fn untouched_first_seed_moves_to_kens_profiles_once() {
        let (store, database) = operator("seed-move");
        old_store(&database, first_seed(), Value::Null);
        let moved = store.presets().unwrap();
        assert_eq!(moved["materials"], ken_materials());
        assert_eq!(
            moved["materials"],
            json!([
                {"name":"PLA","nozzle":218,"bed":40},
                {"name":"PETG","nozzle":250,"bed":55},
                {"name":"ABS","nozzle":255,"bed":100},
                {"name":"ASA","nozzle":260,"bed":100},
                {"name":"TPU","nozzle":230,"bed":50}
            ])
        );
        // Nothing is dropped: the three presets without a Stella profile are the first seed's own.
        assert_eq!(
            moved["materials"].as_array().unwrap()[2..],
            first_seed().as_array().unwrap()[2..]
        );
        assert_eq!(moved["steps"], json!([0.1, 1, 10, 50]));
        let kept = store
            .put_presets(&json!({"materials":first_seed(),"steps":[1],"fan":[0]}))
            .unwrap();
        // Saved values come back as numbers the validator wrote (215.0), so compare by value.
        let nozzles = |v: &Value| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|m| m["nozzle"].as_f64().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            nozzles(&kept["materials"]),
            vec![215.0, 240.0, 255.0, 260.0, 230.0]
        );
        assert_eq!(
            nozzles(&store.presets().unwrap()["materials"]),
            vec![215.0, 240.0, 255.0, 260.0, 230.0]
        );
        cleanup(&database);

        let (edited_store, database) = operator("seed-edited");
        let edited =
            json!([{"name":"PLA","nozzle":215,"bed":60},{"name":"PETG","nozzle":245,"bed":85}]);
        old_store(&database, edited.clone(), Value::Null);
        assert_eq!(edited_store.presets().unwrap()["materials"], edited);
        cleanup(&database);

        // One value away from the first seed (ASA bed 105) is an edit, and it is left alone.
        let (near_store, database) = operator("seed-near");
        let mut near = first_seed();
        near[3]["bed"] = json!(105);
        old_store(&database, near.clone(), Value::Null);
        assert_eq!(near_store.presets().unwrap()["materials"], near);
        cleanup(&database);

        // A loaded material is kept by name, and every name stays, so it moves and stays loaded.
        let (loaded_store, database) = operator("seed-loaded");
        old_store(&database, first_seed(), json!({"name":"ABS","at":5.0}));
        let loaded = loaded_store.presets().unwrap();
        assert_eq!(loaded["materials"], ken_materials());
        assert_eq!(loaded["loaded"], json!({"name":"ABS","at":5.0}));
        cleanup(&database);
    }

    #[test]
    fn preset_names_must_be_different_and_not_cool_down() {
        let (operator, database) = operator("preset-names");
        let before = operator.presets().unwrap();
        for (bad, says) in [
            (
                json!({"materials":[{"name":"PLA","nozzle":218,"bed":60},{"name":"pla","nozzle":220,"bed":60}],"steps":[1],"fan":[0]}),
                "two presets are named pla",
            ),
            (
                json!({"materials":[{"name":"Cool Down","nozzle":0,"bed":0}],"steps":[1],"fan":[0]}),
                "Cool down is built in",
            ),
            (
                json!({"materials":[{"name":"   ","nozzle":218,"bed":60}],"steps":[1],"fan":[0]}),
                "needs a name",
            ),
            (
                json!({"materials":[{"name":"PETG","nozzle":291,"bed":80}],"steps":[1],"fan":[0]}),
                "PETG nozzle must be 0 to 290 °C",
            ),
            (
                json!({"materials":[{"name":"PETG","nozzle":250,"bed":121}],"steps":[1],"fan":[0]}),
                "PETG bed must be 0 to 120 °C",
            ),
        ] {
            let error = operator.put_presets(&bad).unwrap_err();
            assert!(error.contains(says), "{bad}: {error}");
        }
        assert_eq!(operator.presets().unwrap(), before);
        cleanup(&database);
    }

    #[test]
    fn console_history_is_newest_first_and_capped_at_200() {
        let (operator, database) = operator("history");
        assert!(operator.history().unwrap().is_empty());
        for i in 0..(HISTORY_MAX + 25) {
            operator
                .push_history(1000.0 + i as f64, &format!("M115 {i}"))
                .unwrap();
        }
        let rows = operator.history().unwrap();
        assert_eq!(rows.len(), HISTORY_MAX);
        assert_eq!(rows[0]["line"], format!("M115 {}", HISTORY_MAX + 24));
        assert_eq!(rows[HISTORY_MAX - 1]["line"], "M115 25");
        assert_eq!(rows[0]["at"], 1000.0 + (HISTORY_MAX + 24) as f64);

        assert!(operator.push_history(1.0, "   ").is_err());
        assert!(operator.push_history(1.0, "M115\nM112").is_err());
        assert!(operator.push_history(1.0, &"X".repeat(257)).is_err());
        assert_eq!(operator.history().unwrap().len(), HISTORY_MAX);
        cleanup(&database);
    }
}
