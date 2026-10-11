use super::Response;
use crate::{
    config::Config,
    model::{self, CollectorStatus, MOTORS},
    store::{db_error, Store},
    Result,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Write, time::Instant};

const FLAGS: [&str; 12] = [
    "otpw", "ot", "s2vsa", "s2vsb", "s2ga", "s2gb", "ola", "olb", "t120", "t143", "t150", "t157",
];
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn nonnegative(v: &Value) -> Option<f64> {
    number(v).filter(|n| *n >= 0.0)
}
fn fraction(v: &Value) -> Option<f64> {
    number(v).filter(|n| (0.0..=1.0).contains(n))
}
fn boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => v.as_i64().filter(|n| [0, 1].contains(n)).map(|n| n == 1),
    }
}
fn text(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.chars().take(512).collect())
}
fn sparse(v: Option<&Value>) -> Option<bool> {
    match v {
        None => Some(false),
        Some(v) => boolean(v),
    }
}
fn motor(d: &Value, name: &str) -> Value {
    let driver = &d[format!("tmc2209 {name}")];
    let status = driver["drv_status"].as_object();
    let flags = status.and_then(|s| {
        let mut out = Vec::new();
        for k in FLAGS {
            if sparse(s.get(k))? {
                out.push(k);
            }
        }
        Some(out)
    });
    let scale = status.and_then(|s| match s.get("cs_actual") {
        None => Some(0),
        Some(v) => v.as_u64().filter(|n| *n <= 31),
    });
    json!({"current":nonnegative(&driver["run_current"]),"hold":nonnegative(&driver["hold_current"]),
 "scale":scale,"standstill":status.and_then(|s|sparse(s.get("stst"))),"flags":flags,
 "energised":boolean(&d["stepper_enable"]["steppers"][name])})
}
fn heater(v: &Value) -> Value {
    json!({"temperature":number(&v["temperature"]),"target":number(&v["target"]),"power":fraction(&v["power"])})
}
fn fan(v: &Value) -> Value {
    json!({"speed":fraction(&v["speed"]),"rpm":nonnegative(&v["rpm"])})
}
fn matrix(v: &Value) -> Option<Vec<Vec<Option<f64>>>> {
    let rows = v.as_array()?;
    if rows.is_empty() || rows.len() > 64 {
        return None;
    }
    let n = rows[0].as_array()?.len();
    if n == 0 || n > 64 {
        return None;
    }
    rows.iter()
        .map(|r| {
            let a = r.as_array()?;
            if a.len() != n {
                return None;
            }
            Some(a.iter().map(number).collect())
        })
        .collect()
}
fn point(ts: f64, d: &Value, details: bool) -> Value {
    let mut motors = serde_json::Map::new();
    for (n, _) in MOTORS {
        motors.insert(n.to_string(), motor(d, n));
    }
    let p = &d["motion_report"]["live_position"];
    let mut v = json!({"ts":ts,"position":[number(&p[0]),number(&p[1]),number(&p[2])],
 "commanded":[number(&d["toolhead"]["position"][0]),number(&d["toolhead"]["position"][1]),number(&d["toolhead"]["position"][2])],
 "velocity":nonnegative(&d["motion_report"]["live_velocity"]),"homed":text(&d["toolhead"]["homed_axes"]),
 // During a G28 the HUD drew the whole gantry above the frame. The recorded sample at 1790021428.895
 // has homed_axes "xyz" and BOTH live_position and toolhead.position at Z 464.02 against axis_maximum
 // 310 — Klipper's homing force position (position_endstop + 1.5 * (position_max -
 // position_endstop)), not a place the machine ever was. The model needs the machine's own travel to
 // tell that pose apart from a real one, so the limits ride on every point.
 "limits":{"min":[number(&d["toolhead"]["axis_minimum"][0]),number(&d["toolhead"]["axis_minimum"][1]),number(&d["toolhead"]["axis_minimum"][2])],
  "max":[number(&d["toolhead"]["axis_maximum"][0]),number(&d["toolhead"]["axis_maximum"][1]),number(&d["toolhead"]["axis_maximum"][2])]},
 "speed_factor":number(&d["gcode_move"]["speed_factor"]),"extrude_factor":number(&d["gcode_move"]["extrude_factor"]),"max_velocity":number(&d["toolhead"]["max_velocity"]),
 // The space beside X, Y and Z should carry live motion. The acceleration limit, the filament speed
 // (negative while retracting; the HUD turns it into mm³/s) and the speed the G-code asked for are
 // all in the recorded objects already. gcode_move.speed is the last G1 F in mm/min, before M220
 // (the cube print records 12000 while the head runs at 200 mm/s), so it rides here in mm/s.
 "max_accel":nonnegative(&d["toolhead"]["max_accel"]),"extruder_velocity":number(&d["motion_report"]["live_extruder_velocity"]),"requested_speed":nonnegative(&d["gcode_move"]["speed"]).map(|f| f / 60.0),
 // Z is fifteen times slower than XY on this machine (max_z_velocity 20 against max_velocity 300),
 // so the model's travel guard needs Z's own limit or a 441 mm homing jump is inside one tick's
 // reach. Klipper reports it in configfile.settings.printer, which the recorder already holds.
 "max_z_velocity":number(&d["configfile"]["settings"]["printer"]["max_z_velocity"]),
 // The done ramp marked parts of the cube finished before the head got there. The byte the
 // printer is actually reading is what maps onto the parser's own offsets; progress is a ratio
 // and loses the file size. Both ride on the point so the HUD never has to guess.
 "file_position":nonnegative(&d["virtual_sdcard"]["file_position"]),"file_size":nonnegative(&d["virtual_sdcard"]["file_size"]),
 "z_offset":number(&d["gcode_move"]["homing_origin"][2]),"nozzle":heater(&d["extruder"]),"bed":heater(&d["heater_bed"]),
 "fans":{"fan":fan(&d["fan"]),"hotend_fan":fan(&d["heater_fan hotend_fan"]),"controller_fan":fan(&d["controller_fan controller_fan"])},
 "probe":{"triggered":boolean(&d["probe"]["last_query"]),"last_z_result":number(&d["probe"]["last_z_result"])},
 "qgl_applied":boolean(&d["quad_gantry_level"]["applied"]),"motors":motors});
    if details {
        v["telemetry"] = super::signals::projection(d, ts);
        let raw = &d["bed_mesh"]["probed_matrix"];
        v["mesh"] = json!({"profile":text(&d["bed_mesh"]["profile_name"]),"grid":matrix(raw),"grid_supplied":raw.is_array(),"min":d["bed_mesh"]["mesh_min"].as_array().map(|a|a.iter().take(2).map(number).collect::<Vec<_>>()),"max":d["bed_mesh"]["mesh_max"].as_array().map(|a|a.iter().take(2).map(number).collect::<Vec<_>>())});
        v["job"] = job(d);
    }
    v
}
fn bounded_at(db: &Store, ts: f64, cap: usize) -> Result<Option<(f64, Value)>> {
    Ok(db.load_sample(ts, cap)?.map(|(time, data, _)| (time, data)))
}
fn context(db: &Store, ts: f64, kind: &str, cap: usize) -> Result<Option<Value>> {
    let row:Option<(usize,String)>=db.conn.query_row("SELECT length(CAST(message AS BLOB)),CASE WHEN length(CAST(message AS BLOB))<=?3 THEN message ELSE '' END FROM events WHERE kind=?1 AND ts<=?2 ORDER BY ts DESC,rowid DESC LIMIT 1",params![kind,ts,cap as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
    row.map(|(n, s)| {
        if n > cap {
            return Err(
                "recorded metadata exceeds the workspace cap; preserve and inspect the recording"
                    .to_owned(),
            );
        }
        serde_json::from_str(&s)
            .map_err(|e| format!("invalid {kind} metadata: {e}; restore a valid recording"))
    })
    .transpose()
}
fn geometry(d: &Value, g: Option<Value>, cfg: &Config) -> Value {
    let recorded = g.is_some();
    let g = g.unwrap_or_else(|| model::geometry(cfg));
    let settings = &d["configfile"]["settings"];
    let xy = [0, 1].map(|i| {
        let min = number(&d["toolhead"]["axis_minimum"][i])?;
        let max = number(&d["toolhead"]["axis_maximum"][i])?;
        let travel = max - min;
        (10.0..=2000.0).contains(&travel).then_some(travel)
    });
    let z = number(&d["toolhead"]["axis_maximum"][2])
        .or_else(|| number(&settings["stepper_z"]["position_max"]))
        .filter(|n| (10.0..=2000.0).contains(n));
    let configured_bed = g["bed_size_mm"]
        .as_array()
        .filter(|a| {
            a.len() == 2
                && a.iter()
                    .all(|v| number(v).is_some_and(|n| (10.0..=2000.0).contains(&n)))
        })
        .cloned()
        .map(Value::Array)
        .unwrap_or_else(|| json!(cfg.bed_size_mm));
    let (bed, bed_source) = if let (Some(x), Some(y)) = (xy[0], xy[1]) {
        (json!([x,y]), "Klipper XY travel in this recorded sample; physical plate may differ")
    } else if g["bed_size_confirmed"].as_bool() == Some(true) {
        (configured_bed, "From your Klipper X/Y configuration")
    } else {
        (configured_bed, "Unverified display envelope; configure the printer's bed dimensions")
    };
    let fallback_offset = g["probe_offset_mm"]
        .as_array()
        .filter(|a| {
            a.len() == 2
                && a.iter()
                    .all(|v| number(v).is_some_and(|n| n.abs() <= 500.0))
        })
        .cloned()
        .map(Value::Array)
        .unwrap_or_else(|| json!(cfg.probe_offset_mm));
    let mut offset = None;
    let mut source = if g["probe_offset_confirmed"].as_bool() == Some(true) {"From your Klipper probe configuration"} else {"Unverified default; enter the probe's actual offsets"};
    for name in ["bltouch", "probe"] {
        let x = number(&settings[name]["x_offset"]);
        let y = number(&settings[name]["y_offset"]);
        if let (Some(x), Some(y)) = (x, y) {
            if x.abs() <= 500.0 && y.abs() <= 500.0 {
                offset = Some([x, y]);
                source = "Klipper configuration in this recorded sample";
                break;
            }
        }
    }
    json!({"machine":cfg.printer_model.as_deref().unwrap_or("Printer"),"bed_size_mm":bed,"bed_size_source":bed_source,"bed_size_confirmed":g["bed_size_confirmed"].as_bool()==Some(true),"probe_offset_mm":offset.map(|v|json!(v)).unwrap_or(fallback_offset),
     "probe_offset_source":source,"z_max_mm":z.unwrap_or(310.0),"z_max_source":if z.is_some(){"Recorded Klipper travel limit"}else{"310 mm display envelope; not measured or verified"},
 "recorded":recorded,"poll_interval_ms":number(&g["poll_interval_ms"]).filter(|n|*n>0.0).unwrap_or(cfg.poll_interval_ms as f64),"qgl_tolerance_mm":g["qgl_tolerance_mm"],
     "layout_basis":"Reference printer schematic scaled to recorded travel; frame, printed parts, toolhead housing and electronics placement are illustrative until a printer model is loaded. Corner mapping is a reference map, not measured CAD.",
 "kinematics":text(&settings["printer"]["kinematics"]),"independent_corner_positions":false})
}
struct Bounded {
    bytes: Vec<u8>,
    cap: usize,
}
impl Write for Bounded {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > self.cap.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("response cap"));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode(v: &Value, cap: usize) -> Response {
    let mut out = Bounded {
        bytes: Vec::new(),
        cap,
    };
    match serde_json::to_writer(&mut out, v) {
        Ok(()) => Response {
            code: 200,
            mime: "application/json; charset=utf-8",
            body: out.bytes,
        },
        Err(_) => Response::json(
            400,
            json!({"error":"workspace response exceeds max_response_bytes; reduce points or window"}),
        ),
    }
}

pub fn snapshot(
    db: &Store,
    cfg: &Config,
    status: &CollectorStatus,
    cursor: Option<f64>,
    window: f64,
    points: usize,
    step: Option<&str>,
) -> Result<Value> {
    let start = Instant::now();
    let _snapshot = db.conn.unchecked_transaction().map_err(db_error)?;
    let bounds = db.bounds()?;
    let clock = model::now();
    let mut at = cursor.unwrap_or(if status.mode == "replay" {
        bounds.1.unwrap_or(clock)
    } else {
        clock
    });
    let cap = cfg.max_response_bytes.min(2 * 1024 * 1024);
    if let Some(direction) = step {
        let sql = if direction == "previous" {
            "SELECT MAX(ts) FROM samples WHERE ts<?1"
        } else {
            "SELECT MIN(ts) FROM samples WHERE ts>?1"
        };
        let next: Option<f64> = db
            .conn
            .query_row(sql, [at], |r| r.get(0))
            .map_err(db_error)?;
        if let Some(t) = next {
            at = t;
        }
    }
    let from = (at - window).max(0.0);
    let sample = bounded_at(db, at, cap)?;
    let raw = sample.as_ref().map(|(_, d)| d).unwrap_or(&Value::Null);
    let geo = geometry(raw, context(db, at, "scope_geometry", cap)?, cfg);
    let health = context(db, at, "scope_health", cap)?;
    let origin = context(db, at, "scope_origin", cap)?;
    let mut history = Vec::new();
    let mut previous = None;
    let first: Option<f64> = db
        .conn
        .query_row(
            "SELECT MIN(ts) FROM samples WHERE ts>=?1 AND ts<=?2",
            params![from, at],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if let Some(first) = first {
        for i in 0..points {
            if start.elapsed().as_millis() > 5000 {
                return Err(
                    "workspace history exceeded its time budget; reduce history points or window"
                        .into(),
                );
            }
            let t = if i == 0 {
                first
            } else {
                from + (at - from) * i as f64 / (points - 1) as f64
            };
            if let Some((ts, d)) = bounded_at(db, t, cap)? {
                if ts >= from && Some(ts) != previous {
                    history.push(point(ts, &d, false));
                    previous = Some(ts);
                }
            }
        }
    }
    let mut events = Vec::new();
    let mut event_bytes = 0usize;
    let mut event_cap = false;
    let n = cfg.events_limit.min(200);
    let mut q=db.conn.prepare("SELECT rowid,ts,kind,length(CAST(message AS BLOB)),message FROM events WHERE ts>=?1 AND ts<=?2 AND kind NOT LIKE 'scope_%' ORDER BY ts DESC,rowid DESC LIMIT ?3").map_err(db_error)?;
    let mut rows = q
        .query(params![from, at, (n + 1) as i64])
        .map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let len: usize = row.get(3).map_err(db_error)?;
        if events.len() >= n || len > cap / 8 || event_bytes.saturating_add(len) > cap / 8 {
            event_cap = true;
            break;
        }
        event_bytes += len;
        let ts: f64 = row.get(1).map_err(db_error)?;
        if !ts.is_finite() || ts < 0.0 {
            return Err("invalid event timestamp; preserve this recording for repair".into());
        }
        events.push(json!({"id":row.get::<_,i64>(0).map_err(db_error)?,"ts":ts,"kind":row.get::<_,String>(2).map_err(db_error)?,"message":row.get::<_,String>(4).map_err(db_error)?}));
    }
    drop(rows);
    drop(q);
    let limit = cfg.qgl_limit.min(100);
    let mut rounds = db.rounds(from, at, limit + 1)?;
    let rounds_capped = rounds.len() > limit;
    rounds.truncate(limit);
    let mut out_rounds = Vec::new();
    for r in rounds {
        if !r.ts.is_finite() || r.ts < 0.0 || !r.range_mm.is_finite() || r.range_mm < 0.0 {
            return Err("invalid levelling round; preserve the recording for repair".into());
        }
        let recorded = context(db, r.ts, "scope_geometry", cap)?;
        let tol = recorded
            .as_ref()
            .and_then(|g| number(&g["qgl_tolerance_mm"]))
            .filter(|n| *n > 0.0)
            .unwrap_or(cfg.qgl_tolerance_mm);
        let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
        v["tolerance"] = json!(tol);
        v["tolerance_source"] = json!(if recorded.is_some() {
            "Recorded recorder threshold"
        } else {
            "Current recorder threshold; not recorded at this time"
        });
        out_rounds.push(v);
    }
    let mut warnings = Vec::new();
    if sample.is_none() {
        warnings.push(
            "No sample at or before this time. Model position is a ghost layout, not a reading.",
        );
    }
    if !geo["recorded"].as_bool().unwrap_or(false) {
        warnings.push("Recorded geometry is unavailable here. Current recorder defaults are used for the layout.");
    }
    if raw["motion_report"]["live_position"]
        .as_array()
        .is_none_or(|p| p.len() < 3 || p[..3].iter().any(|v| number(v).is_none()))
    {
        warnings.push(
            "Position is not reported. No commanded-position fallback is displayed as live motion.",
        );
    }
    if geo["kinematics"].as_str().is_some_and(|s| s != "corexy") {
        warnings.push("This sample reports non-CoreXY kinematics. The R2 layout may not represent this recording.");
    }
    if event_cap {
        warnings
            .push("Event count/byte cap reached. Narrow the history window for earlier entries.");
    }
    if rounds_capped {
        warnings.push("Levelling list capped at the newest rounds. Narrow the history window.");
    }
    // Detailed point carries the recorded BLTouch probe landings inside the window (board item 3).
    let mut point_v = sample.as_ref().map(|(t, d)| point(*t, d, true));
    if let (Some(v), Some((t, _))) = (&mut point_v, sample.as_ref()) {
        let pts = db
            .probe_points(from, *t, 200)
            .unwrap_or_default()
            .into_iter()
            .map(|p| json!({"ts":p.ts,"x":p.x,"y":p.y,"z":p.z,"context":p.context}))
            .collect::<Vec<_>>();
        v["probe_points"] = json!(pts);
    }
    Ok(
        json!({"schema":"stella-scope.workspace.v1","version":env!("CARGO_PKG_VERSION"),"read_only":true,
 "cursor":at,"from":from,"first_ts":bounds.0,"last_ts":bounds.1,"server_time":clock,
 "point":point_v,"age_seconds":sample.as_ref().map(|(t,_)|(at-t).max(0.0)),
 "geometry":geo,"recorded_health":health,"recording_origin":origin,"observer_now":status,
 "history":history,"history_method":"Indexed representative readings at or before the cursor; lines are visual guides, not a continuous toolpath. Brief peaks may be omitted. Use Investigation for extrema analysis.",
 "events":events,"events_capped":event_cap,"rounds":out_rounds,"rounds_capped":rounds_capped,"warnings":warnings}),
    )
}
fn job(d: &Value) -> Value {
    json!({"name":text(&d["print_stats"]["filename"]),"state":text(&d["print_stats"]["state"]),"progress":fraction(&d["virtual_sdcard"]["progress"]),"duration":nonnegative(&d["print_stats"]["print_duration"]),"filament_used":nonnegative(&d["print_stats"]["filament_used"]),
     // The layer the slicer reports through SET_PRINT_STATS_INFO; null when it does not.
     "layer":nonnegative(&d["print_stats"]["info"]["current_layer"]),"layers":nonnegative(&d["print_stats"]["info"]["total_layer"])})
}
// Current procedure is derived from recorded console evidence plus the newest sample; it is never
// presented as a machine-reported state, and the basis is carried with the answer.
fn activity(db: &Store, at: f64, d: &Value) -> Result<Value> {
    let mut stmt = db.conn.prepare("SELECT ts,substr(message,1,400) FROM events WHERE kind IN ('response','command') AND ts<=?1 AND ts>=?1-120 ORDER BY ts DESC LIMIT 40").map_err(db_error)?;
    let mut rows = stmt.query(params![at]).map_err(db_error)?;
    let mut records: Vec<(f64, String)> = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        records.push((
            row.get::<_, f64>(0).map_err(db_error)?,
            row.get::<_, String>(1).map_err(db_error)?,
        ));
    }
    drop(rows);
    drop(stmt);
    // BLTouch probing is the most specific live state: "probe: at X,Y bed will contact at z=Z"
    // lines stream while any probing procedure runs (formats from a real printer\'s
    // recording).
    let procedure = records.iter().find_map(|(_, msg)| {
        let l = msg.to_lowercase();
        if l.contains("quad_gantry_level") || l.contains("gantry-relative") {
            Some("gantry levelling")
        } else if l.contains("bed_mesh") || l.contains("bed mesh") {
            Some("bed mesh")
        } else if l.contains("g28") {
            Some("homing Z")
        } else if l.contains("probe_accuracy") {
            Some("probe accuracy")
        } else {
            None
        }
    });
    for (ts, message) in records {
        let m = message.to_lowercase();
        if let Some((x, y, z)) = crate::probe::point_from(&message) {
            return Ok(
                json!({"id":"probing","label":"BLTouch probing","detail":format!("{} · point {:.0},{:.0} = {:.3} mm", procedure.unwrap_or("probing"), x, y, z),"since_ts":ts,
                "basis":"Derived from recorded BLTouch console lines; not a machine-reported state"}),
            );
        }
        let kind = if m.contains("gantry-relative probe points")
            || m.contains("quad_gantry_level")
            || m.contains("z adjustments")
        {
            let mut detail = "Gantry-relative levelling run in progress".to_owned();
            if let Some(rest) = message.split("Probed points range: ").nth(1) {
                detail = format!(
                    "Probed range {} mm",
                    rest.split_whitespace().next().unwrap_or("?")
                );
            }
            if let Some(r) = message.split("Retries: ").nth(1) {
                detail = format!("{} · retry {}", detail, r.split('/').next().unwrap_or("?"));
            }
            Some(("levelling", "Gantry levelling (QUAD_GANTRY_LEVEL)", detail))
        } else if m.contains("bed_mesh") || m.contains("mesh point") {
            Some((
                "meshing",
                "Bed mesh probing",
                format!("Console evidence {:.0} s before the newest sample", at - ts),
            ))
        } else if m.contains("g28") || m.contains("homing") {
            Some((
                "homing",
                "Homing",
                format!("Console evidence {:.0} s before the newest sample", at - ts),
            ))
        } else {
            None
        };
        if let Some((id, label, detail)) = kind {
            return Ok(json!({"id":id,"label":label,"detail":detail,"since_ts":ts,
                "basis":"Derived from recent recorded console evidence; not a machine-reported activity state"}));
        }
    }
    let state = job(d);
    let name = state["name"].as_str().unwrap_or("");
    let heating = |temp: Option<f64>, target: Option<f64>, what: &str| -> Option<Value> {
        match (temp, target) {
            (Some(t), Some(g)) if g > 0.0 && t < g - 1.5 => Some(
                json!({"id":"heating","label":format!("Heating {what}"),"detail":format!("{t:.1} → {g:.1} °C"),"since_ts":at,"basis":"Derived from recorded heater targets in the newest sample"}),
            ),
            _ => None,
        }
    };
    if state["state"].as_str() == Some("printing") {
        return Ok(
            json!({"id":"printing","label":"Printing","detail":format!("{} · {}%",name,((state["progress"].as_f64().unwrap_or(0.0)*100.0).round())),"since_ts":at,"basis":"print_stats state in the newest recorded sample"}),
        );
    }
    if state["state"].as_str() == Some("paused") {
        return Ok(
            json!({"id":"paused","label":"Paused","detail":name,"since_ts":at,"basis":"print_stats state in the newest recorded sample"}),
        );
    }
    let nozzle = number(&d["extruder"]["temperature"]);
    let nozzle_t = number(&d["extruder"]["target"]);
    let bed = number(&d["heater_bed"]["temperature"]);
    let bed_t = number(&d["heater_bed"]["target"]);
    if let Some(v) = heating(nozzle, nozzle_t, "nozzle") {
        return Ok(v);
    }
    if let Some(v) = heating(bed, bed_t, "bed") {
        return Ok(v);
    }
    if number(&d["motion_report"]["live_velocity"]).is_some_and(|v| v > 0.5) {
        return Ok(
            json!({"id":"moving","label":"Moving","detail":"Live velocity reported; jog, macro or calibration motion","since_ts":at,"basis":"motion_report live velocity in the newest sample"}),
        );
    }
    Ok(
        json!({"id":"idle","label":"Idle","detail":"No procedure evidence and no motion in the newest sample","since_ts":at,"basis":"Absence of recorded evidence is not proof the printer is idle"}),
    )
}
// Public wrappers for the collector's cached-tick publish: same projection the HTTP tick uses,
// but built by the collector the moment a sample is stored, so /api/tick and SSE never re-query.
pub fn job_for(d: &Value) -> Value {
    job(d)
}
pub fn activity_for(db: &Store, at: f64, d: &Value) -> Value {
    activity(db, at, d).unwrap_or_else(|e| json!({"id":"unknown","label":"Activity unavailable","detail":e,"since_ts":at,"basis":"Derivation failed; the console evidence could not be read"}))
}
/// L4: runout sensors, when Klipper reports any. the test printer has none fitted today, so
/// this is an empty array there — an empty list means "none configured", never "none detected".
/// Discovery already allows the `filament_switch_sensor ` and `filament_motion_sensor `
/// prefixes (`telemetry::allowed`), so a sensor added to printer.cfg appears here with no
/// config change.
pub fn filament_sensors(data: &Value) -> Vec<Value> {
    let Some(map) = data.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<Value> = map
        .iter()
        .filter_map(|(key, value)| {
            let name = key
                .strip_prefix("filament_switch_sensor ")
                .map(|n| (n, "switch"))
                .or_else(|| {
                    key.strip_prefix("filament_motion_sensor ")
                        .map(|n| (n, "motion"))
                })?;
            Some(json!({
                "name": name.0,
                "kind": name.1,
                "filament_detected": value["filament_detected"],
                "enabled": value["enabled"]
            }))
        })
        .collect();
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

pub fn tick_payload(
    ts: f64,
    data: Option<&Value>,
    job_v: Value,
    act: Value,
    homed: Option<&str>,
    probe_points: &[Value],
    cfg: &Config,
    status: &CollectorStatus,
) -> Value {
    let mut point = data.map(|d| point(ts, d, false)).unwrap_or(Value::Null);
    if point.is_object() {
        point["probe_points"] = json!(probe_points);
    }
    json!({"schema":"stella-scope.tick.v1","server_time":ts,"requested_live":true,
        "ts":ts,"age_seconds":0.0,
        "point":point,"job":job_v,"activity":act,"homed_axes":homed,
        "filament_sensors":filament_sensors(data.unwrap_or(&Value::Null)),
        "observer":{"mode":status.mode,"connected":status.connected,"transport":status.transport,
            "updates_per_s_1m":status.updates_per_s_1m,"query_ms_last":status.query_ms_last,
            "frame_age_s":status.push_frame_age_s,"keepalive_ms":status.push_keepalive_ms,
            "last_success":status.last_success,"printer_state":status.printer_state,"printer_message":status.printer_message,"klipper_state":status.klipper_state,
            "degraded_count":status.degraded.len(),"last_error":status.last_error,
            "poll_interval_ms":cfg.poll_interval_ms}})
}
pub fn tick(db: &Store, cfg: &Config, status: &CollectorStatus) -> Result<Value> {
    let clock = model::now();
    // Fresh cached payload from the collector: no DB replay, no activity query per reader.
    if let (Some(payload), Some(t)) = (&status.last_tick, status.last_tick_ts) {
        if clock - t < 2.0 && status.mode != "replay" {
            let mut v = payload.clone();
            v["age_seconds"] = json!((clock - t).max(0.0));
            v["server_time"] = json!(clock);
            return Ok(v);
        }
    }
    let sample = db.at(f64::MAX)?;
    let (ts, point, job_v, act, homed, sensors) = match sample {
        Some((ts, d)) => (
            Some(ts),
            point(ts, &d, false),
            job(&d),
            activity(db, ts, &d)?,
            text(&d["toolhead"]["homed_axes"]),
            filament_sensors(&d),
        ),
        None => (
            None,
            Value::Null,
            job(&Value::Null),
            json!({"id":"no_data","label":"No recorded sample","detail":"The recorder has not captured a sample from the printer yet","since_ts":clock,"basis":"No sample exists"}),
            None,
            Vec::new(),
        ),
    };
    Ok(
        json!({"schema":"stella-scope.tick.v1","server_time":clock,"requested_live":true,
        "ts":ts,"age_seconds":ts.map(|t|(clock-t).max(0.0)),
        "point":point,"job":job_v,"activity":act,"homed_axes":homed,
        "filament_sensors":sensors,
        "observer":{"mode":status.mode,"connected":status.connected,"transport":status.transport,
        "updates_per_s_1m":status.updates_per_s_1m,"query_ms_last":status.query_ms_last,
        "frame_age_s":status.push_frame_age_s,"keepalive_ms":status.push_keepalive_ms,
        "last_success":status.last_success,"printer_state":status.printer_state,"printer_message":status.printer_message,"klipper_state":status.klipper_state,
        "degraded_count":status.degraded.len(),"last_error":status.last_error,
        "poll_interval_ms":cfg.poll_interval_ms}}),
    )
}
pub fn tick_handle(method: &str, db: &Store, cfg: &Config, status: &CollectorStatus) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"read-only tick; only GET is allowed"}));
    }
    match tick(db, cfg, status) {
        Ok(v) => encode(&v, cfg.max_response_bytes),
        Err(e) => Response::json(500, json!({"error":e})),
    }
}
// Long-window trend evidence for the Leveling page: distinct recorded meshes, complete QGL runs,
// and driver-warning density. All of it is read from the recording; nothing is extrapolated.
pub fn trends(db: &Store, _cfg: &Config, to: f64, window_s: f64) -> Result<Value> {
    let from = to - window_s;
    // The mesh series stays sampled, one reading per 30 s bucket, the newest row in the bucket
    // that holds a matrix. The rows now come from Store::replay (the decoder history.rs uses):
    // keyframes, deltas that name a mesh field, deltas that remove an object. The json_extract
    // this replaces saw keyframes only, so a mesh a delta set was dated at the next keyframe, or
    // missed when gone again before one. Unchanged below: a repeat of the previous reading is
    // skipped, and an empty grid in between makes the same mesh count again.
    let mut buckets: Vec<(f64, String, Value)> = Vec::new();
    let read = ["/bed_mesh/probed_matrix", "/bed_mesh/profile_name"];
    db.replay(from, to, &read, |ts, sample| {
        let mesh = &sample["bed_mesh"];
        if mesh["probed_matrix"].is_null() {
            return true;
        }
        let reading = (
            ts,
            mesh["profile_name"].as_str().unwrap_or_default().to_owned(),
            mesh["probed_matrix"].clone(),
        );
        match buckets.last_mut() {
            Some(last) if (last.0 / 30.0).floor() == (ts / 30.0).floor() => *last = reading,
            _ => buckets.push(reading),
        }
        true
    })?;
    let mut meshes: Vec<Value> = Vec::new();
    let mut last_sig = String::new();
    for (ts, profile, v) in buckets {
        let sig: String = v.to_string().chars().take(160).collect();
        if sig == last_sig {
            continue;
        }
        last_sig = sig;
        if let Some(grid) = matrix(&v) {
            let flat: Vec<f64> = grid
                .iter()
                .flat_map(|r| r.iter().filter_map(|x| *x).collect::<Vec<_>>())
                .collect();
            if !flat.is_empty() {
                let lo = flat.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = flat.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                meshes.push(json!({"ts":ts,"profile":profile,"rows":grid.len(),"cols":grid[0].len(),
                    "min":lo,"max":hi,"range":hi-lo,"adaptive":profile.to_lowercase().starts_with("adaptive")}));
            }
        }
    }
    let mut runs_stmt = db.conn.prepare(
        "SELECT run_id, MIN(ts), MAX(ts), COUNT(*), \
         MAX(CASE WHEN round=(SELECT MAX(round) FROM qgl q2 WHERE q2.run_id=qgl.run_id) THEN range_mm END) \
         FROM qgl WHERE ts>=?1 AND ts<=?2 GROUP BY run_id ORDER BY MIN(ts)",
    ).map_err(db_error)?;
    let runs: Vec<Value> = runs_stmt
        .query_map(params![from, to], |r| {
            let run_id: i64 = r.get(0)?;
            let first_ts: f64 = r.get(1)?;
            let last_ts: f64 = r.get(2)?;
            let rounds: i64 = r.get(3)?;
            let final_range: Option<f64> = r.get(4)?;
            Ok(json!({"run_id":run_id,"first_ts":first_ts,"last_ts":last_ts,"rounds":rounds,"final_range_mm":final_range}))
        })
        .map_err(db_error)?
        .filter_map(|r| r.ok())
        .collect();
    drop(runs_stmt);
    let mut warn_stmt = db
        .conn
        .prepare(
            "SELECT CAST(ts/3600 AS INTEGER)*3600, COUNT(*) FROM events \
         WHERE kind='driver_warning' AND ts>=?1 AND ts<=?2 GROUP BY 1 ORDER BY 1",
        )
        .map_err(db_error)?;
    let warnings: Vec<Value> = warn_stmt
        .query_map(params![from, to], |r| {
            Ok(json!({"hour_ts":r.get::<_, f64>(0)?,"count":r.get::<_, i64>(1)?}))
        })
        .map_err(db_error)?
        .filter_map(|r| r.ok())
        .collect();
    drop(warn_stmt);
    // Z health (board item 4): zero-reference series and the Z-offset change log.
    let zeros: Vec<Value> = db
        .probe_points(from, to, 400)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| crate::probe::is_zero_reference(p.x, p.y, &p.context))
        .map(|p| json!({"ts": p.ts, "z": p.z}))
        .collect();
    let mut off_stmt = db
        .conn
        .prepare("SELECT ts,message FROM events WHERE kind='z_offset_change' AND ts>=?1 AND ts<=?2 ORDER BY ts DESC LIMIT 40")
        .map_err(db_error)?;
    let offset_log: Vec<Value> = off_stmt
        .query_map(params![from, to], |r| {
            Ok(json!({"ts": r.get::<_, f64>(0)?, "change": r.get::<_, String>(1)?}))
        })
        .map_err(db_error)?
        .filter_map(|r| r.ok())
        .collect();
    drop(off_stmt);
    // Resonance band: recorded time per hour with belt-tooth frequency within ±3 Hz of the printer's
    // Y resonance, taken from Klipper's [input_shaper] shaper_freq_y (GT2 2 mm pitch: f = v/2, so the
    // band is 2(f-3) to 2(f+3) mm/s). No configured shaper frequency means no band. Pure math on recorded
    // velocities; approximate seconds assuming ~4 samples per second. every recorded sample
    // counts, as the SQL meant to. A delta only carries live_velocity when it changed, so the
    // replay yields the velocity's changes and each sample timestamp in the window is counted at
    // the velocity in force then. The json_extract this replaces saw keyframes only, one sample a
    // minute. The timestamps are streamed, not collected: no cap has to guess the recorder's
    // fastest cadence (poll_interval_ms goes to 50).
    const VELOCITY: &str = "/motion_report/live_velocity";
    let reading = |s: &Value| s.pointer(VELOCITY).and_then(Value::as_f64);
    let mut current = db.at(from)?.and_then(|(_, s)| reading(&s));
    let mut changes: Vec<(f64, Option<f64>)> = Vec::new();
    db.replay(from, to, &[VELOCITY], |ts, sample| {
        let v = reading(sample);
        if changes.last().map_or(current, |(_, last)| *last) != v {
            changes.push((ts, v));
        }
        true
    })?;
    let shaper_y = db
        .at(to)?
        .and_then(|(_, s)| s.pointer("/configfile/settings/input_shaper/shaper_freq_y").and_then(Value::as_f64))
        .filter(|f| *f > 3.0);
    let band = shaper_y.map(|f| (2.0 * (f - 3.0), 2.0 * (f + 3.0)));
    let mut in_band: BTreeMap<i64, u64> = BTreeMap::new();
    let mut next = changes.iter().peekable();
    let mut times = db
        .conn
        .prepare("SELECT ts FROM samples WHERE ts>=?1 AND ts<=?2 ORDER BY ts")
        .map_err(db_error)?;
    for ts in times
        .query_map(params![from, to], |r| r.get::<_, f64>(0))
        .map_err(db_error)?
    {
        let ts = ts.map_err(db_error)?;
        while let Some((_, v)) = next.next_if(|(at, _)| *at <= ts) {
            current = *v;
        }
        if matches!((current, band), (Some(v), Some((lo, hi))) if (lo..=hi).contains(&v)) {
            *in_band.entry((ts / 3600.0) as i64).or_default() += 1;
        }
    }
    drop(times);
    let resonance: Vec<Value> = in_band
        .into_iter()
        .map(|(h, n)| json!({"hour_ts": (h * 3600) as f64, "approx_seconds": (n as f64 / 4.0 * 10.0).round() / 10.0}))
        .collect();
    let prints: Vec<Value> = db
        .print_context(from, to, 50)
        .unwrap_or_default()
        .into_iter()
        .map(|(ts, filename, metadata, orca)| json!({"ts": ts, "filename": filename, "metadata": metadata, "orca": orca}))
        .collect();
    Ok(
        json!({"schema":"stella-scope.trends.v1","from":from,"to":to,"window_s":window_s,
        "meshes":meshes,"qgl_runs":runs,"warnings":warnings,
        "z_health":{"zero_references":zeros,"offset_log":offset_log},
        "resonance":{"band_mm_s":band.map(|(lo, hi)| json!([lo, hi])),"shaper_freq_y":shaper_y,"hours":resonance,
            "note":"Recorded seconds per hour spent at speeds whose GT2 belt-tooth frequency (v/2) sits within ±3 Hz of the Y resonance set in Klipper's [input_shaper] (shaper_freq_y). Derived from recorded velocities only."},
        "print_context":prints,
        "basis":"Distinct recorded bed meshes and complete levelling runs within the window; repeated capture of an unchanged mesh is collapsed. Warnings are recorded event counts, not a health verdict. Resonance seconds are estimates from the recorded sample rate."}),
    )
}
pub fn trends_handle(
    method: &str,
    target: &str,
    db: &Store,
    cfg: &Config,
    _status: &CollectorStatus,
) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"read-only trends; only GET is allowed"}),
        );
    }
    let parsed = (|| -> Result<Response> {
        let (_, q) = super::query(target)?;
        for key in q.keys() {
            if key != "window" {
                return Err(format!("unknown trends parameter {key}; use window").into());
            }
        }
        let window_s = super::number(&q, "window", Some(14400.0))?;
        if !(600.0..=86400.0).contains(&window_s) {
            return Err("trends window must be 600..86400 seconds".into());
        }
        let to = db
            .at(f64::MAX)
            .ok()
            .flatten()
            .map(|(ts, _)| ts)
            .unwrap_or_else(model::now);
        let v = trends(db, cfg, to, window_s)?;
        Ok(encode(&v, cfg.max_response_bytes))
    })();
    match parsed {
        Ok(v) => v,
        Err(e) => Response::json(400, json!({"error":e})),
    }
}
pub fn handle(
    method: &str,
    target: &str,
    db: &Store,
    cfg: &Config,
    status: &CollectorStatus,
) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"read-only workspace; only GET is allowed"}),
        );
    }
    let parse = (|| -> Result<(Option<f64>, f64, usize, Option<String>)> {
        let (_, q) = super::query(target)?;
        for key in q.keys() {
            if !["ts", "window", "points", "step"].contains(&key.as_str()) {
                return Err(format!(
                    "unknown workspace parameter {key}; use ts, window, points, step"
                ));
            }
        }
        let ts = if q.contains_key("ts") {
            Some(super::number(&q, "ts", None)?)
        } else {
            None
        };
        if ts.is_some_and(|t| t > 253402300799.0) {
            return Err("timestamp exceeds the supported calendar range".into());
        }
        let window = super::number(&q, "window", Some(900.0))?;
        if !(1.0..=7200.0).contains(&window) {
            return Err("workspace window must be 1..7200 seconds".into());
        }
        let points = q
            .get("points")
            .map_or(Ok(100.min(cfg.series_max_points)), |s| {
                s.parse::<usize>()
                    .map_err(|_| "points must be an integer".to_owned())
            })?;
        if !(2..=cfg.series_max_points.min(240)).contains(&points) {
            return Err("workspace points must be 2..min(series_max_points,240)".into());
        }
        let step = q.get("step").cloned();
        if step
            .as_deref()
            .is_some_and(|s| !["previous", "next"].contains(&s))
            || step.is_some() && ts.is_none()
        {
            return Err("step needs ts and must be previous or next".into());
        }
        Ok((ts, window, points, step))
    })();
    match parse {
        Err(e) => Response::json(400, json!({"error":e})),
        Ok((ts, w, n, step)) => match snapshot(db, cfg, status, ts, w, n, step.as_deref()) {
            Ok(v) => encode(&v, cfg.max_response_bytes),
            Err(e) => Response::json(500, json!({"error":e})),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qgl::Parser;
    fn store() -> Store {
        Store::open(":memory:", false).unwrap()
    }
    fn put(db: &mut Store, ts: f64, d: Value) {
        db.write(Some((ts, &d)), &[], &mut Parser::default())
            .unwrap();
    }
    fn data() -> Value {
        json!({"motion_report":{"live_position":[100,150,23],"live_velocity":10},"toolhead":{"position":[110,151,25],"homed_axes":"xyz"},"configfile":{"settings":{"secret":"KEEP_PRIVATE","bltouch":{"x_offset":-10,"y_offset":20},"stepper_z":{"position_max":305}}},"tmc2209 stepper_z1":{"drv_status":{},"run_current":0.8}})
    }
    fn snap(db: &Store, t: f64) -> Value {
        snapshot(
            db,
            &Config::default(),
            &CollectorStatus::default(),
            Some(t),
            900.0,
            10,
            None,
        )
        .unwrap()
    }
    #[test]
    fn at_or_before_all_panels() {
        let mut db = store();
        put(&mut db, 100.0, data());
        put(&mut db, 200.0, data());
        db.conn
            .execute("INSERT INTO events VALUES(160,'error','FUTURE')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO qgl VALUES(160,1,0,0.1,NULL,NULL,NULL,NULL)",
                [],
            )
            .unwrap();
        let s = snap(&db, 150.0);
        assert_eq!(s["point"]["ts"], 100.0);
        assert!(s["events"].as_array().unwrap().is_empty());
        assert!(s["rounds"].as_array().unwrap().is_empty());
        assert!(s["history"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["ts"].as_f64().unwrap() <= 150.0));
    }
    #[test]
    fn no_commanded_position_fallback() {
        let mut db = store();
        put(&mut db, 100.0, json!({"toolhead":{"position":[1,2,3]}}));
        assert!(snap(&db, 101.0)["point"]["position"][0].is_null());
    }
    #[test]
    fn no_future_geometry_leak() {
        let mut db = store();
        put(&mut db, 100.0, data());
        db.conn
            .execute(
                "INSERT INTO events VALUES(160,'scope_geometry','{\"bed_size_mm\":[999,999]}')",
                [],
            )
            .unwrap();
        assert_eq!(
            snap(&db, 120.0)["geometry"]["bed_size_mm"],
            json!([350.0, 350.0])
        );
    }
    #[test]
    fn sample_probe_offsets_precede_old_defaults() {
        let mut db = store();
        put(&mut db, 100.0, data());
        assert_eq!(
            snap(&db, 100.0)["geometry"]["probe_offset_mm"],
            json!([-10.0, 20.0])
        );
    }
    #[test]
    fn z_limit_comes_from_recorded_config() {
        let mut db = store();
        put(&mut db, 100.0, data());
        assert_eq!(snap(&db, 100.0)["geometry"]["z_max_mm"], 305.0);
    }
    #[test]
    fn missing_z_limit_is_labelled() {
        let mut db = store();
        put(&mut db, 100.0, json!({}));
        assert!(snap(&db, 100.0)["geometry"]["z_max_source"]
            .as_str()
            .unwrap()
            .contains("not measured"));
    }
    #[test]
    fn no_raw_config_leaks() {
        let mut db = store();
        put(&mut db, 100.0, data());
        let s = snap(&db, 100.0).to_string();
        assert!(!s.contains("KEEP_PRIVATE"));
        assert!(!s.contains("configfile"));
    }
    #[test]
    fn absent_driver_is_unknown() {
        assert!(motor(&json!({}), "stepper_z")["flags"].is_null());
    }
    #[test]
    fn sparse_present_driver_is_zero() {
        assert_eq!(motor(&data(), "stepper_z1")["scale"], 0);
        assert_eq!(motor(&data(), "stepper_z1")["flags"], json!([]));
    }
    #[test]
    fn invalid_driver_values_are_unknown() {
        let d = json!({"tmc2209 stepper_z":{"drv_status":{"cs_actual":99,"ot":"bad"}}});
        assert!(motor(&d, "stepper_z")["flags"].is_null());
        assert!(motor(&d, "stepper_z")["scale"].is_null());
    }
    #[test]
    fn bool_flags_supported() {
        let d = json!({"tmc2209 extruder":{"drv_status":{"otpw":true}}});
        assert_eq!(motor(&d, "extruder")["flags"], json!(["otpw"]));
    }
    #[test]
    fn unreported_fans_are_not_zero() {
        assert!(point(1.0, &json!({}), true)["fans"]["fan"]["speed"].is_null());
    }
    /// The HUD tells Klipper's homing force position apart from a real pose by the machine's own
    /// travel, so the limits have to ride on every point, and stay unknown when Klipper is silent
    /// rather than collapsing to a zero-sized machine that would reject everything.
    #[test]
    fn travel_limits_ride_on_the_point() {
        let v = point(
            1.0,
            &json!({"toolhead":{"axis_minimum":[0,0,-5,0],"axis_maximum":[350,350,310,0]}}),
            false,
        );
        assert_eq!(v["limits"]["min"], json!([0.0, 0.0, -5.0]));
        assert_eq!(v["limits"]["max"], json!([350.0, 350.0, 310.0]));
        assert!(point(1.0, &json!({}), false)["limits"]["max"][0].is_null());
    }
    /// Z's own speed limit and the byte the printer is reading both ride on the point. The travel
    /// guard needs max_z_velocity (20 against max_velocity 300 on the test printer) or a homing
    /// jump is inside one tick's XY reach, and the drawn toolpath needs the file position because
    /// a progress ratio cannot be mapped back onto the parser's byte offsets without the file
    /// size.
    #[test]
    fn z_speed_and_file_position_ride_on_the_point() {
        let v = point(
            1.0,
            &json!({"configfile":{"settings":{"printer":{"max_z_velocity":20.0}}},
                "virtual_sdcard":{"file_position":1_276_320,"file_size":2_439_623}}),
            false,
        );
        assert_eq!(v["max_z_velocity"], json!(20.0));
        assert_eq!(v["file_position"], json!(1_276_320.0));
        assert_eq!(v["file_size"], json!(2_439_623.0));
        let quiet = point(1.0, &json!({}), false);
        assert!(quiet["max_z_velocity"].is_null());
        assert!(quiet["file_position"].is_null());
    }
    /// Live motion beside X, Y and Z, and the layer for the bottom strip. The filament speed keeps
    /// its sign (a retraction is negative); anything not reported is unknown, never zero. The
    /// snapshot's job and the tick's job are the same projection.
    #[test]
    fn motion_readouts_and_layer_ride_on_the_point() {
        let sample = json!({"toolhead":{"max_accel":5000.0},"motion_report":{"live_extruder_velocity":-2.5},
            "gcode_move":{"speed":12000.0},"print_stats":{"info":{"current_layer":12,"total_layer":150}}});
        let v = point(1.0, &sample, true);
        assert_eq!(v["max_accel"], json!(5000.0));
        assert_eq!(v["extruder_velocity"], json!(-2.5));
        assert_eq!(v["requested_speed"], json!(200.0));
        assert_eq!(v["job"]["layer"], json!(12.0));
        assert_eq!(v["job"]["layers"], json!(150.0));
        assert_eq!(v["job"], job(&sample));
        let quiet = point(1.0, &json!({}), false);
        for key in ["max_accel", "extruder_velocity", "requested_speed"] {
            assert!(quiet[key].is_null(), "{key}");
        }
        assert!(job(&json!({}))["layer"].is_null());
        assert!(job(&json!({"print_stats":{"info":{"current_layer":-1}}}))["layer"].is_null());
    }
    #[test]
    fn invalid_fan_percent_is_unknown() {
        assert!(fan(&json!({"speed":2.0}))["speed"].is_null());
    }
    #[test]
    fn invalid_boolean_is_unknown() {
        assert_eq!(boolean(&json!(2)), None);
    }
    #[test]
    fn mesh_validated_and_missing_cells_retained() {
        assert_eq!(matrix(&json!([[1, null], [0, 2]])).unwrap()[0][1], None);
        assert!(matrix(&json!([[1], [1, 2]])).is_none());
    }
    #[test]
    fn mesh_absent_is_unknown_not_flat() {
        assert!(point(1.0, &json!({}), true)["mesh"]["grid"].is_null());
    }
    #[test]
    fn empty_recording_remains_empty() {
        let s = snap(&store(), 10.0);
        assert!(s["point"].is_null());
        assert!(s["first_ts"].is_null());
        assert!(!s["warnings"].as_array().unwrap().is_empty());
    }
    #[test]
    fn history_is_bounded_and_ordered() {
        let mut db = store();
        for i in 0..50 {
            put(&mut db, 100.0 + i as f64, data());
        }
        let s = snap(&db, 149.0);
        let a = s["history"].as_array().unwrap();
        assert!(a.len() <= 10);
        assert!(a
            .windows(2)
            .all(|w| w[0]["ts"].as_f64() < w[1]["ts"].as_f64()));
    }
    #[test]
    fn step_previous_and_next() {
        let mut db = store();
        put(&mut db, 100.0, data());
        put(&mut db, 101.0, data());
        for (at, step, expected) in [(101.0, "previous", 100.0), (100.0, "next", 101.0)] {
            let s = snapshot(
                &db,
                &Config::default(),
                &CollectorStatus::default(),
                Some(at),
                10.0,
                2,
                Some(step),
            )
            .unwrap();
            assert_eq!(s["cursor"], expected);
        }
    }
    #[test]
    fn oversized_status_fails_visibly() {
        let mut db = store();
        put(&mut db, 100.0, json!({"big":"x".repeat(70000)}));
        let mut c = Config::default();
        c.max_response_bytes = 65536;
        assert!(snapshot(
            &db,
            &c,
            &CollectorStatus::default(),
            Some(100.0),
            10.0,
            2,
            None
        )
        .is_err());
    }
    #[test]
    fn oversized_events_are_capped_not_allocated_to_output() {
        let db = store();
        db.conn
            .execute(
                "INSERT INTO events VALUES(100,'error',?1)",
                ["x".repeat(300000)],
            )
            .unwrap();
        let s = snap(&db, 100.0);
        assert_eq!(s["events_capped"], true);
        assert!(s["events"].as_array().unwrap().is_empty());
    }
    #[test]
    fn replay_does_not_write() {
        let mut db = store();
        put(&mut db, 100.0, data());
        let n = db.conn.total_changes();
        snap(&db, 100.0);
        assert_eq!(db.conn.total_changes(), n);
    }
    #[test]
    fn corrupt_json_is_error() {
        let db = store();
        db.conn
            .execute("INSERT INTO samples(ts,data) VALUES(100,'broken')", [])
            .unwrap();
        assert!(snapshot(
            &db,
            &Config::default(),
            &CollectorStatus::default(),
            Some(100.0),
            10.0,
            2,
            None
        )
        .is_err());
    }
    #[test]
    fn filament_sensors_appear_only_when_klipper_reports_them() {
        // The test printer has none fitted: an empty list, never a fabricated "filament
        // present".
        assert!(filament_sensors(&json!({"toolhead":{"homed_axes":"xyz"}})).is_empty());
        assert!(filament_sensors(&Value::Null).is_empty());

        let sensors = filament_sensors(&json!({
            "toolhead":{"homed_axes":"xyz"},
            "filament_switch_sensor toolhead_sensor":{"filament_detected":true,"enabled":true},
            "filament_switch_sensor entry_sensor":{"filament_detected":false,"enabled":false},
            "filament_motion_sensor encoder":{"filament_detected":true,"enabled":true}
        }));
        assert_eq!(sensors.len(), 3);
        // Sorted by name so the readout does not reorder between ticks.
        assert_eq!(sensors[0]["name"], "encoder");
        assert_eq!(sensors[0]["kind"], "motion");
        assert_eq!(sensors[1]["name"], "entry_sensor");
        assert_eq!(sensors[1]["kind"], "switch");
        assert_eq!(sensors[1]["filament_detected"], false);
        assert_eq!(sensors[1]["enabled"], false);
        assert_eq!(sensors[2]["name"], "toolhead_sensor");
        assert_eq!(sensors[2]["filament_detected"], true);

        // A sensor Klipper reports without the field reads as null, not as false.
        let partial = filament_sensors(&json!({"filament_switch_sensor s":{"enabled":true}}));
        assert_eq!(partial[0]["filament_detected"], Value::Null);

        // And the tick carries the list.
        let tick = tick_payload(
            10.0,
            Some(
                &json!({"filament_switch_sensor toolhead_sensor":{"filament_detected":true,"enabled":true}}),
            ),
            Value::Null,
            Value::Null,
            Some("xyz"),
            &[],
            &Config::default(),
            &CollectorStatus::default(),
        );
        assert_eq!(tick["filament_sensors"][0]["name"], "toolhead_sensor");
    }

    #[test]
    fn no_inferred_independent_corner_positions() {
        assert_eq!(
            snap(&store(), 100.0)["geometry"]["independent_corner_positions"],
            false
        );
    }
    #[test]
    fn router_rejects_bad_queries_and_writes() {
        let db = store();
        let c = Config::default();
        let s = CollectorStatus::default();
        for q in [
            "ts=NaN",
            "ts=-1",
            "ts=999999999999",
            "window=99999",
            "points=0",
            "points=241",
            "step=next",
            "ts=1&step=bad",
            "ts=1&ts=2",
            "unknown=x",
        ] {
            assert_eq!(
                handle("GET", &format!("/api/workspace?{q}"), &db, &c, &s).code,
                400,
                "{q}"
            );
        }
        assert_eq!(handle("POST", "/api/workspace", &db, &c, &s).code, 405);
    }
    #[test]
    fn router_rejects_bad_queries_and_writes_tick() {
        let db = store();
        let c = Config::default();
        let s = CollectorStatus::default();
        assert_eq!(tick_handle("POST", &db, &c, &s).code, 405);
        assert_eq!(tick_handle("GET", &db, &c, &s).code, 200);
    }
    #[test]
    fn tick_reports_latest_sample_activity_and_stays_small() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"motion_report":{"live_position":[1,2,3],"live_velocity":0.0},"toolhead":{"position":[1,2,3],"homed_axes":"xyz"},
         "print_stats":{"state":"printing","filename":"cube.gcode"},"virtual_sdcard":{"progress":0.5},
         "extruder":{"temperature":205.0,"target":210.0},"heater_bed":{"temperature":60.0,"target":60.0}}),
        );
        let v = tick(&db, &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(v["point"]["position"], json!([1.0, 2.0, 3.0]));
        assert_eq!(v["activity"]["id"], "printing");
        assert!(v["activity"]["detail"]
            .as_str()
            .unwrap()
            .contains("cube.gcode"));
        assert_eq!(v["homed_axes"], "xyz");
        assert_eq!(v["schema"], "stella-scope.tick.v1");
        assert!(
            serde_json::to_string(&v).unwrap().len() < 8192,
            "tick payload must stay compact"
        );
    }
    #[test]
    fn tick_prefers_recent_console_evidence_over_printing() {
        let mut db = store();
        put(
            &mut db,
            200.0,
            json!({"motion_report":{"live_position":[1,2,3],"live_velocity":5.0},
         "print_stats":{"state":"printing","filename":"cube.gcode"},"virtual_sdcard":{"progress":0.5},
         "extruder":{"temperature":205.0,"target":0.0},"heater_bed":{"temperature":60.0,"target":60.0}}),
        );
        db.conn.execute("INSERT INTO events VALUES(196,'response','Gantry-relative probe points:\nMaking the following Z adjustments:\nRetries: 2/5 Probed points range: 0.034000 tolerance: 0.020000')",[]).unwrap();
        let v = tick(&db, &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(v["activity"]["id"], "levelling");
        assert!(v["activity"]["detail"].as_str().unwrap().contains("0.034"));
    }
    #[test]
    fn tick_reports_heating_and_idle_honestly() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"motion_report":{"live_velocity":0.0},
         "extruder":{"temperature":25.0,"target":210.0},"heater_bed":{"temperature":25.0,"target":60.0}}),
        );
        let v = tick(&db, &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(v["activity"]["id"], "heating");
        put(
            &mut db,
            101.0,
            json!({"motion_report":{"live_velocity":0.0},"idle_timeout":{"state":"Idle"},"extruder":{"temperature":210.0,"target":0.0},"heater_bed":{"temperature":60.0,"target":0.0}}),
        );
        let v = tick(&db, &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(v["activity"]["id"], "idle");
    }
    #[test]
    fn tick_without_samples_says_no_data() {
        let v = tick(&store(), &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(v["activity"]["id"], "no_data");
        assert!(v["point"].is_null());
    }
    #[test]
    fn tick_is_read_only() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"motion_report":{"live_position":[1,2,3]}}),
        );
        let n = db.conn.total_changes();
        tick(&db, &Config::default(), &CollectorStatus::default()).unwrap();
        assert_eq!(db.conn.total_changes(), n);
    }
    #[test]
    fn tick_observer_carries_push_latency() {
        let mut s = CollectorStatus::default();
        s.push_frame_age_s = Some(0.8);
        s.push_keepalive_ms = Some(41.0);
        let v = tick_payload(
            100.0,
            None,
            job(&Value::Null),
            Value::Null,
            None,
            &[],
            &Config::default(),
            &s,
        );
        assert_eq!(v["observer"]["frame_age_s"], 0.8);
        assert_eq!(v["observer"]["keepalive_ms"], 41.0);
        let absent = tick_payload(
            100.0,
            None,
            job(&Value::Null),
            Value::Null,
            None,
            &[],
            &Config::default(),
            &CollectorStatus::default(),
        );
        assert!(absent["observer"]["frame_age_s"].is_null());
        assert!(absent["observer"]["keepalive_ms"].is_null());
    }
    /// The header's printer state rides on both observers: the published tick and the fallback tick
    /// built from the status as it is now (what the HUD gets while no sample lands).
    #[test]
    fn tick_observers_carry_the_printer_state() {
        let mut s = CollectorStatus::default();
        s.printer_state = "unreachable".into();
        s.printer_message = Some("Printer not answering. cannot reach Moonraker at http://127.0.0.1:9".into());
        s.last_success = Some(90.0);
        let v = tick_payload(100.0, None, job(&Value::Null), Value::Null, None, &[], &Config::default(), &s);
        assert_eq!(v["observer"]["printer_state"], "unreachable");
        assert_eq!(v["observer"]["last_success"], 90.0);
        let mut db = store();
        put(&mut db, 100.0, json!({"motion_report":{"live_position":[1,2,3]}}));
        let fallback = tick(&db, &Config::default(), &s).unwrap();
        assert_eq!(fallback["observer"]["printer_state"], "unreachable");
        assert!(fallback["observer"]["printer_message"].as_str().unwrap().contains("cannot reach Moonraker"));
        // Klipper's own state word rides along too: the HUD unlocks the two restarts on it.
        s.printer_state = "not_ready".into();
        s.klipper_state = "error".into();
        let v = tick_payload(100.0, None, job(&Value::Null), Value::Null, None, &[], &Config::default(), &s);
        assert_eq!(v["observer"]["klipper_state"], "error");
        assert_eq!(tick(&db, &Config::default(), &s).unwrap()["observer"]["klipper_state"], "error");
    }
    #[test]
    fn demo_mesh_matches_a_nine_by_nine_probe_count() {
        let d = model::demo(10.0, &Config::default());
        let m = matrix(&d["bed_mesh"]["probed_matrix"])
            .expect("demo must exercise the real 9x9 probe count");
        assert_eq!(m.len(), 9);
        assert!(m.iter().all(|r| r.len() == 9));
    }
    #[test]
    fn trends_collapse_repeated_meshes_and_aggregate_runs() {
        let mut db = store();
        let mesh = json!({"bed_mesh":{"profile_name":"default-9x9","probed_matrix":[[0.01,-0.02],[0.0,0.03]]}});
        for t in [100.0, 130.0, 160.0] {
            put(&mut db, t, mesh.clone());
        }
        put(
            &mut db,
            200.0,
            json!({"bed_mesh":{"profile_name":"adaptive-X1","probed_matrix":[[0.2],[0.1]]}}),
        );
        db.conn.execute(
            "INSERT INTO qgl VALUES (300,4,0,0.12,0,0,0,0),(310,4,1,0.03,0,0,0,0),(320,4,2,0.015,0,0,0,0),(360,5,0,0.2,0,0,0,0)",
            [],
        ).unwrap();
        db.conn
            .execute(
                "INSERT INTO events VALUES (350,'driver_warning','x'),(351,'driver_warning','y')",
                [],
            )
            .unwrap();
        let v = trends(&db, &Config::default(), 400.0, 3600.0).unwrap();
        let meshes = v["meshes"].as_array().unwrap();
        assert_eq!(
            meshes.len(),
            2,
            "three identical captures collapse to one event"
        );
        assert_eq!(meshes[0]["profile"], "default-9x9");
        assert_eq!(meshes[0]["rows"], 2);
        assert!((meshes[0]["range"].as_f64().unwrap() - 0.05).abs() < 1e-9);
        assert_eq!(meshes[1]["adaptive"], true, "adaptive- prefix is flagged");
        let runs = v["qgl_runs"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        let run4 = runs.iter().find(|r| r["run_id"] == 4).unwrap();
        assert_eq!(run4["rounds"], 3);
        assert!(
            (run4["final_range_mm"].as_f64().unwrap() - 0.015).abs() < 1e-9,
            "final range is the deepest round"
        );
        let warn: i64 = v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["count"].as_i64().unwrap_or(0))
            .sum();
        assert_eq!(warn, 2);
    }
    #[test]
    fn trends_handle_bounds_and_methods() {
        let db = store();
        let c = Config::default();
        let s = CollectorStatus::default();
        assert_eq!(trends_handle("POST", "/api/trends", &db, &c, &s).code, 405);
        assert_eq!(
            trends_handle("GET", "/api/trends?window=10", &db, &c, &s).code,
            400
        );
        assert_eq!(
            trends_handle("GET", "/api/trends?window=99999", &db, &c, &s).code,
            400
        );
        assert_eq!(
            trends_handle("GET", "/api/trends?unknown=1", &db, &c, &s).code,
            400
        );
        assert_eq!(
            trends_handle("GET", "/api/trends?window=3600", &db, &c, &s).code,
            200
        );
    }
    #[test]
    fn response_limit_enforced() {
        assert_eq!(encode(&json!({"v":"x".repeat(2000)}), 100).code, 400);
    }
    #[test]
    fn snapshot_has_explicit_observer_time_namespace() {
        assert!(snap(&store(), 100.0).get("observer_now").is_some());
    }
    /// A mesh and a stretch in the resonance band that the recorder stored only in deltas,
    /// between two keyframes, reach the trends. The json_extract reads before saw keyframes only:
    /// no mesh and no second in the band here. Samples 120-130 keep the same velocity, so their
    /// deltas do not carry it at all: only counting every sample at the velocity in force finds
    /// them.
    #[test]
    fn trends_see_changes_stored_only_in_deltas() {
        let mut db = store();
        let sample = |matrix: &Value, profile: &str, x: f64, v: f64| {
            json!({
                "bed_mesh": {"profile_name": profile, "mesh_min": [40.0, 40.0],
                    "mesh_max": [310.0, 310.0], "probed_matrix": matrix},
                "motion_report": {"live_position": [x, 150.0, 5.0, 0.0], "live_velocity": v},
                "toolhead": {"homed_axes": "xyz", "axis_minimum": [0.0, 0.0, -5.0, 0.0],
                    "axis_maximum": [350.0, 350.0, 310.0, 0.0], "max_velocity": 300.0,
                    "max_accel": 3000.0, "square_corner_velocity": 5.0,
                    "minimum_cruise_ratio": 0.5, "extruder": "extruder", "stalls": 0},
                "idle_timeout": {"state": "Printing", "printing_time": 12.0},
                "webhooks": {"state": "ready", "state_message": "Printer is ready"},
                "configfile": {"settings": {"input_shaper": {"shaper_freq_y": 43.4}}}
            })
        };
        let (empty, grid) = (json!([[]]), json!([[0.01, -0.02], [0.0, 0.03]]));
        put(&mut db, 100.0, sample(&empty, "", 100.0, 0.0));
        put(&mut db, 110.0, sample(&grid, "default", 100.0, 0.0));
        for (t, x) in [
            (115.0, 110.0),
            (120.0, 130.0),
            (125.0, 150.0),
            (130.0, 170.0),
        ] {
            put(&mut db, t, sample(&grid, "default", x, 85.0));
        }
        put(&mut db, 135.0, sample(&grid, "default", 180.0, 50.0));
        put(&mut db, 150.0, sample(&empty, "", 180.0, 0.0));
        put(&mut db, 170.0, sample(&empty, "", 180.0, 0.0));
        let kinds: String = db
            .conn
            .prepare("SELECT kind FROM samples ORDER BY ts")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<String>>()
            .unwrap();
        assert_eq!(kinds, "KDDDDDDDK", "every change must live in a delta only");
        let v = trends(&db, &Config::default(), 200.0, 3600.0).unwrap();
        let meshes = v["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 1, "{v}");
        assert_eq!(meshes[0]["ts"], 110.0);
        assert_eq!(meshes[0]["profile"], "default");
        assert!((meshes[0]["range"].as_f64().unwrap() - 0.05).abs() < 1e-9);
        assert_eq!(
            v["resonance"]["hours"],
            json!([{"hour_ts": 0.0, "approx_seconds": 1.0}]),
            "four samples at 85 mm/s, three of them with the velocity unchanged"
        );
        // A window that opens inside the band counts from its first sample: the velocity in
        // force there was set at 115, before the window, and no row in the window changes it.
        let v = trends(&db, &Config::default(), 200.0, 78.0).unwrap();
        assert_eq!(
            v["resonance"]["hours"],
            json!([{"hour_ts": 0.0, "approx_seconds": 0.5}]),
            "samples 125 and 130 at the 85 mm/s in force since 115"
        );
    }
}
