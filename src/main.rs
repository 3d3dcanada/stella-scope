mod api;
mod assets;
mod camera;
mod camera_hub;
mod collector;
mod commands;
mod config;
mod demo;
mod discover;
mod driver;
mod drivers;
mod dry_run;
#[cfg(test)]
mod fakes;
mod farm;
mod feedback;
#[cfg(test)]
mod farm_tests;
mod fleet;
// Polish WO-06 (2026-10-18): the Rust reference of web/gcode-follow.js (the page does the mapping; scripts/gcode-follow-tests.mjs mirrors these numbers).
#[cfg(test)]
mod gcode_follow;
mod gcode_meta;
mod gzip;
// Finish-2 F07 (2026-10-19): the user guide inside the program.
mod help;
mod lan;
mod licence;
mod log_tail;
mod model;
mod moonraker;
#[cfg(test)]
mod next_tests;
mod notify;
mod operator;
mod plate;
mod plugs;
mod printer_config;
mod printer_models;
mod prusa_machines;
mod probe;
mod qgl;
mod queue;
mod queue_flow;
#[cfg(test)]
mod regression_tests;
mod remote;
mod resonance;
mod server;
mod sha256;
mod store;
mod telemetry;
mod timelapse;
#[cfg(test)]
mod tests;
mod ws;
mod ws_source;

use config::Config;
use std::{fs, net::TcpListener, path::Path};
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
                println!("Stella Scope - printer and farm recorder (Klipper, Prusa, FlashForge) with guarded local controls disabled by default\n\n  stella-scope [--config FILE] [--db FILE] [--port PORT]\n  stella-scope --demo [--db DEMO_FILE]\n  stella-scope --replay RECORDING.sqlite3 [--config FILE]\n  stella-scope --print-config\n  stella-scope --benchmark [--config FILE]\n\nSet controls_enabled only on a loopback live instance. Replay and demo never enable controls.");
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
    let used_config: Option<String> = config_path.clone().or_else(|| {
        Path::new("stella-scope.json")
            .exists()
            .then(|| "stella-scope.json".to_owned())
    });
    let database_override = database.clone();
    let mut cfg = match config_path {
        Some(path) => Config::load(&path)?,
        None if Path::new("stella-scope.json").exists() => Config::load("stella-scope.json")?,
        None => {
            startup_warnings.push(
                "Using built-in defaults; use --print-config to create a JSON configuration file"
                    .into(),
            );
            Config {
                unconfigured: true,
                ..Config::default()
            }
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
    // A demonstration and a saved recording have their own first printer; the placeholder is for a live service.
    if demo || readonly {
        cfg.unconfigured = false;
    }
    // A demonstration runs simulated printers instead of real ones: one demo Stella (a Voron 2.4 350)
    // by default, or whichever printers the config names with driver "demo". Nothing leaves the
    // machine and nothing is printed.
    if demo {
        if cfg.printers.is_empty() {
            cfg.printers.push(crate::config::PrinterEntry {
                id: "stella".into(),
                name: "Stella".into(),
                driver: "demo".into(),
                address: String::new(),
                credentials_ref: None,
                database: Some(cfg.database.clone()),
                model_asset: None,
                model: Some("Voron 2.4 350".into()),
                accent: Some("#04D9C4".into()),
                camera: None,
                bed_size_mm: Some([350.0, 350.0]),
                probe_offset_mm: None,
                nozzle_mm: None,
                slicer_model: None,
                material: None,
                cad_model: None,
                cad_calibration: None,
                cad_options: None,
                plug: None,
                demo_start_at: Some(0.3),
                demo_paused: false,
                demo_since: None,
            });
        } else {
            for first in cfg.printers.iter_mut().take(1) {
                first.driver = "demo".into();
                first.demo_start_at.get_or_insert(0.3);
                if first.model.is_none() {
                    first.model = Some("Voron 2.4 350".into());
                }
            }
        }
    }
    // The first printer is the service. A path given on the command line overrides the file's own
    // recording for it, as it always did.
    if let Some(first) = cfg.printers.first_mut() {
        if database_override.is_some() || replay.is_some() || demo {
            first.database = Some(cfg.database.clone());
        }
    }
    let listener=TcpListener::bind((cfg.bind.as_str(),cfg.port))
        .map_err(|e|format!("cannot listen on {}:{}: {e}; stop the other service or change bind/port in the JSON file",cfg.bind,cfg.port))?;
    // Every printer's recording, collector and control service (src/farm.rs). A configuration with no
    // `printers` is one printer, `stella`, with the recording it always had.
    let farm = farm::Farm::start(
        cfg.clone(),
        used_config.map(std::path::PathBuf::from),
        demo,
        readonly,
        startup_warnings,
    )?;
    let first = farm.first();
    // Finish-2 F09 (2026-10-19): the 3D models this build carries (Prusa's CORE One L and CORE One, the MK3S+ and the MINI) are put in the
    // person's library before the first page is asked for, so a fresh install draws them with nothing to download and no file to find.
    if !readonly {
        let carried = farm.library().seed_bundled();
        if !carried.is_empty() {
            println!("  3D models put in the library: {}", carried.join(", "));
        }
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
        first.cfg.database
    );
    for p in farm.list().iter().skip(1) {
        println!(
            "  printer {} ({}): {} | {}",
            p.id, p.cfg.driver, p.cfg.moonraker_url, p.cfg.database
        );
    }
    // Phone access: a second listener that answers paired devices only (src/lan.rs).
    let lan = cfg.lan_port.filter(|_| !readonly).and_then(|port| {
        match TcpListener::bind((cfg.lan_bind.as_str(), port)) {
            Ok(l) => {
                println!("Phone access for paired devices on {}:{port}", cfg.lan_bind);
                Some(l)
            }
            Err(e) => {
                eprintln!(
                    "phone access is off: cannot listen on {}:{port}: {e}",
                    cfg.lan_bind
                );
                None
            }
        }
    });
    farm.start_extras(lan.is_some(), readonly);
    server::serve(listener, lan, farm)
}
fn main() {
    if let Err(error) = start() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
