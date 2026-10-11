use crate::{
    api,
    config::Config,
    log_tail::LogTail,
    model::{self, CollectorStatus},
    moonraker::{self, Moonraker},
    qgl::Parser,
    store::{Event, Store},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

const EXACT: &str = "Gantry-relative probe points:\n0: 15.474970 1: 15.504345 2: 15.487470 3: 15.464345\nMaking the following Z adjustments:\nstepper_z = 0.010297\nstepper_z1 = -0.035458\nstepper_z2 = -0.002368\nstepper_z3 = 0.027528\nRetries: 3/5 Probed points range: 0.040000 tolerance: 0.020000";
fn memory() -> Store {
    Store::open(":memory:", false).unwrap()
}
fn response(db: &Store, target: &str) -> (u16, Value) {
    let r = api::handle(
        "GET",
        target,
        db,
        &Config::default(),
        &CollectorStatus::default(),
    );
    (r.code, serde_json::from_slice(&r.body).unwrap())
}
fn add(db: &mut Store, ts: f64) {
    db.write(
        Some((ts, &model::demo(ts, &Config::default()))),
        &[],
        &mut Parser::default(),
    )
    .unwrap();
}
fn temp(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir()
        .join(format!(
            "stella-test-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned()
}

#[test]
fn parses_exact_brief_message() {
    let mut p = Parser::default();
    let rows = p.feed(100.0, EXACT);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!((r.run_id, r.round), (1, 3));
    assert_eq!(r.range_mm, 0.04);
    assert_eq!(r.z0, Some(0.010297));
    assert_eq!(r.z1, Some(-0.035458));
    assert_eq!(r.z2, Some(-0.002368));
    assert_eq!(r.z3, Some(0.027528));
}
#[test]
fn parses_console_fragments_and_new_runs() {
    let mut p = Parser::new(6);
    let mut rows = Vec::new();
    for line in EXACT.replace("Retries: 3/5", "Retries: 0/5").lines() {
        rows.extend(p.feed(100.0, &format!("// {line}")));
    }
    assert_eq!(rows[0].run_id, 7);
    assert_eq!(rows[0].z1, Some(-0.035458));
    let next = p.feed(
        101.0,
        "Retries: 1/5 Probed points range: 0.010 tolerance: 0.02",
    );
    assert_eq!(next[0].run_id, 7);
    assert_eq!(next[0].z0, None);
    let next = p.feed(
        102.0,
        "Retries: 0/5 Probed points range: 0.1 tolerance: 0.02",
    );
    assert_eq!(next[0].run_id, 8);
}
#[test]
fn abort_discards_partial_adjustments_and_nonfinite_values() {
    let mut p = Parser::default();
    p.feed(
        1.0,
        "stepper_z = 2.0\nRetries aborting: Probed points range is increasing...",
    );
    assert!(p
        .feed(2.0, "Retries: 1/5 Probed points range: NaN")
        .is_empty());
    let r = p.feed(3.0, "Retries: 0/5 Probed points range: 0.2");
    assert_eq!(r[0].z0, None);
}
#[test]
fn sparse_zero_is_not_missing_driver_status() {
    let d = json!({"tmc2209 stepper_z":{"run_current":0.8,"drv_status":{}},"stepper_enable":{"steppers":{"stepper_z":true}}});
    let m = model::motor(&d, "stepper_z");
    assert_eq!(m["cs"], 0);
    assert_eq!(m["stst"], false);
    assert_eq!(m["flags"], json!([]));
    let m = model::motor(
        &json!({"tmc2209 stepper_z":{"drv_status":null}}),
        "stepper_z",
    );
    assert!(m["cs"].is_null());
    assert!(m["stst"].is_null());
    assert!(m["flags"].is_null());
    assert!(model::motor(&json!({}), "stepper_z")["energised"].is_null());
}
#[test]
fn driver_flags_and_enable_are_retained() {
    let m = model::motor(
        &json!({"tmc2209 stepper_z1":{"drv_status":{"cs_actual":23,"otpw":1,"stst":1}},
        "stepper_enable":{"steppers":{"stepper_z1":false}}}),
        "stepper_z1",
    );
    assert_eq!(m["cs"], 23);
    assert_eq!(m["stst"], true);
    assert_eq!(m["flags"], json!(["otpw"]));
    assert_eq!(m["energised"], false);
}
#[test]
fn raw_samples_round_trip_and_rewind_uses_at_or_before() {
    let mut db = memory();
    let raw = json!({"unknown_future_field":{"text":"<script>not HTML</script>"}});
    db.write(Some((10.0, &raw)), &[], &mut Parser::default())
        .unwrap();
    add(&mut db, 20.0);
    assert!(db.at(9.0).unwrap().is_none());
    assert_eq!(db.at(19.0).unwrap().unwrap(), (10.0, raw));
    assert_eq!(db.at(20.0).unwrap().unwrap().0, 20.0);
}
#[test]
fn keyframes_and_deltas_reconstruct_one_thousand_samples() {
    let mut db = memory();
    let mut parser = Parser::default();
    let mut expected = Vec::new();
    for i in 0..1000 {
        let mut data = json!({
            "configfile":{"settings":{"printer":{"kinematics":"corexy"}}},
            "_scope_capture":{"available":["motion_report","mcu"]},
            "bed_mesh":{"profile_name":"default","profiles":{"default":{"points":[[0,1],[2,3]]}}},
            "mcu":{"mcu_version":"v1","mcu_constants":{"MCU":"test"},"last_stats":{"bytes":i}},
            "motion_report":{"live_position":[i,2,3],"padding":"x".repeat(1024)},
            "probe":{"last_query":if i%7==0 {Value::Null}else{json!(false)}}
        });
        if i % 11 == 0 {
            data["probe"].as_object_mut().unwrap().remove("last_query");
        }
        db.write(Some((i as f64 / 4.0, &data)), &[], &mut parser)
            .unwrap();
        expected.push(data);
    }
    let times = db.sample_times(0.0, 1000.0, 1001).unwrap();
    assert_eq!(times.len(), 1000);
    for (i, ts) in times.into_iter().enumerate() {
        assert_eq!(
            db.load_sample(ts, 2 * 1024 * 1024).unwrap().unwrap().1,
            expected[i]
        );
    }
    let (keys,deltas,max_delta):(u64,u64,usize)=db.conn.query_row("SELECT SUM(kind='K'),SUM(kind='D'),MAX(CASE WHEN kind='D' THEN length(CAST(data AS BLOB)) ELSE 0 END) FROM samples",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(keys >= 4 && deltas > 900);
    assert!(
        max_delta <= 400,
        "largest synthetic idle delta was {max_delta} bytes"
    );
}
#[test]
fn shared_context_is_content_addressed_once() {
    let mut db = memory();
    let mut parser = Parser::default();
    for i in 0..3 {
        let data = json!({"configfile":{"settings":{"secret":"same"}},"_scope_capture":{"available":["x"]},"bed_mesh":{"profiles":{"default":[1,2]},"profile_name":"default"},"mcu":{"mcu_version":"v1","mcu_constants":{"MCU":"x"},"last_stats":{"count":i}}});
        db.write(Some((i as f64, &data)), &[], &mut parser).unwrap();
    }
    let contexts: u64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM context", [], |r| r.get(0))
        .unwrap();
    assert_eq!(contexts, 1);
    let raw: String = db
        .conn
        .query_row(
            "SELECT data FROM samples ORDER BY ts DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!raw.contains("secret") && !raw.contains("mcu_version") && !raw.contains("profiles"));
    assert_eq!(
        db.at(2.0).unwrap().unwrap().1["configfile"]["settings"]["secret"],
        "same"
    );
}
#[test]
fn readonly_legacy_schema_rows_still_replay() {
    let path = temp("legacy.sqlite3");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE samples(ts REAL PRIMARY KEY,data TEXT NOT NULL);CREATE TABLE events(ts REAL,kind TEXT,message TEXT);CREATE TABLE qgl(ts REAL,run_id INTEGER,round INTEGER,range_mm REAL,z0 REAL,z1 REAL,z2 REAL,z3 REAL);").unwrap();
        conn.execute(
            "INSERT INTO samples(ts,data) VALUES(1,?1)",
            [json!({"legacy":true}).to_string()],
        )
        .unwrap();
    }
    let db = Store::open(&path, true).unwrap();
    assert_eq!(db.at(1.0).unwrap().unwrap().1, json!({"legacy":true}));
    drop(db);
    fs::remove_file(path).unwrap();
}
#[test]
fn writable_legacy_schema_migrates_without_rewriting_old_rows() {
    let path = temp("legacy-migrate.sqlite3");
    let old = json!({"legacy":true});
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE samples(ts REAL PRIMARY KEY,data TEXT NOT NULL);CREATE TABLE events(ts REAL,kind TEXT,message TEXT);CREATE TABLE qgl(ts REAL,run_id INTEGER,round INTEGER,range_mm REAL,z0 REAL,z1 REAL,z2 REAL,z3 REAL);").unwrap();
        conn.execute(
            "INSERT INTO samples(ts,data) VALUES(1,?1)",
            [old.to_string()],
        )
        .unwrap();
    }
    let mut db = Store::open(&path, false).unwrap();
    let new = json!({"legacy":false,"configfile":{"settings":{"printer":{"kinematics":"corexy"}}}});
    db.write(Some((2.0, &new)), &[], &mut Parser::default())
        .unwrap();
    assert_eq!(db.at(1.0).unwrap().unwrap().1, old);
    assert_eq!(db.at(2.0).unwrap().unwrap().1, new);
    let old_kind: Option<String> = db
        .conn
        .query_row("SELECT kind FROM samples WHERE ts=1", [], |r| r.get(0))
        .unwrap();
    assert!(old_kind.is_none());
    drop(db);
    for suffix in ["", "-wal", "-shm"] {
        let file = format!("{path}{suffix}");
        if std::path::Path::new(&file).exists() {
            fs::remove_file(file).unwrap();
        }
    }
}
#[test]
fn size_retention_drops_only_complete_oldest_groups() {
    let mut db = memory();
    let mut parser = Parser::default();
    for i in 0..100 {
        let data = json!({"changing":{"value":format!("{i}-{}","x".repeat(20000))}});
        db.write(Some((i as f64, &data)), &[], &mut parser).unwrap();
    }
    let before = db.counts().unwrap().0;
    db.prune(0.0, 1, None).unwrap();
    let after = db.counts().unwrap().0;
    assert!(after < before && after > 0);
    let first = db.bounds().unwrap().0.unwrap();
    let kind: String = db
        .conn
        .query_row("SELECT kind FROM samples WHERE ts=?1", [first], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kind, "K");
    assert!(db.at(f64::MAX).unwrap().unwrap().1["changing"]["value"]
        .as_str()
        .unwrap()
        .starts_with("99-"));
    let page: u64 = db
        .conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let pages: u64 = db
        .conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let free: u64 = db
        .conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert!((pages - free) * page <= 1024 * 1024);
}
#[test]
fn duplicate_sample_clocks_do_not_replace_previous_data() {
    let mut db = memory();
    add(&mut db, 10.0);
    add(&mut db, 10.0);
    add(&mut db, 9.0);
    assert_eq!(db.counts().unwrap().0, 3);
    assert!(db.bounds().unwrap().1.unwrap() > 10.0);
}
#[test]
fn console_dedup_is_persistent_and_keeps_original_type() {
    let mut db = memory();
    let mut parser = Parser::default();
    let e = Event::new(10.0, "response", EXACT);
    let w = db
        .write(None, &[e.clone(), e.clone()], &mut parser)
        .unwrap();
    assert_eq!((w.events, w.rounds), (1, 1));
    let mut restarted = Parser::new(db.last_run().unwrap());
    let w = db
        .write(
            None,
            &[Event::new(10.0, "different-type", EXACT)],
            &mut restarted,
        )
        .unwrap();
    assert_eq!((w.events, w.rounds), (0, 0));
    assert_eq!(db.events(0.0, 20.0, 100).unwrap()[0].kind, "response");
}
#[test]
fn failures_and_position_warnings_are_indexed_events() {
    let mut db = memory();
    let mut p = Parser::default();
    db.write(
        None,
        &[
            Event::new(
                5.0,
                "response",
                "Retries aborting: Probed points range is increasing...",
            ),
            Event::new(
                6.0,
                "response",
                "Stepper 'stepper_z1' position skew after probe: pos X now Y",
            ),
        ],
        &mut p,
    )
    .unwrap();
    let e = db.events(0.0, 10.0, 20).unwrap();
    assert!(e.iter().any(|e| e.kind == "levelling_failed"));
    assert!(e.iter().any(|e| e.kind == "position_warning"));
}
#[test]
fn failed_transaction_does_not_advance_parser_or_save_sample() {
    let mut db = memory();
    let mut p = Parser::default();
    let e = [
        Event::new(1.0, "response", EXACT),
        Event::new(f64::NAN, "response", "bad clock"),
    ];
    assert!(db
        .write(Some((1.0, &json!({"sample":1}))), &e, &mut p)
        .is_err());
    assert_eq!(p.run_id, 0);
    assert_eq!(db.counts().unwrap(), (0, 0, 0));
}
#[test]
fn downsampling_is_bounded_ordered_and_keeps_edges() {
    let mut db = memory();
    for i in 0..101 {
        add(&mut db, i as f64);
    }
    let points = db.series(10.0, 90.0, 7).unwrap();
    assert_eq!(points.len(), 7);
    assert_eq!(points.first().unwrap()["ts"], 10.0);
    assert_eq!(points.last().unwrap()["ts"], 90.0);
    assert!(points
        .windows(2)
        .all(|p| p[0]["ts"].as_f64() < p[1]["ts"].as_f64()));
    assert!(points
        .iter()
        .all(|p| p["motors"]["stepper_z1"]["cur"].as_f64().is_some()));
}
#[test]
fn downsampling_empty_single_and_invalid_windows() {
    let mut db = memory();
    assert!(db.series(0.0, 1.0, 10).unwrap().is_empty());
    add(&mut db, 0.5);
    assert_eq!(db.series(0.0, 1.0, 10).unwrap().len(), 1);
    assert_eq!(db.series(0.5, 0.5, 2).unwrap().len(), 1);
    assert!(db.series(2.0, 1.0, 10).is_err());
    assert!(db.series(0.0, 1.0, 1).is_err());
}
#[test]
fn api_meta_live_and_empty_recording() {
    let mut db = memory();
    let (code, v) = response(&db, "/api/live");
    assert_eq!(code, 200);
    assert!(v["sample"].is_null());
    add(&mut db, 100.0);
    let (_, v) = response(&db, "/api/meta");
    assert_eq!(v["first_ts"], 100.0);
    assert_eq!(v["read_only"], true);
    assert_eq!(v["geometry"]["corners"][1]["name"], "stepper_z1");
    let (_, v) = response(&db, "/api/live");
    assert_eq!(v["sample"]["ts"], 100.0);
}
#[test]
fn api_rewind_selects_past_health_and_geometry_during_an_outage() {
    let mut db = memory();
    add(&mut db, 100.0);
    add(&mut db, 200.0);
    let mut p = Parser::default();
    db.write(
        None,
        &[
            Event::new(
                90.0,
                "scope_geometry",
                json!({"bed_size_mm":[350,350]}).to_string(),
            ),
            Event::new(100.0, "scope_health", json!({"connected":true}).to_string()),
            Event::new(
                120.0,
                "scope_health",
                json!({"connected":false,"last_error":"Printer not answering"}).to_string(),
            ),
            Event::new(190.0, "scope_health", json!({"connected":true}).to_string()),
        ],
        &mut p,
    )
    .unwrap();
    let (_, v) = response(&db, "/api/at?ts=150");
    assert_eq!(v["sample"]["ts"], 100.0);
    assert_eq!(v["age_seconds"], 50.0);
    assert_eq!(v["recorded_health"]["connected"], false);
    assert_eq!(v["geometry_recorded"], true);
}
#[test]
fn api_events_and_levelling_exclude_future_rows() {
    let mut db = memory();
    let mut p = Parser::default();
    db.write(
        None,
        &[
            Event::new(10.0, "response", EXACT),
            Event::new(20.0, "response", EXACT),
        ],
        &mut p,
    )
    .unwrap();
    let (_, v) = response(&db, "/api/events?from=0&to=15");
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
    let (_, v) = response(&db, "/api/qgl?from=0&to=15");
    assert_eq!(v["rounds"].as_array().unwrap().len(), 1);
    assert_eq!(v["rounds"][0]["pass"], false);
}
#[test]
fn api_rejects_bad_timestamps_and_unbounded_requests() {
    let db = memory();
    for route in [
        "/api/at",
        "/api/at?ts=NaN",
        "/api/at?ts=-1",
        "/api/at?ts=inf",
        "/api/at?ts=1&ts=2",
        "/api/at?ts=%GG",
        "/api/events?from=10&to=1",
        "/api/series?from=0&to=10&points=1",
        "/api/series?points=999999",
    ] {
        assert_eq!(response(&db, route).0, 400, "{route}");
    }
    assert_eq!(response(&db, "/api/move").0, 404);
    assert_eq!(
        api::handle(
            "POST",
            "/api/live",
            &db,
            &Config::default(),
            &CollectorStatus::default()
        )
        .code,
        405
    );
}
#[test]
fn all_static_assets_are_embedded() {
    for path in ["/", "/app.js", "/style.css"] {
        let r = api::asset("GET", path).unwrap();
        assert_eq!(r.code, 200);
        assert!(!r.body.is_empty());
    }
    assert!(!include_str!("../web/app.js").contains("innerHTML"));
}
#[test]
fn configuration_defaults_and_validation() {
    let cfg: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(cfg.poll_interval_ms, 500);
    assert_eq!(cfg.keep_days, 14);
    assert_eq!(cfg.max_db_mb, 4096);
    assert_eq!(cfg.lan_bind, "127.0.0.1");
    cfg.validate().unwrap();
    assert!(serde_json::from_str::<Config>("{\"typo_setting\":2}").is_err());
    assert!(
        serde_json::from_str::<Config>("{\"burst_poll_ms\":50}").is_err(),
        "removed keys are rejected"
    );
    let mut cfg = cfg;
    cfg.port = 0;
    assert!(cfg.validate().is_err());
    cfg.port = 7420;
    cfg.poll_interval_ms = 0;
    assert!(cfg.validate().is_err());
    cfg.poll_interval_ms = 500;
    cfg.validate().unwrap();
    cfg.lan_port = Some(7422);
    cfg.validate().unwrap();
    cfg.max_http_connections = 4;
    assert!(cfg.validate().unwrap_err().contains("desktop reader slots"));
    cfg.max_http_connections = 32;
    cfg.lan_bind = "0.0.0.0".into();
    assert!(cfg.validate().unwrap_err().contains("127.0.0.1"));
    cfg.lan_bind = "127.0.0.1".into();
    // The desktop Host gate can only match a loopback listener; a LAN bind would answer 403 to all.
    for bind in ["0.0.0.0", "192.168.1.20", "::"] {
        cfg.bind = bind.into();
        assert!(cfg.validate().unwrap_err().contains("loopback"), "{bind}");
    }
    cfg.bind = "::1".into();
    cfg.validate().unwrap();
}
#[test]
fn object_names_use_percent20_and_field_filter_stays_separate() {
    let query = moonraker::object_query(&[
        "tmc2209 stepper_z1".into(),
        "configfile=settings".into(),
        "heater_fan hotend_fan".into(),
    ]);
    assert_eq!(
        query,
        "tmc2209%20stepper_z1&configfile=settings&heater_fan%20hotend_fan"
    );
    assert!(!query.contains('+'));
}
#[test]
fn retention_removes_only_older_rows() {
    let mut db = memory();
    add(&mut db, 1.0);
    add(&mut db, 10.0);
    let mut p = Parser::default();
    db.write(
        None,
        &[
            Event::new(1.0, "response", EXACT),
            Event::new(10.0, "response", EXACT),
        ],
        &mut p,
    )
    .unwrap();
    db.prune(5.0, 4096, None).unwrap();
    assert_eq!(db.counts().unwrap(), (2, 1, 1));
    assert_eq!(db.bounds().unwrap(), (Some(1.0), Some(10.0)));
    assert!(db.at(10.0).unwrap().is_some());
}
#[test]
fn local_log_starts_at_end_and_preserves_split_lines() {
    let path = temp("klippy.log");
    fs::write(&path, "old line\n").unwrap();
    let mut tail = LogTail::default();
    assert!(tail.read_new(&path, 65536).unwrap().0.is_empty());
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    write!(f, "Stepper 'stepper_z1' position skew after probe:").unwrap();
    f.flush().unwrap();
    assert!(tail.read_new(&path, 65536).unwrap().0.is_empty());
    writeln!(f, " pos X now Y").unwrap();
    f.flush().unwrap();
    assert_eq!(tail.read_new(&path, 65536).unwrap().0.len(), 1);
    assert!(tail.read_new(&path, 65536).unwrap().0.is_empty());
    drop(f);
    fs::remove_file(path).unwrap();
}
#[test]
fn local_log_rotation_is_reported() {
    let path = temp("rotate.log");
    fs::write(&path, "original\n").unwrap();
    let mut tail = LogTail::default();
    tail.read_new(&path, 65536).unwrap();
    let old = format!("{path}.old");
    fs::rename(&path, &old).unwrap();
    fs::write(
        &path,
        "Stepper 'stepper_z1' position skew after probe: pos X now Y\n",
    )
    .unwrap();
    let (lines, notices) = tail.read_new(&path, 65536).unwrap();
    assert_eq!(lines.len(), 1);
    assert!(notices.iter().any(|n| n.contains("rotated")));
    fs::remove_file(path).unwrap();
    fs::remove_file(old).unwrap();
}
fn fake_server(body: Value, status: &str) -> (Config, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut all = Vec::new();
        let mut buffer = [0; 1024];
        while !all.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..n]);
        }
        let text = body.to_string();
        write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len()).unwrap();
        String::from_utf8(all).unwrap()
    });
    let cfg = Config {
        moonraker_url: format!("http://{address}"),
        ..Config::default()
    };
    (cfg, worker)
}
#[test]
fn transport_is_get_only_and_decodes_result() {
    let (cfg, worker) = fake_server(json!({"result":{"value":42}}), "200 OK");
    let result = Moonraker::new(&cfg)
        .get("/printer/objects/query?tmc2209%20stepper_z1")
        .unwrap();
    assert_eq!(result["value"], 42);
    let request = worker.join().unwrap();
    assert!(request.starts_with("GET /printer/objects/query?tmc2209%20stepper_z1 "));
}
#[test]
fn transport_errors_name_a_remedy() {
    let (cfg, worker) = fake_server(json!({"error":"denied"}), "401 Unauthorized");
    let error = Moonraker::new(&cfg).get("/server/info").unwrap_err();
    assert!(error.contains("allowed") && error.contains("STELLA_MOONRAKER_API_KEY"));
    worker.join().unwrap();
    let (cfg, worker) = fake_server(json!({"unexpected":{}}), "200 OK");
    assert!(Moonraker::new(&cfg)
        .get("/server/info")
        .unwrap_err()
        .contains("no result"));
    worker.join().unwrap();
}
#[test]
fn demo_changes_motion_and_current_without_hardware() {
    let cfg = Config::default();
    let a = model::demo(1.0, &cfg);
    let b = model::demo(20.0, &cfg);
    assert_ne!(
        a["motion_report"]["live_position"],
        b["motion_report"]["live_position"]
    );
    assert_ne!(a["extruder"]["temperature"], b["extruder"]["temperature"]);
    assert_eq!(
        a["stepper_enable"]["steppers"].as_object().unwrap().len(),
        7
    );
}

/// A replay instance never asks the printer for a print file. The recording keeps no G-code or
/// thumbnails, so the answer is a plain "not stored" and the configured Moonraker is never contacted.
#[test]
fn replay_never_asks_the_printer_for_print_files() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let cfg = Config {
        moonraker_url: format!("http://{}", listener.local_addr().unwrap()),
        ..Config::default()
    };
    let status = CollectorStatus {
        mode: "replay".into(),
        ..CollectorStatus::default()
    };
    let db = memory();
    for (target, says) in [
        ("/api/print/gcode?filename=cube.gcode", "G-code not stored in the recording"),
        ("/api/print/thumbnail?filename=cube.gcode", "Thumbnail not stored in the recording"),
    ] {
        let r = api::handle("GET", target, &db, &cfg, &status);
        assert_eq!(r.code, 404, "{target}");
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["error"], says, "{target}");
        assert_eq!(v["stored"], false, "{target}");
    }
    assert!(listener.accept().is_err(), "the printer was contacted");
}
