// L7: the five histories, each row carrying evidence pointers back into the recording.
//
// Evidence is the whole point of this file. A row here is never a bare number: it names the
// recorded rows it was computed from, as `{kind, id, ts}`, and every one of those is resolvable
// by `GET /api/at?ts=<ts>` — which is what makes a claim on a report page checkable instead of
// merely asserted. `id` is always a string: the `events`/`qgl`/`probe_points` rowid, or the
// sample timestamp for `kind:"sample"`.
//
// Reading strategy (WO-15, 2026-09-22): a recording is keyframes (a whole sample, every 60 s)
// plus deltas that store only what changed, under `$.set`. The first cut here ran `json_extract`
// on `$.<field>`, which only keyframes carry. On the live recording 41 of 47 heat-up switch-ons
// exist only in deltas, so heat-ups were dated by the next keyframe (up to 60 s late, in 60 s
// steps), and its LIMIT on rows scanned from the oldest stopped the thermal page at 2026-09-17
// 12:13 and read 150 s of 2026-09-16 for motor drift. Samples are now rebuilt by
// `Store::replay`, the decoder load_sample and run_start use, narrowed to the rows that carry the
// fields a history reads, and values carried into a window come from `Store::at`. The seed
// timestamp is still reported so nobody has to trust a carried value blind.
// ponytail: each route is one pass over its window (SQL reads every delta's text once): measured
// 1.2 to 2.1 s for the whole 6-day, 553 MB live recording; it grows with the window. Extract
// these fields at write time into an additive table, like print_sessions, if that gets slow.
use crate::{
    api::Response,
    config::Config,
    model::{self, CollectorStatus},
    store::Store,
    Result,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Hard ceiling on rows per history, independent of the window: these are report pages, not
/// exports. Every response says whether it hit the cap.
const MAX_ROWS: usize = 500;
/// Console lines are searched this far either side of a row for corroborating evidence.
const EVIDENCE_WINDOW_S: f64 = 120.0;
/// A heater is "off" below this target; crossing it upward starts a thermal session.
const HEATER_ON_C: f64 = 30.0;
/// Driver flag events are parsed, not text-matched; this bounds how many per request.
const MAX_DRIVER_EVENTS: usize = 2000;
/// A session's overshoot is measured for this long after the target is first reached.
const SETTLE_WINDOW_S: f64 = 180.0;
/// WO-15 bound: a heat-up is replayed from its switch-on only until this long has passed without
/// reaching the target. Longer is a fault or an abandoned target, not a heat-up to rate.
const MAX_HEAT_UP_S: f64 = 3600.0;
/// Target changes read per heater and request; reaching it is declared as `capped`.
const TARGET_CHANGES: usize = 4096;

fn sample_evidence(ts: f64) -> Value {
    json!({"kind":"sample","id":format!("{ts}"),"ts":ts})
}

fn row_evidence(kind: &str, rowid: i64, ts: f64) -> Value {
    json!({"kind":kind,"id":rowid.to_string(),"ts":ts})
}

/// Console lines near `ts` whose text matches `needles`, as evidence rows. Bounded by `limit`
/// so one chatty minute cannot dominate a report.
fn console_evidence(db: &Store, ts: f64, needles: &[&str], limit: usize) -> Result<Vec<Value>> {
    let mut statement = db
        .conn
        .prepare(
            "SELECT rowid, ts, message FROM events
             WHERE ts BETWEEN ?1 AND ?2 AND kind IN ('command','response')
             ORDER BY ABS(ts - ?3) LIMIT 64",
        )
        .map_err(crate::store::db_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![ts - EVIDENCE_WINDOW_S, ts + EVIDENCE_WINDOW_S, ts],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(crate::store::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::store::db_error)?;
    let mut out = Vec::new();
    for (rowid, at, message) in rows {
        let upper = message.to_ascii_uppercase();
        if needles.iter().any(|n| upper.contains(*n)) {
            out.push(row_evidence("console", rowid, at));
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

/// The value of a JSON pointer in force at `ts`, with the sample it was read from: the sample at or
/// before `ts` as load_sample rebuilds it. Used to seed a carried-forward series so the first rows
/// of a window are not unknown.
fn carried_from(db: &Store, pointer: &str, ts: f64) -> Result<Option<(f64, f64)>> {
    Ok(db
        .at(ts)?
        .and_then(|(at, sample)| sample.pointer(pointer)?.as_f64().map(|v| (at, v))))
}

/// Every value change of each JSON pointer in the window, ascending, first value included, at most
/// `limit` per pointer. One replay serves every pointer, narrowed to the rows that carry them.
fn changes(
    db: &Store,
    pointers: &[&str],
    from: f64,
    to: f64,
    limit: usize,
) -> Result<Vec<Vec<(f64, f64)>>> {
    let mut out: Vec<Vec<(f64, f64)>> = vec![Vec::new(); pointers.len()];
    db.replay(from, to, pointers, |ts, sample| {
        for (series, pointer) in out.iter_mut().zip(pointers) {
            let Some(value) = sample.pointer(pointer).and_then(Value::as_f64) else {
                continue;
            };
            if series.len() < limit && series.last().is_none_or(|(_, last)| *last != value) {
                series.push((ts, value));
            }
        }
        out.iter().any(|series| series.len() < limit)
    })?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// /api/history/mesh
// ---------------------------------------------------------------------------------------------

/// Klipper reports the probed grid row-major from the front-left corner. "Worst corner" is the
/// corner cell furthest from the mesh mean — for a Voron gantry that is the number that says
/// which side is dropping, and it is what a mesh report is actually read for.
fn mesh_row(ts: f64, profile: Option<String>, matrix: &Value, adaptive: bool) -> Option<Value> {
    let rows = matrix.as_array()?;
    if rows.is_empty() || rows.len() > 64 {
        return None;
    }
    let mut cells: Vec<Vec<f64>> = Vec::with_capacity(rows.len());
    for row in rows {
        let row = row.as_array()?;
        if row.is_empty() || row.len() > 64 {
            return None;
        }
        let values: Option<Vec<f64>> = row
            .iter()
            .map(|c| c.as_f64().filter(|v| v.is_finite()))
            .collect();
        cells.push(values?);
    }
    let width = cells[0].len();
    if cells.iter().any(|r| r.len() != width) {
        return None;
    }
    let flat: Vec<f64> = cells.iter().flatten().copied().collect();
    let min = flat.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = flat.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = flat.iter().sum::<f64>() / flat.len() as f64;
    let last_row = cells.len() - 1;
    let last_col = width - 1;
    let corners = [
        ("front left", 0usize, 0usize),
        ("front right", 0, last_col),
        ("back left", last_row, 0),
        ("back right", last_row, last_col),
    ];
    let worst = corners
        .iter()
        .map(|(name, r, c)| (*name, cells[*r][*c]))
        .max_by(|a, b| (a.1 - mean).abs().total_cmp(&(b.1 - mean).abs()))?;
    Some(json!({
        "ts": ts,
        "profile": profile,
        "adaptive": adaptive,
        "rows": cells.len(),
        "cols": width,
        "min_mm": min,
        "max_mm": max,
        "range_mm": max - min,
        "mean_mm": mean,
        "worst_corner": {"corner": worst.0, "z_mm": worst.1, "from_mean_mm": worst.1 - mean},
        "corners": corners.iter().map(|(n,r,c)| json!({"corner":n,"z_mm":cells[*r][*c]})).collect::<Vec<_>>()
    }))
}

fn mesh_history(db: &Store, from: f64, to: f64, limit: usize) -> Result<Value> {
    // WO-15: every change of bed_mesh.probed_matrix to a grid, replayed, so a mesh set (or
    // cleared) between two keyframes counts, dated by the sample that carried it. Klipper reports
    // `probed_matrix: [[]]` whenever no mesh is loaded, and on this printer that is most samples
    // (4326 of 4937 rows on 2026-09-20): never a mesh. Identical consecutive grids are the same
    // mesh still being reported, not a new one. `limit` counts meshes.
    let mut found: Vec<(f64, Value, Option<String>)> = Vec::new();
    let read = ["/bed_mesh/probed_matrix", "/bed_mesh/profile_name"];
    db.replay(from, to, &read, |ts, sample| {
        let mesh = &sample["bed_mesh"];
        let matrix = &mesh["probed_matrix"];
        if matrix[0].as_array().is_some_and(|row| !row.is_empty())
            && found.last().is_none_or(|(_, last, _)| last != matrix)
        {
            found.push((
                ts,
                matrix.clone(),
                mesh["profile_name"].as_str().map(str::to_owned),
            ));
        }
        true
    })?;
    let capped = found.len() > limit;
    let mut out = Vec::new();
    for (ts, matrix, profile) in found.into_iter().rev().take(limit) {
        // An adaptive mesh is named `adaptive-<hash>` (seen live: "adaptive-7FAC353990"), not
        // plain "adaptive", so an exact match never fired.
        let adaptive = profile
            .as_deref()
            .is_some_and(|p| p.starts_with("adaptive"));
        let Some(mut row) = mesh_row(ts, profile, &matrix, adaptive) else {
            continue;
        };
        let mut evidence = vec![sample_evidence(ts)];
        evidence.extend(console_evidence(
            db,
            ts,
            &["BED_MESH", "BED MESH", "MESH"],
            3,
        )?);
        row["evidence"] = Value::Array(evidence);
        out.push(row);
    }
    Ok(json!({
        "meshes": out,
        "capped": capped,
        "basis": "every change of bed_mesh.probed_matrix to a non-empty grid, rebuilt from keyframes and deltas and dated by the sample that first carried it; consecutive identical grids collapse to one mesh; newest first, at most `limit` meshes"
    }))
}

// ---------------------------------------------------------------------------------------------
// /api/history/z-offset
// ---------------------------------------------------------------------------------------------

fn z_offset_history(db: &Store, from: f64, to: f64, limit: usize) -> Result<Value> {
    // The recorder already classifies the console lines that move the offset; use its work
    // rather than re-parsing G-code here.
    let mut statement = db
        .conn
        .prepare(
            "SELECT rowid, ts, kind, message FROM events
             WHERE ts BETWEEN ?1 AND ?2 AND kind IN ('z_offset_change','z_health_alert')
             ORDER BY ts DESC LIMIT ?3",
        )
        .map_err(crate::store::db_error)?;
    let raw = statement
        .query_map(rusqlite::params![from, to, (limit + 1) as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(crate::store::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::store::db_error)?;
    let capped = raw.len() > limit;
    let mut out = Vec::new();
    for (rowid, ts, kind, message) in raw.into_iter().take(limit) {
        // The live offset the printer was actually running at the moment of the change.
        let applied = carried_from(db, "/gcode_move/homing_origin/2", ts)?;
        out.push(json!({
            "ts": ts,
            "kind": kind,
            "message": message,
            "applied_z_mm": applied.map(|(_, v)| v),
            "applied_from_ts": applied.map(|(t, _)| t),
            "evidence": [row_evidence("event", rowid, ts), sample_evidence(ts)]
        }));
    }
    // The zero-reference probe results are the independent check on all of the above: they are
    // measured, not commanded, so a drifting offset shows here even when nobody typed anything.
    let zero_references: Vec<Value> = db
        .probe_points(from, to, 200)?
        .into_iter()
        .filter(|p| crate::probe::is_zero_reference(p.x, p.y, &p.context))
        .map(|p| {
            json!({"ts":p.ts,"z_mm":p.z,"context":p.context,"evidence":[sample_evidence(p.ts)]})
        })
        .collect();
    Ok(json!({
        "changes": out,
        "zero_references": zero_references,
        "capped": capped,
        "basis": "recorder z_offset_change / z_health_alert events, with the live gcode_move.homing_origin[2] at each change and the measured zero-reference probes beside them"
    }))
}

// ---------------------------------------------------------------------------------------------
// /api/history/qgl
// ---------------------------------------------------------------------------------------------

fn qgl_history(db: &Store, cfg: &Config, from: f64, to: f64, limit: usize) -> Result<Value> {
    let mut statement = db
        .conn
        .prepare(
            "SELECT rowid, ts, run_id, round, range_mm, z0, z1, z2, z3 FROM qgl
             WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts DESC LIMIT ?3",
        )
        .map_err(crate::store::db_error)?;
    #[allow(clippy::type_complexity)]
    let raw: Vec<(i64, f64, i64, i64, f64, [Option<f64>; 4])> = statement
        .query_map(rusqlite::params![from, to, (limit + 1) as i64], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
            ))
        })
        .map_err(crate::store::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::store::db_error)?;
    let capped = raw.len() > limit;

    // Group into runs, newest first, keeping each round's own rowid as evidence.
    let mut runs: BTreeMap<i64, Vec<(i64, f64, i64, f64, [Option<f64>; 4])>> = BTreeMap::new();
    for (rowid, ts, run_id, round, range_mm, z) in raw.into_iter().take(limit) {
        runs.entry(run_id)
            .or_default()
            .push((rowid, ts, round, range_mm, z));
    }
    let mut out = Vec::new();
    for (run_id, mut rounds) in runs.into_iter().rev() {
        rounds.sort_by_key(|r| r.2);
        let first_ts = rounds.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
        let last = rounds.last().cloned();
        let last_ts = rounds.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max);
        // The tolerance in force when the run happened, not today's, if one was recorded.
        let tolerance = db
            .context_at(last_ts, "scope_geometry")?
            .and_then(|v| v["qgl_tolerance_mm"].as_f64())
            .filter(|t| t.is_finite() && *t > 0.0)
            .unwrap_or(cfg.qgl_tolerance_mm);
        let final_range = last.as_ref().map(|r| r.3);
        let mut evidence: Vec<Value> = rounds
            .iter()
            .map(|(rowid, ts, _, _, _)| row_evidence("qgl", *rowid, *ts))
            .collect();
        evidence.push(sample_evidence(last_ts));
        // A run that gave up says so in the console; that line is the evidence for a failure.
        let mut failures = db
            .conn
            .prepare(
                "SELECT rowid, ts FROM events WHERE ts BETWEEN ?1 AND ?2
                 AND kind='levelling_failed' ORDER BY ts LIMIT 4",
            )
            .map_err(crate::store::db_error)?;
        let aborted = failures
            .query_map(rusqlite::params![first_ts - 1.0, last_ts + 60.0], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(crate::store::db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(crate::store::db_error)?;
        for (rowid, ts) in &aborted {
            evidence.push(row_evidence("event", *rowid, *ts));
        }
        out.push(json!({
            "run_id": run_id,
            "first_ts": first_ts,
            "last_ts": last_ts,
            "rounds": rounds.iter().map(|(rowid, ts, round, range_mm, z)| json!({
                "rowid": rowid, "ts": ts, "round": round, "range_mm": range_mm,
                "z0": z[0], "z1": z[1], "z2": z[2], "z3": z[3],
                "pass": *range_mm <= tolerance
            })).collect::<Vec<_>>(),
            "round_count": rounds.len(),
            "final_range_mm": final_range,
            "tolerance_mm": tolerance,
            "pass": final_range.map(|r| r <= tolerance),
            "aborted": !aborted.is_empty(),
            "evidence": evidence
        }));
    }
    Ok(json!({
        "runs": out,
        "capped": capped,
        "basis": "parsed QUAD_GANTRY_LEVEL rounds grouped by run, against the levelling tolerance recorded at the time"
    }))
}

// ---------------------------------------------------------------------------------------------
// /api/history/motor-drift
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct MotorFlags {
    count: i64,
    evidence: Vec<Value>,
    flags: BTreeSet<String>,
}

/// The recorder writes one `driver_warning` / `driver_status` event whose message lists EVERY
/// motor, set flags or not: `Driver warning state changed: [{"flags":[],"motor":"stepper_x"},
/// ...]`. Matching the motor name as text therefore credits every event to all seven — the
/// first cut of this file did exactly that and reported an identical 6843 for each. Parse the
/// array once and tally only the motors whose flags were actually raised.
fn driver_flag_events(
    db: &Store,
    from: f64,
    to: f64,
) -> Result<(BTreeMap<String, MotorFlags>, bool)> {
    let mut statement = db
        .conn
        .prepare(
            "SELECT rowid, ts, message FROM events WHERE ts BETWEEN ?1 AND ?2
             AND kind IN ('driver_warning','driver_status') ORDER BY ts DESC LIMIT ?3",
        )
        .map_err(crate::store::db_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![from, to, (MAX_DRIVER_EVENTS + 1) as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(crate::store::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::store::db_error)?;
    let truncated = rows.len() > MAX_DRIVER_EVENTS;
    let mut out: BTreeMap<String, MotorFlags> = BTreeMap::new();
    for (rowid, ts, message) in rows.into_iter().take(MAX_DRIVER_EVENTS) {
        let Some(open) = message.find('[') else {
            continue;
        };
        let Ok(list) = serde_json::from_str::<Value>(&message[open..]) else {
            continue;
        };
        let Some(list) = list.as_array() else {
            continue;
        };
        for item in list {
            let (Some(motor), Some(flags)) = (item["motor"].as_str(), item["flags"].as_array())
            else {
                continue;
            };
            if flags.is_empty() {
                continue;
            }
            let entry = out.entry(motor.to_owned()).or_default();
            entry.count += 1;
            if entry.evidence.len() < 4 {
                entry.evidence.push(row_evidence("event", rowid, ts));
            }
            for flag in flags {
                if let Some(flag) = flag.as_str() {
                    entry.flags.insert(flag.to_owned());
                }
            }
        }
    }
    Ok((out, truncated))
}

/// Drift here is movement in the TMC driver's actual current scaling (`drv_status.cs_actual`,
/// 0..31). It tracks how hard a driver is working: a motor whose cs_actual climbs across
/// otherwise identical jobs is fighting more friction than it used to. `run_current` changes
/// are reported beside it so an operator edit is never mistaken for mechanical drift.
fn motor_drift_history(db: &Store, from: f64, to: f64, limit: usize) -> Result<Value> {
    let mut motors = Vec::new();
    let (raised, mut any_capped) = driver_flag_events(db, from, to)?;
    // One replay for all seven drivers: cs_actual and run_current, per motor, in MOTORS order.
    let pointers: Vec<String> = model::MOTORS
        .iter()
        .flat_map(|(name, _)| {
            [
                format!("/tmc2209 {name}/drv_status/cs_actual"),
                format!("/tmc2209 {name}/run_current"),
            ]
        })
        .collect();
    let pointers: Vec<&str> = pointers.iter().map(String::as_str).collect();
    let mut all = changes(db, &pointers, from, to, limit + 1)?.into_iter();
    for (name, label) in model::MOTORS {
        let series = all.next().unwrap_or_default();
        let current_changes: Vec<(f64, f64)> = all
            .next()
            .unwrap_or_default()
            .into_iter()
            .take(32)
            .collect();
        any_capped |= series.len() > limit;
        let series: Vec<(f64, f64)> = series.into_iter().take(limit).collect();
        let seed = carried_from(db, &format!("/tmc2209 {name}/drv_status/cs_actual"), from)?;

        // No report inside the window means the value never changed there; fall back to the
        // last one before it rather than calling the motor unknown.
        let first = series.first().copied().or(seed);
        let last = series.last().copied();
        let (min, max) = series
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, v)| {
                (lo.min(*v), hi.max(*v))
            });
        // The single largest step between consecutive reports: where to look first.
        let biggest = series
            .windows(2)
            .map(|w| (w[1].0, w[1].1 - w[0].1))
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        let raised = raised.get(name);
        let mut evidence = Vec::new();
        if let Some((ts, _)) = series.first() {
            evidence.push(sample_evidence(*ts));
        }
        if let Some((ts, _)) = biggest {
            evidence.push(sample_evidence(ts));
        }
        for (ts, _) in &current_changes {
            evidence.push(sample_evidence(*ts));
        }
        if let Some(raised) = raised {
            evidence.extend(raised.evidence.iter().cloned());
        }
        motors.push(json!({
            "motor": name,
            "label": label,
            "samples": series.len(),
            "first": first.map(|(ts, v)| json!({"ts":ts,"cs_actual":v})),
            "last": last.map(|(ts, v)| json!({"ts":ts,"cs_actual":v})),
            "drift": match (first, last) { (Some(a), Some(b)) => json!(b.1 - a.1), _ => Value::Null },
            "min_cs_actual": if min.is_finite() { json!(min) } else { Value::Null },
            "max_cs_actual": if max.is_finite() { json!(max) } else { Value::Null },
            "largest_step": biggest.map(|(ts, delta)| json!({"ts":ts,"change":delta})),
            "run_current_changes": current_changes.iter()
                .map(|(ts, a)| json!({"ts":ts,"run_current":a})).collect::<Vec<_>>(),
            "flag_events": raised.map_or(0, |r| r.count),
            "flags_seen": raised.map(|r| r.flags.iter().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "evidence": evidence
        }));
    }
    Ok(json!({
        "motors": motors,
        "capped": any_capped,
        "driver_events_scanned": MAX_DRIVER_EVENTS,
        "basis": "drv_status.cs_actual changes in the recording, with run_current edits beside them, and driver flag events attributed to the motor that actually raised a flag",
        "note": "cs_actual is the driver's own 0..31 current scaling, not a measured current; a motor with no changes in the window never moved its scaling there, which is a real answer and not a gap"
    }))
}

// ---------------------------------------------------------------------------------------------
// /api/history/thermal
// ---------------------------------------------------------------------------------------------

/// One heat-up: target set above HEATER_ON_C, temperature climbing to meet it. Reports the
/// climb rate over the middle of the ramp and the overshoot once the target is first touched.
fn thermal_sessions(
    db: &Store,
    heater: &str,
    targets: Vec<(f64, f64)>,
    from: f64,
    to: f64,
    limit: usize,
) -> Result<(Vec<Value>, bool)> {
    let target_at = format!("/{heater}/target");
    let temperature_at = format!("/{heater}/temperature");
    let capped = targets.len() >= TARGET_CHANGES;

    // A session starts at the moment the TARGET was commanded, not at the next temperature
    // report: the ramp's clock starts when the operator asked for heat.
    let seeded = carried_from(db, &target_at, from)?;
    let mut previous = seeded.map_or(0.0, |(_, v)| v);
    let mut switch_ons: Vec<(f64, f64)> = Vec::new();
    for (ts, target) in &targets {
        if *target >= HEATER_ON_C && previous < HEATER_ON_C {
            switch_ons.push((*ts, *target));
        }
        previous = *target;
    }

    let mut sessions: Vec<Value> = Vec::new();
    for (start_ts, session_target) in switch_ons {
        // A later target change closes the window: a rate measured across two setpoints is a
        // number about nothing, so an abandoned or re-aimed heat-up is simply not reported.
        let next_change = targets
            .iter()
            .find(|(ts, t)| *ts > start_ts && (t - session_target).abs() > 0.5)
            .map(|(ts, _)| *ts)
            .unwrap_or(f64::INFINITY);
        // WO-15: only this heat-up's own stretch is replayed, from the switch-on sample (it carries
        // the target change, so it is the first one visited) to the settle window after the target
        // is reached; a heater still short after MAX_HEAT_UP_S, or re-aimed first, is not reported.
        let (mut start_c, mut reached, mut peak) = (None, None::<(f64, f64)>, None::<(f64, f64)>);
        db.replay(
            start_ts,
            to,
            &[&temperature_at, &target_at],
            |ts, sample| {
                let Some(c) = sample.pointer(&temperature_at).and_then(Value::as_f64) else {
                    return true;
                };
                start_c.get_or_insert(c);
                match reached {
                    None if ts >= next_change || ts - start_ts > MAX_HEAT_UP_S => false,
                    None => {
                        if c + 0.5 >= session_target {
                            reached = Some((ts, c));
                            peak = reached;
                        }
                        true
                    }
                    // Overshoot is the peak inside the settle window that follows.
                    Some((reached_ts, _)) if ts > reached_ts + SETTLE_WINDOW_S => false,
                    Some(_) => {
                        if peak.is_some_and(|(_, best)| c > best) {
                            peak = Some((ts, c));
                        }
                        true
                    }
                }
            },
        )?;
        let (Some(start_c), Some((reached_ts, _)), Some(peak)) = (start_c, reached, peak) else {
            continue;
        };
        let seconds = reached_ts - start_ts;
        let rate = if seconds > 0.0 {
            (session_target - start_c) / seconds
        } else {
            0.0
        };
        sessions.push(json!({
            "heater": heater,
            "ts": start_ts,
            "start_c": start_c,
            "target_c": session_target,
            "reached_ts": reached_ts,
            "seconds_to_target": seconds,
            "heat_up_c_per_s": rate,
            "peak_c": peak.1,
            "peak_ts": peak.0,
            "overshoot_c": (peak.1 - session_target).max(0.0),
            "settle_window_s": SETTLE_WINDOW_S,
            "evidence": [sample_evidence(start_ts), sample_evidence(reached_ts), sample_evidence(peak.0)]
        }));
        if sessions.len() >= limit {
            break;
        }
    }
    sessions.reverse();
    Ok((sessions, capped))
}

fn thermal_history(db: &Store, from: f64, to: f64, limit: usize) -> Result<Value> {
    // One replay reads both heaters' targets; each heat-up then replays only its own stretch.
    let pointers = ["/extruder/target", "/heater_bed/target"];
    let mut targets = changes(db, &pointers, from, to, TARGET_CHANGES)?.into_iter();
    let extruder = targets.next().unwrap_or_default();
    let (mut sessions, mut capped) = thermal_sessions(db, "extruder", extruder, from, to, limit)?;
    let bed = targets.next().unwrap_or_default();
    let (bed, bed_capped) = thermal_sessions(db, "heater_bed", bed, from, to, limit)?;
    capped |= bed_capped;
    sessions.extend(bed);
    sessions.sort_by(|a, b| {
        b["ts"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["ts"].as_f64().unwrap_or(0.0))
    });
    sessions.truncate(limit);
    // Drift is the point of the page: is this heater slower than it used to be?
    let mut drift = serde_json::Map::new();
    for heater in ["extruder", "heater_bed"] {
        let rates: Vec<f64> = sessions
            .iter()
            .filter(|s| s["heater"] == heater)
            .filter_map(|s| s["heat_up_c_per_s"].as_f64())
            .filter(|r| r.is_finite() && *r > 0.0)
            .collect();
        if rates.is_empty() {
            continue;
        }
        let mean = rates.iter().sum::<f64>() / rates.len() as f64;
        // `sessions` is newest-first, so rates[0] is the most recent heat-up.
        drift.insert(
            heater.into(),
            json!({
                "sessions": rates.len(),
                "mean_c_per_s": mean,
                "latest_c_per_s": rates[0],
                "latest_vs_mean_pct": if mean > 0.0 { (rates[0] / mean - 1.0) * 100.0 } else { 0.0 }
            }),
        );
    }
    Ok(json!({
        "sessions": sessions,
        "drift": Value::Object(drift),
        "capped": capped,
        "basis": "each heat-up from the target crossing 30 C upward to the temperature first reaching it, with the peak in the 180 s that follow as overshoot"
    }))
}

// ---------------------------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------------------------

/// WO-13: every finished print, newest first, straight from the additive print_sessions table the
/// recorder fills at write time. No reconstruction, no extrapolation: a row exists only for a
/// transition the recorder actually saw, and the ones it joined mid-run say so.
fn prints_history(db: &Store, from: f64, to: f64, limit: usize) -> Result<Value> {
    let sessions = db.print_sessions(from, to, limit)?;
    let mut out = Vec::with_capacity(sessions.len());
    for s in sessions {
        out.push(json!({
            "start_ts": s.start_ts, "end_ts": s.end_ts, "filename": s.filename,
            "duration_s": s.duration_s, "filament_used": s.filament_used,
            "result": s.result, "progress": s.progress, "evidence_note": s.evidence,
            "evidence": [sample_evidence(s.start_ts), sample_evidence(s.end_ts)],
        }));
    }
    Ok(json!({"prints":out,
        "basis":"one row per print the recorder saw end; prints joined mid-run carry their caveat in evidence_note"}))
}

pub fn handle(
    method: &str,
    target: &str,
    db: &Store,
    cfg: &Config,
    _status: &CollectorStatus,
) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"history is read-only; only GET is allowed"}),
        );
    }
    let (path, q) = match super::query(target) {
        Ok(v) => v,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let clock = model::now();
    let parsed = (|| -> Result<(f64, f64, usize)> {
        for key in q.keys() {
            if !["from", "to", "limit"].contains(&key.as_str()) {
                return Err(format!(
                    "unknown parameter {key}; history accepts from, to and limit"
                ));
            }
        }
        let from = super::number(&q, "from", Some(0.0))?;
        let to = super::number(&q, "to", Some(clock))?;
        if from > to {
            return Err("from must not be later than to; reverse the time window".into());
        }
        let limit = match q.get("limit") {
            None => MAX_ROWS.min(cfg.qgl_limit),
            Some(s) => match s.parse::<usize>() {
                Ok(n) if (1..=MAX_ROWS).contains(&n) => n,
                _ => return Err(format!("limit must be an integer from 1 to {MAX_ROWS}")),
            },
        };
        Ok((from, to, limit))
    })();
    let (from, to, limit) = match parsed {
        Ok(v) => v,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    // Read-consistency, as every other analysis route does.
    let Ok(_snapshot) = db.conn.unchecked_transaction() else {
        return Response::json(
            500,
            json!({"error":"cannot open a read snapshot of the recording; is the database file readable?"}),
        );
    };
    let built = match path {
        "/api/history/mesh" => mesh_history(db, from, to, limit),
        "/api/history/z-offset" => z_offset_history(db, from, to, limit),
        "/api/history/qgl" => qgl_history(db, cfg, from, to, limit),
        "/api/history/motor-drift" => motor_drift_history(db, from, to, limit),
        "/api/history/thermal" => thermal_history(db, from, to, limit),
        "/api/history/prints" => prints_history(db, from, to, limit),
        _ => {
            return Response::json(
                404,
                json!({"error":"unknown history; use mesh, z-offset, qgl, motor-drift, thermal or prints"}),
            )
        }
    };
    match built {
        Ok(mut value) => {
            if let Some(map) = value.as_object_mut() {
                map.insert("from".into(), json!(from));
                map.insert("to".into(), json!(to));
                map.insert("server_time".into(), json!(clock));
                map.insert(
                    "evidence_note".into(),
                    json!("every evidence entry is {kind,id,ts}; open it with GET /api/at?ts=<ts>"),
                );
            }
            Response::json(200, value)
        }
        Err(e) => Response::json(500, json!({"error":e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qgl::Parser;

    fn store() -> Store {
        Store::open(":memory:", false).unwrap()
    }
    fn put(db: &mut Store, ts: f64, data: Value) {
        db.write(Some((ts, &data)), &[], &mut Parser::default())
            .unwrap();
    }
    fn body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }
    fn get(db: &Store, target: &str) -> Response {
        handle(
            "GET",
            target,
            db,
            &Config::default(),
            &CollectorStatus::default(),
        )
    }

    /// WO-13: the prints history is the recorder's own session rows, newest first, each with
    /// resolvable evidence and the honest caveat when the recorder joined mid-run.
    #[test]
    fn print_history_returns_recorded_sessions_with_evidence() {
        let mut db = store();
        put(
            &mut db,
            10.0,
            json!({"print_stats":{"state":"standby","filename":""}}),
        );
        put(
            &mut db,
            20.0,
            json!({"print_stats":{"state":"printing","filename":"cube.gcode","print_duration":60.0,"filament_used":1250.0},"virtual_sdcard":{"progress":0.5}}),
        );
        // WO-15: sampled up to the end, as a print the recorder watched through is; the 60 s of
        // silence this fixture had before the completion now reads, correctly, as a gap.
        put(
            &mut db,
            70.0,
            json!({"print_stats":{"state":"printing","filename":"cube.gcode","print_duration":700.0,"filament_used":1850.0},"virtual_sdcard":{"progress":0.97}}),
        );
        put(
            &mut db,
            80.0,
            json!({"print_stats":{"state":"complete","filename":"cube.gcode","print_duration":720.0,"filament_used":1880.0},"virtual_sdcard":{"progress":1.0}}),
        );
        put(
            &mut db,
            90.0,
            json!({"print_stats":{"state":"standby","filename":""}}),
        );
        let value = body(&get(&db, "/api/history/prints"));
        let prints = value["prints"].as_array().expect("prints array");
        assert_eq!(prints.len(), 1, "one print, one row: {prints:?}");
        assert_eq!(prints[0]["filename"], "cube.gcode");
        assert_eq!(prints[0]["result"], "complete");
        assert_eq!(prints[0]["start_ts"], json!(20.0));
        assert_eq!(prints[0]["end_ts"], json!(80.0));
        assert_eq!(prints[0]["duration_s"], json!(720.0));
        assert_eq!(prints[0]["filament_used"], json!(1880.0));
        assert!(prints[0]["evidence_note"]
            .as_str()
            .unwrap()
            .contains("both recorded"));
        check_evidence(&prints[0]);
        // Read-only, like every history route.
        assert_eq!(
            handle(
                "POST",
                "/api/history/prints",
                &db,
                &Config::default(),
                &CollectorStatus::default()
            )
            .code,
            405
        );
        // An interrupted print (service stopped before a terminal state) is recorded as such,
        // not invented into a completion.
        put(
            &mut db,
            100.0,
            json!({"print_stats":{"state":"printing","filename":"second.gcode","print_duration":10.0},"virtual_sdcard":{"progress":0.1}}),
        );
        put(
            &mut db,
            110.0,
            json!({"print_stats":{"state":"standby","filename":""}}),
        );
        let value = body(&get(&db, "/api/history/prints"));
        let prints = value["prints"].as_array().unwrap();
        assert_eq!(prints.len(), 2);
        assert_eq!(prints[0]["result"], "interrupted");
    }
    /// Every evidence entry must carry the three keys a report page resolves with.
    fn check_evidence(rows: &Value) {
        let list = rows["evidence"].as_array().expect("row has no evidence");
        assert!(!list.is_empty(), "row has empty evidence: {rows}");
        for item in list {
            assert!(item["kind"].is_string(), "evidence kind: {item}");
            assert!(
                item["id"].is_string(),
                "evidence id must be a string: {item}"
            );
            assert!(
                item["ts"].as_f64().is_some_and(|t| t.is_finite()),
                "evidence ts: {item}"
            );
        }
    }

    fn mesh(offset: f64) -> Value {
        json!({"bed_mesh":{"profile_name":"default","probed_matrix":[
            [0.00 + offset, 0.02, 0.05],
            [0.01, 0.00, 0.03],
            [-0.04, 0.01, 0.20]
        ]}})
    }

    #[test]
    fn mesh_history_reports_range_worst_corner_and_evidence() {
        let mut db = store();
        put(&mut db, 100.0, mesh(0.0));
        put(&mut db, 200.0, mesh(0.0)); // identical grid: the same mesh, not a second one
        put(&mut db, 300.0, mesh(-0.30));
        let value = body(&get(&db, "/api/history/mesh"));
        let meshes = value["meshes"].as_array().unwrap();
        assert_eq!(
            meshes.len(),
            2,
            "identical consecutive grids collapse: {value}"
        );
        let newest = &meshes[0];
        assert_eq!(newest["ts"], 300.0);
        assert_eq!(newest["rows"], 3);
        assert_eq!(newest["cols"], 3);
        // -0.30 front left against a 0.20 back right corner.
        assert!((newest["range_mm"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(newest["worst_corner"]["corner"], "front left");
        assert!((newest["worst_corner"]["z_mm"].as_f64().unwrap() + 0.30).abs() < 1e-9);
        assert_eq!(newest["corners"].as_array().unwrap().len(), 4);
        for row in meshes {
            check_evidence(row);
            assert_eq!(row["evidence"][0]["kind"], "sample");
        }
        assert_eq!(value["from"], 0.0);
        assert!(value["evidence_note"].is_string());
    }

    #[test]
    fn mesh_history_rejects_a_ragged_grid_instead_of_averaging_it() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"bed_mesh":{"profile_name":"bad","probed_matrix":[[0.0,0.1],[0.2]]}}),
        );
        let value = body(&get(&db, "/api/history/mesh"));
        assert!(
            value["meshes"].as_array().unwrap().is_empty(),
            "a ragged grid must be dropped, not reported: {value}"
        );
    }

    #[test]
    fn an_unloaded_mesh_is_skipped_in_sql_so_the_limit_counts_meshes() {
        let mut db = store();
        // Klipper reports [[]] with an empty profile name whenever no mesh is loaded. On the
        // live recording those outnumber real meshes 7:1 and sit at the newest timestamps, so
        // filtering them after LIMIT returned an empty page.
        // The recorder forces monotonic timestamps, so the real mesh goes in first and the
        // blanks after it — which is also the live ordering that broke the first cut.
        put(&mut db, 100.0, mesh(0.0));
        for i in 0..8 {
            put(
                &mut db,
                200.0 + i as f64,
                json!({"bed_mesh":{"profile_name":"","probed_matrix":[[]]}}),
            );
        }
        let value = body(&get(&db, "/api/history/mesh?limit=2"));
        let meshes = value["meshes"].as_array().unwrap();
        assert_eq!(
            meshes.len(),
            1,
            "the real mesh survives eight blanks: {value}"
        );
        assert_eq!(meshes[0]["ts"], 100.0);
        assert_eq!(value["capped"], false);
        // An adaptive mesh is named adaptive-<hash>, not "adaptive".
        put(
            &mut db,
            300.0,
            json!({"bed_mesh":{"profile_name":"adaptive-7FAC353990","probed_matrix":[[0.0,0.1],[0.2,0.3]]}}),
        );
        let value = body(&get(&db, "/api/history/mesh?limit=5"));
        let newest = &value["meshes"][0];
        assert_eq!(newest["profile"], "adaptive-7FAC353990");
        assert_eq!(newest["adaptive"], true);
    }

    #[test]
    fn z_offset_history_pairs_each_change_with_the_live_offset() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"gcode_move":{"homing_origin":[0.0,0.0,-0.035]}}),
        );
        put(
            &mut db,
            200.0,
            json!({"gcode_move":{"homing_origin":[0.0,0.0,-0.085]}}),
        );
        db.conn
            .execute(
                "INSERT INTO events(ts,kind,message) VALUES(205,'z_offset_change','SET_GCODE_OFFSET Z=-0.085')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO events(ts,kind,message) VALUES(210,'z_health_alert','zero reference drifted 0.06 mm')",
                [],
            )
            .unwrap();
        let value = body(&get(&db, "/api/history/z-offset"));
        let changes = value["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["kind"], "z_health_alert");
        let applied = &changes[1];
        assert_eq!(applied["kind"], "z_offset_change");
        assert!((applied["applied_z_mm"].as_f64().unwrap() + 0.085).abs() < 1e-9);
        assert_eq!(applied["applied_from_ts"], 200.0);
        for row in changes {
            check_evidence(row);
            assert_eq!(row["evidence"][0]["kind"], "event");
        }
    }

    #[test]
    fn qgl_history_groups_runs_and_names_every_round_as_evidence() {
        let db = store();
        for (ts, run, round, range) in [
            (100.0, 1, 0, 0.30),
            (140.0, 1, 1, 0.08),
            (180.0, 1, 2, 0.011),
            (500.0, 2, 0, 0.40),
        ] {
            db.conn
                .execute(
                    "INSERT INTO qgl(ts,run_id,round,range_mm,z0,z1,z2,z3) VALUES(?1,?2,?3,?4,0.1,0.2,0.3,0.4)",
                    rusqlite::params![ts, run, round, range],
                )
                .unwrap();
        }
        db.conn
            .execute(
                "INSERT INTO events(ts,kind,message) VALUES(520,'levelling_failed','Retries aborting: ...')",
                [],
            )
            .unwrap();
        let value = body(&get(&db, "/api/history/qgl"));
        let runs = value["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        // Newest run first.
        assert_eq!(runs[0]["run_id"], 2);
        assert_eq!(runs[0]["aborted"], true);
        assert_eq!(runs[0]["pass"], false);
        let good = &runs[1];
        assert_eq!(good["run_id"], 1);
        assert_eq!(good["round_count"], 3);
        assert!((good["final_range_mm"].as_f64().unwrap() - 0.011).abs() < 1e-9);
        assert_eq!(good["tolerance_mm"], 0.02);
        assert_eq!(good["pass"], true);
        assert_eq!(good["aborted"], false);
        assert_eq!(good["rounds"][0]["pass"], false, "0.30 mm is outside 0.02");
        assert_eq!(good["rounds"][2]["pass"], true);
        for run in runs {
            check_evidence(run);
        }
        // Every round contributes a qgl evidence pointer, so a claim maps to a stored row.
        let kinds: Vec<&str> = good["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["kind"].as_str())
            .collect();
        assert_eq!(kinds.iter().filter(|k| **k == "qgl").count(), 3);
    }

    #[test]
    fn motor_drift_reports_every_motor_and_finds_the_biggest_step() {
        let mut db = store();
        let driver = |cs: i64, current: f64| json!({"tmc2209 stepper_x":{"run_current":current,"drv_status":{"cs_actual":cs}}});
        put(&mut db, 100.0, driver(12, 0.8));
        put(&mut db, 200.0, driver(13, 0.8));
        put(&mut db, 300.0, driver(22, 0.8)); // the step that matters
        put(&mut db, 400.0, driver(23, 1.1)); // and an operator current change
                                              // The real message shape: every motor listed, only some with flags raised. Counting by
                                              // text match would credit this event to all seven motors.
        db.conn
            .execute(
                r#"INSERT INTO events(ts,kind,message) VALUES(350,'driver_warning',
                   'Driver warning state changed: [{"flags":["otpw"],"motor":"stepper_x"},{"flags":[],"motor":"stepper_y"},{"flags":[],"motor":"stepper_z"}]')"#,
                [],
            )
            .unwrap();
        let value = body(&get(&db, "/api/history/motor-drift"));
        let motors = value["motors"].as_array().unwrap();
        assert_eq!(motors.len(), model::MOTORS.len(), "every motor gets a row");
        let x = motors
            .iter()
            .find(|m| m["motor"] == "stepper_x")
            .expect("stepper_x row");
        assert_eq!(x["samples"], 4);
        assert_eq!(x["first"]["cs_actual"], 12.0);
        assert_eq!(x["last"]["cs_actual"], 23.0);
        assert_eq!(x["drift"], 11.0);
        assert_eq!(x["min_cs_actual"], 12.0);
        assert_eq!(x["max_cs_actual"], 23.0);
        assert_eq!(x["largest_step"]["ts"], 300.0);
        assert_eq!(x["largest_step"]["change"], 9.0);
        assert_eq!(x["flag_events"], 1);
        assert_eq!(x["flags_seen"], json!(["otpw"]));
        assert_eq!(x["run_current_changes"].as_array().unwrap().len(), 2);
        check_evidence(x);
        assert!(
            x["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "event"),
            "the driver warning must be cited: {x}"
        );
        // A motor with nothing recorded says so rather than inventing a zero, and a motor named
        // in that event with NO flags raised is not charged with it.
        let y = motors.iter().find(|m| m["motor"] == "stepper_y").unwrap();
        assert_eq!(y["samples"], 0);
        assert!(y["drift"].is_null());
        assert!(y["min_cs_actual"].is_null());
        assert_eq!(
            y["flag_events"], 0,
            "a motor listed with empty flags did not raise anything: {y}"
        );
        assert_eq!(y["flags_seen"], json!([]));
    }

    #[test]
    fn thermal_history_measures_heat_up_rate_and_overshoot() {
        let mut db = store();
        // Idle at 25 C, then a 225 C target reached in 100 s, overshooting to 228.
        put(
            &mut db,
            100.0,
            json!({"extruder":{"temperature":25.0,"target":0.0}}),
        );
        put(
            &mut db,
            110.0,
            json!({"extruder":{"temperature":25.0,"target":225.0}}),
        );
        for (i, celsius) in [60.0, 120.0, 180.0, 224.6, 225.0].iter().enumerate() {
            put(
                &mut db,
                120.0 + i as f64 * 20.0,
                json!({"extruder":{"temperature":celsius}}),
            );
        }
        put(&mut db, 230.0, json!({"extruder":{"temperature":228.0}}));
        put(&mut db, 260.0, json!({"extruder":{"temperature":225.5}}));
        let value = body(&get(&db, "/api/history/thermal"));
        let sessions = value["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1, "one heat-up: {value}");
        let s = &sessions[0];
        assert_eq!(s["heater"], "extruder");
        assert_eq!(s["ts"], 110.0);
        assert_eq!(s["target_c"], 225.0);
        assert_eq!(s["start_c"], 25.0);
        // 224.6 at t=180 is inside the 0.5 C tolerance, so that is when the target was reached —
        // not the later exact 225.0. Waiting for the exact number would inflate every rate.
        assert_eq!(s["reached_ts"], 180.0);
        assert_eq!(s["seconds_to_target"], 70.0);
        let rate = s["heat_up_c_per_s"].as_f64().unwrap();
        assert!(
            (rate - (225.0 - 25.0) / 70.0).abs() < 1e-9,
            "heat-up rate {rate}"
        );
        assert_eq!(s["peak_c"], 228.0);
        assert_eq!(s["peak_ts"], 230.0);
        assert!((s["overshoot_c"].as_f64().unwrap() - 3.0).abs() < 1e-9);
        check_evidence(s);
        let drift = &value["drift"]["extruder"];
        assert_eq!(drift["sessions"], 1);
        assert!((drift["latest_c_per_s"].as_f64().unwrap() - rate).abs() < 1e-12);
    }

    #[test]
    fn thermal_ignores_a_target_that_is_never_reached() {
        let mut db = store();
        put(
            &mut db,
            100.0,
            json!({"heater_bed":{"temperature":22.0,"target":0.0}}),
        );
        put(
            &mut db,
            110.0,
            json!({"heater_bed":{"temperature":22.0,"target":100.0}}),
        );
        put(&mut db, 150.0, json!({"heater_bed":{"temperature":40.0}}));
        // Cancelled before it got there.
        put(
            &mut db,
            160.0,
            json!({"heater_bed":{"temperature":41.0,"target":0.0}}),
        );
        let value = body(&get(&db, "/api/history/thermal"));
        assert!(
            value["sessions"].as_array().unwrap().is_empty(),
            "an abandoned heat-up is not a session: {value}"
        );
        assert!(value["drift"].as_object().unwrap().is_empty());
    }

    #[test]
    fn every_history_is_read_only_and_validates_its_query() {
        let mut db = store();
        put(&mut db, 100.0, mesh(0.0));
        let before = db.conn.total_changes();
        for name in ["mesh", "z-offset", "qgl", "motor-drift", "thermal"] {
            let path = format!("/api/history/{name}");
            assert_eq!(get(&db, &path).code, 200, "{name}");
            assert_eq!(
                handle(
                    "POST",
                    &path,
                    &db,
                    &Config::default(),
                    &CollectorStatus::default()
                )
                .code,
                405,
                "{name}"
            );
            assert_eq!(get(&db, &format!("{path}?bogus=1")).code, 400, "{name}");
            assert_eq!(get(&db, &format!("{path}?limit=0")).code, 400, "{name}");
            assert_eq!(
                get(&db, &format!("{path}?from=900&to=100")).code,
                400,
                "{name}"
            );
            assert_eq!(
                get(&db, &format!("{path}?from=0&to=99999&limit=5")).code,
                200
            );
        }
        assert_eq!(get(&db, "/api/history/nothing").code, 404);
        assert_eq!(
            db.conn.total_changes(),
            before,
            "a history request wrote to the recording"
        );
    }

    #[test]
    fn the_window_is_honoured_and_the_cap_is_declared() {
        let mut db = store();
        for i in 0..6 {
            put(&mut db, 100.0 + i as f64 * 10.0, mesh(i as f64 * 0.01));
        }
        let all = body(&get(&db, "/api/history/mesh"));
        assert_eq!(all["meshes"].as_array().unwrap().len(), 6);
        assert_eq!(all["capped"], false);
        // Meshes sit at 100,110,120,130,140,150; the window holds the last three.
        let windowed = body(&get(&db, "/api/history/mesh?from=130&to=160"));
        assert_eq!(windowed["meshes"].as_array().unwrap().len(), 3);
        assert_eq!(windowed["from"], 130.0);
        assert!(windowed["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["ts"]
                .as_f64()
                .is_some_and(|t| (130.0..=160.0).contains(&t))));
        let capped = body(&get(&db, "/api/history/mesh?limit=2"));
        assert_eq!(capped["meshes"].as_array().unwrap().len(), 2);
        assert_eq!(capped["capped"], true, "a truncated history must say so");
    }

    /// A sample in the shape the recorder stores: the fields the histories read, plus the static
    /// objects every real sample carries, so a small change is written as a delta.
    fn live(bed_target: f64, bed_c: f64, z: f64, cs: i64, matrix: &Value, profile: &str) -> Value {
        json!({
            "heater_bed": {"temperature": bed_c, "target": bed_target, "power": 0.0},
            "gcode_move": {"homing_origin": [0.0, 0.0, z, 0.0], "speed_factor": 1.0,
                "extrude_factor": 1.0, "absolute_coordinates": true},
            "tmc2209 stepper_x": {"run_current": 0.8, "hold_current": 0.8,
                "drv_status": {"cs_actual": cs, "stealth": true}},
            "bed_mesh": {"profile_name": profile, "mesh_min": [40.0, 40.0],
                "mesh_max": [310.0, 310.0], "probed_matrix": matrix},
            "toolhead": {"homed_axes": "xyz", "axis_minimum": [0.0, 0.0, -5.0, 0.0],
                "axis_maximum": [350.0, 350.0, 310.0, 0.0], "max_velocity": 300.0,
                "max_accel": 3000.0, "square_corner_velocity": 5.0, "minimum_cruise_ratio": 0.5,
                "extruder": "extruder", "stalls": 0},
            "idle_timeout": {"state": "Idle", "printing_time": 0.0},
            "webhooks": {"state": "ready", "state_message": "Printer is ready"}
        })
    }

    /// WO-15: changes the recorder stored only in deltas, made and undone between two keyframes,
    /// are returned by every history that reads them. The first cut read keyframes only and saw
    /// none of it: no heat-up, no mesh, one cs_actual value, and the old offset beside the change.
    #[test]
    fn a_change_stored_only_in_a_delta_is_returned() {
        let mut db = store();
        let (empty, grid) = (json!([[]]), json!([[0.0, 0.1], [0.2, 0.3]]));
        put(&mut db, 100.0, live(0.0, 22.0, -0.035, 12, &empty, ""));
        put(&mut db, 110.0, live(60.0, 22.0, -0.035, 12, &empty, ""));
        put(&mut db, 120.0, live(60.0, 40.0, -0.035, 12, &empty, ""));
        put(&mut db, 130.0, live(60.0, 59.6, -0.085, 12, &empty, ""));
        put(
            &mut db,
            140.0,
            live(60.0, 61.0, -0.085, 20, &grid, "default"),
        );
        put(&mut db, 150.0, live(0.0, 60.5, -0.085, 12, &empty, ""));
        put(&mut db, 170.0, live(0.0, 58.0, -0.085, 12, &empty, ""));
        db.conn
            .execute(
                "INSERT INTO events(ts,kind,message) VALUES(135,'z_offset_change','SET_GCODE_OFFSET Z=-0.085')",
                [],
            )
            .unwrap();
        let kinds: String = db
            .conn
            .prepare("SELECT kind FROM samples ORDER BY ts")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<String>>()
            .unwrap();
        assert_eq!(kinds, "KDDDDDK", "every change must live in a delta only");

        let thermal = body(&get(&db, "/api/history/thermal"));
        let sessions = thermal["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1, "{thermal}");
        let s = &sessions[0];
        assert_eq!(s["heater"], "heater_bed");
        assert_eq!(
            (s["ts"].as_f64(), s["start_c"].as_f64()),
            (Some(110.0), Some(22.0))
        );
        assert_eq!(s["reached_ts"], 130.0);
        assert_eq!(s["seconds_to_target"], 20.0, "not a 60 s keyframe step");
        assert_eq!(
            (s["peak_c"].as_f64(), s["peak_ts"].as_f64()),
            (Some(61.0), Some(140.0))
        );

        let mesh = body(&get(&db, "/api/history/mesh"));
        let meshes = mesh["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 1, "{mesh}");
        assert_eq!(
            (meshes[0]["ts"].as_f64(), meshes[0]["profile"].as_str()),
            (Some(140.0), Some("default"))
        );

        let drift = body(&get(&db, "/api/history/motor-drift"));
        let x = drift["motors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["motor"] == "stepper_x")
            .unwrap();
        assert_eq!(x["samples"], 3, "12, 20 and back to 12: {x}");
        assert_eq!(x["max_cs_actual"], 20.0);

        let z = body(&get(&db, "/api/history/z-offset"));
        let change = &z["changes"][0];
        assert!(
            (change["applied_z_mm"].as_f64().unwrap() + 0.085).abs() < 1e-9,
            "{z}"
        );
        assert_eq!(change["applied_from_ts"], 130.0);
    }
}
