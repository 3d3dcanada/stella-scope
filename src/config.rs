use crate::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub moonraker_url: String,
    pub bind: String,
    pub port: u16,
    /// Phone access (src/lan.rs): off unless set. A second listener for paired devices only.
    pub lan_port: Option<u16>,
    pub lan_bind: String,
    /// Anywhere access (link/, stella-link beside this binary): phones that paired over the phone
    /// listener reach it from any network through iroh. Needs lan_port. Off unless set.
    pub remote_link: bool,
    /// Print failure detection (eye/, stella-eye beside this binary, Obico's open model on the
    /// camera): warns in the app and on WhatsApp. It never pauses a print by itself. Off unless set.
    pub failure_watch: bool,
    /// 0.5 (fewer alarms) to 2.0 (more); 1.0 is Obico's default.
    pub failure_sensitivity: f64,
    /// What the header calls the printer ("My Voron", "Voron 2.4 R2 · 350"). The desktop
    /// app writes the name from the printer's host name at setup; unset shows "Stella Scope".
    pub printer_name: Option<String>,
    pub printer_model: Option<String>,
    pub database: String,
    pub controls_enabled: bool,
    pub poll_interval_ms: u64,
    pub console_interval_ms: u64,
    pub request_timeout_ms: u64,
    pub command_timeout_ms: u64,
    pub offline_retry_ms: u64,
    pub discovery_interval_s: u64,
    pub keep_days: u32,
    pub max_db_mb: u64,
    pub prune_interval_s: u64,
    pub max_http_connections: usize,
    pub http_timeout_ms: u64,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub series_max_points: usize,
    pub events_limit: usize,
    pub qgl_limit: usize,
    pub chart_window_s: u64,
    pub qgl_tolerance_mm: f64,
    pub bed_size_mm: [f64; 2],
    pub bed_size_confirmed: bool,
    pub probe_offset_mm: [f64; 2],
    pub probe_offset_confirmed: bool,
    pub klippy_log_path: Option<String>,
    pub api_key_env: String,
    pub desktop_notifications: bool,
    pub objects: Vec<String>,
    pub telemetry: crate::telemetry::Options,
    pub investigation: crate::api::investigation::Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            moonraker_url: "http://127.0.0.1:7125".into(),
            bind: "127.0.0.1".into(),
            port: 7420,
            lan_port: None,
            lan_bind: "127.0.0.1".into(),
            remote_link: false,
            failure_watch: false,
            failure_sensitivity: 1.0,
            printer_name: None,
            printer_model: None,
            database: "stella-scope.sqlite3".into(),
            controls_enabled: false,
            poll_interval_ms: 500,
            console_interval_ms: 1000,
            request_timeout_ms: 1500,
            command_timeout_ms: 180_000,
            offline_retry_ms: 2000,
            discovery_interval_s: 60,
            keep_days: 14,
            max_db_mb: 4096,
            prune_interval_s: 3600,
            // 8 was too few once NEXT loaded eight modules at once beside a live stream and a camera:
            // a refused module (503) left the page half built (2026-09-25).
            max_http_connections: 32,
            http_timeout_ms: 5000,
            max_request_bytes: 8192,
            max_response_bytes: 2 * 1024 * 1024,
            series_max_points: 1200,
            events_limit: 500,
            qgl_limit: 500,
            chart_window_s: 900,
            qgl_tolerance_mm: 0.02,
            bed_size_mm: [350.0, 350.0],
            bed_size_confirmed: false,
            probe_offset_mm: [0.0, 0.0],
            probe_offset_confirmed: false,
            klippy_log_path: None,
            api_key_env: "STELLA_MOONRAKER_API_KEY".into(),
            // Print done/fault and probe-alarm notices while their window is closed. The launcher's
            // watcher already keeps the recorder alive through a print, so this only fires while
            // the recorder is running. notify-send must exist; its absence degrades to a recorded
            // event, never an error.
            desktop_notifications: true,
            investigation: crate::api::investigation::Limits::default(),
            telemetry: crate::telemetry::Options::default(),
            objects: [
                "motion_report",
                "toolhead",
                "gcode_move",
                "idle_timeout",
                "print_stats",
                "virtual_sdcard",
                "stepper_enable",
                "extruder",
                "heater_bed",
                "fan",
                "heater_fan hotend_fan",
                "controller_fan controller_fan",
                "probe",
                "bed_mesh",
                "quad_gantry_level",
                "configfile=settings",
                "tmc2209 stepper_x",
                "tmc2209 stepper_y",
                "tmc2209 stepper_z",
                "tmc2209 stepper_z1",
                "tmc2209 stepper_z2",
                "tmc2209 stepper_z3",
                "tmc2209 extruder",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|e| {
            format!("cannot read configuration {path}: {e}; check the path and read permission")
        })?;
        let cfg: Self = serde_json::from_str(&text).map_err(|e| {
            format!(
                "invalid configuration in {path}: {e}; use --print-config for the supported keys"
            )
        })?;
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&self) -> Result<()> {
        self.investigation.validate()?;
        self.telemetry.validate()?;
        let url = self.moonraker_url.trim_end_matches('/');
        if !url.starts_with("http://")
            || url.len() <= 7
            || url.contains(['?', '#', '@', '\r', '\n'])
        {
            return Err("moonraker_url must be an http:// host and optional port, without credentials or query text; use the local Moonraker HTTP address".into());
        }
        if self.bind.parse::<std::net::IpAddr>().is_err() || self.port == 0 {
            return Err(
                "bind must be an IP address and port must be 1..65535; use 127.0.0.1 and 7420"
                    .into(),
            );
        }
        // Every desktop request must carry exactly this Host (DNS-rebinding guard, src/server.rs),
        // which a wildcard or network address can never match: the desktop listener is loopback.
        if !self.bind.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
            return Err("bind must be a loopback address (127.0.0.1); reach Stella Scope from another device with the phone app's encrypted link".into());
        }
        if let Some(p) = self.lan_port {
            if p == 0 || p == self.port || self.lan_bind != "127.0.0.1" {
                return Err("phone access requires a separate lan_port and lan_bind 127.0.0.1; use the encrypted link from a phone".into());
            }
            if self.max_http_connections < 5 {
                return Err("phone access needs at least 5 max_http_connections to reserve desktop reader slots".into());
            }
        }
        if self.controls_enabled
            && !self
                .bind
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
        {
            return Err(
                "controls_enabled requires a loopback bind such as 127.0.0.1 or ::1".into(),
            );
        }
        if self.database.is_empty() || Path::new(&self.database).is_dir() {
            return Err("database must name a SQLite file, not a directory".into());
        }
        if !(50..=60000).contains(&self.poll_interval_ms)
            || !(100..=60000).contains(&self.console_interval_ms)
            || !(100..=30000).contains(&self.request_timeout_ms)
            || !(1000..=900000).contains(&self.command_timeout_ms)
            || !(250..=60000).contains(&self.offline_retry_ms)
            || self.discovery_interval_s == 0
            || self.prune_interval_s == 0
            || self.keep_days == 0
            || self.keep_days > 3650
            || self.max_db_mb == 0
            || self.max_db_mb > 1048576
            || !(1..=32).contains(&self.max_http_connections)
            || !(100..=30000).contains(&self.http_timeout_ms)
            || !(1024..=65536).contains(&self.max_request_bytes)
            || !(65536..=16777216).contains(&self.max_response_bytes)
            || !(2..=4000).contains(&self.series_max_points)
            || !(1..=10000).contains(&self.events_limit)
            || !(1..=5000).contains(&self.qgl_limit)
            || self.chart_window_s == 0
            || self.chart_window_s > 31536000
        {
            return Err("configuration limits are invalid; compare intervals, retention, and caps with --print-config and README".into());
        }
        if !self.qgl_tolerance_mm.is_finite()
            || self.qgl_tolerance_mm <= 0.0
            || self.bed_size_mm.iter().any(|x| !x.is_finite() || *x <= 0.0)
            || self.probe_offset_mm.iter().any(|x| !x.is_finite())
        {
            return Err("bed dimensions and levelling tolerance must be positive finite numbers; probe offsets must be finite".into());
        }
        if self.objects.is_empty()
            || self.objects.len() > 128
            || self
                .objects
                .iter()
                .any(|o| o.is_empty() || o.len() > 200 || o.contains(['&', '?', '#', '\r', '\n']))
        {
            return Err("objects must contain 1..128 non-empty Klipper object names; use name=field for a field filter".into());
        }
        Ok(())
    }

    pub fn host(&self) -> String {
        match self.bind.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(address)) => format!("[{address}]:{}", self.port),
            _ => format!("{}:{}", self.bind, self.port),
        }
    }
}
