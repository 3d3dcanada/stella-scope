// Moonraker push source: printer.objects.subscribe over the websocket, status deltas merged
// into the full object map (same shape the HTTP poll produced), reconnect forever. GET-only
// philosophy kept: subscribing requests data; it sends no printer command.
use crate::{config::Config, ws::WsClient, Result};
use serde_json::{json, Value};
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

/// Idle-resample cadence: the recorder re-sends the merged status at 2 Hz even when
/// the printer reports no changes, so the recording keeps its cadence without traffic.
const RESAMPLE: Duration = Duration::from_millis(500);

pub enum WsEvent {
    /// (Re)connected and subscribed; the collector refetches configfile once.
    Connected,
    /// A full merged status map, ready to store exactly like a polled sample.
    Sample(Value),
    /// Klippy restarted (shutdown/ready notification); the collector refetches configfile.
    KlippyRestart,
    /// Push-link health, measured on the periodic `server.info` keepalive: how old the
    /// previous inbound frame was when the response arrived, and the round trip in ms.
    Latency { frame_age_s: f64, keepalive_ms: f64 },
}

/// One decoded JSON-RPC frame, classified for the connection loop. Pure so the routing
/// is testable without a live websocket.
pub enum Inbound {
    /// A status delta to merge; carries the params[0] object.
    Status(Value),
    /// notify_klippy_shutdown / _ready / _disconnected.
    KlippyRestart,
    /// The subscribe response; carries the initial full status object (or Null if unusable).
    SubscribeResult(Value),
    /// Moonraker rejected the subscription; carries the message.
    SubscribeReject(String),
    /// The response to a `server.info` keepalive (id 99).
    KeepaliveResponse,
    /// Anything else: proc-stat pushes, file-list notifications, unsolicited pongs.
    Other,
}

pub fn classify(v: &Value) -> Inbound {
    if v["id"] == json!(1) {
        if !v["error"].is_null() {
            return Inbound::SubscribeReject(
                v["error"]["message"].as_str().unwrap_or("?").to_owned(),
            );
        }
        return Inbound::SubscribeResult(v["result"]["status"].clone());
    }
    if v["id"] == json!(99) {
        return Inbound::KeepaliveResponse;
    }
    match v["method"].as_str().unwrap_or("") {
        "notify_status_update" => Inbound::Status(v["params"][0].clone()),
        "notify_klippy_shutdown" | "notify_klippy_ready" | "notify_klippy_disconnected" => {
            Inbound::KlippyRestart
        }
        _ => Inbound::Other,
    }
}

pub fn host_port(cfg: &Config) -> Result<(String, u16)> {
    let rest = cfg.moonraker_url.trim_end_matches('/');
    let rest = rest
        .strip_prefix("http://")
        .ok_or("moonraker_url must be http:// for the websocket transport")?;
    match rest.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            Ok((
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| "invalid port in moonraker_url".to_owned())?,
            ))
        }
        _ => Ok((rest.to_string(), 80)),
    }
}

/// Live objects for the subscription: everything configured except configfile, which the
/// collector fetches once per connect/restart/SAVE_CONFIG instead of every sample.
pub fn subscribe_objects(cfg: &Config) -> Vec<String> {
    cfg.objects
        .iter()
        .filter(|o| !o.split('=').next().unwrap_or(o).eq("configfile"))
        .map(|o| o.split('=').next().unwrap_or(o).to_string())
        .collect()
}

// Klipper status deltas are one object -> field map deep. Merge that layer only; a non-object
// delta replaces the object wholesale.
fn merge_delta(full: &mut Value, delta: &Value) {
    let Some(dst) = full.as_object_mut() else {
        return;
    };
    let Some(src) = delta.as_object() else { return };
    for (k, v) in src {
        match dst.get_mut(k) {
            Some(Value::Object(existing)) if v.is_object() => {
                if let Some(vm) = v.as_object() {
                    for (fk, fv) in vm {
                        existing.insert(fk.clone(), fv.clone());
                    }
                }
            }
            _ => {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
}

// L4 `reconnect`: the operator asks for a fresh subscription without restarting the service.
// The flag is read at the top of every read tick, so the drop happens within one RESAMPLE
// (500 ms) rather than waiting out the 30 s idle timeout.
static RECONNECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Set once `spawn` has started the reader thread. In poll mode nothing ever spawns, and a
/// reconnect request there has nothing to act on — so it is reported as refused, not as done.
static SOURCE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the push source to drop its connection. Returns false when there is no push source,
/// which is the caller's cue to say so rather than to claim a reconnect happened.
pub fn request_reconnect() -> bool {
    if !SOURCE_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    RECONNECT.store(true, std::sync::atomic::Ordering::Relaxed);
    true
}

/// True once per request, so one press drops one connection.
fn reconnect_requested() -> bool {
    RECONNECT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

pub fn spawn(cfg: &Config) -> Receiver<WsEvent> {
    // Bounded (round-4 item 4): a wedged collector cannot grow this queue forever —
    // backpressure reaches Moonraker's TCP buffers instead of process memory.
    let (tx, rx) = std::sync::mpsc::sync_channel::<WsEvent>(1024);
    let cfg = cfg.clone();
    SOURCE_RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        // Reconnect forever; the safe failure direction is the collector's HTTP poll fallback.
        loop {
            match run_connection(&cfg, &tx) {
                // Ok is only ever returned for an operator-requested reconnect: go straight back
                // round without the 5 s error backoff, or the button appears not to work.
                Ok(()) => continue,
                Err(e) => {
                    let _ = tx.send(WsEvent::Sample(Value::Null)); // probe: let the collector see silence
                    eprintln!("websocket push source: {e}"); // observable, one line per 5 s retry
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    rx
}

fn run_connection(cfg: &Config, tx: &SyncSender<WsEvent>) -> Result<()> {
    let (host, port) = host_port(cfg)?;
    let mut client = WsClient::connect(
        &host,
        port,
        "/websocket",
        Duration::from_millis(cfg.request_timeout_ms.max(2000)),
    )?;
    // Moonraker v0.10 takes the subscription as a plain dict param: {"objects": {name: fields}}.
    // A null value means every field of that object. Confirmed live against the test printer
    // 2026-09-17.
    let mut sub = serde_json::Map::new();
    for name in subscribe_objects(cfg) {
        sub.insert(name, Value::Null);
    }
    client.send_text(
        &json!({"jsonrpc":"2.0","method":"printer.objects.subscribe","params":{"objects":Value::Object(sub)},"id":1}).to_string(),
    )?;
    let mut full = Value::Null;
    let mut last_rx = Instant::now();
    let mut last_ping = Instant::now();
    let mut last_send = Instant::now();
    let mut keepalive_sent: Option<Instant> = None;
    // The read deadline IS the idle-resample tick: the 500 ms cadence runs on wall
    // clock regardless of inbound frames, pings or their timing (round-4 item 1
    // completion + item 4: partial frames stay buffered across quiet timeouts).
    let mut next_tick = last_send + RESAMPLE;
    loop {
        if reconnect_requested() {
            return Ok(());
        }
        match client.read_text_until(next_tick) {
            Ok(text) => {
                // Age of the newest frame before this one — published alongside the keepalive
                // round trip as the push-mode freshness figure.
                let frame_age_s = last_rx.elapsed().as_secs_f64();
                last_rx = Instant::now();
                // last_send is NOT reset here: Moonraker pushes notify_proc_stat_update at
                // 1 Hz even while idle, and resetting the idle-resample timer on those frames
                // degraded the 2 Hz cadence to exactly 1 sample/s. The timer restarts only
                // when a sample is actually pushed below (status update, resample, subscribe).
                let v: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match classify(&v) {
                    Inbound::Status(delta) => {
                        if !delta.is_null() && full.is_object() {
                            merge_delta(&mut full, &delta);
                            let _ = tx.send(WsEvent::Sample(full.clone()));
                            last_send = Instant::now();
                        }
                    }
                    Inbound::KlippyRestart => {
                        let _ = tx.send(WsEvent::KlippyRestart);
                    }
                    Inbound::SubscribeReject(message) => {
                        return Err(format!("Moonraker rejected the subscription: {message}"));
                    }
                    Inbound::SubscribeResult(status) => {
                        if status.is_object() && !status.as_object().unwrap().is_empty() {
                            full = status;
                            let _ = tx.send(WsEvent::Connected);
                            let _ = tx.send(WsEvent::Sample(full.clone()));
                            last_send = Instant::now();
                        }
                    }
                    Inbound::KeepaliveResponse => {
                        if let Some(sent) = keepalive_sent.take() {
                            let _ = tx.send(WsEvent::Latency {
                                frame_age_s,
                                keepalive_ms: sent.elapsed().as_secs_f64() * 1000.0,
                            });
                        }
                    }
                    Inbound::Other => {}
                }
            }
            Err(e) => {
                // Read timeouts land here as "timed out" — the keepalive case, not an error.
                let quiet = e.contains("timed out");
                if !quiet {
                    return Err(format!("websocket stream: {e}"));
                }
                // Idle printer: status pushes only arrive on change. Keep the link alive with a
                // cheap JSON-RPC ping and drop the connection if nothing answers for 30 s.
                if last_rx.elapsed() > Duration::from_secs(30) {
                    return Err("no websocket traffic for 30 s; reconnecting".into());
                }
                if last_ping.elapsed() >= Duration::from_secs(10) {
                    client.send_text(
                        &json!({"jsonrpc":"2.0","method":"server.info","params":{},"id":99})
                            .to_string(),
                    )?;
                    keepalive_sent = Some(Instant::now());
                    last_ping = Instant::now();
                }
            }
        }
        // Idle continuity on wall clock: if nothing changed for 500 ms, resend the
        // full merged status so the recording keeps its 2 Hz cadence with zero
        // network traffic per sample. The read deadline above is this same tick,
        // so no inbound traffic pattern can stretch it.
        if full.is_object() && last_send.elapsed() >= RESAMPLE {
            let _ = tx.send(WsEvent::Sample(full.clone()));
            last_send = Instant::now();
        }
        next_tick = last_send + RESAMPLE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_port_parses_plain_host_and_port() {
        let cfg = Config::default();
        assert_eq!(host_port(&cfg).unwrap(), ("127.0.0.1".into(), 7125));
    }
    #[test]
    fn subscribe_objects_drop_configfile_and_field_filters() {
        let mut cfg = Config::default();
        cfg.objects = vec![
            "motion_report".into(),
            "configfile=settings".into(),
            "tmc2209 stepper_z".into(),
        ];
        assert_eq!(
            subscribe_objects(&cfg),
            vec!["motion_report".to_string(), "tmc2209 stepper_z".to_string()]
        );
    }
    #[test]
    fn deltas_merge_one_level_and_replace_scalars() {
        let mut full = json!({"toolhead":{"position":[1,2,3],"homed_axes":"xyz"},"extruder":{"temperature":200.0}});
        merge_delta(
            &mut full,
            &json!({"toolhead":{"position":[4,5,6]},"extruder":{"temperature":210.0}}),
        );
        assert_eq!(full["toolhead"]["position"], json!([4, 5, 6]));
        assert_eq!(
            full["toolhead"]["homed_axes"], "xyz",
            "untouched fields survive"
        );
        assert_eq!(full["extruder"]["temperature"], 210.0);
        merge_delta(&mut full, &json!({"toolhead":{"position":null}}));
        assert!(full["toolhead"]["position"].is_null());
    }
    #[test]
    fn classify_routes_every_frame_kind() {
        assert!(matches!(
            classify(
                &json!({"jsonrpc":"2.0","method":"notify_status_update","params":[{"toolhead":{"position":[1,2,3]}},1.0]})
            ),
            Inbound::Status(_)
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","method":"notify_klippy_ready","params":[]})),
            Inbound::KlippyRestart
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","id":1,"result":{"status":{"toolhead":{}}}})),
            Inbound::SubscribeResult(_)
        ));
        // Empty status still classifies as a result; the loop guards against using it.
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","id":1,"result":{"status":{}}})),
            Inbound::SubscribeResult(_)
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","id":1,"error":{"message":"bad object"}})),
            Inbound::SubscribeReject(m) if m == "bad object"
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","id":99,"result":{}})),
            Inbound::KeepaliveResponse
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","method":"notify_proc_stat_update","params":[{}]})),
            Inbound::Other
        ));
    }
}
