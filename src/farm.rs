//! Many printers in one service (farm board WO-03, 2026-10-12).
//!
//! One process hosts N printers. Each keeps what it always had: its own recording (SQLite), its own
//! asset library beside it, its own collector thread, its own driver, its own control service (token,
//! busy slots, command audit). `server.rs` picks the printer by URL: `/api/p/<id>/...`, and a request
//! with no prefix means the first printer, so the current page, the phone app 0.4.x and stella-link keep
//! working untouched. An old configuration with no `printers` is a farm of one, `stella`, using its
//! existing fields and its existing recording; nothing is migrated or moved.
use crate::{
    collector::{self, Shared},
    commands,
    config::{Config, PrinterEntry},
    licence::FREE_PRINTERS,
    model::{self, CollectorStatus},
    store::Store,
    Result,
};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread,
};

pub struct Printer {
    pub id: String,
    pub entry: PrinterEntry,
    pub cfg: Config,
    pub state: Shared,
    pub controls: Arc<commands::Service>,
    stop: Arc<AtomicBool>,
    /// The recording's writer lock, held for the life of the printer.
    _lock: Option<File>,
}

impl Printer {
    /// Ask this printer's collector (and its push source) to end at their next pass.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct Farm {
    /// The configuration as read: shared limits and intervals, and the process-wide extras.
    pub base: Config,
    /// When the farm's one thirty-day trial started (epoch seconds). `base` holds what the file said at the start; this is what is true now (finish-2 F06:
    /// the sixth printer wrote the start into the file but the running service went on reading none, so it said "a licence is asked for" with no trial running,
    /// and every printer added after it wrote a newer start, so the trial could be stretched for as long as the service ran).
    trial_started: Mutex<Option<f64>>,
    /// `live`, `demo` or `replay`. A demo or replay is always a farm of one.
    pub mode: String,
    pub config_path: Option<PathBuf>,
    /// The token farm-level writes carry (add, rename, remove). Absent when controls are off.
    pub token: Option<String>,
    printers: RwLock<Vec<Arc<Printer>>>,
    /// Printers that are configured but could not start, with why (a secret file readable by others,
    /// a recording another process holds). They are listed, offline, with the reason.
    failed: Mutex<Vec<(PrinterEntry, String)>>,
    /// Polish WO-06 item 7 (2026-10-18): the id of the placeholder printer an unconfigured service starts with (see
    /// `Config::unconfigured`), until a real printer is added and it is retired. It is never listed or counted
    /// while nothing has answered there.
    placeholder: Mutex<Option<String>>,
    demo: bool,
    /// The job queue and the plate flow (WO-08), when this service can run one.
    queue: Option<Arc<crate::queue::Queue>>,
    queue_error: Option<String>,
}

impl Farm {
    pub fn start(
        base: Config,
        config_path: Option<PathBuf>,
        demo: bool,
        replay: bool,
        warnings: Vec<String>,
    ) -> Result<Arc<Farm>> {
        let mode = if replay {
            "replay"
        } else if demo {
            "demo"
        } else {
            "live"
        }
        .to_owned();
        let eligible = (mode == "demo")
            || (base.controls_enabled
                && mode == "live"
                && base
                    .bind
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|a| a.is_loopback()));
        let token = if eligible {
            Some(random_token()?)
        } else {
            None
        };
        // The queue lives beside the first printer's recording. Not in a demonstration or a saved
        // recording, and not with controls off: it exists to send files to printers.
        let (queue, queue_error) = if mode == "live" && base.controls_enabled {
            let dir = Path::new(&base.database)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .join("farm-queue");
            match crate::queue::Queue::open(&dir) {
                Ok(q) => (Some(Arc::new(q)), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, Some("the queue is off while controls are off, in a demonstration and in a saved recording".to_owned()))
        };
        let placeholder =
            (base.unconfigured && mode == "live").then(|| crate::config::PLACEHOLDER_ID.to_owned());
        let farm = Arc::new(Farm {
            trial_started: Mutex::new(base.licence_trial_started),
            base,
            mode,
            config_path,
            token,
            printers: RwLock::new(Vec::new()),
            failed: Mutex::new(Vec::new()),
            placeholder: Mutex::new(placeholder),
            demo,
            queue,
            queue_error,
        });
        let mut entries = farm.base.entries();
        if farm.mode != "live" {
            entries.truncate(1); // a demo or a replay is the first printer only
        }
        for (i, entry) in entries.iter().enumerate() {
            match farm.launch(entry, if i == 0 { warnings.clone() } else { Vec::new() }) {
                Ok(p) => farm
                    .printers
                    .write()
                    .map_err(|_| "farm lock poisoned")?
                    .push(p),
                // The first printer is the service: if it cannot start, nothing can.
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    eprintln!("printer {} did not start: {e}", entry.id);
                    farm.failed
                        .lock()
                        .map_err(|_| "farm lock poisoned")?
                        .push((entry.clone(), e));
                }
            }
        }
        // WO-04: printers beyond the free five are locked once the trial ends and no licence covers
        // them. The free five are the first five real (non-demo) printers added. Compute the lock
        // state once at startup from the licence file and the trial start recorded in the config;
        // a running print is never interrupted by it.
        let st = farm.licence_status();
        if st.get("locked").and_then(|v| v.as_bool()).unwrap_or(false) {
            let reason = st
                .get("note")
                .and_then(|v| v.as_str())
                .unwrap_or("this printer is locked by the farm licence")
                .to_owned();
            let real: Vec<&PrinterEntry> = entries.iter().filter(|e| e.driver != "demo").collect();
            for (idx, entry) in real.iter().enumerate() {
                if idx < FREE_PRINTERS {
                    continue;
                }
                if let Ok(printers) = farm.printers.read() {
                    if let Some(p) = printers.iter().find(|pr| pr.entry.id == entry.id) {
                        p.controls.set_licence_locked(true, reason.clone());
                    }
                }
            }
        }
        if farm.queue.is_some() {
            crate::queue_flow::spawn_watch(&farm);
        }
        if !replay {
            crate::fleet::spawn_watch(&farm);
        }
        Ok(farm)
    }

    /// Printers on this service, running or not.
    pub fn printer_count(&self) -> usize {
        // Demonstration printers are simulations on this machine: they never count toward the
        // farm licence, which covers real printers.
        self.entries_now()
            .iter()
            .filter(|e| e.driver != "demo")
            .count()
    }

    fn licence_path(&self) -> PathBuf {
        self.config_dir().join("farm-licence.txt")
    }

    /// Where the farm stands on the licence (`GET /api/licence`).
    pub fn licence_status(&self) -> Value {
        self.licence_status_with(&crate::licence::PUBLIC_KEY)
    }

    pub fn licence_status_with(&self, key: &[u8; 32]) -> Value {
        let saved = fs::read_to_string(self.licence_path()).ok();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        crate::licence::status_with_clock(
            saved.as_deref(),
            self.printer_count(),
            key,
            now,
            self.trial_started.lock().ok().and_then(|t| *t),
        )
    }

    /// Keep a licence that checks out. A text that does not is refused in words and nothing is written.
    pub fn save_licence(&self, text: &str) -> Result<Value> {
        self.save_licence_with(text, &crate::licence::PUBLIC_KEY)
    }

    pub fn save_licence_with(&self, text: &str, key: &[u8; 32]) -> Result<Value> {
        crate::licence::verify_with(text, key)?;
        let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let path = self.licence_path();
        let tmp = path.with_extension("txt.tmp");
        fs::write(&tmp, format!("{clean}\n"))
            .map_err(|e| format!("cannot save the licence: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot save the licence: {e}"))?;
        Ok(self.licence_status_with(key))
    }

    pub fn remove_licence(&self) -> Result<Value> {
        let _ = fs::remove_file(self.licence_path());
        Ok(self.licence_status())
    }

    pub fn queue(&self) -> Option<Arc<crate::queue::Queue>> {
        self.queue.clone()
    }
    pub fn queue_error(&self) -> String {
        self.queue_error
            .clone()
            .unwrap_or_else(|| "the queue is not available".into())
    }

    /// Where the person's installed 3D models live, beside the first printer's recording.
    pub fn models_dir(&self) -> PathBuf {
        crate::printer_models::library_dir(
            Path::new(&self.base.database)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
    }

    pub fn library(&self) -> crate::printer_models::Library {
        crate::printer_models::Library::with_carried(self.models_dir(), crate::printer_models::carried_dir())
    }

    /// The folder or address models were last fetched from, so the page offers it again.
    pub fn model_source(&self) -> Option<String> {
        fs::read_to_string(self.models_dir().join("source.txt"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }

    pub fn set_model_source(&self, source: &str) {
        let dir = self.models_dir();
        if fs::create_dir_all(&dir).is_ok() {
            let _ = fs::write(dir.join("source.txt"), source);
        }
    }

    pub fn config_dir(&self) -> PathBuf {
        match self.config_path.as_deref().and_then(Path::parent) {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    /// Start the farm's own extras (finish-2 F05, 2026-10-19): alerts, the anywhere link (`lan`) and the failure watch. They
    /// keep their files beside the farm's recording, where their settings routes write them; started from the first
    /// printer's copy they read beside *its* recording, so in a farm of added printers no alert was ever sent (in a
    /// one-printer file both are the same path). The failure watch looks at the first printer, whose address it keeps.
    pub fn start_extras(&self, lan: bool, readonly: bool) {
        if readonly {
            return;
        }
        crate::notify::init(&self.base);
        crate::remote::set_farm(&self.base.database);
        self.refresh_watched();
        if lan {
            crate::remote::start(&self.base);
        }
        crate::remote::start_eye(&self.base);
    }

    pub fn list(&self) -> Vec<Arc<Printer>> {
        self.printers.read().map(|p| p.clone()).unwrap_or_default()
    }
    pub fn get(&self, id: &str) -> Option<Arc<Printer>> {
        self.list().into_iter().find(|p| p.id == id)
    }
    /// The printer an un-prefixed request means: the first one a person can see (the placeholder of an unconfigured
    /// service is not one), else the first one there is.
    pub fn first(&self) -> Arc<Printer> {
        self.visible()
            .first()
            .cloned()
            .unwrap_or_else(|| self.lead())
    }
    /// The first printer there is, placeholder or not: it carries the phone link and the failure watch.
    fn lead(&self) -> Arc<Printer> {
        self.list()
            .first()
            .cloned()
            .expect("a farm always has its first printer")
    }

    /// The placeholder printer, while it is one: an unconfigured service's only printer, which nothing has answered.
    fn placeholder_printer(&self) -> Option<Arc<Printer>> {
        let id = self.placeholder.lock().ok()?.clone()?;
        let p = self.get(&id)?;
        let answered = p
            .state
            .lock()
            .map(|s| s.last_success.is_some() || s.last_tick_ts.is_some())
            .unwrap_or(false);
        (!answered).then_some(p)
    }
    /// Every printer a person is shown: the farm without its placeholder.
    pub fn visible(&self) -> Vec<Arc<Printer>> {
        let hidden = self.placeholder_printer().map(|p| p.id.clone());
        self.list()
            .into_iter()
            .filter(|p| Some(&p.id) != hidden.as_ref())
            .collect()
    }
    /// A real printer was added: the placeholder has done its job and goes (its recording stays, empty).
    fn retire_placeholder(&self) {
        if let Some(p) = self.placeholder_printer() {
            p.stop();
            crate::drivers::forget(&p.id);
            if let Ok(mut all) = self.printers.write() {
                all.retain(|q| q.id != p.id);
            }
        }
        if let Ok(mut slot) = self.placeholder.lock() {
            *slot = None;
        }
    }

    /// The last printer was taken away (finish-2 F06, 2026-10-19): the farm is a fresh install again, with its inert placeholder, so the
    /// first-printer machinery (`lead`, `first`, the page for `/`) has what it needs and nothing is polled. Its recording is its own
    /// empty file, never the one of the printer that went.
    fn revive_placeholder(&self) -> Result<()> {
        let mut base = self.base.clone();
        base.unconfigured = true;
        base.moonraker_url = String::new();
        let mut entry = base
            .entries()
            .into_iter()
            .next()
            .ok_or("no placeholder to start")?;
        entry.database = None;
        let printer = self.launch_in(&base, &entry, Vec::new(), None)?;
        self.printers
            .write()
            .map_err(|_| "farm lock poisoned")?
            .insert(0, printer);
        if let Ok(mut slot) = self.placeholder.lock() {
            *slot = Some(crate::config::PLACEHOLDER_ID.to_owned());
        }
        Ok(())
    }

    /// The printer the failure watch looks at: the first one a person can see that is real (a demo has no camera and no address).
    pub fn watch_target(&self) -> Option<Arc<Printer>> {
        self.visible()
            .into_iter()
            .find(|p| p.entry.driver != "demo" && !p.cfg.moonraker_url.trim().is_empty())
    }

    /// The failure watch looks at the first printer a person can see that has an address. It followed the printer the service
    /// started with, so a fresh install that added its first printer later, or a farm whose first printer was taken away, never
    /// watched anything (finish-2 F06): it follows the farm now, and starts again on the new address when the printer changes.
    fn refresh_watched(&self) {
        let first = self.watch_target();
        let changed = match &first {
            Some(p) => crate::remote::set_watched(&p.id, &p.cfg.moonraker_url),
            None => crate::remote::set_watched("", ""),
        };
        if changed && !self.base.database.is_empty() {
            crate::remote::rewatch(&self.base);
        }
    }

    /// Open one printer's recording and start its collector. This is what `main` used to do once.
    fn launch(&self, entry: &PrinterEntry, warnings: Vec<String>) -> Result<Arc<Printer>> {
        self.launch_in(&self.base, entry, warnings, None)
    }

    /// `launch`, with the configuration the printer's own copy is made from (the placeholder of an emptied farm needs the
    /// "nothing is configured" one, whatever the file this service started from said).
    fn launch_in(&self, base: &Config, entry: &PrinterEntry, warnings: Vec<String>, reuse_lock: Option<File>) -> Result<Arc<Printer>> {
        let cfg = base.for_printer(entry, &self.config_dir())?;
        cfg.validate()?;
        let readonly = self.mode == "replay";
        let lock = if readonly {
            None
        } else if reuse_lock.is_some() {
            reuse_lock   // a restart of the same printer keeps the lock it holds on its own recording
        } else {
            if let Some(dir) = Path::new(&cfg.database).parent() {
                if !dir.as_os_str().is_empty() {
                    fs::create_dir_all(dir).map_err(|e| {
                        format!(
                            "cannot create {}: {e}; grant write permission",
                            dir.display()
                        )
                    })?;
                }
            }
            let path = format!("{}.lock", cfg.database);
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| format!("cannot open recorder lock {path}: {e}; create the parent directory and grant write permission"))?;
            file.try_lock().map_err(|e| {
                format!(
                    "another recorder may already own {}: {e}; stop it or choose a different database",
                    cfg.database
                )
            })?;
            Some(file)
        };
        let db = Store::open(&cfg.database, readonly)?;
        if !readonly {
            let origin = db.context_at(f64::MAX, "scope_origin")?;
            let expected = if self.demo { "demo" } else { "live" };
            if origin
                .as_ref()
                .and_then(|v| v.as_str())
                .is_some_and(|v| v != expected)
                || (self.demo && origin.is_none() && db.bounds()?.0.is_some())
            {
                return Err(
                    "refusing to mix demonstration and real recordings; select a new --db file"
                        .into(),
                );
            }
        }
        let (a, b, c) = db.counts()?;
        let (_, last) = db.bounds()?;
        let state: Shared = Arc::new(Mutex::new(CollectorStatus {
            mode: self.mode.clone(),
            samples: a,
            events: b,
            qgl_rounds: c,
            last_success: last,
            degraded: warnings,
            ..CollectorStatus::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        // A printer with no address (the placeholder of a service with no printer) has nothing to collect from:
        // no thread, no socket, nothing to read (finish-2 F01).
        let nowhere = cfg.driver != "demo" && cfg.moonraker_url.trim().is_empty();
        if !readonly && !nowhere {
            let (worker_cfg, worker_state, worker_stop, demo) =
                (cfg.clone(), state.clone(), stop.clone(), self.demo);
            let id = entry.id.clone();
            thread::Builder::new()
                .name(format!("collector-{id}"))
                .stack_size(1024 * 1024)
                .spawn(move || {
                    let status = worker_state.clone();
                    let ended = worker_stop.clone();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        collector::run(worker_cfg, demo, worker_state, db, worker_stop)
                    }));
                    if ended.load(Ordering::Relaxed) {
                        return; // removed from the farm: not an error
                    }
                    let error = match result {
                        Ok(Err(e)) => e,
                        Ok(Ok(())) => "recording of this printer stopped unexpectedly; restart Stella Scope".into(),
                        Err(_) => "recording of this printer failed; what was recorded is kept, restart Stella Scope to record again".into(),
                    };
                    eprintln!("{id}: {error}");
                    if let Ok(mut s) = status.lock() {
                        s.connected = false;
                        s.last_error = Some(error);
                    };
                })
                .map_err(|e| format!("cannot start recording this printer: {e}; free memory and restart"))?;
        }
        // the timelapse of each print (2026-10-26): a live, real printer only; a demo printer has no camera
        if !readonly && !nowhere && cfg.driver != "demo" && self.mode == "live" {
            crate::timelapse::spawn(cfg.clone(), state.clone(), stop.clone(), entry.id.clone());
        }
        let controls = Arc::new(commands::Service::new(&cfg, &self.mode)?);
        commands::Service::spawn_heat_guard(&controls, stop.clone());
        Ok(Arc::new(Printer {
            id: entry.id.clone(),
            entry: entry.clone(),
            cfg,
            state,
            controls,
            stop,
            _lock: lock,
        }))
    }

    fn writable(&self) -> Result<()> {
        if self.mode != "live" {
            return Err(format!("printers cannot be changed in {} mode", self.mode));
        }
        if self.token.is_none() {
            return Err(
                "printer controls are switched off, so printers cannot be added or changed here; the Help guide, under Controls, shows how to switch them on"
                    .into(),
            );
        }
        Ok(())
    }

    fn entries_now(&self) -> Vec<PrinterEntry> {
        let mut out: Vec<PrinterEntry> = self.visible().iter().map(|p| p.entry.clone()).collect();
        if let Ok(f) = self.failed.lock() {
            out.extend(f.iter().map(|(e, _)| e.clone()));
        }
        out
    }

    /// Add a printer: validate, write its credentials (mode 600) and the configuration (atomic, with a
    /// dated backup), start its collector. Nothing is saved if any step fails.
    pub fn add(
        &self,
        entry: PrinterEntry,
        secret: Option<BTreeMap<String, String>>,
    ) -> Result<Arc<Printer>> {
        self.add_one(entry, secret, true)
    }

    /// `add`, with the choice of writing the configuration now or leaving that to the caller, which
    /// adds several printers and writes once (the demo farm).
    fn add_one(
        &self,
        mut entry: PrinterEntry,
        secret: Option<BTreeMap<String, String>>,
        save: bool,
    ) -> Result<Arc<Printer>> {
        self.writable()?;
        if entry.id.is_empty() {
            entry.id = slug(&entry.name);
        }
        // `/api/printers/demo-farm` adds or removes the demo farm, so no printer may be called that.
        if entry.id == "demo-farm" {
            return Err("that name is the demo farm's button; choose another name".into());
        }
        if entry.driver.is_empty() {
            entry.driver = "moonraker".into();
        }
        if self.entries_now().iter().any(|e| e.id == entry.id) {
            return Err(format!("a printer with id {} already exists", entry.id));
        }
        if secret.is_some() && entry.credentials_ref.is_none() {
            entry.credentials_ref = Some(entry.id.clone());
        }
        // Validate the whole resulting configuration before touching disk.
        let mut check = self.base.clone();
        check.printers = self.entries_now();
        check.printers.push(entry.clone());
        check.validate()?;
        let secret_path = match (&entry.credentials_ref, &secret) {
            (Some(r), Some(s)) => Some(write_secret(&self.config_dir(), r, s)?),
            _ => None,
        };
        // WO-14 item 4: a printer that was not given a bed size gets it from the machine's own
        // report (a read-only query), never the 350 mm default of another machine. The queue's
        // size matching depends on this. Done after the login file exists so for_printer can load it.
        if entry.bed_size_mm.is_none() || entry.model.is_none() {
            if let Some(mut d) = self
                .base
                .for_printer(&entry, &self.config_dir())
                .ok()
                .and_then(|cfg| crate::driver::open(&cfg).ok())
            {
                if entry.model.is_none() {
                    entry.model = d.model_hint();
                    // a bridged machine's size comes from what it is: ask again knowing the model
                    if let Some(again) = self
                        .base
                        .for_printer(&entry, &self.config_dir())
                        .ok()
                        .and_then(|cfg| crate::driver::open(&cfg).ok())
                    {
                        d = again;
                    }
                }
                if entry.bed_size_mm.is_none() {
                    entry.bed_size_mm = d.bed_hint();
                }
            }
        }
        // Finish-2 F09 (2026-10-19): a PrusaLink printer that names its machine starts with that machine's own defaults for the queue (the name its sliced
        // files carry, and its nozzle), as long as the person did not set them. The 3D model is the person's choice and is never set here.
        if entry.driver == "prusalink" {
            if let Some(m) = entry.model.as_deref().and_then(crate::prusa_machines::for_short) {
                if entry.slicer_model.is_none() {
                    entry.slicer_model = m.slicer.first().map(|s| (*s).to_owned());
                }
                if entry.nozzle_mm.is_none() {
                    entry.nozzle_mm = Some(m.nozzle);
                }
            }
        }
        // WO-13 (2026-10-12): a new printer never opens a file an earlier printer with the same id
        // left behind (the old one showed "LAST SEEN 1.7 H AGO" and a stale position). Rename it
        // with the date, the same as remove() does.
        if entry.database.is_none() {
            let database = self.base.recording_for(&entry);
            if Path::new(&database).exists() {
                let stamp = utc_stamp(model::now() as u64);
                let renamed = format!("{database}.replaced-{stamp}");
                if fs::rename(&database, &renamed).is_ok() {
                    for suffix in ["-wal", "-shm"] {
                        let _ =
                            fs::rename(format!("{database}{suffix}"), format!("{renamed}{suffix}"));
                    }
                }
            }
        }
        // A real printer arriving beside the placeholder replaces it, and leads the farm in its place (polish WO-06 item 7).
        let retiring = entry.driver != "demo" && self.placeholder_printer().is_some();
        let launched = self.launch(&entry, Vec::new());
        let printer = match launched {
            Ok(p) => p,
            Err(e) => {
                if let Some(p) = secret_path {
                    let _ = fs::remove_file(p);
                }
                return Err(e);
            }
        };
        if let Err(e) = save
            .then(|| {
                self.persist(&{
                    let mut all = self.entries_now();
                    if retiring {
                        all.insert(0, entry.clone());
                    } else {
                        all.push(entry.clone());
                    }
                    all
                })
            })
            .unwrap_or(Ok(()))
        {
            printer.stop();
            if let Some(p) = secret_path {
                let _ = fs::remove_file(p);
            }
            return Err(e);
        }
        self.printers
            .write()
            .map_err(|_| "farm lock poisoned")?
            .push(printer.clone());
        if retiring {
            self.retire_placeholder();
            if let Ok(mut all) = self.printers.write() {
                if let Some(at) = all.iter().position(|p| p.id == printer.id) {
                    let me = all.remove(at);
                    all.insert(0, me);
                }
            }
        }
        // the failure watch looks at the first real printer there is, which a farm that started empty did not have until now (finish-2 F06)
        self.refresh_watched();
        // WO-04: the sixth real printer starts a 30-day trial (one trial per farm). A licence that
        // covers the farm, or being at five or fewer, means no trial. The trial start is stored in
        // the config so it survives a restart.
        if entry.driver != "demo" && self.trial_started.lock().ok().is_some_and(|t| t.is_none()) {
            let real = self
                .entries_now()
                .iter()
                .filter(|e| e.driver != "demo")
                .count();
            if real >= 6 {
                let st = self.licence_status();
                let covered = st
                    .get("licence")
                    .and_then(|l| l.get("printers"))
                    .and_then(|p| p.as_u64())
                    .is_some_and(|n| n as usize >= real);
                if !covered {
                    let now = model::now();
                    self.write_licence_trial_started(Some(now));
                    if let Ok(mut t) = self.trial_started.lock() {
                        *t = Some(now);
                    }
                }
            }
        }
        Ok(printer)
    }

    /// "Try the demo farm" (polish WO-02): add every printer of `demo::demo_farm` that is not there yet,
    /// with one write of the configuration. Demo printers are simulations on this machine: they take no
    /// address, never count toward the farm licence, and are badged DEMO everywhere. Nothing real is
    /// touched. Reports what was added, what was already there and which of their models are not installed.
    pub fn add_demo_farm(&self) -> Result<Value> {
        self.writable()?;
        let have: Vec<String> = self.entries_now().iter().map(|e| e.id.clone()).collect();
        let (mut added, mut skipped): (Vec<Arc<Printer>>, Vec<String>) = (Vec::new(), Vec::new());
        for entry in crate::demo::demo_farm() {
            if have.contains(&entry.id) {
                skipped.push(entry.id);
                continue;
            }
            match self.add_one(entry, None, false) {
                Ok(p) => added.push(p),
                Err(e) => {
                    self.withdraw(&added);
                    return Err(e);
                }
            }
        }
        if !added.is_empty() {
            if let Err(e) = self.persist(&self.entries_now()) {
                self.withdraw(&added);
                return Err(e);
            }
        }
        let lib = self.library();
        let missing: Vec<Value> = crate::demo::demo_farm()
            .iter()
            .filter_map(|e| e.cad_model.as_deref())
            .filter(|m| lib.model_file(m).is_none())
            .filter_map(|m| crate::printer_models::machine(m))
            .map(|m| json!({"id":m["id"],"label":m["label"],"licence":m["licence"]["text"]}))
            .collect();
        Ok(json!({
            "added":added.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            "already_there":skipped,
            "missing_models":missing,
        }))
    }

    /// Take printers that were just started back out, as if they had never been added.
    fn withdraw(&self, printers: &[Arc<Printer>]) {
        for p in printers {
            p.stop();
            crate::drivers::forget(&p.id);
        }
        if let Ok(mut all) = self.printers.write() {
            all.retain(|p| !printers.iter().any(|q| q.id == p.id));
        }
    }

    /// "Remove the demo farm": every demo printer goes, nothing else. A real printer is never touched,
    /// whatever it is called. The first printer carries the phone link and the failure watch, so if it
    /// is itself a demo it stays (`kept_first`). The recordings of the demos are deleted: they hold
    /// nothing but a simulation.
    pub fn remove_demo_farm(&self) -> Result<Value> {
        self.writable()?;
        let first = self.lead().id.clone();
        let (gone, kept): (Vec<PrinterEntry>, Vec<PrinterEntry>) = self
            .entries_now()
            .into_iter()
            .partition(|e| e.driver == "demo" && e.id != first);
        let kept_first = kept.iter().any(|e| e.driver == "demo" && e.id == first);
        if gone.is_empty() {
            return Ok(json!({"removed":[],"kept_first":kept_first}));
        }
        self.persist(&kept)?;
        let ids: Vec<String> = gone.iter().map(|e| e.id.clone()).collect();
        for id in &ids {
            if let Some(p) = self.get(id) {
                p.stop();
            }
            crate::drivers::forget(id);
        }
        self.printers
            .write()
            .map_err(|_| "farm lock poisoned")?
            .retain(|p| !ids.contains(&p.id));
        if let Ok(mut f) = self.failed.lock() {
            f.retain(|(e, _)| !ids.contains(&e.id));
        }
        // Give the collectors a moment to close their recordings before the files go.
        thread::sleep(std::time::Duration::from_millis(300));
        for e in &gone {
            let database = self.base.recording_for(e);
            if database.is_empty() {
                continue;
            }
            for suffix in [
                "",
                "-wal",
                "-shm",
                ".lock",
                ".commands.sqlite3",
                ".resonance.sqlite3",
            ] {
                let _ = fs::remove_file(format!("{database}{suffix}"));
            }
        }
        Ok(json!({"removed":ids,"kept_first":kept_first}))
    }

    /// Rewrite the configuration file with `change` applied to its JSON, every other key as it was: a dated copy of the old file first,
    /// then an atomic replace. Nothing is written when there is no file.
    fn patch_config(&self, change: impl FnOnce(&mut Value)) -> Result<()> {
        let path = self.config_path.as_ref().ok_or(
            "this service was started without --config FILE, so settings cannot be saved; start it with a configuration file",
        )?;
        let text = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut value: Value = serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
        change(&mut value);
        let stamp = utc_stamp(model::now() as u64);
        let mut backup = PathBuf::from(format!("{}.bak-{stamp}", path.display()));
        for n in 1.. {
            if !backup.exists() {
                break;
            }
            backup = PathBuf::from(format!("{}.bak-{stamp}-{n}", path.display()));
        }
        fs::copy(path, &backup).map_err(|e| format!("cannot keep a dated copy at {}: {e}", backup.display()))?;
        let temp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut f = File::create(&temp)?;
            f.write_all(serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
            if let Ok(meta) = fs::metadata(path) {
                let _ = fs::set_permissions(&temp, meta.permissions());
            }
            fs::rename(&temp, path)
        })();
        result.map_err(|e| {
            let _ = fs::remove_file(&temp);
            format!("cannot save {}: {e}; nothing was changed", path.display())
        })
    }

    /// Store the trial start (epoch seconds) in the configuration file, rewriting just that field.
    fn write_licence_trial_started(&self, started: Option<f64>) {
        let _ = self.patch_config(|v| v["licence_trial_started"] = serde_json::to_value(started).unwrap_or(Value::Null));
    }

    /// Phone access was switched off and the person wants a phone (finish-2 F06, 2026-10-19): the page used to say "Enable remote_link", a name
    /// in a file. This writes it: `remote_link` on and a `lan_port` (the file's own, else the first free port two above this service's).
    /// The phone's door is opened when Stella Scope starts, so it works from the next start; the answer says so.
    pub fn enable_phone_access(&self) -> Result<Value> {
        self.writable()?;
        let port = self.base.lan_port.unwrap_or_else(|| {
            (self.base.port.saturating_add(2)..self.base.port.saturating_add(40))
                .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
                .unwrap_or(self.base.port.saturating_add(2))
        });
        self.patch_config(|v| {
            v["remote_link"] = json!(true);
            if v.get("lan_port").is_none_or(|p| p.is_null()) {
                v["lan_port"] = json!(port);
            }
        })?;
        let restart_needed = !(self.base.remote_link && self.base.lan_port.is_some());
        if restart_needed {
            crate::remote::PHONE_PENDING.store(true, Ordering::SeqCst);
        }
        Ok(json!({"enabled":true,"lan_port":port,"restart_needed":restart_needed}))
    }

    /// Type a printer's login (finish-2 F06, 2026-10-19). A farm restored from a backup, or a printer whose password changed, said "Type the login in Printers"
    /// and Printers had nowhere to type it ("remove the printer and add it again"). The login is written beside the others (mode 600, never in the
    /// configuration file) and the printer starts again on it, keeping its recording and its place in the farm. A wrong login is said by the printer's own state.
    pub fn set_login(&self, id: &str, secret: BTreeMap<String, String>) -> Result<PrinterEntry> {
        self.writable()?;
        if secret.values().all(|v| v.trim().is_empty()) {
            return Err("type the login first".into());
        }
        let mut entries = self.entries_now();
        let entry = entries.iter_mut().find(|e| e.id == id).ok_or_else(|| format!("no printer called {id}"))?;
        if entry.driver == "demo" {
            return Err("a demo printer has no login".into());
        }
        let secret: BTreeMap<String, String> = secret.into_iter().map(|(k, v)| (k, v.trim().to_owned())).filter(|(_, v)| !v.is_empty()).collect();
        let reference = entry.credentials_ref.clone().unwrap_or_else(|| entry.id.clone());
        write_secret(&self.config_dir(), &reference, &secret)?;
        let first_time = entry.credentials_ref.is_none();
        entry.credentials_ref = Some(reference);
        let updated = entry.clone();
        if first_time {
            self.persist(&entries)?;
        }
        let old = self.get(id).ok_or_else(|| format!("no printer called {id}"))?;
        old.stop();
        crate::drivers::forget(id);
        let lock = old._lock.as_ref().and_then(|f| f.try_clone().ok());
        let at = self.list().iter().position(|p| p.id == id);
        // the collector ends at its next pass (a request in flight can take a moment); then the same printer starts on the new login
        thread::sleep(std::time::Duration::from_millis(1500));
        drop(old);
        let printer = self.launch_in(&self.base, &updated, Vec::new(), lock)?;
        let mut list = self.printers.write().map_err(|_| "farm lock poisoned")?;
        match at.filter(|i| *i < list.len() && list[*i].id == id) {
            Some(i) => list[i] = printer,
            None => {
                list.retain(|p| p.id != id);
                list.push(printer);
            }
        }
        if let Ok(mut f) = self.failed.lock() {
            f.retain(|(e, _)| e.id != id);
        }
        Ok(updated)
    }

    /// Rename or recolour a printer, bind its model, or set its camera. Only these fields: its address and driver
    /// change by removing it and adding it again, so a recording is never silently re-pointed.
    pub fn patch(&self, id: &str, patch: &Value) -> Result<PrinterEntry> {
        self.writable()?;
        let object = patch.as_object().ok_or("send a JSON object")?;
        for key in object.keys() {
            if ![
                "name",
                "accent",
                "model_asset",
                "model",
                "camera",
                "nozzle_mm",
                "slicer_model",
                "material",
                "cad_model",
                "cad_calibration",
                "cad_options",
                "bed_size_mm",
                "plug",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("{key} cannot be changed here; remove the printer and add it again to change its address or brand"));
            }
        }
        let mut entries = self.entries_now();
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("no printer called {id}"))?;
        if let Some(v) = object.get("name") {
            entry.name = v.as_str().ok_or("name must be text")?.trim().to_owned();
        }
        if let Some(v) = object.get("plug") {
            entry.plug = match v {
                Value::Null => None,
                Value::String(t) if t.trim().is_empty() => None,
                Value::String(t) => {
                    crate::plugs::parse(t)?;
                    Some(t.trim().to_owned())
                }
                _ => return Err("plug must be text like shelly://192.168.1.50, or null".into()),
            };
        }
        if let Some(v) = object.get("nozzle_mm") {
            entry.nozzle_mm = match v {
                Value::Null => None,
                Value::Number(n) => Some(
                    n.as_f64()
                        .filter(|x| (0.1..=2.0).contains(x))
                        .ok_or("nozzle_mm must be between 0.1 and 2.0")?,
                ),
                _ => return Err("nozzle_mm must be a number or null".into()),
            };
        }
        if let Some(v) = object.get("cad_model") {
            entry.cad_model = match v {
                Value::Null => None,
                Value::String(s) if s.is_empty() => None,
                Value::String(s) if crate::printer_models::machine(s).is_some() => Some(s.clone()),
                Value::String(s) => return Err(format!("{s} is not a model in the catalogue")),
                _ => return Err("cad_model must be text or null".into()),
            };
            // A new model starts from its own sensible calibration unless one is sent with it.
            if !object.contains_key("cad_calibration") {
                entry.cad_calibration = None;
            }
            // The options were chosen for the old machine; a new model starts with none unless some are sent with it.
            if !object.contains_key("cad_options") {
                entry.cad_options = None;
            }
        }
        if let Some(v) = object.get("bed_size_mm") {
            // WO-14 item 4: the bed size stays editable in printer settings; queue matching reads it.
            entry.bed_size_mm = match v {
                Value::Null => None,
                Value::Array(a) if a.len() == 2 => {
                    let x = a[0]
                        .as_f64()
                        .filter(|x| (50.0..=2000.0).contains(x))
                        .ok_or("bed_size_mm values must be between 50 and 2000")?;
                    let y = a[1]
                        .as_f64()
                        .filter(|y| (50.0..=2000.0).contains(y))
                        .ok_or("bed_size_mm values must be between 50 and 2000")?;
                    Some([x, y])
                }
                _ => return Err("bed_size_mm must be two millimetre numbers or null".into()),
            };
        }
        if let Some(v) = object.get("cad_options") {
            // Checked against the machine as it will be after this change (the model may be changing in the same patch).
            let machine = entry
                .cad_model
                .as_deref()
                .ok_or("choose a model for this printer before choosing options for it")?;
            let checked = crate::printer_models::check_selection(machine, v)?;
            entry.cad_options = checked
                .as_array()
                .filter(|a| !a.is_empty())
                .map(|_| checked.clone());
        }
        if let Some(v) = object.get("cad_calibration") {
            entry.cad_calibration = match v {
                Value::Null => None,
                v => Some(crate::printer_models::Calibration::from_value(v)?.to_value()),
            };
        }
        for (key, slot) in [
            ("slicer_model", &mut entry.slicer_model),
            ("material", &mut entry.material),
            ("accent", &mut entry.accent),
            ("model_asset", &mut entry.model_asset),
            ("model", &mut entry.model),
            ("camera", &mut entry.camera),
        ] {
            if let Some(v) = object.get(key) {
                *slot = match v {
                    Value::Null => None,
                    Value::String(s) if s.is_empty() => None,
                    Value::String(s) => Some(s.clone()),
                    _ => return Err(format!("{key} must be text or null")),
                };
            }
        }
        let updated = entry.clone();
        let mut check = self.base.clone();
        check.printers = entries.clone();
        check.validate()?;
        self.persist(&entries)?;
        // The running printer takes the new name and look at once.
        let mut list = self.printers.write().map_err(|_| "farm lock poisoned")?;
        if let Some(slot) = list.iter_mut().find(|p| p.id == id) {
            let old = slot.clone();
            let cfg = self.base.for_printer(&updated, &self.config_dir())?;
            *slot = Arc::new(Printer {
                id: old.id.clone(),
                entry: updated.clone(),
                cfg,
                state: old.state.clone(),
                controls: old.controls.clone(),
                stop: old.stop.clone(),
                _lock: old._lock.as_ref().and_then(|f| f.try_clone().ok()),
            });
        }
        Ok(updated)
    }

    /// Remove a printer. Its recording is kept, renamed with the date, unless `delete_data`.
    pub fn remove(&self, id: &str, delete_data: bool) -> Result<Value> {
        self.writable()?;
        // Finish-2 F06 (2026-10-19, F01 and F02 both met it): the first printer can be taken away like any other. It was refused ("it carries the
        // phone link and the failure watch"), so a wrong address typed for the first printer could be neither changed nor removed. The farm's next
        // printer leads from then on, the failure watch follows it, and a farm with no printer left is a fresh install again.
        let mut entries = self.entries_now();
        let pos = entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| format!("no printer called {id}"))?;
        let entry = entries.remove(pos);
        // With no real printer left, a file that still carries the old single-printer address would bring the printer back at the next start.
        self.persist_with(&entries, !entries.iter().any(|e| e.driver != "demo"))?;
        if let Some(p) = self.get(id) {
            p.stop();
        }
        crate::drivers::forget(id);
        self.printers
            .write()
            .map_err(|_| "farm lock poisoned")?
            .retain(|p| p.id != id);
        if let Ok(mut f) = self.failed.lock() {
            f.retain(|(e, _)| e.id != id);
        }
        if self.list().is_empty() {
            self.revive_placeholder()?;
        }
        self.refresh_watched();
        // Credentials nobody else references go with the printer.
        if let Some(r) = &entry.credentials_ref {
            if !entries
                .iter()
                .any(|e| e.credentials_ref.as_ref() == Some(r))
            {
                let _ = fs::remove_file(
                    self.config_dir()
                        .join("printers")
                        .join(format!("{r}.secret")),
                );
            }
        }
        let database = self.base.recording_for(&entry);
        let mut kept = Value::Null;
        if !database.is_empty() {
            // Give the collector a moment to close the recording before it is renamed.
            thread::sleep(std::time::Duration::from_millis(300));
            if delete_data {
                for suffix in [
                    "",
                    "-wal",
                    "-shm",
                    ".lock",
                    ".commands.sqlite3",
                    ".resonance.sqlite3",
                ] {
                    let _ = fs::remove_file(format!("{database}{suffix}"));
                }
            } else {
                let stamp = utc_stamp(model::now() as u64);
                let renamed = format!("{database}.removed-{stamp}");
                if fs::rename(&database, &renamed).is_ok() {
                    for suffix in ["-wal", "-shm"] {
                        let _ =
                            fs::rename(format!("{database}{suffix}"), format!("{renamed}{suffix}"));
                    }
                    kept = json!(renamed);
                }
            }
        }
        Ok(json!({"removed":id,"recording_kept_as":kept,"recording_deleted":delete_data}))
    }

    /// Write the `printers` array into the configuration file: atomic (temp file, fsync, rename) and
    /// with a dated copy of the file it replaces. Every other key is kept as it was.
    fn persist(&self, entries: &[PrinterEntry]) -> Result<()> {
        self.persist_with(entries, false)
    }

    /// `persist`; `forget_address` also empties the file's own single-printer address (the farm has no real printer any more).
    fn persist_with(&self, entries: &[PrinterEntry], forget_address: bool) -> Result<()> {
        let path = self.config_path.as_ref().ok_or(
            "this service was started without --config FILE, so printers cannot be saved; start it with a configuration file",
        )?;
        let text =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut value: Value = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
        value["printers"] = serde_json::to_value(entries).map_err(|e| e.to_string())?;
        if forget_address && value.get("moonraker_url").is_some_and(|u| u.as_str().is_some_and(|u| !u.trim().is_empty())) {
            value["moonraker_url"] = json!("");
        }
        // A dated copy of the file being replaced. Two writes in one second get -1, -2, ...: a copy is
        // never overwritten.
        let stamp = utc_stamp(model::now() as u64);
        let mut backup = PathBuf::from(format!("{}.bak-{stamp}", path.display()));
        for n in 1.. {
            if !backup.exists() {
                break;
            }
            backup = PathBuf::from(format!("{}.bak-{stamp}-{n}", path.display()));
        }
        fs::copy(path, &backup)
            .map_err(|e| format!("cannot keep a dated copy at {}: {e}", backup.display()))?;
        let temp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut f = File::create(&temp)?;
            f.write_all(
                serde_json::to_string_pretty(&value)
                    .map_err(std::io::Error::other)?
                    .as_bytes(),
            )?;
            f.write_all(b"\n")?;
            f.sync_all()?;
            if let Ok(meta) = fs::metadata(path) {
                let _ = fs::set_permissions(&temp, meta.permissions());
            }
            fs::rename(&temp, path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(format!(
                "cannot save {}: {e}; nothing was changed",
                path.display()
            ));
        }
        Ok(())
    }

    /// `GET /api/printers`: every printer at a glance. State words are the ones a person says.
    pub fn summary(&self) -> Value {
        let now = model::now();
        let first = self.first().id.clone();
        let mut rows: Vec<Value> = self
            .visible()
            .iter()
            .map(|p| summarize(p, &first, now))
            .collect();
        if let Ok(f) = self.failed.lock() {
            for (e, why) in f.iter() {
                rows.push(json!({
                    "id":e.id,"name":e.name,"driver":if e.driver.is_empty(){"moonraker"}else{&e.driver},"address":e.address,
                    "online":false,"state":"error","detail":why,"default":false,"accent":e.accent,
                    "model_asset":e.model_asset,"model":e.model,
                }));
            }
        }
        json!({
            "printers":rows,
            "default":first,
            "mode":self.mode,
            "writable":self.mode=="live" && self.token.is_some() && self.config_path.is_some(),
            "token":self.token,
            "count":rows.len(),
        })
    }
}

/// One printer's line for the wall: what is true now, from the collector's own cached tick.
fn summarize(p: &Printer, first: &str, now: f64) -> Value {
    let s: CollectorStatus = p.state.lock().map(|g| g.clone()).unwrap_or_default();
    let tick = s.last_tick.clone().unwrap_or(Value::Null);
    let point = &tick["point"];
    let job = &tick["job"];
    let progress = job["progress"].as_f64();
    let duration = job["duration"].as_f64();
    // Finish-2 F02 (CORE One L, 2026-10-19): the printer's own figure first. Worked out from progress alone it said 1 h 27 min
    // while the CORE One L said 14 min: early progress counts the heating and probing as if they were printing.
    let time_left = job["time_left"].as_f64().or(match (progress, duration) {
        (Some(p), Some(d)) if p > 0.01 && d >= 0.0 => Some((d / p - d).max(0.0)),
        _ => None,
    });
    let temp = |k: &str| json!({"temp":point[k]["temperature"],"target":point[k]["target"]});
    let (nozzle, bed) = (temp("nozzle"), temp("bed"));
    let job_state = job["state"].as_str().unwrap_or("");
    let online = s.connected && s.printer_state == "ready";
    let heating = [&nozzle, &bed].iter().any(|h| {
        matches!((h["temp"].as_f64(), h["target"].as_f64()), (Some(t), Some(g)) if g > 0.0 && (t - g).abs() > 3.0)
    });
    let state = if s.printer_state == "not_ready" {
        "error"
    } else if !online {
        "offline"
    } else {
        match job_state {
            // A print that has not begun and whose heaters are still climbing is warming up, not printing
            // (polish WO-02: the demo farm's wall shows one). Klipper reports "printing" through PRINT_START.
            "printing" if heating && progress.unwrap_or(0.0) <= 0.0 => "heating",
            "printing" => "printing",
            "paused" => "paused",
            "error" => "error",
            "complete" | "cancelled" if job["name"].as_str().is_some_and(|n| !n.is_empty()) => {
                "finished"
            }
            _ if heating => "heating",
            _ => "idle",
        }
    };
    let file = job["name"].as_str().unwrap_or("");
    let caps = crate::driver::open(&p.cfg)
        .map(|d| (d.capabilities(), d.brand()))
        .ok();
    json!({
        "id":p.id,
        "name":p.entry.name,
        "driver":p.cfg.driver,
        "brand":caps.as_ref().map(|c| c.1),
        "address":p.cfg.moonraker_url,
        // whether a login is saved (never the login itself): Settings says "a login is saved" or offers to type one
        "login_saved":!p.cfg.secret.0.is_empty(),
        // a printer that is not answering because it has no login yet (a restored backup holds none) or the one it has is refused: the tile says so, and where to type it
        "needs_login":p.cfg.driver != "demo" && p.cfg.driver != "moonraker" && !online
            && (p.cfg.secret.0.is_empty() || s.last_error.as_deref().is_some_and(|e| { let e = e.to_ascii_lowercase(); e.contains("login") || e.contains("check code") })),
        "model":p.cfg.printer_model,
        "accent":p.entry.accent,
        "camera":p.entry.camera,
        "nozzle_mm":p.entry.nozzle_mm,
        "slicer_model":p.entry.slicer_model,
        "material":p.entry.material,
        "bed_mm":p.cfg.bed_size_mm,
        "model_asset":p.entry.model_asset,
        // WO-14 item 1: true when this printer's own scene model (the machine the main page draws)
        // is in its managed library and so can stand as its farm-wall model with no setting.
        "scene_model":crate::assets::has_reference_model(&p.cfg.database),
        "cad_model":p.entry.cad_model,
        "plug":crate::plugs::status(&p.id, p.entry.plug.as_deref()),
        "cad_calibration":p.entry.cad_calibration,
        "cad_options":p.entry.cad_options,
        "default":p.id == first,
        "online":online,
        "state":state,
        "printer_state":s.printer_state,
        // Polish WO-06 (live finding, 2026-10-19): a paused or stopped print says why in the printer's own words (a PrusaLink in
        // ATTENTION: "check its screen"), which used to reach neither the wall nor the page.
        "detail":s.printer_message.clone().or(s.last_error.clone()).or_else(|| {
            job["message"].as_str().filter(|m| !m.is_empty() && matches!(state, "paused" | "error")).map(str::to_owned)
        }),
        "job_state":job_state,
        "file":file,
        "progress":progress,
        "time_left_s":time_left,
        "eta_ts":time_left.map(|l| now + l),
        "temps":{"nozzle":nozzle,"bed":bed},
        "thumbnail_url":(!file.is_empty()).then(|| format!("/api/p/{}/print/thumbnail?filename={}", p.id, crate::moonraker::encode(file))),
        "camera_url":caps.as_ref().filter(|c| c.0.camera).map(|_| format!("/api/p/{}/camera/snapshot", p.id)),
        "capabilities":caps.map(|c| c.0),
        "tick_age_s":s.last_tick_ts.map(|t| (now - t).max(0.0)),
        "controls":p.controls_enabled(),
    })
}

/// A demo printer's own state right now (polish WO-03): see `demo::peek`. None for every other printer.
fn demo_now(p: &Printer) -> Option<Value> {
    (p.cfg.driver == "demo").then(|| crate::demo::peek(&p.cfg)).flatten()
}

/// The position the printer reports now, from the collector's cached tick, and how old it is.
pub fn position_of(p: &Printer, now: f64) -> ([Option<f64>; 3], Option<f64>) {
    if let Some(r) = demo_now(p) {
        let at = |i: usize| r["motion_report"]["live_position"][i].as_f64();
        return ([at(0), at(1), at(2)], Some(0.0));
    }
    let s: CollectorStatus = p.state.lock().map(|g| g.clone()).unwrap_or_default();
    let point = s
        .last_tick
        .as_ref()
        .map_or(Value::Null, |t| t["point"].clone());
    // Finish-2 F05 (Opus supplement 1, item 2): Klipper reports X 0 Y 0 Z 0 for an axis it has not homed; that is not where the
    // head is, so an unhomed axis is unknown here (the tile and the model leave the head where it was last seen). Brands that
    // never report homing (PrusaLink, FlashForge: no homed axes at all) keep what they report.
    let homed = s
        .last_tick
        .as_ref()
        .and_then(|t| t["homed_axes"].as_str().map(str::to_ascii_lowercase));
    let reports_homing = p.cfg.driver == "moonraker";
    let at = |i: usize| {
        let axis = ["x", "y", "z"][i];
        point["position"][i]
            .as_f64()
            .filter(|_| !reports_homing || homed.as_deref().is_some_and(|h| h.contains(axis)))
    };
    (
        [at(0), at(1), at(2)],
        s.last_tick_ts.map(|t| (now - t).max(0.0)),
    )
}

/// The tool the printer says is active (IDEX, toolchanger), when it says.
pub fn tool_of(p: &Printer) -> Option<usize> {
    if let Some(r) = demo_now(p) {
        return r["toolchanger"]["tool_number"].as_u64().map(|t| t as usize).or_else(|| match r["toolhead"]["extruder"].as_str()?.strip_prefix("extruder")? {
            "" => Some(0),
            n => n.parse().ok(),
        });
    }
    let s: CollectorStatus = p.state.lock().map(|g| g.clone()).unwrap_or_default();
    s.last_tick.as_ref()?["point"]["tool"]
        .as_u64()
        .map(|t| t as usize)
}

/// What the printer says about the print it is running, for the model to follow the file: the file, how far
/// the printer has read it, the layer, the active tool. Taken from the collector's cached tick.
pub fn follow_of(p: &Printer) -> Value {
    if let Some(r) = demo_now(p) {
        let n = |v: &Value| v.as_f64().filter(|x| x.is_finite());
        return json!({
            "file":r["print_stats"]["filename"],"state":r["print_stats"]["state"],"progress":n(&r["virtual_sdcard"]["progress"]),
            "file_position":n(&r["virtual_sdcard"]["file_position"]),"file_size":n(&r["virtual_sdcard"]["file_size"]),
            "layer":n(&r["print_stats"]["info"]["current_layer"]),"layers":n(&r["print_stats"]["info"]["total_layer"]),
            "tool":tool_of(p),"toolchanger":r["toolchanger"].is_object(),
        });
    }
    let s: CollectorStatus = p.state.lock().map(|g| g.clone()).unwrap_or_default();
    follow_from(&s.last_tick.unwrap_or(Value::Null))
}

/// What a tick says about the print the model follows (the live one, or one the recorder held: finish-2 F02).
fn follow_from(tick: &Value) -> Value {
    let (point, job) = (&tick["point"], &tick["job"]);
    json!({
        "file":job["name"],"state":job["state"],"progress":job["progress"],
        "file_position":point["file_position"],"file_size":point["file_size"],
        "layer":job["layer"],"layers":job["layers"],"tool":point["tool"],
        "toolchanger":point["toolchanger"].as_bool().unwrap_or(false),
    })
}

/// Is the head's position the printer's own, live? A demo printer plays its file and says where the file puts the head;
/// a printer whose driver cannot report X and Y live (PrusaLink) has only its progress. Both are the file followed,
/// not a measurement, and every view that shows a position says so (polish WO-03, 2026-10-18).
/// Polish WO-07 (2026-10-19, live pass): PrusaLink does report X and Y whenever the head is still, so an idle or paused
/// CORE One L showed its real parked position labelled "following the file" with no file at all. Only a print in
/// motion on such a printer is the file followed.
pub fn position_is_live(p: &Printer) -> bool {
    let live_xy = crate::driver::open(&p.cfg)
        .map(|d| d.capabilities().live_xy)
        .unwrap_or(true);
    let job_state = p
        .state
        .lock()
        .ok()
        .and_then(|g| g.last_tick.as_ref()?["job"]["state"].as_str().map(str::to_owned))
        .unwrap_or_default();
    xy_is_live(&p.cfg.driver, live_xy, &job_state)
}

fn xy_is_live(driver: &str, live_xy: bool, job_state: &str) -> bool {
    driver != "demo" && (live_xy || job_state != "printing")
}

/// What the model page needs for one printer: its model, how it is oriented, its calibration and the pose
/// for the position the printer reports right now.
///
/// At a moment of the recording (finish-2 F02, CORE One L, 2026-10-19): a rewound page drew the model at the
/// printer's live pose under "Live from the printer". With `recorded`, the tick the recorder held then gives the position,
/// the print and the tool.
pub fn model_json_at(
    p: &Printer,
    lib: &crate::printer_models::Library,
    now: f64,
    recorded: Option<&Value>,
) -> Value {
    use crate::printer_models as pm;
    // an axis Klipper had not homed then is unknown then, as in position_of
    let at = |i: usize| {
        recorded.and_then(|t| {
            let homed = t["homed_axes"].as_str().or(t["point"]["homed"].as_str()).unwrap_or("").to_ascii_lowercase();
            t["point"]["position"][i]
                .as_f64()
                .filter(|_| p.cfg.driver != "moonraker" || homed.contains(["x", "y", "z"][i]))
        })
    };
    let position_of = |p: &Printer, now: f64| match recorded {
        Some(_) => ([at(0), at(1), at(2)], Some(0.0)),
        None => position_of(p, now),
    };
    let follow_of = |p: &Printer| recorded.map_or_else(|| follow_of(p), follow_from);
    let tool_of = |p: &Printer| match recorded {
        Some(t) => t["point"]["tool"].as_u64().map(|t| t as usize),
        None => tool_of(p),
    };
    let position_is_live = |p: &Printer| match recorded {
        Some(t) => xy_is_live(
            &p.cfg.driver,
            crate::driver::open(&p.cfg).map(|d| d.capabilities().live_xy).unwrap_or(true),
            t["job"]["state"].as_str().unwrap_or(""),
        ),
        None => position_is_live(p),
    };
    let Some(id) = p.entry.cad_model.as_deref() else {
        // No catalogue model bound: if the printer's own scene CAD (the machine the main page
        // draws) is in its managed library, that is its farm-wall model (WO-14 item 1). The tile
        // still needs the last known pose, so report it here with the position the collector holds,
        // which also covers an offline printer drawn dimmed at its last pose (WO-14 item 2).
        if crate::assets::has_reference_model(&p.cfg.database) {
            let (position, age) = position_of(p, now);
            return json!({
                "model":null,
                "scene":true,
                "position":{"x":position[0],"y":position[1],"z":position[2]},
                "tick_age_s":age,
            });
        }
        return json!({"model":null,"reason":"This printer has no model. Choose one from the catalogue; Stella Scope never draws a stand-in."});
    };
    let Some(row) = pm::machine(id) else {
        return json!({"model":null,"reason":format!("{id} is no longer in the catalogue")});
    };
    let installed = lib.model_file(id).is_some();
    let profile = pm::profile(id);
    let (position, age) = position_of(p, now);
    let follow = follow_of(p);
    // Polish WO-03: a toolchanger that reports its tool gets its tools as motion groups of their own, each at its
    // dock until it is the active one; the scene then places every tool at its dock.
    let selection = p
        .entry
        .cad_options
        .as_ref()
        .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()));
    let swapping = follow["toolchanger"] == true
        && tool_of(p).is_some()
        && selection.is_some_and(|sel| pm::bank_tools(id, sel).is_some());
    let spec = pm::pose_spec(id, if swapping { selection } else { None });
    let cal = p
        .entry
        .cad_calibration
        .as_ref()
        .and_then(|v| pm::Calibration::from_value(v).ok())
        .or_else(|| profile.map(pm::Calibration::starting));
    json!({
        "model":id,
        "view":pm::view(row, lib),
        "installed":installed,
        "file_url":installed.then(|| format!("/api/printer-models/{id}/file")),
        "basis":pm::basis(id),
        "motion":profile.is_some(),
        // One plain sentence (polish WO-03): the page says it once, under the model. A machine with no registered movement keeps
        // its head where the model has it; the part on the plate still grows with the print when the plate is measured.
        "note":if profile.is_none() {
            Some(if pm::print_frame(id).is_some() {
                "This model is static: no movement is registered for this machine, so it is shown as built and its head stays where it is. The part on its plate still grows with the print."
            } else {
                "This model is static: no movement is registered for this machine, so it is shown as built and does not follow the printer."
            })
        } else { None },
        "calibration":cal.as_ref().map(pm::Calibration::to_value),
        "calibrated":p.entry.cad_calibration.is_some(),
        // Polish WO-05: where the nozzle is in the model when the printer reports X 0, Y 0, when the maker's profile or the model itself says so
        // (measured, not guessed); the page's one-press "use the bed corner". Null for a profile that only has the travel.
        "bed_corner":profile.and_then(|pr| pm::Calibration::from_value(&pr["calibration"]).ok().map(|c| json!({"calibration":c.to_value(),"basis":pr["calibration_basis"]}))),
        "position":{"x":position[0],"y":position[1],"z":position[2]},
        "tick_age_s":age,
        "pose":spec.as_ref().zip(cal.as_ref()).map(|(pr, c)| pm::pose_for_tool(pr, c, position, tool_of(p))),
        // The arithmetic of the pose (travel, motions, tools), so the page can move the head between reports at render rate.
        "pose_spec":spec,
        // Where a print file's coordinates land on this model, whether the position is live, and what the printer is printing.
        "print_frame":pm::print_frame(id),
        "position_source":if position_is_live(p) { "live" } else { "file" },
        "job":follow,
        // Polish WO-01: the toolchanger, toolhead and add-on options chosen for this model, as one scene.
        "options":p.entry.cad_options,
        "scene":selection.map(|sel| if swapping { pm::scene_swapping(id, sel, lib) } else { pm::scene(id, sel, lib) }),
    })
}

impl Printer {
    pub fn controls_enabled(&self) -> bool {
        self.cfg.controls_enabled && self.state.lock().map(|s| s.mode == "live").unwrap_or(false)
    }
}

/// An id from a name: lower-case letters and digits, dashes between, at most 32.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(32).collect();
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "printer".into()
    } else {
        out
    }
}

fn write_secret(dir: &Path, reference: &str, secret: &BTreeMap<String, String>) -> Result<PathBuf> {
    if reference.is_empty()
        || reference.len() > 64
        || !reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("credentials_ref may hold only letters, digits, dash and underscore".into());
    }
    let folder = dir.join("printers");
    fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    let path = folder.join(format!("{reference}.secret"));
    let temp = folder.join(format!("{reference}.secret.tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut f = options.open(&temp)?;
        f.write_all(
            serde_json::to_string(secret)
                .map_err(std::io::Error::other)?
                .as_bytes(),
        )?;
        f.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot store the credentials: {e}"));
    }
    Ok(path)
}

/// `YYYYMMDD-HHMMSS` in UTC, without a date crate (days-to-civil, Hinnant).
pub fn utc_stamp(secs: u64) -> String {
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot create farm control token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Used by `server`: split `/api/p/<id>/rest?q` into (`id`, `/api/rest?q`).
pub fn split_prefix(target: &str) -> Option<(&str, String)> {
    let rest = target.strip_prefix("/api/p/")?;
    let (id, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let tail = if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_owned()
    };
    Some((
        id,
        format!("/api{}", if tail.is_empty() { "/" } else { &tail }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polish WO-07: a PrusaLink printer's still head is its own reading; only a print in motion is the file followed.
    #[test]
    fn a_still_prusalink_head_is_live_and_only_a_running_print_follows_the_file() {
        for st in ["", "standby", "paused", "complete", "cancelled", "error"] {
            assert!(xy_is_live("prusalink", false, st), "prusalink {st}");
        }
        assert!(!xy_is_live("prusalink", false, "printing"));
        assert!(xy_is_live("moonraker", true, "printing"));
        for st in ["", "printing", "paused"] {
            assert!(!xy_is_live("demo", true, st), "demo {st}");
        }
    }

    #[test]
    fn prefix_splits_into_id_and_the_ordinary_route() {
        assert_eq!(
            split_prefix("/api/p/stella/tick"),
            Some(("stella", "/api/tick".to_owned()))
        );
        assert_eq!(
            split_prefix("/api/p/core-one/print/thumbnail?filename=a%20b.gcode"),
            Some((
                "core-one",
                "/api/print/thumbnail?filename=a%20b.gcode".to_owned()
            ))
        );
        assert_eq!(split_prefix("/api/p/x"), Some(("x", "/api/".to_owned())));
        assert_eq!(split_prefix("/api/tick"), None);
        assert_eq!(split_prefix("/api/printers"), None);
    }

    #[test]
    fn slugs_and_stamps() {
        assert_eq!(slug("CORE One L"), "core-one-l");
        assert_eq!(slug("  Voron 2.4 / 350  "), "voron-2-4-350");
        assert_eq!(slug("!!!"), "printer");
        assert!(crate::config::id_ok(&slug("Adventurer 5M (shop)")));
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_791_811_170), "20261012-131930");
    }
}
