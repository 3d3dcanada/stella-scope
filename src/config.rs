use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// The id an old single-printer configuration gets, and the one our recording lives under.
pub const LEGACY_ID: &str = "stella";
/// The id of the printer an unconfigured service starts with (polish WO-06 item 7, 2026-10-18): it is not
/// "stella", so a person adding their first printer, most likely called Stella, never collides with it.
pub const PLACEHOLDER_ID: &str = "placeholder";
/// What a service with no printer says when a page asks it for one (finish-2 F01).
pub const NO_PRINTER: &str =
    "No printer is set up yet. Add one with Add a printer, or try the demo farm.";

/// One printer of the farm (farm board WO-03). Credentials are never here: `credentials_ref` names a
/// file `<config dir>/printers/<ref>.secret` that must be mode 600.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrinterEntry {
    /// Lower-case letters, digits and dashes; the URL segment `/p/<id>` and the database file name.
    pub id: String,
    pub name: String,
    /// moonraker, prusalink or flashforge; empty means moonraker.
    pub driver: String,
    /// The printer's HTTP base address, e.g. http://192.168.1.139
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_ref: Option<String>,
    /// Where this printer's recording lives. Absent: `<data dir>/printers/<id>.sqlite3`. The printer an
    /// old configuration describes keeps its own path here, so a recording is never moved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// The id of the 3D model in this printer's library (WO-10). Absent means no model, never a generic one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A colour for this printer's tile and switcher dot, `#RRGGBB`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Where this printer's camera is, when the printer itself does not say: `rtsp://` or `http://`
    /// on this network (the CORE One's Buddy3D is its own device, e.g. rtsp://192.168.1.132:554/live).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bed_size_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_offset_mm: Option<[f64; 2]>,
    /// What the queue (WO-08) needs to know to send a sliced file here: the nozzle fitted, what this
    /// printer is called in the slicer (`COREONEL`, `Anycubic Kobra X`), and the material loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nozzle_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slicer_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// The real CAD of this printer (WO-10): an id from the 3D Print Rig catalogue, and the calibration
    /// that turns the position it reports into the model's axis values. Absent means no model, never a
    /// generic one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad_calibration: Option<serde_json::Value>,
    /// The toolchanger, toolhead and add-on options chosen for this printer's model (polish WO-01): a list of
    /// `{option, tools, active}` choices checked against the machine (`printer_models::check_selection`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cad_options: Option<serde_json::Value>,
    /// The smart plug this printer is on (src/plugs.rs): `shelly://host`, `tasmota://host`, `kasa://host`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plug: Option<String>,
    /// A demo printer (`driver: "demo"`) starts its sample print this far in (0 to 1) when the service
    /// starts, already hot. Absent: the demo waits for someone to start a print.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_start_at: Option<f64>,
    /// A demo printer that joins part way starts paused, so the wall can show one that needs a hand
    /// (2026-10-18, polish WO-02). Ignored unless `demo_start_at` is set.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub demo_paused: bool,
    /// When the demo farm was added (unix seconds). A demo's sample print is placed by the time since then, so a restart
    /// of the service continues it instead of starting it over (finish-2 F10, 2026-10-19).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_since: Option<f64>,
}

/// Credentials read from a secret file. `Debug` never prints them.
#[derive(Clone, Default, PartialEq)]
pub struct Secret(pub BTreeMap<String, String>);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({} fields, redacted)", self.0.len())
    }
}
impl Secret {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// A camera address a user typed: rtsp:// or http://, and on this network only (a private, link-local
/// or loopback IPv4 address, or a .local name), so the service is never a way to reach somewhere else.
pub fn camera_ok(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("rtsp://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    if url.chars().any(|c| c.is_control() || c == ' ') || rest.contains('@') {
        return false;
    }
    let host = rest.split(['/', '?']).next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    if host.ends_with(".local") && host.len() > 6 {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Err(_) => false,
    }
}

/// `id`: the form every URL, file and config entry uses.
pub fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

/// Read `<dir>/printers/<reference>.secret`: a JSON object of strings. A file that anyone but its owner
/// can read is refused (the same rule slicer-upload keeps), with the fix in the message.
pub fn load_secret(dir: &Path, reference: &str) -> Result<Secret> {
    if reference.is_empty()
        || reference.len() > 64
        || !reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("credentials_ref may hold only letters, digits, dash and underscore".into());
    }
    let path = dir.join("printers").join(format!("{reference}.secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(&path).map_err(|e| {
            format!(
                "cannot read credentials {}: {e}; create it with mode 600",
                path.display()
            )
        })?;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "credentials {} are readable by others (mode {mode:o}); run chmod 600 on it",
                path.display()
            ));
        }
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("cannot read credentials {}: {e}", path.display()))?;
    let value: BTreeMap<String, String> = serde_json::from_str(&text).map_err(|e| {
        format!(
            "credentials {} must be a JSON object of strings: {e}",
            path.display()
        )
    })?;
    Ok(Secret(value))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The brand of this printer: moonraker (Klipper), prusalink or flashforge (farm board WO-02).
    /// An older file without it is a Moonraker printer, as it always was.
    pub driver: String,
    /// The printer's HTTP base address. The name is kept for every brand so older files still load;
    /// for another brand it is that brand's own address (PrusaLink :80, FlashForge :8898).
    pub moonraker_url: String,
    /// The printers of the farm. Empty: one printer made of the fields below, id `stella`.
    pub printers: Vec<PrinterEntry>,
    /// Set for each printer's own copy of the configuration (never read from the file).
    #[serde(skip)]
    pub printer_id: String,
    /// Credentials for this printer's own copy (never read from or written to the file).
    #[serde(skip)]
    pub secret: Secret,
    /// The camera address for this printer's own copy (from its entry).
    #[serde(skip)]
    pub camera_url: Option<String>,
    /// The demo start point for this printer's own copy (from its entry).
    #[serde(skip)]
    pub demo_start_at: Option<f64>,
    #[serde(skip)]
    pub demo_paused: bool,
    #[serde(skip)]
    pub demo_since: Option<f64>,
    /// The catalogue machine this printer is (from its entry); a demo printer takes its bed, travel and
    /// motion from it.
    #[serde(skip)]
    pub cad_model: Option<String>,
    /// The options chosen for it (polish WO-03): a demo printer with a toolchanger bank has that many tools.
    #[serde(skip)]
    pub cad_options: Option<serde_json::Value>,
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
    /// What the header calls the printer ("Stella", "Voron 2.4 R2 · 350"). The desktop app writes
    /// the name from the printer's host name at setup; unset shows "Stella Scope".
    pub printer_name: Option<String>,
    pub printer_model: Option<String>,
    /// Epoch seconds when the sixth printer was added and the 30-day trial started. Absent means no
    /// trial has begun; once set, the farm runs in trial for 30 days and then locks printers beyond
    /// the free five until a licence is entered.
    pub licence_trial_started: Option<f64>,
    /// Nothing named a printer: no configuration file at all, or one with neither `moonraker_url` nor a
    /// `printers` list (polish WO-06 item 7, 2026-10-18). The one printer such a service starts with is a
    /// placeholder for the farm, not a printer: it is never shown or counted while nothing answers there.
    /// A file that names an address is a configured printer, however long it has been off. Never saved.
    #[serde(skip)]
    pub unconfigured: bool,
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
            driver: "moonraker".into(),
            // No address until the person names a printer (finish-2 F01, 2026-10-19). This was
            // http://127.0.0.1:7125, so a service with no printer polled whatever answered there: on our PC
            // that is the Kobra X bridge, which was read twice in testing. An empty address opens nothing.
            moonraker_url: String::new(),
            printers: Vec::new(),
            printer_id: LEGACY_ID.into(),
            secret: Secret::default(),
            camera_url: None,
            demo_start_at: None,
            demo_paused: false,
            demo_since: None,
            cad_model: None,
            cad_options: None,
            bind: "127.0.0.1".into(),
            port: 7420,
            lan_port: None,
            lan_bind: "127.0.0.1".into(),
            remote_link: false,
            failure_watch: false,
            failure_sensitivity: 1.0,
            printer_name: None,
            printer_model: None,
            licence_trial_started: None,
            unconfigured: false,
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
            // 2026-09-22: print done/fault and probe-alarm notices while his window is closed.
            // The launcher's watcher already keeps the recorder alive through a print, so this only
            // fires while the recorder is running. notify-send must exist; its absence degrades to
            // a recorded event, never an error.
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
        let mut cfg: Self = serde_json::from_str(&text).map_err(|e| {
            format!(
                "invalid configuration in {path}: {e}; use --print-config for the supported keys"
            )
        })?;
        cfg.unconfigured = Self::names_no_printer(&text);
        cfg.validate()?;
        Ok(cfg)
    }
    /// True when a configuration file's text names no printer: no `moonraker_url`, and no `printers` but demo ones.
    pub fn names_no_printer(text: &str) -> bool {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
            return false;
        };
        v.get("moonraker_url")
            .and_then(|u| u.as_str())
            .is_none_or(|u| u.trim().is_empty())
            && v.get("printers")
                .and_then(|p| p.as_array())
                .is_none_or(|a| a.iter().all(|e| e["driver"] == "demo"))
    }
    pub fn validate(&self) -> Result<()> {
        self.investigation.validate()?;
        self.telemetry.validate()?;
        if !crate::driver::KINDS.contains(&self.driver.as_str()) {
            return Err(format!(
                "driver must be one of {}; an older configuration without it is a Moonraker printer",
                crate::driver::KINDS.join(", ")
            ));
        }
        let url = self.moonraker_url.trim_end_matches('/');
        // No address: the placeholder of a service with no printer (nothing is ever opened to it), or a farm
        // file whose printers carry their own addresses (each is checked as its own printer's copy).
        if self.driver != "demo"
            && !(url.is_empty() && (self.unconfigured || !self.printers.is_empty()))
            && (!url.starts_with("http://")
                || url.len() <= 7
                || url.contains(['?', '#', '@', '\r', '\n']))
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
        if !self
            .bind
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
        {
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
        let mut seen = std::collections::BTreeSet::new();
        for e in &self.printers {
            if !id_ok(&e.id) {
                return Err(format!(
                    "printer id {:?} must be 1 to 32 lower-case letters, digits or dashes",
                    e.id
                ));
            }
            if !seen.insert(e.id.clone()) {
                return Err(format!(
                    "two printers are called {}; ids must be unique",
                    e.id
                ));
            }
            let driver = if e.driver.is_empty() {
                "moonraker"
            } else {
                e.driver.as_str()
            };
            if !crate::driver::KINDS.contains(&driver) {
                return Err(format!(
                    "printer {} has driver {driver:?}; use one of {}",
                    e.id,
                    crate::driver::KINDS.join(", ")
                ));
            }
            let url = e.address.trim_end_matches('/');
            // A demo printer has no address: it runs inside this program (src/demo.rs).
            if driver == "demo" {
                if !url.is_empty() {
                    return Err(format!("demo printer {} takes no address", e.id));
                }
            } else if !url.starts_with("http://")
                || url.len() <= 7
                || url.contains(['?', '#', '@', '\r', '\n'])
            {
                return Err(format!(
                    "printer {} needs an http:// address without credentials or query text",
                    e.id
                ));
            }
            if e.name.trim().is_empty()
                || e.name.chars().count() > 60
                || e.name.chars().any(char::is_control)
            {
                return Err(format!(
                    "printer {} needs a name of 1 to 60 characters",
                    e.id
                ));
            }
            if let Some(cam) = &e.camera {
                if !camera_ok(cam) {
                    return Err(format!("printer {} camera must be an rtsp:// or http:// address on this network (a private address)", e.id));
                }
            }
            if let Some(a) = &e.accent {
                if !(a.len() == 7
                    && a.starts_with('#')
                    && a[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    return Err(format!("printer {} accent must look like #04D9C4", e.id));
                }
            }
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

    /// The printers this configuration describes. An old file with no `printers` is a farm of one:
    /// `stella`, built from its existing fields and keeping its existing database path.
    ///
    /// Polish WO-06 item 7 (2026-10-18): an unconfigured service (nothing names a printer, or only demo printers)
    /// has the placeholder printer first, so the farm always has a first printer that is not a demo; the farm
    /// never shows it (see `Farm::visible`).
    pub fn entries(&self) -> Vec<PrinterEntry> {
        if !self.printers.is_empty() && !self.unconfigured {
            return self.printers.clone();
        }
        let first = PrinterEntry {
            id: if self.unconfigured {
                PLACEHOLDER_ID
            } else {
                LEGACY_ID
            }
            .into(),
            name: self.printer_name.clone().unwrap_or_else(|| "Stella".into()),
            driver: self.driver.clone(),
            address: self.moonraker_url.clone(),
            database: Some(self.database.clone()),
            model: self.printer_model.clone(),
            bed_size_mm: Some(self.bed_size_mm),
            probe_offset_mm: Some(self.probe_offset_mm),
            ..PrinterEntry::default()
        };
        let mut all = vec![first];
        all.extend(self.printers.iter().cloned());
        all
    }

    /// The recording a printer keeps: its own path if the entry names one, else
    /// `<database>.printers/<id>.sqlite3` (WO-13, 2026-10-12: named after the configured recording,
    /// so a scratch config writes a scratch folder and never the live one; the live 0.4.2 install
    /// holds no per-printer recordings, so there is nothing to carry over).
    pub fn recording_for(&self, e: &PrinterEntry) -> String {
        match &e.database {
            Some(d) => d.clone(),
            None => PathBuf::from(format!("{}.printers", self.database))
                .join(format!("{}.sqlite3", e.id))
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// This printer's own copy of the configuration: the shared limits and intervals, its own address,
    /// driver, recording, name and credentials. Process-wide extras (the phone link, the failure watch,
    /// the klippy log) stay with the first printer only.
    pub fn for_printer(&self, e: &PrinterEntry, config_dir: &Path) -> Result<Config> {
        let first = self
            .entries()
            .first()
            .map(|f| f.id == e.id)
            .unwrap_or(false);
        let mut c = self.clone();
        c.printers.clear();
        c.printer_id = e.id.clone();
        // Only the placeholder itself is unconfigured: a real printer added to such a farm has its own address.
        c.unconfigured = self.unconfigured && e.id == PLACEHOLDER_ID;
        c.driver = if e.driver.is_empty() {
            "moonraker".into()
        } else {
            e.driver.clone()
        };
        c.moonraker_url = e.address.clone();
        c.database = self.recording_for(e);
        c.printer_name = Some(e.name.clone());
        c.printer_model = e.model.clone().or_else(|| {
            if first {
                self.printer_model.clone()
            } else {
                None
            }
        });
        if let Some(b) = e.bed_size_mm {
            c.bed_size_mm = b;
        } else if !first {
            c.bed_size_confirmed = false;
        }
        if let Some(p) = e.probe_offset_mm {
            c.probe_offset_mm = p;
        } else if !first {
            c.probe_offset_confirmed = false;
        }
        if !first {
            c.klippy_log_path = None;
            c.failure_watch = false;
            c.remote_link = false;
        }
        c.camera_url = e.camera.clone();
        c.demo_start_at = e.demo_start_at;
        c.demo_paused = e.demo_paused;
        c.demo_since = e.demo_since;
        c.cad_model = e.cad_model.clone();
        c.cad_options = e.cad_options.clone();
        c.secret = match &e.credentials_ref {
            Some(r) => load_secret(config_dir, r)?,
            None => Secret::default(),
        };
        Ok(c)
    }

    pub fn host(&self) -> String {
        match self.bind.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(address)) => format!("[{address}]:{}", self.port),
            _ => format!("{}:{}", self.bind, self.port),
        }
    }
}
