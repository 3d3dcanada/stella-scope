use crate::{
    api::workspace,
    config::Config,
    log_tail::LogTail,
    model::{self, CollectorStatus, MOTORS},
    moonraker::Moonraker,
    qgl::Parser,
    store::{Event, Store},
    ws_source::{self, WsEvent},
    Result,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub type Shared = Arc<Mutex<CollectorStatus>>;

fn new_print(last: &Option<(String, Option<f64>)>, filename: &str, duration: Option<f64>) -> bool {
    last.as_ref().is_none_or(|(old, previous)| old != filename || previous.zip(duration).is_some_and(|(a,b)| b + 2.0 < a))
}

// Resonance/shaper console output gets its own event kind so sweeps are searchable in the timeline.
pub fn is_resonance(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("test_resonances")
        || m.contains("shaper_calibrate")
        || m.contains("accelerometer_query")
        || m.contains("fitted shaper")
}

// BLTouch alarm, from a real printer\'s recording of 2026-09-21 (11:19:52-54, live DB): the console
// shows "// Failed to verify BLTouch probe is raised; retrying." and then "!! BLTouch failed to
// deploy". only the hard failures are the alarm. In Klipper's bltouch.py the "; retrying." lines
// are respond_info inside a three-try loop; the command_error after the last try is "BLTouch failed
// to raise probe" / "BLTouch failed to verify sensor state" (no retrying). "BLTouch failed to
// deploy" is what the Klipper printed on 2026-09-21 and stays hard.
const SENSOR_STATE: &str = "bltouch failed to verify sensor state";
fn sensor_state(m: &str, retrying: bool) -> bool {
    m.match_indices(SENSOR_STATE).any(|(i, _)| {
        m[i + SENSOR_STATE.len()..]
            .trim_start_matches([';', ' '])
            .starts_with("retrying")
            == retrying
    })
}
pub fn is_probe_alarm(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("bltouch failed to deploy")
        || m.contains("bltouch failed to raise probe")
        || sensor_state(&m, false)
}
/// A BLTouch check Klipper is retrying. Recorded and searchable, never an alarm.
pub fn is_probe_retry(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("failed to verify bltouch probe is raised") || sensor_state(&m, true)
}
/// The fix, in the words both the timeline advisory and the desktop notice use.
const PROBE_ALARM_ADVICE: &str = "The probe failed to deploy, failed to raise, or failed its sensor check, and Klipper stopped the command. Power-cycle the probe and run BLTOUCH_DEBUG COMMAND=reset before the next print.";

/// /14: this poll's console lines tagged for the timeline. A BLTouch hard failure becomes
/// `probe_alarm` plus one plain-words advisory and its timestamp for the notice; a retry Klipper
/// is still working through becomes `probe_retry` with no advisory and no notice.
fn tag_console(events: Vec<Event>) -> (Vec<Event>, Vec<Event>, Option<f64>) {
    let mut advisories = Vec::new();
    let mut alarm = None;
    let events = events
        .into_iter()
        .map(|mut e| {
            if is_resonance(&e.message) {
                e.kind = "resonance".into();
            } else if is_probe_alarm(&e.message) {
                e.kind = "probe_alarm".into();
                alarm = Some(e.ts);
                advisories.push(Event::new(
                    e.ts,
                    "probe_alarm",
                    format!("BLTouch alarm: {PROBE_ALARM_ADVICE}"),
                ));
            } else if is_probe_retry(&e.message) {
                e.kind = "probe_retry".into();
            }
            e
        })
        .collect();
    (events, advisories, alarm)
}

/// One notice per alarm, and none for an alarm older than this service start. Moonraker's
/// console window replays its last 100 lines on every poll and the launcher starts the service
/// each time the user opens the app, so an old alarm still in that window would otherwise notify
/// on every start. `ts` is Moonraker's console `time` and `started` this process's start: both
/// are Unix epoch seconds (the Pi's clock and this machine's).
pub fn should_notify_alarm(ts: f64, last: f64, started: f64) -> bool {
    ts > last && ts >= started
}

/// One desktop notice through notify-send. A missing binary or a failed spawn degrades to
/// nothing: the notice is a courtesy on top of the recording, never a failure path, so the spawn
/// result is deliberately ignored.
fn desktop_notify(summary: &str, body: &str, urgent: bool) {
    if std::env::var_os("PATH").is_none() {
        return;
    }
    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "Stella Scope",
            "-u",
            if urgent { "critical" } else { "normal" },
            "-c",
            "printer",
            summary,
            body,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn add_pending(pending: &mut Vec<Event>, events: Vec<Event>) {
    for e in events {
        if !pending
            .iter()
            .any(|p| p.ts == e.ts && p.message == e.message)
        {
            pending.push(e);
        }
    }
    pending.sort_by(|a, b| a.ts.total_cmp(&b.ts));
}

// BLTouch harvest from console lines: probe points with procedure context, Z-offset change
// events, and a zero-reference drift alert. Formats are from a real printer\'s recording.
struct ProbeHarvest {
    points: Vec<crate::probe::ProbePoint>,
    offset_events: Vec<Event>,
    alerts: Vec<Event>,
}
fn harvest_probe(
    events: &[Event],
    captured: f64,
    recent_commands: &mut std::collections::VecDeque<String>,
    qgl_round: &mut u64,
    zeros: &mut Vec<f64>,
    printing: Option<&str>,
) -> ProbeHarvest {
    let mut out = ProbeHarvest {
        points: Vec::new(),
        offset_events: Vec::new(),
        alerts: Vec::new(),
    };
    for e in events {
        let lower = e.message.to_lowercase();
        if e.kind == "command" {
            recent_commands.push_back(lower.chars().take(120).collect());
            if recent_commands.len() > 24 {
                recent_commands.pop_front();
            }
            if lower.contains("quad_gantry_level") {
                *qgl_round = 0;
            }
        }
        if let Some(desc) = crate::probe::offset_change(&e.message) {
            out.offset_events
                .push(Event::new(e.ts, "z_offset_change", desc));
        }
        if let Some((x, y, z)) = crate::probe::point_from(&e.message) {
            if lower.contains("gantry-relative probe points") {
                *qgl_round += 1;
            }
            let commands: Vec<String> = recent_commands.iter().cloned().collect();
            let in_qgl = commands.iter().any(|c| c.contains("quad_gantry_level"));
            let context = crate::probe::context_for(
                &commands,
                if in_qgl { Some(*qgl_round) } else { None },
                printing,
            );
            if crate::probe::is_zero_reference(x, y, &context) && !zeros.is_empty() {
                let mean = zeros.iter().sum::<f64>() / zeros.len() as f64;
                if (z - mean).abs() > 0.05 {
                    out.alerts.push(Event::new(
                        e.ts,
                        "z_health_alert",
                        format!("Zero reference {z:.3} mm at 175,175 drifts {d:+.3} mm from the {n}-print mean {mean:.3}; check what changed before trusting first layers", d = z - mean, n = zeros.len(), mean = mean),
                    ));
                }
            }
            if crate::probe::is_zero_reference(x, y, &context) {
                zeros.push(z);
                if zeros.len() > 5 {
                    zeros.remove(0);
                }
            }
            out.points.push(crate::probe::ProbePoint {
                ts: e.ts,
                x,
                y,
                z,
                context,
            });
        }
    }
    let _ = captured;
    out
}

/// The notice text for a finished print. The summary is one word of outcome; the body
/// carries what the user can act on without opening the window.
fn print_notice(session: &crate::store::PrintSession) -> (&'static str, String, bool) {
    let file = session
        .filename
        .rsplit('/')
        .next()
        .unwrap_or(&session.filename);
    match session.result.as_str() {
        "error" => (
            "Print fault",
            format!(
                "{file} stopped with an error state. Klipper's reason is in the console; the recording kept it."
            ),
            true,
        ),
        "cancelled" => ("Print cancelled", format!("{file} was cancelled."), false),
        _ => {
            let body = match session.duration_s {
                Some(d) => format!("{file} finished after {} min.", (d / 60.0).round()),
                None => format!("{file} finished; duration unknown."),
            };
            ("Print finished", body, false)
        }
    }
}

const PUSH_STALE_S: f64 = 2.0;

pub fn run(cfg: Config, demo: bool, state: Shared, mut db: Store) -> Result<()> {
    let mut source = Moonraker::new(&cfg);
    let mut parser = Parser::new(db.last_run()?);
    let mut tail = LogTail::default();
    let start = Instant::now();
    let started = model::now();
    // Push transport: Moonraker pushes every status change over the websocket; the collector
    // only falls back to HTTP polling when push goes quiet. GET-only towards the printer.
    let ws_rx = if demo {
        None
    } else {
        Some(ws_source::spawn(&cfg))
    };
    let mut pending = vec![
        Event::new(started, "scope_geometry", model::geometry(&cfg).to_string()),
        Event::new(
            started,
            "scope_origin",
            json!(if demo { "demo" } else { "live" }).to_string(),
        ),
        Event::new(
            started,
            "recorder",
            "Recorder started. Incomplete levelling rounds may have unknown corrections.",
        ),
    ];
    let mut next_console = 0.0;
    let mut next_log = 0.0;
    let mut last_prune = Instant::now();
    let mut console_error: Option<String> = None;
    let mut maintenance_error: Option<String> = None;
    let mut log_error: Option<String> = None;
    let mut last_health = String::new();
    let mut last_flags = Value::Null;
    let mut last_demo_round: Option<(u64, u64)> = None;
    let mut last_disk_error = String::new();
    let mut lost_backlog = false;
    let mut writes = 0_u64;
    let mut write_total = 0.0;
    // Config cache: configfile is fetched once per (re)connect, Klippy restart and SAVE_CONFIG —
    // never per sample. The recorded sample schema keeps configfile so history stays comparable.
    let mut cached_config = Value::Null;
    let mut need_config = true;
    let mut manifest = Value::Null;
    let mut transport = if demo { "demo" } else { "" }.to_string();
    let mut last_ws = 0.0_f64;
    let mut next_poll = 0.0_f64;
    let mut update_times: VecDeque<f64> = VecDeque::new();
    let mut query_ms_last: Option<f64> = None;
    let mut query_ms_ema: Option<f64> = None;
    let mut push_frame_age_s: Option<f64> = None;
    let mut push_keepalive_ms: Option<f64> = None;
    let mut activity_cache: Option<(f64, Value)> = None;
    let mut last_poll_ok = 0.0_f64;
    // What the printer-state indicator is read from (model::printer_link). The newest error is
    // kept until a success, so the state does not flicker between two failed polls.
    let mut last_webhooks = Value::Null;
    let mut link_error: Option<String> = None;
    // BLTouch + print-context state
    let mut recent_commands: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut qgl_round_seen: u64 = 0;
    let mut last_zeros: Vec<f64> = Vec::new();
    let mut last_print: Option<(String, Option<f64>)> = None;
    let mut context_captured = false;
    // A failed capture (no Moonraker metadata for the file, a write error) retries every 30 s, not
    // every poll: each try asks the printer for metadata and 64 KiB of G-code.
    let mut context_retry = Instant::now();
    let mut context_error: Option<String> = None;
    // BLTouch alarm notice state. `fresh_alarm` carries this poll's alarm timestamp to the write
    // stage; `last_alarm_notice` stops the 100-line console window from re-notifying the same alarm
    // on every poll until it scrolls out. it starts at this process's start, so an alarm from
    // before the start is recorded but never notified (should_notify_alarm).
    let mut fresh_alarm: Option<f64> = None;
    let mut last_alarm_notice: f64 = started;
    let mut probe_cache: Vec<Value> = Vec::new();
    let mut pending_probe: Vec<crate::probe::ProbePoint> = Vec::new();
    loop {
        let tick = Instant::now();
        let attempt = model::now();
        let elapsed = start.elapsed().as_secs_f64();
        let captured = model::now();
        let mut error: Option<String> = None;
        let mut degraded: Vec<String> = Vec::new();
        // --- acquire a sample -------------------------------------------------------------
        let mut sample: Option<Value> = if demo {
            Some(model::demo(elapsed, &cfg))
        } else {
            None
        };
        if !demo {
            let rx = ws_rx
                .as_ref()
                .expect("live mode always spawns the websocket source");
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(WsEvent::Sample(d)) if d.is_object() => {
                    last_ws = attempt;
                    transport = "push".into();
                    sample = Some(d);
                }
                Ok(WsEvent::Sample(_)) => {} // silence probe from a reconnecting source
                Ok(WsEvent::Connected) => {
                    need_config = true;
                    transport = "push".into();
                    last_ws = attempt;
                }
                Ok(WsEvent::KlippyRestart) => {
                    need_config = true;
                }
                Ok(WsEvent::Latency {
                    frame_age_s,
                    keepalive_ms,
                }) => {
                    // Push-mode freshness (round-4 item 2): reported in /api/meta and the tick
                    // observer so the header can answer "how fresh is this really" in push mode,
                    // where the poll-mode query_ms figures never move.
                    push_frame_age_s = Some(frame_age_s);
                    push_keepalive_ms = Some(keepalive_ms);
                }
                Err(_) => {
                    // Push quiet: healthy links stay silent while nothing changes; the source
                    // resends a full sample at least twice a second, so quiet means trouble.
                    if attempt - last_ws < PUSH_STALE_S {
                        transport = "push".into();
                    } else if attempt >= next_poll {
                        next_poll = attempt + cfg.poll_interval_ms as f64 / 1000.0;
                        transport = "poll".into();
                        let t0 = Instant::now();
                        match source.sample(attempt) {
                            Ok((d, w)) => {
                                degraded.extend(w);
                                sample = Some(d);
                                last_poll_ok = attempt;
                                query_ms_last = Some(t0.elapsed().as_secs_f64() * 1000.0);
                                query_ms_ema = Some(match query_ms_ema {
                                    Some(m) => m * 0.9 + query_ms_last.unwrap() * 0.1,
                                    None => query_ms_last.unwrap(),
                                });
                            }
                            Err(e) => {
                                error = Some(format!("Printer not answering. {e}"));
                            }
                        }
                    } else {
                        transport = "poll".into();
                    }
                }
            }
        }
        if !demo {
            if need_config {
                match source.fetch_config() {
                    Ok(settings) => {
                        cached_config = json!({"settings":settings});
                        need_config = false;
                    }
                    Err(e) => {
                        degraded.push(format!("Configuration could not be read: {e}"));
                        error = error.or(Some(format!("Configuration could not be read: {e}")));
                    }
                }
                if manifest.is_null() {
                    source.discover();
                    manifest = source.manifest();
                }
            }
            if let Some(data) = sample.as_mut() {
                if let Some(obj) = data.as_object_mut() {
                    if cached_config.is_object() {
                        obj.insert("configfile".into(), cached_config.clone());
                    }
                    if manifest.is_object() {
                        obj.insert("_scope_capture".into(), manifest.clone());
                    }
                    if cfg.telemetry.host_stats {
                        let host = source.host_tick(attempt);
                        if let Some(e) = host["error"].as_str() {
                            degraded.push(format!("Host statistics unavailable: {e}"));
                        }
                        obj.insert("_scope_host".into(), host);
                    }
                }
            }
        }
        // --- console + log capture -------------------------------------------------------
        if demo {
            let cycle = (elapsed / 90.0) as u64;
            let round = ((elapsed % 90.0) / 8.0) as u64;
            if round < 4 && last_demo_round != Some((cycle, round)) {
                last_demo_round = Some((cycle, round));
                let range = [0.24, 0.08, 0.034, 0.015][round as usize];
                let msg=format!("Gantry-relative probe points:\n0: 15.474970 1: 15.504345 2: 15.487470 3: 15.464345\nMaking the following Z adjustments:\nstepper_z = 0.010297\nstepper_z1 = -0.035458\nstepper_z2 = -0.002368\nstepper_z3 = 0.027528\nRetries: {round}/5 Probed points range: {range:.6} tolerance: 0.020000");
                add_pending(&mut pending, vec![Event::new(captured, "response", msg)]);
                recent_commands.push_back("QUAD_GANTRY_LEVEL".into());
                if recent_commands.len() > 24 {
                    recent_commands.pop_front();
                }
                let mut demo_probe = vec![Event::new(
                    captured,
                    "response",
                    format!("QUAD_GANTRY_LEVEL"),
                )];
                for [px, py, pz] in [
                    [27.05, 24.28, -1.95 + 0.02 * round as f64],
                    [27.05, 319.28, -1.61 + 0.01 * round as f64],
                    [297.05, 319.28, 0.448 - 0.01 * round as f64],
                    [297.05, 24.28, 0.101 + 0.005 * round as f64],
                ] {
                    demo_probe.push(Event::new(
                        captured,
                        "response",
                        format!("// probe: at {px:.3},{py:.3} bed will contact at z={pz:.6}"),
                    ));
                }
                let printing = sample
                    .as_ref()
                    .and_then(|d| d["print_stats"]["filename"].as_str());
                let harvest = harvest_probe(
                    &demo_probe,
                    captured,
                    &mut recent_commands,
                    &mut qgl_round_seen,
                    &mut last_zeros,
                    printing,
                );
                add_pending(&mut pending, harvest.offset_events);
                add_pending(&mut pending, harvest.alerts);
                add_pending(&mut pending, demo_probe);
                for p in harvest.points {
                    probe_cache
                        .push(json!({"ts":p.ts,"x":p.x,"y":p.y,"z":p.z,"context":p.context}));
                }
                if probe_cache.len() > 40 {
                    let cut = probe_cache.len() - 40;
                    probe_cache.drain(0..cut);
                }
            }
        } else {
            if attempt >= next_console {
                next_console = attempt + cfg.console_interval_ms as f64 / 1000.0;
                match source.console() {
                    Ok((events, full)) => {
                        console_error = if full {
                            Some("Console history is at its 100-message cap; messages older than this window may have been missed. Shorten console_interval_ms for busy macros.".into())
                        } else {
                            None
                        };
                        for e in &events {
                            if e.message.to_lowercase().contains("save_config") {
                                need_config = true;
                            }
                        }
                        let printing = sample
                            .as_ref()
                            .and_then(|d| d["print_stats"]["filename"].as_str());
                        let harvest = harvest_probe(
                            &events,
                            captured,
                            &mut recent_commands,
                            &mut qgl_round_seen,
                            &mut last_zeros,
                            printing,
                        );
                        // A BLTouch alarm is tagged and explained in one plain-words event, so the
                        // timeline and the findings list carry the fix, not just the symptom.
                        let (events, advisories, alarm_notice) = tag_console(events);
                        add_pending(&mut pending, advisories);
                        add_pending(&mut pending, harvest.offset_events);
                        add_pending(&mut pending, harvest.alerts);
                        add_pending(&mut pending, events);
                        fresh_alarm = alarm_notice;
                        if !harvest.points.is_empty() {
                            pending_probe.extend(harvest.points);
                            for p in pending_probe.iter().rev().take(40) {
                                probe_cache.push(
                                    json!({"ts":p.ts,"x":p.x,"y":p.y,"z":p.z,"context":p.context}),
                                );
                            }
                            if probe_cache.len() > 40 {
                                let cut = probe_cache.len() - 40;
                                probe_cache.drain(0..cut);
                            }
                        }
                    }
                    Err(e) => console_error = Some(format!("Console capture unavailable. {e}")),
                }
            }
            match &cfg.klippy_log_path {
                None=>degraded.push("Local log capture is not configured; position-skew warnings that appear only in klippy.log cannot be recorded. Set klippy_log_path on the Pi.".into()),
                Some(path) if attempt>=next_log=>{
                    next_log=attempt+1.0;
                    match tail.read_new(path,65536) {
                        Ok((lines,notices))=>{
                            log_error=None;
                            add_pending(&mut pending,lines.into_iter().map(|s|Event::new(captured,"position_warning",s)).collect());
                            add_pending(&mut pending,notices.into_iter().map(|s|Event::new(captured,"log_notice",s)).collect());
                        }
                        Err(e)=>log_error=Some(e),
                    }
                }
                _=>{},
            }
        }
        for message in [&console_error, &maintenance_error, &log_error]
            .into_iter()
            .flatten()
        {
            degraded.push(message.clone());
        }
        if lost_backlog {
            degraded.push("Storage backlog exceeded 1024 console events during a disk failure; some new events were lost. Existing recorded data was not deleted.".into());
        }
        if captured < attempt {
            degraded.push("The recorder clock moved backward during this poll; check time synchronization before interpreting timestamps.".into());
        }
        if let Some(data) = &sample {
            let flags = json!(MOTORS
                .iter()
                .map(|(n, _)| json!({"motor":n,"flags":model::motor(data,n)["flags"]}))
                .collect::<Vec<_>>());
            if flags != last_flags {
                let any = flags.as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|r| r["flags"].as_array().is_some_and(|a| !a.is_empty()))
                });
                let text = driver_message(&flags);
                let warning = text.is_some();
                if let Some(text) = &text {
                    crate::notify::send(text);
                }
                if any || !last_flags.is_null() {
                    add_pending(
                        &mut pending,
                        vec![Event::new(
                            captured,
                            if warning {
                                "driver_warning"
                            } else {
                                "driver_status"
                            },
                            text.clone().unwrap_or_else(|| if any {
                                format!("Driver open-load flag only (usually harmless at standstill or slow moves): {flags}")
                            } else {
                                "Driver flags cleared".to_owned()
                            }),
                        )],
                    );
                }
                last_flags = flags;
            }
        }
        let connected = error.is_none()
            && (demo
                || (transport == "push" && attempt - last_ws < 20.0)
                || (transport == "poll" && attempt - last_poll_ok < 10.0));
        if !connected {
            last_webhooks = Value::Null;
        } else if let Some(w) = sample.as_ref().map(|d| &d["webhooks"]).filter(|w| w.is_object()) {
            last_webhooks = w.clone();
        }
        if error.is_some() {
            link_error = error.clone();
        } else if connected {
            link_error = None;
        }
        let (printer_state, printer_message) = if demo {
            ("ready", None)
        } else {
            model::printer_link(connected, link_error.as_deref(), &last_webhooks)
        };
        crate::notify::on_state(
            printer_state,
            sample.as_ref().and_then(|d| d["print_stats"]["state"].as_str()),
            printer_message.as_deref(),
        );
        let health=json!({"connected":connected,"transport":transport,"last_error":error,"degraded":degraded,"mode":if demo {"demo"}else{"live"}}).to_string();
        if health != last_health {
            add_pending(
                &mut pending,
                vec![
                    Event::new(captured, "scope_health", &health),
                    Event::new(
                        captured,
                        if connected {
                            "connection"
                        } else {
                            "printer_offline"
                        },
                        error.clone().unwrap_or_else(|| {
                            if degraded.is_empty() {
                                format!("Printer data is being recorded via {transport}")
                            } else {
                                format!(
                                    "Recording via {transport} with limitations: {}",
                                    degraded.join(" | ")
                                )
                            }
                        }),
                    ),
                ],
            );
            last_health = health;
        }
        if pending.len() > 1024 {
            pending.truncate(1024);
            lost_backlog = true;
        }
        let events_written = !pending.is_empty();
        // L8: the same console lines that are about to be stored, ready to push to any open
        // stream. Captured before the write because a successful write clears `pending`.
        let console_lines: Vec<Value> = pending
            .iter()
            .map(|e| serde_json::json!({"ts":e.ts,"kind":e.kind,"message":e.message}))
            .collect();
        let wrote = if sample.is_some() || !pending.is_empty() {
            db.write(
                sample.as_ref().map(|v| (captured, v)),
                &pending,
                &mut parser,
            )
            .map(Some)
        } else {
            Ok(None)
        };
        // Probe points land in their own additive table the moment the console shows them.
        if !pending_probe.is_empty() {
            match db.write_probe_points(&pending_probe) {
                Ok(n) => {
                    let _ = n;
                    pending_probe.clear();
                }
                Err(e) => degraded.push(format!("Probe points could not be stored: {e}")),
            }
        }
        // Print context: once per observed print start, two read-only GETs (metadata + gcode
        // tail) captured into the additive print_context table.
        if !demo {
            if let Some(data) = &sample {
                let filename = data["print_stats"]["filename"].as_str().unwrap_or("");
                let pstate = data["print_stats"]["state"].as_str().unwrap_or("");
                if pstate == "printing" && !filename.is_empty() {
                    let duration = data["print_stats"]["total_duration"].as_f64();
                    if new_print(&last_print, filename, duration) { context_captured = false; context_retry = Instant::now(); context_error = None; }
                    if !context_captured && Instant::now() >= context_retry {
                        match source.print_context(filename).and_then(|(meta, orca)| db.write_print_context(captured, filename, &meta, &orca)) {
                            Ok(()) => { context_captured = true; context_error = None; }
                            Err(e) => { context_retry = Instant::now() + Duration::from_secs(30); context_error = Some(e); }
                        }
                    }
                    if let Some(e) = &context_error { degraded.push(format!("Print context unavailable: {e}")); }
                    last_print = Some((filename.to_string(), duration));
                } else if matches!(pstate, "standby" | "complete" | "cancelled" | "error") {
                    last_print = None;
                    context_captured = false;
                    context_error = None;
                }
            }
        }
        let mut published_tick: Option<Value> = None;
        let mut console_written = false;
        // The outcome notice is prepared under the lock and sent after it, so the state lock
        // is never held across a process spawn.
        let mut notice: Option<(&'static str, String, bool)> = None;
        {
            let mut s = state.lock().map_err(|_| {
                "collector status lock failed; restart the service and retain the recording"
                    .to_owned()
            })?;
            s.connected = connected;
            s.printer_state = printer_state.to_owned();
            s.printer_message = printer_message;
            s.klipper_state = last_webhooks["state"].as_str().unwrap_or("").to_owned();
            s.transport = transport.clone();
            s.last_attempt = Some(attempt);
            s.last_error = error.clone();
            s.degraded = degraded;
            s.query_ms_last = query_ms_last;
            s.query_ms_mean = query_ms_ema.unwrap_or(0.0);
            s.push_frame_age_s = push_frame_age_s;
            s.push_keepalive_ms = push_keepalive_ms;
            if error.is_some() {
                s.errors += 1;
            }
            match wrote {
                Ok(Some(w)) => {
                    console_written = true;
                    pending.clear();
                    last_disk_error.clear();
                    s.samples += w.samples;
                    s.events += w.events;
                    s.qgl_rounds += w.rounds;
                    if let Some(session) = &w.outcome {
                        notice = Some(print_notice(session));
                    }
                    if w.ts.is_some() {
                        s.last_success = w.ts;
                    }
                    writes += 1;
                    write_total += w.ms;
                    s.last_write_ms = w.ms;
                    s.mean_write_ms = write_total / writes as f64;
                    if w.samples > 0 {
                        update_times.push_back(captured);
                        while update_times.front().is_some_and(|t| captured - t > 60.0) {
                            update_times.pop_front();
                        }
                        s.updates_per_s_1m = update_times.len() as f64 / 60.0;
                        if let Some(data) = &sample {
                            // Publish the newest sample as the cached tick payload + SSE push.
                            let stale_activity = activity_cache
                                .as_ref()
                                .is_none_or(|(t, _)| captured - t > 5.0);
                            if stale_activity || events_written {
                                activity_cache =
                                    Some((captured, workspace::activity_for(&db, captured, data)));
                            }
                            let activity = activity_cache
                                .as_ref()
                                .map(|(_, v)| v.clone())
                                .unwrap_or(Value::Null);
                            let payload = workspace::tick_payload(
                                captured,
                                Some(data),
                                workspace::job_for(data),
                                activity,
                                data["toolhead"]["homed_axes"].as_str(),
                                &probe_cache,
                                &cfg,
                                &s,
                            );
                            s.last_tick = Some(payload.clone());
                            s.last_tick_ts = Some(captured);
                            published_tick = Some(payload);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    if last_disk_error != e {
                        eprintln!("{e}");
                        last_disk_error = e.clone();
                    }
                    s.last_error = Some(e.clone());
                    s.degraded.push(format!("New data is not being saved. {e}"));
                    s.errors += 1;
                    last_health.clear();
                }
            }
        }
        // Desktop notices. The launcher's watcher keeps this service alive through a print and
        // for 15 minutes after, so these land even when the Cockpit window is closed. One notice
        // per recorded session row (the row is written exactly once) and one per distinct alarm
        // timestamp; a missing notify-send degrades silently by design. WhatsApp (src/notify.rs)
        // hears the same two moments, whatever the desktop setting.
        if let Some((summary, body, _)) = &notice {
            crate::notify::send(&format!("Stella: {summary}. {body}"));
        }
        if fresh_alarm.is_some_and(|ts| ts > last_alarm_notice && ts >= started) {
            crate::notify::send(&format!("Stella: BLTouch alarm. {PROBE_ALARM_ADVICE}"));
        }
        if cfg.desktop_notifications {
            if let Some((summary, body, urgent)) = notice {
                desktop_notify(summary, &body, urgent);
            }
            if let Some(ts) =
                fresh_alarm.filter(|ts| should_notify_alarm(*ts, last_alarm_notice, started))
            {
                last_alarm_notice = ts;
                desktop_notify("BLTouch alarm", PROBE_ALARM_ADVICE, true);
            }
        }
        if let Some(payload) = published_tick {
            crate::server::sse_broadcast(&payload.to_string());
        }
        // Console lines only leave here once they are safely in the recording, so a line a reader
        // saw is a line /api/events can show again.
        if console_written && !console_lines.is_empty() {
            crate::server::sse_broadcast_event(
                "console",
                &serde_json::json!({"lines":console_lines}).to_string(),
            );
        }
        if last_prune.elapsed() >= Duration::from_secs(cfg.prune_interval_s) {
            let active = last_print.as_ref().map(|(filename, _)| filename.as_str());
            match db
                .prune(model::now() - cfg.keep_days as f64 * 86400.0, cfg.max_db_mb, active)
                .and_then(|_| db.counts())
            {
                Ok((a, b, c)) => {
                    maintenance_error = None;
                    let mut s = state
                        .lock()
                        .map_err(|_| "status lock failed; restart the service".to_owned())?;
                    s.samples = a;
                    s.events = b;
                    s.qgl_rounds = c;
                }
                Err(e) => maintenance_error = Some(format!("Retention cleanup failed. {e}")),
            }
            last_prune = Instant::now();
        }
        // Pacing: push mode waits on the channel (50 ms recv timeout is the heartbeat); poll
        // mode and demo keep their interval, counting overruns as before.
        if transport != "push" && !demo {
            let interval = Duration::from_millis(if connected {
                cfg.poll_interval_ms
            } else {
                cfg.offline_retry_ms
            });
            if let Some(rest) = interval.checked_sub(tick.elapsed()) {
                thread::sleep(rest);
            } else {
                if let Ok(mut s) = state.lock() {
                    s.poll_overruns += 1;
                }
                thread::sleep(Duration::from_millis(1));
            }
        } else if demo {
            let interval = Duration::from_millis(cfg.poll_interval_ms.min(250));
            if let Some(rest) = interval.checked_sub(tick.elapsed()) {
                thread::sleep(rest);
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_filename_new_duration_needs_new_context() {
        let last = Some(("cube.gcode".to_owned(), Some(120.0)));
        assert!(!super::new_print(&last, "cube.gcode", Some(121.0)));
        assert!(super::new_print(&last, "cube.gcode", Some(1.0)));
        assert!(super::new_print(&None, "cube.gcode", Some(1.0)));
    }
    use super::*;
    use crate::store::PrintSession;
    #[test]
    fn resonance_evidence_is_tagged() {
        assert!(is_resonance("SHAPER_CALIBRATE frequency=..."));
        assert!(is_resonance("accelerometer_query: [-3426.299, ...]"));
        assert!(is_resonance("Fitted shaper 'zv' frequency=57.4"));
        assert!(!is_resonance("Print started"));
    }
    /// The alarm detector matches the Klipper strings recorded on 2026-09-21 and nothing else.
    /// Probe landings and BLTOUCH_DEBUG commands are normal console lines. the recorded ";
    /// retrying." line is Klipper's recoverable retry, so the assertion that it alarms is
    /// reversed here (the one changed assertion, by the WO's order).
    #[test]
    fn probe_alarm_matches_only_the_recorded_signature() {
        assert!(is_probe_alarm("!! BLTouch failed to deploy"));
        assert!(!is_probe_alarm(
            "// Failed to verify BLTouch probe is raised; retrying."
        ));
        assert!(!is_probe_alarm(
            "// probe: at 255.000,190.002 bed will contact at z=0.088750"
        ));
        assert!(!is_probe_alarm("BLTOUCH_DEBUG COMMAND=pin_up"));
        assert!(!is_probe_alarm("Sending BLTOUCH_DEBUG COMMAND=reset"));
        assert!(!is_probe_alarm("Print started"));
    }
    /// Hard failures alarm, retries do not, and landings/debug lines are neither. The first two
    /// strings are the exact lines in the live recording of 2026-09-21 11:19:52-54; the rest are
    /// Klipper bltouch.py's respond_info (retry) and command_error (failure) texts.
    #[test]
    fn probe_retries_are_recorded_but_only_hard_failures_alarm() {
        let retry = "// Failed to verify BLTouch probe is raised; retrying.";
        assert!(is_probe_retry(retry) && !is_probe_alarm(retry));
        for hard in [
            "!! BLTouch failed to deploy",
            "BLTouch failed to raise probe",
            "!! BLTouch failed to raise probe",
            "BLTouch failed to verify sensor state",
            "!! BLTouch failed to verify sensor state",
        ] {
            assert!(is_probe_alarm(hard), "{hard}");
            assert!(!is_probe_retry(hard), "{hard}");
        }
        let sensor_retry = "// BLTouch failed to verify sensor state; retrying.";
        assert!(is_probe_retry(sensor_retry) && !is_probe_alarm(sensor_retry));
        // Recorded neighbours of the alarm in the live DB: landings, the z_offset report, debug.
        for neither in [
            "// probe: at 27.050,24.282 bed will contact at z=3.102750",
            "// bltouch: z_offset: 1.988\n// The SAVE_CONFIG command will update the printer config file\n// with the above and restart the printer.",
            "BLTOUCH_DEBUG COMMAND=pin_up",
            "// Sending BLTOUCH_DEBUG COMMAND=reset",
            "Print started",
        ] {
            assert!(
                !is_probe_alarm(neither) && !is_probe_retry(neither),
                "{neither}"
            );
        }
    }
    /// What the poll does with those lines. A retry is tagged `probe_retry` with no advisory and no
    /// notice timestamp; a hard failure is tagged `probe_alarm` with exactly one advisory (which no
    /// longer calls the raise check a failure) and its timestamp for the notice.
    #[test]
    fn a_retry_line_gets_no_advisory_and_no_notice() {
        // The live recording of 2026-09-21 11:19:52-54, timestamps and text as stored.
        let retry = || {
            Event::new(
                1790000392.518,
                "response",
                "// Failed to verify BLTouch probe is raised; retrying.",
            )
        };
        let (events, advisories, alarm) = tag_console(vec![retry()]);
        assert_eq!(events[0].kind, "probe_retry");
        assert!(advisories.is_empty());
        assert_eq!(alarm, None);
        let (events, advisories, alarm) = tag_console(vec![
            retry(),
            Event::new(1790000394.647, "response", "!! BLTouch failed to deploy"),
            Event::new(
                1790001151.687,
                "response",
                "// Sending BLTOUCH_DEBUG COMMAND=reset",
            ),
        ]);
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["probe_retry", "probe_alarm", "response"]);
        assert_eq!(advisories.len(), 1);
        assert_eq!(advisories[0].ts, 1790000394.647);
        assert!(
            !advisories[0].message.contains("verify it is raised"),
            "{}",
            advisories[0].message
        );
        assert_eq!(alarm, Some(1790000394.647));
    }
    /// The notice gate. An alarm older than the service start is recorded but never
    /// notified; a new one notifies once, however many polls still carry it.
    #[test]
    fn alarm_notices_skip_alarms_from_before_the_start_and_fire_once() {
        let started = 1790100000.0;
        assert!(!should_notify_alarm(started - 60.0, 0.0, started));
        assert!(!should_notify_alarm(started - 60.0, started, started));
        let mut last = started;
        let mut sent = 0;
        for _poll in 0..3 {
            let ts = started + 12.5;
            if should_notify_alarm(ts, last, started) {
                last = ts;
                sent += 1;
            }
        }
        assert_eq!(sent, 1, "the same alarm must notify once");
        assert!(should_notify_alarm(started + 40.0, last, started));
    }
    /// The notice text carries the outcome the user can act on. Duration is minutes when the
    /// machine reported one, honest unknown when it did not.
    #[test]
    fn print_notice_names_the_outcome_and_duration() {
        let done = PrintSession {
            start_ts: 10.0,
            end_ts: 100.0,
            filename: "gcodes/cube.gcode".into(),
            duration_s: Some(730.0),
            filament_used: Some(1.88),
            result: "complete".into(),
            progress: Some(1.0),
            evidence: String::new(),
        };
        let (summary, body, urgent) = print_notice(&done);
        assert_eq!(summary, "Print finished");
        assert_eq!(body, "cube.gcode finished after 12 min.");
        assert!(!urgent);
        let fault = PrintSession {
            result: "error".into(),
            ..done.clone()
        };
        let (summary, _, urgent) = print_notice(&fault);
        assert_eq!(summary, "Print fault");
        assert!(urgent);
        let unknown = PrintSession {
            duration_s: None,
            result: "cancelled".into(),
            ..done
        };
        let (summary, body, _) = print_notice(&unknown);
        assert_eq!(summary, "Print cancelled");
        assert_eq!(body, "cube.gcode was cancelled.");
    }
}

/// WhatsApp words for a driver warning. Only motors with a flag, by name, in plain words. Open-load
/// alone (ola/olb) is not sent: TMC2209s report it falsely at standstill and slow moves, and Klipper
/// does not treat it as a fault.
pub fn driver_message(flags: &serde_json::Value) -> Option<String> {
    let word = |f: &str| -> String { match f {
        "ola" => "open load on coil A",
        "olb" => "open load on coil B",
        "s2ga" | "s2gb" => "short to ground",
        "s2vsa" | "s2vsb" => "short to supply",
        "ot" => "over temperature, shut down",
        "otpw" => "over temperature warning",
        "t120" | "t143" | "t150" | "t157" => "hot",
        "uv_cp" => "charge pump undervoltage",
        "drv_err" => "driver error",
        other => other,
    }.to_owned() };
    let names: std::collections::HashMap<&str, &str> = crate::model::MOTORS.iter().copied().collect();
    let mut lines = Vec::new();
    for row in flags.as_array()? {
        let list: Vec<String> = row["flags"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_lowercase)).collect()).unwrap_or_default();
        if list.is_empty() || list.iter().all(|f| f == "ola" || f == "olb") {
            continue;
        }
        let motor = row["motor"].as_str().unwrap_or("");
        let what: Vec<String> = list.iter().map(|f| word(f)).collect();
        lines.push(format!("{} ({motor}): {}", names.get(motor).copied().unwrap_or(motor), what.join(", ")));
    }
    (!lines.is_empty()).then(|| format!("Stella: stepper driver warning. {}. Check that motor and its connector.", lines.join("; ")))
}

#[cfg(test)]
mod driver_message_tests {
    #[test]
    fn plain_words_only_for_flagged_motors_and_open_load_alone_is_quiet() {
        let quiet = serde_json::json!([{"motor":"stepper_x","flags":[]},{"motor":"stepper_z","flags":["olb"]}]);
        assert_eq!(super::driver_message(&quiet), None);
        let loud = serde_json::json!([{"motor":"stepper_x","flags":[]},{"motor":"stepper_z1","flags":["otpw","olb"]}]);
        let text = super::driver_message(&loud).unwrap();
        assert!(text.contains("Back left (stepper_z1): over temperature warning, open load on coil B"), "{text}");
        assert!(!text.contains("stepper_x"));
    }
}
