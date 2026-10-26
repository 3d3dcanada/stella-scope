use super::Response;
use crate::{
    config::Config,
    model::{CollectorStatus, MOTORS},
    store::{db_error, Store},
    Result,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Write, sync::Mutex, time::Instant};

static ANALYSIS_SLOT: Mutex<()> = Mutex::new(());
const FLAGS: [&str; 12] = [
    "otpw", "ot", "s2vsa", "s2vsb", "s2ga", "s2gb", "ola", "olb", "t120", "t143", "t150", "t157",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_samples: usize,
    pub max_scan_bytes: usize,
    pub max_window_s: u64,
    pub max_query_ms: u64,
    pub gap_multiple: f64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_samples: 12000,
            max_scan_bytes: 64 * 1024 * 1024,
            max_window_s: 86400,
            max_query_ms: 5000,
            gap_multiple: 3.0,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if !(2..=100000).contains(&self.max_samples)
            || !(65536..=268435456).contains(&self.max_scan_bytes)
            || !(1..=604800).contains(&self.max_window_s)
            || !(50..=30000).contains(&self.max_query_ms)
            || !self.gap_multiple.is_finite()
            || !(1.5..=100.0).contains(&self.gap_multiple)
        {
            return Err("investigation limits are invalid; use --print-config defaults or the documented bounds".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Reading {
    pub current_a: Option<f64>,
    pub drive_level: Option<u8>,
    pub standstill: Option<bool>,
    pub energised: Option<bool>,
    pub flag_bits: Option<u16>,
    pub segment: u64,
}
fn bit(v: Option<&Value>) -> Option<bool> {
    match v {
        None => Some(false),
        Some(Value::Bool(v)) => Some(*v),
        Some(v) => v.as_u64().filter(|n| *n <= 1).map(|n| n != 0),
    }
}
fn finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn reading(data: &Value, name: &str) -> Reading {
    let d = &data[format!("tmc2209 {name}")];
    let status = d["drv_status"].as_object();
    let drive_level = status.and_then(|s| match s.get("cs_actual") {
        None => Some(0),
        Some(v) => v.as_u64().filter(|n| *n <= 31).map(|n| n as u8),
    });
    let flag_bits = status.and_then(|s| {
        let mut bits = 0;
        for (i, flag) in FLAGS.iter().enumerate() {
            if bit(s.get(*flag))? {
                bits |= 1 << i;
            }
        }
        Some(bits)
    });
    Reading {
        current_a: finite(&d["run_current"]).filter(|n| *n >= 0.0),
        drive_level,
        standstill: status.and_then(|s| bit(s.get("stst"))),
        energised: data["stepper_enable"]["steppers"]
            .as_object()
            .and_then(|s| s.get(name))
            .and_then(|v| bit(Some(v))),
        flag_bits,
        segment: 0,
    }
}
fn flag_names(bits: u16) -> Vec<&'static str> {
    FLAGS
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, s)| *s)
        .collect()
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Point {
    pub ts: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub velocity: Option<f64>,
    pub nozzle: Option<f64>,
    pub bed: Option<f64>,
    pub segment: u64,
    pub motors: [Reading; 7],
}
impl Point {
    fn from(ts: f64, d: &Value) -> Self {
        Self {
            ts,
            x: finite(&d["motion_report"]["live_position"][0]),
            y: finite(&d["motion_report"]["live_position"][1]),
            z: finite(&d["motion_report"]["live_position"][2]),
            velocity: finite(&d["motion_report"]["live_velocity"]),
            nozzle: finite(&d["extruder"]["temperature"]),
            bed: finite(&d["heater_bed"]["temperature"]),
            segment: 0,
            motors: std::array::from_fn(|i| reading(d, MOTORS[i].0)),
        }
    }
    fn signals(&self) -> [Option<f64>; 16] {
        std::array::from_fn(|i| match i {
            0..=6 => self.motors[i].current_a,
            7..=13 => self.motors[i - 7].drive_level.map(f64::from),
            14 => self.z,
            _ => self.velocity,
        })
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Metric {
    pub known: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
}
impl Metric {
    fn add(&mut self, value: Option<f64>) {
        if let Some(v) = value {
            self.known += 1;
            self.min = Some(self.min.map_or(v, |n| n.min(v)));
            self.max = Some(self.max.map_or(v, |n| n.max(v)));
            self.mean = Some(self.mean.map_or(v, |m| {
                m * (1.0 - 1.0 / self.known as f64) + v / self.known as f64
            }))
            .filter(|n| n.is_finite());
        }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct MotorSummary {
    pub name: String,
    pub label: String,
    pub current_a: Metric,
    pub drive_level: Metric,
    pub status_known: usize,
    pub status_unknown: usize,
    pub flagged_samples: usize,
    pub observed_flags: Vec<&'static str>,
}

// A representative-only chart discarded narrow driver peaks. For budgets >=34,
// each time bucket retains both edges and all seven motors' current/scale extrema,
// plus height/speed extrema. Statistics and incidents use every scanned sample.
#[derive(Clone)]
struct Bucket {
    first: Option<Point>,
    last: Option<Point>,
    lo: [Option<Point>; 16],
    hi: [Option<Point>; 16],
}
impl Bucket {
    fn new() -> Self {
        Self {
            first: None,
            last: None,
            lo: [None; 16],
            hi: [None; 16],
        }
    }
    fn add(&mut self, p: Point) {
        self.first.get_or_insert(p);
        self.last = Some(p);
        for (i, v) in p.signals().iter().enumerate() {
            if let Some(v) = v {
                if self.lo[i]
                    .and_then(|p| p.signals()[i])
                    .is_none_or(|old| *v < old)
                {
                    self.lo[i] = Some(p);
                }
                if self.hi[i]
                    .and_then(|p| p.signals()[i])
                    .is_none_or(|old| *v > old)
                {
                    self.hi[i] = Some(p);
                }
            }
        }
    }
    fn output(&self, rich: bool) -> Vec<Point> {
        if !rich {
            return self.last.into_iter().collect();
        }
        self.first
            .into_iter()
            .chain(self.last)
            .chain(self.lo.into_iter().flatten())
            .chain(self.hi.into_iter().flatten())
            .collect()
    }
}
struct Envelope {
    from: f64,
    to: f64,
    rich: bool,
    buckets: Vec<Bucket>,
    first: Option<Point>,
    last: Option<Point>,
    budget: usize,
}
impl Envelope {
    fn new(from: f64, to: f64, budget: usize) -> Self {
        let rich = budget >= 34;
        let n = if rich {
            budget / 34
        } else {
            budget.saturating_sub(2).max(1)
        };
        Self {
            from,
            to,
            rich,
            buckets: vec![Bucket::new(); n],
            first: None,
            last: None,
            budget,
        }
    }
    fn add(&mut self, p: Point) {
        self.first.get_or_insert(p);
        self.last = Some(p);
        let i = if self.to == self.from {
            0
        } else {
            (((p.ts - self.from) / (self.to - self.from)) * self.buckets.len() as f64) as usize
        };
        let i = i.min(self.buckets.len() - 1);
        self.buckets[i].add(p);
    }
    fn finish(self) -> Vec<Point> {
        let mut out = if self.rich {
            Vec::new()
        } else {
            self.first.into_iter().chain(self.last).collect()
        };
        if self.budget > 2 || self.rich {
            for b in &self.buckets {
                out.extend(b.output(self.rich));
            }
        }
        out.sort_by(|a, b| a.ts.total_cmp(&b.ts));
        out.dedup_by(|a, b| a.ts == b.ts);
        out
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Incident {
    pub ts: f64,
    pub category: String,
    pub motor: Option<String>,
    pub title: String,
    pub flags: Vec<&'static str>,
    pub source: String,
    pub message: Option<String>,
}
fn incident(
    ts: f64,
    category: &str,
    motor: Option<&str>,
    title: &str,
    flags: Vec<&'static str>,
    source: &str,
) -> Incident {
    Incident {
        ts,
        category: category.into(),
        motor: motor.map(str::to_owned),
        title: title.into(),
        flags,
        source: source.into(),
        message: None,
    }
}
fn event_kind(kind: &str, message: &str) -> Option<(&'static str, &'static str)> {
    if kind == "command" {
        return None;
    }
    let lower = message.to_lowercase();
    if kind == "position_warning" || message.contains("position skew after probe:") {
        Some(("position", "Position discrepancy reported"))
    } else if kind == "levelling_failed" || message.contains("Retries aborting:") {
        Some(("levelling", "Gantry levelling aborted"))
    } else if kind == "printer_offline" || kind == "connection_lost" {
        Some(("connection", "Printer stopped answering"))
    } else if kind == "connection" || kind == "connection_restored" {
        Some(("connection", "Collector connection or limitations changed"))
    } else if kind == "driver_warning" {
        Some(("driver", "Driver warning recorded in console"))
    } else if kind == "console_gap" || kind == "degraded" || kind == "log_notice" {
        Some(("capture", "Recording limitation reported"))
    } else if kind == "error" || message.starts_with("!!") || lower.contains("shutdown") {
        Some(("error", "Printer error message"))
    } else {
        None
    }
}
struct CappedWriter {
    bytes: Vec<u8>,
    max: usize,
}
impl Write for CappedWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other(
                "export exceeds response cap; reduce the time window or chart points",
            ));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn response(code: u16, v: &impl Serialize, cap: usize) -> Response {
    let mut w = CappedWriter {
        bytes: Vec::new(),
        max: cap,
    };
    match serde_json::to_writer(&mut w, v) {
        Ok(()) => Response {
            code,
            mime: "application/json; charset=utf-8",
            body: w.bytes,
        },
        Err(_) => Response::json(
            400,
            json!({"error":"investigation output exceeds max_response_bytes; reduce points, the window, or exclude console messages"}),
        ),
    }
}

#[derive(Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub version: &'static str,
    pub recording_origin: String,
    pub from: f64,
    pub to: f64,
    pub analyzed_to: f64,
    pub first_sample: Option<f64>,
    pub last_sample: Option<f64>,
    pub samples_scanned: usize,
    pub bytes_scanned: usize,
    pub complete: bool,
    pub next_from: Option<f64>,
    pub limitations: Vec<String>,
    pub max_gap_seconds: f64,
    pub gap_count: usize,
    pub gap_threshold_seconds: f64,
    pub geometry_recorded: bool,
    pub motors: Vec<MotorSummary>,
    pub points: Vec<Point>,
    pub incidents: Vec<Incident>,
    pub events: Vec<Value>,
    pub rounds: Vec<Value>,
    pub chart_method: &'static str,
    pub messages_included: bool,
    pub privacy: &'static str,
}

pub fn build(
    db: &Store,
    cfg: &Config,
    from: f64,
    to: f64,
    points: usize,
    messages: bool,
) -> Result<Report> {
    let started = Instant::now();
    let limits = &cfg.investigation;
    // A single deferred read transaction prevents an export from combining different
    // retention/collection instants. Dropping it never edits the recording.
    let _snapshot = db.conn.unchecked_transaction().map_err(db_error)?;
    let geometry = db.context_at(from, "scope_geometry")?;
    let poll = geometry
        .as_ref()
        .and_then(|v| v["poll_interval_ms"].as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(cfg.poll_interval_ms as f64)
        / 1000.0;
    let threshold = poll * limits.gap_multiple;
    let mut report = Report {
        schema: "stella-scope.investigation.v1",
        version: env!("CARGO_PKG_VERSION"),
        recording_origin: "unknown".into(),
        from,
        to,
        analyzed_to: to,
        first_sample: None,
        last_sample: None,
        samples_scanned: 0,
        bytes_scanned: 0,
        complete: true,
        next_from: None,
        limitations: Vec::new(),
        max_gap_seconds: 0.0,
        gap_count: 0,
        gap_threshold_seconds: threshold,
        geometry_recorded: geometry.is_some(),
        motors: MOTORS
            .iter()
            .map(|(n, l)| MotorSummary {
                name: (*n).into(),
                label: (*l).into(),
                ..MotorSummary::default()
            })
            .collect(),
        points: Vec::new(),
        incidents: Vec::new(),
        events: Vec::new(),
        rounds: Vec::new(),
        chart_method: if points >= 34 {
            "time-bucket edges and motor/height/speed extrema; incident times use all scanned samples"
        } else {
            "representative points; increase points to at least 34 to preserve per-bucket extrema"
        },
        messages_included: messages,
        privacy: if messages {
            "Console text is included verbatim and may contain private filenames, paths or secrets. Review before sharing. Raw status and printer configuration are not exported."
        } else {
            "Telemetry-only export. Console text, job filenames, raw status, printer configuration and server paths are omitted."
        },
    };
    let mut envelope = Envelope::new(from, to, points);
    let mut prior: [Option<u16>; 7] = [None; 7];
    let mut segments = [0_u64; 7];
    let mut segment = 0_u64;
    let mut all_flags = [0_u16; 7];
    let mut previous = None;
    let mut lost_incidents = 0;
    let mut push_incident = |report: &mut Report, i: Incident| {
        if report.incidents.len() < cfg.events_limit {
            report.incidents.push(i);
        } else {
            lost_incidents += 1;
        }
    };
    let sample_times = db.sample_times(from, to, limits.max_samples + 1)?;
    for ts in sample_times {
        if !ts.is_finite() || ts < 0.0 {
            return Err(
                "recording contains an invalid sample timestamp; preserve the file for repair"
                    .into(),
            );
        }
        let why = if report.samples_scanned >= limits.max_samples {
            Some("Sample scan cap reached")
        } else if started.elapsed().as_millis() >= limits.max_query_ms as u128 {
            Some("Analysis time budget reached")
        } else {
            None
        };
        if let Some(why) = why {
            report.complete = false;
            report.next_from = Some(ts);
            report.analyzed_to = report.last_sample.unwrap_or(from);
            report.limitations.push(format!("{why}; shorten the selected window or continue from next_from. Unscanned data is not claimed to be clear."));
            break;
        }
        let (_,data,size)=db.load_sample(ts,cfg.max_response_bytes)?.ok_or_else(||format!("sample at {ts} disappeared during the read transaction; preserve this recording for repair"))?;
        if size > limits.max_scan_bytes.saturating_sub(report.bytes_scanned) {
            report.complete = false;
            report.next_from = Some(ts);
            report.analyzed_to = report.last_sample.unwrap_or(from);
            report.limitations.push("Byte scan cap reached; shorten the selected window or continue from next_from. Unscanned data is not claimed to be clear.".into());
            break;
        }
        let mut p = Point::from(ts, &data);
        if let Some(last) = previous {
            let gap: f64 = ts - last;
            report.max_gap_seconds = report.max_gap_seconds.max(gap);
            if gap > threshold {
                segment += 1;
                report.gap_count += 1;
                prior = [None; 7];
                push_incident(
                    &mut report,
                    incident(
                        ts,
                        "capture",
                        None,
                        &format!("Sampling gap: {gap:.3} seconds"),
                        vec![],
                        "recorded sample timestamps",
                    ),
                );
            }
        }
        p.segment = segment;
        for i in 0..7 {
            let r = &mut p.motors[i];
            let summary = &mut report.motors[i];
            summary.current_a.add(r.current_a);
            summary.drive_level.add(r.drive_level.map(f64::from));
            if r.current_a.is_none() || r.drive_level.is_none() || r.flag_bits.is_none() {
                segments[i] += 1;
            }
            r.segment = segments[i];
            match r.flag_bits {
                None => {
                    summary.status_unknown += 1;
                    prior[i] = None;
                }
                Some(bits) => {
                    summary.status_known += 1;
                    all_flags[i] |= bits;
                    if bits != 0 {
                        summary.flagged_samples += 1;
                    }
                    if bits != 0 && prior[i] != Some(bits) {
                        let title = if prior[i].is_none() {
                            "Driver flags present at first available reading"
                        } else {
                            "Driver warning flags changed"
                        };
                        push_incident(
                            &mut report,
                            incident(
                                ts,
                                "driver",
                                Some(MOTORS[i].0),
                                title,
                                flag_names(bits),
                                "sampled driver status; observation time, not exact onset",
                            ),
                        );
                    } else if bits == 0 && prior[i].is_some_and(|b| b != 0) {
                        push_incident(
                            &mut report,
                            incident(
                                ts,
                                "driver",
                                Some(MOTORS[i].0),
                                "Previously reported driver flags no longer present",
                                vec![],
                                "sampled driver status",
                            ),
                        );
                    }
                    prior[i] = Some(bits);
                }
            }
        }
        report.samples_scanned += 1;
        report.bytes_scanned += size;
        report.first_sample.get_or_insert(ts);
        report.last_sample = Some(ts);
        previous = Some(ts);
        envelope.add(p);
    }
    if lost_incidents > 0 {
        report.complete = false;
        report.limitations.push(format!("Incident display cap reached; {lost_incidents} additional sample transitions are not listed. Reduce the window."));
    }
    for (i, bits) in all_flags.iter().enumerate() {
        report.motors[i].observed_flags = flag_names(*bits);
    }
    report.points = envelope.finish();
    if report.samples_scanned == 0 {
        report.limitations.push(
            "No samples in the scanned part of this window. This is not a healthy-printer result."
                .into(),
        );
    }
    if !report.geometry_recorded {
        report.limitations.push("No geometry metadata at the window start; sampling-gap threshold uses the current configuration.".into());
    }
    let geometry_changes:i64=db.conn.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM events WHERE kind='scope_geometry' AND ts>?1 AND ts<=?2 LIMIT 1)",params![from,report.analyzed_to],|r|r.get(0)).map_err(db_error)?;
    if geometry_changes > 0 {
        report.limitations.push("Recorder geometry/settings changed within this window. Gap threshold is based on the window start; split at the restart for a uniform interval.".into());
    }
    if let Some(v) = db.context_at(report.analyzed_to, "scope_origin")? {
        if let Some(s) = v.as_str().filter(|s| ["demo", "live"].contains(s)) {
            report.recording_origin = s.into();
        }
    }
    let mut q=db.conn.prepare("SELECT ts,kind,length(CAST(message AS BLOB)),message FROM events WHERE ts>=?1 AND ts<=?2 AND kind NOT IN ('scope_health','scope_geometry','scope_origin') ORDER BY ts,rowid LIMIT ?3").map_err(db_error)?;
    let mut rows = q
        .query(params![
            from,
            report.analyzed_to,
            (cfg.events_limit + 1) as i64
        ])
        .map_err(db_error)?;
    let mut event_bytes = 0_usize;
    while let Some(row) = rows.next().map_err(db_error)? {
        if report.events.len() >= cfg.events_limit {
            report.complete = false;
            report.limitations.push(
                "Console/event cap reached; reduce the window to capture all messages.".into(),
            );
            break;
        }
        let ts: f64 = row.get(0).map_err(db_error)?;
        let kind: String = row.get(1).map_err(db_error)?;
        let size: usize = row.get(2).map_err(db_error)?;
        if size > cfg.max_response_bytes / 4
            || event_bytes.saturating_add(size) > cfg.max_response_bytes / 4
            || started.elapsed().as_millis() >= limits.max_query_ms as u128
        {
            report.complete = false;
            report.limitations.push("Console byte/time budget reached; shorten the window. Event text is not silently truncated.".into());
            break;
        }
        event_bytes += size;
        let message: String = row.get(3).map_err(db_error)?;
        let category = event_kind(&kind, &message);
        let mut v = json!({"ts":ts,"category":category.map(|(c,_)|c).unwrap_or("console")});
        if messages {
            v["kind"] = json!(kind);
            v["message"] = json!(message);
        }
        report.events.push(v);
        if let Some((cat, title)) = category {
            let mut i = incident(
                ts,
                cat,
                None,
                title,
                vec![],
                "recorded console/log event; log timestamps may be capture time",
            );
            if messages {
                i.message = Some(message);
            }
            report.incidents.push(i);
        }
    }
    drop(rows);
    drop(q);
    report
        .incidents
        .sort_by(|a, b| a.ts.total_cmp(&b.ts).then(a.category.cmp(&b.category)));
    report.incidents.dedup_by(|a, b| {
        a.ts == b.ts && a.category == b.category && a.motor == b.motor && a.title == b.title
    });
    if report.incidents.len() > cfg.events_limit {
        report.incidents.truncate(cfg.events_limit);
        report.complete = false;
        report.limitations.push(
            "Combined incident list capped; select a smaller window for complete navigation."
                .into(),
        );
    }
    let mut rounds = db.rounds(from, report.analyzed_to, cfg.qgl_limit + 1)?;
    if rounds.len() > cfg.qgl_limit {
        rounds.truncate(cfg.qgl_limit);
        report.complete = false;
        report
            .limitations
            .push("Levelling-round cap reached; choose a smaller time window.".into());
    }
    for row in rounds {
        if started.elapsed().as_millis() >= limits.max_query_ms as u128 {
            report.complete = false;
            report
                .limitations
                .push("Levelling analysis time budget reached; shorten the window.".into());
            break;
        }
        let g = db.context_at(row.ts, "scope_geometry")?;
        let tol = g
            .as_ref()
            .and_then(|v| v["qgl_tolerance_mm"].as_f64())
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(cfg.qgl_tolerance_mm);
        let mut v = serde_json::to_value(&row).map_err(|e| e.to_string())?;
        v["tolerance_mm"] = json!(tol);
        v["within_tolerance"] = json!(row.range_mm <= tol);
        v["tolerance_source"] = json!(if g.is_some() {
            "recorded recorder setting"
        } else {
            "current recorder setting; not the parsed macro tolerance"
        });
        report.rounds.push(v);
    }
    report.rounds.sort_by(|a, b| {
        a["ts"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&b["ts"].as_f64().unwrap_or(0.0))
    });
    report.limitations.push("Driver current scale is not mechanical load, torque, or proof of lost steps. No physical position feedback is recorded.".into());
    Ok(report)
}
fn csv(report: &Report, cap: usize) -> Response {
    let mut w = CappedWriter {
        bytes: Vec::new(),
        max: cap,
    };
    let result = (|| -> std::io::Result<()> {
        write!(w, "timestamp,x_mm,y_mm,z_mm,velocity_mm_s,nozzle_c,bed_c")?;
        for (name, _) in MOTORS {
            write!(w,",{name}_current_a,{name}_drive_level,{name}_standstill,{name}_energised,{name}_flags")?;
        }
        writeln!(w)?;
        for p in &report.points {
            write!(w, "{}", p.ts)?;
            for n in [p.x, p.y, p.z, p.velocity, p.nozzle, p.bed] {
                write!(w, ",{}", n.map(|v| v.to_string()).unwrap_or_default())?;
            }
            for r in &p.motors {
                write!(
                    w,
                    ",{},{},{},{},{}",
                    r.current_a.map(|v| v.to_string()).unwrap_or_default(),
                    r.drive_level.map(|v| v.to_string()).unwrap_or_default(),
                    r.standstill.map(|v| v.to_string()).unwrap_or_default(),
                    r.energised.map(|v| v.to_string()).unwrap_or_default(),
                    r.flag_bits
                        .map(|v| flag_names(v).join("|"))
                        .unwrap_or_default()
                )?;
            }
            writeln!(w)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Response {
            code: 200,
            mime: "text/csv; charset=utf-8",
            body: w.bytes,
        },
        Err(_) => Response::json(
            400,
            json!({"error":"CSV exceeds max_response_bytes; reduce chart points"}),
        ),
    }
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
            json!({"error":"read-only investigation; only GET is allowed"}),
        );
    }
    let parsed = (|| -> Result<(&str, BTreeMap<String, String>, f64, f64, usize, bool)> {
        let (path, q) = super::query(target)?;
        for k in q.keys() {
            if !["from", "to", "points", "messages", "format"].contains(&k.as_str()) {
                return Err(format!("unknown investigation parameter {k}; use from, to, points, messages, or format"));
            }
        }
        let from = super::number(&q, "from", None)?;
        let to = super::number(&q, "to", None)?;
        if from > to || to - from > cfg.investigation.max_window_s as f64 {
            return Err(format!(
                "select an ordered time window no longer than {} seconds",
                cfg.investigation.max_window_s
            ));
        }
        let points = q
            .get("points")
            .map_or(Ok(400.min(cfg.series_max_points)), |s| {
                s.parse::<usize>()
                    .map_err(|_| "points must be an integer".to_owned())
            })?;
        if !(2..=cfg.series_max_points).contains(&points) {
            return Err(format!(
                "points must be between 2 and {}",
                cfg.series_max_points
            ));
        }
        let messages = match q.get("messages").map(String::as_str) {
            None | Some("0") => false,
            Some("1") => true,
            _ => {
                return Err(
                    "messages must be 0 (private text omitted) or 1 (include console text)".into(),
                )
            }
        };
        if !["json", "csv"].contains(&q.get("format").map(String::as_str).unwrap_or("json")) {
            return Err("format must be json or csv".into());
        }
        Ok((path, q, from, to, points, messages))
    })();
    let (path, q, from, to, points, messages) = match parsed {
        Ok(v) => v,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let _permit = match ANALYSIS_SLOT.try_lock() {
        Ok(p) => p,
        Err(_) => {
            return Response::json(
                503,
                json!({"error":"another investigation is running; retry after it finishes"}),
            )
        }
    };
    match build(db, cfg, from, to, points, messages) {
        Err(e) => Response::json(500, json!({"error":e})),
        Ok(r) => {
            if path == "/api/export" && (!r.complete || r.samples_scanned == 0) {
                return Response::json(
                    400,
                    json!({"error":"export refused: the selected window is empty or capped; shorten it until analysis is complete","limitations":r.limitations,"next_from":r.next_from}),
                );
            }
            if path == "/api/export" && q.get("format").is_some_and(|v| v == "csv") {
                csv(&r, cfg.max_response_bytes)
            } else {
                response(200, &r, cfg.max_response_bytes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qgl::Parser;
    fn store() -> Store {
        Store::open(":memory:", false).unwrap()
    }
    fn sample(db: &mut Store, ts: f64, d: Value) {
        db.write(Some((ts, &d)), &[], &mut Parser::default())
            .unwrap();
    }
    fn data(cs: u8, flags: Value) -> Value {
        json!({"motion_report":{"live_position":[1,2,3],"live_velocity":4},"tmc2209 stepper_z1":{"run_current":0.8,"drv_status":{"cs_actual":cs,"otpw":flags}},"configfile":{"settings":{"private_key":"DO_NOT_EXPORT"}},"print_stats":{"filename":"PRIVATE_FILENAME"}})
    }
    #[test]
    fn old_configs_receive_investigation_defaults() {
        let c: Config = serde_json::from_str(r#"{"moonraker_url":"http://192.168.1.131:7125"}"#).unwrap();
        assert_eq!(c.investigation.max_samples, 12000);
        c.validate().unwrap();
    }
    #[test]
    fn invalid_limits_rejected() {
        let mut l = Limits::default();
        l.gap_multiple = f64::NAN;
        assert!(l.validate().is_err());
    }
    #[test]
    fn sparse_status_is_zero_but_absent_is_unknown() {
        let d = json!({"tmc2209 stepper_z1":{"drv_status":{}}});
        assert_eq!(reading(&d, "stepper_z1").flag_bits, Some(0));
        assert_eq!(reading(&d, "stepper_z").flag_bits, None);
    }
    #[test]
    fn malformed_status_fields_are_unknown() {
        let d = json!({"tmc2209 stepper_z1":{"drv_status":{"cs_actual":100,"ot": "yes"}}});
        let r = reading(&d, "stepper_z1");
        assert_eq!(r.drive_level, None);
        assert_eq!(r.flag_bits, None);
    }
    #[test]
    fn bool_flags_are_supported() {
        assert_eq!(
            reading(&data(10, json!(true)), "stepper_z1").flag_bits,
            Some(1)
        );
    }
    #[test]
    fn absent_enable_is_unknown() {
        assert_eq!(reading(&data(10, json!(0)), "stepper_z1").energised, None);
    }
    #[test]
    fn negative_current_is_unknown() {
        let d = json!({"tmc2209 stepper_z1":{"run_current":-1}});
        assert_eq!(reading(&d, "stepper_z1").current_a, None);
    }
    #[test]
    fn transitions_are_not_repeated_and_unknown_does_not_clear() {
        let mut db = store();
        for (i, d) in [
            data(10, json!(1)),
            data(10, json!(1)),
            json!({}),
            data(10, json!(0)),
        ]
        .into_iter()
        .enumerate()
        {
            sample(&mut db, 100.0 + i as f64 / 4.0, d);
        }
        let r = build(&db, &Config::default(), 100.0, 101.0, 100, false).unwrap();
        assert_eq!(
            r.incidents
                .iter()
                .filter(|i| i.category == "driver")
                .count(),
            1
        );
        assert_eq!(r.motors[3].status_unknown, 1);
    }
    #[test]
    fn clear_transition_requires_previous_known_flags() {
        let mut db = store();
        sample(&mut db, 100.0, data(10, json!(1)));
        sample(&mut db, 100.25, data(10, json!(0)));
        let r = build(&db, &Config::default(), 100.0, 101.0, 100, false).unwrap();
        assert_eq!(r.incidents.len(), 2);
    }
    #[test]
    fn tall_peaks_and_small_dips_survive_envelope() {
        let mut e = Envelope::new(0.0, 100.0, 68);
        for i in 0..=100 {
            let mut p = Point {
                ts: i as f64,
                ..Point::default()
            };
            p.motors[3].drive_level = Some(if i == 13 { 31 } else { 8 });
            e.add(p);
        }
        let p = e.finish();
        assert!(p.len() <= 68);
        assert_eq!(p.first().unwrap().ts, 0.0);
        assert_eq!(p.last().unwrap().ts, 100.0);
        assert!(p.iter().any(|p| p.motors[3].drive_level == Some(31)));
    }
    #[test]
    fn envelope_respects_all_small_budgets() {
        for n in 2..100 {
            let mut e = Envelope::new(0.0, 100.0, n);
            for i in 0..100 {
                e.add(Point {
                    ts: i as f64,
                    ..Point::default()
                });
            }
            let v = e.finish();
            assert!(v.len() <= n, "budget {n} got {}", v.len());
            assert_eq!(v.first().unwrap().ts, 0.0);
            assert_eq!(v.last().unwrap().ts, 99.0);
        }
    }
    #[test]
    fn snapshots_exclude_future_samples_and_events() {
        let mut db = store();
        sample(&mut db, 100.0, data(3, json!(0)));
        sample(&mut db, 101.0, data(31, json!(1)));
        db.conn
            .execute("INSERT INTO events VALUES(101,'error','future')", [])
            .unwrap();
        let r = build(&db, &Config::default(), 99.0, 100.0, 100, true).unwrap();
        assert_eq!(r.samples_scanned, 1);
        assert!(r.events.is_empty());
        assert!(r.incidents.is_empty());
    }
    #[test]
    fn default_export_omits_private_text_and_config() {
        let mut db = store();
        sample(&mut db, 100.0, data(8, json!(1)));
        db.conn
            .execute("INSERT INTO events VALUES(100,'error','SECRET_TOKEN')", [])
            .unwrap();
        let r = build(&db, &Config::default(), 99.0, 101.0, 100, false).unwrap();
        let s = serde_json::to_string(&r).unwrap();
        assert!(!s.contains("SECRET_TOKEN"));
        assert!(!s.contains("DO_NOT_EXPORT"));
        assert!(!s.contains("PRIVATE_FILENAME"));
    }
    #[test]
    fn private_messages_require_opt_in() {
        let db = store();
        db.conn
            .execute(
                "INSERT INTO events VALUES(100,'error','PRIVATE_MESSAGE')",
                [],
            )
            .unwrap();
        let r = build(&db, &Config::default(), 99.0, 101.0, 100, true).unwrap();
        assert!(serde_json::to_string(&r)
            .unwrap()
            .contains("PRIVATE_MESSAGE"));
    }
    #[test]
    fn sample_cap_reports_continuation() {
        let mut db = store();
        let mut c = Config::default();
        c.investigation.max_samples = 2;
        for i in 0..4 {
            sample(&mut db, 100.0 + i as f64 / 4.0, data(8, json!(0)));
        }
        let r = build(&db, &c, 100.0, 101.0, 100, false).unwrap();
        assert!(!r.complete);
        assert_eq!(r.next_from, Some(100.5));
        assert_eq!(r.analyzed_to, 100.25);
    }
    #[test]
    fn byte_cap_is_visible_without_reading_whole_status() {
        let mut db = store();
        sample(&mut db, 100.0, data(8, json!(0)));
        let mut c = Config::default();
        c.investigation.max_scan_bytes = 1;
        let r = build(&db, &c, 99.0, 101.0, 100, false).unwrap();
        assert!(!r.complete);
        assert_eq!(r.samples_scanned, 0);
    }
    #[test]
    fn gaps_break_chart_segments() {
        let mut db = store();
        sample(&mut db, 100.0, data(8, json!(1)));
        sample(&mut db, 105.0, data(8, json!(0)));
        let r = build(&db, &Config::default(), 100.0, 106.0, 100, false).unwrap();
        assert_eq!(r.gap_count, 1);
        assert_ne!(r.points[0].segment, r.points[1].segment);
        assert!(!r.incidents.iter().any(|i| i.title.contains("no longer")));
    }
    #[test]
    fn empty_window_does_not_claim_health() {
        let r = build(&store(), &Config::default(), 1.0, 2.0, 100, false).unwrap();
        assert_eq!(r.samples_scanned, 0);
        assert!(r.limitations.iter().any(|s| s.contains("not a healthy")));
    }
    #[test]
    fn corrupt_recording_errors_are_visible() {
        let db = store();
        db.conn
            .execute("INSERT INTO samples(ts,data) VALUES(100,'{broken')", [])
            .unwrap();
        assert!(build(&db, &Config::default(), 99.0, 101.0, 100, false).is_err());
    }
    #[test]
    fn event_caps_are_not_silent() {
        let db = store();
        for i in 0..4 {
            db.conn
                .execute("INSERT INTO events VALUES(?1,'error','x')", [100 + i])
                .unwrap();
        }
        let mut c = Config::default();
        c.events_limit = 2;
        let r = build(&db, &c, 99.0, 110.0, 100, false).unwrap();
        assert!(!r.complete);
        assert_eq!(r.events.len(), 2);
    }
    #[test]
    fn console_bytes_are_bounded_before_allocation() {
        let db = store();
        db.conn
            .execute(
                "INSERT INTO events VALUES(100,'error',?1)",
                ["x".repeat(20000)],
            )
            .unwrap();
        let mut c = Config::default();
        c.max_response_bytes = 65536;
        let r = build(&db, &c, 99.0, 101.0, 100, true).unwrap();
        assert!(!r.complete);
        assert!(r.events.is_empty());
    }
    #[test]
    fn csv_contains_only_fixed_headers_numbers_and_flag_names() {
        let mut db = store();
        sample(&mut db, 100.0, data(10, json!(1)));
        let r = build(&db, &Config::default(), 100.0, 101.0, 100, false).unwrap();
        let s = String::from_utf8(csv(&r, 100000).body).unwrap();
        assert!(s.contains("stepper_z1_current_a"));
        assert!(s.contains("otpw"));
        assert!(!s.contains("PRIVATE"));
        assert!(!s.contains("="));
        assert_eq!(s.lines().count(), 2);
    }
    #[test]
    fn serialization_has_a_hard_byte_limit() {
        let r = response(200, &json!({"v":"x".repeat(200)}), 100);
        assert_eq!(r.code, 400);
    }
    #[test]
    fn commands_do_not_create_faults() {
        assert!(event_kind("command", "SHUTDOWN").is_none());
    }
    #[test]
    fn qgl_future_rounds_are_excluded() {
        let db = store();
        db.conn
            .execute(
                "INSERT INTO qgl VALUES(100,1,0,0.01,NULL,NULL,NULL,NULL)",
                [],
            )
            .unwrap();
        db.conn
            .execute("INSERT INTO qgl VALUES(110,1,1,0.5,0,0,0,0)", [])
            .unwrap();
        let r = build(&db, &Config::default(), 99.0, 101.0, 100, false).unwrap();
        assert_eq!(r.rounds.len(), 1);
        assert!(r.rounds[0]["z0"].is_null());
        assert_eq!(r.rounds[0]["within_tolerance"], true);
    }
    #[test]
    fn read_only_analysis_does_not_mutate_tables() {
        let mut db = store();
        sample(&mut db, 100.0, data(8, json!(0)));
        let before = db.conn.total_changes();
        build(&db, &Config::default(), 99.0, 101.0, 100, false).unwrap();
        assert_eq!(db.conn.total_changes(), before);
    }
    #[test]
    fn router_validates_and_refuses_partial_exports() {
        let mut db = store();
        for i in 0..3 {
            sample(&mut db, 100.0 + i as f64 / 4.0, data(8, json!(0)));
        }
        let mut c = Config::default();
        c.investigation.max_samples = 2;
        let s = CollectorStatus::default();
        for target in [
            "/api/investigation?from=NaN&to=1",
            "/api/investigation?from=2&to=1",
            "/api/investigation?from=1&to=2&points=1",
            "/api/investigation?from=1&to=2&messages=yes",
            "/api/investigation?from=1&from=1&to=2",
            "/api/investigation?from=1&to=2&unknown=x",
        ] {
            assert_eq!(handle("GET", target, &db, &c, &s).code, 400);
        }
        assert_eq!(handle("POST", "/api/investigation", &db, &c, &s).code, 405);
        assert_eq!(
            handle("GET", "/api/export?from=99&to=102", &db, &c, &s).code,
            400
        );
        assert_eq!(
            handle("GET", "/api/investigation?from=99&to=102", &db, &c, &s).code,
            200
        );
    }
}
