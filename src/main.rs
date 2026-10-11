mod api;
mod assets;
mod camera;
mod collector;
mod commands;
mod config;
mod lan;
mod log_tail;
mod model;
mod notify;
mod moonraker;
#[cfg(test)]
mod next_tests;
mod operator;
mod printer_config;
mod probe;
mod remote;
mod qgl;
#[cfg(test)]
mod regression_tests;
mod resonance;
mod server;
mod store;
mod telemetry;
#[cfg(test)]
mod tests;
mod ws;
mod ws_source;

use config::Config;
use model::CollectorStatus;
use std::{
    fs::{self, File, OpenOptions},
    net::TcpListener,
    path::Path,
    sync::{Arc, Mutex},
    thread,
};
use store::Store;
pub type Result<T> = std::result::Result<T, String>;

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| format!("{flag} needs a value; run --help for examples"))
}

fn benchmark(cfg: &Config) -> Result<()> {
    let path = format!(
        "{}.benchmark-{}-{}.sqlite3",
        cfg.database,
        std::process::id(),
        model::now().to_bits()
    );
    let mut db = Store::open(&path, false)?;
    let mut parser = qgl::Parser::default();
    let mut total = 0.0;
    let data = model::demo(1.0, cfg);
    for i in 0..2000 {
        total += db
            .write(Some((i as f64 / 4.0, &data)), &[], &mut parser)?
            .ms;
    }
    println!("benchmark_samples=2000\nraw_sample_bytes={}\nmean_write_ms={:.6}\ntransaction_mode=WAL synchronous=FULL\nbenchmark_database={path}",data.to_string().len(),total/2000.0);
    drop(db);
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{path}{suffix}");
        if Path::new(&p).exists() {
            fs::remove_file(&p).map_err(|e|format!("benchmark finished but cannot remove temporary {p}: {e}; remove that temporary file manually"))?;
        }
    }
    Ok(())
}

fn start() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut config_path = None;
    let mut database = None;
    let mut port = None;
    let mut demo = false;
    let mut replay = None;
    let mut print_config = false;
    let mut bench = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = Some(value(&mut args, &arg)?),
            "--db" => database = Some(value(&mut args, &arg)?),
            "--port" => {
                port = Some(
                    value(&mut args, &arg)?
                        .parse::<u16>()
                        .map_err(|_| "--port needs a number from 1 to 65535".to_owned())?,
                )
            }
            "--demo" => demo = true,
            "--replay" => replay = Some(value(&mut args, &arg)?),
            "--print-config" => print_config = true,
            "--benchmark" => bench = true,
            "--help" | "-h" => {
                println!("Stella Scope - Klipper recorder with guarded local controls disabled by default\n\n  stella-scope [--config FILE] [--db FILE] [--port PORT]\n  stella-scope --demo [--db DEMO_FILE]\n  stella-scope --replay RECORDING.sqlite3 [--config FILE]\n  stella-scope --print-config\n  stella-scope --benchmark [--config FILE]\n\nSet controls_enabled only on a loopback live instance. Replay and demo never enable controls.");
                return Ok(());
            }
            _ => {
                return Err(format!(
                    "unknown option {arg}; use --help for supported options"
                ))
            }
        }
    }
    if demo && replay.is_some() {
        return Err("choose either --demo or --replay, not both".into());
    }
    if replay.is_some() && database.is_some() {
        return Err("--replay already supplies the recording path; omit --db".into());
    }
    let mut startup_warnings = Vec::new();
    let mut cfg = match config_path {
        Some(path) => Config::load(&path)?,
        None if Path::new("stella-scope.json").exists() => Config::load("stella-scope.json")?,
        None => {
            startup_warnings.push(
                "Using built-in defaults; use --print-config to create a JSON configuration file"
                    .into(),
            );
            Config::default()
        }
    };
    if let Some(db) = database {
        cfg.database = db;
    } else if demo {
        cfg.database = "stella-scope-demo.sqlite3".into();
    }
    if let Some(path) = &replay {
        cfg.database = path.clone();
    }
    if let Some(p) = port {
        cfg.port = p;
    }
    cfg.validate()?;
    if print_config {
        println!(
            "{}",
            serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if bench {
        return benchmark(&cfg);
    }
    let readonly = replay.is_some();
    let _writer_lock: Option<File> = if readonly {
        None
    } else {
        let path = format!("{}.lock", cfg.database);
        let file=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)
            .map_err(|e|format!("cannot open recorder lock {path}: {e}; create the parent directory and grant write permission"))?;
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
        let expected = if demo { "demo" } else { "live" };
        if origin
            .as_ref()
            .and_then(|v| v.as_str())
            .is_some_and(|v| v != expected)
            || (demo && origin.is_none() && db.bounds()?.0.is_some())
        {
            return Err(
                "refusing to mix demonstration and real recordings; select a new --db file".into(),
            );
        }
    }
    let (a, b, c) = db.counts()?;
    let (_, last) = db.bounds()?;
    let state = Arc::new(Mutex::new(CollectorStatus {
        mode: if readonly {
            "replay"
        } else if demo {
            "demo"
        } else {
            "live"
        }
        .into(),
        samples: a,
        events: b,
        qgl_rounds: c,
        last_success: last,
        degraded: startup_warnings,
        ..CollectorStatus::default()
    }));
    let listener=TcpListener::bind((cfg.bind.as_str(),cfg.port))
        .map_err(|e|format!("cannot listen on {}:{}: {e}; stop the other service or change bind/port in the JSON file",cfg.bind,cfg.port))?;
    if !readonly {
        let worker_cfg = cfg.clone();
        let worker_state = state.clone();
        thread::Builder::new().name("stella-collector".into()).stack_size(1024*1024).spawn(move||{
            let status=worker_state.clone();
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||collector::run(worker_cfg,demo,worker_state,db)));
            let error=match result {Ok(Err(e))=>e,Ok(Ok(()))=>"collector stopped unexpectedly; restart Stella Scope".into(),
                Err(_)=>"collector panicked; recorded data remains available, but restart Stella Scope to resume capture".into()};
            eprintln!("{error}");
            // CI exposed a temporary MutexGuard outliving the final closure-local Arc.
            if let Ok(mut s)=status.lock() {s.connected=false;s.last_error=Some(error);};
        }).map_err(|e|format!("cannot start collector thread: {e}; free memory and restart"))?;
    }
    println!(
        "Stella Scope {}: http://{}:{} | {} | {}",
        env!("CARGO_PKG_VERSION"),
        cfg.bind,
        cfg.port,
        if readonly {
            "replay only"
        } else if demo {
            "DEMO - synthetic data"
        } else {
            "read-only recording"
        },
        cfg.database
    );
    if !readonly {
        notify::init(&cfg);
    }
    // Phone access: a second listener that answers paired devices only (src/lan.rs).
    let lan = cfg.lan_port.filter(|_| !readonly).and_then(|port| match TcpListener::bind((cfg.lan_bind.as_str(), port)) {
        Ok(l) => {
            println!("Phone access for paired devices on {}:{port}", cfg.lan_bind);
            Some(l)
        }
        Err(e) => {
            eprintln!("phone access is off: cannot listen on {}:{port}: {e}", cfg.lan_bind);
            None
        }
    });
    if lan.is_some() {
        remote::start(&cfg);
    }
    if !readonly {
        remote::start_eye(&cfg);
    }
    server::serve(listener, lan, cfg, state)
}
fn main() {
    if let Err(error) = start() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
