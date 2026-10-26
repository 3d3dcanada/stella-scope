// Farm board WO-03 tests: many printers in one service, over the real listener and against fake
// Moonrakers (src/fakes.rs). Nothing here touches a real printer or our files.
use crate::{
    config::{self, Config, PrinterEntry},
    fakes::{FakeFlashForge, FakeMoonraker, FakePrusaLink},
    farm::Farm,
    server,
};
use serde_json::{json, Value};
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Dir(PathBuf);
impl Dir {
    fn new(label: &str) -> Self {
        let p = PathBuf::from(format!(
            "/tmp/stella-farm-test-{}-{}-{label}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Running {
    base: String,
    port: u16,
    farm: Arc<Farm>,
    token: String,
    agent: ureq::Agent,
}
impl Running {
    fn call(&self, method: &str, path: &str, body: Option<&Value>, token: bool) -> (u16, Value) {
        let url = format!("{}{path}", self.base);
        let origin = self.base.clone();
        let mut response = match (method, body) {
            ("GET", _) => self.agent.get(&url).call(),
            ("DELETE", _) => {
                let mut r = self.agent.delete(&url).header("Origin", &origin);
                if token {
                    r = r.header("X-Stella-Control-Token", &self.token);
                }
                r.call()
            }
            (m, Some(b)) => {
                let mut r = match m {
                    "POST" => self.agent.post(&url),
                    _ => self.agent.patch(&url),
                }
                .header("Origin", &origin)
                .header("Content-Type", "application/json");
                if token {
                    r = r.header("X-Stella-Control-Token", &self.token);
                }
                r.send(b.to_string().as_bytes())
            }
            _ => panic!("bad call"),
        }
        .unwrap();
        let code = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        (
            code,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
    fn get(&self, path: &str) -> Value {
        self.call("GET", path, None, false).1
    }
}

/// Start a farm from a configuration on disk, listening on its own port.
fn run(dir: &Dir, mut cfg: Config) -> Running {
    // STELLA_TEST_BIND names an address for the one ignored test that serves a service to a browser (polish WO-06).
    let listener = TcpListener::bind(
        std::env::var("STELLA_TEST_BIND").unwrap_or_else(|_| "127.0.0.1:0".into()),
    )
    .unwrap();
    let port = listener.local_addr().unwrap().port();
    cfg.port = port;
    cfg.controls_enabled = true;
    cfg.poll_interval_ms = 100;
    cfg.offline_retry_ms = 250;
    cfg.request_timeout_ms = 400;
    cfg.console_interval_ms = 200;
    cfg.bed_size_mm = [350.0, 350.0];
    // The configuration file the farm will edit: the same fields, as JSON.
    let path = dir.file("stella-scope.json");
    fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
    let farm = Farm::start(cfg, Some(PathBuf::from(&path)), false, false, Vec::new()).unwrap();
    let token = farm.token.clone().unwrap();
    {
        let farm = farm.clone();
        thread::spawn(move || {
            let _ = server::serve(listener, None, farm);
        });
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .proxy(None)
        .build()
        .into();
    Running {
        base: format!("http://127.0.0.1:{port}"),
        port,
        farm,
        token,
        agent,
    }
}

fn base_for(dir: &Dir, fake: &FakeMoonraker) -> Config {
    Config {
        moonraker_url: fake.url(),
        database: dir.file("stella.sqlite3"),
        printer_name: Some("Stella".into()),
        printer_model: Some("Voron 2.4 R2".into()),
        ..Config::default()
    }
}

// 30 s, not 15 (finish-2, 2026-10-19): with two builders compiling beside the suite (load average 11) three fake printers took
// longer than 15 s to answer their first poll; nothing waits this long on a quiet machine, and a real hang still fails.
fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// our live file, with nothing in it but what it holds today, loads as a farm of one: id stella,
/// the same address and the same recording path, and the file is not rewritten by loading it.
#[test]
fn an_old_single_printer_file_is_a_farm_of_one_with_the_same_recording() {
    let text = r#"{"moonraker_url":"http://192.168.1.131:7125","bind":"127.0.0.1","port":7420,"controls_enabled":true,
        "database":"/home/user/.local/share/stella-scope/stella.sqlite3","poll_interval_ms":250,
        "objects":["motion_report","toolhead","extruder"],"printer_name":"Stella","printer_model":"Voron 2.4 R2 · 350",
        "bed_size_confirmed":true,"probe_offset_confirmed":true,"probe_offset_mm":[-32.95,-10.718]}"#;
    let dir = Dir::new("migrate");
    let path = dir.file("old.json");
    fs::write(&path, text).unwrap();
    let cfg = Config::load(&path).unwrap();
    assert!(cfg.printers.is_empty());
    let entries = cfg.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "stella");
    assert_eq!(entries[0].name, "Stella");
    assert_eq!(entries[0].address, "http://192.168.1.131:7125");
    assert_eq!(entries[0].driver, "moonraker");
    let own = cfg.for_printer(&entries[0], &dir.0).unwrap();
    assert_eq!(
        own.database,
        "/home/user/.local/share/stella-scope/stella.sqlite3"
    );
    assert_eq!(own.moonraker_url, "http://192.168.1.131:7125");
    assert_eq!(own.printer_model.as_deref(), Some("Voron 2.4 R2 · 350"));
    assert_eq!(own.probe_offset_mm, [-32.95, -10.718]);
    assert_eq!(
        own.objects, cfg.objects,
        "the printer's own copy keeps the same object list"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "loading never writes the file"
    );
}

#[test]
fn credentials_are_refused_unless_only_their_owner_can_read_them() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("secret");
    fs::create_dir_all(dir.0.join("printers")).unwrap();
    let path = dir.0.join("printers/core-one.secret");
    fs::write(&path, r#"{"user":"maker","password":"hunter2"}"#).unwrap();
    for mode in [0o644, 0o640, 0o604, 0o664] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        let e = config::load_secret(&dir.0, "core-one").unwrap_err();
        assert!(
            e.contains("chmod 600") && e.contains(&format!("{mode:o}")),
            "{e}"
        );
        assert!(!e.contains("hunter2"));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let s = config::load_secret(&dir.0, "core-one").unwrap();
    assert_eq!(s.get("user"), Some("maker"));
    assert!(
        !format!("{s:?}").contains("hunter2"),
        "Debug never prints a secret"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(config::load_secret(&dir.0, "core-one").is_ok());
    for bad in ["", "../etc", "a/b", "with space", &"x".repeat(65)] {
        assert!(config::load_secret(&dir.0, bad).is_err(), "{bad:?}");
    }
    assert!(config::load_secret(&dir.0, "missing")
        .unwrap_err()
        .contains("mode 600"));
}

#[test]
fn three_printers_each_have_their_own_recording_name_and_routes() {
    let dir = Dir::new("three");
    let (a, b, c) = (
        FakeMoonraker::start(0),
        FakeMoonraker::start(0),
        FakeMoonraker::start(0),
    );
    b.state.lock().unwrap().nozzle = 111.0;
    c.state.lock().unwrap().nozzle = 222.0;
    let cfg = Config {
        printers: vec![
            PrinterEntry {
                id: "stella".into(),
                name: "Stella".into(),
                address: a.url(),
                database: Some(dir.file("stella.sqlite3")),
                ..Default::default()
            },
            PrinterEntry {
                id: "bench".into(),
                name: "Bench".into(),
                address: b.url(),
                accent: Some("#FF6A00".into()),
                ..Default::default()
            },
            PrinterEntry {
                id: "shop".into(),
                name: "Shop".into(),
                address: c.url(),
                ..Default::default()
            },
        ],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    wait_for("three printers online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .is_some_and(|p| p.len() == 3 && p.iter().all(|x| x["online"] == true))
    });
    let list = r.get("/api/printers");
    assert_eq!(list["default"], "stella");
    let by = |id: &str| {
        list["printers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by("bench")["name"], "Bench");
    assert_eq!(by("bench")["accent"], "#FF6A00");
    assert_eq!(by("shop")["state"], "idle");
    // Own routes: each answers for itself, and un-prefixed means the first.
    for (id, name) in [("stella", "Stella"), ("bench", "Bench"), ("shop", "Shop")] {
        assert_eq!(r.get(&format!("/api/p/{id}/meta"))["printer"]["name"], name);
    }
    assert_eq!(r.get("/api/meta")["printer"]["name"], "Stella");
    wait_for("each printer's own nozzle", || {
        let t = |id: &str| {
            r.get(&format!("/api/p/{id}/tick"))["point"]["nozzle"]["temperature"].as_f64()
        };
        t("bench").is_some_and(|v| v > 100.0 && v < 140.0)
            && t("shop").is_some_and(|v| v > 200.0 && v < 240.0)
    });
    assert_eq!(r.call("GET", "/api/p/nobody/tick", None, false).0, 404);
    // Three recordings, one per printer, in the places the configuration says.
    let dbs: Vec<String> = r
        .farm
        .list()
        .iter()
        .map(|p| p.cfg.database.clone())
        .collect();
    assert_eq!(dbs[0], dir.file("stella.sqlite3"));
    // WO-13: per-printer recordings sit beside the configured database, named after it.
    assert_eq!(dbs[1], dir.file("stella.sqlite3.printers/bench.sqlite3"));
    assert_eq!(dbs[2], dir.file("stella.sqlite3.printers/shop.sqlite3"));
    for d in &dbs {
        assert!(fs::metadata(d).is_ok(), "{d}");
    }
    // Each printer's commands go to that printer only.
    let id = "bench";
    let ctl = r.get(&format!("/api/p/{id}/controls"));
    let token = ctl["token"].as_str().unwrap().to_owned();
    let (code, _) = {
        let url = format!("{}/api/p/{id}/commands", r.base);
        let mut resp = r
            .agent
            .post(&url)
            .header("Origin", &r.base)
            .header("Content-Type", "application/json")
            .header("X-Stella-Control-Token", &token)
            .send(
                json!({"id":"set_heater","target":"extruder","value":205})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        (
            resp.status().as_u16(),
            resp.body_mut().read_to_string().unwrap_or_default(),
        )
    };
    assert_eq!(code, 200);
    assert_eq!(
        b.scripts(),
        vec!["SET_HEATER_TEMPERATURE HEATER=extruder TARGET=205"]
    );
    assert!(
        a.scripts().is_empty() && c.scripts().is_empty(),
        "another printer's commands never reach this one"
    );
}

/// Add, rename and remove over the API: the configuration file is edited atomically with a dated copy,
/// credentials land in a mode-600 file and never in the JSON, and a removed printer's recording is kept.
/// WO-13: a per-printer recording sits beside the configured database (`<db>.printers/<id>`), so a
/// scratch run never touches the live folder; a recording left at the pre-WO-13 `printers/<id>`
/// stays in use. And a new printer never opens a file an earlier printer with the same id left
/// behind (the "LAST SEEN 1.7 H AGO" bug): the stale file is renamed with the date.
#[test]
fn a_new_printer_never_reuses_a_stale_recording_and_scratch_stays_scratch() {
    let dir = Dir::new("stale");
    let cfg = base_for(&dir, &FakeMoonraker::start(0));
    let e = |id: &str| PrinterEntry {
        id: id.into(),
        name: id.into(),
        address: "http://127.0.0.1:9".into(),
        ..Default::default()
    };
    // Fresh layout: named after the configured database, not the shared data dir.
    assert_eq!(
        cfg.recording_for(&e("bench")),
        dir.file("stella.sqlite3.printers/bench.sqlite3")
    );
    // Another database in the same folder answers with its own folder, so a scratch run never sees
    // the live recordings.
    let mut scratch = cfg.clone();
    scratch.database = dir.file("scratch.sqlite3");
    assert_eq!(
        scratch.recording_for(&e("bench")),
        dir.file("scratch.sqlite3.printers/bench.sqlite3")
    );

    // Through the API: seed a stale recording at the legacy place, add a printer of the same id, and
    // the stale file is renamed away before the new one starts.
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let r = run(&dir, base_for(&dir, &a));
    wait_for("stella online", || {
        r.get("/api/printers")["printers"][0]["online"] == true
    });
    let stale_dir = dir.file("stella.sqlite3.printers");
    fs::create_dir_all(&stale_dir).unwrap();
    let stale = format!("{stale_dir}/bench-voron.sqlite3");
    fs::write(&stale, b"stale").unwrap();
    let add = json!({"name":"Bench Voron","address":b.url()});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    // The new collector opens the same path again: the proof of the rename is the dated copy.
    assert!(
        Path::new(&stale).exists(),
        "the fresh recording takes the path"
    );
    let renamed: Vec<String> = fs::read_dir(&stale_dir)
        .unwrap()
        .filter_map(|x| x.ok())
        .map(|x| x.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        renamed
            .iter()
            .any(|n| n.starts_with("bench-voron.sqlite3.replaced-")),
        "{renamed:?}"
    );
    let fresh = dir.file("stella.sqlite3.printers/bench-voron.sqlite3");
    assert_eq!(r.farm.get("bench-voron").unwrap().cfg.database, fresh);
    wait_for("the new recording exists", || fs::metadata(&fresh).is_ok());
    assert_eq!(
        cfg.recording_for(&e("shop")),
        dir.file("stella.sqlite3.printers/shop.sqlite3")
    );
    let _ = (r.port, &b, &a);
}

#[test]
fn add_rename_and_remove_a_printer_through_the_api() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("crud");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let r = run(&dir, base_for(&dir, &a));
    let cfg_path = dir.file("stella-scope.json");
    let before: Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    assert!(before["printers"].as_array().unwrap().is_empty());
    wait_for("stella online", || {
        r.get("/api/printers")["printers"][0]["online"] == true
    });

    // Without the token, nothing changes.
    let add = json!({"name":"Bench Voron","address":b.url(),"credentials":{"api_key":"k-123"},"accent":"#04D9C4"});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), false).0, 401);
    assert_eq!(r.get("/api/printers")["count"], 1);
    // A bad address saves nothing and leaves no secret behind.
    let bad = json!({"name":"Bad","address":"ftp://nope","credentials":{"api_key":"x"}});
    let (code, body) = r.call("POST", "/api/printers", Some(&bad), true);
    assert_eq!(code, 400, "{body}");
    assert!(!dir.0.join("printers/bad.secret").exists());
    assert_eq!(
        serde_json::from_str::<Value>(&fs::read_to_string(&cfg_path).unwrap()).unwrap(),
        before
    );

    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["printer"]["id"], "bench-voron");
    assert_eq!(body["count"], 2);
    // The file now lists both printers; the first keeps its own recording path, nothing else changed.
    let text = fs::read_to_string(&cfg_path).unwrap();
    let after: Value = serde_json::from_str(&text).unwrap();
    let printers = after["printers"].as_array().unwrap();
    assert_eq!(printers.len(), 2);
    assert_eq!(printers[0]["id"], "stella");
    assert_eq!(printers[0]["database"], dir.file("stella.sqlite3"));
    assert_eq!(printers[1]["id"], "bench-voron");
    assert_eq!(printers[1]["credentials_ref"], "bench-voron");
    for key in [
        "port",
        "poll_interval_ms",
        "moonraker_url",
        "database",
        "objects",
        "printer_name",
    ] {
        assert_eq!(after[key], before[key], "{key} is untouched");
    }
    assert!(!text.contains("k-123"), "credentials are never in the JSON");
    let secret = dir.0.join("printers/bench-voron.secret");
    assert_eq!(
        fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(fs::read_to_string(&secret).unwrap().contains("k-123"));
    let backups = |d: &Dir| -> Vec<String> {
        fs::read_dir(&d.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("stella-scope.json.bak-"))
            .collect()
    };
    assert!(
        !backups(&dir).is_empty(),
        "a dated copy of the previous file is kept"
    );
    wait_for("the new printer online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "bench-voron" && p["online"] == true)
    });

    // Rename and recolour; the address cannot be changed this way.
    let (code, body) = r.call(
        "PATCH",
        "/api/printers/bench-voron",
        Some(&json!({"name":"Bench","accent":"#FF2D92"})),
        true,
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["printer"]["name"], "Bench");
    assert_eq!(r.get("/api/p/bench-voron/meta")["printer"]["name"], "Bench");
    let saved: Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    assert_eq!(saved["printers"][1]["name"], "Bench");
    assert_eq!(saved["printers"][1]["accent"], "#FF2D92");
    assert_eq!(
        r.call(
            "PATCH",
            "/api/printers/bench-voron",
            Some(&json!({"address":"http://10.0.0.9"})),
            true
        )
        .0,
        400
    );
    assert_eq!(
        r.call(
            "PATCH",
            "/api/printers/nobody",
            Some(&json!({"name":"x"})),
            true
        )
        .0,
        400
    );

    // Finish-2 F06 (2026-10-19): this line used to assert that the first printer cannot be removed (400). It can: a wrong address typed for the first
    // printer could be neither changed nor removed. `the_first_printer_can_be_removed_and_a_farm_left_with_none_is_a_fresh_install_again` and
    // `a_single_printer_file_that_loses_its_printer_does_not_get_it_back` hold what replaced it.

    // Remove: the recording is kept under a dated name, the secret goes, the file lists one printer.
    let recording = r.farm.get("bench-voron").unwrap().cfg.database.clone();
    assert!(fs::metadata(&recording).is_ok());
    let (code, body) = r.call("DELETE", "/api/printers/bench-voron", None, true);
    assert_eq!(code, 200, "{body}");
    let kept = body["recording_kept_as"].as_str().unwrap().to_owned();
    assert!(
        kept.contains(".removed-") && fs::metadata(&kept).is_ok(),
        "{kept}"
    );
    assert!(fs::metadata(&recording).is_err());
    assert!(
        !secret.exists(),
        "credentials nobody references are deleted"
    );
    let last: Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    assert_eq!(last["printers"].as_array().unwrap().len(), 1);
    assert_eq!(r.get("/api/printers")["count"], 1);
    assert_eq!(r.call("GET", "/api/p/bench-voron/meta", None, false).0, 404);
    assert!(
        backups(&dir).len() >= 3,
        "every write kept its previous file"
    );
    let _ = (r.port, &b);
}

/// WO-06: a connection is tested before anything is saved; a wrong password or check code is said in
/// words and leaves the configuration and the credentials directory exactly as they were; a right one
/// adds the printer, which then shows up live. The search is desktop-only and refuses a public network.
#[test]
fn a_printer_is_tested_before_it_is_saved_and_a_wrong_login_saves_nothing() {
    let dir = Dir::new("add-flow");
    let (a, p, f) = (
        FakeMoonraker::start(0),
        FakePrusaLink::start(0, "maker", "secret"),
        FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000"),
    );
    let r = run(&dir, base_for(&dir, &a));
    let cfg_path = dir.file("stella-scope.json");
    let before = fs::read_to_string(&cfg_path).unwrap();
    let listing = |d: &Dir| -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(d.0.join("printers"))
            .map(|e| {
                e.filter_map(|x| x.ok())
                    .map(|x| x.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let files_before = listing(&dir);

    // Search: needs the token; a public network is refused; the answer says how much was looked at.
    let (code, _) = r.call(
        "POST",
        "/api/printers/discover",
        Some(&json!({"subnet":"127.0.0"})),
        false,
    );
    assert_eq!(code, 401);
    let (code, body) = r.call(
        "POST",
        "/api/printers/discover",
        Some(&json!({"subnet":"8.8.8"})),
        true,
    );
    assert_eq!(code, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("private network"));
    let (code, body) = r.call(
        "POST",
        "/api/printers/discover",
        Some(&json!({"subnet":"127.0.0"})),
        true,
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["scanned"], 254);
    assert!(body["printers"].is_array());

    // PrusaLink: wrong password first. Nothing is written and the password is never echoed.
    let bad = json!({"driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"nope-nope"}});
    let (code, body) = r.call("POST", "/api/printers/test", Some(&bad), true);
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().contains("login"), "{body}");
    assert!(!body.to_string().contains("nope-nope"));
    assert_eq!(
        fs::read_to_string(&cfg_path).unwrap(),
        before,
        "the configuration file is untouched"
    );
    assert_eq!(
        listing(&dir),
        files_before,
        "no credentials file was written"
    );
    assert_eq!(r.get("/api/printers")["count"], 1);

    // The right password: the test lists what the printer can do, then adding it saves and starts it.
    let good = json!({"driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"secret"}});
    let (_, body) = r.call("POST", "/api/printers/test", Some(&good), true);
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["brand"], "PrusaLink");
    assert_eq!(
        r.get("/api/printers")["count"],
        1,
        "a test saves nothing either"
    );
    let add = json!({"name":"Core One L","driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"secret"},"camera":"rtsp://192.168.1.132:554/live"});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["printer"]["id"], "core-one-l");
    wait_for("the Prusa online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "core-one-l" && x["online"] == true)
    });

    // FlashForge: the check code in the wrong case is refused in words; the right one connects.
    let ff = |code: &str| json!({"driver":"flashforge","address":f.http_url(),"credentials":{"serial":"SNFAKE5M00001","check_code":code,"tcp_port":f.tcp_port().to_string()}});
    let (_, body) = r.call("POST", "/api/printers/test", Some(&ff("0FAKE000")), true);
    assert_eq!(body["ok"], false, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("check code"),
        "{body}"
    );
    assert!(!body.to_string().to_lowercase().contains("0fake000"));
    assert_eq!(r.get("/api/printers")["count"], 2);
    let (_, body) = r.call("POST", "/api/printers/test", Some(&ff("0fake000")), true);
    assert_eq!(body["ok"], true, "{body}");
    assert!(
        !f.state.lock().unwrap().controlled,
        "the test released the printer"
    );
    let add = json!({"name":"Adventurer 5M","driver":"flashforge","address":f.http_url(),"credentials":{"serial":"SNFAKE5M00001","check_code":"0fake000","tcp_port":f.tcp_port().to_string()}});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    wait_for("the AD5M online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "adventurer-5m" && x["online"] == true)
    });
    assert_eq!(r.get("/api/printers")["count"], 3);

    // The camera can be set later; a bad address is refused and changes nothing.
    let (code, _) = r.call(
        "PATCH",
        "/api/printers/adventurer-5m",
        Some(&json!({"camera":"rtsp://8.8.8.8/x"})),
        true,
    );
    assert_eq!(code, 400);
    let (code, body) = r.call(
        "PATCH",
        "/api/printers/adventurer-5m",
        Some(&json!({"camera":"http://192.168.1.126:8080/?action=stream"})),
        true,
    );
    assert_eq!(code, 200, "{body}");
    let saved: Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    assert_eq!(
        saved["printers"][2]["camera"],
        "http://192.168.1.126:8080/?action=stream"
    );
    assert!(!fs::read_to_string(&cfg_path).unwrap().contains("0fake000"));
}

/// U04 (2026-10-20): the farm wall has one tile size and a press grows a tile in place; the tile's nozzle, bed and progress are the control kit's LED gauges, its
/// actions and the header's are the kit's square keys. What the browser is served says so: no S/M/L switch or size rule is left, the kit is imported and its
/// stylesheet linked, the grown tile takes the room of four (the whole width on a phone), a heat gauge sends set_heater once through the farm's command path
/// (preview first), and the wall asks for its printers with a time limit so a printer going off or coming back is never held up behind a request nobody answers.
#[test]
fn the_farm_wall_has_one_tile_size_that_grows_with_gauges_and_keys() {
    let dir = Dir::new("u04-wall");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let get = |p: &str| r.call("GET", p, None, false).1.as_str().unwrap_or("").to_owned();
    let (html, js, css, wall) = (get("/farm"), get("/farm.js"), get("/farm.css"), get("/farm-wall.js"));
    assert!(html.contains("/next-control.css") && !html.contains("fm-size"), "the page links the kit's stylesheet and has no size switch");
    for needle in ["from './next-control.js'", "control(spec)", "sq('nav-hud','Open'", "sq('stop-square','Cancel print'", "sendHeat(", "'set_heater'", "command-preview", "SET_MAX", "toggleOpen", "AbortSignal.timeout(8000)", "fm-led"] {
        assert!(js.contains(needle), "farm.js lost {needle}");
    }
    for gone in ["SIZES", "defaultSize", "validSize", "fm-size", "openMenu", "bindLongPress", "dial({"] {
        assert!(!js.contains(gone), "farm.js still has {gone}");
    }
    assert!(!css.contains("data-size="), "an S, M or L rule is left in farm.css");
    for needle in [".fm-tile[data-open]{grid-column:span 2;grid-row:span 2", ".fm-tile[data-open]{grid-column:1/-1", ".fm-heat", ".fm-g[data-g=progress]", "body.fm:has(.nxctl.live) .fm-root{z-index:auto}", "repeat(2,minmax(0,1fr))"] {
        assert!(css.contains(needle), "farm.css lost {needle}");
    }
    assert!(wall.contains("tvLayout") && wall.contains("heatOf") && !wall.contains("SIZES"), "farm-wall.js has the wall screen's layout and not the sizes");
}

/// WO-07: the wall is a page of the service itself (no inline script or style, so the CSP holds), and the
/// summary it draws from carries every field a tile shows.
#[test]
fn the_farm_page_is_served_and_the_summary_carries_what_a_tile_draws() {
    let dir = Dir::new("wall");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Bench","address":b.url(),"accent":"#FF6A00"});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    for (path, needle) in [
        ("/farm", "fm-grid"),
        ("/farm/", "fm-grid"),
        ("/farm.js", "/api/printers"),
        ("/farm-wall.js", "plan3d"),
        ("/gcode-worker.js", "parseGcode"),
        ("/farm.css", ".fm-tile"),
        ("/next-switch.js", "initSwitcher"),
        ("/next-farm.js", "sheetPrinters"),
    ] {
        let (code, body) = r.call("GET", path, None, false);
        assert_eq!(code, 200, "{path}");
        assert!(
            body.as_str().unwrap_or("").contains(needle),
            "{path} should contain {needle}"
        );
    }
    let (_, html) = r.call("GET", "/farm", None, false);
    let html = html.as_str().unwrap();
    assert!(
        !html.contains("<style") && !html.contains("style=\"") && !html.contains("onclick"),
        "inline style or script would break the CSP"
    );
    // Polish WO-04: the wall's tools (search, show, sort) and its pure logic module are part of the page. U04 (2026-10-20): there is one tile size, so the
    // S/M/L switch (id="fm-size") is gone and must stay gone; the page loads the control kit's stylesheet for the gauges and the square keys.
    for marker in ["id=\"fm-q\"", "id=\"fm-filter\"", "id=\"fm-sort\"", "fm-sticky", "/next-control.css"] {
        assert!(html.contains(marker), "the farm page lost {marker}");
    }
    assert!(!html.contains("fm-size"), "the tile size switch came back");
    let (_, js) = r.call("GET", "/farm.js", None, false);
    assert!(
        js.as_str().unwrap_or("").contains("from './farm-wall.js'"),
        "farm.js no longer imports its logic module"
    );
    wait_for("both online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["online"] == true)
    });
    let list = r.get("/api/printers");
    let bench = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "bench")
        .unwrap();
    for key in [
        "id",
        "name",
        "brand",
        "address",
        "accent",
        "state",
        "temps",
        "capabilities",
        "camera_url",
        "controls",
        "tick_age_s",
        "default",
    ] {
        assert!(bench.get(key).is_some(), "the summary has {key}");
    }
    assert_eq!(bench["capabilities"]["pause_resume"], true);
    // The wall's quick actions use the printer's own routes, so each printer answers for itself.
    assert_eq!(r.call("GET", "/api/p/bench/controls", None, false).0, 200);
    assert_eq!(r.call("GET", "/api/p/nobody/controls", None, false).0, 404);
    let _ = (r.port, &b);
}

// ---- WO-08: the queue, the plate check and the rule that nothing starts without one ----------------------

/// A camera that serves whatever JPEG the test last set, as an MJPEG stream.
struct FakeCam {
    addr: String,
    frame: Arc<std::sync::Mutex<Vec<u8>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for FakeCam {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl FakeCam {
    fn start(first: Vec<u8>) -> Self {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();
        let frame = Arc::new(std::sync::Mutex::new(first));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (f, s) = (frame.clone(), stop.clone());
        thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut sock, _)) => {
                        let (f, s) = (f.clone(), s.clone());
                        thread::spawn(move || {
                            let _ = sock.set_nonblocking(false);
                            let _ = write!(sock, "HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\n\r\n");
                            while !s.load(Ordering::Relaxed) {
                                let jpeg = f.lock().unwrap().clone();
                                if write!(sock, "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len()).is_err()
                                    || sock.write_all(&jpeg).is_err()
                                    || sock.write_all(b"\r\n").is_err()
                                {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(60));
                            }
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(15)),
                }
            }
        });
        Self { addr, frame, stop }
    }
    fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }
    fn show(&self, jpeg: Vec<u8>) {
        *self.frame.lock().unwrap() = jpeg;
    }
}

/// A 640x360 plate-like frame: fine texture, a soft gradient, optionally a bright block (a part).
fn plate_jpeg(seed: u32, gain: f64, block: Option<(u32, u32, u32, u32)>) -> Vec<u8> {
    let (w, h) = (640_u32, 360_u32);
    let mut s = seed;
    let img = image::GrayImage::from_fn(w, h, |x, y| {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        let tex = f64::from((s >> 24) as u8) / 255.0 * 36.0 - 18.0;
        let mut v = (96.0 + tex + f64::from(x) / f64::from(w) * 24.0) * gain;
        if let Some((bx, by, bw, bh)) = block {
            if x >= bx && x < bx + bw && y >= by && y < by + bh {
                v = 220.0;
            }
        }
        image::Luma([v.clamp(0.0, 255.0) as u8])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&image::DynamicImage::ImageLuma8(img))
        .unwrap();
    out
}

fn sliced(model: &str, nozzle: f64, name: &str) -> Vec<u8> {
    let mut g = format!("; generated by PrusaSlicer 2.9.6 on 2026-10-12 at 10:00:00 UTC\nG90\nM83\nG1 X20 Y20 Z0.2 F7800\nG1 X80 Y20 E2\nG1 X80 Y60 E2\nG1 X20 Y60 E2\n; sliced as {name}\n");
    for _ in 0..200 {
        g.push_str("G1 X21 Y21 E0.01\n");
    }
    g.push_str(&format!("; bed_shape = 0x0,250x0,250x220,0x220\n; filament_type = PETG\n; nozzle_diameter = {nozzle}\n; printer_model = {model}\n"));
    g.into_bytes()
}

impl Running {
    fn upload(&self, name: &str, bytes: &[u8], token: bool) -> (u16, Value) {
        let url = format!(
            "{}/api/queue/upload?name={}&copies=2",
            self.base,
            name.replace(' ', "%20")
        );
        let mut r = self
            .agent
            .post(&url)
            .header("Origin", &self.base)
            .header("Content-Type", "application/octet-stream");
        if token {
            r = r.header("X-Stella-Control-Token", &self.token);
        }
        // A body refused unread may be answered and closed before the client has finished sending it:
        // the refusal then reaches the client as a closed connection, which is the same refusal.
        let mut resp = match r.send(bytes) {
            Ok(resp) => resp,
            Err(_) if !token => return (400, json!({"error":"refused before the file was read"})),
            Err(e) => panic!("upload failed: {e}"),
        };
        let code = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (
            code,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
    /// A write to one printer's own plate or queue route, with that printer's control token.
    fn printer_call(&self, method: &str, path: &str, id: &str, body: &Value) -> (u16, Value) {
        let t = self.get(&format!("/api/p/{id}/controls"))["token"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let url = format!("{}{path}", self.base);
        let b = body.to_string();
        let r = match method {
            "PATCH" => self.agent.patch(&url),
            _ => self.agent.post(&url),
        }
        .header("Origin", &self.base)
        .header("Content-Type", "application/json")
        .header("X-Stella-Control-Token", &t);
        let mut resp = r.send(b.as_bytes()).unwrap();
        let code = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (
            code,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
}

/// A file goes to a printer that fits it, in order, uploaded whole and started through the command
/// route; a file for another machine is refused in words; the job is closed out when the print ends.
#[test]
fn a_queued_file_is_matched_uploaded_whole_and_started_on_the_printer_that_fits() {
    let dir = Dir::new("queue-flow");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core","address":b.url(),"slicer_model":"COREONEL","nozzle_mm":0.4,"bed_size_mm":[250.0,220.0]});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    wait_for("both online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["online"] == true)
    });

    // An upload without the token is refused unread; with it the file is parsed and queued.
    let bytes = sliced("COREONEL", 0.4, "tray");
    assert_eq!(
        r.upload("tray.gcode", &bytes, false).0,
        400,
        "refused: it did not show the farm token"
    );
    let (code, body) = r.upload("tray one.gcode", &bytes, true);
    assert_eq!(code, 200, "{body}");
    let job = body["job"]["id"].as_i64().unwrap();
    assert_eq!(body["job"]["needs"]["printer_model"], "COREONEL");
    assert_eq!(body["job"]["needs"]["nozzle_mm"], 0.4);
    assert_eq!(body["job"]["copies"], 2);
    // It fits the Core printer and says why it does not fit Stella (whose slicer name is not set).
    let fits = body["job"]["fits"].as_array().unwrap();
    let core = fits.iter().find(|f| f["printer"] == "core").unwrap();
    assert_eq!(
        (core["ok"].clone(), core["verified"].clone()),
        (json!(true), json!(true)),
        "{core}"
    );
    // A file for another machine, and a bad extension.
    let other = sliced("Anycubic Kobra X", 0.25, "other");
    let (_, ob) = r.upload("other.gcode", &other, true);
    let other_id = ob["job"]["id"].as_i64().unwrap();
    let wrong = ob["job"]["fits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["printer"] == "core")
        .unwrap()
        .clone();
    assert_eq!(wrong["ok"], false);
    assert!(
        wrong["reasons"].to_string().contains("different machine"),
        "{wrong}"
    );
    assert_eq!(r.upload("notes.txt", b"hello", true).0, 400);

    // Starting that file on the Core printer: refused in words, nothing uploaded or started.
    let (code, body) = r.printer_call(
        "POST",
        "/api/p/core/queue/start",
        "core",
        &json!({"job":other_id}),
    );
    assert_eq!(code, 400, "{body}");
    assert!(b.state.lock().unwrap().uploads.is_empty());

    // The right file: no empty-plate picture is saved, so the tap-less start does its own check, which
    // is skipped, and the file goes: whole, then started by the command route.
    let (code, body) = r.printer_call(
        "POST",
        "/api/p/core/queue/start",
        "core",
        &json!({"job":job}),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["started"], true, "{body}");
    let up = b.state.lock().unwrap().uploads.clone();
    assert_eq!(up.len(), 1, "{up:?}");
    assert_eq!(up[0].0, "tray_one.gcode");
    assert_eq!(up[0].1, bytes.len(), "the bytes arrive whole");
    assert!(
        b.posts("/printer/print/start")
            .iter()
            .any(|e| e["path"].as_str().unwrap_or("").contains("tray_one.gcode")),
        "{:?}",
        b.log()
    );
    let q = r.farm.queue().unwrap();
    assert_eq!(q.job(job).unwrap().unwrap().state, "running");

    // The print runs long enough to be seen (the watcher looks every two seconds), then finishes: the
    // copy counts and the plate needs clearing.
    wait_for("the print to show as printing", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "core" && p["state"] == "printing")
    });
    thread::sleep(Duration::from_millis(2600));
    {
        let mut s = b.state.lock().unwrap();
        s.print_state = "complete".into();
        s.progress = 1.0;
    }
    wait_for("the copy to count", || {
        q.job(job).unwrap().unwrap().done == 1
    });
    let j = q.job(job).unwrap().unwrap();
    assert_eq!(
        (j.state.as_str(), j.done, j.copies),
        ("queued", 1, 2),
        "one copy of two is done, the job waits for the next"
    );
    wait_for("the plate to need clearing", || {
        q.flow("core").awaiting_clear
    });
    // The plate needs clearing and a copy is waiting for it: that is an alert (WO-11), named for the printer.
    let said = |kind: &str, printer: &str| {
        r.get("/api/notify")["recent"].as_array().is_some_and(|a| {
            a.iter()
                .any(|e| e["kind"] == kind && e["printer"] == printer)
        })
    };
    wait_for("the plate-clear alert", || said("plate_clear", "core"));
}

/// The rule: no start without a passing plate check or an override the person confirmed.
#[test]
fn nothing_starts_without_a_passing_plate_check_or_a_confirmed_override() {
    let dir = Dir::new("plate-flow");
    let a = FakeMoonraker::start(0);
    let p = FakePrusaLink::start(0, "maker", "secret");
    let cam = FakeCam::start(plate_jpeg(7, 1.0, None));
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Prusa","driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"secret"},
        "camera":cam.url(),"slicer_model":"COREONEL","nozzle_mm":0.4});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    wait_for("prusa online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "prusa" && x["online"] == true)
    });
    wait_for("the camera answers", || {
        r.get("/api/p/prusa/camera")["cameras"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
    });
    let (_, ub) = r.upload("lid.gcode", &sliced("COREONEL", 0.4, "lid"), true);
    let job = ub["job"]["id"].as_i64().unwrap();
    let started = || {
        p.log()
            .iter()
            .filter(|e| {
                e["method"] == "PUT" && e["path"].as_str().unwrap_or("").contains("/api/v1/files/")
            })
            .count()
            + p.log()
                .iter()
                .filter(|e| {
                    e["method"] == "POST"
                        && e["path"].as_str().unwrap_or("").contains("/api/v1/job")
                })
                .count()
    };
    let before = started();

    // 1. The empty plate is captured; the region is the whole frame until it is marked.
    let (code, body) = r.printer_call("POST", "/api/p/prusa/plate/reference", "prusa", &json!({}));
    assert_eq!(code, 200, "{body}");
    assert_eq!(r.get("/api/p/prusa/plate")["has_reference"], true);
    let (code, _) = r.printer_call(
        "PATCH",
        "/api/p/prusa/plate",
        "prusa",
        &json!({"region":[[0.0,0.0],[1.0,0.0],[1.0,0.0],[0.0,0.0]]}),
    );
    assert_eq!(code, 400, "a flat region is refused");
    let (code, _) = r.printer_call(
        "PATCH",
        "/api/p/prusa/plate",
        "prusa",
        &json!({"region":[[0.05,0.05],[0.95,0.05],[0.95,0.95],[0.05,0.95]],"sensitivity":1.0}),
    );
    assert_eq!(code, 200);

    // 2. The same plate in other light passes; the part left behind does not.
    cam.show(plate_jpeg(7, 1.3, None));
    thread::sleep(Duration::from_millis(400));
    let (_, c) = r.printer_call("POST", "/api/p/prusa/plate/cleared", "prusa", &json!({}));
    assert_eq!(c["cleared"], true, "{c}");
    assert_eq!(c["check"]["outcome"], "clear", "{c}");
    cam.show(plate_jpeg(7, 1.0, Some((260, 120, 110, 90))));
    thread::sleep(Duration::from_millis(400));
    let (_, c) = r.printer_call("POST", "/api/p/prusa/plate/cleared", "prusa", &json!({}));
    assert_eq!(c["cleared"], false, "{c}");
    assert_eq!(c["check"]["outcome"], "not_clear");
    let bbox = c["check"]["box"].as_array().unwrap();
    assert!(
        bbox[0].as_f64().unwrap() > 0.3 && bbox[2].as_f64().unwrap() < 0.75,
        "the part is located, not the whole plate: {bbox:?}"
    );
    let frame_id = c["check"]["check_id"].as_i64().unwrap();
    assert_eq!(
        r.call(
            "GET",
            &format!("/api/p/prusa/plate/frame?id={frame_id}"),
            None,
            false
        )
        .0,
        200,
        "the frame is kept with the decision"
    );
    // A plate that is not clear is an alert, said once, naming the printer (WO-11).
    let alerts = r.get("/api/notify")["recent"].clone();
    let failed: Vec<&serde_json::Value> = alerts
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "plate_failed" && e["printer"] == "prusa")
        .collect();
    assert_eq!(failed.len(), 1, "{alerts}");
    assert!(
        failed[0]["text"].as_str().unwrap().contains("Prusa")
            && failed[0]["text"].as_str().unwrap().contains("not started"),
        "{failed:?}"
    );

    // 3. Starting with the part still there: refused, and the printer was sent nothing.
    let (code, s) = r.printer_call(
        "POST",
        "/api/p/prusa/queue/start",
        "prusa",
        &json!({"job":job}),
    );
    assert_eq!(code, 200, "{s}");
    assert_eq!(s["started"], false, "{s}");
    assert_eq!(s["can_override"], true);
    assert_eq!(started(), before, "nothing reached the printer");
    assert!(p.state.lock().unwrap().state != "PRINTING");

    // 4. A confirmed override sends it.
    let (code, s) = r.printer_call(
        "POST",
        "/api/p/prusa/queue/start",
        "prusa",
        &json!({"job":job,"override":true}),
    );
    assert_eq!(code, 200, "{s}");
    assert_eq!(s["started"], true, "{s}");
    assert!(
        started() > before,
        "the file was sent and started: {:?}",
        p.log()
    );
    let checks = r.get("/api/p/prusa/plate")["checks"].clone();
    let outcomes: Vec<&str> = checks
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["outcome"].as_str())
        .collect();
    assert!(
        outcomes.contains(&"override")
            && outcomes.contains(&"not_clear")
            && outcomes.contains(&"clear"),
        "{outcomes:?}"
    );
    let _ = (r.port, &cam.addr);
}


/// Finish-2 F05 (2026-10-19): a dry run of a queued file sends its no-extrusion copy, named so, without looking at the plate,
/// and leaves the job waiting (a FlashForge cannot hand back a file on it, so the queue's copy is the only way there).
#[test]
fn a_queued_file_can_be_dry_run_and_stays_in_the_queue() {
    let dir = Dir::new("queue-dry");
    let a = FakeMoonraker::start(0);
    let p = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Prusa","driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"secret"},
        "slicer_model":"COREONEL","nozzle_mm":0.4});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    wait_for("prusa online", || {
        r.get("/api/printers")["printers"].as_array().unwrap().iter().any(|x| x["id"] == "prusa" && x["online"] == true)
    });
    let (_, ub) = r.upload("lid.gcode", &sliced("COREONEL", 0.4, "lid"), true);
    let job = ub["job"]["id"].as_i64().unwrap();
    let (code, s) = r.printer_call("POST", "/api/p/prusa/queue/start", "prusa", &json!({"job":job,"dry_run":true}));
    assert_eq!(code, 200, "{s}");
    assert_eq!((s["started"].clone(), s["file"].clone()), (json!(true), json!("lid (dry run).gcode")), "{s}");
    let sent: Vec<String> = p.log().iter().filter(|e| e["method"] == "PUT").filter_map(|e| e["path"].as_str().map(str::to_owned)).collect();
    assert!(sent.iter().any(|x| x.contains("dry")) && !sent.iter().any(|x| x.ends_with("/lid.gcode")), "{sent:?}");
    assert!(r.get("/api/p/prusa/plate")["checks"].as_array().is_none_or(|c| c.is_empty()), "the plate was not looked at");
    let waiting = r.get("/api/queue")["jobs"].as_array().unwrap().iter().any(|j| j["id"] == job && j["state"] == "queued");
    assert!(waiting, "{}", r.get("/api/queue"));
}
// ---- WO-09: the farm licence ----------------------------------------------------------------------------

/// Five printers are free and unmentioned; the sixth is asked about and still added; a licence that checks
/// out is kept and says who it is for; one that does not is refused and writes nothing.
#[test]
fn the_sixth_printer_is_asked_about_and_never_refused() {
    let dir = Dir::new("licence");
    let a = FakeMoonraker::start(0);
    let fakes: Vec<FakeMoonraker> = (0..5).map(|_| FakeMoonraker::start(0)).collect();
    let r = run(&dir, base_for(&dir, &a));
    let path = dir.file("farm-licence.txt");
    // One printer, then five: free, no licence, no note.
    let status = r.get("/api/licence");
    assert_eq!(
        (
            status["state"].clone(),
            status["wanted"].clone(),
            status["free_printers"].clone()
        ),
        (json!("free"), json!(false), json!(5))
    );
    for (i, f) in fakes.iter().take(4).enumerate() {
        let add = json!({"name":format!("Bench {i}"),"address":f.url()});
        assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    }
    let s5 = r.get("/api/licence");
    assert_eq!(
        (
            s5["printers"].clone(),
            s5["state"].clone(),
            s5["wanted"].clone()
        ),
        (json!(5), json!("free"), json!(false))
    );
    // The sixth is added, as it always was; it is only that a licence is now wanted.
    let add = json!({"name":"Bench 5","address":fakes[4].url()});
    assert_eq!(
        r.call("POST", "/api/printers", Some(&add), true).0,
        200,
        "no feature is switched off"
    );
    let s6 = r.get("/api/licence");
    assert_eq!(
        (
            s6["printers"].clone(),
            s6["state"].clone(),
            s6["wanted"].clone()
        ),
        // Finish-2 F06 (2026-10-19): this said `wanted` / true. That was the bug: the sixth printer wrote the start of the 30-day trial into the file, but the running
        // service went on reading the file as it was at its start, so it showed "a licence is wanted" with no trial running until the next restart. The sixth real printer
        // starts the trial and the running service says so: `the_sixth_real_printer_starts_one_trial_the_running_service_shows_and_later_printers_do_not_restart_it`.
        (json!(6), json!("trial"), json!(false))
    );

    // A made-up licence is refused in words and nothing is saved.
    let (code, body) = r.call(
        "POST",
        "/api/licence",
        Some(&json!({"text":"SFL1.AAAA.BBBB"})),
        true,
    );
    assert_eq!(code, 400, "{body}");
    assert!(body["error"].as_str().unwrap().len() > 20, "{body}");
    assert!(!std::path::Path::new(&path).exists());
    assert_eq!(
        r.call(
            "POST",
            "/api/licence",
            Some(&json!({"text":"SFL1.AAAA.BBBB"})),
            false
        )
        .0,
        401,
        "needs the farm token"
    );

    // A licence made by the test key: saved, says who, covers 8; the same text is refused by the built-in key.
    let seed = [3_u8; 32];
    let key = crate::licence::public_key_of(&seed);
    let text = crate::licence::issue(&seed, "Maple Street Print Farm", 8, "2026-10-12", None);
    assert!(
        r.farm.save_licence(&text).is_err(),
        "the built-in key does not know this one"
    );
    let saved = r.farm.save_licence_with(&text, &key).unwrap();
    assert_eq!(
        (
            saved["state"].clone(),
            saved["licence"]["name"].clone(),
            saved["licence"]["printers"].clone()
        ),
        (
            json!("licensed"),
            json!("Maple Street Print Farm"),
            json!(8)
        ),
        "{saved}"
    );
    assert!(fs::read_to_string(&path).unwrap().starts_with("SFL1."));
    // It covers 8: a ninth would be asked about again.
    let more = crate::licence::status(Some(&text), 9, &key);
    assert_eq!(more["state"], "too_few");
    r.farm.remove_licence().unwrap();
    assert!(!std::path::Path::new(&path).exists());
}

/// A hung printer must not hold up the others. 2026-10-19 (finish-2 F01): the test used to compare tick
/// counts per window of wall time, healthy against hung, which a busy machine (a build beside it) spoils with
/// no hang at all: it failed under load in polish WO-02 and WO-06. It now asks the one question that a hang
/// changes and load does not: while Stella answers nothing for 8 s, do Bench and Shop still get fresh ticks?
/// Each polls every 500 ms, so a free printer shows a dozen new ticks in 6 s and still several under heavy
/// load; one stuck behind Stella's 8 s stall could show at most one. The same counter then sees Bench stall.
#[test]
fn a_hung_printer_does_not_delay_the_others() {
    let dir = Dir::new("hung");
    let (a, b, c) = (
        FakeMoonraker::start(0),
        FakeMoonraker::start(0),
        FakeMoonraker::start(0),
    );
    let cfg = Config {
        printers: vec![
            PrinterEntry {
                id: "stella".into(),
                name: "Stella".into(),
                address: a.url(),
                database: Some(dir.file("stella.sqlite3")),
                ..Default::default()
            },
            PrinterEntry {
                id: "bench".into(),
                name: "Bench".into(),
                address: b.url(),
                ..Default::default()
            },
            PrinterEntry {
                id: "shop".into(),
                name: "Shop".into(),
                address: c.url(),
                ..Default::default()
            },
        ],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    wait_for("three online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .is_some_and(|p| p.iter().all(|x| x["online"] == true))
    });
    // Distinct tick timestamps per printer over `seconds`.
    let fresh = |seconds: f64| -> [usize; 3] {
        let ids = ["stella", "bench", "shop"];
        let mut seen: [Vec<f64>; 3] = Default::default();
        let start = Instant::now();
        while start.elapsed().as_secs_f64() < seconds {
            for (i, id) in ids.iter().enumerate() {
                if let Some(ts) = r.get(&format!("/api/p/{id}/tick"))["ts"].as_f64() {
                    if seen[i].last() != Some(&ts) {
                        seen[i].push(ts);
                    }
                }
            }
            thread::sleep(Duration::from_millis(40));
        }
        // the first value seen is the tick from before the window, not a fresh one
        seen.map(|s| s.len().saturating_sub(1))
    };
    a.state.lock().unwrap().stall_ms = 8000;
    thread::sleep(Duration::from_millis(1000)); // Stella's poll in flight is now stuck
    let during = fresh(6.0);
    eprintln!("TIMING stella hung: stella {} bench {} shop {}", during[0], during[1], during[2]);
    assert!(during[1] >= 4, "Bench was held up while Stella hung: {during:?}");
    assert!(during[2] >= 4, "Shop was held up while Stella hung: {during:?}");
    assert!(during[0] <= 1, "the hung printer itself falls behind, as it should: {during:?}");
    a.state.lock().unwrap().stall_ms = 0;
    // The same counter must see a real stall: stall Bench, and it falls to at most one fresh tick.
    b.state.lock().unwrap().stall_ms = 8000;
    thread::sleep(Duration::from_millis(1000));
    let stalled = fresh(6.0);
    eprintln!("TIMING bench hung: stella {} bench {} shop {}", stalled[0], stalled[1], stalled[2]);
    assert!(stalled[1] <= 1, "the counter cannot tell a stalled printer from a healthy one: {stalled:?}");
    assert!(stalled[2] >= 4, "Shop was held up while Bench hung: {stalled:?}");
    b.state.lock().unwrap().stall_ms = 0;
}


/// Finish-2 F10 (2026-10-19): a printer switched off says so while the others go on, and when it is back on (the same
/// address) it is online again and its recording takes up where it left off, without a restart of the service.
#[test]
fn a_printer_switched_off_and_on_again_comes_back_by_itself() {
    let dir = Dir::new("off-and-on");
    let a = FakeMoonraker::start(0);
    let b = FakeMoonraker::start(0);
    let port_b: u16 = b.addr.rsplit(':').next().unwrap().parse().unwrap();
    let cfg = Config {
        printers: vec![
            PrinterEntry { id: "stella".into(), name: "Stella".into(), address: a.url(), database: Some(dir.file("stella.sqlite3")), ..Default::default() },
            PrinterEntry { id: "bench".into(), name: "Bench".into(), address: b.url(), ..Default::default() },
        ],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    let online = |id: &str| {
        r.get("/api/printers")["printers"].as_array().unwrap().iter().any(|p| p["id"] == id && p["online"] == true)
    };
    let tick = |id: &str| r.get(&format!("/api/p/{id}/tick"))["ts"].as_f64().unwrap_or(0.0);
    wait_for("both online", || online("stella") && online("bench"));
    drop(b); // switched off: nothing answers on its address
    wait_for("bench offline", || !online("bench"));
    let (stella_before, bench_off) = (tick("stella"), tick("bench"));
    thread::sleep(Duration::from_secs(3));
    assert!(online("stella") && tick("stella") > stella_before, "Stella stopped while Bench was off");
    let b = FakeMoonraker::start(port_b); // on again, same address
    b.state.lock().unwrap().print_state = "printing".into();
    wait_for("bench online again", || online("bench"));
    wait_for("bench recording again", || tick("bench") > bench_off);
    wait_for("bench state is live", || {
        r.get("/api/printers")["printers"].as_array().unwrap().iter().any(|p| p["id"] == "bench" && p["state"] == "printing")
    });
}
/// The model library over HTTP (WO-10): the catalogue, the licence gate, a refused file, binding a model to
/// a printer, the calibration and the match helper. A real published model is installed too when
/// `STELLA_TEST_MODEL_SOURCE` names a folder that holds one (skipped otherwise).
#[test]
fn printer_models_are_listed_gated_refused_bound_and_calibrated() {
    let dir = Dir::new("models");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let list = r.get("/api/printer-models");
    assert_eq!(
        list["count"], 72,
        "67 from 3D Print Rig, the E3NG toolchanger from the rig's bundle, and the CORE One L, CORE One, MK3S+ and MINI whose CAD the build carries (finish-2 F09; it was 69)"
    );
    assert!(list["models"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["licence"]["class"].is_string()));
    let sv08 = list["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "sovol_sv08_350")
        .unwrap();
    assert_eq!(
        (sv08["bundled"].clone(), sv08["motion"].clone()),
        (json!(true), json!(true))
    );
    let rook = list["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "rook_mk2_120")
        .unwrap();
    assert_eq!(
        rook["bundled"], false,
        "a non-commercial model is never part of a build"
    );
    assert!(rook["thumb"].is_null(), "and neither is its picture");
    // The picture of an open model comes with the build; the other has none to give.
    let pic = r
        .agent
        .get(&format!(
            "{}/api/printer-models/sovol_sv08_350/thumb",
            r.base
        ))
        .call()
        .unwrap();
    assert_eq!(pic.headers().get("content-type").unwrap(), "image/webp");
    assert_eq!(
        r.agent
            .get(&format!("{}/api/printer-models/rook_mk2_120/thumb", r.base))
            .call()
            .unwrap()
            .status()
            .as_u16(),
        404
    );

    // The licence is confirmed before anything is fetched; the token is needed to install at all.
    let none = dir.file("empty-source");
    fs::create_dir_all(&none).unwrap();
    let body = json!({"source":none,"accept_licence":false});
    assert_eq!(
        r.call(
            "POST",
            "/api/printer-models/rook_mk2_120/install",
            Some(&body),
            false
        )
        .0,
        401
    );
    let (code, v) = r.call(
        "POST",
        "/api/printer-models/rook_mk2_120/install",
        Some(&body),
        true,
    );
    assert_eq!(code, 400);
    assert!(
        v["error"].as_str().unwrap().contains("CC-BY-NC")
            && v["error"].as_str().unwrap().contains("confirm"),
        "{v}"
    );
    // Confirmed, but the folder does not hold it: said plainly.
    let (code, v) = r.call(
        "POST",
        "/api/printer-models/rook_mk2_120/install",
        Some(&json!({"source":none,"accept_licence":true})),
        true,
    );
    assert_eq!(code, 400);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("does not hold this model"),
        "{v}"
    );
    let (_, v) = r.call(
        "POST",
        "/api/printer-models/rook_mk2_120/install",
        Some(&json!({"source":"https://example.com/m","accept_licence":true})),
        true,
    );
    // WO-13: https is now a working source; the failure is the fetch itself, not the scheme.
    assert!(v["error"].as_str().unwrap().contains("answered 404"), "{v}");
    assert_eq!(
        r.call(
            "POST",
            "/api/printer-models/not_a_machine/install",
            Some(&json!({"source":none,"accept_licence":true})),
            true
        )
        .0,
        400
    );

    // A file that is not the published model is refused and nothing is kept.
    let upload = |bytes: &[u8], token: bool| {
        let mut req = r
            .agent
            .post(&format!(
                "{}/api/printer-models/sovol_sv08_350/upload?accept=0&name=x.glb.gz",
                r.base
            ))
            .header("Origin", &r.base)
            .header("Content-Type", "application/octet-stream");
        if token {
            req = req.header("X-Stella-Control-Token", &r.token);
        }
        let mut resp = match req.send(bytes) {
            Ok(resp) => resp,
            Err(_) if !token => return (400, json!({"error":"refused before the file was read"})),
            Err(e) => panic!("upload failed: {e}"),
        };
        let code = resp.status().as_u16();
        (
            code,
            serde_json::from_str::<Value>(&resp.body_mut().read_to_string().unwrap_or_default())
                .unwrap_or(Value::Null),
        )
    };
    assert_eq!(
        upload(b"glTF but not the published model", false).0,
        400,
        "an upload without the token is refused unread"
    );
    let (code, v) = upload(b"glTF but not the published model", true);
    assert_eq!(code, 400);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("not the model the author published"),
        "{v}"
    );
    assert!(r.get("/api/printer-models")["library_has"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        !r.farm.models_dir().join("sovol_sv08_350").exists(),
        "a refused file leaves nothing behind"
    );

    // Binding: only a model in the catalogue; the calibration is checked.
    let id = "stella";
    assert_eq!(
        r.call(
            "PATCH",
            &format!("/api/printers/{id}"),
            Some(&json!({"cad_model":"made_up"})),
            true
        )
        .0,
        400
    );
    assert_eq!(
        r.call(
            "PATCH",
            &format!("/api/printers/{id}"),
            Some(&json!({"cad_model":"sovol_sv08_350"})),
            true
        )
        .0,
        200
    );
    let m = r.get(&format!("/api/p/{id}/model"));
    assert_eq!(
        (
            m["model"].clone(),
            m["installed"].clone(),
            m["motion"].clone()
        ),
        (json!("sovol_sv08_350"), json!(false), json!(true)),
        "{m}"
    );
    assert!(
        m["file_url"].is_null(),
        "an uninstalled model has no file to draw"
    );
    // The fake reports (175, 175, 10): the starting calibration puts the model's axis X at that plus its own offset. (Polish WO-03,
    // 2026-10-18: the SV08's starting calibration is now measured from its model, -156.8 where it was the travel's low end,
    // -160, so the expectation is read from the registered calibration instead of written as 15.)
    wait_for("a position", || {
        r.get(&format!("/api/p/{id}/model"))["pose"]["known"] == true
    });
    let m = r.get(&format!("/api/p/{id}/model"));
    let off = crate::printer_models::profile("sovol_sv08_350").unwrap()["calibration"]["offset_mm"][0]
        .as_f64()
        .unwrap();
    assert!((m["pose"]["axes"]["x"].as_f64().unwrap() - (175.0 + off)).abs() < 1e-9, "{m}");
    assert!((off + 156.79).abs() < 0.01, "the measured offset: {off}");
    assert_eq!(m["calibrated"], false);
    let bad = json!({"cad_calibration":{"offset_mm":[0,0,0],"sign":[1,1,3],"source":[0,1,2]}});
    assert_eq!(
        r.call("PATCH", &format!("/api/printers/{id}"), Some(&bad), true)
            .0,
        400
    );
    let good =
        json!({"cad_calibration":{"offset_mm":[-100,-80,0],"sign":[1,-1,1],"source":[0,1,2]}});
    assert_eq!(
        r.call("PATCH", &format!("/api/printers/{id}"), Some(&good), true)
            .0,
        200
    );
    let m = r.get(&format!("/api/p/{id}/model"));
    // The position -175 - 80 is below the end of Y's travel, so Y is held at its low end: -80 in the registration before polish
    // WO-03 (2026-10-18), -86.3 now that the travel reaches the plate; read from the profile so the test says what it means.
    let low_y = crate::printer_models::profile("sovol_sv08_350").unwrap()["axes"]["y"][0].as_f64().unwrap();
    assert_eq!(
        (
            m["calibrated"].clone(),
            m["pose"]["axes"]["x"].clone(),
            m["pose"]["axes"]["y"].clone()
        ),
        (json!(true), json!(75.0), json!(low_y)),
        "{m}"
    );
    assert_eq!(
        m["pose"]["clamped"],
        json!(["y"]),
        "reversed Y runs past the end of the model's travel"
    );
    // Match: ask for the model to read (0, 0, 50) where the printer is now.
    let (code, v) = r.call(
        "GET",
        &format!("/api/p/{id}/model/match?x=0&y=0&z=50"),
        None,
        false,
    );
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["pose"]["axes"], json!({"x":0.0,"y":0.0,"z":50.0}));
    assert_eq!(
        r.call("GET", &format!("/api/p/{id}/model/match?x=0"), None, false)
            .0,
        400
    );
    // A bound model cannot be removed from under the printer.
    let (code, v) = r.call("DELETE", "/api/printer-models/sovol_sv08_350", None, true);
    assert_eq!(code, 400);
    assert!(
        v["error"].as_str().unwrap().contains("uses this model"),
        "{v}"
    );
    // Unbinding clears the calibration with it.
    assert_eq!(
        r.call(
            "PATCH",
            &format!("/api/printers/{id}"),
            Some(&json!({"cad_model":null})),
            true
        )
        .0,
        200
    );
    let m = r.get(&format!("/api/p/{id}/model"));
    assert!(
        m["model"].is_null() && m["reason"].as_str().unwrap().contains("no model"),
        "{m}"
    );

    // A real model, when one is at hand: installs, is checked, is served, and can be removed.
    if let Ok(src) = std::env::var("STELLA_TEST_MODEL_SOURCE") {
        let (code, v) = r.call(
            "POST",
            "/api/printer-models/rook_mk2_120/install",
            Some(&json!({"source":src,"accept_licence":true})),
            true,
        );
        assert_eq!(code, 200, "{v}");
        assert_eq!(
            v["decoded_sha256"],
            "16d278caa466868340ad8c97d91b80d96bfbc2af2d871565ab8d1e3559faf861"
        );
        assert_eq!(v["licence"]["class"], "nc");
        let file = r
            .agent
            .get(&format!("{}/api/printer-models/rook_mk2_120/file", r.base))
            .call()
            .unwrap();
        assert_eq!(file.status().as_u16(), 200);
        assert_eq!(
            r.get("/api/printer-models")["library_has"],
            json!(["rook_mk2_120"])
        );
        assert_eq!(
            r.call("DELETE", "/api/printer-models/rook_mk2_120", None, true)
                .0,
            200
        );
        assert!(r.get("/api/printer-models")["library_has"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}

/// Polish WO-01 (2026-10-18): the options (toolchangers, toolheads, add-ons) over HTTP: listed with their licences,
/// gated before anything is fetched, refused on a machine they do not fit, chosen for a printer and drawn as a scene.
#[test]
fn options_are_listed_gated_refused_on_the_wrong_machine_and_served_as_a_scene() {
    let dir = Dir::new("options");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let list = r.get("/api/printer-models/options");
    assert!(list["count"].as_u64().unwrap() > 150, "{}", list["count"]);
    let bank = list["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "stealthchanger_bank")
        .unwrap();
    assert_eq!(
        (
            bank["licence"]["class"].clone(),
            bank["bundled"].clone(),
            bank["drawable"].clone()
        ),
        (json!("nc"), json!(false), json!(true))
    );
    assert!(bank["fits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m == "voron_v24_300_printed"));
    assert!(
        !bank["fits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m == "voron_trident_300"),
        "a Trident does not take the standard dock"
    );
    assert!(list["options"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["licence"]["class"].is_string() && o["licence"]["text"].is_string()));
    let detail = r.get("/api/printer-models/options/stealthchanger_bank");
    assert_eq!(detail["files"].as_array().unwrap().len(), 8);
    assert!(detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["id"] == "xol" && f["class"] == "nc"));
    assert!(detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["id"] == "changer_sc_dock_xol" && f["class"] == "gpl"));
    let for_machine = r.get("/api/printer-models/voron_v24_300_printed/options");
    let b = for_machine["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "stealthchanger_bank")
        .unwrap();
    assert_eq!(b["capacity"], 4);
    assert_eq!(b["tools"].as_array().unwrap().len(), 2);
    assert_eq!(
        r.agent
            .get(&format!("{}/api/printer-models/options/made_up", r.base))
            .call()
            .map(|x| x.status().as_u16())
            .unwrap_or(404),
        404
    );
    assert_eq!(
        r.agent
            .get(&format!("{}/api/printer-models/parts/xol/file", r.base))
            .call()
            .map(|x| x.status().as_u16())
            .unwrap_or(404),
        404,
        "a file that is not installed is not served"
    );

    // The licence is confirmed before anything is fetched, and the token is needed to install at all.
    let none = dir.file("empty-options-source");
    fs::create_dir_all(&none).unwrap();
    let path = "/api/printer-models/options/stealthchanger_bank/install";
    assert_eq!(
        r.call(
            "POST",
            path,
            Some(&json!({"source":none,"accept_licence":true})),
            false
        )
        .0,
        401
    );
    let (code, v) = r.call(
        "POST",
        path,
        Some(&json!({"source":none,"accept_licence":false})),
        true,
    );
    assert_eq!(code, 400);
    assert!(
        v["error"].as_str().unwrap().contains("CC BY-NC-SA")
            && v["error"].as_str().unwrap().contains("confirm"),
        "{v}"
    );
    let (code, v) = r.call(
        "POST",
        path,
        Some(&json!({"source":none,"accept_licence":true})),
        true,
    );
    assert_eq!(code, 400);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("does not hold this model"),
        "{v}"
    );
    assert_eq!(
        r.call(
            "POST",
            "/api/printer-models/options/made_up/install",
            Some(&json!({"source":none,"accept_licence":true})),
            true
        )
        .0,
        400
    );

    // Choosing options: a model first, then only what fits it.
    let patch = |body: Value| r.call("PATCH", "/api/printers/stella", Some(&body), true);
    let (code, v) = patch(json!({"cad_options":[{"option":"sturdy_handles_m5"}]}));
    assert_eq!(code, 400, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("choose a model"),
        "{v}"
    );
    assert_eq!(patch(json!({"cad_model":"voron_trident_300"})).0, 200);
    let (code, v) = patch(json!({"cad_options":[{"option":"stealthchanger_bank"}]}));
    assert_eq!(code, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("does not fit"), "{v}");
    assert_eq!(
        patch(json!({"cad_options":[{"option":"sturdy_handles_m5"}]})).0,
        200
    );
    let m = r.get("/api/p/stella/model");
    assert_eq!(
        m["scene"]["placements"].as_array().unwrap().len(),
        1,
        "the handles are one file on a Trident 300: {m}"
    );
    assert!(m["scene"]["placements"][0]["url"]
        .as_str()
        .unwrap()
        .starts_with("/api/printer-models/parts/sturdy_handles_m5"));
    assert_eq!(
        m["scene"]["missing"],
        json!(["sturdy_handles_m5_trident_300"]),
        "nothing is installed, and the page is told what to fetch"
    );
    // The pose tickers leave the scene out (`?scene=0`); the options themselves stay.
    let quick = r.get("/api/p/stella/model?scene=0");
    assert!(
        quick["scene"].is_null() && quick["options"][0]["option"] == "sturdy_handles_m5",
        "{quick}"
    );
    // A new model starts without the old machine's options.
    assert_eq!(patch(json!({"cad_model":"voron_v24_300_printed"})).0, 200);
    assert!(r.get("/api/p/stella/model")["scene"].is_null());
    let sel = json!({"cad_options":[{"option":"stealthchanger_bank","tools":["xol_sherpa_mini_rapido2_uhf","jabberwocky_jabberwocky_drive_conch"],"active":1}]});
    assert_eq!(patch(sel).0, 200);
    let m = r.get("/api/p/stella/model");
    assert_eq!(m["options"][0]["active"], 1);
    let placed: Vec<&str> = m["scene"]["placements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["follows"] == "head")
        .filter_map(|p| p["part"].as_str())
        .collect();
    assert!(
        placed.contains(&"head_jabberwocky") && !placed.contains(&"xol"),
        "the second dock's head is the one on the gantry: {placed:?}"
    );
    assert_eq!(
        m["scene"]["hide_machine_keys"].as_array().unwrap().len(),
        117
    );
    // The printer in use blocks removing what it chose.
    let (code, v) = r.call(
        "DELETE",
        "/api/printer-models/options/stealthchanger_bank",
        None,
        true,
    );
    assert_eq!(code, 400);
    assert!(
        v["error"].as_str().unwrap().contains("uses this option"),
        "{v}"
    );
    assert_eq!(patch(json!({"cad_options":null})).0, 200);
    assert!(r.get("/api/p/stella/model")["scene"].is_null());
    assert_eq!(
        r.call(
            "DELETE",
            "/api/printer-models/options/stealthchanger_bank",
            None,
            true
        )
        .0,
        200
    );
}

/// WO-14 item 4 (2026-10-13): a printer added without a bed size takes it from the machine's own
/// report (the Adventurer 5M's M115 X/Y; a Prusa's hostname through the known-model table with
/// prusa3d.com published volumes), never the 350 mm default of another machine. It stays editable
/// in printer settings.
#[test]
fn a_printer_without_a_bed_size_takes_the_machines_own() {
    let dir = Dir::new("bed-hint");
    let a = FakeMoonraker::start(0);
    let f = FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
    let p = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));

    // FlashForge: M115 carries X: 220 Y: 220.
    let add = json!({"name":"Adventurer 5M","driver":"flashforge","address":f.http_url(),"credentials":{"serial":"SNFAKE5M00001","check_code":"0fake000","tcp_port":f.tcp_port().to_string()}});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    wait_for("the AD5M online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "adventurer-5m" && x["online"] == true)
    });
    let ad5m = r.get("/api/printers");
    let ad5m = ad5m["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "adventurer-5m")
        .unwrap()
        .clone();
    assert_eq!(ad5m["bed_mm"], json!([220.0, 220.0]), "{ad5m}");

    // PrusaLink: /api/v1/info hostname "prusa-core-one-l" maps to 300x300 (prusa3d.com CORE One L
    // print volume 300 x 300 x 330 mm).
    let add = json!({"name":"Core One L","driver":"prusalink","address":p.url(),"credentials":{"user":"maker","password":"secret"}});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    wait_for("the Prusa online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "core-one-l" && x["online"] == true)
    });
    let list = r.get("/api/printers");
    let core = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "core-one-l")
        .unwrap()
        .clone();
    assert_eq!(core["bed_mm"], json!([300.0, 300.0]), "{core}");

    // It stays editable in printer settings (PATCH), with validation.
    let (code, body) = r.call(
        "PATCH",
        "/api/printers/core-one-l",
        Some(&json!({"bed_size_mm":[10.0,10.0]})),
        true,
    );
    assert_eq!(code, 400, "{body}");
    let (code, body) = r.call(
        "PATCH",
        "/api/printers/core-one-l",
        Some(&json!({"bed_size_mm":[250.0,220.0]})),
        true,
    );
    assert_eq!(code, 200, "{body}");
    let list = r.get("/api/printers");
    let core = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "core-one-l")
        .unwrap()
        .clone();
    assert_eq!(core["bed_mm"], json!([250.0, 220.0]), "{core}");
}

/// WO-14 item 1: a printer with no catalogue model but its own scene CAD in the managed library
/// reports scene_model and a scene fallback on its model endpoint, so the farm wall draws the machine.
#[test]
fn a_scene_model_counts_as_the_bound_model() {
    let dir = Dir::new("scene-model");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let list = r.get("/api/printers");
    let stella = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "stella")
        .unwrap()
        .clone();
    // The scratch base has no assets yet: no scene model is claimed.
    assert_eq!(stella["scene_model"], json!(false));

    // The managed library gains the reference machine CAD, as a real Stella config has.
    let lib = format!("{}.assets", dir.file("stella.sqlite3"));
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        format!("{lib}/manifest.json"),
        r#"{"schema":"stella-scope.assets.v1","assets":[{"id":"hartk-voron-24r2-350-reference"}]}"#,
    )
    .unwrap();
    let list = r.get("/api/printers");
    let stella = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "stella")
        .unwrap()
        .clone();
    assert_eq!(stella["scene_model"], json!(true));
    // The model endpoint falls back to the scene, with the last known position for the pose.
    let m = r.get("/api/p/stella/model");
    assert_eq!(m["scene"], json!(true), "{m}");
    assert!(m["position"].is_object(), "{m}");
}

/// WO-15 (2026-10-13): a slicer posting an OctoPrint shape upload (`POST /api/files/local`,
/// X-Api-Key: the farm control token) gets the host check from `/api/version` and the file lands
/// in the farm queue. A wrong key is refused before the bytes are read; `print` is never armed
/// without the plate check.
#[test]
fn an_octoprint_upload_lands_in_the_farm_queue() {
    let dir = Dir::new("octo-upload");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));

    let (code, v) = r.call("GET", "/api/version", None, false);
    assert_eq!(code, 200, "{v}");
    assert!(v["text"].as_str().unwrap().contains("OctoPrint"), "{v}");

    let gcode = sliced("COREONEL", 0.4, "cube");
    let boundary = "----octo-kilo-1";
    let mut mp = Vec::new();
    mp.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"print\"\r\n\r\ntrue\r\n")
            .as_bytes(),
    );
    mp.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cube.gcode\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
    mp.extend_from_slice(&gcode);
    mp.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    // A wrong key never gets as far as reading the file.
    let bad = r
        .agent
        .post(&format!("{}/api/files/local", r.base))
        .header("X-Api-Key", "wrong")
        .header(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send(mp.clone());
    assert_eq!(bad.unwrap().status().as_u16(), 400);
    assert_eq!(r.get("/api/queue")["jobs"].as_array().unwrap().len(), 0);

    let post = |key: &str| {
        r.agent
            .post(&format!("{}/api/files/local", r.base))
            .header("X-Api-Key", key)
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(mp.clone())
            // a refused upload is answered before its body is read, so the client may see a reset
            .map_or(400, |r| r.status().as_u16())
    };
    // 2026-10-13: the slicer has its own key; the farm control key is refused until the owner allows it.
    let slicer = r.get("/api/slicer");
    let key = slicer["key"].as_str().unwrap().to_owned();
    assert_eq!(slicer["accept_farm_key"], json!(false), "{slicer}");
    assert_eq!(
        post(&r.token),
        400,
        "the farm key is not a slicer key by default"
    );
    let (code, v) = r.call("POST", "/api/slicer", Some(&json!({"enabled":false})), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(
        post(&key),
        400,
        "uploads off: even the slicer key is refused"
    );
    let (code, _) = r.call(
        "POST",
        "/api/slicer",
        Some(&json!({"enabled":true,"accept_farm_key":true})),
        true,
    );
    assert_eq!(code, 200);
    assert_eq!(post(&r.token), 201, "allowed: the farm key works too");
    let (_, v) = r.call(
        "POST",
        "/api/slicer",
        Some(&json!({"accept_farm_key":false,"new_key":true})),
        true,
    );
    assert_ne!(v["key"], json!(key), "a new key replaces the old one");
    assert_eq!(post(&key), 400, "the old key stops working");
    let code = post(v["key"].as_str().unwrap());
    assert_eq!(code, 201);
    let q = r.get("/api/queue");
    let jobs = q["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2, "{q}");
    assert_eq!(jobs[0]["file"], "cube.gcode", "{q}");
    assert_eq!(jobs[0]["needs"]["printer_model"], "COREONEL", "{q}");
}

/// 2026-10-18 (polish WO-06 item 2): the cause of `an_octoprint_upload_lands_in_the_farm_queue`
/// failing about one run in fifteen. A refused upload is answered before its body is read; closing
/// a socket that still holds unread bytes sends a reset, and the reset can destroy the answer on its
/// way, so the slicer saw "Connection reset by peer" instead of the reason. The service now says its
/// answer, closes its sending side and reads away what is still arriving. A body far larger than the
/// socket buffers makes the old behaviour fail every time instead of one time in fifteen.
#[test]
fn a_refused_upload_hears_why_however_much_it_was_sending() {
    use std::io::{Read, Write};
    let dir = Dir::new("refused-upload");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let big = 8 * 1024 * 1024;
    for round in 0..8 {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", r.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(format!("POST /api/files/local HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Api-Key: wrong\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {big}\r\n\r\n", r.port).as_bytes()).unwrap();
        // Keep sending the way a slicer does; a write that fails because the service closed is not
        // the test's business, the answer is.
        let chunk = vec![b'x'; 64 * 1024];
        let mut sent = 0;
        while sent < big && s.write_all(&chunk).is_ok() {
            sent += chunk.len();
        }
        let mut answer = Vec::new();
        let read = s.read_to_end(&mut answer);
        let text = String::from_utf8_lossy(&answer);
        assert!(
            read.is_ok()
                && text.starts_with("HTTP/1.1 400")
                && text.contains("API key did not match"),
            "round {round}: sent {sent} of {big}, read {read:?}, got {text:?}"
        );
    }
}

/// 2026-10-13 (the Watchtower floor): a spool on a printer loses what that printer's finished
/// prints used, says so on the farm list, warns once when low (alarm feed), the analytics read the
/// same recording, and a backup carries the farm and its books but no secret, and restores into
/// another farm without touching what is there.
#[test]
fn spools_analytics_feed_and_backup_come_from_the_recording() {
    let dir = Dir::new("fleet-books");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let spool = json!({"material":"PETG","brand":"Sunlu","colour":"#ff6a00","grams_total":1000,"low_at_g":950,"printer":"stella","price_per_kg":25});
    let (code, _) = r.call("POST", "/api/spools", Some(&spool), false);
    assert_eq!(code, 401, "no token, no spool");
    let (code, v) = r.call("POST", "/api/spools", Some(&spool), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["spools"][0]["grams_left"], json!(1000.0), "{v}");
    assert_eq!(v["spools"][0]["colour"], "#FF6A00");
    let (code, v) = r.call(
        "POST",
        "/api/spools",
        Some(&json!({"material":"PLA","colour":"red"})),
        true,
    );
    assert_eq!(code, 400, "a colour is #RRGGBB: {v}");

    // Two prints end on Stella after the spool went on: 20 m (complete) and 1 m (failed) of PETG.
    let on = r.get("/api/spools")["spools"][0]["assigned_at"]
        .as_f64()
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    let db = rusqlite::Connection::open(dir.file("stella.sqlite3")).unwrap();
    for (start, mm, result) in [
        (on + 0.05, 20_000.0, "complete"),
        (on + 0.15, 1_000.0, "error"),
    ] {
        db.execute(
            "INSERT INTO print_sessions(start_ts,end_ts,filename,duration_s,filament_used,result,evidence) VALUES(?1,?2,'x.gcode',3600,?3,?4,'test')",
            rusqlite::params![start, start + 0.05, mm, result],
        )
        .unwrap();
    }
    wait_for("the spool to see the prints", || {
        r.get("/api/spools")["spools"][0]["grams_left"]
            .as_f64()
            .is_some_and(|g| g < 1000.0)
    });
    let s = &r.get("/api/spools")["spools"][0];
    let left = s["grams_left"].as_f64().unwrap();
    // 21 m of 1.75 mm PETG at 1.27 g/cm³ is 64.1 g
    assert!((left - 935.9).abs() < 0.3, "{s}");
    assert_eq!(s["low"], json!(true));
    let list = r.get("/api/printers");
    assert_eq!(list["printers"][0]["spool"]["material"], "PETG", "{list}");

    let an = r.get("/api/analytics?days=1");
    let st = &an["printers"][0];
    assert_eq!(st["completed"], json!(1), "{an}");
    assert_eq!(st["failed"], json!(1), "{an}");
    assert_eq!(st["success_rate"], json!(0.5), "{an}");
    assert!((st["grams"].as_f64().unwrap() - 64.1).abs() < 0.3, "{an}");
    assert!((st["cost"].as_f64().unwrap() - 1.60).abs() < 0.02, "{an}");

    wait_for("the low spool warning in the feed", || {
        r.get("/api/alerts/feed")["events"]
            .as_array()
            .is_some_and(|e| {
                e.iter().any(|x| {
                    x["kind"] == "spool_low"
                        && x["text"]
                            .as_str()
                            .unwrap_or("")
                            .contains("Sunlu PETG on Stella")
                })
            })
    });

    // The backup has the farm and the books, nothing secret; another farm takes the spool and
    // leaves its own Stella alone.
    let backup = r.get("/api/farm/backup");
    assert_eq!(backup["format"], "stella-scope-farm-backup");
    let text = backup.to_string();
    assert!(
        !text.contains("credentials_ref") && !text.contains("apikey") && !text.contains(".secret"),
        "{text}"
    );
    assert_eq!(backup["books"]["spools"].as_array().unwrap().len(), 1);
    let dir2 = Dir::new("fleet-restore");
    let b = FakeMoonraker::start(0);
    let r2 = run(&dir2, base_for(&dir2, &b));
    let (code, v) = r2.call("POST", "/api/farm/restore", Some(&backup), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["skipped"], json!(["stella"]), "{v}");
    assert_eq!(v["spools_added"], json!(1), "{v}");
    assert_eq!(r2.get("/api/spools")["spools"].as_array().unwrap().len(), 1);

    // Webhook addresses: only the two services' own https hosts.
    assert!(crate::notify::webhook_address(
        "discord",
        "https://discord.com/api/webhooks/123/abc-DEF_9"
    )
    .is_ok());
    assert!(
        crate::notify::webhook_address("discord", "https://evil.example/api/webhooks/1/x").is_err()
    );
    assert!(
        crate::notify::webhook_address("discord", "http://discord.com/api/webhooks/1/x").is_err()
    );
    assert!(
        crate::notify::webhook_address("slack", "https://hooks.slack.com/services/T0/B0/xyz")
            .is_ok()
    );
    assert!(
        crate::notify::webhook_address("slack", "https://hooks.slack.com/services/T0?x=@evil")
            .is_err()
    );
    assert_eq!(
        crate::notify::webhook_body("slack", "Stella", "done")["text"],
        "*Stella*: done"
    );
    assert_eq!(
        crate::notify::severity("done", "Stella: print failed"),
        "alarm"
    );
}

/// 2026-10-13: a printer on a smart plug shows its power on the farm list, switches through the farm,
/// and its power is not cut while it prints unless the person confirmed it.
#[test]
fn a_smart_plug_is_read_switched_and_never_cut_mid_print_by_accident() {
    use crate::fakes::{serve_http, Rsp};
    let on = std::sync::Arc::new(std::sync::Mutex::new(true));
    let o = on.clone();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let plug = l.local_addr().unwrap().to_string();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    serve_http(
        l,
        stop.clone(),
        std::sync::Arc::new(move |req, _| {
            let mut v = o.lock().unwrap();
            if let Some(q) = req.target.strip_prefix("/rpc/Switch.Set?id=0&on=") {
                *v = q == "true";
            }
            Some(Rsp::json(
                json!({"output":*v,"apower":if *v {120.0} else {0.0},"aenergy":{"total":2000.0}}),
            ))
        }),
    );
    let dir = Dir::new("smart-plug");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let (code, v) = r.call(
        "PATCH",
        "/api/printers/stella",
        Some(&json!({"plug":"amazon://plug"})),
        true,
    );
    assert_eq!(
        code, 400,
        "a cloud-only plug is refused with the reason: {v}"
    );
    let (code, v) = r.call(
        "PATCH",
        "/api/printers/stella",
        Some(&json!({"plug":format!("shelly://{plug}")})),
        true,
    );
    assert_eq!(code, 200, "{v}");
    wait_for("the plug's power on the farm list", || {
        r.get("/api/printers")["printers"][0]["plug"]["reading"]["watts"] == json!(120.0)
    });
    a.state.lock().unwrap().print_state = "printing".into();
    a.state.lock().unwrap().filename = "cube.gcode".into();
    wait_for("the printer to be printing", || {
        r.get("/api/printers")["printers"][0]["state"] == "printing"
    });
    let (code, v) = r.call(
        "POST",
        "/api/plugs/stella",
        Some(&json!({"on":false})),
        true,
    );
    assert_eq!(code, 400, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("ends the print"),
        "{v}"
    );
    assert!(*on.lock().unwrap(), "the power stayed on");
    let (code, _) = r.call(
        "POST",
        "/api/plugs/stella",
        Some(&json!({"on":false})),
        false,
    );
    assert_eq!(code, 401, "no token, no switch");
    let (code, v) = r.call(
        "POST",
        "/api/plugs/stella",
        Some(&json!({"on":false,"force":true})),
        true,
    );
    assert_eq!(code, 200, "{v}");
    assert!(!*on.lock().unwrap());
    assert_eq!(v["reading"]["on"], json!(false));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
}

// ---- polish WO-02: the demo farm -------------------------------------------------------------------------

/// One press adds the eight demo printers and one takes them away. A real printer beside them is never
/// touched, a demo never counts toward the licence, the configuration is written once for all eight, and a
/// demo that is the first printer stays. (Polish WO-03, 2026-10-18: the DoomCube 2, which never moved, gave its place
/// to a toolchanger, so the machines changed and the counts did not.)
#[test]
fn the_demo_farm_comes_and_goes_and_only_demo_printers_are_touched() {
    let dir = Dir::new("demofarm");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let r = run(&dir, base_for(&dir, &a));
    let ids = |r: &Running| -> Vec<String> {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_owned())
            .collect()
    };
    // A real printer (a fake Moonraker) beside the first one.
    let add = json!({"name":"Bench","address":b.url()});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    let real = ids(&r);
    assert_eq!(real.len(), 2);
    let backups = || {
        fs::read_dir(&dir.0)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".bak-")
            })
            .count()
    };
    let before = backups();

    // Nothing without the token.
    assert_eq!(
        r.call("POST", "/api/printers/demo-farm", Some(&json!({})), false)
            .0,
        401
    );
    assert_eq!(
        r.call("DELETE", "/api/printers/demo-farm", None, false).0,
        401
    );
    assert_eq!(ids(&r), real);

    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    // finish-2 F09 (2026-10-19): nine printers, the Prusa MK3S+ among them (it was eight)
    assert_eq!(v["added"].as_array().unwrap().len(), 9, "{v}");
    assert_eq!(v["already_there"], json!([]));
    assert_eq!(
        v["missing_models"].as_array().unwrap().len(),
        9,
        "no model is installed in this test (nine demos since finish-2 F09)"
    );
    assert_eq!(
        backups(),
        before + 1,
        "one write of the configuration, not eight"
    );
    // F03 (2026-10-19): without the person's yes to the open licences (`accept_licence`) nothing is fetched and nobody is asked: the answer has no fetch.
    // The wall reads what each tile waits for, and the licences in one line to show beside the button.
    assert!(v.get("fetch").is_none(), "{v}");
    let w = r.get("/api/printer-models/wanted");
    assert_eq!(w["needs"].as_object().unwrap().len(), 9, "{w}");
    assert_eq!(w["fetch"]["state"], "idle", "{w}");
    let l = r.get("/api/printer-models/demo-farm");
    assert!(l["line"].as_str().unwrap().starts_with("Open licences: ") && l["files"] == 14, "{l}");
    let rows = r.get("/api/printers");
    let rows = rows["printers"].as_array().unwrap();
    assert_eq!(rows.len(), 11);
    let demos: Vec<&Value> = rows.iter().filter(|p| p["driver"] == "demo").collect();
    assert_eq!(demos.len(), 9);
    assert!(demos
        .iter()
        .all(|p| p["model"].is_string() && p["id"].as_str().unwrap().starts_with("demo-")));
    // The saved file holds all eleven (finish-2 F09: it was ten), each demo with its machine and where it starts.
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(dir.file("stella-scope.json")).unwrap()).unwrap();
    let entries = saved["printers"].as_array().unwrap();
    assert_eq!(entries.len(), 11);
    let trident = entries.iter().find(|e| e["id"] == "demo-trident").unwrap();
    assert_eq!(
        (
            trident["cad_model"].clone(),
            trident["demo_start_at"].clone()
        ),
        (json!("voron_trident_300"), json!(0.18))
    );
    // finish-2 F03: none starts paused (all eight move at the press)
    assert!(entries.iter().all(|e| e["demo_paused"] != json!(true)));

    // The demos are running, each at its own place in its print.
    wait_for("the staggered demo states", || {
        let rows = r.get("/api/printers");
        let state = |id: &str| {
            rows["printers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == id)
                .map(|p| p["state"].as_str().unwrap_or("").to_owned())
        };
        ["demo-v0", "demo-sv08", "demo-voron-24", "demo-core-one-l"]
            .iter()
            .all(|id| matches!(state(id).as_deref(), Some("printing" | "heating")))
    });
    // The model page knows the machine, its motion and where the head is.
    let m = r.get("/api/p/demo-trident/model");
    assert_eq!(m["model"], "voron_trident_300");
    assert_eq!(m["installed"], json!(false));
    assert_eq!(m["motion"], json!(true));

    // They never count toward the licence: two real printers, free.
    let lic = r.get("/api/licence");
    assert_eq!(
        (lic["printers"].clone(), lic["state"].clone()),
        (json!(2), json!("free")),
        "{lic}"
    );

    // Pressing it again adds nothing and says so.
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!((code, v["added"].clone()), (200, json!([])), "{v}");
    assert_eq!(v["already_there"].as_array().unwrap().len(), 9);
    assert_eq!(ids(&r).len(), 11);

    // No printer can take the name of the button.
    let (code, v) = r.call(
        "POST",
        "/api/printers",
        Some(&json!({"id":"demo-farm","name":"Demo farm","address":b.url()})),
        true,
    );
    assert_eq!(code, 400, "{v}");

    // Taking it away removes the eight and nothing else.
    let (code, v) = r.call("DELETE", "/api/printers/demo-farm", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["removed"].as_array().unwrap().len(), 9);
    assert_eq!(v["kept_first"], json!(false));
    assert_eq!(ids(&r), real, "the real printers are all still here");
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(dir.file("stella-scope.json")).unwrap()).unwrap();
    let left: Vec<&str> = saved["printers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(left, real.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        !dir.0
            .join("stella.sqlite3.printers/demo-trident.sqlite3")
            .exists(),
        "a demo's recording goes with it"
    );
    assert!(
        dir.0.join("stella.sqlite3.printers/bench.sqlite3").exists(),
        "a real printer's recording stays"
    );
    let (code, v) = r.call("DELETE", "/api/printers/demo-farm", None, true);
    assert_eq!((code, v["removed"].clone()), (200, json!([])), "{v}");
    assert_eq!(r.get("/api/licence")["printers"], json!(2));
    // The real printer still answers on its own page.
    assert_eq!(r.get("/api/p/bench/meta")["printer"]["name"], "Bench");

    // Where the first printer is itself a demo it carries the phone link and stays.
    let dir2 = Dir::new("demofarm-first");
    let cfg = Config {
        database: dir2.file("stella.sqlite3"),
        printers: vec![PrinterEntry {
            id: "stella".into(),
            name: "Demo Stella".into(),
            driver: "demo".into(),
            ..PrinterEntry::default()
        }],
        ..Config::default()
    };
    let r2 = run(&dir2, cfg);
    assert_eq!(
        r2.call("POST", "/api/printers/demo-farm", Some(&json!({})), true)
            .0,
        200
    );
    assert_eq!(ids(&r2).len(), 10); // nine demos and the real printer (finish-2 F09: it was eight and one)
    let (code, v) = r2.call("DELETE", "/api/printers/demo-farm", None, true);
    assert_eq!(code, 200, "{v}");
    // nine demos since finish-2 F09 (the Prusa MK3S+ joined the eight)
    assert_eq!(
        (
            v["removed"].as_array().unwrap().len(),
            v["kept_first"].clone()
        ),
        (9, json!(true))
    );
    assert_eq!(ids(&r2), ["stella"]);
}

#[test]
fn the_model_page_says_where_the_bed_corner_is_and_what_a_fetch_brings() {
    // Polish WO-05 (2026-10-18): the page's one press "use the bed corner" needs the model's own measured corner, and the files step
    // says how much a fetch brings before anything is fetched. Neither is guessed: a machine with no measured corner reports none.
    let dir = Dir::new("bed-corner");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let list = r.get("/api/printer-models");
    let row = |id: &str| {
        list["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        row("voron_trident_300")["bytes"],
        json!(11_129_383),
        "a machine in the release bundle: the size of its file in the zip"
    );
    assert_eq!(
        row("ender3_stock_220")["bytes"],
        json!(26_662_316),
        "a machine of the registry: the size the registry pins"
    );
    assert!(
        list["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["has_file"] == true)
            .all(|m| m["bytes"].is_null() || m["bytes"].as_u64().unwrap() > 1000),
        "a size is a real size or unknown, never zero"
    );
    let id = "stella";
    let patch = |body: Value| r.call("PATCH", &format!("/api/printers/{id}"), Some(&body), true);
    // A machine whose profile carries a measured calibration reports it as the corner, with where it comes from.
    assert_eq!(patch(json!({"cad_model":"voron_trident_300"})).0, 200);
    let m = r.get(&format!("/api/p/{id}/model"));
    let corner = &m["bed_corner"];
    assert_eq!(
        corner["calibration"], m["calibration"],
        "before any saved calibration the starting one is the corner: {m}"
    );
    assert!(
        corner["basis"].as_str().unwrap().contains("machine profile"),
        "{corner}"
    );
    // Pressing it saves exactly that calibration (the page sends it back), and the pose reads it.
    let saved = corner["calibration"].clone();
    assert_eq!(patch(json!({"cad_calibration": saved.clone()})).0, 200);
    wait_for("a position", || {
        r.get(&format!("/api/p/{id}/model"))["pose"]["known"] == true
    });
    let m = r.get(&format!("/api/p/{id}/model"));
    assert_eq!(m["calibrated"], true);
    assert_eq!(m["calibration"], saved);
    // The fake reports (175, 175, 10) and the corner's offsets are (-149.6, -151.6, 0): the model's nozzle is at X 25.4, Y 23.4.
    let ax = &m["pose"]["axes"];
    assert!(
        (ax["x"].as_f64().unwrap() - 25.39).abs() < 0.01
            && (ax["y"].as_f64().unwrap() - 23.36).abs() < 0.01,
        "{ax}"
    );
    // A change of options alone keeps the calibration; a new machine starts from its own (what the page relies on).
    assert_eq!(
        patch(json!({"cad_options":[{"option":"sturdy_handles_m5"}]})).0,
        200
    );
    assert_eq!(
        r.get(&format!("/api/p/{id}/model"))["calibrated"],
        true,
        "options alone do not reset the calibration"
    );
    assert_eq!(
        patch(json!({"cad_model":"voron_trident_250","cad_options":[]})).0,
        200
    );
    let m = r.get(&format!("/api/p/{id}/model"));
    assert_eq!(
        (m["calibrated"].clone(), m["options"].clone()),
        (json!(false), json!(null)),
        "another machine: its own starting calibration, and no options carried over: {m}"
    );
    // A machine with no registered motion has no measured corner to offer. (The Annex K1 stood here while its profile held only a travel range; F03, 2026-10-19,
    // measured its plate and nozzle, so it is the other case below.)
    assert_eq!(patch(json!({"cad_model":"lh_stinger_200"})).0, 200);
    let m = r.get(&format!("/api/p/{id}/model"));
    assert!(
        m["bed_corner"].is_null(),
        "no measured corner, no button: {m}"
    );
    // F03: and it says in words that the model is static (nothing moves, and no part is drawn: its plate is not measured).
    assert!(
        m["note"].as_str().is_some_and(|n| n.starts_with("This model is static: no movement is registered") && n.contains("does not follow the printer")),
        "{m}"
    );
    assert_eq!(patch(json!({"cad_model":"annex_k1_assembly"})).0, 200);
    let m = r.get(&format!("/api/p/{id}/model"));
    assert!(
        m["bed_corner"]["basis"].as_str().is_some_and(|b| b.starts_with("Measured from the model")),
        "the Annex K1's corner is measured from its model: {m}"
    );
    // No model: nothing to report.
    assert_eq!(patch(json!({"cad_model":null})).0, 200);
    assert!(r.get(&format!("/api/p/{id}/model"))["bed_corner"].is_null());
}

#[test]
fn the_model_flow_ships_with_the_page_and_add_a_printer_reads_the_licence_before_adding() {
    // Polish WO-05 (2026-10-18). The 2026-10-17 licence change left `lic` undeclared in the Add a printer flow, so pressing Add printer
    // ended in "lic is not defined" and nothing could be added from the page. The browser run in the WO-05 evidence is the real proof;
    // these markers keep the pieces from being lost again.
    let dir = Dir::new("model-flow-files");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let get = |p: &str| {
        let (code, body) = r.call("GET", p, None, false);
        assert_eq!(code, 200, "{p}");
        body.as_str().unwrap_or("").to_owned()
    };
    let (pick, models, farm) = (get("/model-pick.js"), get("/next-models.js"), get("/next-farm.js"));
    assert!(pick.contains("export function needs") && pick.contains("export const plainLabel"));
    assert!(models.contains("from './model-pick.js'"), "the picker lost its logic module");
    // The files step: two plain buttons, and nobody types the word "published".
    for button in ["From the author", "From a folder on this computer..."] {
        assert!(models.contains(button), "the files step lost {button}");
    }
    assert!(
        !models.contains("source.value='published'") && !models.contains("Type published"),
        "nobody should be asked to type the word published"
    );
    // Add a printer: the model step comes before the printer is made, and the licence is read where it is used.
    assert!(farm.contains("function modelStep()") && farm.contains("modelFlow(body"));
    assert!(
        farm.contains("const lic=await licenceState()"),
        "Add printer must read the licence state it checks"
    );
    // A choice that reads in full replaces the native select for the long tool labels.
    assert!(models.contains("function choice(") && models.contains("nx-choice"));
    let (_, css) = r.call("GET", "/next.css", None, false);
    assert!(css.as_str().unwrap_or("").contains(".nx-choice-list"));
    // What a printer cannot do is not offered (item 3): no Klipper offset, thumbnails, driver rows or Klipper wording on another brand.
    let (pages, main) = (get("/next-pages.js"), get("/next.js"));
    for marker in ["ctx.has('z_offset')", "ctx.has('thumbnails')", "const kp=ctx.has('klipper')", "if(ctx.has('klipper'))body.append(section('Objects on the plate'", "Use the ${v?'Voron':'standard'} schedule", "ctx.has('klipper')?'The graphs show the last ten minutes"] {
        assert!(pages.contains(marker), "next-pages.js lost the capability gate {marker}");
    }
    for marker in ["has('thumbnails')", "const doesOf=", "has('klipper')?'on a BLTouch alarm", "img.parentElement===c", "const MORE_NEEDS=", "has('restart')&&act('Firmware restart'"] {
        assert!(main.contains(marker), "next.js lost the capability gate {marker}");
    }
}

/// Polish WO-06 item 7 (2026-10-18, Opus supplement S2), from a fresh configuration. A service that nothing has told
/// about a printer starts with one placeholder printer so the farm has a first printer; it is not a printer. Empty
/// farm: nothing is shown. After "Try the demo farm": the eight demos and no placeholder beside them. After "Remove
/// the demo farm": empty again, and the demos are all gone (the placeholder leads, so no demo is kept as the first).
/// A real printer arriving replaces it and leads the farm.
#[test]
fn a_fresh_install_shows_no_placeholder_printer_empty_with_the_demo_farm_or_with_a_real_one() {
    let dir = Dir::new("placeholder");
    let real = FakeMoonraker::start(0);
    // Fresh: nothing named a printer, and nothing answers at the built-in address (a closed port here).
    let cfg = Config {
        unconfigured: true,
        moonraker_url: "http://127.0.0.1:9".into(),
        database: dir.file("stella.sqlite3"),
        ..Config::default()
    };
    let r = run(&dir, cfg);
    let shown = |r: &Running| -> Vec<String> {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let saved = |dir: &Dir| -> Vec<String> {
        let v: Value =
            serde_json::from_str(&fs::read_to_string(dir.file("stella-scope.json")).unwrap())
                .unwrap();
        v["printers"].as_array().map_or(Vec::new(), |a| {
            a.iter()
                .map(|e| e["id"].as_str().unwrap().to_owned())
                .collect()
        })
    };

    // First run: the farm is empty. It has a printer inside (a farm always has a first one), but it is not shown or counted.
    thread::sleep(Duration::from_millis(600));
    let all = r.get("/api/printers");
    assert_eq!(all["count"], json!(0), "{all}");
    assert_eq!(all["printers"], json!([]), "{all}");
    assert_eq!(
        r.farm.list().len(),
        1,
        "the placeholder is there, out of sight"
    );
    assert_eq!(r.get("/api/licence")["printers"], json!(0));

    // Try the demo farm: eight demos, the placeholder not among them, and the first printer is a demo that can be seen.
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    let ids = shown(&r);
    assert_eq!(ids.len(), 9, "nine demos since finish-2 F09 (the Prusa MK3S+ joined the eight): {ids:?}");
    assert!(ids.iter().all(|i| i.starts_with("demo-")), "{ids:?}");
    let all = r.get("/api/printers");
    assert_eq!(all["count"], json!(9));
    assert!(
        ids.contains(&all["default"].as_str().unwrap().to_owned()),
        "the default page is a demo: {all}"
    );
    assert!(
        all["printers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "Placeholder" && p["id"] != "placeholder"),
        "{all}"
    );
    assert_eq!(
        saved(&dir).len(),
        9,
        "the file holds the demos and not the placeholder"
    );
    assert_eq!(
        r.get("/api/licence")["printers"],
        json!(0),
        "demos and placeholder count for nothing"
    );

    // Remove the demo farm: empty again, and nothing is kept as the first printer.
    let (code, v) = r.call("DELETE", "/api/printers/demo-farm", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(
        (
            v["removed"].as_array().unwrap().len(),
            v["kept_first"].clone()
        ),
        (9, json!(false)),
        "{v}"
    );
    assert_eq!(shown(&r), Vec::<String>::new());
    assert_eq!(saved(&dir), Vec::<String>::new());

    // A real printer, called Stella (the likeliest first name, and the placeholder's old id), is taken without a clash,
    // replaces the placeholder and leads the farm; the demo farm beside it then comes and goes with nothing kept.
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    let (code, v) = r.call(
        "POST",
        "/api/printers",
        Some(&json!({"name":"Stella","address":real.url()})),
        true,
    );
    assert_eq!(code, 200, "{v}");
    assert_eq!(
        r.farm
            .list()
            .iter()
            .filter(|p| p.id == "placeholder")
            .count(),
        0,
        "retired"
    );
    let ids = shown(&r);
    assert_eq!(ids.len(), 10, "the real printer and nine demos (finish-2 F09: it was eight): {ids:?}");
    assert_eq!(r.get("/api/printers")["default"], json!("stella"));
    assert_eq!(
        saved(&dir)[0],
        "stella",
        "the real printer is first in the file, so it carries the phone link after a restart"
    );
    assert_eq!(r.get("/api/licence")["printers"], json!(1));
    let (code, v) = r.call("DELETE", "/api/printers/demo-farm", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(
        (
            v["removed"].as_array().unwrap().len(),
            v["kept_first"].clone()
        ),
        (9, json!(false)), // nine demos (finish-2 F09: it was eight)
        "{v}"
    );
    assert_eq!(shown(&r), ["stella"]);
}

/// Finish-2 F04 (2026-10-19), found by walking the screens as a person: a service that holds only the demo farm keeps its hidden
/// placeholder printer, and the Queue sheet listed it as "Stella, offline" (a place a file could be sent to), and Analytics gave it a row
/// of zeros. Everything a person is shown lists the printers they can see, nothing else.
#[test]
fn the_hidden_placeholder_is_not_listed_in_the_queue_or_the_analytics() {
    let dir = Dir::new("placeholder-lists");
    let cfg = Config {
        unconfigured: true,
        database: dir.file("stella.sqlite3"),
        ..Config::default()
    };
    let r = run(&dir, cfg);
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(r.farm.list().len(), 10, "the placeholder is there, out of sight, beside the nine demos");
    let ids = |v: &Value| -> Vec<String> {
        v.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_owned()).collect()
    };
    let queue = r.get("/api/queue");
    let listed = ids(&queue["printers"]);
    assert_eq!(listed.len(), 9, "{listed:?}");
    assert!(listed.iter().all(|i| i.starts_with("demo-")), "{listed:?}");
    let analytics = r.get("/api/analytics?days=30");
    let rows = ids(&analytics["printers"]);
    assert_eq!(rows.len(), 9, "{rows:?}");
    assert!(rows.iter().all(|i| i.starts_with("demo-")), "{rows:?}");
}

/// Polish WO-06 item 7: a configured printer is a real printer, however long it has been off. our file names
/// `moonraker_url` and a name and has no `printers` list; it must show, offline, exactly as before, and so must a printer
/// in a `printers` list that has never answered. A file that names this machine's own Moonraker address is a printer too.
#[test]
fn a_configured_printer_that_is_off_is_still_a_printer_and_an_answering_default_is_too() {
    // our shape: legacy keys, one printer, off.
    let dir = Dir::new("placeholder-ken");
    let cfg = Config {
        moonraker_url: "http://127.0.0.1:9".into(),
        printer_name: Some("Stella".into()),
        database: dir.file("stella.sqlite3"),
        ..Config::default()
    };
    let r = run(&dir, cfg);
    thread::sleep(Duration::from_millis(600));
    let all = r.get("/api/printers");
    let rows = all["printers"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{all}");
    assert_eq!(
        (
            rows[0]["id"].clone(),
            rows[0]["name"].clone(),
            rows[0]["state"].clone()
        ),
        (json!("stella"), json!("Stella"), json!("offline")),
        "{all}"
    );
    assert_eq!(r.get("/api/licence")["printers"], json!(1));

    // A `printers` list entry that never answered.
    let dir2 = Dir::new("placeholder-list");
    let cfg = Config {
        database: dir2.file("stella.sqlite3"),
        printers: vec![PrinterEntry {
            id: "bench".into(),
            name: "Bench".into(),
            address: "http://127.0.0.1:9".into(),
            ..Default::default()
        }],
        ..Config::default()
    };
    let r2 = run(&dir2, cfg);
    thread::sleep(Duration::from_millis(600));
    let rows = r2.get("/api/printers");
    assert_eq!(rows["count"], json!(1), "{rows}");
    assert_eq!(rows["printers"][0]["id"], "bench");

    // A Pi next to its printer names Moonraker's address in its file, even when it is this machine's own:
    // it is the printer (finish-2 F01: there is no built-in address any more, so it must be named).
    let dir3 = Dir::new("placeholder-answers");
    let live = FakeMoonraker::start(0);
    fs::write(
        dir3.file("pi.json"),
        json!({"moonraker_url":live.url(),"database":dir3.file("stella.sqlite3")}).to_string(),
    )
    .unwrap();
    let cfg = Config::load(&dir3.file("pi.json")).unwrap();
    assert!(!cfg.unconfigured);
    let r3 = run(&dir3, cfg);
    wait_for("the default printer to answer", || {
        r3.get("/api/printers")["printers"]
            .as_array()
            .is_some_and(|p| p.len() == 1 && p[0]["online"] == true)
    });
    assert_eq!(r3.get("/api/printers")["count"], json!(1));
}

/// Polish WO-06 item 7: what counts as "nothing named a printer", from the text of a configuration file.
#[test]
fn a_configuration_names_a_printer_by_an_address_or_a_real_printers_list() {
    use crate::config::Config;
    for (text, unconfigured) in [
        ("{}", true),
        (r#"{"port":7420,"database":"/tmp/x.sqlite3"}"#, true),
        (r#"{"printers":[]}"#, true),
        (
            r#"{"printers":[{"id":"demo-a","driver":"demo"},{"id":"demo-b","driver":"demo"}]}"#,
            true,
        ),
        (r#"{"moonraker_url":"http://192.168.1.131:7125"}"#, false),
        (
            r#"{"moonraker_url":"http://127.0.0.1:7125","printer_name":"Stella"}"#,
            false,
        ),
        (
            r#"{"printers":[{"id":"bench","address":"http://bench:7125"}]}"#,
            false,
        ),
        (
            r#"{"printers":[{"id":"demo-a","driver":"demo"},{"id":"bench","address":"http://bench:7125"}]}"#,
            false,
        ),
    ] {
        assert_eq!(Config::names_no_printer(text), unconfigured, "{text}");
    }
    // and through the loader, with our own old file
    let dir = Dir::new("names-a-printer");
    let ken = r#"{"moonraker_url":"http://192.168.1.131:7125","bind":"127.0.0.1","port":7420,"controls_enabled":true,"database":"/tmp/ken.sqlite3","printer_name":"Stella"}"#;
    fs::write(dir.file("ken.json"), ken).unwrap();
    fs::write(
        dir.file("fresh.json"),
        r#"{"port":7430,"database":"/tmp/fresh.sqlite3"}"#,
    )
    .unwrap();
    assert!(!Config::load(&dir.file("ken.json")).unwrap().unconfigured);
    let fresh = Config::load(&dir.file("fresh.json")).unwrap();
    assert!(fresh.unconfigured);
    let entries = fresh.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].id, "placeholder",
        "not the id a first printer called Stella would take"
    );
    assert_eq!(
        Config::load(&dir.file("ken.json")).unwrap().entries()[0].id,
        "stella"
    );
}

/// Polish WO-06 item 8 (2026-10-18, Opus supplement S3, README rule 8): no Klipper words on a printer that is not
/// Klipper. A stopped demo tile used to read "Klipper state: shutdown". Every state a demo, a PrusaLink and a FlashForge
/// printer can be in is rendered through the real service and every word the service says about it is searched for
/// the names of Klipper's engine. The stopped demo says "Stopped (emergency stop)".
#[test]
fn no_klipper_word_on_a_demo_a_prusalink_or_a_flashforge_printer_in_any_state() {
    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o
                .iter()
                // the address and the driver are what the person typed or chose, not words the service says
                .filter(|(k, _)| {
                    ![
                        "address",
                        "driver",
                        "thumbnail_url",
                        "camera_url",
                        "file",
                        "id",
                    ]
                    .contains(&k.as_str())
                })
                .for_each(|(_, x)| strings(x, out)),
            _ => {}
        }
    }
    let engine = |text: &str| {
        let t = text.to_ascii_lowercase();
        [
            "klipper",
            "moonraker",
            "klippy",
            "firmware_restart",
            "firmware restart",
            "gcode/help",
            "mcu",
        ]
        .iter()
        .any(|w| t.contains(w))
    };
    let dir = Dir::new("plain-words");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let ff = FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
    let demo = |id: &str, start: f64, paused: bool| PrinterEntry {
        id: id.into(),
        name: id.into(),
        driver: "demo".into(),
        model: Some("Voron 2.4 350".into()),
        demo_start_at: Some(start),
        demo_paused: paused,
        ..PrinterEntry::default()
    };
    let cfg = Config {
        printers: vec![
            demo("d-idle", 0.0, false), // a demo that has just begun is heating up
            demo("d-print", 0.5, false),
            demo("d-pause", 0.5, true),
            demo("d-done", 1.0, false),
            demo("d-cancel", 0.4, false),
            demo("d-stop", 0.4, false),
        ],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    for (name, body) in [
        (
            "Core One L",
            json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}}),
        ),
        (
            "Adventurer 5M",
            json!({"name":"Adventurer 5M","driver":"flashforge","address":ff.http_url(),"credentials":{"serial":"SNFAKE5M00001","check_code":"0fake000","tcp_port":ff.tcp_port().to_string()}}),
        ),
    ] {
        let (code, v) = r.call("POST", "/api/printers", Some(&body), true);
        assert_eq!(code, 200, "{name}: {v}");
    }
    let row = |id: &str| -> Value {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    };
    let said = |id: &str| -> Vec<String> {
        let mut words = Vec::new();
        strings(&row(id), &mut words);
        strings(&r.get(&format!("/api/p/{id}/tick"))["observer"], &mut words);
        words
    };
    let mut table: Vec<String> = Vec::new();
    let mut check = |brand: &str, state: &str, id: &str, want: &str| {
        wait_for(&format!("{brand} {state} to read {want}"), || {
            row(id)["state"] == json!(want)
        });
        let words = said(id);
        table.push(format!(
            "{brand:<10} {state:<10} farm state {want:<9} says {:?}",
            row(id)["detail"]
        ));
        for w in &words {
            assert!(!engine(w), "{brand} in {state} says {w:?} (all: {words:?})");
        }
    };

    // The demo printer, every state it has. The stop is the emergency stop, the only way a demo stops.
    check("demo", "heating", "d-idle", "heating");
    check("demo", "printing", "d-print", "printing");
    check("demo", "paused", "d-pause", "paused");
    check("demo", "finished", "d-done", "finished");
    let (code, v) = r.printer_call(
        "POST",
        "/api/p/d-cancel/commands",
        "d-cancel",
        &json!({"id":"cancel","confirm":true}),
    );
    assert_eq!(code, 200, "{v}");
    check("demo", "cancelled", "d-cancel", "finished");
    let (code, v) = r.printer_call(
        "POST",
        "/api/p/d-stop/commands",
        "d-stop",
        &json!({"id":"emergency_stop","confirm":true}),
    );
    assert_eq!(code, 200, "{v}");
    check("demo", "stopped", "d-stop", "error");
    assert_eq!(row("d-stop")["detail"], json!("Stopped (emergency stop)"));
    let tick = r.get("/api/p/d-stop/tick");
    assert_eq!(
        tick["observer"]["printer_message"],
        json!("Stopped (emergency stop)"),
        "{tick}"
    );

    // PrusaLink: the words its own firmware says.
    for (word, want) in [
        ("IDLE", "idle"),
        ("PRINTING", "printing"),
        ("PAUSED", "paused"),
        ("ATTENTION", "paused"),
        ("FINISHED", "finished"),
        ("STOPPED", "finished"),
        ("ERROR", "error"),
    ] {
        {
            let mut s = prusa.state.lock().unwrap();
            s.state = word.into();
            s.job_id = (word == "PRINTING").then_some(5);
            s.file = "bracket_PETG.gcode".into();
        }
        check("PrusaLink", word, "core-one-l", want);
    }

    // FlashForge: both doors' words.
    for (word, want) in [
        ("ready", "idle"),
        ("printing", "printing"),
        ("paused", "paused"),
        ("completed", "finished"),
        ("cancel", "finished"),
        ("error", "error"),
    ] {
        {
            let mut s = ff.state.lock().unwrap();
            s.status = word.into();
            s.machine_status = word.to_ascii_uppercase();
            s.file = "benchy.gcode".into();
        }
        check("FlashForge", word, "adventurer-5m", want);
    }
    eprintln!("{}", table.join("\n"));
}

/// Polish WO-06 item 1: Prusa's notice is served to be read before the CORE One L model is fetched; nothing is fetched without
/// the person's confirmation, and nothing in this test reaches the network. (Finish-2 F09, 2026-10-19: the notice is still served, now with the OCL
/// text and the pins, but nothing is fetched behind it: the model ships inside the build, so the wanted list has no `notice` row for it and the
/// library of a service that has not started yet says it is on its way. The test keeps its name; the confirmation rule is unchanged.)
#[test]
fn the_core_one_l_notice_is_read_first_and_a_fetch_needs_the_confirmation() {
    let dir = Dir::new("core-notice");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    let n = r.get("/api/printer-models/prusa_core_one_l/notice");
    assert_eq!(n["id"], "prusa_core_one_l", "{n}");
    let text = n["text"].as_str().unwrap();
    assert!(
        text.contains("Prusa Research")
            && text.contains("Open Community License")
            && text.contains("5c0e2bd142d015e9"),
        "{n}"
    );
    assert_eq!(n["licence"]["class"], "ocl");
    let (code, v) = r.call(
        "GET",
        "/api/printer-models/tictac_21_120/notice",
        None,
        false,
    );
    assert_eq!(code, 404, "{v}");
    // a farm with a CORE One L: it is not fetched on one press (it is inside the build; a service that has not seeded its library says so)
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    let w = r.get("/api/printer-models/wanted");
    let core = w["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "prusa_core_one_l")
        .unwrap()
        .clone();
    assert!(core["notice"].is_null(), "there is no notice-first fetch any more: {core}");
    assert_eq!(core["open"], json!(false), "{core}");
    assert!(core["why"].as_str().unwrap().contains("ships inside Stella Scope"), "{core}");
    // no confirmation, no fetch (the request is refused before anything is asked of the network)
    let (code, v) = r.call(
        "POST",
        "/api/printer-models/fetch-open",
        Some(&json!({"ids":["prusa_core_one_l"]})),
        true,
    );
    assert_eq!(code, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("confirm"), "{v}");
}

/// Polish WO-06 item 3 (2026-10-18): the console of a page that lists a FlashForge's files was full of 502s, one per picture the
/// printer cannot have. The picture route answers such a printer with nothing (204), a printer that has pictures with its picture or
/// with its reason, and a demo printer's picture is a real PNG.
#[test]
fn a_printer_with_no_pictures_answers_the_picture_route_with_nothing_and_a_demo_with_its_picture() {
    let dir = Dir::new("pictures");
    let a = FakeMoonraker::start(0);
    let ff = FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
    let cfg = Config {
        printers: vec![PrinterEntry {
            id: "demo-one".into(),
            name: "Demo".into(),
            driver: "demo".into(),
            model: Some("Voron 2.4 350".into()),
            demo_start_at: Some(0.4),
            ..PrinterEntry::default()
        }],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    let add = json!({"name":"Adventurer 5M","driver":"flashforge","address":ff.http_url(),"credentials":{"serial":"SNFAKE5M00001","check_code":"0fake000","tcp_port":ff.tcp_port().to_string()}});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    let status_of = |path: &str| -> (u16, usize, String) {
        let mut resp = r.agent.get(&format!("{}{path}", r.base)).call().unwrap();
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        (
            resp.status().as_u16(),
            resp.body_mut().read_to_vec().unwrap().len(),
            ct,
        )
    };
    let (code, len, _) =
        status_of("/api/p/adventurer-5m/print/thumbnail?filename=Benchy_PLA.gcode.3mf");
    assert_eq!(
        (code, len),
        (204, 0),
        "a printer with no pictures says nothing, not 502"
    );
    // the demo printer's picture is a PNG (its sample prints carry thumbnails)
    wait_for("the demo to have a file", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "demo-one" && p["file"].as_str().is_some_and(|f| !f.is_empty()))
    });
    let file = r.get("/api/printers")["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "demo-one")
        .unwrap()["file"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, len, ct) = status_of(&format!(
        "/api/p/demo-one/print/thumbnail?filename={}",
        file.replace(' ', "%20")
    ));
    assert!(
        code == 200 && len > 100 && ct.starts_with("image/png"),
        "{code} {len} {ct}"
    );
    // a printer that has pictures but not for this file says so: not found, not a failing printer (live finding of WO-07)
    let (code, _, _) = status_of("/api/p/demo-one/print/thumbnail?filename=nothing-here.gcode");
    assert_eq!(code, 404);
}

/// Not a test: serves a fresh install (nothing names a printer, nothing answers at the built-in address) to a browser, for the
/// screenshots of polish WO-06 item 7. `STELLA_TEST_BIND=127.0.0.1:7423 cargo test --release serve_a_fresh_install -- --ignored
/// --nocapture`; it runs until it is killed. The built-in address is a closed port here, so nothing on this computer is asked.
#[test]
#[ignore]
fn serve_a_fresh_install_for_a_browser() {
    let dir = Dir::new("fresh-serve");
    let cfg = Config {
        unconfigured: true,
        moonraker_url: "http://127.0.0.1:9".into(),
        database: dir.file("stella.sqlite3"),
        ..Config::default()
    };
    let r = run(&dir, cfg);
    eprintln!("FRESH INSTALL SERVED ON {}", r.base);
    loop {
        thread::sleep(Duration::from_secs(5));
    }
}

/// Not a test: serves one printer of every brand the service drives, each in the middle of a print and each with a camera, from this crate's
/// own fakes, to a browser for the screenshots of finish-2 F04 (2026-10-19). The pages of a Klipper printer, a PrusaLink printer and a FlashForge
/// printer differ in what the printer can do, so each is looked at. `STELLA_TEST_BIND=127.0.0.1:7425 cargo test --release serve_every_brand --
/// --ignored --nocapture`; it runs until it is killed. Everything is on this computer: nothing real is asked, no printer is touched.
#[test]
#[ignore]
fn serve_every_brand_for_a_browser() {
    let dir = Dir::new("brands-serve");
    let klipper = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let ff = FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
    let cam = crate::fakes::FakeMjpeg::start(0, 10);
    let cfg = Config {
        unconfigured: true,
        database: dir.file("stella.sqlite3"),
        ..Config::default()
    };
    let r = run(&dir, cfg);
    for (name, v) in [
        ("Voron 2.4 (Klipper)", json!({"name":"Voron 2.4 (Klipper)","address":klipper.url(),"accent":"#04D9C4","camera":cam.stream_url()})),
        ("Core One L (PrusaLink)", json!({"name":"Core One L (PrusaLink)","driver":"prusalink","address":prusa.url(),"accent":"#FF6A00","cad_model":"prusa_core_one_l","camera":cam.stream_url(),"credentials":{"user":"maker","password":"secret"}})),
        ("Adventurer 5M (FlashForge)", json!({"name":"Adventurer 5M (FlashForge)","driver":"flashforge","address":ff.http_url(),"accent":"#FF2D92","credentials":{"serial":"SNFAKE5M00001","check_code":"0fake000","tcp_port":ff.tcp_port().to_string()}})),
    ] {
        let (code, body) = r.call("POST", "/api/printers", Some(&v), true);
        assert_eq!(code, 200, "{name}: {body}");
    }
    {
        let mut s = klipper.state.lock().unwrap();
        s.print_state = "printing".into();
        s.filename = "bracket_PETG.gcode".into();
        s.progress = 0.41;
        s.nozzle_target = 240.0;
        s.nozzle = 239.0;
        s.bed_target = 80.0;
        s.bed = 79.5;
    }
    {
        let mut s = prusa.state.lock().unwrap();
        s.state = "PRINTING".into();
        s.job_id = Some(5);
        s.file = "twist bowl for the CORE One L.gcode".into();
        s.progress = 41.0;
        s.printing_s = 1500.0;
        s.nozzle = 249.0;
        s.nozzle_target = 250.0;
        s.bed = 84.0;
        s.bed_target = 85.0;
    }
    {
        let mut s = ff.state.lock().unwrap();
        s.status = "printing".into();
        s.machine_status = "BUILDING_FROM_SD".into();
        s.file = "Benchy_PLA.gcode.3mf".into();
        s.progress = 0.32;
        s.layer = 62;
        s.layers = 240;
        s.nozzle = 219.5;
        s.nozzle_target = 220.0;
        s.bed = 59.8;
        s.bed_target = 60.0;
        s.camera_url = cam.stream_url();
    }
    // STELLA_STALL_MS=8000: the Klipper and the PrusaLink printer answer each request that late (a printer that is on but slow, or half off the network), for the first-paint
    // numbers of F04: the wall must still show the other printers at once.
    if let Some(ms) = std::env::var("STELLA_STALL_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        klipper.state.lock().unwrap().stall_ms = ms;
        prusa.state.lock().unwrap().stall_ms = ms;
    }
    eprintln!("EVERY BRAND SERVED ON {}", r.base);
    loop {
        thread::sleep(Duration::from_secs(5));
    }
}

/// UI board U06 (2026-10-20): only the fake Klipper, on STELLA_FAKE_PORT, for a scratch instance whose config names it as a printer (so a printer with a 3D scene
/// and real Klipper capabilities can be looked at). Run with `cargo test --release serve_a_fake_klipper_for_a_browser -- --ignored --nocapture`; it runs until killed.
/// Idle and homed, a little warm, a console that has some history in it. Everything is on this computer; nothing real is asked.
#[test]
#[ignore]
fn serve_a_fake_klipper_for_a_browser() {
    let port = std::env::var("STELLA_FAKE_PORT").ok().and_then(|v| v.parse::<u16>().ok()).unwrap_or(7457);
    let klipper = FakeMoonraker::start(port);
    {
        let mut s = klipper.state.lock().unwrap();
        s.nozzle = 41.0;
        s.bed = 33.0;
        s.mesh = true;
        s.say("command", "G28");
        s.say("response", "// Homing...");
        s.say("command", "QUAD_GANTRY_LEVEL");
        s.say("response", "// Gantry-relative probe points:\n// stepper_z = -0.012\n// stepper_z1 = 0.034\n// stepper_z2 = -0.021\n// stepper_z3 = 0.008");
        s.say("response", "// Retries: 0/5 Probed points range: 0.004000 tolerance: 0.007500");
    }
    eprintln!("A FAKE KLIPPER ON {}", klipper.url());
    loop {
        thread::sleep(Duration::from_secs(5));
    }
}

/// Polish WO-06 (live findings 1 and 2 of the WO-07 pass, 2026-10-19). A PrusaLink in ATTENTION shows as paused; the printer's own
/// words ("check its screen") now reach the wall's detail and the tick the page reads. A resume the printer refuses (its 409) comes back
/// as "refused", a known not-done, not as "unknown" (a request that may or may not have arrived).
#[test]
fn a_prusalink_in_attention_says_why_and_a_refused_resume_is_a_known_not_done() {
    let dir = Dir::new("attention");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    let (code, v) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{v}");
    let row = || -> Value {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "core-one-l")
            .cloned()
            .unwrap_or(Value::Null)
    };
    let set = |state: &str| {
        let mut s = prusa.state.lock().unwrap();
        s.state = state.into();
        s.job_id = (state != "IDLE").then_some(5);
        s.file = "bracket_PETG.gcode".into();
    };
    set("ATTENTION");
    wait_for("the attention state to be seen", || row()["state"] == "paused");
    wait_for("its message to reach the wall", || row()["detail"].as_str().is_some_and(|d| d.contains("asking a question on its screen")));
    assert_eq!(row()["detail"], "The printer is asking a question on its screen. Answer it there to go on.");
    let tick = r.get("/api/p/core-one-l/tick");
    assert_eq!(tick["job"]["message"], "The printer is asking a question on its screen. Answer it there to go on.", "{tick}");
    // an ordinary pause says nothing more than it is paused
    set("PAUSED");
    wait_for("the plain pause", || row()["state"] == "paused" && row()["detail"].is_null());
    // a resume the printer refuses: it is printing, so it cannot be resumed (409)
    set("PRINTING");
    wait_for("printing", || row()["state"] == "printing");
    let (code, v) = r.printer_call("POST", "/api/p/core-one-l/commands", "core-one-l", &json!({"id":"resume","confirm":true}));
    assert_eq!((code, v["status"].clone()), (409, json!("refused")), "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("cannot resume")), "{v}");
    // the file it is printing is not on the storage the farm asks (the fake lists none): that is "not found", never a 502 in the console
    let url = format!(
        "{}/api/p/core-one-l/print/thumbnail?filename=bracket_PETG.gcode",
        r.base
    );
    let mut resp = r.agent.get(&url).call().unwrap();
    assert_eq!(
        resp.status().as_u16(),
        404,
        "{:?}",
        resp.body_mut().read_to_string()
    );
}

/// Finish-2 F01 (2026-10-19): a service that names no printer reaches nothing. The built-in address was
/// `http://127.0.0.1:7125`, which on our PC is the Kobra X bridge; a fresh scratch instance read it twice.
/// Now there is no built-in address, the placeholder has no collector, and every un-prefixed page request
/// (status, files, camera) has nothing to open. A printer removed from the farm stops being asked at its
/// next pass: a bare listener stands in for it and its connection count stops growing.
#[test]
fn a_service_with_no_printer_opens_nothing_and_a_removed_printer_stops_being_asked() {
    assert_eq!(Config::default().moonraker_url, "", "no built-in printer address");
    // Thread names are process-wide, so only a name no other test uses is counted ("collector-f01-g").
    let collectors = || -> Vec<String> {
        fs::read_dir("/proc/self/task")
            .unwrap()
            .filter_map(|t| fs::read_to_string(t.ok()?.path().join("comm")).ok())
            .map(|c| c.trim().to_owned())
            .filter(|c| c.starts_with("collector-"))
            .collect()
    };
    let dir = Dir::new("no-printer-no-poll");
    let cfg = Config {
        unconfigured: true,
        database: dir.file("stella.sqlite3"),
        controls_enabled: true,
        ..Config::default()
    };
    let r = run(&dir, cfg);
    for path in ["/api/status", "/api/printers", "/api/files", "/api/camera", "/api/tick"] {
        let _ = r.get(path);
    }
    // A collector would have tried, failed and said so within its first pass; the placeholder has none, so
    // its state stays exactly as it was made.
    thread::sleep(Duration::from_millis(2500));
    let placeholder = r.farm.list()[0].clone();
    assert_eq!(placeholder.id, "placeholder");
    {
        let s = placeholder.state.lock().unwrap();
        assert!(
            s.last_error.is_none() && s.last_tick_ts.is_none() && !s.connected,
            "the placeholder was polled: {:?}",
            s.last_error
        );
    }
    assert_eq!(r.get("/api/printers")["count"], json!(0));
    // The home page of a fresh install is the farm (demo card, Add a printer), not the placeholder's page.
    let (code, home) = r.call("GET", "/", None, false);
    assert_eq!(code, 200);
    assert!(home.as_str().unwrap_or("").contains("fm-grid"), "/ is not the farm on a fresh install");
    let (camera, said) = r.call("GET", "/api/camera", None, false);
    assert_eq!((camera, said["error"].as_str()), (404, Some(crate::config::NO_PRINTER)));

    // A printer that is added is asked; once removed, it is not asked again.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = accepts.clone();
    thread::spawn(move || {
        for s in listener.incoming() {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(s); // closed at once: the printer is "off"
        }
    });
    let lead = FakeMoonraker::start(0);
    for (name, url) in [
        ("Lead", lead.url()),
        ("F01 gone", format!("http://127.0.0.1:{port}")),
    ] {
        let add = json!({"name":name,"address":url});
        let (code, v) = r.call("POST", "/api/printers", Some(&add), true);
        assert_eq!(code, 200, "{v}");
    }
    wait_for("the printer to be asked", || accepts.load(Ordering::SeqCst) >= 2);
    let (code, v) = r.call("DELETE", "/api/printers/f01-gone", None, true);
    assert_eq!(code, 200, "{v}");
    wait_for("its collector to end", || {
        !collectors().iter().any(|c| c == "collector-f01-g")
    });
    let after = accepts.load(Ordering::SeqCst);
    thread::sleep(Duration::from_millis(4500));
    assert_eq!(accepts.load(Ordering::SeqCst), after, "a removed printer is still being asked");
}

/// Finish-2 F02 (CORE One L, 2026-10-19): the review before a print starts said Klipper's own call ("POST /printer/print/start?
/// filename=... (no G-code)") on a PrusaLink printer. It now says what will happen, in words, on every brand.
#[test]
fn the_review_before_a_print_says_what_will_happen_not_a_klipper_call() {
    let dir = Dir::new("review-words");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    for (path, id) in [("/api/p/core-one-l/command-preview", "core-one-l"), ("/api/p/stella/command-preview", "stella")] {
        let (code, v) = r.printer_call("POST", path, id, &json!({"id":"print_start","filename":"twistbowl (dry run).gcode"}));
        assert_eq!(code, 200, "{v}");
        assert_eq!(v["action"], "Start \"twistbowl (dry run).gcode\" on the printer, once.", "{id}: {v}");
        assert_eq!(v["tier"], "T3", "a print start is still reviewed");
    }
}

/// Finish-2 F02: Pause, Resume and Cancel on a PrusaLink printer are reviewed in words, not as Klipper's macros; a Klipper
/// printer still shows the exact G-code.
#[test]
fn pause_resume_and_cancel_are_reviewed_in_words_on_a_printer_that_is_not_klipper() {
    let dir = Dir::new("review-words-2");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    for (id, words) in [("pause", "Pause the print."), ("resume", "Resume the print."), ("cancel", "Cancel the print. It cannot be resumed.")] {
        let (code, v) = r.printer_call("POST", "/api/p/core-one-l/command-preview", "core-one-l", &json!({"id":id}));
        assert_eq!(code, 200, "{v}");
        assert_eq!((v["action"].as_str(), v["script"].is_null()), (Some(words), true), "{id}: {v}");
        let (_, k) = r.printer_call("POST", "/api/p/stella/command-preview", "stella", &json!({"id":id}));
        assert!(k["script"].as_str().is_some_and(|s| !s.is_empty()), "Klipper keeps its G-code: {k}");
    }
}

/// Finish-2 F02 (CORE One L, 2026-10-19): a file with no picture (sliced on PrusaSlicer's command line) is "no picture", 404,
/// not a failing printer (502).
#[test]
fn a_prusalink_file_without_a_picture_is_not_found_not_a_failure() {
    let dir = Dir::new("nopic");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    let (code, v) = r.call("GET", "/api/p/core-one-l/print/thumbnail?filename=nopic.gcode", None, false);
    assert_eq!(code, 404, "{v}");
    let (code, _) = r.call("GET", "/api/p/core-one-l/print/thumbnail?filename=camera_cap.gcode", None, false);
    assert_eq!(code, 200);
}

/// Finish-2 F02 (CORE One L, 2026-10-19): the time left is the printer's own when it states one (worked out from progress it
/// said 1 h 27 min while the CORE One L said 14 min), and a PrusaLink's print history is the recorder's, never a question to
/// Moonraker ("cannot reach Moonraker ... set STELLA_MOONRAKER_API_KEY").
#[test]
fn a_prusalink_gives_its_own_time_left_and_its_history_never_asks_moonraker() {
    let dir = Dir::new("prusa-time-left");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    {
        let mut s = prusa.state.lock().unwrap();
        s.state = "PRINTING".into();
        s.job_id = Some(7);
        s.file = "stellavase.gcode".into();
        s.progress = 50.0;
        s.printing_s = 60.0;
    }
    let row = || {
        r.get("/api/printers")["printers"]
            .as_array()
            .and_then(|p| p.iter().find(|x| x["id"] == "core-one-l").cloned())
            .unwrap_or(Value::Null)
    };
    // the fake states (100 - progress) * 6 s, about 300 s at 50 %, where progress alone would say about 60 s
    let own = |v: &Value| v.as_f64().is_some_and(|s| (250.0..=300.0).contains(&s));
    wait_for("the printer's own time left", || own(&row()["time_left_s"]));
    assert!(own(&r.get("/api/p/core-one-l/tick")["job"]["time_left"]));
    let h = r.get("/api/p/core-one-l/print/history");
    assert_eq!(h["source"], "prints this recorder saw", "{h}");
    assert!(h.get("printer_error").is_none() && !h.to_string().contains("Moonraker"), "{h}");
}

/// Finish-2 F02: the model page at a recorded moment takes the recorder's position, not the live one.
#[test]
fn the_model_at_a_recorded_moment_has_the_position_recorded_then() {
    let dir = Dir::new("model-at");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    wait_for("a recording", || r.get("/api/tick")["point"]["position"][2].as_f64().is_some());
    // not the session's first sample: it carries the whole printer state and the readers skip it (as the workspace does)
    thread::sleep(Duration::from_millis(1500));
    let then = r.get("/api/tick")["point"]["ts"].as_f64().unwrap();
    let z_then = r.get("/api/tick")["point"]["position"][2].as_f64().unwrap();
    thread::sleep(Duration::from_millis(1200));
    a.state.lock().unwrap().pos[2] = z_then + 40.0;
    wait_for("the new Z", || r.get("/api/tick")["point"]["position"][2].as_f64() == Some(z_then + 40.0));
    let live = r.get("/api/model");
    // the tick is published as its sample is written: give the recording a moment to hold it
    wait_for("the moment in the recording", || r.get(&format!("/api/model?ts={then}"))["recorded"] == true);
    let past = r.get(&format!("/api/model?ts={then}"));
    assert_eq!(live["recorded"], false);
    // with no model bound the route still answers; the position is what matters here when a model is bound
    if past["model"].is_string() {
        assert_eq!(past["position"]["z"].as_f64(), Some(z_then));
        assert_eq!(live["position"]["z"].as_f64(), Some(z_then + 40.0));
    }
}

/// Finish-2 F05 (2026-10-19): in a farm of added printers an alert reaches ntfy. The settings are written beside the farm's
/// recording; the sender used to read beside the first printer's own recording, so nothing was ever sent (the test said
/// "sent: []" on a scratch farm of the CORE One L, Stella and the AD5M). A bare listener stands in for ntfy.
#[test]
fn an_alert_reaches_ntfy_in_a_farm_of_added_printers() {
    use std::io::Read as _;
    let dir = Dir::new("ntfy-farm");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let cfg = Config {
        printers: vec![
            PrinterEntry { id: "lead".into(), name: "Lead".into(), address: a.url(), ..Default::default() },
            PrinterEntry { id: "bench".into(), name: "Bench".into(), address: b.url(), ..Default::default() },
        ],
        moonraker_url: String::new(),
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    r.farm.start_extras(false, false);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(mut s) = s else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let _ = std::io::Write::write_all(&mut s, b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        }
    });
    // The alert settings live in one place per process; tests running beside this one point it at their own files, so in a full run the
    // test message can read another test's settings (no ntfy) and say queued:false. One running service has one settings file, so retry.
    let mut v = Value::Null;
    for _ in 0..10 {
        let (code, s) = r.call("POST", "/api/notify/ntfy", Some(&json!({"url":format!("http://127.0.0.1:{port}/farm-test"),"enabled":true})), true);
        assert_eq!(code, 200, "{s}");
        let (code, t) = r.call("POST", "/api/notify/test", Some(&json!({})), true);
        assert_eq!(code, 200, "{t}");
        v = t;
        if v["queued"] == json!(true) {
            break;
        }
        thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(v["queued"], json!(true), "{v}");
    let got = rx.recv_timeout(Duration::from_secs(10)).expect("ntfy got nothing");
    assert!(got.starts_with("POST /farm-test") && got.contains("test message from Stella Scope"), "{got}");
}

/// Finish-2 F05: the failure watch learns that a print runs from this service, for every brand. It asked a Moonraker for
/// `print_stats`, so on a PrusaLink (the CORE One L) it never saw a print and never watched.
#[test]
fn the_failure_watch_sees_a_prusalink_print_through_this_service() {
    let dir = Dir::new("eye-state");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    assert_eq!(r.call("POST", "/api/printers", Some(&add), true).0, 200);
    {
        let mut s = prusa.state.lock().unwrap();
        s.state = "PRINTING".into();
        s.job_id = Some(9);
        s.file = "stellavase.gcode".into();
        s.progress = 30.0;
    }
    let q = "/api/p/core-one-l/printer/objects/query?print_stats=state,filename,total_duration";
    wait_for("the print to be seen", || r.get(q)["result"]["status"]["print_stats"]["state"] == "printing");
    let ps = &r.get(q)["result"]["status"]["print_stats"];
    assert_eq!(ps["filename"], "stellavase.gcode", "{ps}");
}

/// Finish-2 F05 (Opus supplement 1, item 2): Klipper says X 0 Y 0 Z 0 before homing. The model and the wall take an axis
/// Klipper has not homed as unknown, never as 0; once homed, the numbers are real.
#[test]
fn an_unhomed_klipper_axis_is_unknown_not_zero() {
    let dir = Dir::new("unhomed");
    let a = FakeMoonraker::start(0);
    a.state.lock().unwrap().homed = String::new();
    a.state.lock().unwrap().pos = [0.0, 0.0, 0.0];
    let cfg = Config {
        printers: vec![PrinterEntry {
            id: "stella".into(),
            name: "Stella".into(),
            address: a.url(),
            cad_model: Some("voron_v24_350_printed".into()),
            database: Some(dir.file("stella.sqlite3")),
            ..Default::default()
        }],
        ..base_for(&dir, &a)
    };
    let r = run(&dir, cfg);
    wait_for("a tick", || r.get("/api/tick")["point"]["position"].is_array());
    let m = r.get("/api/p/stella/model?scene=0");
    assert_eq!(m["position"], json!({"x":null,"y":null,"z":null}), "{m}");
    a.state.lock().unwrap().homed = "xyz".into();
    a.state.lock().unwrap().pos = [175.0, 175.0, 10.0];
    wait_for("the homed position", || r.get("/api/p/stella/model?scene=0")["position"]["x"] == json!(175.0));
}

/// Finish-2 F09 (2026-10-19): a PrusaLink printer that names its machine (the fake's hostname is prusa-core-one-l) is added with that machine's own defaults
/// for the queue, taken from the one table of every Prusa machine: what its sliced files call it, its nozzle, its plate and its short name. The 3D
/// model is the person's choice and is never set for them; a person who typed their own values keeps them; the table (and the machines that have no
/// model, with why) is served to the page, and Printer models lists the four models the build carries.
#[test]
fn a_prusalink_printer_that_names_its_machine_starts_with_that_machines_defaults() {
    let dir = Dir::new("prusa-defaults");
    let a = FakeMoonraker::start(0);
    let (p1, p2) = (FakePrusaLink::start(0, "maker", "secret"), FakePrusaLink::start(0, "maker", "secret"));
    let r = run(&dir, base_for(&dir, &a));
    let login = json!({"user":"maker","password":"secret"});
    // nothing given but a name and the login
    let add = json!({"name":"Bench","driver":"prusalink","address":p1.url(),"credentials":login});
    let (code, body) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{body}");
    // a person who typed their own keeps them
    let add2 = json!({"name":"Mine","driver":"prusalink","address":p2.url(),"credentials":login,"slicer_model":"MYCORE","nozzle_mm":0.6,"bed_size_mm":[250.0,220.0]});
    let (code, body) = r.call("POST", "/api/printers", Some(&add2), true);
    assert_eq!(code, 200, "{body}");
    let get = |id: &str| r.get("/api/printers")["printers"].as_array().unwrap().iter().find(|x| x["id"] == id).cloned().unwrap();
    let bench = get("bench");
    assert_eq!((bench["model"].clone(), bench["slicer_model"].clone(), bench["nozzle_mm"].clone(), bench["bed_mm"].clone()), (json!("CORE One L"), json!("COREONEL"), json!(0.4), json!([300.0, 300.0])), "{bench}");
    assert!(bench["cad_model"].is_null(), "the 3D model is the person's choice, not set for them: {bench}");
    let mine = get("mine");
    assert_eq!((mine["slicer_model"].clone(), mine["nozzle_mm"].clone(), mine["bed_mm"].clone()), (json!("MYCORE"), json!(0.6), json!([250.0, 220.0])), "{mine}");
    // the table the page reads
    let t = r.get("/api/printer-models/prusa");
    let rows = t["machines"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|m| m["id"] == id).unwrap().clone();
    assert_eq!((find("prusa_mini")["bed_mm"].clone(), find("prusa_mini")["cad_model"].clone()), (json!([180.0, 180.0]), json!("prusa_mini")));
    assert!(find("prusa_xl")["cad_model"].is_null() && find("prusa_xl")["no_model"].as_str().unwrap().contains("no assembly"));
    assert!(find("prusa_mk4s")["no_model"].as_str().unwrap().contains("heatbed"), "the MK4's reason is the exact one");
    // Printer models: the four the build carries, and the Prusa machines with no model, each with its reason
    let list = r.get("/api/printer-models");
    let carried: Vec<&str> = list["models"].as_array().unwrap().iter().filter(|m| m["in_build"] == true).map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(carried.len(), 4, "{carried:?}");
    for id in ["prusa_core_one_l", "prusa_core_one", "prusa_mk3s", "prusa_mini"] {
        assert!(carried.contains(&id), "{id}");
    }
    let none: Vec<&str> = list["no_model"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(none.contains(&"prusa_xl") && none.contains(&"prusa_mk4s") && !none.contains(&"prusa_mini"), "{none:?}");
    assert!(list["no_model"].as_array().unwrap().iter().all(|m| m["why"].as_str().unwrap().len() > 40));
    // About: who is credited, and each model's notice ends with its licence text
    let c = r.get("/api/printer-models/credits");
    assert_eq!(c["models"].as_array().unwrap().len(), 4);
    assert_eq!((c["licence"].clone(), c["maker"].clone()), (json!("AGPL-3.0"), json!("3D3D Canada")));
    // finish-2 F07: who to write to, and the 3D Print Rig credit with its MIT notice
    assert_eq!(c["contact"], json!("info@3d3d.ca"));
    assert_eq!((c["rig"]["name"].clone(), c["rig"]["licence"].clone()), (json!("3D Print Rig"), json!("MIT")));
    assert!(c["rig"]["notice"].as_str().unwrap().contains("Permission is hereby granted, free of charge"), "{c}");
    let n = r.get("/api/printer-models/prusa_core_one/notice");
    assert!(n["text"].as_str().unwrap().contains("Printables model 1520471") && n["text"].as_str().unwrap().contains("Open Community License (OCL v1)"), "{n}");
    let none = r.get("/api/printer-models/tictac_21_120/notice");
    assert!(none["error"].is_string(), "a model with no notice says so: {none}");
}

/// Finish-2 F05 (Stella, 2026-10-19): Klipper's RESUME macro aborts with a cold nozzle (always after a heaters-off dry run)
/// and still answers "ok". The page is told it was refused, in the printer's own words; with a hot nozzle it resumes.
#[test]
fn a_klipper_resume_the_macro_aborts_is_reported_as_refused() {
    let dir = Dir::new("resume-cold");
    let a = FakeMoonraker::start(0);
    a.state.lock().unwrap().print_state = "paused".into();
    a.state.lock().unwrap().filename = "cube.gcode".into();
    let cfg = Config { controls_enabled: true, ..base_for(&dir, &a) };
    let r = run(&dir, cfg);
    wait_for("paused", || r.get("/api/tick")["job"]["state"] == "paused");
    let (code, v) = r.printer_call("POST", "/api/p/stella/commands", "stella", &json!({"id":"resume","confirm":true}));
    assert_eq!((code, v["status"].as_str()), (409, Some("refused")), "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("not hot enough")), "{v}");
    a.state.lock().unwrap().nozzle = 215.0;
    let (code, v) = r.printer_call("POST", "/api/p/stella/commands", "stella", &json!({"id":"resume","confirm":true}));
    assert_eq!((code, v["status"].as_str()), (200, Some("completed")), "{v}");
}

// ---------------------------------------------------------------------------------------------------------------
// Finish-2 F06 (2026-10-19): a new user's whole day. Each test below is a dead end the walk met.
// ---------------------------------------------------------------------------------------------------------------

/// The first printer could not be removed ("it carries the phone link and the failure watch"), so a wrong address typed for it could be neither
/// changed nor removed. It goes like any other: the next printer leads (its page is `/`, the failure watch follows it), and a farm with none left
/// is a fresh install again (the placeholder is back, `/` is the farm, and the file names no printer, so the next start is a fresh one too).
#[test]
fn the_first_printer_can_be_removed_and_a_farm_left_with_none_is_a_fresh_install_again() {
    let dir = Dir::new("remove-first");
    let (a, b) = (FakeMoonraker::start(0), FakeMoonraker::start(0));
    let cfg = Config { unconfigured: true, moonraker_url: String::new(), database: dir.file("stella.sqlite3"), ..Config::default() };
    let r = run(&dir, cfg);
    for (name, url) in [("Bench", a.url()), ("Shop", b.url())] {
        let (code, v) = r.call("POST", "/api/printers", Some(&json!({"name":name,"address":url})), true);
        assert_eq!(code, 200, "{v}");
    }
    let listing = |r: &Running| -> (Vec<String>, String) {
        let d = r.get("/api/printers");
        (d["printers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_owned()).collect(), d["default"].as_str().unwrap_or("").to_owned())
    };
    let saved = |dir: &Dir| -> Value { serde_json::from_str(&fs::read_to_string(dir.file("stella-scope.json")).unwrap()).unwrap() };
    assert_eq!(listing(&r), (vec!["bench".to_owned(), "shop".to_owned()], "bench".to_owned()));
    assert_eq!(r.farm.watch_target().map(|p| p.id.clone()), Some("bench".to_owned()), "the failure watch looks at the first real printer");

    // The first printer goes: the second leads, in the page, in the watch and in the file.
    let (code, v) = r.call("DELETE", "/api/printers/bench", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(listing(&r), (vec!["shop".to_owned()], "shop".to_owned()));
    assert_eq!(r.farm.watch_target().map(|p| p.id.clone()), Some("shop".to_owned()));
    assert_eq!(saved(&dir)["printers"].as_array().unwrap().len(), 1);

    // The last one goes: a fresh install again.
    let (code, v) = r.call("DELETE", "/api/printers/shop", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(listing(&r).0, Vec::<String>::new());
    assert_eq!(r.get("/api/printers")["count"], json!(0));
    assert_eq!(r.farm.list().len(), 1, "the placeholder is back, out of sight, so the first-printer machinery has what it needs");
    assert_eq!(r.farm.first().id, "placeholder");
    assert!(r.farm.watch_target().is_none(), "nothing to watch");
    let (code, home) = r.call("GET", "/", None, false);
    assert_eq!(code, 200);
    assert!(home.as_str().unwrap_or("").contains("fm-grid"), "/ is the farm, not a page stuck at 'Connecting'");
    let reload = Config::load(&dir.file("stella-scope.json")).unwrap();
    assert!(reload.unconfigured && reload.entries()[0].id == "placeholder", "the next start is a fresh install too");
    // and a printer can be added again at once
    let (code, v) = r.call("POST", "/api/printers", Some(&json!({"name":"Again","address":a.url()})), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(listing(&r), (vec!["again".to_owned()], "again".to_owned()));
}

/// A file from before the farm (one `moonraker_url`, no `printers`) whose only printer is removed must not bring it back at the next start.
#[test]
fn a_single_printer_file_that_loses_its_printer_does_not_get_it_back() {
    let dir = Dir::new("remove-legacy");
    let fake = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &fake));
    assert_eq!(r.get("/api/printers")["default"], json!("stella"));
    let (code, v) = r.call("DELETE", "/api/printers/stella", None, true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(r.get("/api/printers")["count"], json!(0));
    let reload = Config::load(&dir.file("stella-scope.json")).unwrap();
    assert!(reload.unconfigured, "the file no longer names a printer");
    assert_eq!(reload.entries()[0].id, "placeholder", "{:?}", reload.entries());
}

/// The hidden placeholder of a farm with no printer of its own was listed as "Stella" in the analytics and the queue, and written into every backup (which
/// then restored a printer called Stella with no address on the next computer).
#[test]
fn the_placeholder_is_never_a_printer_in_the_analytics_the_queue_or_a_backup() {
    let dir = Dir::new("ghost");
    let cfg = Config { unconfigured: true, moonraker_url: String::new(), database: dir.file("stella.sqlite3"), ..Config::default() };
    let r = run(&dir, cfg);
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(r.farm.list().len(), 10, "nine demos and the placeholder");
    let names = |v: &Value, key: &str| -> Vec<String> { v[key].as_array().unwrap().iter().map(|p| p["id"].as_str().or(p["printer"].as_str()).unwrap_or("?").to_owned()).collect() };
    let analytics = r.get("/api/analytics?days=7");
    assert_eq!(names(&analytics, "printers").len(), 9, "{analytics}");
    assert!(!names(&analytics, "printers").contains(&"placeholder".to_owned()));
    let queue = r.get("/api/queue");
    let queue_printers = names(&queue, "printers");
    assert_eq!(queue_printers.len(), 9, "{queue}");
    assert!(!queue_printers.contains(&"placeholder".to_owned()));
    let backup = r.get("/api/farm/backup");
    assert_eq!(names(&backup, "printers").len(), 9, "{backup}");
    assert!(!names(&backup, "printers").contains(&"placeholder".to_owned()), "a backup restores real printers only");
}

/// A slicer's "Test" button sends its key to GET /api/version and believes a 200. A wrong key (or uploads switched off) used to read "works" and fail at the
/// first upload; a key that was sent is checked now, and the question with no key still answers.
#[test]
fn a_slicers_test_with_a_wrong_key_is_refused_and_a_right_key_or_none_answers() {
    let dir = Dir::new("slicer-test");
    let fake = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &fake));
    let key = r.get("/api/slicer")["key"].as_str().unwrap().to_owned();
    assert!(key.len() >= 16, "{key}");
    let ask = |k: Option<&str>| -> u16 {
        let mut req = r.agent.get(&format!("{}/api/version", r.base));
        if let Some(k) = k {
            req = req.header("X-Api-Key", k);
        }
        req.call().unwrap().status().as_u16()
    };
    assert_eq!(ask(Some(&key)), 200, "the right key");
    assert_eq!(ask(None), 200, "no key: who are you");
    assert_eq!(ask(Some("not-the-key")), 403, "a wrong key is said, not believed");
    assert_eq!(ask(Some("")), 403, "an empty key is a wrong key");
}

/// The phone page said "Enable remote_link to pair a phone": a name in a file, and no way to set it. The route writes it, with a port that is free,
/// keeps the person's own port when the file has one, and asks for the control token like every change to the farm.
#[test]
fn phone_access_is_switched_on_from_the_page_and_says_it_starts_with_the_next_start() {
    let dir = Dir::new("phone-access");
    let fake = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &fake));
    let (code, said) = r.call("POST", "/api/pair/new", Some(&json!({})), false);
    assert_eq!(code, 400);
    let words = said["error"].as_str().unwrap_or("");
    assert!(words.contains("switched off") && !words.contains("lan_port") && !words.contains("json"), "plain words: {words}");
    assert_eq!(r.call("POST", "/api/farm/phone-access", Some(&json!({})), false).0, 401, "no token, no change");
    let (code, v) = r.call("POST", "/api/farm/phone-access", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!((v["enabled"].clone(), v["restart_needed"].clone()), (json!(true), json!(true)), "{v}");
    let port = v["lan_port"].as_u64().unwrap();
    assert!(port > r.port as u64 && port != r.port as u64, "a port of its own: {port}");
    let saved: Value = serde_json::from_str(&fs::read_to_string(dir.file("stella-scope.json")).unwrap()).unwrap();
    assert_eq!((saved["remote_link"].clone(), saved["lan_port"].as_u64()), (json!(true), Some(port)), "{saved}");
    assert_eq!(saved["moonraker_url"], json!(fake.url()), "nothing else in the file changed");
    assert!(Config::load(&dir.file("stella-scope.json")).is_ok(), "the file still loads");
    assert_eq!(r.get("/api/pair/remote")["pending_restart"], json!(true), "the phone page says it starts with the next start instead of offering the button again");
    // a second press keeps the port the file already names
    let (code, v) = r.call("POST", "/api/farm/phone-access", Some(&json!({})), true);
    assert_eq!((code, v["lan_port"].as_u64()), (200, Some(port)), "{v}");
}

/// A person trying the demo sends the sample file to the queue and it has to fit the demo machine it was made for, with no note asking for a nozzle size
/// that a demo printer has no way to be told.
#[test]
fn a_demo_machine_has_a_nozzle_so_a_sample_file_fits_it_without_a_note() {
    let farm = crate::demo::demo_farm();
    assert!(farm.iter().all(|e| e.nozzle_mm == Some(0.4)), "every demo machine has the usual nozzle");
    let core = farm.iter().find(|e| e.id == "demo-core-one-l").unwrap();
    let profile = crate::queue::Profile {
        id: core.id.clone(),
        name: core.name.clone(),
        slicer_model: core.slicer_model.clone(),
        model_name: core.model.clone(),
        bed_mm: core.bed_size_mm,
        nozzle_mm: core.nozzle_mm,
        material: core.material.clone(),
    };
    let needs = crate::queue::Needs { printer_model: Some("COREONEL".into()), nozzle_mm: Some(0.4), filament: vec!["PETG".into()], bed_mm: Some([300.0, 300.0]), extents_mm: Some([125.0, 125.0, 55.0]) };
    let fit = crate::queue::fit(&needs, false, &profile);
    assert!(fit.ok && fit.reasons.is_empty(), "{fit:?}");
    assert!(!fit.notes.iter().any(|n| n.contains("nozzle")), "no note asking which nozzle the demo has: {:?}", fit.notes);
}

/// The sixth real printer starts the farm's one thirty-day trial. The walk found that the running service went on saying "a licence is asked for" with no
/// trial running until it was restarted, and that every printer added after the sixth wrote a newer start into the file: the trial lasted as long as the
/// service kept running. Demo printers never count.
#[test]
fn the_sixth_real_printer_starts_one_trial_the_running_service_shows_and_later_printers_do_not_restart_it() {
    let dir = Dir::new("trial");
    let fakes: Vec<FakeMoonraker> = (0..7).map(|_| FakeMoonraker::start(0)).collect();
    let r = run(&dir, base_for(&dir, &fakes[0]));
    let add = |i: usize| {
        let (code, v) = r.call("POST", "/api/printers", Some(&json!({"name":format!("Fake {i}"),"address":fakes[i].url()})), true);
        assert_eq!(code, 200, "{v}");
    };
    let started = || -> Option<f64> { serde_json::from_str::<Value>(&fs::read_to_string(dir.file("stella-scope.json")).unwrap()).unwrap()["licence_trial_started"].as_f64() };
    for i in 1..5 {
        add(i);
    }
    let (code, v) = r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true);
    assert_eq!(code, 200, "{v}");
    let st = r.get("/api/licence");
    assert_eq!((st["state"].clone(), st["printers"].clone()), (json!("free"), json!(5)), "five real printers and a demo farm are free: {st}");
    assert!(started().is_none(), "no trial yet");
    add(5);
    let st = r.get("/api/licence");
    assert_eq!(st["state"], json!("trial"), "the running service shows the trial at once: {st}");
    assert!(st["trial_days_left"].as_f64().unwrap() > 29.0, "{st}");
    let first = started().expect("the start is kept in the file");
    thread::sleep(Duration::from_millis(1100));
    add(6);
    assert_eq!(started(), Some(first), "a seventh printer does not start the trial again");
    assert_eq!(r.get("/api/licence")["state"], json!("trial"));
}

/// The analytics said 0.13 hours for a print the history lists as 54 minutes: the print had been joined half-way, and only the minutes the recorder saw
/// were counted. The hours are the print's own duration, the same figure the history shows; a print that began before the window is clipped to it.
#[test]
fn the_analytics_hours_are_the_prints_own_duration_as_the_history_shows_it() {
    let dir = Dir::new("analytics-hours");
    let a = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &a));
    thread::sleep(Duration::from_millis(300));
    let now = crate::model::now();
    let db = rusqlite::Connection::open(dir.file("stella.sqlite3")).unwrap();
    // joined half-way: seen for 8 minutes, 54 minutes long by the printer's own count
    // and a long one that began two days ago and ended an hour ago: only the last 23 hours are inside a one-day window
    for (start, end, secs, name) in [(now - 600.0, now - 120.0, 3227.0, "joined.gcode"), (now - 2.0 * 86400.0, now - 3600.0, 2.0 * 86400.0 - 3600.0, "long.gcode")] {
        db.execute(
            "INSERT INTO print_sessions(start_ts,end_ts,filename,duration_s,filament_used,result,evidence) VALUES(?1,?2,?3,?4,NULL,'complete','test')",
            rusqlite::params![start, end, name, secs],
        )
        .unwrap();
    }
    let hist = r.get("/api/history/prints");
    let shown: f64 = hist["prints"].as_array().unwrap().iter().filter(|p| p["filename"] == "joined.gcode").map(|p| p["duration_s"].as_f64().unwrap()).sum();
    assert_eq!(shown, 3227.0, "{hist}");
    let an = r.get("/api/analytics?days=1");
    let hours = an["printers"][0]["print_h"].as_f64().unwrap();
    assert!((hours - (3227.0 + 23.0 * 3600.0) / 3600.0).abs() < 0.01, "the joined print counts as the history says, the long one only for the part inside the day: {an}");
}

/// Restoring a backup of a farm of ten printers was refused ("request body exceeds the 4096-byte command limit"): a backup is more than 6 KB at ten printers. The one route that
/// takes a backup takes a big one; any other route keeps the small limit.
#[test]
fn a_backup_of_a_big_farm_restores_and_only_the_restore_route_takes_a_big_body() {
    let dir = Dir::new("big-backup");
    let cfg = Config { unconfigured: true, moonraker_url: String::new(), database: dir.file("stella.sqlite3"), ..Config::default() };
    let r = run(&dir, cfg);
    assert_eq!(r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true).0, 200);
    for i in 0..12 {
        let (code, v) = r.call("POST", "/api/spools", Some(&json!({"material":"PETG","brand":format!("Brand {i}"),"name":"Colour name that is a little long","colour":"#FF6A00","grams_total":1000})), true);
        assert_eq!(code, 200, "{v}");
    }
    let backup = r.get("/api/farm/backup");
    let size = backup.to_string().len();
    assert!(size > 6000, "a ten-printer backup is bigger than the command limit: {size}");
    let dir2 = Dir::new("big-restore");
    let r2 = run(&dir2, Config { unconfigured: true, moonraker_url: String::new(), database: dir2.file("stella.sqlite3"), ..Config::default() });
    let (code, v) = r2.call("POST", "/api/farm/restore", Some(&backup), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!((v["added"].as_array().map(Vec::len), v["spools_added"].clone()), (Some(9), json!(12)), "{v}");
    // a command route still has the small limit
    let big = json!({"name":"x".repeat(5000),"address":"http://127.0.0.1:9"});
    assert_eq!(r2.call("POST", "/api/printers", Some(&big), true).0, 413);
}

/// A backup restored on a new computer holds no logins, so a PrusaLink printer in it said "asked for a login" and Settings had nowhere to type one ("remove the printer
/// and add it again"). The login is typed in Settings now: it is written beside the others (mode 600, not in the settings file), the printer starts again on it, keeps its
/// place and its recording, and the list says whether a login is saved without ever showing it.
#[test]
fn a_restored_printer_gets_its_login_in_settings_and_comes_up_with_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("set-login");
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let cfg = Config { unconfigured: true, moonraker_url: String::new(), database: dir.file("stella.sqlite3"), ..Config::default() };
    let r = run(&dir, cfg);
    // a backup as another computer made it: the printer, no login
    let backup = json!({"format":"stella-scope-farm-backup","version":1,"printers":[{"id":"shop","name":"Shop","driver":"prusalink","address":prusa.url(),"needs_login":true}],"books":{"spools":[]}});
    let (code, v) = r.call("POST", "/api/farm/restore", Some(&backup), true);
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["needs_login"], json!(["shop"]), "{v}");
    let row = |r: &Running| r.get("/api/printers")["printers"][0].clone();
    wait_for("the printer to say it needs a login", || {
        let p = row(&r);
        p["state"] == "offline" || p["state"] == "error"
    });
    assert_eq!(row(&r)["login_saved"], json!(false));
    assert_eq!(row(&r)["needs_login"], json!(true), "the tile says it needs its login and where to type it, not 'check that it is on'");
    // typing nothing, or a login for a demo, is refused in words
    assert_eq!(r.call("PATCH", "/api/printers/shop", Some(&json!({"credentials":{"user":" ","password":""}})), true).0, 400);
    let (code, v) = r.call("PATCH", "/api/printers/shop", Some(&json!({"credentials":{"user":"maker","password":"wrong"}})), true);
    assert_eq!(code, 200, "a wrong login is saved and the printer says so by its state, as when it was added: {v}");
    assert_eq!(row(&r)["login_saved"], json!(true));
    // the right one
    let (code, v) = r.call("PATCH", "/api/printers/shop", Some(&json!({"credentials":{"user":"maker","password":"secret"}})), true);
    assert_eq!(code, 200, "{v}");
    wait_for("the printer to come up on its login", || {
        let p = row(&r);
        p["state"] == "idle" && p["online"] == true
    });
    assert_eq!(row(&r)["needs_login"], json!(false));
    assert_eq!(r.farm.visible().len(), 1, "still one printer, in its place");
    // where the login lives: the secret file, mode 600, and not in the settings file, the list or the backup
    let secret = dir.0.join("printers/shop.secret");
    assert_eq!(fs::metadata(&secret).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(fs::read_to_string(&secret).unwrap().contains("secret"));
    let saved = fs::read_to_string(dir.file("stella-scope.json")).unwrap();
    assert!(!saved.contains("secret") && saved.contains("credentials_ref"), "{saved}");
    assert!(!r.get("/api/printers").to_string().contains("secret"), "the list says a login is saved, never what it is");
    assert!(!r.get("/api/farm/backup").to_string().contains("secret"));
    // the settings alongside the login still work, and the demo has none
    assert_eq!(r.call("PATCH", "/api/printers/shop", Some(&json!({"name":"Shop Prusa"})), true).0, 200);
    assert_eq!(r.call("POST", "/api/printers/demo-farm", Some(&json!({})), true).0, 200);
    assert_eq!(r.call("PATCH", "/api/printers/demo-mk3s", Some(&json!({"credentials":{"user":"x"}})), true).0, 400);
}

/// U03 b (2026-10-20: "it said back online, the wall did not change"). A Klipper printer that is switched off must stay offline on the wall, not read Ready
/// again for half a second every few seconds. The push source (which a Moonraker that does not answer websockets leaves retrying) sends a silence probe every 5 s;
/// a collector cycle that carried no poll and no error read "connected" for the 10 s after the last good poll, so the printer flickered to Ready, and the farm's
/// watcher said "back online" for a printer that was not. Looks every 25 ms for 13 s after it goes dark, which covers a probe.
#[test]
fn a_printer_that_has_gone_dark_stays_offline_and_never_flickers_to_ready() {
    let dir = Dir::new("flicker");
    let fake = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &fake));
    let state = |r: &Running| r.get("/api/printers")["printers"][0]["state"].as_str().unwrap_or("").to_owned();
    wait_for("online", || r.get("/api/printers")["printers"][0]["online"] == true);
    drop(fake);
    wait_for("offline", || state(&r) == "offline");
    let start = Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_secs(13) {
        let s = state(&r);
        if s != "offline" {
            seen.push((start.elapsed().as_millis(), s));
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(seen.is_empty(), "a printer that is off read something else after it was seen offline: {seen:?}");
}

/// UI board U07 (2026-10-20): the failure watch's alert carries where it saw the failure, so the alarm screen can draw the box on the camera picture. The feed has the
/// verdict, the score and the boxes beside the words; an alert that has none (every other kind) is exactly what it was.
#[test]
fn a_failure_watch_alert_carries_where_it_saw_the_failure_into_the_feed() {
    let dir = Dir::new("alarm-detail");
    let fake = FakeMoonraker::start(0);
    let r = run(&dir, base_for(&dir, &fake));
    let detail = json!({"verdict":"failing","score":0.87,"boxes":[{"conf":0.87,"box":[0.52,0.30,0.84,0.62]}]});
    crate::notify::event_with("failure_watch", "u07-printer", "U07 printer", "U07 printer: this print looks like it is FAILING (spaghetti detector at 87%). Check the camera now; the print is still running.", detail.clone());
    crate::notify::event("done", "u07-printer", "U07 printer", "U07 printer: PETG cube finished.");
    let rows = |r: &Running| -> Vec<Value> { r.get("/api/alerts/feed?printer=u07-printer")["events"].as_array().cloned().unwrap_or_default() };
    wait_for("both alerts in the feed", || rows(&r).len() >= 2);
    let feed = rows(&r);
    let fail = feed.iter().find(|e| e["kind"] == "failure_watch").expect("the failure alert");
    assert_eq!(fail["detail"], detail, "{fail}");
    assert_eq!(fail["severity"], "alarm");
    let done = feed.iter().find(|e| e["kind"] == "done").expect("the other alert");
    assert!(done.get("detail").is_none(), "an alert with no detail has no detail key: {done}");
}

/// 2026-10-25: "The Prusa is on, but I can't do anything ... I can't change the nozzle." PrusaLink has no temperature endpoint, so a heat
/// target goes to the printer as a one-line file it starts by itself (Print-After-Upload), only while it is idle; during a print it is
/// refused with the reason and nothing is sent. The command file never shows as a print.
#[test]
fn a_prusalink_takes_heat_as_a_one_line_file_while_idle_and_refuses_it_while_printing() {
    let dir = Dir::new("prusa-heat");
    let a = FakeMoonraker::start(0);
    let prusa = FakePrusaLink::start(0, "maker", "secret");
    let r = run(&dir, base_for(&dir, &a));
    let add = json!({"name":"Core One L","driver":"prusalink","address":prusa.url(),"credentials":{"user":"maker","password":"secret"}});
    let (code, v) = r.call("POST", "/api/printers", Some(&add), true);
    assert_eq!(code, 200, "{v}");
    wait_for("the prusa online", || {
        r.get("/api/printers")["printers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "core-one-l" && x["online"] == true)
    });
    let sent = || -> Vec<Value> {
        prusa
            .log()
            .into_iter()
            .filter(|e| e["method"] == "PUT" && e["path"].as_str().unwrap_or("").ends_with("/stella-command.gcode"))
            .collect()
    };
    let (code, v) = r.printer_call("POST", "/api/p/core-one-l/commands", "core-one-l", &json!({"id":"set_heater","target":"extruder","value":215,"confirm":true}));
    assert_eq!(code, 200, "{v}");
    let puts = sent();
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert_eq!(puts[0]["path"], "/api/v1/files/usb/stella-command.gcode");
    assert_eq!(puts[0]["headers"]["print-after-upload"], "?1", "it runs by itself");
    // printing: refused, nothing more sent
    {
        let mut s = prusa.state.lock().unwrap();
        s.state = "PRINTING".into();
        s.job_id = Some(9);
        s.file = "bracket_PETG.gcode".into();
    }
    wait_for("printing", || r.get("/api/p/core-one-l/tick")["job"]["state"] == "printing");
    let (code, v) = r.printer_call("POST", "/api/p/core-one-l/commands", "core-one-l", &json!({"id":"set_heater","target":"extruder","value":220,"confirm":true}));
    assert_ne!(code, 200, "{v}");
    assert!(v.to_string().contains("only while the printer is idle"), "{v}");
    assert_eq!(sent().len(), 1, "nothing more was sent");
}
