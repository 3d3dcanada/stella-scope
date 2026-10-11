const FLAGS: [&str; 12] = [
    "otpw", "ot", "s2vsa", "s2vsb", "s2ga", "s2gb", "ola", "olb", "t120", "t143", "t150", "t157",
];
use super::Response;
use crate::{
    config::Config,
    model::{self, CollectorStatus},
    store::{db_error, Store},
    Result,
};
use serde_json::{json, Value};

fn classified(object: &str, field: &str) -> (&'static str, &'static str) {
    if (object == "probe" || object == "query_endstops") && field.starts_with("last_query") {
        return ("cached query","Last explicit query result; sample capture time is not trigger time. No command is sent to refresh it.");
    }
    if object == "probe" && (field.starts_with("last_probe_position") || field == "last_z_result") {
        return (
            "last probe result",
            "Stored result of an earlier probing operation, not continuous contact height.",
        );
    }
    if object == "motion_report" {
        return (
            "requested motion",
            "Klipper interpolated requested motion; not encoder-confirmed physical position.",
        );
    }
    if field == "temperature" || field == "rpm" {
        return (
            "sensor reading",
            if field == "rpm" {
                "Only reported when tachometer feedback exists; fan command alone cannot prove rotation."
            } else {
                "Converted sensor reading; sensor type and input pin come from recorded configuration when available."
            },
        );
    }
    if [
        "target",
        "power",
        "speed",
        "value",
        "run_current",
        "hold_current",
        "enable",
        "angle",
    ]
    .contains(&field)
    {
        return ("command / setting","Requested setting or output duty, not measured current, voltage, power, torque or rotation.");
    }
    if field.starts_with("last_stats")
        || object.starts_with("canbus_stats")
        || object == "system_stats"
    {
        return ("reported statistics","Counters/statistics from software or controller; a nonzero cumulative counter alone does not prove an active fault.");
    }
    (
        "reported status",
        "Read from the selected sample. No additional physical measurement is inferred.",
    )
}
pub fn readable(name: &str) -> bool {
    crate::telemetry::allowed(name)
        || [
            "motion_report",
            "toolhead",
            "gcode_move",
            "extruder",
            "heater_bed",
            "fan",
            "probe",
            "quad_gantry_level",
            "idle_timeout",
            "print_stats",
            "virtual_sdcard",
            "bed_mesh",
        ]
        .contains(&name)
}
fn flatten(object: &str, path: &str, value: &Value, out: &mut Vec<Value>, depth: usize) {
    if out.len() >= 512 || depth > 4 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter().take(128) {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                flatten(object, &p, v, out, depth + 1);
            }
        }
        Value::Array(a) => {
            if a.len() <= 8 {
                for (i, v) in a.iter().enumerate() {
                    flatten(object, &format!("{path}.{i}"), v, out, depth + 1);
                }
            }
        }
        _ => {
            let val = if let Some(s) = value.as_str() {
                json!(s.chars().take(256).collect::<String>())
            } else {
                value.clone()
            };
            let (kind, meaning) = classified(object, path);
            out.push(json!({"id":format!("{object}/{path}"),"object":object,"field":path,"value":val,"kind":kind,"meaning":meaning}));
        }
    }
}
fn finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn fraction(v: &Value) -> Option<f64> {
    finite(v).filter(|n| (0.0..=1.0).contains(n))
}
pub fn projection(d: &Value, ts: f64) -> Value {
    let mut signals = Vec::new();
    let mut mcus = Vec::new();
    let mut pins = Vec::new();
    let mut fans = Vec::new();
    let mut sensors = Vec::new();
    if let Some(map) = d.as_object() {
        for (name, value) in map {
            if readable(name) && name != "bed_mesh" {
                flatten(name, "", value, &mut signals, 0);
            }
            if name == "mcu" || name.starts_with("mcu ") {
                mcus.push(json!({"object":name,"firmware":value["mcu_version"],"chip":value["mcu_constants"]["MCU"],"clock_hz":value["mcu_constants"]["CLOCK_FREQ"],"stats":value["last_stats"],"board_identity":"MCU chip is not a board model or PCB revision; record the label in Parts."}));
            }
            if name == "fan"
                || [
                    "fan_generic ",
                    "heater_fan ",
                    "controller_fan ",
                    "temperature_fan ",
                ]
                .iter()
                .any(|p| name.starts_with(p))
            {
                fans.push(json!({"object":name,"command":fraction(&value["speed"]),"rpm":finite(&value["rpm"]).filter(|n|*n>=0.0),"temperature":finite(&value["temperature"]),"target":finite(&value["target"])}));
            }
            if name.starts_with("temperature_sensor ") {
                sensors.push(json!({"object":name,"temperature":value["temperature"],"minimum":value["measured_min_temp"],"maximum":value["measured_max_temp"]}));
            }
        }
    }
    let settings = &d["configfile"]["settings"];
    if let Some(map) = settings.as_object() {
        for (section, opts) in map.iter().take(512) {
            if !readable(section) && !section.starts_with("stepper_") && section != "bltouch" {
                continue;
            }
            if let Some(opts) = opts.as_object() {
                for (key, v) in opts.iter().take(128) {
                    if ![
                        "pin",
                        "step_pin",
                        "dir_pin",
                        "enable_pin",
                        "sensor_pin",
                        "heater_pin",
                        "control_pin",
                        "tachometer_pin",
                        "uart_pin",
                        "tx_pin",
                        "rx_pin",
                        "diag_pin",
                        "endstop_pin",
                        "spi_software_mosi_pin",
                        "spi_software_miso_pin",
                        "spi_software_sclk_pin",
                        "cs_pin",
                    ]
                    .contains(&key.as_str())
                    {
                        continue;
                    }
                    if pins.len() >= 256 {
                        break;
                    }
                    if let Some(pin) = v.as_str().filter(|s| s.len() <= 160) {
                        let clean = pin.trim_start_matches(['!', '^', '~']);
                        let (mcu, pad) = clean.split_once(':').unwrap_or(("mcu", clean));
                        pins.push(json!({"component":section,"signal":key,"configured_pin":pin,"controller":mcu,"pad":pad,"inverted":pin.contains('!'),"pullup":pin.contains('^'),"pulldown":pin.contains('~'),"physical_connector":null,"basis":"Configured route at Klipper start/restart; not electrically verified. Aliases/virtual pins require resolution against the board and wiring."}));
                    }
                }
            }
        }
    }
    let bl = &settings["bltouch"];
    let probe = json!({"configured":bl.is_object(),"name":d["probe"]["name"],"sensor_pin":bl["sensor_pin"],"control_pin":bl["control_pin"],"x_offset":bl["x_offset"],"y_offset":bl["y_offset"],"z_offset":bl["z_offset"],"last_query":d["probe"]["last_query"],"last_probe_position":d["probe"]["last_probe_position"],"last_z_result_legacy":d["probe"]["last_z_result"],"pin_extension":null,"basis":"BLTouch section present in the printer configuration. Recorded configuration can differ across time. Last query is cached; pin extension is not observed by this feed."});
    // Configuration-only sections: Klipper exposes no status object for these three; surface their
    // recorded settings so the app shows real values instead of a wrong "not reported" warning.
    let config_sections: Vec<Value> = ["input_shaper", "adxl345", "safe_z_home"]
        .iter()
        .filter_map(|name| {
            let s = &d["configfile"]["settings"][name];
            if s.is_object() && !s.as_object().unwrap().is_empty() {
                Some(json!({"name":name,"fields":s}))
            } else {
                None
            }
        })
        .collect();
    json!({"sample_ts":ts,"signals":signals,"signal_limit":512,"limit_reached":signals.len()>=512,"controllers":mcus,"pins":pins,"pin_limit_reached":pins.len()>=256,"fans":fans,"sensors":sensors,"bltouch":probe,"config_sections":config_sections,"coverage":d["_scope_capture"],"host":d["_scope_host"],"unmeasured":["Physical bolt tightness and belt tension","Actual motor shaft position without encoders","Voltage/current waveforms or measured mains power without suitable sensors","Fan RPM without tachometer feedback","Printed part model, revision and material without an inventory record","BLTouch pin extension and continuous trigger edge timing"]})
}
fn path_value<'a>(d: &'a Value, object: &str, field: &str) -> Option<&'a Value> {
    let mut v = d.get(object)?;
    for k in field.split('.') {
        v = if let Some(a) = v.as_array() {
            a.get(k.parse::<usize>().ok()?)?
        } else {
            v.get(k)?
        };
    }
    Some(v)
}
fn historical_value(d: &Value, object: &str, field: &str) -> Value {
    if let Some(v) = path_value(d, object, field).filter(|v| v.is_number() || v.is_boolean()) {
        return v.clone();
    }
    if object.starts_with("tmc") && d[object]["drv_status"].is_object() {
        if let Some(key) = field.strip_prefix("drv_status.") {
            if FLAGS.contains(&key) || ["cs_actual", "stst", "stealth"].contains(&key) {
                if d[object]["drv_status"].get(key).is_none() {
                    return json!(0);
                }
            }
        }
    }
    Value::Null
}
pub fn handle(
    method: &str,
    target: &str,
    db: &Store,
    cfg: &Config,
    _status: &CollectorStatus,
) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"signal history is read-only; use GET"}));
    }
    let result = (|| -> Result<Value> {
        let (_, q) = super::query(target)?;
        if q.keys()
            .any(|k| !["ts", "window", "points", "object", "field"].contains(&k.as_str()))
        {
            return Err(
                "unknown signal-history parameter; use ts, window, points, object, field".into(),
            );
        }
        let ts = super::number(&q, "ts", Some(model::now()))?;
        let window = super::number(&q, "window", Some(300.0))?;
        let points = q.get("points").map_or(Ok(60), |s| {
            s.parse::<usize>()
                .map_err(|_| "points must be an integer".to_owned())
        })?;
        let object = q.get("object").ok_or("choose a recorded object")?;
        let field = q.get("field").ok_or("choose a recorded field")?;
        if ts > 253402300799.0
            || !(1.0..=7200.0).contains(&window)
            || !(2..=120).contains(&points)
            || !readable(object)
            || object.len() > 200
            || field.len() > 256
            || field.split('.').count() > 5
        {
            return Err("signal request outside supported bounds; select a documented object, field, 2..120 points and a window of 1..7200 seconds".into());
        }
        let _tx = db.conn.unchecked_transaction().map_err(db_error)?;
        let start = std::time::Instant::now();
        let from = (ts - window).max(0.0);
        let mut out = Vec::new();
        let mut previous = None;
        let mut bytes = 0usize;
        for i in 0..points {
            if start.elapsed().as_secs() >= 5 {
                return Err("signal history exceeded time budget; reduce window or points".into());
            }
            let t = from + (ts - from) * i as f64 / (points - 1) as f64;
            if let Some((at, data, n)) = db.load_sample(t, cfg.max_response_bytes.min(2097152))? {
                if at < from || Some(at) == previous {
                    continue;
                }
                if bytes.saturating_add(n) > 67108864 {
                    return Err("signal-history input cap reached; reduce points or window".into());
                }
                bytes += n;
                let v = historical_value(&data, object, field);
                out.push(json!({"ts":at,"value":v}));
                previous = Some(at);
            }
        }
        let (kind, meaning) = classified(object, field);
        Ok(
            json!({"object":object,"field":field,"from":from,"to":ts,"points":out,"kind":kind,"meaning":meaning,"method":"bounded indexed representative history; peaks can be missed; null is unknown"}),
        )
    })();
    match result {
        Ok(v) => Response::json(200, v),
        Err(e) => Response::json(400, json!({"error":e})),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_sparse_driver_history_is_zero() {
        let d = json!({"tmc2209 stepper_z":{"drv_status":{}}});
        assert_eq!(
            historical_value(&d, "tmc2209 stepper_z", "drv_status.cs_actual"),
            json!(0)
        );
        assert!(historical_value(&d, "tmc2209 stepper_z1", "drv_status.cs_actual").is_null());
    }
    #[test]
    fn invalid_dynamic_fan_is_unknown() {
        let p = projection(&json!({"fan_generic x":{"speed":2,"rpm":-1}}), 1.0);
        assert!(p["fans"][0]["command"].is_null());
        assert!(p["fans"][0]["rpm"].is_null());
    }
    #[test]
    fn query_is_not_live() {
        assert_eq!(classified("probe", "last_query").0, "cached query");
    }
    #[test]
    fn duty_not_power_measurement() {
        assert_eq!(classified("extruder", "power").0, "command / setting");
    }
    #[test]
    fn rpm_is_feedback() {
        assert_eq!(classified("fan_generic chamber", "rpm").0, "sensor reading");
    }
    #[test]
    fn config_secret_omitted() {
        let p = projection(
            &json!({"configfile":{"settings":{"gcode_macro SECRET":{"secret":"PRIVATE"},"bltouch":{"sensor_pin":"^PB7","control_pin":"PB6","secret":"PRIVATE"}}},"gcode_macro PRIVATE":{"key":"PRIVATE"}}),
            3.0,
        );
        assert!(!p.to_string().contains("PRIVATE"));
        assert_eq!(p["pins"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn named_mcu_routing_retains_flags() {
        let p = projection(
            &json!({"configfile":{"settings":{"fan_generic chamber":{"pin":"!toolboard:PA1"}}}}),
            3.0,
        );
        assert_eq!(p["pins"][0]["controller"], "toolboard");
        assert_eq!(p["pins"][0]["inverted"], true);
        assert!(p["pins"][0]["physical_connector"].is_null());
    }
    #[test]
    fn modified_fans_are_discovered() {
        let p = projection(
            &json!({"fan_generic exhaust":{"speed":1},"temperature_sensor bay":{"temperature":45}}),
            3.0,
        );
        assert_eq!(p["fans"].as_array().unwrap().len(), 1);
        assert!(p["fans"][0]["rpm"].is_null());
        assert_eq!(p["sensors"][0]["temperature"], 45);
    }
    #[test]
    fn bltouch_does_not_invent_extension() {
        let p = projection(
            &json!({"probe":{"last_query":false,"last_probe_position":[1,2,3]},"configfile":{"settings":{"bltouch":{"control_pin":"PA1"}}}}),
            3.0,
        );
        assert!(p["bltouch"]["pin_extension"].is_null());
        assert_eq!(p["bltouch"]["last_query"], false);
        assert_eq!(p["bltouch"]["last_probe_position"][2], 3);
    }
    #[test]
    fn scalar_cap() {
        let map: serde_json::Map<String, Value> = (0..100)
            .map(|i| {
                (
                    format!("mcu {i}"),
                    json!({"last_stats":{"a":1,"b":2,"c":3,"d":4,"e":5,"f":6}}),
                )
            })
            .collect();
        let p = projection(&Value::Object(map), 1.0);
        assert_eq!(p["signals"].as_array().unwrap().len(), 512);
        assert_eq!(p["limit_reached"], true);
    }
    #[test]
    fn scalar_path_zero() {
        assert_eq!(
            path_value(
                &json!({"motion_report":{"live_position":[0,1,2]}}),
                "motion_report",
                "live_position.0"
            ),
            Some(&json!(0))
        );
    }
    #[test]
    fn refuses_arbitrary_config_history() {
        let db = Store::open(":memory:", false).unwrap();
        let r = handle(
            "GET",
            "/api/signal-history?object=configfile&field=settings",
            &db,
            &Config::default(),
            &CollectorStatus::default(),
        );
        assert_eq!(r.code, 400);
    }
}
