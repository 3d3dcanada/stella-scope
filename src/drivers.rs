//! Printer brands other than Klipper. WO-04 adds PrusaLink and WO-05 FlashForge here; until then a
//! configuration that names one is told so plainly instead of being quietly treated as Moonraker.
use crate::{config::Config, driver::Driver, Result};

pub mod digest;
pub mod flashforge;
pub mod prusalink;

pub fn open_other(kind: &str, cfg: &Config) -> Result<Box<dyn Driver>> {
    #[cfg(test)]
    if kind == "testbrand" {
        return Ok(Box::new(testing::TestDriver {
            database: cfg.database.clone(),
        }));
    }
    if matches!(kind, "prusalink" | "flashforge") {
        return Ok(Box::new(shared(kind, cfg)));
    }
    Err(format!(
        "the {kind} driver is not available in this build; set \"driver\" to moonraker or remove the printer"
    ))
}

// One driver per printer, shared by whoever asks (the collector, a command, a page's camera request):
// what it has learned (the digest nonce, the serial, a camera address, the file last seen printing)
// lives with it instead of being forgotten after every request. Klipper's client stays per-call as it
// always was.
type Handle = std::sync::Arc<std::sync::Mutex<Box<dyn Driver>>>;
/// What a driver can do and what its brand is called, as it last said (polish WO-06: see `Shared::face`).
type Face = (crate::driver::Capabilities, &'static str);
#[derive(Clone)]
struct Entry {
    handle: Handle,
    face: std::sync::Arc<std::sync::Mutex<Face>>,
    /// One file transfer at a time beside the polls (see `Shared::transfer`).
    files: std::sync::Arc<std::sync::Mutex<()>>,
}
fn registry() -> &'static std::sync::Mutex<std::collections::HashMap<String, Entry>> {
    static R: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Entry>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Drop what is kept for a printer that has been removed from the farm.
pub fn forget(printer_id: &str) {
    if let Ok(mut r) = registry().lock() {
        r.retain(|k, _| !k.contains(&format!("|{printer_id}|")));
    }
}

fn shared(kind: &str, cfg: &Config) -> Shared {
    // The credentials are part of the key, so a changed password is a new driver; the key holds a hash.
    let secret: String = cfg
        .secret
        .0
        .iter()
        .map(|(k, v)| format!("{k}={v};"))
        .collect();
    let key = format!(
        "{kind}|{}|{}|{}|{}",
        cfg.printer_id,
        cfg.moonraker_url,
        cfg.camera_url.as_deref().unwrap_or(""),
        digest::md5_hex(&secret)
    );
    let entry = {
        let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
        r.entry(key)
            .or_insert_with(|| {
                let d: Box<dyn Driver> = if kind == "prusalink" {
                    Box::new(prusalink::PrusaLink::new(cfg))
                } else {
                    Box::new(flashforge::FlashForge::new(cfg))
                };
                let face = std::sync::Arc::new(std::sync::Mutex::new((d.capabilities(), d.brand())));
                Entry {
                    handle: std::sync::Arc::new(std::sync::Mutex::new(d)),
                    face,
                    files: std::sync::Arc::new(std::sync::Mutex::new(())),
                }
            })
            .clone()
    };
    Shared {
        handle: entry.handle,
        face: entry.face,
        cfg: cfg.clone(),
        files: entry.files,
        kind: if kind == "prusalink" {
            "prusalink"
        } else {
            "flashforge"
        },
    }
}

/// A handle on the shared driver: every call takes the lock for its own duration.
pub struct Shared {
    handle: Handle,
    face: std::sync::Arc<std::sync::Mutex<Face>>,
    /// For a PrusaLink file transfer on its own connection (see `transfer`).
    cfg: Config,
    files: std::sync::Arc<std::sync::Mutex<()>>,
    kind: &'static str,
}
impl Shared {
    /// What the driver can do and its brand, without waiting for it. The driver is locked for the whole of a poll, which on a printer that
    /// answers in seconds is several seconds, and the wall asks every printer this on every refresh: with a real PrusaLink in the farm its
    /// first load took over 8 s (live finding of polish WO-07, 2026-10-19). A driver that is busy answers with what it said last.
    fn face(&self) -> Face {
        match self.handle.try_lock() {
            Ok(g) => {
                let now = (g.capabilities(), g.brand());
                if let Ok(mut last) = self.face.lock() {
                    *last = now.clone();
                }
                now
            }
            Err(std::sync::TryLockError::Poisoned(p)) => {
                let g = p.into_inner();
                (g.capabilities(), g.brand())
            }
            Err(std::sync::TryLockError::WouldBlock) => self
                .face
                .lock()
                .map(|f| f.clone())
                .unwrap_or_else(|e| e.into_inner().clone()),
        }
    }
    fn with<T>(&self, f: impl FnOnce(&mut Box<dyn Driver>) -> T) -> T {
        let mut g = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }
    /// Files go on their own connection on a PrusaLink (finish-2 F02, CORE One L, 2026-10-19). Under the shared lock the 3D
    /// view's download of a 4.3 MB print file held the printer for about a minute: no status, the page said the printer was
    /// not answering, and Pause stayed greyed for 57 s while the head moved. Listing, pictures, downloads and uploads now run
    /// beside the polls; a fresh PrusaLink costs one more login handshake. FlashForge keeps the lock: its door takes one
    /// client at a time. Transfers still go one at a time (their own lock): with two page loads each downloading the file at
    /// once beside the polls, the printer's web server stopped answering status for 6 s at a time until they ended.
    fn transfer<T>(&self, f: impl FnOnce(&mut Box<dyn Driver>) -> T) -> T {
        if self.kind == "prusalink" {
            let _one = self.files.lock().unwrap_or_else(|e| e.into_inner());
            let mut own: Box<dyn Driver> = Box::new(prusalink::PrusaLink::new(&self.cfg));
            return f(&mut own);
        }
        self.with(f)
    }
}
impl Driver for Shared {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn brand(&self) -> &'static str {
        self.face().1
    }
    fn capabilities(&self) -> crate::driver::Capabilities {
        self.face().0
    }
    fn bed_hint(&mut self) -> Option<[f64; 2]> {
        self.with(|d| d.bed_hint())
    }
    fn model_hint(&mut self) -> Option<String> {
        self.with(|d| d.model_hint())
    }
    fn sample(&mut self, ts: f64) -> Result<(serde_json::Value, Vec<String>)> {
        self.with(|d| d.sample(ts))
    }
    fn files(&self) -> Result<Vec<serde_json::Value>> {
        self.transfer(|d| d.files())
    }
    fn file_metadata(&self, f: &str) -> Result<serde_json::Value> {
        self.transfer(|d| d.file_metadata(f))
    }
    fn file_size(&self, f: &str) -> Result<usize> {
        self.transfer(|d| d.file_size(f))
    }
    fn file_bytes(&self, f: &str, max: usize) -> Result<Vec<u8>> {
        self.transfer(|d| d.file_bytes(f, max))
    }
    fn file_thumbnail(&self, f: &str, max: usize) -> Result<Vec<u8>> {
        self.transfer(|d| d.file_thumbnail(f, max))
    }
    fn cameras(&self) -> Result<Vec<crate::driver::CameraSource>> {
        self.with(|d| d.cameras())
    }
    fn snapshot(&self) -> Result<crate::driver::StateSnapshot> {
        self.with(|d| d.snapshot())
    }
    fn act(&self, a: &crate::driver::Action) -> Result<serde_json::Value> {
        self.with(|d| d.act(a))
    }
    fn upload(&self, name: &str, bytes: &[u8], start: bool) -> Result<serde_json::Value> {
        self.transfer(|d| d.upload(name, bytes, start))
    }
    fn emergency_stop(&self) -> Result<serde_json::Value> {
        self.with(|d| d.emergency_stop())
    }
}

#[cfg(test)]
mod face_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Polish WO-06 (live finding of WO-07): asking a printer what it can do never waits for a poll that holds its driver.
    #[test]
    fn what_a_printer_can_do_is_answered_while_its_driver_is_busy() {
        let cfg = Config {
            moonraker_url: "http://127.0.0.1:9".into(),
            printer_id: "face-test-busy".into(),
            ..Config::default()
        };
        let d = shared("prusalink", &cfg);
        let held = d.handle.clone();
        let (started, hold) = std::sync::mpsc::channel::<()>();
        let busy = std::thread::spawn(move || {
            let _poll = held.lock().unwrap();
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(1500));
        });
        hold.recv().unwrap();
        let start = Instant::now();
        let again = shared("prusalink", &cfg);
        let (caps, brand) = (again.capabilities(), again.brand());
        assert!(start.elapsed() < Duration::from_millis(300), "waited {:?}", start.elapsed());
        assert!(caps.pause_resume && caps.thumbnails && !caps.klipper && brand == "PrusaLink");
        busy.join().unwrap();
        // once the poll is over the driver answers for itself again
        assert!(again.capabilities().start_print);
        forget("face-test-busy");
    }

    /// Finish-2 F02: a PrusaLink file call does not wait for a poll (or a long download) that holds the driver.
    #[test]
    fn a_prusalink_file_call_does_not_wait_for_the_shared_lock() {
        let cfg = Config {
            moonraker_url: "http://127.0.0.1:9".into(),
            printer_id: "transfer-test-busy".into(),
            ..Config::default()
        };
        let d = shared("prusalink", &cfg);
        let held = d.handle.clone();
        let (started, hold) = std::sync::mpsc::channel::<()>();
        let busy = std::thread::spawn(move || {
            let _download = held.lock().unwrap();
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(3000));
        });
        hold.recv().unwrap();
        let start = Instant::now();
        assert!(d.file_thumbnail("a.gcode", 1024).is_err(), "nothing answers on port 9");
        assert!(d.files().is_err());
        assert!(start.elapsed() < Duration::from_millis(2000), "waited {:?} for the lock", start.elapsed());
        busy.join().unwrap();
        forget("transfer-test-busy");
    }
}

/// A brand that exists only in tests: a printer that heats, pauses and starts files but cannot move
/// the head (the case WO-02 asks for), recording what it was asked so a test sees what reached it.
#[cfg(test)]
pub mod testing {
    use crate::driver::{Action, Capabilities, Driver, StateSnapshot};
    use crate::Result;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    /// (database path that opened the driver, what it was asked), oldest first.
    pub static LOG: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

    pub struct TestDriver {
        pub database: String,
    }

    pub fn asked(database: &str) -> Vec<String> {
        LOG.lock()
            .unwrap()
            .iter()
            .filter(|(d, _)| d == database)
            .map(|(_, a)| a.clone())
            .collect()
    }

    impl Driver for TestDriver {
        fn kind(&self) -> &'static str {
            "testbrand"
        }
        fn brand(&self) -> &'static str {
            "Testbrand"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                heat: true,
                pause_resume: true,
                cancel: true,
                start_print: true,
                emergency_stop: true,
                ..Capabilities::none()
            }
        }
        fn sample(&mut self, _ts: f64) -> Result<(Value, Vec<String>)> {
            Ok((json!({"print_stats":{"state":"standby"}}), Vec::new()))
        }
        fn snapshot(&self) -> Result<StateSnapshot> {
            Ok(StateSnapshot {
                state: "ready".into(),
                homed_axes: String::new(),
                print_state: "standby".into(),
                idle_state: "idle".into(),
                help: Default::default(),
            })
        }
        fn act(&self, action: &Action) -> Result<Value> {
            LOG.lock()
                .unwrap()
                .push((self.database.clone(), format!("{action:?}")));
            Ok(json!({"result":"ok"}))
        }
        fn emergency_stop(&self) -> Result<Value> {
            LOG.lock()
                .unwrap()
                .push((self.database.clone(), "emergency_stop".into()));
            Ok(json!({"result":"ok"}))
        }
        fn files(&self) -> Result<Vec<Value>> {
            Ok(vec![json!({"path":"plate.gcode","size":1,"modified":2.0})])
        }
    }
}
