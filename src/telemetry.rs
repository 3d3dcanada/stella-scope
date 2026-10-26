use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub discover: bool,
    pub max_extra_objects: usize,
    pub host_stats: bool,
    pub host_interval_ms: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            discover: true,
            max_extra_objects: 96,
            host_stats: false,
            host_interval_ms: 5000,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        if self.max_extra_objects > 256 || !(1000..=60000).contains(&self.host_interval_ms) {
            return Err("telemetry.max_extra_objects must be 0..256 and host_interval_ms 1000..60000; restore the documented defaults".into());
        }
        Ok(())
    }
}
pub fn allowed(name: &str) -> bool {
    [
        "mcu",
        "system_stats",
        "webhooks",
        "heaters",
        "query_endstops",
        "pause_resume",
        "display_status",
    ]
    .contains(&name)
        || [
            "mcu ",
            "temperature_sensor ",
            "temperature_fan ",
            "heater_generic ",
            "heater_fan ",
            "controller_fan ",
            "fan_generic ",
            "canbus_stats ",
            "filament_switch_sensor ",
            "filament_motion_sensor ",
            "output_pin ",
            "pwm_cycle_time ",
            "servo ",
            "extruder_stepper ",
            "tmc2209 ",
            "tmc2240 ",
            "tmc5160 ",
            "tmc2130 ",
            "tmc2208 ",
            "tmc2660 ",
        ]
        .iter()
        .any(|p| name.starts_with(p))
}
pub fn select(
    configured: &[String],
    names: &[Value],
    opts: &Options,
) -> (Vec<String>, Vec<String>, Value) {
    let available: BTreeSet<String> = names
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 200 && !s.contains(['&', '?', '#', '\r', '\n']))
        .take(2048)
        .map(str::to_owned)
        .collect();
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut notes = Vec::new();
    for spec in configured {
        let name = spec.split('=').next().unwrap_or(spec);
        if available.contains(name) {
            if seen.insert(name.to_owned()) {
                selected.push(spec.clone());
            }
        } else if matches!(name, "input_shaper" | "adxl345" | "safe_z_home") {
            // These are real config sections, but Klipper exposes no status object for them
            // (observed live 2026-09-17). Their settings are read from configfile instead;
            // telling the owner to "check its Klipper configuration name" sends him to fix a
            // configuration that is correct.
            notes.push(format!(
                "Configuration-only object: {name} has no status object; its settings are read from the recorded configfile"
            ));
        } else {
            notes.push(format!(
                "Not reported: {name}; check its Klipper configuration name"
            ));
        }
    }
    let mut added = 0;
    let mut omitted = Vec::new();
    for name in &available {
        if seen.contains(name) {
            continue;
        }
        if opts.discover && allowed(name) && added < opts.max_extra_objects {
            selected.push(name.clone());
            seen.insert(name.clone());
            added += 1;
        } else {
            omitted.push(json!({"object":name,"reason":if !opts.discover {"automatic discovery disabled"}else if !allowed(name){"not in automatic safe-status families; add explicitly to objects after review"}else{"extra-object count cap"}}));
        }
    }
    if names.len() > 2048 {
        notes.push("Discovery list capped at 2048 names; review the object list before claiming complete coverage".into());
    }
    if omitted
        .iter()
        .any(|v| v["reason"] == "extra-object count cap")
    {
        notes.push("Telemetry object cap reached; see Signals coverage for omitted objects".into());
    }
    let manifest = json!({"available":available,"captured":seen,"omitted":omitted,"extra_objects":added,
      "policy":"configured objects plus bounded automatic status families; macro variables are not automatically collected"});
    (selected, notes, manifest)
}
pub fn host_summary(data: &Value, ts: f64) -> Value {
    // Network interfaces and system identity are deliberately absent from the host record.
    let mut selected = serde_json::Map::new();
    for key in [
        "cpu_temp",
        "system_memory",
        "system_cpu_usage",
        "throttled_state",
        "moonraker_stats",
    ] {
        if let Some(v) = data.get(key) {
            selected.insert(key.into(), v.clone());
        }
    }
    json!({"captured_at":ts,"source":"GET /machine/proc_stats","data":selected})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_adds_named_fans_and_mcus_not_macro_secrets() {
        let (v, _, m) = select(
            &["extruder".into()],
            &vec![
                json!("extruder"),
                json!("mcu toolboard"),
                json!("fan_generic exhaust"),
                json!("gcode_macro PRIVATE"),
            ],
            &Options::default(),
        );
        assert_eq!(v.len(), 3);
        assert_eq!(m["omitted"][0]["object"], "gcode_macro PRIVATE");
    }
    #[test]
    fn explicit_macros_are_not_silently_removed() {
        let (v, _, _) = select(
            &["gcode_macro USER_APPROVED".into()],
            &[json!("gcode_macro USER_APPROVED")],
            &Options::default(),
        );
        assert_eq!(v.len(), 1);
    }
    #[test]
    fn field_filters_and_order_are_preserved() {
        let (v, _, _) = select(
            &["configfile=settings".into(), "mcu".into(), "mcu".into()],
            &[json!("configfile"), json!("mcu")],
            &Options::default(),
        );
        assert_eq!(v, vec!["configfile=settings", "mcu"]);
    }
    #[test]
    fn cap_is_visible() {
        let mut o = Options::default();
        o.max_extra_objects = 0;
        let (_, n, m) = select(&[], &[json!("mcu")], &o);
        assert!(!n.is_empty());
        assert_eq!(m["omitted"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn no_automatic_new_queries_when_disabled() {
        let mut o = Options::default();
        o.discover = false;
        let (v, _, _) = select(&[], &[json!("mcu")], &o);
        assert!(v.is_empty());
    }
    #[test]
    fn host_identity_is_not_copied() {
        let v = host_summary(
            &json!({"cpu_temp":48,"network":{"ip":"PRIVATE"},"system_info":"PRIVATE"}),
            1.0,
        );
        assert!(!v.to_string().contains("PRIVATE"));
    }
    #[test]
    fn option_bounds() {
        let mut o = Options::default();
        o.host_interval_ms = 0;
        assert!(o.validate().is_err());
    }
}
