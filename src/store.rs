use crate::{
    model,
    qgl::{Parser, Round},
    Result,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

const SAMPLE_CAP: usize = 64 * 1024 * 1024;
const KEYFRAME_SECONDS: f64 = 60.0;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub ts: f64,
    pub kind: String,
    pub message: String,
}
impl Event {
    pub fn new(ts: f64, kind: &str, message: impl Into<String>) -> Self {
        Self {
            ts,
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// One finished print, extracted at record time from print_stats. `result` is the Klipper state
/// that ended it (complete / error / cancelled) or "interrupted" when no completion was recorded
/// before the sample stream moved on. `evidence` carries the honest caveat when the recorder
/// joined a print already running: the start time is then the first sample seen, not the real
/// start, and it says so instead of inventing one.
#[derive(Clone, Debug, Serialize)]
pub struct PrintSession {
    pub start_ts: f64,
    pub end_ts: f64,
    pub filename: String,
    pub duration_s: Option<f64>,
    pub filament_used: Option<f64>,
    pub result: String,
    pub progress: Option<f64>,
    pub evidence: String,
}

/// Write-side session state, seeded on open from the newest recorded sample so a service restart
/// mid-print continues the same session instead of opening a second one. `quiet_ts` is the
/// honest-start witness: a session counts as fully recorded only when the tracker saw the machine
/// NOT printing within START_SEEN_S before its first sample. Anything else is a print the recorder
/// joined mid-run.
#[derive(Clone, Debug, Default)]
pub struct PrintTracker {
    open: Option<OpenPrint>,
    quiet_ts: Option<f64>,
    terminal: Option<(String, String)>,
}
#[derive(Clone, Debug)]
struct OpenPrint {
    filename: String,
    start_ts: f64,
    joined_mid_print: bool,
    /// The recorder restarted inside this print and read its start back from the recording.
    recovered: bool,
    /// The newest sample seen inside this print: where an interrupted print really stopped
    /// being recorded, instead of the first sample after a gap (which can be days later).
    last_ts: f64,
    duration_s: Option<f64>,
    filament_used: Option<f64>,
    progress: Option<f64>,
}

/// A print's start counts as recorded only when a quiet sample (idle, finished, or another file)
/// lies at most this far before its first printing sample. In the recording of 2026-09-16..22 every
/// print start the recorder saw followed a quiet sample by under 0.7 s; the one print it joined late
/// (2026-09-21 20:06:45, Klipper's total_duration already 162.9 s) followed the previous quiet
/// sample by 5518 s, and a stale quiet sample like that proves nothing.
const START_SEEN_S: f64 = 30.0;
/// How far back a restarted recorder replays the recording to find where its print began.
const RUN_SCAN_S: f64 = 48.0 * 3600.0;
fn seen_start(quiet_ts: Option<f64>, first_ts: f64) -> bool {
    quiet_ts.is_some_and(|q| first_ts - q <= START_SEEN_S)
}
/// A span of seconds in the words the HUD shows: "35 min 20 s", "2 h 5 min", "40 s".
fn span(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

impl PrintTracker {
    /// Seed from the newest sample at Store::open. `joined` flags a session the recorder never saw
    /// start; a terminal state is remembered so a restarted recorder does not record the same
    /// finished print once per restart. A printing seed starts at the newest sample, flagged
    /// joined; Store::open then reads the real start back from the recording.
    pub fn seed(ts: f64, data: &Value) -> Self {
        let state = data["print_stats"]["state"].as_str().unwrap_or("");
        let filename = data["print_stats"]["filename"].as_str().unwrap_or("");
        let mut out = Self { ..Self::default() };
        if filename.is_empty() {
            return out;
        }
        match state {
            "printing" | "paused" => {
                out.open = Some(OpenPrint {
                    filename: filename.to_owned(),
                    start_ts: ts,
                    joined_mid_print: true,
                    recovered: false,
                    last_ts: ts,
                    duration_s: nonneg(&data["print_stats"]["print_duration"]),
                    filament_used: nonneg(&data["print_stats"]["filament_used"]),
                    progress: nonneg(&data["virtual_sdcard"]["progress"]),
                });
            }
            _ => {
                out.quiet_ts = Some(ts);
                if matches!(state, "complete" | "error" | "cancelled") {
                    out.terminal = Some((filename.to_owned(), state.to_owned()));
                }
            }
        }
        out
    }
    /// One sample in; Some(session) out when this sample closed one. Pure aside from the internal
    /// state, so the notification transition detection reuses the same state machine (collector).
    pub fn observe(&mut self, ts: f64, data: &Value) -> Option<PrintSession> {
        let state = data["print_stats"]["state"].as_str().unwrap_or("");
        let filename = data["print_stats"]["filename"].as_str().unwrap_or("");
        let duration_s = nonneg(&data["print_stats"]["print_duration"]);
        let filament_used = nonneg(&data["print_stats"]["filament_used"]);
        let progress = nonneg(&data["virtual_sdcard"]["progress"]);
        // An interrupted close happens on a sample that already belongs to something else (standby
        // after a Klipper restart reads 0.0 everywhere, or another file), so the print's own last
        // readings are the truth; only a terminal state's sample describes the print it closes.
        let close = |open: &OpenPrint, result: &str, evidence: String| {
            let own = result != "interrupted";
            let pick = |now: Option<f64>, last: Option<f64>| if own { now.or(last) } else { last.or(now) };
            PrintSession {
                start_ts: open.start_ts,
                end_ts: if own { ts } else { open.last_ts },
                filename: open.filename.clone(),
                duration_s: pick(duration_s, open.duration_s),
                filament_used: pick(filament_used, open.filament_used),
                result: result.to_owned(),
                progress: pick(progress, open.progress),
                evidence,
            }
        };
        match state {
            "printing" | "paused" => {
                // Same file, same session: keep its start and its joined flag, refresh the
                // rolling duration/filament/progress readings.
                if let Some(open) = self.open.as_mut().filter(|o| o.filename == filename) {
                    open.duration_s = duration_s.or(open.duration_s);
                    open.filament_used = filament_used.or(open.filament_used);
                    open.progress = progress.or(open.progress);
                    open.last_ts = ts;
                    return None;
                }
                // A different filename while a session is open: the previous print ended without a
                // terminal state recorded (restart, cancelled from the panel). It closes honestly,
                // and this same sample opens the new file's session, not the next one.
                let ended = self.open.take();
                // Fully recorded only when the tracker saw the machine quiet just before. because
                // a quiet sample from before a hole in the recording (service stopped, printer
                // unreachable) says nothing about when this print began. the ended print's last
                // sample is a witness too (another file, the rule run_start applies after a
                // restart), whichever of the two is newer.
                let witness = ended
                    .as_ref()
                    .map(|o| o.last_ts)
                    .into_iter()
                    .chain(self.quiet_ts)
                    .reduce(f64::max);
                self.open = Some(OpenPrint {
                    filename: filename.to_owned(),
                    start_ts: ts,
                    joined_mid_print: !seen_start(witness, ts),
                    recovered: false,
                    last_ts: ts,
                    duration_s,
                    filament_used,
                    progress,
                });
                ended.map(|open| {
                    close(
                        &open,
                        "interrupted",
                        "no completion was recorded before a different file started printing"
                            .into(),
                    )
                })
            }
            "complete" | "error" | "cancelled" => {
                self.quiet_ts = Some(ts);
                // The previous terminal is needed to decide whether THIS terminal is news: a
                // recorder that joins after a print finished records it once, and then never
                // again, but the check has to see the terminal state from BEFORE this sample.
                let prior = self.terminal.clone();
                self.terminal = Some((filename.to_owned(), state.to_owned()));
                let open = self.open.take();
                match open {
                    Some(open) if open.filename == filename => {
                        let start = if open.joined_mid_print {
                            "recorder joined this print mid-run; start time is the first sample seen"
                        } else if open.recovered {
                            "recorder restarted mid-print; start recovered from its own recording"
                        } else {
                            "start recorded"
                        };
                        // A terminal state first seen after a gap in the recording ended
                        // somewhere inside that gap. The row still ends at this sample, the only
                        // recorded proof, and says how much earlier it may have finished. The
                        // Files modal shows end_ts as a local time and this text as its tooltip,
                        // so the gap is a duration against that shown time, not raw timestamps.
                        let gap = ts - open.last_ts;
                        let evidence = if gap > START_SEEN_S {
                            format!("{start}; completion seen after a gap in the recording: it finished in the {} before the recorded end", span(gap))
                        } else if start == "start recorded" {
                            "start and completion both recorded".to_owned()
                        } else {
                            start.to_owned()
                        };
                        Some(close(&open, state, evidence))
                    }
                    Some(open) => Some(close(
                        &open,
                        "interrupted",
                        "no completion was recorded before the printer reported another file finished".into(),
                    )),
                    // No open session but a terminal state: the recorder started (or restarted)
                    // after this print began. Recorded once, flagged, never repeated.
                    None if !filename.is_empty()
                        && prior.as_ref().is_none_or(|(prior_file, _)| prior_file != filename) =>
                    {
                        Some(PrintSession {
                            start_ts: ts,
                            end_ts: ts,
                            filename: filename.to_owned(),
                            duration_s,
                            filament_used,
                            result: state.to_owned(),
                            progress,
                            evidence: "recorder joined after this print finished; duration and start are unknown".into(),
                        })
                    }
                    None => None,
                }
            }
            // Standby / unknown: an open session ends without Klipper reporting a terminal state
            // only when the stream moves to another file, which the arm above handles. A standby
            // with the same filename still attached is Klipper idling after the report we already
            // consumed; it closes nothing.
            _ => {
                self.quiet_ts = Some(ts);
                if filename.is_empty() {
                    if let Some(open) = self.open.take() {
                        return Some(close(
                            &open,
                            "interrupted",
                            "no completion was recorded before the sample stream left the file"
                                .into(),
                        ));
                    }
                }
                None
            }
        }
    }
}
fn nonneg(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}

pub struct Store {
    pub conn: Connection,
    last_ts: Option<f64>,
    compact: bool,
    write_state: Option<Value>,
    keyframe_ts: Option<f64>,
    keyframe_bytes: usize,
    sessions: PrintTracker,
}
pub struct Written {
    pub ts: Option<f64>,
    pub samples: u64,
    pub events: u64,
    pub rounds: u64,
    pub outcome: Option<PrintSession>,
    pub ms: f64,
}

pub fn db_error(e: rusqlite::Error) -> String {
    format!("recording database error: {e}; check free disk space, file permissions, and that the recording is a valid SQLite file")
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name=?1)");
    conn.query_row(&sql, [column], |r| r.get(0))
        .map_err(db_error)
}

impl Store {
    pub fn open(path: &str, readonly: bool) -> Result<Self> {
        let flags = if readonly {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags).map_err(db_error)?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(db_error)?;
        conn.execute_batch("PRAGMA cache_size=-512; PRAGMA temp_store=MEMORY;")
            .map_err(db_error)?;
        if readonly {
            conn.execute_batch("PRAGMA query_only=ON;")
                .map_err(db_error)?;
        } else {
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
                CREATE TABLE IF NOT EXISTS samples (ts REAL PRIMARY KEY, data TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS context (id INTEGER PRIMARY KEY, sha TEXT UNIQUE NOT NULL, data TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS events (ts REAL, kind TEXT, message TEXT);
                CREATE TABLE IF NOT EXISTS qgl (ts REAL, run_id INTEGER, round INTEGER, range_mm REAL,
                    z0 REAL, z1 REAL, z2 REAL, z3 REAL);
                CREATE TABLE IF NOT EXISTS probe_points (ts REAL, context TEXT, x REAL, y REAL, z REAL);
                CREATE TABLE IF NOT EXISTS print_context (ts REAL, filename TEXT, metadata TEXT, orca TEXT);
                CREATE TABLE IF NOT EXISTS print_sessions (start_ts REAL PRIMARY KEY, end_ts REAL NOT NULL,
                    filename TEXT NOT NULL, duration_s REAL, filament_used REAL, result TEXT NOT NULL,
                    progress REAL, evidence TEXT NOT NULL);
                CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
                CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind,ts);
                CREATE INDEX IF NOT EXISTS qgl_ts ON qgl(ts);
                CREATE INDEX IF NOT EXISTS probe_points_ts ON probe_points(ts);").map_err(db_error)?;
            if !has_column(&conn, "samples", "kind")? {
                conn.execute("ALTER TABLE samples ADD COLUMN kind TEXT", [])
                    .map_err(db_error)?;
            }
            if !has_column(&conn, "samples", "context_id")? {
                conn.execute("ALTER TABLE samples ADD COLUMN context_id INTEGER", [])
                    .map_err(db_error)?;
            }
            conn.execute_batch("CREATE INDEX IF NOT EXISTS samples_kind_ts ON samples(kind,ts);")
                .map_err(db_error)?;
        }
        let compact =
            has_column(&conn, "samples", "kind")? && has_column(&conn, "samples", "context_id")?;
        let last_ts = conn
            .query_row("SELECT MAX(ts) FROM samples", [], |r| r.get(0))
            .map_err(db_error)?;
        conn.prepare("SELECT ts,kind,message FROM events LIMIT 0")
            .map_err(db_error)?;
        conn.prepare("SELECT ts,run_id,round,range_mm,z0,z1,z2,z3 FROM qgl LIMIT 0")
            .map_err(db_error)?;
        let mut store = Self {
            conn,
            last_ts,
            compact,
            write_state: None,
            keyframe_ts: None,
            keyframe_bytes: 0,
            sessions: PrintTracker::default(),
        };
        if !readonly {
            if let Some((ts, data, _)) = store.load_sample(f64::MAX, SAMPLE_CAP)? {
                store.write_state = Some(split_context(&data)?.0);
                // A restart mid-print continues the open session instead of opening a second one.
                let mut sessions = PrintTracker::seed(ts, &data);
                // ...and keeps the start its own recording holds, instead of restarting the clock
                // at the last sample before the restart. A replay that cannot read the recording
                // leaves the seed's honest default (joined, at the newest sample): the recorder
                // starting matters more than the session's start time.
                if let Some(open) = sessions.open.as_mut() {
                    if let Ok((start, quiet)) = store.run_start(ts, &open.filename) {
                        open.start_ts = start;
                        open.joined_mid_print = !seen_start(quiet, start);
                        open.recovered = true;
                    }
                }
                store.sessions = sessions;
            }
            if compact {
                let base: Option<(f64, usize)> = store
                    .conn
                    .query_row(
                        "SELECT ts,length(CAST(data AS BLOB)) FROM samples
                    WHERE kind='K' OR kind IS NULL ORDER BY ts DESC LIMIT 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(db_error)?;
                if let Some((ts, n)) = base {
                    store.keyframe_ts = Some(ts);
                    store.keyframe_bytes = n;
                }
            }
        }
        Ok(store)
    }

    pub fn bounds(&self) -> Result<(Option<f64>, Option<f64>)> {
        self.conn
            .query_row(
                "SELECT (SELECT MIN(ts) FROM samples),(SELECT MAX(ts) FROM samples)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)
    }
    pub fn counts(&self) -> Result<(u64, u64, u64)> {
        self.conn.query_row("SELECT (SELECT COUNT(*) FROM samples),(SELECT COUNT(*) FROM events),(SELECT COUNT(*) FROM qgl)",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)
    }
    pub fn last_run(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COALESCE(MAX(run_id),0) FROM qgl", [], |r| r.get(0))
            .map_err(db_error)
    }
    // BLTouch probe points: every recorded probe landing with its time, position and context.
    // Additive table (2026-09-17); old recordings simply have none.
    pub fn write_probe_points(&mut self, points: &[crate::probe::ProbePoint]) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction().map_err(db_error)?;
        let mut n = 0;
        for p in points {
            if !p.ts.is_finite() || !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(
                    "probe point coordinates must be finite; check Klipper console output".into(),
                );
            }
            n += tx
                .execute(
                    "INSERT INTO probe_points(ts,context,x,y,z) VALUES(?1,?2,?3,?4,?5)",
                    rusqlite::params![p.ts, p.context, p.x, p.y, p.z],
                )
                .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(n)
    }
    pub fn probe_points(
        &self,
        from: f64,
        to: f64,
        limit: usize,
    ) -> Result<Vec<crate::probe::ProbePoint>> {
        let mut stmt = self.conn
            .prepare("SELECT ts,context,x,y,z FROM probe_points WHERE ts>=?1 AND ts<=?2 ORDER BY ts DESC LIMIT ?3")
            .map_err(db_error)?;
        let rows = stmt
            .query_map(rusqlite::params![from, to, limit as i64], |r| {
                Ok(crate::probe::ProbePoint {
                    ts: r.get(0)?,
                    context: r.get(1)?,
                    x: r.get(2)?,
                    y: r.get(3)?,
                    z: r.get(4)?,
                })
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_error)?);
        }
        out.reverse();
        Ok(out)
    }
    // Print context: one row per observed print start — slicer metadata plus Orca's settings
    // block read once from the file tail. Additive table (2026-09-17).
    pub fn write_print_context(
        &mut self,
        ts: f64,
        filename: &str,
        metadata: &Value,
        orca: &Value,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO print_context(ts,filename,metadata,orca) VALUES(?1,?2,?3,?4)",
                rusqlite::params![
                    ts,
                    filename,
                    serde_json::to_string(metadata).map_err(|e| e.to_string())?,
                    serde_json::to_string(orca).map_err(|e| e.to_string())?
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// Finished prints, newest first. Every row is a real recorded transition, never an
    /// extrapolation; rows the recorder only joined mid-run say so in `evidence`.
    pub fn print_sessions(&self, from: f64, to: f64, limit: usize) -> Result<Vec<PrintSession>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT start_ts,end_ts,filename,duration_s,filament_used,result,progress,evidence
             FROM print_sessions WHERE end_ts>=?1 AND end_ts<=?2
             ORDER BY start_ts DESC LIMIT ?3",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(rusqlite::params![from, to, limit as i64], |r| {
                Ok(PrintSession {
                    start_ts: r.get(0)?,
                    end_ts: r.get(1)?,
                    filename: r.get(2)?,
                    duration_s: r.get(3)?,
                    filament_used: r.get(4)?,
                    result: r.get(5)?,
                    progress: r.get(6)?,
                    evidence: r.get(7)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }
    pub fn print_context(
        &self,
        from: f64,
        to: f64,
        limit: usize,
    ) -> Result<Vec<(f64, String, Value, Value)>> {
        let mut stmt = self.conn
            .prepare("SELECT ts,filename,metadata,orca FROM print_context WHERE ts>=?1 AND ts<=?2 ORDER BY ts DESC LIMIT ?3")
            .map_err(db_error)?;
        let rows = stmt
            .query_map(rusqlite::params![from, to, limit as i64], |r| {
                Ok((
                    r.get::<_, f64>(0)?,
                    r.get::<_, String>(1)?,
                    serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
                ))
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_error)?);
        }
        out.reverse();
        Ok(out)
    }
    pub fn at(&self, ts: f64) -> Result<Option<(f64, Value)>> {
        Ok(self
            .load_sample(ts, SAMPLE_CAP)?
            .map(|(ts, data, _)| (ts, data)))
    }
    pub fn load_sample(&self, ts: f64, cap: usize) -> Result<Option<(f64, Value, usize)>> {
        let target: Option<f64> = self
            .conn
            .query_row(
                "SELECT ts FROM samples WHERE ts<=?1 ORDER BY ts DESC LIMIT 1",
                [ts],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(target) = target else {
            return Ok(None);
        };
        if !target.is_finite() || target < 0.0 {
            return Err("invalid recorded timestamp; preserve this database for repair".into());
        }
        if !self.compact {
            let (n, s): (usize, String) = self
                .conn
                .query_row(
                    "SELECT length(CAST(data AS BLOB)),data FROM samples WHERE ts=?1",
                    [target],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(db_error)?;
            if n > cap {
                return Err("recorded status exceeds the input limit; reduce the recorded object payload or raise the configured cap".into());
            }
            return Ok(Some((target, decode_sample(target, &s)?, n)));
        }
        let base: Option<f64> = self
            .conn
            .query_row(
                "SELECT ts FROM samples WHERE ts<=?1 AND (kind='K' OR kind IS NULL)
            ORDER BY ts DESC LIMIT 1",
                [target],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(base) = base else {
            return Err(format!(
                "sample at {target} has no keyframe; preserve this recording for repair"
            ));
        };
        let mut q = self
            .conn
            .prepare(
                "SELECT ts,data,kind,context_id FROM samples WHERE ts>=?1 AND ts<=?2 ORDER BY ts",
            )
            .map_err(db_error)?;
        let mut rows = q.query(params![base, target]).map_err(db_error)?;
        let mut state = None;
        let mut context_id: Option<i64> = None;
        while let Some(row) = rows.next().map_err(db_error)? {
            let row_ts: f64 = row.get(0).map_err(db_error)?;
            let text: String = row.get(1).map_err(db_error)?;
            if text.len() > cap {
                return Err("recorded sample fragment exceeds the input limit; preserve and inspect this recording".into());
            }
            let kind: Option<String> = row.get(2).map_err(db_error)?;
            match kind.as_deref() {
                None|Some("K")=>state=Some(decode_sample(row_ts,&text)?),
                Some("D")=>apply_delta(state.as_mut().ok_or_else(||format!("delta at {row_ts} has no keyframe; preserve this recording for repair"))?,&decode_sample(row_ts,&text)?)?,
                Some(other)=>return Err(format!("sample at {row_ts} has unknown kind {other}; preserve this recording for repair")),
            }
            context_id = row.get(3).map_err(db_error)?;
        }
        let mut state = state.ok_or_else(|| {
            format!(
                "sample at {target} could not be reconstructed; preserve this recording for repair"
            )
        })?;
        if let Some(id) = context_id {
            let (n, text): (usize, String) = self
                .conn
                .query_row(
                    "SELECT length(CAST(data AS BLOB)),data FROM context WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(db_error)?;
            if n > cap {
                return Err(
                    "recorded context exceeds the input limit; preserve and inspect this recording"
                        .into(),
                );
            }
            merge(&mut state, decode_sample(target, &text)?);
        }
        let n = serde_json::to_vec(&state)
            .map_err(|e| format!("cannot measure reconstructed sample at {target}: {e}"))?
            .len();
        if n > cap {
            return Err("reconstructed status exceeds the input limit; reduce the recorded object payload or raise the configured cap".into());
        }
        Ok(Some((target, state, n)))
    }
    /// Where the print holding the newest sample began, for a recorder restarted mid-print. The run
    /// is the contiguous stretch of samples printing/paused with `file`. Returns its first sample
    /// and the sample just before it, which is quiet by construction (idle, finished, or another
    /// file); None when the run reaches the start of the recording or of the scan. A state change
    /// is stored as a delta, not a keyframe: the 2026-09-21 cancel (11:53:19) and reprint
    /// (11:54:37) of the same file both are, so reading keyframes alone would start that reprint at
    /// the 11:54:45 keyframe, and a cancel and reprint inside one 60 s keyframe interval would
    /// vanish. The run is replayed through the same keyframe/delta decoder load_sample uses.
    // Ponytail: replays every sample of the current run from the newest keyframe outside it, never
    // more than RUN_SCAN_S back. Measured 2026-09-22 on the 55 min Voron run of 2026-09-17 (8385
    // samples): +110 ms to service start, so about 6 s at the 48 h bound. Skip deltas that cannot
    // touch print_stats in the SQL, or record print starts as they happen, if prints get that long.
    fn run_start(&self, newest: f64, file: &str) -> Result<(f64, Option<f64>)> {
        let in_run = |d: &Value| {
            matches!(
                d["print_stats"]["state"].as_str(),
                Some("printing" | "paused")
            ) && d["print_stats"]["filename"].as_str() == Some(file)
        };
        let mut keyframes = self
            .conn
            .prepare(
                "SELECT ts,data FROM samples WHERE ts>=?1 AND ts<=?2 AND (kind='K' OR kind IS NULL)
                 ORDER BY ts DESC",
            )
            .map_err(db_error)?;
        let mut rows = keyframes
            .query(params![newest - RUN_SCAN_S, newest])
            .map_err(db_error)?;
        let mut from = None;
        while let Some(row) = rows.next().map_err(db_error)? {
            let ts: f64 = row.get(0).map_err(db_error)?;
            from = Some(ts);
            if !in_run(&decode_sample(ts, &row.get::<_, String>(1).map_err(db_error)?)?) {
                break;
            }
        }
        let Some(from) = from else {
            return Ok((newest, None));
        };
        // Every row, nothing narrowed: the quiet witness is the sample right before the run.
        let (mut prev, mut run) = (None::<(f64, bool)>, (newest, None));
        self.replay(from, newest, &[], |ts, sample| {
            let now = in_run(sample);
            match prev {
                None if now => run = (ts, None),
                Some((quiet, false)) if now => run = (ts, Some(quiet)),
                _ => {}
            }
            prev = Some((ts, now));
            true
        })?;
        Ok(run)
    }
    /// The one forward replay (run_start above, the history routes). Every sample from the
    /// keyframe at or before `from` to `to` is rebuilt through the decoder load_sample uses
    /// (decode_sample + apply_delta) and handed to `visit` in time order from `from` on; `visit`
    /// returns false to stop. With no `paths` every row is rebuilt whole and visited. With JSON
    /// pointers the caller will read, SQL narrows what reaches the decoder: a keyframe comes back
    /// trimmed to those pointers' top-level objects, and a delta only when its text names a
    /// pointer's second segment (a change under /A/B always writes "B" into A's delta) or it
    /// removes anything (a whole object can go without naming its fields). The rebuilt sample is
    /// then exact for those objects and holds nothing else, and only fetched rows are visited.
    pub fn replay(
        &self,
        from: f64,
        to: f64,
        paths: &[&str],
        mut visit: impl FnMut(f64, &Value) -> bool,
    ) -> Result<()> {
        let (mut keys, mut terms): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        for path in paths {
            let mut segment = path.trim_start_matches('/').split('/');
            let key = segment.next().unwrap_or_default();
            let term = segment.next().unwrap_or(key);
            if !keys.contains(&key) {
                keys.push(key);
            }
            if !terms.contains(&term) {
                terms.push(term);
            }
        }
        let mut args: Vec<rusqlite::types::Value> = vec![from.into(), to.into()];
        // A recording from before the compact format is one full sample per row.
        let sql = if !self.compact {
            "SELECT ts,data,NULL FROM samples WHERE ts>=?1 AND ts<=?2 ORDER BY ts".to_owned()
        } else {
            let base: Option<f64> = self
                .conn
                .query_row(
                    "SELECT MAX(ts) FROM samples WHERE ts<=?1 AND (kind='K' OR kind IS NULL)",
                    [from],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            args[0] = base.unwrap_or(from).into();
            let (mut data, mut narrow) = ("data".to_owned(), String::new());
            if !keys.is_empty() {
                let mut pairs = Vec::new();
                for key in &keys {
                    args.push((*key).to_owned().into());
                    args.push(format!("$.\"{key}\"").into());
                    pairs.push(format!("?{},data -> ?{}", args.len() - 1, args.len()));
                }
                data = format!(
                    "CASE WHEN kind IS NOT 'D' THEN json_object({}) ELSE data END",
                    pairs.join(",")
                );
                narrow.push_str(" AND (kind IS NOT 'D' OR instr(data,'\"remove\":[]')=0");
                for term in &terms {
                    args.push(format!("\"{term}\"").into());
                    narrow.push_str(&format!(" OR instr(data,?{})>0", args.len()));
                }
                narrow.push(')');
            }
            format!("SELECT ts,{data},kind FROM samples WHERE ts>=?1 AND ts<=?2{narrow} ORDER BY ts")
        };
        let mut q = self.conn.prepare(&sql).map_err(db_error)?;
        let mut rows = q
            .query(rusqlite::params_from_iter(args))
            .map_err(db_error)?;
        let mut state: Option<Value> = None;
        while let Some(row) = rows.next().map_err(db_error)? {
            let ts: f64 = row.get(0).map_err(db_error)?;
            let data = decode_sample(ts, &row.get::<_, String>(1).map_err(db_error)?)?;
            match row
                .get::<_, Option<String>>(2)
                .map_err(db_error)?
                .as_deref()
            {
                None | Some("K") => state = Some(data),
                Some("D") => apply_delta(
                    state.as_mut().ok_or_else(|| {
                        format!("delta at {ts} has no keyframe; preserve this recording for repair")
                    })?,
                    &data,
                )?,
                Some(other) => {
                    return Err(format!(
                        "sample at {ts} has unknown kind {other}; preserve this recording for repair"
                    ))
                }
            }
            if let Some(sample) = &state {
                if ts >= from && !visit(ts, sample) {
                    break;
                }
            }
        }
        Ok(())
    }
    pub fn sample_times(&self, from: f64, to: f64, limit: usize) -> Result<Vec<f64>> {
        let mut q = self
            .conn
            .prepare("SELECT ts FROM samples WHERE ts>=?1 AND ts<=?2 ORDER BY ts LIMIT ?3")
            .map_err(db_error)?;
        let rows = q
            .query_map(params![from, to, limit as i64], |r| r.get(0))
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }
    pub fn context_at(&self, ts: f64, kind: &str) -> Result<Option<Value>> {
        let s: Option<String> = self.conn.query_row(
            "SELECT message FROM events WHERE ts<=?1 AND kind=?2 ORDER BY ts DESC,rowid DESC LIMIT 1",
            params![ts,kind], |r| r.get(0)).optional().map_err(db_error)?;
        s.map(|s| {
            serde_json::from_str(&s).map_err(|e| {
                format!("invalid recorded {kind}: {e}; restore the recording from a backup")
            })
        })
        .transpose()
    }
    pub fn events(&self, from: f64, to: f64, limit: usize) -> Result<Vec<Event>> {
        let mut q = self.conn.prepare("SELECT ts,kind,message FROM events WHERE ts>=?1 AND ts<=?2
            AND kind NOT IN ('scope_health','scope_geometry','scope_origin') ORDER BY ts DESC,rowid DESC LIMIT ?3").map_err(db_error)?;
        let rows = q
            .query_map(params![from, to, limit as i64], |r| {
                Ok(Event {
                    ts: r.get(0)?,
                    kind: r.get(1)?,
                    message: r.get(2)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }
    pub fn rounds(&self, from: f64, to: f64, limit: usize) -> Result<Vec<Round>> {
        let mut q = self
            .conn
            .prepare(
                "SELECT ts,run_id,round,range_mm,z0,z1,z2,z3 FROM qgl
            WHERE ts>=?1 AND ts<=?2 ORDER BY ts DESC,rowid DESC LIMIT ?3",
            )
            .map_err(db_error)?;
        let rows = q
            .query_map(params![from, to, limit as i64], |r| {
                Ok(Round {
                    ts: r.get(0)?,
                    run_id: r.get(1)?,
                    round: r.get(2)?,
                    range_mm: r.get(3)?,
                    z0: r.get(4)?,
                    z1: r.get(5)?,
                    z2: r.get(6)?,
                    z3: r.get(7)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn series(&self, from: f64, to: f64, points: usize) -> Result<Vec<Value>> {
        if points < 2 || !from.is_finite() || !to.is_finite() || from > to {
            return Err("series needs finite from<=to timestamps and at least two points".into());
        }
        let first: Option<f64> = self
            .conn
            .query_row(
                "SELECT MIN(ts) FROM samples WHERE ts>=?1 AND ts<=?2",
                params![from, to],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let Some(first) = first else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(points);
        let mut previous = None;
        for i in 0..points {
            let target = if i == 0 {
                first
            } else {
                from + (to - from) * i as f64 / (points - 1) as f64
            };
            if let Some((ts, data, _)) = self.load_sample(target, SAMPLE_CAP)? {
                if ts >= from && previous != Some(ts) {
                    out.push(model::point(ts, &data));
                    previous = Some(ts);
                }
            }
        }
        Ok(out)
    }

    pub fn write(
        &mut self,
        sample: Option<(f64, &Value)>,
        events: &[Event],
        parser: &mut Parser,
    ) -> Result<Written> {
        let begin = Instant::now();
        if sample.is_some_and(|(ts, _)| !ts.is_finite() || ts < 0.0) {
            return Err("sample timestamp must be finite and non-negative; check the recorder's system clock".into());
        }
        let actual_ts = sample.map(|(ts, _)| {
            self.last_ts
                .map(|last| ts.max(last + 0.000001))
                .unwrap_or(ts)
        });
        let encoded = if let (Some((_, data)), Some(ts)) = (sample, actual_ts) {
            let (core, context) = split_context(data)?;
            let full = serde_json::to_string(&core)
                .map_err(|e| format!("cannot encode sample: {e}; check printer JSON"))?;
            let delta = self
                .write_state
                .as_ref()
                .map(|prior| delta(prior, &core))
                .transpose()?;
            let delta_text = delta
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| format!("cannot encode sample delta: {e}"))?;
            let keyframe = self.write_state.is_none()
                || self
                    .keyframe_ts
                    .is_none_or(|base| ts - base >= KEYFRAME_SECONDS)
                || delta_text
                    .as_ref()
                    .is_none_or(|d| d.len() > self.keyframe_bytes / 2);
            Some((
                core,
                context,
                if keyframe { "K" } else { "D" },
                if keyframe {
                    full.clone()
                } else {
                    delta_text.unwrap()
                },
                full.len(),
            ))
        } else {
            None
        };
        let mut next_parser = parser.clone();
        // Print-session extraction runs on the same sample the write stores, so the session row
        // lands in the same transaction and a crash replays from the seeded tracker on reopen.
        let closed = if let (Some((_, data)), Some(ts)) = (sample, actual_ts) {
            self.sessions.observe(ts, data)
        } else {
            None
        };
        let tx = self.conn.transaction().map_err(db_error)?;
        let mut result = Written {
            ts: actual_ts,
            samples: 0,
            events: 0,
            rounds: 0,
            outcome: None,
            ms: 0.0,
        };
        if let (Some((_, context, kind, text, _)), Some(ts)) = (&encoded, actual_ts) {
            let context_id = store_context(&tx, context)?;
            tx.execute(
                "INSERT INTO samples(ts,data,kind,context_id) VALUES(?1,?2,?3,?4)",
                params![ts, text, kind, context_id],
            )
            .map_err(db_error)?;
            result.samples = 1;
        }
        for e in events {
            if !e.ts.is_finite() {
                return Err("console timestamp is not finite; inspect Moonraker's clock".into());
            }
            let inserted = insert_event(&tx, e).map_err(db_error)?;
            result.events += inserted as u64;
            if inserted == 0 || e.kind == "command" || e.kind.starts_with("scope_") {
                continue;
            }
            for round in next_parser.feed(e.ts, &e.message) {
                tx.execute("INSERT INTO qgl(ts,run_id,round,range_mm,z0,z1,z2,z3) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![round.ts,round.run_id,round.round,round.range_mm,round.z0,round.z1,round.z2,round.z3]).map_err(db_error)?;
                result.rounds += 1;
            }
            if e.message.contains("Retries aborting:") {
                result.events += insert_event(
                    &tx,
                    &Event::new(
                        e.ts,
                        "levelling_failed",
                        format!("Run {}: {}", next_parser.run_id, e.message),
                    ),
                )
                .map_err(db_error)? as u64;
            }
            if e.message.contains("position skew after probe:") && e.kind != "position_warning" {
                result.events += insert_event(
                    &tx,
                    &Event::new(
                        e.ts,
                        "position_warning",
                        format!("Position discrepancy reported: {}", e.message),
                    ),
                )
                .map_err(db_error)? as u64;
            }
        }
        if let Some(session) = &closed {
            tx.execute(
                "INSERT INTO print_sessions(start_ts,end_ts,filename,duration_s,filament_used,result,progress,evidence)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    session.start_ts,
                    session.end_ts,
                    session.filename,
                    session.duration_s,
                    session.filament_used,
                    session.result,
                    session.progress,
                    session.evidence
                ],
            )
            .map_err(db_error)?;
            // Terminal outcomes (not "interrupted") are what the collector announces.
            if matches!(session.result.as_str(), "complete" | "error" | "cancelled") {
                result.outcome = Some(session.clone());
            }
        }
        tx.commit().map_err(db_error)?;
        if let (Some((core, _, kind, _, full_len)), Some(ts)) = (encoded, actual_ts) {
            self.last_ts = Some(ts);
            self.write_state = Some(core);
            if kind == "K" {
                self.keyframe_ts = Some(ts);
                self.keyframe_bytes = full_len;
            }
        }
        *parser = next_parser;
        result.ms = begin.elapsed().as_secs_f64() * 1000.0;
        Ok(result)
    }

    pub fn prune(&mut self, cutoff: f64, max_db_mb: u64, active_filename: Option<&str>) -> Result<()> {
        if !cutoff.is_finite() {
            return Err("retention cutoff is not finite; check the recorder's clock".into());
        }
        let boundary: Option<f64> = if self.compact {
            self.conn
                .query_row(
                    "SELECT MAX(ts) FROM samples WHERE ts<=?1 AND (kind='K' OR kind IS NULL)",
                    [cutoff],
                    |r| r.get(0),
                )
                .map_err(db_error)?
        } else {
            self.conn
                .query_row("SELECT MAX(ts) FROM samples WHERE ts<=?1", [cutoff], |r| {
                    r.get(0)
                })
                .map_err(db_error)?
        };
        self.prune_before(boundary, cutoff, active_filename)?;
        let max = max_db_mb.saturating_mul(1024 * 1024);
        while self.used_bytes()? > max {
            let clause = if self.compact {
                "kind='K' OR kind IS NULL"
            } else {
                "1"
            };
            let groups: u64 = self
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM samples WHERE {clause}"),
                    [],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if groups <= 1 {
                break;
            }
            let used = self.used_bytes()?;
            let drop = ((groups as u128 * (used - max) as u128 + used as u128 - 1) / used as u128)
                .clamp(1, (groups - 1) as u128) as u64;
            let boundary: f64 = self
                .conn
                .query_row(
                    &format!("SELECT ts FROM samples WHERE {clause} ORDER BY ts LIMIT 1 OFFSET ?1"),
                    [drop],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            self.prune_before(Some(boundary), boundary, active_filename)?;
        }
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(db_error)?;
        Ok(())
    }
    fn used_bytes(&self) -> Result<u64> {
        let page: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(db_error)?;
        let pages: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(db_error)?;
        let free: u64 = self
            .conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(db_error)?;
        Ok(pages.saturating_sub(free).saturating_mul(page))
    }
    fn prune_before(&mut self, sample_boundary: Option<f64>, timeline_boundary: f64, active_filename: Option<&str>) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_error)?;
        for kind in ["scope_geometry", "scope_health", "scope_origin"] {
            let row: Option<(f64,String)>=tx.query_row("SELECT ts,message FROM events WHERE kind=?1 AND ts<=?2 ORDER BY ts DESC,rowid DESC LIMIT 1",
                params![kind,timeline_boundary],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
            if let Some((ts, message)) = row {
                if ts < timeline_boundary {
                    insert_event(&tx, &Event::new(timeline_boundary, kind, message))
                        .map_err(db_error)?;
                }
            }
        }
        tx.execute("DELETE FROM probe_points WHERE ts<=?1", [timeline_boundary]).map_err(db_error)?;
        for (table, boundary) in [
            ("samples", sample_boundary),
            ("events", Some(timeline_boundary)),
            ("qgl", Some(timeline_boundary)),
        ] {
            let Some(boundary) = boundary else { continue };
            loop {
                let sql=format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE ts<?1 LIMIT 2000)");
                let n = tx.execute(&sql, [boundary]).map_err(db_error)?;
                if n < 2000 {
                    break;
                }
            }
        }
        if self.compact {
            tx.execute("DELETE FROM context WHERE id NOT IN (SELECT DISTINCT context_id FROM samples WHERE context_id IS NOT NULL)",[]).map_err(db_error)?;
        }
        // Finished-print rows leave with the timeline. A session spanning the cutoff keeps its
        // end (it ended after it), so a row is never cut in half.
        tx.execute(
                "DELETE FROM print_sessions WHERE end_ts<?1",
                [timeline_boundary],
            )
            .map_err(db_error)?;
        tx.execute("DELETE FROM print_context WHERE ts<?1 AND NOT EXISTS (
            SELECT 1 FROM print_sessions s WHERE s.end_ts>=?1 AND s.filename=print_context.filename
            AND print_context.ts>=s.start_ts-60 AND print_context.ts<=s.end_ts
        ) AND (?2 IS NULL OR print_context.rowid!=(
            SELECT rowid FROM print_context WHERE filename=?2 ORDER BY ts DESC,rowid DESC LIMIT 1
        ))", params![timeline_boundary, active_filename]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }
}

fn decode_sample(ts: f64, text: &str) -> Result<Value> {
    let data: Value = serde_json::from_str(text).map_err(|e| {
        format!("sample at {ts} has invalid JSON: {e}; restore this recording from a backup")
    })?;
    if !data.is_object() {
        return Err(format!(
            "sample at {ts} is not a status object; preserve this recording for repair"
        ));
    }
    Ok(data)
}
fn split_context(data: &Value) -> Result<(Value, Value)> {
    let mut core = data
        .as_object()
        .cloned()
        .ok_or("printer sample must be a JSON object")?;
    let mut context = Map::new();
    for key in ["configfile", "_scope_capture"] {
        if let Some(v) = core.remove(key) {
            context.insert(key.into(), v);
        }
    }
    if let Some(Value::Object(bed)) = core.get_mut("bed_mesh") {
        if let Some(v) = bed.remove("profiles") {
            context.entry("bed_mesh").or_insert_with(|| json!({}))["profiles"] = v;
        }
    }
    let mcu: Vec<String> = core
        .keys()
        .filter(|k| *k == "mcu" || k.starts_with("mcu "))
        .cloned()
        .collect();
    for name in mcu {
        if let Some(Value::Object(fields)) = core.get_mut(&name) {
            for key in ["mcu_version", "mcu_build_versions", "mcu_constants"] {
                if let Some(v) = fields.remove(key) {
                    context.entry(name.clone()).or_insert_with(|| json!({}))[key] = v;
                }
            }
        }
    }
    Ok((Value::Object(core), Value::Object(context)))
}
fn merge(base: &mut Value, extra: Value) {
    match extra {
        Value::Object(extra) if base.is_object() => {
            let base = base.as_object_mut().unwrap();
            for (k, v) in extra {
                if let Some(old) = base.get_mut(&k) {
                    merge(old, v)
                } else {
                    base.insert(k, v);
                }
            }
        }
        other => *base = other,
    }
}
fn delta(old: &Value, new: &Value) -> Result<Value> {
    let old = old.as_object().ok_or("previous sample is not an object")?;
    let new = new.as_object().ok_or("printer sample is not an object")?;
    let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
    let mut set = Map::new();
    let mut remove = Vec::new();
    for key in keys {
        match (old.get(key), new.get(key)) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let fields: BTreeSet<_> = a.keys().chain(b.keys()).collect();
                let mut changed = Map::new();
                for field in fields {
                    match (a.get(field), b.get(field)) {
                        (Some(x), Some(y)) if x == y => {}
                        (_, Some(y)) => {
                            changed.insert(field.clone(), y.clone());
                        }
                        (Some(_), None) => remove.push(json!([key, field])),
                        _ => {}
                    }
                }
                if !changed.is_empty() {
                    set.insert(key.clone(), Value::Object(changed));
                }
            }
            (Some(a), Some(b)) if a == b => {}
            (_, Some(v)) => {
                set.insert(key.clone(), v.clone());
            }
            (Some(_), None) => remove.push(json!([key])),
            _ => {}
        }
    }
    Ok(json!({"set":set,"remove":remove}))
}
fn apply_delta(state: &mut Value, patch: &Value) -> Result<()> {
    let state = state
        .as_object_mut()
        .ok_or("keyframe is not a JSON object")?;
    let set = patch
        .get("set")
        .and_then(Value::as_object)
        .ok_or("sample delta has no set object")?;
    let remove = patch
        .get("remove")
        .and_then(Value::as_array)
        .ok_or("sample delta has no remove list")?;
    for (key, value) in set {
        if let (Some(Value::Object(old)), Value::Object(fields)) = (state.get_mut(key), value) {
            for (k, v) in fields {
                old.insert(k.clone(), v.clone());
            }
        } else {
            state.insert(key.clone(), value.clone());
        }
    }
    for path in remove {
        let p = path
            .as_array()
            .ok_or("sample delta removal is not a path")?;
        match p.as_slice() {
            [Value::String(key)] => {
                state.remove(key);
            }
            [Value::String(key), Value::String(field)] => {
                if let Some(Value::Object(v)) = state.get_mut(key) {
                    v.remove(field);
                }
            }
            _ => return Err("sample delta removal path is invalid".into()),
        }
    }
    Ok(())
}
fn store_context(conn: &Connection, value: &Value) -> Result<Option<i64>> {
    if value.as_object().is_some_and(Map::is_empty) {
        return Ok(None);
    }
    let data =
        serde_json::to_string(value).map_err(|e| format!("cannot encode sample context: {e}"))?;
    let sha = sha256_hex(data.as_bytes());
    conn.execute(
        "INSERT OR IGNORE INTO context(sha,data) VALUES(?1,?2)",
        params![sha, data],
    )
    .map_err(db_error)?;
    let (id, stored): (i64, String) = conn
        .query_row("SELECT id,data FROM context WHERE sha=?1", [sha], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(db_error)?;
    if stored != data {
        return Err("sample context SHA-256 collision; recording stopped without replacing existing context".into());
    }
    Ok(Some(id))
}
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut bytes = data.to_vec();
    let bits = (bytes.len() as u64) * 8;
    bytes.push(0x80);
    while bytes.len() % 64 != 56 {
        bytes.push(0)
    }
    bytes.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for block in bytes.chunks_exact(64) {
        let mut w = [0_u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut z) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = z
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            z = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, z].iter().enumerate() {
            h[i] = h[i].wrapping_add(*v);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

fn insert_event(conn: &Connection, e: &Event) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO events(ts,kind,message) SELECT ?1,?2,?3
        WHERE NOT EXISTS(SELECT 1 FROM events WHERE ts=?1 AND message=?3)",
        params![e.ts, e.kind, e.message],
    )
}

pub fn sample_json(sample: Option<(f64, Value)>) -> Value {
    sample
        .map(|(ts, data)| json!({"ts":ts,"data":data,"summary":model::point(ts,&data)}))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn print_context_retention_keeps_spanning_and_active_runs_atomically() {
        let mut db = Store::open(":memory:", false).unwrap();
        for (ts, name) in [(1.0,"expired.gcode"),(2.0,"spanning.gcode"),(3.0,"active.gcode"),(18.0,"recent.gcode")] {
            db.write_print_context(ts, name, &json!({"slicer":"fixture"}), &json!({})).unwrap();
        }
        db.conn.execute("INSERT INTO print_sessions(start_ts,end_ts,filename,result,evidence) VALUES(2,20,'spanning.gcode','complete','fixture')", []).unwrap();
        db.conn.execute("INSERT INTO events(ts,kind,message) VALUES(1,'response','old')", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER refuse_context_delete BEFORE DELETE ON print_context BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert!(db.prune(10.0, 4096, Some("active.gcode")).is_err());
        assert_eq!(db.print_context(0.0, 30.0, 10).unwrap().len(), 4);
        assert_eq!(db.events(0.0, 30.0, 10).unwrap().len(), 1, "the failed prune rolls back earlier deletes");
        db.conn.execute_batch("DROP TRIGGER refuse_context_delete").unwrap();
        db.prune(10.0, 4096, Some("active.gcode")).unwrap();
        let names = db.print_context(0.0, 30.0, 10).unwrap().into_iter().map(|r|r.1).collect::<Vec<_>>();
        assert_eq!(names, ["spanning.gcode","active.gcode","recent.gcode"]);
        db.prune(10.0, 4096, None).unwrap();
        let names = db.print_context(0.0, 30.0, 10).unwrap().into_iter().map(|r|r.1).collect::<Vec<_>>();
        assert_eq!(names, ["spanning.gcode","recent.gcode"]);
    }
    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// The session state machine, exercised on the shapes the printer really produces.
    #[test]
    fn print_sessions_track_complete_interrupted_and_joined_runs() {
        let sample = |state: &str, file: &str, dur: f64| {
            json!({"print_stats":{"state":state,"filename":file,"print_duration":dur,"filament_used":1.25},
             "virtual_sdcard":{"progress":0.5}})
        };
        let mut t = PrintTracker::default();
        assert!(t.observe(10.0, &sample("standby", "", 0.0)).is_none());
        assert!(t
            .observe(20.0, &sample("printing", "cube.gcode", 60.0))
            .is_none());
        // A print the recorder watched to the end is sampled right up to it; without this sample
        // the 60 s of silence before the completion is, correctly, a gap.
        assert!(t
            .observe(70.0, &sample("printing", "cube.gcode", 700.0))
            .is_none());
        let closed = t
            .observe(80.0, &sample("complete", "cube.gcode", 720.0))
            .expect("complete closes the session");
        assert_eq!(closed.start_ts, 20.0);
        assert_eq!(closed.end_ts, 80.0);
        assert_eq!(closed.result, "complete");
        assert_eq!(closed.duration_s, Some(720.0));
        assert!(closed.evidence.contains("both recorded"));
        assert!(
            t.observe(85.0, &sample("complete", "cube.gcode", 720.0))
                .is_none(),
            "a repeated terminal state must not record a second row"
        );
        // A run that ends without a terminal state is recorded as interrupted, not invented.
        assert!(t
            .observe(100.0, &sample("printing", "second.gcode", 10.0))
            .is_none());
        let closed = t
            .observe(110.0, &sample("standby", "", 0.0))
            .expect("standby with no filename ends the open run");
        assert_eq!(closed.result, "interrupted");
        assert_eq!(closed.filename, "second.gcode");
        // The standby sample reads 0.0 after a Klipper restart; the row keeps the print's own
        // readings, and ends at its own last sample, not at the first sample after it.
        assert_eq!(closed.duration_s, Some(10.0));
        assert_eq!(closed.end_ts, 100.0);
        // A recorder that starts mid-print closes the run honestly flagged.
        let mut joined = PrintTracker::seed(200.0, &sample("printing", "third.gcode", 50.0));
        let closed = joined
            .observe(300.0, &sample("error", "third.gcode", 55.0))
            .expect("error closes the seeded run");
        assert_eq!(closed.result, "error");
        assert!(closed.evidence.contains("mid-run"), "{}", closed.evidence);
        // A recorder that starts (or restarts) AFTER a print finished records that print once,
        // flagged, and a repeated terminal state never records it again (the guard reads the
        // terminal state from BEFORE the sample that carries it).
        let mut late = PrintTracker::default();
        let closed = late
            .observe(500.0, &sample("complete", "cube.gcode", 720.0))
            .expect("a terminal state with no open session is recorded once");
        assert_eq!(closed.result, "complete");
        assert_eq!(closed.start_ts, closed.end_ts);
        assert!(closed.evidence.contains("joined after this print finished"));
        assert!(
            late.observe(510.0, &sample("complete", "cube.gcode", 720.0))
                .is_none(),
            "a repeated terminal must not record the same finished print again"
        );
    }

    /// The write path fills the table and reports the outcome for the collector's notice.
    #[test]
    fn write_records_print_sessions_and_reports_the_outcome() {
        let mut db = Store::open(":memory:", false).unwrap();
        let mut p = Parser::default();
        let printing = json!({"print_stats":{"state":"printing","filename":"cube.gcode","print_duration":60.0},"virtual_sdcard":{"progress":0.5}});
        let complete = json!({"print_stats":{"state":"complete","filename":"cube.gcode","print_duration":720.0},"virtual_sdcard":{"progress":1.0}});
        db.write(Some((20.0, &printing)), &[], &mut p).unwrap();
        let w = db.write(Some((80.0, &complete)), &[], &mut p).unwrap();
        assert_eq!(
            w.outcome.as_ref().map(|s| s.result.as_str()),
            Some("complete")
        );
        let rows = db.print_sessions(0.0, 100.0, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "cube.gcode");
        assert_eq!(rows[0].start_ts, 20.0);
        assert_eq!(rows[0].duration_s, Some(720.0));
    }

    /// A service restart mid-print continues the same session instead of opening a second.
    #[test]
    fn a_restart_mid_print_continues_the_same_session() {
        let path = std::env::temp_dir().join(format!("stella-wo13-{}.sqlite3", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap().to_string_lossy()
            )));
        }
        {
            let mut db = Store::open(path.to_str().unwrap(), false).unwrap();
            let mut p = Parser::default();
            let d = json!({"print_stats":{"state":"printing","filename":"cube.gcode","print_duration":100.0},"virtual_sdcard":{"progress":0.3}});
            db.write(Some((10.0, &d)), &[], &mut p).unwrap();
        }
        let mut db = Store::open(path.to_str().unwrap(), false).unwrap();
        let mut p = Parser::default();
        let d = json!({"print_stats":{"state":"complete","filename":"cube.gcode","print_duration":600.0},"virtual_sdcard":{"progress":1.0}});
        let w = db.write(Some((500.0, &d)), &[], &mut p).unwrap();
        assert!(w.outcome.is_some());
        let rows = db.print_sessions(0.0, 1000.0, 10).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "restart must continue, not duplicate: {rows:?}"
        );
        assert!(rows[0].evidence.contains("mid-run"), "{}", rows[0].evidence);
        // The start is the first sample of the run, not the last one before the restart.
        assert_eq!(rows[0].start_ts, 10.0);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap().to_string_lossy()
            )));
        }
        // The recorder saw the start (idle at 5, printing from 10), then restarted at 50. One
        // row, starting at 10, and the evidence says the start was recorded.
        let path = temp_db("wo14-seen");
        write_all(&path, &[(5.0, ps("standby", "", 0.0)), (10.0, ps("printing", "cube.gcode", 0.2)), (50.0, ps("printing", "cube.gcode", 40.2))]);
        // The restarted recorder keeps sampling the print up to its end (at 495); a first sample
        // after a restart that is already the completion is the gap case, tested apart.
        let rows = write_all(&path, &[(495.0, ps("printing", "cube.gcode", 485.2)), (500.0, ps("complete", "cube.gcode", 490.2))]);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].start_ts, 10.0);
        assert_eq!(rows[0].result, "complete");
        assert_eq!(
            rows[0].evidence,
            "recorder restarted mid-print; start recovered from its own recording"
        );
        remove_db(&path);
        // No quiet sample before the run: the run reaches the start of the recording. The start
        // is the first sample seen (10, not the 50 before the restart), flagged joined.
        let path = temp_db("wo14-joined");
        write_all(&path, &[(10.0, ps("printing", "cube.gcode", 300.0)), (50.0, ps("printing", "cube.gcode", 340.0))]);
        let rows = write_all(&path, &[(500.0, ps("complete", "cube.gcode", 790.0))]);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].start_ts, 10.0);
        assert!(rows[0].evidence.contains("joined this print mid-run"), "{}", rows[0].evidence);
        remove_db(&path);
    }

    /// print_stats in the shape Klipper reports it (all seven fields, as in the live DB). The
    /// caller gives state, file and total_duration; print_duration and filament_used are
    /// placeholders no assertion reads. The static objects every real sample carries make small
    /// changes store as deltas.
    fn ps(state: &str, file: &str, total: f64) -> Value {
        json!({"print_stats":{"filename":file,"total_duration":total,"print_duration":(total-0.2).max(0.0),
                "filament_used":0.0,"state":state,"message":"","info":{"total_layer":null,"current_layer":null}},
               "virtual_sdcard":{"file_path":null,"progress":0.0,"is_active":(state=="printing"),"file_position":0,"file_size":0},
               "toolhead":{"homed_axes":"xyz","axis_minimum":[0.0,0.0,-5.0,0.0],"axis_maximum":[350.0,350.0,310.0,0.0],
                "max_velocity":300.0,"max_accel":3000.0,"square_corner_velocity":5.0,"extruder":"extruder"},
               "heater_bed":{"temperature":24.5,"target":0.0,"power":0.0}})
    }
    /// A fresh on-disk recording path; a restart is a reopen of the same file.
    fn temp_db(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("stella-{tag}-{}.sqlite3", std::process::id()));
        remove_db(&path);
        path
    }
    fn remove_db(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap().to_string_lossy()
            )));
        }
    }
    /// One recorder run: open the file (seeding from what is there), write the samples, close.
    /// Returns every session row recorded so far.
    fn write_all(path: &std::path::Path, samples: &[(f64, Value)]) -> Vec<PrintSession> {
        let mut db = Store::open(path.to_str().unwrap(), false).unwrap();
        let mut p = Parser::default();
        for (ts, d) in samples {
            db.write(Some((*ts, d)), &[], &mut p).unwrap();
        }
        db.print_sessions(0.0, f64::MAX, 50).unwrap()
    }

    /// A cancel and a reprint of the same file. The real ones (2026-09-21 11:53:19 and 11:54:37)
    /// were both stored as deltas; here they fall inside one keyframe interval, where a
    /// keyframe-only walk cannot see the cancel at all and hands the reprint the first print's
    /// start (and its primary key). The replay must find the reprint's own start.
    #[test]
    fn a_restart_finds_the_reprint_not_the_cancelled_print_before_it() {
        let path = temp_db("wo14-reprint");
        let f = "Voron_Design_Cube_v7_0.2mm_PETG_Generic Klipper Printer_37m22s.gcode";
        write_all(&path, &[
            (100.0, ps("printing", f, 0.1)),
            (110.0, ps("printing", f, 10.1)),
            (120.0, ps("cancelled", f, 20.1)),
            (130.0, ps("printing", f, 0.1)),
            (140.0, ps("printing", f, 10.1)),
        ]);
        {
            let db = Store::open(path.to_str().unwrap(), true).unwrap();
            let kinds: Vec<String> = db
                .conn
                .prepare("SELECT kind FROM samples WHERE ts IN (120.0,130.0) ORDER BY ts")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(kinds, ["D", "D"], "the test must exercise the delta replay");
        }
        let rows = write_all(&path, &[(200.0, ps("complete", f, 70.1))]);
        assert_eq!(rows.len(), 2, "{rows:?}");
        let done = rows.iter().find(|r| r.result == "complete").unwrap();
        assert_eq!(done.start_ts, 130.0);
        assert!(done.evidence.contains("start recovered"), "{}", done.evidence);
        let cancelled = rows.iter().find(|r| r.result == "cancelled").unwrap();
        assert_eq!(cancelled.start_ts, 100.0);
        remove_db(&path);
    }

    /// On the recording's own numbers. 2026-09-17: the Voron cube started at 1789641574.518 (idle
    /// 0.236 s before) and the recorder restarted at 08:16:27 (last sample 1789643787.979, next
    /// 1789643790.337, Klipper total_duration 2215.899 there). 2026-09-21: the recorder was down
    /// for 5518 s and came back at 1790032005.993 with the next cube already 162.87 s in. A quiet
    /// sample from before that hole must not pass for a witnessed start, whether or not the
    /// recorder restarts again during the print.
    #[test]
    fn real_restart_keeps_the_start_and_a_late_join_stays_joined() {
        let cube = "Voron_Design_Cube_v7_0.2mm_PLA_Generic Klipper Printer_31m39s.gcode";
        let path = temp_db("wo14-0917");
        write_all(&path, &[
            (1789641574.282, ps("standby", "", 0.0)),
            (1789641574.518, ps("printing", cube, 0.2233640919998834)),
            (1789643787.979, ps("printing", cube, 2213.64350268)),
        ]);
        // Plus the recording's own last printing sample before the completion (0.51 s
        // earlier), so the fixture has no gap the recording did not have.
        let rows = write_all(&path, &[
            (1789643790.337, ps("printing", cube, 2215.8989497720004)),
            (1789644527.958, ps("printing", cube, 2953.686487999)),
            (1789644528.471, ps("complete", cube, 2953.86146361)),
        ]);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].start_ts, 1789641574.518);
        assert!(rows[0].evidence.contains("start recovered"), "{}", rows[0].evidence);
        assert!(!rows[0].evidence.contains("gap"), "{}", rows[0].evidence);
        // Klipper's own clock agrees to within a sample: first sample after the restart minus its
        // total_duration is where the print began.
        assert!((rows[0].start_ts - (1789643790.337 - 2215.8989497720004)).abs() < 0.5);
        remove_db(&path);

        let flow = "flowrate_0_0.2mm_PETG_Generic Klipper Printer_34m34s.gcode";
        let cube = "Voron_Design_Cube_v7_0.2mm_PETG_Generic Klipper Printer_32m3s.gcode";
        for restart_mid_print in [false, true] {
            let path = temp_db(&format!("wo14-0921-{restart_mid_print}"));
            write_all(&path, &[(1790026487.540, ps("complete", flow, 2741.209989996))]);
            let first = [
                (1790032005.993, ps("printing", cube, 162.87465042199983)),
                (1790033000.0, ps("printing", cube, 1156.9)),
            ];
            // The recording's own last printing sample, 0.26 s before the completion.
            let last = [
                (1790034532.319, ps("printing", cube, 2688.2021875459995)),
                (1790034532.574, ps("complete", cube, 2688.250605065)),
            ];
            let rows = if restart_mid_print {
                write_all(&path, &first);
                write_all(&path, &last)
            } else {
                write_all(&path, &[first.as_slice(), last.as_slice()].concat())
            };
            let row = rows.iter().find(|r| r.filename == cube).expect("the cube row");
            assert_eq!(row.start_ts, 1790032005.993);
            assert!(
                row.evidence.contains("joined this print mid-run"),
                "restart={restart_mid_print}: {}",
                row.evidence
            );
            assert!(!row.evidence.contains("gap"), "{}", row.evidence);
            remove_db(&path);
        }
    }

    /// Narrowing the replay to a field never hides a change to it. A delta that removes a whole
    /// object names the object, not its fields (the recording has such removals when the
    /// transport switches, e.g. [["system_stats"],["webhooks"]]), so it is decoded anyway and the
    /// rebuilt sample drops the field instead of keeping a stale value.
    #[test]
    fn a_narrowed_replay_still_decodes_the_removal_of_a_whole_object() {
        let mut db = Store::open(":memory:", false).unwrap();
        let mut p = Parser::default();
        let bulk = json!({"homed_axes":"xyz","axis_minimum":[0.0,0.0,-5.0,0.0],
            "axis_maximum":[350.0,350.0,310.0,0.0],"max_velocity":300.0,"max_accel":3000.0,
            "square_corner_velocity":5.0,"minimum_cruise_ratio":0.5,"extruder":"extruder"});
        let with = json!({"toolhead":bulk,"tmc2209 stepper_x":{"run_current":0.8,"drv_status":{"cs_actual":12}}});
        let without = json!({"toolhead":bulk});
        db.write(Some((100.0, &with)), &[], &mut p).unwrap();
        db.write(Some((110.0, &without)), &[], &mut p).unwrap();
        let kind: String = db
            .conn
            .query_row("SELECT kind FROM samples WHERE ts=110.0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            kind, "D",
            "the removal must be a delta for this test to mean anything"
        );
        let mut seen = Vec::new();
        let cs_actual = "/tmc2209 stepper_x/drv_status/cs_actual";
        db.replay(0.0, 200.0, &[cs_actual], |ts, sample| {
            seen.push((ts, sample.pointer(cs_actual).and_then(Value::as_f64)));
            true
        })
        .unwrap();
        assert_eq!(seen, [(100.0, Some(12.0)), (110.0, None)]);
    }

    fn stats(state: &str, file: &str, dur: f64) -> Value {
        json!({"print_stats":{"state":state,"filename":file,"print_duration":dur,"filament_used":1.25},
               "virtual_sdcard":{"progress":0.5}})
    }

    /// A direct switch from one file to another (the recorder missed the first print's end)
    /// closes the first as interrupted at its own last sample and opens the second on the same
    /// sample, not the next one. The first print's last sample witnesses the second start, as
    /// another file does for run_start after a restart; after a hole it is too old to.
    #[test]
    fn a_file_switch_opens_the_new_print_on_its_first_sample() {
        let mut t = PrintTracker::default();
        assert!(t
            .observe(10.0, &stats("printing", "a.gcode", 5.0))
            .is_none());
        let a = t
            .observe(20.0, &stats("printing", "b.gcode", 1.0))
            .expect("the switch closes A");
        assert_eq!(a.filename, "a.gcode");
        assert_eq!(a.result, "interrupted");
        assert_eq!(a.end_ts, 10.0);
        let b = t
            .observe(90.0, &stats("complete", "b.gcode", 70.0))
            .expect("complete closes B");
        assert_eq!(b.filename, "b.gcode");
        assert_eq!(b.start_ts, 20.0, "B opens on its own first sample");
        assert!(b.evidence.starts_with("start recorded"), "{}", b.evidence);
        let mut t = PrintTracker::default();
        t.observe(10.0, &stats("printing", "a.gcode", 5.0));
        let a = t
            .observe(100.0, &stats("printing", "b.gcode", 400.0))
            .expect("the switch closes A");
        assert_eq!(a.end_ts, 10.0);
        let b = t
            .observe(110.0, &stats("complete", "b.gcode", 410.0))
            .expect("complete closes B");
        assert_eq!(b.start_ts, 100.0);
        assert!(
            b.evidence.starts_with("recorder joined this print mid-run"),
            "{}",
            b.evidence
        );
    }

    /// A completion first seen more than START_SEEN_S after the print's last sample happened
    /// somewhere in that gap. The row still ends at the terminal sample (the only recorded
    /// proof), keeps Klipper's own print_duration, and says so; with no gap (30 s is still
    /// none) the evidence is what it was.
    #[test]
    fn a_completion_seen_after_a_gap_says_it_finished_inside_the_gap() {
        let mut t = PrintTracker::default();
        t.observe(10.0, &stats("standby", "", 0.0));
        t.observe(20.0, &stats("printing", "cube.gcode", 1.0));
        t.observe(50.0, &stats("printing", "cube.gcode", 30.0));
        let done = t
            .observe(80.0, &stats("complete", "cube.gcode", 59.0))
            .expect("complete closes the session");
        assert_eq!(done.evidence, "start and completion both recorded");
        assert_eq!((done.end_ts, done.duration_s), (80.0, Some(59.0)));
        let mut t = PrintTracker::default();
        t.observe(10.0, &stats("standby", "", 0.0));
        t.observe(20.0, &stats("printing", "cube.gcode", 1.0));
        let done = t
            .observe(200.0, &stats("complete", "cube.gcode", 95.0))
            .expect("complete closes the session");
        assert_eq!(
            done.evidence,
            "start recorded; completion seen after a gap in the recording: it finished in the 3 min 0 s before the recorded end"
        );
        assert_eq!(
            done.end_ts, 200.0,
            "the terminal sample, never an invented end"
        );
        assert_eq!(done.duration_s, Some(95.0), "Klipper's own print_duration");
        let mut t = PrintTracker::seed(300.0, &stats("printing", "cube.gcode", 30.0));
        let done = t
            .observe(400.0, &stats("error", "cube.gcode", 120.0))
            .expect("error closes the seeded session");
        assert_eq!(
            done.evidence,
            "recorder joined this print mid-run; start time is the first sample seen; completion seen after a gap in the recording: it finished in the 1 min 40 s before the recorded end"
        );
    }

    /// On the recording's own numbers. 2026-09-17 the recorder's last sample inside a Cube
    /// print was 11:18:31 (1789654711.750); it came back at 11:53:50 and its first sample
    /// already said complete (1789656831.397). Klipper's own total_duration there (1504.19 s)
    /// puts the real end about 770 s before that sample, inside the 2119.6 s gap the row names.
    #[test]
    fn the_real_09_17_completion_after_a_restart_gap_names_the_gap() {
        let cube = "Cube_0.2mm_PLA_Generic Klipper Printer_6m56s.gcode";
        let path = temp_db("wo15-0917-gap");
        write_all(
            &path,
            &[
                (1789654557.366, ps("standby", "", 0.0)),
                (1789654557.965, ps("printing", cube, 0.4849887769996712)),
                (1789654711.750, ps("printing", cube, 154.14840440800072)),
            ],
        );
        let rows = write_all(
            &path,
            &[(1789656831.397, ps("complete", cube, 1504.18881677))],
        );
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert_eq!(row.start_ts, 1789654557.965);
        assert_eq!(row.end_ts, 1789656831.397);
        assert_eq!(
            row.evidence,
            "recorder restarted mid-print; start recovered from its own recording; completion seen after a gap in the recording: it finished in the 35 min 20 s before the recorded end"
        );
        let klipper_end = row.start_ts - 0.4849887769996712 + 1504.18881677;
        assert!(
            klipper_end > 1789654711.750 && klipper_end < row.end_ts,
            "{klipper_end}"
        );
        remove_db(&path);
    }
}
