use serde::Serialize;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MOTORS: [(&str, &str); 7] = [
    ("stepper_x", "X motor"),
    ("stepper_y", "Y motor"),
    ("stepper_z", "Front left"),
    ("stepper_z1", "Back left"),
    ("stepper_z2", "Back right"),
    ("stepper_z3", "Front right"),
    ("extruder", "Extruder"),
];

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Clone, Default, Serialize)]
pub struct CollectorStatus {
    pub mode: String,
    pub connected: bool,
    /// Active sampling transport: "push" (websocket subscribe), "poll" (HTTP fallback) or "" before start.
    pub transport: String,
    pub updates_per_s_1m: f64,
    pub query_ms_last: Option<f64>,
    pub query_ms_mean: f64,
    /// Push-mode freshness, measured by the push source on each server.info keepalive: how old
    /// the previous inbound frame was (s) and the keepalive round trip (ms). None in poll mode
    /// and until the first keepalive completes.
    pub push_frame_age_s: Option<f64>,
    pub push_keepalive_ms: Option<f64>,
    pub last_error: Option<String>,
    pub last_attempt: Option<f64>,
    pub last_success: Option<f64>,
    pub degraded: Vec<String>,
    pub samples: u64,
    pub events: u64,
    pub qgl_rounds: u64,
    pub errors: u64,
    pub last_write_ms: f64,
    pub mean_write_ms: f64,
    pub poll_overruns: u64,
    /// Cached /api/tick payload for the newest written sample; kept fresh by the collector so
    /// SSE and tick requests never replay deltas for every reader. Read-only for the server.
    pub last_tick: Option<Value>,
    pub last_tick_ts: Option<f64>,
    /// What the printer can do now, read off the link this collector already keeps (no extra poll):
    /// "ready", "not_ready" (Moonraker answers, Klipper does not) or "unreachable" (Moonraker does
    /// not answer: the printer is off or off the network). Empty before the first attempt and in
    /// replay.
    pub printer_state: String,
    pub printer_message: Option<String>,
    /// Klipper's own state word from the newest sample (ready, startup, error, and the halted one),
    /// empty while Moonraker does not answer. With Klipper halted or in error the HUD unlocks the two
    /// restarts, the way out (reviewer's decision on 2026-09-23).
    pub klipper_state: String,
}

/// The printer state from the collector's own link. `error` is the newest failed attempt's error,
/// kept until a success; `webhooks` is Klipper's own state object from the newest sample.
pub fn printer_link(
    connected: bool,
    error: Option<&str>,
    webhooks: &Value,
) -> (&'static str, Option<String>) {
    if !connected {
        return match error {
            // The header needs only what Moonraker said: the collector's "Printer not answering."
            // prefix and the client's advice after the ';' (meant for a wrong URL) are dropped.
            Some(e) if !e.contains(crate::moonraker::UNREACHABLE) => {
                let said = e.trim_start_matches("Printer not answering.").split(';').next().unwrap_or(e).trim();
                ("not_ready", Some(said.to_owned()))
            }
            other => ("unreachable", other.map(str::to_owned)),
        };
    }
    match webhooks["state"].as_str() {
        Some(state) if state != "ready" => {
            // Klipper's state_message is several lines: what is wrong first, then advice and a closing
            // line ("Printer is shutdown"). The header and tooltips keep the first line, which is the
            // cause, and say the state after it (2026-09-23).
            let first = webhooks["state_message"]
                .as_str()
                .and_then(|m| m.lines().map(str::trim).find(|l| !l.is_empty()))
                .map(|l| l.trim_end_matches('.'));
            ("not_ready", Some(match first {
                Some(m) => format!("{m} (Klipper state: {state})"),
                None => format!("Klipper state: {state}"),
            }))
        }
        _ => ("ready", None),
    }
}

pub fn bool_value(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| v.as_i64().map(|n| n != 0))
}

pub fn motor(data: &Value, name: &str) -> Value {
    let key = format!("tmc2209 {name}");
    let driver = &data[&key];
    let status = driver["drv_status"].as_object();
    let field = |key: &str| status.map(|s| s.get(key).and_then(Value::as_u64).unwrap_or(0));
    let flags = status.map(|_| {
        [
            "otpw", "ot", "s2vsa", "s2vsb", "s2ga", "s2gb", "ola", "olb", "t120", "t143", "t150",
            "t157",
        ]
        .iter()
        .filter(|k| field(k).unwrap_or(0) != 0)
        .copied()
        .collect::<Vec<_>>()
    });
    json!({"cur": driver["run_current"].as_f64(), "hold": driver["hold_current"].as_f64(),
        "cs": field("cs_actual"), "stst": field("stst").map(|v| v != 0),
        "stealth": field("stealth").map(|v| v != 0), "flags": flags,
        "energised": bool_value(&data["stepper_enable"]["steppers"][name])})
}

pub fn point(ts: f64, data: &Value) -> Value {
    let mut motors = serde_json::Map::new();
    for (name, _) in MOTORS {
        motors.insert(name.to_owned(), motor(data, name));
    }
    let p = &data["motion_report"]["live_position"];
    json!({"ts": ts, "x": p[0].as_f64(), "y": p[1].as_f64(), "z": p[2].as_f64(),
        "velocity": data["motion_report"]["live_velocity"].as_f64(),
        "nozzle": data["extruder"]["temperature"].as_f64(),
        "nozzle_target": data["extruder"]["target"].as_f64(),
        "bed": data["heater_bed"]["temperature"].as_f64(),
        "bed_target": data["heater_bed"]["target"].as_f64(),
        "fan": data["fan"]["speed"].as_f64(), "motors": motors})
}

pub fn geometry(cfg: &crate::config::Config) -> Value {
    json!({"bed_size_mm": cfg.bed_size_mm, "bed_size_confirmed":cfg.bed_size_confirmed,
        "probe_offset_mm": cfg.probe_offset_mm,"probe_offset_confirmed":cfg.probe_offset_confirmed,
        "corners": [
            {"name":"stepper_z", "label":"Front left", "xy":[0,0]},
            {"name":"stepper_z1", "label":"Back left", "xy":[0,1]},
            {"name":"stepper_z2", "label":"Back right", "xy":[1,1]},
            {"name":"stepper_z3", "label":"Front right", "xy":[1,0]}],
        "motors": MOTORS.iter().map(|(n,l)| json!({"name":n,"label":l})).collect::<Vec<_>>(),
        "poll_interval_ms":cfg.poll_interval_ms, "chart_window_s":cfg.chart_window_s,
        "qgl_tolerance_mm":cfg.qgl_tolerance_mm})
}

pub fn demo(t: f64, cfg: &crate::config::Config) -> Value {
    let x = cfg.bed_size_mm[0] * (0.5 + 0.38 * (t / 8.0).sin());
    let y = cfg.bed_size_mm[1] * (0.5 + 0.38 * (t / 11.0).cos());
    let z = 2.0 + (t / 180.0).rem_euclid(50.0);
    // A 9x9 synthetic mesh so the UI is exercised at the real probe count.
    let mesh_rows = (0..9)
        .map(|yi| {
            (0..9)
                .map(|xi| {
                    let fx = xi as f64 / 8.0;
                    let fy = yi as f64 / 8.0;
                    let centre = 1.0 - (2.0 * fy - 1.0).abs();
                    json!(
                        -0.459 + 0.13 * fx - 0.10 * centre * centre * centre
                            + 0.012 * (((xi * 7 + yi * 3) % 9) as f64) / 20.0
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut d = json!({
        "motion_report":{"live_position":[x,y,z,t/8.0],"live_velocity":55.0+35.0*(t/5.0).sin()},
        "toolhead":{"position":[x,y,z,t/8.0],"homed_axes":"xyz","max_velocity":300.0},
        "gcode_move":{"homing_origin":[0,0,-0.035],"speed_factor":1.0+0.2*(t/23.0).sin(),"extrude_factor":1.0,"speed":120.0},
        "idle_timeout":{"state":"Printing"},
        "extruder":{"temperature":210.0+1.8*(t/9.0).sin(),"target":210,"power":0.43},
        "heater_bed":{"temperature":60.0+0.5*(t/17.0).sin(),"target":60,"power":0.31},
        "fan":{"speed":0.65},"heater_fan hotend_fan":{"speed":1.0},
        "controller_fan controller_fan":{"speed":0.5},"probe":{"last_query":false},
        "quad_gantry_level":{"applied":t.rem_euclid(90.0)>24.0},
        "bed_mesh":{"profile_name":"Demo mesh 9x9","probed_matrix":mesh_rows,"mesh_min":[40,40],"mesh_max":[310,310]},
        "print_stats":{"state":"printing","filename":"DEMO - not a real print.gcode"},
        "virtual_sdcard":{"progress":t.rem_euclid(3600.0)/3600.0},
        "configfile":{"settings":{"printer":{"kinematics":"corexy"}}},
        "stepper_enable":{"steppers":{}}
    });
    for (i, (name, _)) in MOTORS.iter().enumerate() {
        let moving = !(2..6).contains(&i) || t.rem_euclid(90.0) < 24.0;
        let scale = if moving {
            12 + ((t / 2.0 + i as f64).sin().abs() * 17.0) as u64
        } else {
            8
        };
        let mut status = json!({"cs_actual":scale});
        if !moving {
            status["stst"] = json!(1);
        }
        if *name == "stepper_z1" && (30.0..34.0).contains(&t.rem_euclid(90.0)) {
            status["otpw"] = json!(1);
        }
        d[format!("tmc2209 {name}")] =
            json!({"run_current":if i<2 {1.0} else {0.8},"hold_current":0.5,"drv_status":status});
        d["stepper_enable"]["steppers"][*name] = json!(true);
    }
    d
}

#[cfg(test)]
mod printer_link_tests {
    use super::printer_link;
    use serde_json::{json, Value};

    /// The three states the header shows, from the link the collector keeps. The error texts are
    /// the shapes the collector really produces: `Printer not answering.` plus the Moonraker
    /// client's own message.
    #[test]
    fn printer_state_tells_off_from_klipper_not_ready_from_ready() {
        let off = "Printer not answering. cannot reach Moonraker at http://192.168.1.50:7125: io: Connection refused; is the printer on, and is this host allowed by Moonraker? For 401/403, set STELLA_MOONRAKER_API_KEY";
        assert_eq!(printer_link(false, Some(off), &Value::Null), ("unreachable", Some(off.to_owned())));
        // No error kept yet (a push link gone quiet before its first failed poll) reads as off.
        assert_eq!(printer_link(false, None, &Value::Null), ("unreachable", None));
        // Moonraker answers, but without printer data: Klipper is the one not ready, and the header
        // gets what Moonraker said without the prefix and the wrong-URL advice.
        let klippy = "Printer not answering. Moonraker returned HTTP 503 Service Unavailable; use the direct local HTTP endpoint, not a redirect";
        assert_eq!(
            printer_link(false, Some(klippy), &Value::Null),
            ("not_ready", Some("Moonraker returned HTTP 503 Service Unavailable".to_owned()))
        );
        // Connected, and Klipper reports it has stopped: not ready, in Klipper's own words. The
        // startup message is the one in the recording (15 samples, 2026-09-16 to 09-22); the
        // shutdown one is Klipper's cause line plus its standard FIRMWARE_RESTART advice.
        let startup = json!({"state":"startup","state_message":"\nPrinter is not ready\nThe klippy host software is attempting to connect.  Please\nretry in a few moments.\n"});
        assert_eq!(
            printer_link(true, None, &startup),
            ("not_ready", Some("Printer is not ready (Klipper state: startup)".to_owned()))
        );
        let stopped = json!({"state":"shutdown","state_message":"Lost communication with MCU 'EBBCan'\nOnce the underlying issue is corrected, use the\n\"FIRMWARE_RESTART\" command to reset the firmware, reload the\nconfig, and restart the host software.\nPrinter is shutdown\n"});
        assert_eq!(
            printer_link(true, None, &stopped),
            ("not_ready", Some("Lost communication with MCU 'EBBCan' (Klipper state: shutdown)".to_owned()))
        );
        assert_eq!(
            printer_link(true, None, &json!({"state":"startup","state_message":""})).1.as_deref(),
            Some("Klipper state: startup")
        );
        assert_eq!(
            printer_link(true, None, &json!({"state":"ready","state_message":"Printer is ready"})),
            ("ready", None)
        );
        // A recording without the webhooks object cannot say otherwise, so a live link is ready.
        assert_eq!(printer_link(true, None, &Value::Null), ("ready", None));
    }
}
