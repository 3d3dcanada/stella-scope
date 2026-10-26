//! The farm's job queue (farm board WO-08): which sliced files wait, which printer each may go to, and
//! the per-printer plate state. One SQLite file next to the first printer's recording, so it survives a
//! restart. The rules that decide where a file may go are plain functions below, tested on their own;
//! the flow that actually uploads and starts lives in `queue_flow.rs`.
use crate::{gcode_meta::Meta, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct Queue {
    conn: Mutex<Connection>,
    dir: PathBuf,
}

/// What a printer is, for the purpose of deciding whether a file may go to it.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// What the slicer calls this printer (`COREONEL`, `Anycubic Kobra X`).
    pub slicer_model: Option<String>,
    /// The model text of its entry (`Prusa CORE One L`); also tried against the file's printer.
    pub model_name: Option<String>,
    pub bed_mm: Option<[f64; 2]>,
    pub nozzle_mm: Option<f64>,
    pub material: Option<String>,
}

/// What a job needs: the file's own header, overridden by anything the person set by hand.
#[derive(Clone, Debug, Default)]
pub struct Needs {
    pub printer_model: Option<String>,
    pub nozzle_mm: Option<f64>,
    pub filament: Vec<String>,
    pub bed_mm: Option<[f64; 2]>,
    pub extents_mm: Option<[f64; 3]>,
}

impl Needs {
    pub fn from(meta: &Meta, overrides: &Value) -> Self {
        let pair = |k: &str| -> Option<[f64; 2]> {
            let a = overrides.get(k)?.as_array()?;
            Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
        };
        Self {
            printer_model: overrides["printer_model"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| meta.printer_model.clone()),
            nozzle_mm: overrides["nozzle_mm"].as_f64().or(meta.nozzle_mm),
            filament: overrides["filament"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| meta.filament.clone()),
            bed_mm: pair("bed_mm").or(meta.bed_mm),
            extents_mm: meta.extents_mm,
        }
    }
}

/// The answer for one job and one printer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fit {
    /// Nothing known contradicts sending it here.
    pub ok: bool,
    /// Everything that decides it is known on both sides, so a job may be given to this printer
    /// without anyone having named it.
    pub verified: bool,
    pub reasons: Vec<String>,
    pub notes: Vec<String>,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn same_model(file: &str, printer: &str) -> bool {
    let (a, b) = (norm(file), norm(printer));
    a.len() >= 4 && b.len() >= 4 && (a.contains(&b) || b.contains(&a))
}

/// Can this job go to this printer? `force_model` is the person saying "send it to a different model
/// anyway"; it excuses the model only. A nozzle, a material or a bed that does not fit is never excused.
pub fn fit(n: &Needs, force_model: bool, p: &Profile) -> Fit {
    let mut f = Fit {
        ok: true,
        verified: true,
        ..Fit::default()
    };
    let block = |f: &mut Fit, why: String| {
        f.ok = false;
        f.reasons.push(why);
    };
    // The printer model: G-code is made for one machine.
    let candidates: Vec<&str> = [p.slicer_model.as_deref(), p.model_name.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect();
    match (&n.printer_model, candidates.is_empty()) {
        (Some(file), false) => {
            if !candidates.iter().any(|c| same_model(file, c)) {
                if force_model {
                    f.notes.push(format!(
                        "Sliced for {file}, sent to {} because you said so.",
                        p.name
                    ));
                    f.verified = false;
                } else {
                    block(
                        &mut f,
                        format!("Made for {file}; {} is a different machine.", p.name),
                    );
                }
            }
        }
        (Some(_), true) => {
            f.verified = false;
            f.notes.push(format!("Set what {} is called in your slicer (in its settings) so files can be matched to it.", p.name));
        }
        (None, _) => {
            f.verified = false;
            f.notes
                .push("This file does not say which printer it was sliced for.".into());
        }
    }
    // The nozzle.
    match (n.nozzle_mm, p.nozzle_mm) {
        (Some(want), Some(have)) if (want - have).abs() > 0.011 => block(
            &mut f,
            format!("Sliced for a {want} mm nozzle; {} has {have} mm.", p.name),
        ),
        (Some(_), Some(_)) => {}
        (Some(want), None) => {
            f.verified = false;
            f.notes.push(format!(
                "Sliced for a {want} mm nozzle; tell the app which nozzle {} has.",
                p.name
            ));
        }
        (None, _) => f.verified = false,
    }
    // The material, when the printer's is known.
    if let Some(have) = p.material.as_deref().filter(|m| !m.trim().is_empty()) {
        let mut kinds: Vec<String> = n
            .filament
            .iter()
            .map(|k| norm(k))
            .filter(|k| !k.is_empty())
            .collect();
        kinds.sort();
        kinds.dedup();
        if !kinds.is_empty() && kinds.iter().any(|k| *k != norm(have)) {
            block(
                &mut f,
                format!(
                    "The file is {}; {} has {have} loaded.",
                    n.filament.join(" + "),
                    p.name
                ),
            );
        }
    }
    // The bed and the print.
    if let Some([bw, bd]) = p.bed_mm {
        if let Some([w, d]) = n.bed_mm {
            if w > bw + 1.0 || d > bd + 1.0 {
                block(
                    &mut f,
                    format!(
                        "Sliced for a {w:.0} x {d:.0} mm bed; {} has {bw:.0} x {bd:.0} mm.",
                        p.name
                    ),
                );
            }
        }
        if let Some([x, y, _]) = n.extents_mm {
            if x > bw + 1.0 || y > bd + 1.0 {
                block(
                    &mut f,
                    format!(
                        "The print is {x:.0} x {y:.0} mm; the bed of {} is {bw:.0} x {bd:.0} mm.",
                        p.name
                    ),
                );
            }
        }
    }
    if !f.ok {
        f.verified = false;
    }
    f
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A file name that is safe on every printer: letters, digits, dot, dash, underscore.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while out.starts_with('.') {
        out.remove(0);
    }
    out.chars().take(120).collect()
}

pub fn file_ok(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    !name.is_empty()
        && (l.ends_with(".gcode")
            || l.ends_with(".bgcode")
            || l.ends_with(".gco")
            || l.ends_with(".gcode.3mf"))
}

impl Queue {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir.join("files"))
            .map_err(|e| format!("cannot create the queue folder {}: {e}", dir.display()))?;
        std::fs::create_dir_all(dir.join("incoming")).map_err(|e| e.to_string())?;
        // Half-sent uploads from a previous run are not jobs.
        if let Ok(rd) = std::fs::read_dir(dir.join("incoming")) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let conn = Connection::open(dir.join("queue.sqlite3"))
            .map_err(|e| format!("cannot open the queue: {e}"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS jobs(
               id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, file_name TEXT NOT NULL, source TEXT NOT NULL,
               stored TEXT, copies INTEGER NOT NULL DEFAULT 1, done INTEGER NOT NULL DEFAULT 0, priority INTEGER NOT NULL DEFAULT 0,
               position REAL NOT NULL, target TEXT, force_model INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT 'queued',
               meta TEXT NOT NULL, overrides TEXT NOT NULL DEFAULT '{}', printer_id TEXT, note TEXT,
               created_at REAL NOT NULL, started_at REAL, finished_at REAL);
             CREATE TABLE IF NOT EXISTS flow(
               printer_id TEXT PRIMARY KEY, awaiting_clear INTEGER NOT NULL DEFAULT 0, released INTEGER NOT NULL DEFAULT 0,
               auto_start INTEGER NOT NULL DEFAULT 0, updated_at REAL);
             CREATE TABLE IF NOT EXISTS plate(
               printer_id TEXT PRIMARY KEY, px BLOB, jpeg BLOB, region TEXT, sensitivity REAL NOT NULL DEFAULT 1.0, captured_at REAL);
             CREATE TABLE IF NOT EXISTS plate_checks(
               id INTEGER PRIMARY KEY AUTOINCREMENT, printer_id TEXT NOT NULL, at REAL NOT NULL, outcome TEXT NOT NULL,
               score REAL, detail TEXT, frame BLOB);",
        )
        .map_err(|e| format!("cannot set up the queue: {e}"))?;
        // Finish-2 F02: where the head and bed were when the empty plate was captured. Added to older queues; an error here
        // only means the column is already there.
        let _ = conn.execute("ALTER TABLE plate ADD COLUMN pose TEXT", []);
        Ok(Self {
            conn: Mutex::new(conn),
            dir: dir.to_path_buf(),
        })
    }

    pub fn incoming(&self) -> PathBuf {
        self.dir.join("incoming")
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- jobs ---------------------------------------------------------------------------------

    fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
        Ok(Job {
            id: r.get("id")?,
            title: r.get("title")?,
            file_name: r.get("file_name")?,
            source: serde_json::from_str(&r.get::<_, String>("source")?).unwrap_or(Value::Null),
            stored: r.get("stored")?,
            copies: r.get("copies")?,
            done: r.get("done")?,
            priority: r.get("priority")?,
            target: r.get("target")?,
            force_model: r.get::<_, i64>("force_model")? != 0,
            state: r.get("state")?,
            meta: serde_json::from_str(&r.get::<_, String>("meta")?).unwrap_or_default(),
            overrides: serde_json::from_str(&r.get::<_, String>("overrides")?).unwrap_or(json!({})),
            printer_id: r.get("printer_id")?,
            note: r.get("note")?,
            created_at: r.get("created_at")?,
            started_at: r.get("started_at")?,
            finished_at: r.get("finished_at")?,
        })
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let db = self.db();
        let mut st = db
            .prepare("SELECT * FROM jobs ORDER BY priority DESC, position ASC, id ASC")
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([], Self::row).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    pub fn job(&self, id: i64) -> Result<Option<Job>> {
        self.db()
            .query_row("SELECT * FROM jobs WHERE id=?1", params![id], Self::row)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Take a received file (already spooled under `incoming/`) into the queue.
    pub fn add_upload(&self, spooled: &Path, original_name: &str, copies: u32) -> Result<Job> {
        let name = safe_name(original_name);
        if !file_ok(&name) {
            let _ = std::fs::remove_file(spooled);
            return Err("only .gcode, .bgcode and .gcode.3mf files go in the queue".into());
        }
        let meta = if name.to_ascii_lowercase().ends_with(".3mf") {
            Meta::default() // a zip: its header is not read; the person names the printer
        } else {
            match crate::gcode_meta::read(spooled, true) {
                Ok(m) => m,
                Err(e) => {
                    let _ = std::fs::remove_file(spooled);
                    return Err(e);
                }
            }
        };
        let id = self.insert(&name, &json!({"kind":"upload"}), None, &meta, copies)?;
        let stored = format!("files/{id}-{name}");
        std::fs::rename(spooled, self.dir.join(&stored))
            .or_else(|_| {
                std::fs::copy(spooled, self.dir.join(&stored))
                    .map(|_| ())
                    .and_then(|_| std::fs::remove_file(spooled))
            })
            .map_err(|e| format!("cannot keep the file: {e}"))?;
        self.db()
            .execute("UPDATE jobs SET stored=?1 WHERE id=?2", params![stored, id])
            .map_err(|e| e.to_string())?;
        self.job(id)?.ok_or_else(|| "the job vanished".into())
    }

    /// A file that is already on a printer: it can only run there, and nothing is uploaded.
    pub fn add_on_printer(
        &self,
        printer: &str,
        file: &str,
        meta: &Meta,
        copies: u32,
    ) -> Result<Job> {
        if file.is_empty() || file.contains("..") || file.len() > 200 {
            return Err("that is not a file name".into());
        }
        let id = self.insert(
            file,
            &json!({"kind":"printer","printer":printer}),
            Some(printer),
            meta,
            copies,
        )?;
        self.job(id)?.ok_or_else(|| "the job vanished".into())
    }

    fn insert(
        &self,
        file: &str,
        source: &Value,
        target: Option<&str>,
        meta: &Meta,
        copies: u32,
    ) -> Result<i64> {
        let db = self.db();
        let pos: f64 = db
            .query_row("SELECT COALESCE(MAX(position),0)+1 FROM jobs", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        let title = file
            .trim_end_matches(".3mf")
            .trim_end_matches(".gcode")
            .trim_end_matches(".bgcode")
            .trim_end_matches(".gco")
            .replace(['_'], " ");
        db.execute(
            "INSERT INTO jobs(title,file_name,source,copies,position,target,meta,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![title, file, source.to_string(), copies.clamp(1, 500), pos, target, serde_json::to_string(meta).unwrap_or_default(), now()],
        )
        .map_err(|e| e.to_string())?;
        Ok(db.last_insert_rowid())
    }

    /// Change what a person may change about a job.
    pub fn patch(&self, id: i64, p: &Value) -> Result<Job> {
        let job = self.job(id)?.ok_or("no such job")?;
        for k in p.as_object().ok_or("send a JSON object")?.keys() {
            if ![
                "title",
                "copies",
                "priority",
                "target",
                "force_model",
                "overrides",
                "state",
            ]
            .contains(&k.as_str())
            {
                return Err(format!("{k} cannot be changed"));
            }
        }
        let db = self.db();
        if let Some(v) = p.get("title").and_then(Value::as_str) {
            db.execute(
                "UPDATE jobs SET title=?1 WHERE id=?2",
                params![v.trim().chars().take(80).collect::<String>(), id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(v) = p.get("copies").and_then(Value::as_u64) {
            db.execute(
                "UPDATE jobs SET copies=?1 WHERE id=?2",
                params![v.clamp(1, 500) as i64, id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(v) = p.get("priority").and_then(Value::as_i64) {
            db.execute(
                "UPDATE jobs SET priority=?1 WHERE id=?2",
                params![v.clamp(-10, 10), id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(t) = p.get("target") {
            if job.source["kind"] == "printer" {
                return Err("a file that is on a printer can only run on that printer".into());
            }
            let t = t.as_str().filter(|s| !s.is_empty());
            db.execute("UPDATE jobs SET target=?1 WHERE id=?2", params![t, id])
                .map_err(|e| e.to_string())?;
        }
        if let Some(v) = p.get("force_model").and_then(Value::as_bool) {
            db.execute(
                "UPDATE jobs SET force_model=?1 WHERE id=?2",
                params![i64::from(v), id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(o) = p.get("overrides") {
            db.execute(
                "UPDATE jobs SET overrides=?1 WHERE id=?2",
                params![o.to_string(), id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(s) = p.get("state").and_then(Value::as_str) {
            // Only back to the queue (after a stopped copy) or cancelled.
            if !["queued", "cancelled"].contains(&s) {
                return Err("a job can be put back in the queue or cancelled".into());
            }
            db.execute(
                "UPDATE jobs SET state=?1, note=NULL WHERE id=?2",
                params![s, id],
            )
            .map_err(|e| e.to_string())?;
        }
        drop(db);
        self.job(id)?.ok_or_else(|| "the job vanished".into())
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let job = self.job(id)?.ok_or("no such job")?;
        if job.state == "running" {
            return Err("this job is printing now; cancel the print first".into());
        }
        self.db()
            .execute("DELETE FROM jobs WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        if let Some(s) = job.stored {
            let _ = std::fs::remove_file(self.dir.join(s));
        }
        Ok(())
    }

    /// Put the jobs in this order (the rest keep theirs, after these).
    pub fn reorder(&self, ids: &[i64]) -> Result<()> {
        let db = self.db();
        for (i, id) in ids.iter().enumerate() {
            db.execute(
                "UPDATE jobs SET position=?1 WHERE id=?2",
                params![(i + 1) as f64, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn stored_path(&self, job: &Job) -> Option<PathBuf> {
        job.stored.as_ref().map(|s| self.dir.join(s))
    }

    pub fn set_running(&self, id: i64, printer: &str) -> Result<()> {
        self.db()
            .execute("UPDATE jobs SET state='running', printer_id=?1, started_at=?2, note=NULL WHERE id=?3", params![printer, now(), id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A print that was this job ended: a finished copy counts, anything else stops the job and says why.
    pub fn finish(&self, id: i64, completed: bool, why: &str) -> Result<()> {
        let job = self.job(id)?.ok_or("no such job")?;
        let db = self.db();
        if completed {
            let done = job.done + 1;
            let state = if done >= job.copies { "done" } else { "queued" };
            db.execute(
                "UPDATE jobs SET done=?1, state=?2, finished_at=?3, note=NULL WHERE id=?4",
                params![done, state, now(), id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            db.execute(
                "UPDATE jobs SET state='stopped', finished_at=?1, note=?2 WHERE id=?3",
                params![now(), why, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn running_on(&self, printer: &str) -> Result<Option<Job>> {
        self.db().query_row("SELECT * FROM jobs WHERE state='running' AND printer_id=?1 ORDER BY id DESC LIMIT 1", params![printer], Self::row).optional().map_err(|e| e.to_string())
    }

    // ---- per-printer flow -----------------------------------------------------------------------

    pub fn flow(&self, printer: &str) -> Flow {
        self.db()
            .query_row(
                "SELECT awaiting_clear, released, auto_start FROM flow WHERE printer_id=?1",
                params![printer],
                |r| {
                    Ok(Flow {
                        awaiting_clear: r.get::<_, i64>(0)? != 0,
                        released: r.get::<_, i64>(1)? != 0,
                        auto_start: r.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn set_flow(&self, printer: &str, f: Flow) -> Result<()> {
        self.db()
            .execute(
                "INSERT INTO flow(printer_id,awaiting_clear,released,auto_start,updated_at) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(printer_id) DO UPDATE SET awaiting_clear=?2, released=?3, auto_start=?4, updated_at=?5",
                params![printer, i64::from(f.awaiting_clear), i64::from(f.released), i64::from(f.auto_start), now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- the empty-plate picture ------------------------------------------------------------------

    pub fn plate(&self, printer: &str) -> Option<PlateRef> {
        self.db()
            .query_row(
                "SELECT px, jpeg, region, sensitivity, captured_at, pose FROM plate WHERE printer_id=?1",
                params![printer],
                |r| {
                    Ok(PlateRef {
                        px: r.get(0)?,
                        jpeg: r.get(1)?,
                        region: serde_json::from_str(&r.get::<_, String>(2).unwrap_or_default())
                            .unwrap_or(Value::Null),
                        sensitivity: r.get(3)?,
                        captured_at: r.get(4)?,
                        pose: r
                            .get::<_, Option<String>>(5)?
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(Value::Null),
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Save a captured empty plate. The region starts as the whole frame until the person marks it.
    pub fn save_reference(&self, printer: &str, px: &[u8], jpeg: &[u8], pose: &Value) -> Result<()> {
        self.db()
            .execute(
                "INSERT INTO plate(printer_id,px,jpeg,region,sensitivity,captured_at,pose) VALUES(?1,?2,?3,'null',1.0,?4,?5)
                 ON CONFLICT(printer_id) DO UPDATE SET px=?2, jpeg=?3, region='null', captured_at=?4, pose=?5",
                params![printer, px, jpeg, now(), pose.to_string()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn save_region(
        &self,
        printer: &str,
        region: &Value,
        sensitivity: Option<f64>,
    ) -> Result<()> {
        let n = self.db()
            .execute(
                "UPDATE plate SET region=?1, sensitivity=COALESCE(?2, sensitivity) WHERE printer_id=?3",
                params![region.to_string(), sensitivity.map(|s| s.clamp(0.5, 2.0)), printer],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("capture the empty plate first".into());
        }
        Ok(())
    }

    pub fn forget_plate(&self, printer: &str) {
        let _ = self
            .db()
            .execute("DELETE FROM plate WHERE printer_id=?1", params![printer]);
    }

    pub fn add_check(
        &self,
        printer: &str,
        outcome: &str,
        score: Option<f64>,
        detail: &Value,
        frame: Option<&[u8]>,
    ) -> Result<i64> {
        let db = self.db();
        db.execute(
            "INSERT INTO plate_checks(printer_id,at,outcome,score,detail,frame) VALUES(?1,?2,?3,?4,?5,?6)",
            params![printer, now(), outcome, score, detail.to_string(), frame],
        )
        .map_err(|e| e.to_string())?;
        // Keep the last 40 per printer: a decision and its frame, not an archive.
        let _ = db.execute(
            "DELETE FROM plate_checks WHERE printer_id=?1 AND id NOT IN (SELECT id FROM plate_checks WHERE printer_id=?1 ORDER BY id DESC LIMIT 40)",
            params![printer],
        );
        Ok(db.last_insert_rowid())
    }

    pub fn check_frame(&self, id: i64, printer: &str) -> Option<Vec<u8>> {
        self.db()
            .query_row(
                "SELECT frame FROM plate_checks WHERE id=?1 AND printer_id=?2",
                params![id, printer],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn checks(&self, printer: &str, n: u32) -> Vec<Value> {
        let db = self.db();
        let Ok(mut st) = db.prepare("SELECT id, at, outcome, score, detail FROM plate_checks WHERE printer_id=?1 ORDER BY id DESC LIMIT ?2") else { return Vec::new() };
        st.query_map(params![printer, n], |r| {
            Ok(json!({"id":r.get::<_, i64>(0)?,"at":r.get::<_, f64>(1)?,"outcome":r.get::<_, String>(2)?,"score":r.get::<_, Option<f64>>(3)?,
                "detail":serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(Value::Null)}))
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub awaiting_clear: bool,
    pub released: bool,
    pub auto_start: bool,
}

pub struct PlateRef {
    pub px: Vec<u8>,
    pub jpeg: Vec<u8>,
    pub region: Value,
    pub sensitivity: f64,
    pub captured_at: f64,
    /// [x, y, z] the printer reported at capture (a value may be null), or null when not known.
    pub pose: Value,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: i64,
    pub title: String,
    pub file_name: String,
    pub source: Value,
    pub stored: Option<String>,
    pub copies: u32,
    pub done: u32,
    pub priority: i32,
    pub target: Option<String>,
    pub force_model: bool,
    pub state: String,
    pub meta: Meta,
    pub overrides: Value,
    pub printer_id: Option<String>,
    pub note: Option<String>,
    pub created_at: f64,
    pub started_at: Option<f64>,
    pub finished_at: Option<f64>,
}

impl Job {
    pub fn needs(&self) -> Needs {
        Needs::from(&self.meta, &self.overrides)
    }
    /// Copies still to print, counting the one that is printing.
    pub fn waiting(&self) -> bool {
        self.state == "queued" && self.done < self.copies
    }
}

/// The next job for a printer: highest priority first, then queue order; a job named for another
/// printer is not a candidate; one that nobody named goes only where every fact is known and fits.
pub fn next_for<'a>(jobs: &'a [Job], p: &Profile) -> Option<&'a Job> {
    jobs.iter().filter(|j| j.waiting()).find(|j| {
        let f = fit(&j.needs(), j.force_model, p);
        match &j.target {
            Some(t) => t == &p.id && f.ok,
            None => f.ok && f.verified,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_one_l() -> Profile {
        Profile {
            id: "core".into(),
            name: "Core One L".into(),
            slicer_model: Some("COREONEL".into()),
            model_name: Some("Prusa CORE One L".into()),
            bed_mm: Some([300.0, 300.0]),
            nozzle_mm: Some(0.4),
            material: None,
        }
    }
    fn kobra() -> Profile {
        Profile {
            id: "kobra".into(),
            name: "Kobra X".into(),
            slicer_model: Some("Anycubic Kobra X".into()),
            model_name: None,
            bed_mm: Some([260.0, 260.0]),
            nozzle_mm: Some(0.25),
            material: Some("PETG".into()),
        }
    }
    fn prusa_job() -> Needs {
        Needs {
            printer_model: Some("COREONEL".into()),
            nozzle_mm: Some(0.4),
            filament: vec!["PETG".into()],
            bed_mm: Some([300.0, 300.0]),
            extents_mm: Some([120.0, 80.0, 40.0]),
        }
    }

    #[test]
    fn a_file_goes_to_the_machine_it_was_sliced_for_and_nowhere_else() {
        let f = fit(&prusa_job(), false, &core_one_l());
        assert!(f.ok && f.verified, "{f:?}");
        let f = fit(&prusa_job(), false, &kobra());
        assert!(!f.ok, "{f:?}");
        assert!(
            f.reasons
                .iter()
                .any(|r| r.contains("COREONEL") && r.contains("different machine")),
            "{:?}",
            f.reasons
        );
    }

    #[test]
    fn forcing_the_model_excuses_the_model_only() {
        // Same bed and nozzle: the model is the only objection, and force_model lifts it.
        let mut k = kobra();
        k.nozzle_mm = Some(0.4);
        k.bed_mm = Some([300.0, 300.0]);
        k.material = None;
        assert!(!fit(&prusa_job(), false, &k).ok);
        let f = fit(&prusa_job(), true, &k);
        assert!(
            f.ok && !f.verified,
            "forced is allowed but never chosen on its own: {f:?}"
        );
        // A wrong nozzle is not excused by force.
        let f = fit(&prusa_job(), true, &kobra());
        assert!(
            !f.ok && f.reasons.iter().any(|r| r.contains("0.25 mm")),
            "{f:?}"
        );
    }

    #[test]
    fn nozzle_material_bed_and_size_each_say_why() {
        let mut p = core_one_l();
        p.nozzle_mm = Some(0.6);
        assert!(fit(&prusa_job(), false, &p)
            .reasons
            .iter()
            .any(|r| r.contains("0.4 mm") && r.contains("0.6 mm")));
        let mut p = core_one_l();
        p.material = Some("PLA".into());
        assert!(fit(&prusa_job(), false, &p)
            .reasons
            .iter()
            .any(|r| r.contains("PETG") && r.contains("PLA")));
        let mut p = core_one_l();
        p.material = Some("petg".into());
        assert!(
            fit(&prusa_job(), false, &p).ok,
            "material compares without case"
        );
        let mut p = core_one_l();
        p.bed_mm = Some([250.0, 220.0]);
        let r = fit(&prusa_job(), false, &p).reasons.join(" ");
        assert!(r.contains("300 x 300") && r.contains("250 x 220"), "{r}");
        let mut big = prusa_job();
        big.bed_mm = None;
        big.extents_mm = Some([280.0, 100.0, 10.0]);
        let mut p = core_one_l();
        p.bed_mm = Some([250.0, 220.0]);
        assert!(fit(&big, false, &p)
            .reasons
            .iter()
            .any(|r| r.contains("The print is 280 x 100")));
    }

    #[test]
    fn what_is_not_known_is_not_assumed() {
        let mut p = core_one_l();
        p.slicer_model = None;
        p.model_name = None;
        let f = fit(&prusa_job(), false, &p);
        assert!(
            f.ok && !f.verified && f.notes.iter().any(|n| n.contains("called in your slicer")),
            "{f:?}"
        );
        let f = fit(&Needs::default(), false, &core_one_l());
        assert!(
            f.ok && !f.verified
                && f.notes
                    .iter()
                    .any(|n| n.contains("does not say which printer")),
            "{f:?}"
        );
        let mut p = core_one_l();
        p.nozzle_mm = None;
        assert!(!fit(&prusa_job(), false, &p).verified);
    }

    #[test]
    fn the_queue_survives_a_restart_and_keeps_its_order() {
        let dir =
            std::path::PathBuf::from(format!("/tmp/stella-queue-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spool = |q: &Queue, n: &str| {
            let p = q.incoming().join(n);
            std::fs::write(&p, "; generated by PrusaSlicer 2.9.6\nG28\n; nozzle_diameter = 0.4\n; printer_model = COREONEL\n; filament_type = PETG\n").unwrap();
            p
        };
        let (a, b);
        {
            let q = Queue::open(&dir).unwrap();
            a = q
                .add_upload(&spool(&q, "x1.part"), "first part.gcode", 2)
                .unwrap();
            b = q
                .add_upload(&spool(&q, "x2.part"), "second_part.gcode", 1)
                .unwrap();
            assert_eq!(a.meta.printer_model.as_deref(), Some("COREONEL"));
            assert_eq!(a.title, "first part");
            q.reorder(&[b.id, a.id]).unwrap();
            q.patch(a.id, &json!({"priority":0,"target":"core","copies":3}))
                .unwrap();
            q.set_flow(
                "core",
                Flow {
                    awaiting_clear: true,
                    released: false,
                    auto_start: true,
                },
            )
            .unwrap();
            q.save_reference("core", &[1, 2, 3], b"jpeg", &Value::Null).unwrap();
        }
        let q = Queue::open(&dir).unwrap();
        let ids: Vec<i64> = q.jobs().unwrap().iter().map(|j| j.id).collect();
        assert_eq!(ids, vec![b.id, a.id], "order is kept");
        let a2 = q.job(a.id).unwrap().unwrap();
        assert_eq!((a2.copies, a2.target.as_deref()), (3, Some("core")));
        assert!(q.stored_path(&a2).unwrap().exists());
        assert_eq!(
            q.flow("core"),
            Flow {
                awaiting_clear: true,
                released: false,
                auto_start: true
            }
        );
        assert_eq!(q.plate("core").unwrap().px, vec![1, 2, 3]);
        // A refused file leaves nothing behind.
        let junk = q.incoming().join("j.part");
        std::fs::write(&junk, "x").unwrap();
        assert!(q
            .add_upload(&junk, "notes.txt", 1)
            .unwrap_err()
            .contains("only .gcode"));
        assert!(!junk.exists());
        // Removing a job removes its file.
        let stored = q.stored_path(&a2).unwrap();
        q.remove(a.id).unwrap();
        assert!(!stored.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_next_job_respects_priority_target_and_what_is_verified() {
        let dir =
            std::path::PathBuf::from(format!("/tmp/stella-queue-next-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let q = Queue::open(&dir).unwrap();
        let mk = |n: &str, model: &str| {
            let p = q.incoming().join(format!("{n}.part"));
            std::fs::write(&p, format!("; generated by PrusaSlicer 2.9.6\n; printer_model = {model}\n; nozzle_diameter = 0.4\n; filament_type = PETG\n")).unwrap();
            q.add_upload(&p, &format!("{n}.gcode"), 1).unwrap()
        };
        let other = mk("for-kobra", "Anycubic Kobra X");
        let low = mk("low", "COREONEL");
        let high = mk("high", "COREONEL");
        q.patch(high.id, &json!({"priority":5})).unwrap();
        let mut p = core_one_l();
        p.nozzle_mm = Some(0.4);
        let jobs = q.jobs().unwrap();
        assert_eq!(next_for(&jobs, &p).unwrap().id, high.id, "priority first");
        q.patch(high.id, &json!({"target":"somewhere-else"}))
            .unwrap();
        assert_eq!(
            next_for(&q.jobs().unwrap(), &p).unwrap().id,
            low.id,
            "a job named for another printer is skipped"
        );
        q.patch(low.id, &json!({"state":"cancelled"})).unwrap();
        assert!(
            next_for(&q.jobs().unwrap(), &p).is_none(),
            "the Kobra file is never given to the Prusa"
        );
        let _ = (other, std::fs::remove_dir_all(&dir));
    }
}
