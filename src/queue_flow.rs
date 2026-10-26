//! What happens around the queue (farm board WO-08): noticing that a print ended, asking whether the
//! plate is empty, and sending the next file. Nothing starts without a passing plate check or an
//! override the person confirmed; every start goes through the printer's own command route, so the
//! state check, the tier rules and the audit apply exactly as for a start from the HUD.
use crate::{
    driver,
    farm::{Farm, Printer},
    plate,
    queue::{self, Job, Profile, Queue},
    Result,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Weak},
    thread,
    time::Duration,
};

pub fn profile(p: &Printer) -> Profile {
    Profile {
        id: p.id.clone(),
        name: if p.entry.name.is_empty() {
            p.id.clone()
        } else {
            p.entry.name.clone()
        },
        slicer_model: p.entry.slicer_model.clone(),
        model_name: p.entry.model.clone(),
        bed_mm: p.entry.bed_size_mm,
        nozzle_mm: p.entry.nozzle_mm,
        material: p.entry.material.clone(),
    }
}

/// (state, job_state, file) of a printer as the wall shows it.
fn state_of(farm: &Farm, id: &str) -> (String, String) {
    let s = farm.summary();
    s["printers"]
        .as_array()
        .and_then(|a| a.iter().find(|p| p["id"] == id))
        .map(|p| {
            (
                p["state"].as_str().unwrap_or("offline").to_owned(),
                p["job_state"].as_str().unwrap_or("").to_owned(),
            )
        })
        .unwrap_or_else(|| ("offline".into(), String::new()))
}

/// One frame from this printer's camera, asked for through the service's own camera route so a
/// Klipper webcam, a PrusaLink RTSP camera and a FlashForge MJPEG stream are all the same call.
fn frame(farm: &Farm, id: &str) -> Result<Vec<u8>> {
    let url = format!("http://{}/api/p/{id}/camera/snapshot", farm.base.host());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    let mut r = agent
        .get(&url)
        .call()
        .map_err(|e| format!("the camera did not answer: {e}"))?;
    if r.status().as_u16() != 200 {
        let body = r.body_mut().read_to_string().unwrap_or_default();
        let why = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("status {}", r.status().as_u16()));
        return Err(format!("the camera did not answer: {why}"));
    }
    let mut buf = Vec::new();
    r.body_mut()
        .as_reader()
        .take(8 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| format!("the camera frame could not be read: {e}"))?;
    Ok(buf)
}

fn quad_of(region: &Value) -> plate::Quad {
    let pts: Option<Vec<[f64; 2]>> = region.as_array().map(|a| {
        a.iter()
            .filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]))
            .collect()
    });
    match pts {
        Some(v) if v.len() == 4 => {
            let q = plate::Quad([v[0], v[1], v[2], v[3]]);
            if q.valid() {
                q
            } else {
                plate::Quad::whole()
            }
        }
        _ => plate::Quad::whole(),
    }
}

/// Look at this printer's plate now. The record (and the frame) is kept with the decision.
pub fn check(farm: &Farm, q: &Queue, p: &Printer) -> Value {
    let Some(reference) = q.plate(&p.id) else {
        let _ = q.add_check(
            &p.id,
            "skipped",
            None,
            &json!({"why":"no empty-plate picture is saved"}),
            None,
        );
        return json!({"outcome":"skipped","reason":"No picture of the empty plate is saved for this printer, so nothing was compared. Set it up from the printer's settings."});
    };
    // Finish-2 F02 (CORE One L, 2026-10-19): the empty plate was captured with the bed at Z 98.1 and checked at Z 77.8, after a
    // print. On a printer whose bed or head moves, that is another picture: the plate sits elsewhere in the frame, the head
    // covers other places, and the part on it was missed. The pictures are compared only when the printer is where it was.
    let (now_pose, _) = crate::farm::position_of(p, 0.0);
    if let Some(why) = moved_since_capture(&reference.pose, &now_pose) {
        let _ = q.add_check(&p.id, "moved", None, &json!({"why":why}), None);
        return json!({"outcome":"moved","reason":why});
    }
    let got = frame(farm, &p.id).and_then(|jpeg| plate::prepare(&jpeg).map(|g| (jpeg, g)));
    let (jpeg, now) = match got {
        Ok(v) => v,
        Err(e) => {
            let _ = q.add_check(&p.id, "failed", None, &json!({"why":e}), None);
            return json!({"outcome":"failed","reason":e});
        }
    };
    let empty = match plate::from_raw(reference.px.clone()) {
        Ok(g) => g,
        Err(e) => return json!({"outcome":"failed","reason":e}),
    };
    let quad = quad_of(&reference.region);
    let v = plate::compare(&empty, &now, &quad, reference.sensitivity);
    let detail = json!({"score":v.score,"cells_different":v.cells_different,"cells_in_region":v.cells_in_region,"box":v.bbox,"sensitivity":reference.sensitivity});
    if v.clear {
        let id = q
            .add_check(&p.id, "clear", Some(v.score), &detail, Some(&jpeg))
            .unwrap_or(0);
        json!({"outcome":"clear","check_id":id,"score":v.score,"cells_in_region":v.cells_in_region})
    } else {
        let id = q
            .add_check(&p.id, "not_clear", Some(v.score), &detail, Some(&jpeg))
            .unwrap_or(0);
        json!({"outcome":"not_clear","check_id":id,"score":v.score,"cells_different":v.cells_different,"cells_in_region":v.cells_in_region,
            "box":v.bbox,"cells":v.cells,"grid":[plate::COLS,plate::ROWS],
            "reason":"Something on the plate looks different from the empty plate. The part that differs is marked."})
    }
}

/// Capture the empty plate now, as the picture later frames are compared with.
pub fn capture_reference(farm: &Farm, q: &Queue, p: &Printer) -> Result<Value> {
    let jpeg = frame(farm, &p.id)?;
    let g = plate::prepare(&jpeg)?;
    let (pose, _) = crate::farm::position_of(p, 0.0);
    q.save_reference(&p.id, &g.px, &jpeg, &json!(pose))?;
    Ok(json!({"captured":true,"pose":pose}))
}

/// Why the picture now cannot be compared with the empty plate: an axis the printer reported both times that is more than
/// 2 mm from where it was. An axis it did not report, then or now, is not held against it; a reference older than this
/// rule (no pose) is compared as before.
fn moved_since_capture(then: &Value, now: &[Option<f64>; 3]) -> Option<String> {
    let moved: Vec<String> = ["X", "Y", "Z"]
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let (t, n) = (then.get(i)?.as_f64()?, now[i]?);
            ((t - n).abs() > 2.0).then(|| format!("{a} {t:.1} then, {n:.1} now"))
        })
        .collect();
    (!moved.is_empty()).then(|| {
        format!(
            "The printer is not where it was when the empty plate was captured ({}), so the two pictures cannot be compared. Look at the plate: if it is clear, start anyway. To check by camera at this position, capture the empty plate again here.",
            moved.join("; ")
        )
    })
}

/// "The plate is cleared." Checks it (when there is a picture and a camera), and on a pass releases the
/// next job; with auto start on, sends it.
pub fn plate_cleared(farm: &Farm, q: &Queue, p: &Printer, override_: bool) -> Result<Value> {
    let mut flow = q.flow(&p.id);
    let c = check(farm, q, p);
    let outcome = c["outcome"].as_str().unwrap_or("failed").to_owned();
    let passed = matches!(outcome.as_str(), "clear" | "skipped");
    if !passed && !override_ {
        flow.released = false;
        q.set_flow(&p.id, flow)?;
        plate_found(p, &outcome);
        return Ok(json!({"cleared":false,"check":c,"can_override":true}));
    }
    if !passed {
        let _ = q.add_check(&p.id, "override", None, &json!({"after":outcome}), None);
    }
    flow.awaiting_clear = false;
    flow.released = true;
    q.set_flow(&p.id, flow.clone())?;
    let mut out = json!({"cleared":true,"check":c,"overridden":!passed});
    if flow.auto_start {
        match start(farm, q, p, None, false, passed) {
            Ok(v) => out["start"] = v,
            Err(e) => out["start_error"] = json!(e),
        }
    }
    Ok(out)
}

/// A plate check that did not pass is an alert (the "plate check failed" event, which the person can turn
/// off): the next file was not sent.
fn plate_found(p: &Printer, outcome: &str) {
    let name = &p.entry.name;
    let why = match outcome {
        "failed" => "the camera could not be read, so the plate could not be checked",
        _ => "the plate check found something on the plate",
    };
    crate::notify::event(
        "plate_failed",
        &p.id,
        name,
        &format!("{name}: {why}. The next file was not started."),
    );
}

/// Send the next fitting job (or `job_id`) to this printer and start it. Refuses unless the plate was
/// released by a passing check or a confirmed override.
pub fn start_next(
    farm: &Farm,
    q: &Queue,
    p: &Printer,
    job_id: Option<i64>,
    override_: bool,
    dry: bool,
) -> Result<Value> {
    if dry {
        return dry_run(farm, q, p, job_id);
    }
    start(farm, q, p, job_id, override_, false)
}

/// Finish-2 F05 (2026-10-19): a dry run of a queued file. A FlashForge cannot hand a file on it back to this service, so a
/// dry run of a file on the printer is impossible there; the queue holds the file itself, and this sends its no-extrusion,
/// heaters-off copy (`<name> (dry run).gcode`) and starts it. Nothing is laid down, so the plate is not looked at and the
/// job stays waiting in the queue.
fn dry_run(farm: &Farm, q: &Queue, p: &Printer, job_id: Option<i64>) -> Result<Value> {
    let (state, _) = state_of(farm, &p.id);
    if !matches!(state.as_str(), "idle" | "finished") {
        return Err(format!("{} is {state}; a dry run can start when it is ready or done", p.entry.name));
    }
    let job = q.jobs()?.into_iter().find(|j| Some(j.id) == job_id).ok_or("no such job")?;
    if job.source["kind"] == "printer" {
        return Err("This file is on the printer, which cannot send it back; add it to the queue from this computer to dry-run it.".into());
    }
    let path = q.stored_path(&job).ok_or("the file is missing from the queue")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the queued file: {e}"))?;
    if crate::dry_run::is_binary_gcode(&bytes) {
        return Err("This is a binary G-code file (.bgcode). A dry run needs plain G-code: slice it as .gcode to dry-run it.".into());
    }
    let text = crate::dry_run::transform(&String::from_utf8_lossy(&bytes), &crate::dry_run::Recipe::default())?;
    let name = crate::commands::dry_run_name(&queue::safe_name(&job.file_name));
    driver::open(&p.cfg)?
        .upload(&name, text.as_bytes(), false)
        .map_err(|e| format!("the dry run could not be sent to {}: {e}", p.entry.name))?;
    p.controls.run(&json!({"id":"print_start","confirm":true,"filename":name}))?;
    Ok(json!({"started":true,"dry_run":true,"job":job.id,"file":name}))
}

/// `prechecked`: the plate was just looked at (the "plate cleared" tap), so it is not looked at twice.
fn start(
    farm: &Farm,
    q: &Queue,
    p: &Printer,
    job_id: Option<i64>,
    override_: bool,
    prechecked: bool,
) -> Result<Value> {
    let (state, _) = state_of(farm, &p.id);
    if !matches!(state.as_str(), "idle" | "finished") {
        return Err(format!(
            "{} is {state}; a file can be started when it is ready or done",
            p.entry.name
        ));
    }
    let jobs = q.jobs()?;
    let prof = profile(p);
    let job: Job = match job_id {
        Some(id) => {
            let j = jobs.iter().find(|j| j.id == id).ok_or("no such job")?;
            if !j.waiting() {
                return Err("that job is not waiting".into());
            }
            if j.target.as_deref().is_some_and(|t| t != p.id) {
                return Err("that job is named for another printer".into());
            }
            let f = queue::fit(&j.needs(), j.force_model, &prof);
            if !f.ok {
                return Err(f.reasons.join(" "));
            }
            j.clone()
        }
        None => queue::next_for(&jobs, &prof)
            .cloned()
            .ok_or("nothing in the queue fits this printer")?,
    };
    // The plate gate: every start looks at the plate now. A tap earlier does not stand in for it, because
    // anything may have been put on the plate since.
    let mut flow = q.flow(&p.id);
    if !prechecked {
        let c = check(farm, q, p);
        let passed = matches!(c["outcome"].as_str(), Some("clear" | "skipped"));
        if !passed && !override_ {
            flow.released = false;
            let _ = q.set_flow(&p.id, flow);
            plate_found(p, c["outcome"].as_str().unwrap_or("failed"));
            return Ok(json!({"started":false,"check":c,"can_override":true,"job":job.id}));
        }
        if !passed {
            let _ = q.add_check(
                &p.id,
                "override",
                None,
                &json!({"after":c["outcome"]}),
                None,
            );
        }
    }
    // Send it.
    let on_printer = job.source["kind"] == "printer";
    let name = if on_printer {
        job.file_name.clone()
    } else {
        queue::safe_name(&job.file_name)
    };
    if !on_printer {
        let path = q
            .stored_path(&job)
            .ok_or("the file is missing from the queue")?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot read the queued file: {e}"))?;
        driver::open(&p.cfg)?
            .upload(&name, &bytes, false)
            .map_err(|e| format!("the file could not be sent to {}: {e}", p.entry.name))?;
    }
    p.controls
        .run(&json!({"id":"print_start","confirm":true,"filename":name}))?;
    q.set_running(job.id, &p.id)?;
    flow.awaiting_clear = false;
    flow.released = false;
    q.set_flow(&p.id, flow)?;
    Ok(json!({"started":true,"job":job.id,"title":job.title,"file":name}))
}

// ---- the picture the pages draw -------------------------------------------------------------------

pub fn job_json(job: &Job, profiles: &[Profile]) -> Value {
    let n = job.needs();
    let fits: Vec<Value> = profiles
        .iter()
        .map(|p| {
            let f = queue::fit(&n, job.force_model, p);
            let named_elsewhere = job.target.as_deref().is_some_and(|t| t != p.id);
            json!({"printer":p.id,"name":p.name,"ok":f.ok && !named_elsewhere,"verified":f.verified,
                "reasons":if named_elsewhere { vec![format!("Named for another printer.")] } else { f.reasons },"notes":f.notes})
        })
        .collect();
    json!({
        "id":job.id,"title":job.title,"file":job.file_name,"copies":job.copies,"done":job.done,"priority":job.priority,
        "target":job.target,"force_model":job.force_model,"state":job.state,"note":job.note,"printer":job.printer_id,
        "source":job.source["kind"],"created":job.created_at,"started":job.started_at,"finished":job.finished_at,
        "needs":{"printer_model":n.printer_model,"nozzle_mm":n.nozzle_mm,"filament":n.filament,"bed_mm":n.bed_mm,"extents_mm":n.extents_mm,
            "slicer":job.meta.slicer,"binary":job.meta.binary},
        "overrides":job.overrides,"fits":fits,
    })
}

pub fn overview(farm: &Farm, q: &Queue) -> Result<Value> {
    // F04 (2026-10-19): the placeholder of a service with only demo printers is not a printer a person can see; it was listed here as
    // "Stella, offline" (and as a place a queued file could go).
    let list = farm.visible();
    let profiles: Vec<Profile> = list.iter().map(|p| profile(p)).collect();
    let jobs = q.jobs()?;
    let summary = farm.summary();
    let rows = summary["printers"].as_array().cloned().unwrap_or_default();
    let printers: Vec<Value> = list
        .iter()
        .zip(&profiles)
        .map(|(p, prof)| {
            let flow = q.flow(&p.id);
            let plate = q.plate(&p.id);
            let row = rows.iter().find(|r| r["id"] == p.id.as_str());
            json!({"id":p.id,"name":prof.name,"state":row.map(|r| r["state"].clone()).unwrap_or(json!("offline")),
                "awaiting_clear":flow.awaiting_clear,"released":flow.released,"auto_start":flow.auto_start,
                "has_reference":plate.is_some(),"region_set":plate.as_ref().is_some_and(|x| !x.region.is_null()),
                "sensitivity":plate.as_ref().map(|x| x.sensitivity),"captured_at":plate.as_ref().map(|x| x.captured_at),
                "camera":row.map(|r| r["camera_url"].is_string()).unwrap_or(false),
                "next_job":queue::next_for(&jobs, prof).map(|j| j.id),
                "slicer_model":p.entry.slicer_model,"nozzle_mm":p.entry.nozzle_mm,"material":p.entry.material})
        })
        .collect();
    let waiting = jobs
        .iter()
        .filter(|j| j.waiting())
        .map(|j| j.copies - j.done)
        .sum::<u32>();
    Ok(
        json!({"jobs":jobs.iter().map(|j| job_json(j, &profiles)).collect::<Vec<_>>(),"printers":printers,"waiting":waiting}),
    )
}

// ---- noticing that a print ended -------------------------------------------------------------------

/// A printer that was working and has stopped. Two things follow, and they are not the same set of states:
/// - a queue job that was running on it is closed out, whatever the printer was doing (a FlashForge reports
///   `heating` while it warms up for a file it was sent, so a job that fails or is cancelled in the warm-up ends here);
/// - the plate is left to be cleared only after a print (`printing`, `paused`). Heating alone is not a print: the
///   farm state `heating` is a preheat, or a print still at 0 % while its heaters climb (polish WO-02), so a printer
///   that stops from there has laid nothing on its plate (polish WO-04, 2026-10-18: the wall showed "plate needs clearing" on every printer that finished a preheat).
fn job_ended(before: Option<&str>, now: &str) -> bool {
    matches!(before, Some("printing" | "paused" | "heating")) && matches!(now, "finished" | "idle" | "error")
}
fn plate_dirtied(before: Option<&str>, now: &str) -> bool {
    matches!(before, Some("printing" | "paused")) && matches!(now, "finished" | "idle" | "error")
}

/// Apply one printer's change of state to the queue. Returns whether the plate was marked as needing clearing and
/// whether a running job was closed out (a finished copy counts, anything else stops the job).
fn settle(q: &Queue, id: &str, before: Option<&str>, state: &str, job_state: &str, file: &str) -> (bool, bool) {
    if !job_ended(before, state) {
        return (false, false);
    }
    // Finish-2 F02 (CORE One L, 2026-10-19): a cancelled dry run left the plate "needs clearing" and the farm counting one
    // printer that needs attention, though a dry run lays nothing down. Its file is named so (commands::dry_run_name).
    // A FlashForge stores the copy as `<name>__dry_run_.gcode` (its file names keep letters, digits, dot, dash and underscore).
    let dirty = plate_dirtied(before, state) && !file.contains("(dry run)") && !file.contains("_dry_run_");
    if dirty {
        let mut flow = q.flow(id);
        flow.awaiting_clear = true;
        flow.released = false;
        let _ = q.set_flow(id, flow);
    }
    let mut closed = false;
    if let Ok(Some(job)) = q.running_on(id) {
        let completed = state == "finished" && job_state == "complete";
        let why = match (state, job_state) {
            ("error", _) => "The printer reported an error.".to_owned(),
            (_, "cancelled") => "The print was cancelled.".to_owned(),
            (s, j) => format!("The print ended ({s}, {j})."),
        };
        closed = q.finish(job.id, completed, &why).is_ok();
    }
    (dirty, closed)
}

/// Watch the printers' states; when one that was working stops, the plate needs clearing, and a queue
/// job that was running on it is closed out (a finished copy counts, anything else stops the job).
pub fn spawn_watch(farm: &Arc<Farm>) {
    let weak: Weak<Farm> = Arc::downgrade(farm);
    let _ = thread::Builder::new()
        .name("queue-watch".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut prev: HashMap<String, String> = HashMap::new();
            loop {
                thread::sleep(Duration::from_secs(2));
                let Some(farm) = weak.upgrade() else { break };
                let Some(q) = farm.queue() else { continue };
                for p in farm.summary()["printers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    let (Some(id), Some(state)) = (p["id"].as_str(), p["state"].as_str()) else {
                        continue;
                    };
                    let before = prev.insert(id.to_owned(), state.to_owned());
                    let (dirty, closed) = settle(
                        &q,
                        id,
                        before.as_deref(),
                        state,
                        p["job_state"].as_str().unwrap_or(""),
                        p["file"].as_str().unwrap_or(""),
                    );
                    let name = p["name"].as_str().unwrap_or(id);
                    // The last queued file is done and nothing is waiting anywhere.
                    if closed
                        && q.jobs()
                            .is_ok_and(|jobs| !jobs.is_empty() && jobs.iter().all(|j| !j.waiting()))
                    {
                        crate::notify::event(
                            "queue_empty",
                            "",
                            "",
                            "The queue is empty: every file has been printed.",
                        );
                    }
                    // Something is waiting for this plate: say it needs clearing (the print's own end is its
                    // own alert, from the recorder).
                    let waiting = dirty && farm.get(id).is_some_and(|pr| {
                        q.jobs()
                            .is_ok_and(|jobs| queue::next_for(&jobs, &profile(&pr)).is_some())
                    });
                    if waiting {
                        crate::notify::event(
                            "plate_clear",
                            id,
                            name,
                            &format!("{name}: the plate needs clearing. A file is waiting for it."),
                        );
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{job_ended, plate_dirtied, settle};
    use crate::queue::Queue;

    #[test]
    fn a_stopped_print_dirties_the_plate_but_a_finished_preheat_does_not() {
        for before in ["printing", "paused"] {
            for now in ["finished", "idle", "error"] {
                assert!(plate_dirtied(Some(before), now), "{before} to {now}");
                assert!(job_ended(Some(before), now), "{before} to {now}");
            }
        }
        // a preheat that reaches its temperature goes heating -> idle: nothing was printed
        assert!(!plate_dirtied(Some("heating"), "idle"));
        assert!(!plate_dirtied(Some("heating"), "finished"));
        // still working, or never seen before, or not stopped
        for (before, now) in [
            (Some("printing"), "printing"),
            (Some("printing"), "paused"),
            (Some("printing"), "offline"),
            (None, "idle"),
            (Some("idle"), "heating"),
            (Some("offline"), "idle"),
        ] {
            assert!(!plate_dirtied(before, now), "{before:?} to {now}");
            assert!(!job_ended(before, now), "{before:?} to {now}");
        }
        // a warm-up is still a state a queue job can end in (FlashForge reports it for a file it was sent)
        assert!(job_ended(Some("heating"), "error"));
        assert!(job_ended(Some("heating"), "idle"));
    }

    /// The rule the farm wall relies on, with a real queue: a job whose printer drops out of warm-up is closed out and the plate
    /// is not marked; a job whose print ends is closed out and the plate is marked.
    #[test]
    fn a_job_closes_when_its_printer_stops_but_only_a_print_marks_the_plate() {
        let dir = std::path::PathBuf::from(format!("/tmp/stella-queue-flow-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let q = Queue::open(&dir).unwrap();
        let spool = |name: &str| {
            let p = q.incoming().join(name);
            std::fs::write(&p, "; generated by PrusaSlicer 2.9.6\nG28\n; nozzle_diameter = 0.4\n; printer_model = COREONEL\n; filament_type = PETG\n").unwrap();
            p
        };
        let warm = q.add_upload(&spool("warm.part"), "warm.gcode", 1).unwrap();
        q.set_running(warm.id, "fx").unwrap();
        // heating -> error with a running queue job: the job closes (stopped, with the printer's error), the plate is untouched
        assert_eq!(settle(&q, "fx", Some("heating"), "error", "error", "part.gcode"), (false, true));
        let j = q.job(warm.id).unwrap().unwrap();
        assert_eq!(j.state, "stopped");
        assert_eq!(j.note.as_deref(), Some("The printer reported an error."));
        assert!(!q.flow("fx").awaiting_clear, "a warm-up that failed left nothing on the plate");
        // heating -> idle with a running job (cancelled in the warm-up) closes it too
        let cancelled = q.add_upload(&spool("cancel.part"), "cancel.gcode", 1).unwrap();
        q.set_running(cancelled.id, "fx").unwrap();
        assert_eq!(settle(&q, "fx", Some("heating"), "idle", "standby", "part.gcode"), (false, true));
        assert_eq!(q.job(cancelled.id).unwrap().unwrap().state, "stopped");
        assert!(!q.flow("fx").awaiting_clear);
        // a warm-up with no job in the queue (a plain preheat) changes nothing at all
        assert_eq!(settle(&q, "fx", Some("heating"), "idle", "standby", "part.gcode"), (false, false));
        assert!(!q.flow("fx").awaiting_clear);
        // printing -> finished: the copy counts and the plate needs clearing
        let print = q.add_upload(&spool("print.part"), "print.gcode", 1).unwrap();
        q.set_running(print.id, "fx").unwrap();
        assert_eq!(settle(&q, "fx", Some("printing"), "finished", "complete", "part.gcode"), (true, true));
        assert_eq!(q.job(print.id).unwrap().unwrap().state, "done");
        let f = q.flow("fx");
        assert!(f.awaiting_clear && !f.released);
        // finish-2 F02: a dry run that ends leaves nothing on the plate
        let mut flow = q.flow("fx");
        flow.awaiting_clear = false;
        let _ = q.set_flow("fx", flow);
        assert_eq!(settle(&q, "fx", Some("printing"), "finished", "cancelled", "twistbowl (dry run).gcode").0, false);
        assert_eq!(settle(&q, "fx", Some("printing"), "finished", "cancelled", "nfc-cube-tpu__dry_run_.gcode").0, false, "the FlashForge copy");
        assert!(!q.flow("fx").awaiting_clear);
        // a printer that is still working changes nothing
        assert_eq!(settle(&q, "fx", Some("printing"), "printing", "printing", "part.gcode"), (false, false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod moved_tests {
    use super::moved_since_capture;
    use serde_json::json;

    /// Finish-2 F02: the empty plate at Z 98.1, the check at Z 77.8 after a print: not compared, and it says why.
    #[test]
    fn a_printer_that_moved_since_the_empty_plate_is_not_compared() {
        let why = moved_since_capture(&json!([156.8, 131.6, 98.1]), &[Some(242.0), Some(211.0), Some(77.8)]).unwrap();
        assert!(why.contains("Z 98.1 then, 77.8 now") && why.contains("X 156.8 then"), "{why}");
        // within 2 mm, or an axis not reported (PrusaLink while moving), or an old reference with no pose: compared
        assert!(moved_since_capture(&json!([156.8, 131.6, 98.1]), &[Some(157.5), Some(131.6), Some(98.0)]).is_none());
        assert!(moved_since_capture(&json!([null, null, 98.1]), &[Some(10.0), None, Some(98.1)]).is_none());
        assert!(moved_since_capture(&serde_json::Value::Null, &[Some(1.0), Some(2.0), Some(3.0)]).is_none());
    }
}
