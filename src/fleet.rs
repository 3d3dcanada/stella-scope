//! The farm's own books (2026-10-13, the Watchtower floor): spools, fleet analytics, and a backup of
//! the farm that leaves every secret behind. Nothing here talks to a printer: usage and analytics
//! are read from each printer's own recording (`print_sessions`, a row per print the recorder saw
//! end), so a figure exists only where the printer reported it.

use crate::{farm::Farm, store::Store, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
    thread,
    time::Duration,
};

/// Filament is 1.75 mm everywhere in this farm; a figure for 2.85 mm machines would need its own field.
const FILAMENT_MM: f64 = 1.75;

/// g/cm³ by material, from the makers' data sheets (typical values). Unknown material: PLA's.
pub fn density(material: &str) -> f64 {
    let m = material.to_ascii_lowercase();
    if m.contains("petg") || m.contains("pet-g") {
        1.27
    } else if m.contains("pla") {
        1.24
    } else if m.contains("asa") {
        1.07
    } else if m.contains("abs") {
        1.04
    } else if m.contains("tpu") || m.contains("tpe") {
        1.21
    } else if m.contains("pc") || m.contains("polycarbonate") {
        1.20
    } else if m.contains("pa") || m.contains("nylon") {
        1.14
    } else {
        1.24
    }
}

/// Grams in a length of filament.
pub fn grams(mm: f64, material: &str) -> f64 {
    let r_cm = FILAMENT_MM / 20.0;
    std::f64::consts::PI * r_cm * r_cm * (mm / 10.0) * density(material)
}

fn lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn books_path(farm: &Farm) -> PathBuf {
    farm.config_dir().join("stella-scope-farm-books.json")
}

/// The stored books: `{"spools":[...],"next_id":N,"price_per_kg":X|null}`.
fn load(farm: &Farm) -> Value {
    let v: Value = fs::read_to_string(books_path(farm))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if v.is_object() {
        v
    } else {
        json!({"spools":[],"next_id":1,"price_per_kg":null})
    }
}

fn save(farm: &Farm, v: &Value) -> Result<()> {
    let path = books_path(farm);
    let temp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?,
    )
    .and_then(|_| fs::rename(&temp, &path))
    .map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot save the farm books at {}: {e}", path.display())
    })
}

/// Millimetres of filament a printer's finished prints used between two times, from its recording.
fn used_mm(farm: &Farm, printer: &str, from: f64, to: f64) -> f64 {
    let Some(p) = farm.get(printer) else {
        return 0.0;
    };
    let Ok(db) = Store::open(&p.cfg.database, true) else {
        return 0.0;
    };
    db.print_sessions(from, to, 100_000)
        .map(|rows| {
            rows.iter()
                .filter(|s| s.start_ts >= from)
                .filter_map(|s| s.filament_used)
                .filter(|mm| mm.is_finite() && *mm > 0.0)
                .sum()
        })
        .unwrap_or(0.0)
}

/// One spool with what is left on it worked out now.
fn spool_now(farm: &Farm, s: &Value, now: f64) -> Value {
    let material = s["material"].as_str().unwrap_or("");
    let mut used = s["used_before_g"].as_f64().unwrap_or(0.0);
    if let (Some(p), Some(at)) = (s["printer"].as_str(), s["assigned_at"].as_f64()) {
        used += grams(used_mm(farm, p, at, now), material);
    }
    let left = (s["grams_start"].as_f64().unwrap_or(0.0) - used).max(0.0);
    let mut out = s.clone();
    out["used_g"] = json!((used * 10.0).round() / 10.0);
    out["grams_left"] = json!((left * 10.0).round() / 10.0);
    out["low"] = json!(left <= s["low_at_g"].as_f64().unwrap_or(150.0));
    out["printer_name"] = s["printer"]
        .as_str()
        .and_then(|id| farm.get(id))
        .map_or(Value::Null, |p| json!(p.entry.name));
    out
}

/// `GET /api/spools`.
pub fn spools(farm: &Farm) -> Value {
    let books = load(farm);
    let now = crate::model::now();
    let list: Vec<Value> = books["spools"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s["archived"] != true)
                .map(|s| spool_now(farm, s, now))
                .collect()
        })
        .unwrap_or_default();
    json!({"spools":list,"price_per_kg":books["price_per_kg"],"filament_mm":FILAMENT_MM,
        "basis":"What is left: the spool's weight when added, less every finished print on its printer since it was put on (Stella Scope's filament figure, at the material's density)"})
}

fn text_field(v: &Value, key: &str, max: usize) -> Result<Option<String>> {
    match &v[key] {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().chars().count() <= max => Ok(Some(s.trim().to_owned())),
        Value::String(_) => Err(format!("{key} is at most {max} characters")),
        _ => Err(format!("{key} must be text")),
    }
}

fn number_field(v: &Value, key: &str, lo: f64, hi: f64) -> Result<Option<f64>> {
    match &v[key] {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .filter(|x| (lo..=hi).contains(x))
            .map(Some)
            .ok_or_else(|| format!("{key} must be between {lo} and {hi}")),
        _ => Err(format!("{key} must be a number")),
    }
}

fn colour_field(v: &Value) -> Result<Option<String>> {
    let Some(c) = text_field(v, "colour", 7)? else {
        return Ok(None);
    };
    if c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(Some(c.to_ascii_uppercase()))
    } else {
        Err("colour is #RRGGBB".into())
    }
}

/// Put a spool on a printer (or take it off with None): what it used so far is carried, the new
/// printer counts from now. A printer holds one spool; the one it had comes off.
fn assign(
    farm: &Farm,
    spools: &mut [Value],
    idx: usize,
    printer: Option<&str>,
    now: f64,
) -> Result<()> {
    if let Some(id) = printer {
        if farm.get(id).is_none() {
            return Err(format!("no printer called {id}"));
        }
        for (i, s) in spools.iter_mut().enumerate() {
            if i != idx && s["printer"] == id {
                settle(farm, s, now);
                s["printer"] = Value::Null;
                s["assigned_at"] = Value::Null;
            }
        }
    }
    let s = &mut spools[idx];
    settle(farm, s, now);
    s["printer"] = printer.map_or(Value::Null, |p| json!(p));
    s["assigned_at"] = printer.map_or(Value::Null, |_| json!(now));
    Ok(())
}

/// Fold the current assignment's usage into `used_before_g`.
fn settle(farm: &Farm, s: &mut Value, now: f64) {
    if let (Some(p), Some(at)) = (s["printer"].as_str(), s["assigned_at"].as_f64()) {
        let g = grams(
            used_mm(farm, p, at, now),
            s["material"].as_str().unwrap_or(""),
        );
        s["used_before_g"] = json!(s["used_before_g"].as_f64().unwrap_or(0.0) + g);
        s["assigned_at"] = json!(now);
    }
}

/// `POST /api/spools` (add), `PATCH /api/spools/<id>`, `DELETE /api/spools/<id>` (archived, kept
/// for the analytics), `POST /api/spools/price` {price_per_kg}.
pub fn spools_write(farm: &Farm, method: &str, id: Option<&str>, body: &[u8]) -> Result<Value> {
    let _g = lock().lock().map_err(|_| "books lock poisoned")?;
    let v: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body).map_err(|_| "send a JSON object")?
    };
    let mut books = load(farm);
    let now = crate::model::now();
    match (method, id) {
        ("POST", Some("price")) => {
            books["price_per_kg"] =
                number_field(&v, "price_per_kg", 0.0, 1000.0)?.map_or(Value::Null, |p| json!(p));
        }
        ("POST", None) => {
            let material = text_field(&v, "material", 24)?
                .filter(|m| !m.is_empty())
                .ok_or("material is needed (PETG, PLA, ASA...)")?;
            let total = number_field(&v, "grams_total", 1.0, 20_000.0)?.unwrap_or(1000.0);
            let start = number_field(&v, "grams_left", 0.0, 20_000.0)?.unwrap_or(total);
            let next = books["next_id"].as_u64().unwrap_or(1);
            let spool = json!({
                "id":next,"material":material,
                "brand":text_field(&v, "brand", 40)?.unwrap_or_default(),
                "name":text_field(&v, "name", 60)?.unwrap_or_default(),
                "colour":colour_field(&v)?.unwrap_or_else(|| "#9AA4B2".into()),
                "grams_total":total,"grams_start":start.min(total),"used_before_g":0.0,
                "price_per_kg":number_field(&v, "price_per_kg", 0.0, 1000.0)?,
                "low_at_g":number_field(&v, "low_at_g", 0.0, 5000.0)?.unwrap_or(150.0),
                "printer":null,"assigned_at":null,"added":now,"archived":false,"low_told":false,
            });
            let list = books["spools"]
                .as_array_mut()
                .ok_or("the farm books are damaged: spools is not a list")?;
            list.push(spool);
            let idx = list.len() - 1;
            if let Some(p) = text_field(&v, "printer", 32)?.filter(|p| !p.is_empty()) {
                assign(farm, list, idx, Some(&p), now)?;
            }
            books["next_id"] = json!(next + 1);
        }
        (m @ ("PATCH" | "DELETE"), Some(sid)) => {
            let sid: u64 = sid.parse().map_err(|_| "a spool id is a number")?;
            let list = books["spools"]
                .as_array_mut()
                .ok_or("the farm books are damaged: spools is not a list")?;
            let idx = list
                .iter()
                .position(|s| s["id"] == sid && s["archived"] != true)
                .ok_or_else(|| format!("no spool {sid}"))?;
            if m == "DELETE" {
                assign(farm, list, idx, None, now)?;
                list[idx]["archived"] = json!(true);
            } else {
                for key in v.as_object().ok_or("send a JSON object")?.keys() {
                    if ![
                        "material",
                        "brand",
                        "name",
                        "colour",
                        "grams_left",
                        "grams_total",
                        "price_per_kg",
                        "low_at_g",
                        "printer",
                    ]
                    .contains(&key.as_str())
                    {
                        return Err(format!("{key} cannot be changed on a spool"));
                    }
                }
                for key in ["material", "brand", "name"] {
                    if let Some(t) = text_field(&v, key, 60)? {
                        list[idx][key] = json!(t);
                    }
                }
                if let Some(c) = colour_field(&v)? {
                    list[idx]["colour"] = json!(c);
                }
                for (key, hi) in [
                    ("price_per_kg", 1000.0),
                    ("low_at_g", 5000.0),
                    ("grams_total", 20_000.0),
                ] {
                    if let Some(n) = number_field(&v, key, 0.0, hi)? {
                        list[idx][key] = json!(n);
                    }
                }
                // Weighed it: what is left now is the new starting point.
                if let Some(left) = number_field(&v, "grams_left", 0.0, 20_000.0)? {
                    let s = &mut list[idx];
                    s["grams_start"] = json!(left);
                    s["used_before_g"] = json!(0.0);
                    if s["printer"].is_string() {
                        s["assigned_at"] = json!(now);
                    }
                    s["low_told"] = json!(false);
                }
                if v.get("printer").is_some() {
                    let p = text_field(&v, "printer", 32)?.filter(|p| !p.is_empty());
                    assign(farm, list, idx, p.as_deref(), now)?;
                }
            }
        }
        _ => return Err("use GET, POST /api/spools, PATCH or DELETE /api/spools/<id>".into()),
    }
    save(farm, &books)?;
    Ok(spools(farm))
}

/// The spool on each printer, for the tiles and printer pages: `{printer id: {material, colour, ...}}`.
pub fn loaded(farm: &Farm) -> HashMap<String, Value> {
    let books = load(farm);
    let now = crate::model::now();
    books["spools"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s["archived"] != true)
                .filter_map(|s| {
                    let p = s["printer"].as_str()?.to_owned();
                    let n = spool_now(farm, s, now);
                    Some((p, json!({"id":n["id"],"material":n["material"],"brand":n["brand"],"name":n["name"],"colour":n["colour"],"grams_left":n["grams_left"],"low":n["low"]})))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A spool that runs low says so once (alarm feed, pop-up, and any channel that is on), and again
/// only after it is weighed or replaced.
fn check_low(farm: &Farm) {
    let Ok(_g) = lock().lock() else { return };
    let mut books = load(farm);
    let now = crate::model::now();
    let mut changed = false;
    if let Some(list) = books["spools"].as_array_mut() {
        for s in list.iter_mut().filter(|s| s["archived"] != true) {
            let n = spool_now(farm, s, now);
            let low = n["low"] == true;
            if low && s["low_told"] != true {
                let who = n["printer_name"]
                    .as_str()
                    .map(|p| format!(" on {p}"))
                    .unwrap_or_default();
                let what = [
                    n["brand"].as_str(),
                    n["material"].as_str(),
                    n["name"].as_str(),
                ]
                .iter()
                .flatten()
                .filter(|t| !t.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
                crate::notify::event(
                    "spool_low",
                    s["printer"].as_str().unwrap_or(""),
                    n["printer_name"].as_str().unwrap_or(""),
                    &format!(
                        "Spool {what}{who} is low: {:.0} g left.",
                        n["grams_left"].as_f64().unwrap_or(0.0)
                    ),
                );
                s["low_told"] = json!(true);
                changed = true;
            } else if !low && s["low_told"] == true {
                s["low_told"] = json!(false);
                changed = true;
            }
        }
    }
    if changed {
        let _ = save(farm, &books);
    }
}

/// What a printer's change of state says to the farm, if anything: `Some("offline")` or `Some("online")`.
/// U03 b (2026-10-20: "it said back online, the wall did not change"): a Klipper printer that is switched on answers Moonraker for the half
/// minute Klipper takes to start, and in that time the wall (rightly) says "Needs attention", but the feed said "back online" the moment it was
/// no longer offline. A printer is back online when it has left offline AND is in a state it can work from; while it answers but is in error it
/// stays lost, and says so once, when it is ready. `lost` holds the printers that have gone offline and not yet come back.
fn state_note(lost: &mut HashSet<String>, id: &str, before: Option<&str>, now: &str) -> Option<&'static str> {
    if now == "offline" {
        let first = lost.insert(id.to_owned());
        // a print lost mid-way is told by the recorder already; the first look at a printer that is already off is not news either
        return (first && before.is_some_and(|b| b != "offline" && !matches!(b, "printing" | "paused" | "heating")))
            .then_some("offline");
    }
    if now != "error" && lost.remove(id) {
        return Some("online");
    }
    None
}

/// Background: spools running low, and printers going off or coming back (a pop-up and the feed
/// only; the phone channels already hear about a printer lost mid-print from the recorder).
pub fn spawn_watch(farm: &Arc<Farm>) {
    let weak: Weak<Farm> = Arc::downgrade(farm);
    let _ = thread::Builder::new()
        .name("fleet-watch".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut prev: HashMap<String, String> = HashMap::new();
            let mut lost: HashSet<String> = HashSet::new();
            let mut tick = 0_u64;
            loop {
                thread::sleep(Duration::from_secs(5));
                let Some(farm) = weak.upgrade() else { break };
                for p in farm.summary()["printers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    let (Some(id), Some(state)) = (p["id"].as_str(), p["state"].as_str()) else {
                        continue;
                    };
                    let name = p["name"].as_str().unwrap_or(id);
                    let before = prev.insert(id.to_owned(), state.to_owned());
                    match state_note(&mut lost, id, before.as_deref(), state) {
                        Some("offline") => crate::notify::note(
                            "offline",
                            id,
                            name,
                            &format!("{name} went offline."),
                        ),
                        Some(_) => crate::notify::note(
                            "online",
                            id,
                            name,
                            &format!("{name} is back online."),
                        ),
                        None => {}
                    }
                }
                if tick % 12 == 0 {
                    check_low(&farm);
                }
                // smart plugs: power and on/off every 10 s, one short local request each
                if tick % 2 == 0 {
                    for p in farm.list() {
                        if let Some(spec) = p.entry.plug.as_deref() {
                            crate::plugs::refresh(&p.id, spec);
                        }
                    }
                }
                tick += 1;
            }
        });
}

/// `GET /api/analytics?days=N`: per printer and in total, from the recordings only.
pub fn analytics(farm: &Farm, days: f64) -> Value {
    let days = days.clamp(1.0, 365.0);
    let now = crate::model::now();
    let from = now - days * 86_400.0;
    let books = load(farm);
    let default_price = books["price_per_kg"].as_f64();
    let loaded = loaded(farm);
    let spool_price = |id: &str| {
        books["spools"].as_array().and_then(|a| {
            a.iter()
                .find(|s| s["printer"] == id && s["archived"] != true)
                .and_then(|s| s["price_per_kg"].as_f64())
        })
    };
    let day_count = days.ceil() as usize;
    let mut daily_g = vec![0.0_f64; day_count];
    let mut daily_prints = vec![0_u64; day_count];
    let mut rows = Vec::new();
    let mut total = json!({"prints":0,"completed":0,"failed":0,"cancelled":0,"print_h":0.0,"grams":0.0,"cost":0.0,"cost_known":true});
    for p in farm.visible() {
        let material = loaded
            .get(&p.id)
            .and_then(|s| s["material"].as_str().map(str::to_owned))
            .or_else(|| p.entry.material.clone())
            .unwrap_or_default();
        let sessions = Store::open(&p.cfg.database, true)
            .ok()
            .and_then(|db| db.print_sessions(from, now, 100_000).ok())
            .unwrap_or_default();
        let (mut done, mut failed, mut cancelled, mut secs, mut g) =
            (0_u64, 0_u64, 0_u64, 0.0_f64, 0.0_f64);
        for s in &sessions {
            match s.result.as_str() {
                "complete" => done += 1,
                "error" => failed += 1,
                "cancelled" => cancelled += 1,
                _ => {}
            }
            // The hours are the print's own duration (the one the history shows), clipped to the window only when the print began before it.
            // Finish-2 F06: a print the recorder joined half-way (the service started during it) counted only the minutes the recorder saw, so the
            // analytics said 8 minutes for the print the history lists as 54.
            let d = s.duration_s.unwrap_or((s.end_ts - s.start_ts).max(0.0)).max(0.0);
            let inside = if s.start_ts >= from { d } else { (s.end_ts - from).max(0.0).min(d) };
            secs += inside;
            let grams_here = s
                .filament_used
                .filter(|m| m.is_finite() && *m > 0.0)
                .map_or(0.0, |mm| grams(mm, &material));
            g += grams_here;
            let day = (((s.end_ts - from) / 86_400.0).floor() as usize).min(day_count - 1);
            daily_g[day] += grams_here;
            daily_prints[day] += 1;
        }
        let ended = done + failed + cancelled;
        let price = spool_price(&p.id).or(default_price);
        let cost = price.map(|kg| g / 1000.0 * kg);
        let window_h = days * 24.0;
        rows.push(json!({
            "id":p.id,"name":p.entry.name,"accent":p.entry.accent,"material":material,
            "prints":sessions.len(),"completed":done,"failed":failed,"cancelled":cancelled,
            "success_rate":(ended > 0).then(|| done as f64 / ended as f64),
            "print_h":secs / 3600.0,"idle_h":(window_h - secs / 3600.0).max(0.0),
            "utilisation":(secs / 3600.0 / window_h).min(1.0),
            "grams":g,"cost":cost,"price_per_kg":price,
        }));
        total["prints"] = json!(total["prints"].as_u64().unwrap_or(0) + sessions.len() as u64);
        total["completed"] = json!(total["completed"].as_u64().unwrap_or(0) + done);
        total["failed"] = json!(total["failed"].as_u64().unwrap_or(0) + failed);
        total["cancelled"] = json!(total["cancelled"].as_u64().unwrap_or(0) + cancelled);
        total["print_h"] = json!(total["print_h"].as_f64().unwrap_or(0.0) + secs / 3600.0);
        total["grams"] = json!(total["grams"].as_f64().unwrap_or(0.0) + g);
        match cost {
            Some(c) => total["cost"] = json!(total["cost"].as_f64().unwrap_or(0.0) + c),
            None if g > 0.0 => total["cost_known"] = json!(false),
            None => {}
        }
    }
    let (c, f, x) = (
        total["completed"].as_u64().unwrap_or(0),
        total["failed"].as_u64().unwrap_or(0),
        total["cancelled"].as_u64().unwrap_or(0),
    );
    total["success_rate"] = if c + f + x > 0 {
        json!(c as f64 / (c + f + x) as f64)
    } else {
        Value::Null
    };
    let printers = farm.visible().len().max(1) as f64;
    total["utilisation"] =
        json!((total["print_h"].as_f64().unwrap_or(0.0) / (days * 24.0 * printers)).min(1.0));
    let start_day = (from / 86_400.0).floor() * 86_400.0;
    json!({
        "days":days,"from":from,"to":now,"printers":rows,"total":total,
        "daily":(0..day_count).map(|i| json!({"day":start_day + i as f64 * 86_400.0,"grams":daily_g[i],"prints":daily_prints[i]})).collect::<Vec<_>>(),
        "basis":"Every print that ended in the window; grams from the printer's filament figure at the loaded material's density; cost from the spool's price, else the farm price",
    })
}

/// `GET /api/farm/backup`: the printer list, the farm books and the alert choices. No password,
/// API key, check code, webhook or WhatsApp key, and no recording.
pub fn backup(farm: &Farm) -> Value {
    // The placeholder is not a printer: a backup that carried it restored a ghost "Stella" with no address on the next computer (finish-2 F06).
    let printers: Vec<Value> = farm
        .visible()
        .iter()
        .map(|p| {
            let mut e = serde_json::to_value(&p.entry).unwrap_or(Value::Null);
            if let Some(o) = e.as_object_mut() {
                o.remove("credentials_ref");
                o.remove("database");
                o.insert(
                    "needs_login".into(),
                    json!(p.entry.credentials_ref.is_some()),
                );
            }
            e
        })
        .collect();
    let notify = crate::notify::settings(&farm.first().cfg);
    json!({
        "format":"stella-scope-farm-backup","version":1,"made_at":crate::model::now(),
        "app_version":env!("CARGO_PKG_VERSION"),
        "printers":printers,
        "books":load(farm),
        "alerts":{"events":notify["events"],"ntfy":notify["ntfy"]["url"]},
        "left_out":"logins, keys, check codes, webhook and WhatsApp keys, and the recordings",
    })
}

/// `POST /api/farm/restore`: add the printers this farm does not have yet (by id) and the spools it
/// does not have yet. Nothing that exists is changed. Printers that need a login are named so it can
/// be typed in their settings.
pub fn restore(farm: &Farm, body: &[u8]) -> Result<Value> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| "that is not a Stella Scope farm backup")?;
    if v["format"] != "stella-scope-farm-backup" {
        return Err("that is not a Stella Scope farm backup".into());
    }
    let (mut added, mut skipped, mut failed, mut login) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in v["printers"].as_array().cloned().unwrap_or_default() {
        let needs_login = p["needs_login"] == true;
        let mut clean = p.clone();
        if let Some(o) = clean.as_object_mut() {
            o.remove("needs_login");
            o.remove("credentials_ref");
            o.remove("database");
        }
        let entry: crate::config::PrinterEntry = match serde_json::from_value(clean) {
            Ok(e) => e,
            Err(e) => {
                failed.push(json!({"id":p["id"],"error":format!("not a printer entry: {e}")}));
                continue;
            }
        };
        if farm.get(&entry.id).is_some() {
            skipped.push(json!(entry.id));
            continue;
        }
        let id = entry.id.clone();
        match farm.add(entry, None) {
            Ok(_) => {
                added.push(json!(id));
                if needs_login {
                    login.push(json!(id));
                }
            }
            Err(e) => failed.push(json!({"id":id,"error":e})),
        }
    }
    let mut spools_added = 0;
    {
        let _g = lock().lock().map_err(|_| "books lock poisoned")?;
        let mut books = load(farm);
        let have: Vec<Value> = books["spools"].as_array().cloned().unwrap_or_default();
        let mut next = books["next_id"].as_u64().unwrap_or(1);
        let mut list = have.clone();
        for s in v["books"]["spools"].as_array().cloned().unwrap_or_default() {
            let same = have.iter().any(|h| {
                h["added"] == s["added"] && h["material"] == s["material"] && h["name"] == s["name"]
            });
            if same || s["archived"] == true {
                continue;
            }
            let mut s = s.clone();
            s["id"] = json!(next);
            next += 1;
            // a printer this farm does not have holds nothing
            if s["printer"].as_str().is_some_and(|p| farm.get(p).is_none()) {
                s["printer"] = Value::Null;
                s["assigned_at"] = Value::Null;
            }
            list.push(s);
            spools_added += 1;
        }
        books["spools"] = json!(list);
        books["next_id"] = json!(next);
        if books["price_per_kg"].is_null() {
            books["price_per_kg"] = v["books"]["price_per_kg"].clone();
        }
        save(farm, &books)?;
    }
    Ok(
        json!({"added":added,"skipped":skipped,"failed":failed,"needs_login":login,"spools_added":spools_added}),
    )
}

// ---- smart plugs ------------------------------------------------------------------------------

/// `POST /api/plugs/<printer>` {on, force?}: switch a printer's plug. Cutting the power of a printer
/// that is working is refused unless `force` says the person confirmed it.
pub fn plug_switch(farm: &Farm, id: &str, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "send {\"on\":true}")?;
    let on = v["on"].as_bool().ok_or("on is true or false")?;
    let p = farm
        .get(id)
        .ok_or_else(|| format!("no printer called {id}"))?;
    let spec = p
        .entry
        .plug
        .clone()
        .ok_or("this printer has no smart plug; set one in its settings")?;
    if !on && v["force"] != true {
        let state = farm.summary()["printers"]
            .as_array()
            .and_then(|a| {
                a.iter()
                    .find(|r| r["id"] == id)
                    .and_then(|r| r["state"].as_str().map(str::to_owned))
            })
            .unwrap_or_default();
        if matches!(state.as_str(), "printing" | "paused" | "heating") {
            return Err(format!(
                "{} is {state}; turning its power off ends the print. Confirm to do it anyway.",
                p.entry.name
            ));
        }
    }
    let r = crate::plugs::set(&spec, on)?;
    crate::plugs::remember(id, &spec, &r);
    crate::notify::note(
        "plug",
        id,
        &p.entry.name,
        &format!(
            "{} power turned {}.",
            p.entry.name,
            if on { "on" } else { "off" }
        ),
    );
    Ok(json!({"printer":id,"reading":r.to_json()}))
}

// ---- slicer upload key -------------------------------------------------------------------------
// A slicer's "OctoPrint" printer stores a key. By default that is its own upload key, which can only
// drop a file in the queue (the plate check and a person still stand between it and a printer). The
// owner can also let the farm control key work there, or turn slicer uploads off (2026-10-13: "both
// ways, a person can switch that in settings").

fn slicer_path(farm: &Farm) -> PathBuf {
    farm.config_dir().join("stella-scope-slicer.json")
}

fn new_key() -> String {
    let mut b = [0_u8; 20];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn slicer_cache() -> &'static Mutex<Option<Value>> {
    static C: OnceLock<Mutex<Option<Value>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn write_private(path: &PathBuf, v: &Value) -> Result<()> {
    use std::io::Write;
    let temp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(&temp)
        .and_then(|mut f| {
            f.write_all(v.to_string().as_bytes())
                .and_then(|_| f.sync_all())
        })
        .and_then(|_| fs::rename(&temp, path))
        .map_err(|e| {
            let _ = fs::remove_file(&temp);
            format!("cannot save the slicer settings: {e}")
        })
}

/// The stored settings, made (with a fresh key) the first time anyone asks.
fn slicer(farm: &Farm) -> Value {
    let mut c = slicer_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = c.as_ref() {
        return v.clone();
    }
    let path = slicer_path(farm);
    let v = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v["key"].as_str().is_some_and(|k| k.len() >= 32))
        .unwrap_or_else(|| {
            let v = json!({"enabled":true,"key":new_key(),"accept_farm_key":false});
            let _ = write_private(&path, &v);
            v
        });
    *c = Some(v.clone());
    v
}

/// The keys a slicer upload is taken with right now (none when uploads are off).
pub fn slicer_keys(farm: &Farm) -> Vec<String> {
    let v = slicer(farm);
    if v["enabled"] == false {
        return Vec::new();
    }
    let mut keys = vec![v["key"].as_str().unwrap_or_default().to_owned()];
    if v["accept_farm_key"] == true {
        if let Some(t) = &farm.token {
            keys.push(t.clone());
        }
    }
    keys.retain(|k| !k.is_empty());
    keys
}

/// `GET /api/slicer` (desktop only): what a slicer needs to be set up.
pub fn slicer_settings(farm: &Farm) -> Value {
    let v = slicer(farm);
    json!({"enabled":v["enabled"] != false,"key":v["key"],"accept_farm_key":v["accept_farm_key"] == true,
        "address":format!("http://{}", farm.base.host()),"host_type":"OctoPrint"})
}

/// `POST /api/slicer` {enabled?, accept_farm_key?, new_key?}.
pub fn slicer_write(farm: &Farm, body: &[u8]) -> Result<Value> {
    let b: Value = serde_json::from_slice(body).map_err(|_| "send a JSON object")?;
    let mut v = slicer(farm);
    for k in ["enabled", "accept_farm_key"] {
        match &b[k] {
            Value::Null => {}
            Value::Bool(x) => v[k] = json!(x),
            _ => return Err(format!("{k} is true or false")),
        }
    }
    if b["new_key"] == true {
        v["key"] = json!(new_key());
    }
    write_private(&slicer_path(farm), &v)?;
    *slicer_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
    Ok(slicer_settings(farm))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U03 b (2026-10-20): "back online" is told when the printer can work, not when it merely answers while Klipper is still starting.
    #[test]
    fn a_printer_is_back_online_when_it_is_ready_not_when_it_merely_answers() {
        let mut lost = HashSet::new();
        let step = |l: &mut HashSet<String>, before: Option<&str>, now: &str| state_note(l, "a", before, now);
        assert_eq!(step(&mut lost, Some("idle"), "offline"), Some("offline"), "an idle printer going dark is news");
        assert_eq!(step(&mut lost, Some("offline"), "offline"), None, "once");
        // Klipper starting: Moonraker answers, Klipper is not ready (the wall says Needs attention)
        assert_eq!(step(&mut lost, Some("offline"), "error"), None, "answering in error is not back online");
        assert_eq!(step(&mut lost, Some("error"), "error"), None);
        assert_eq!(step(&mut lost, Some("error"), "idle"), Some("online"), "ready now: say so, once");
        assert_eq!(step(&mut lost, Some("idle"), "idle"), None);
        // straight back, no error in between
        step(&mut lost, Some("idle"), "offline");
        assert_eq!(step(&mut lost, Some("offline"), "heating"), Some("online"));
        // an error that was never an outage is not "back online" when it clears
        assert_eq!(step(&mut lost, Some("idle"), "error"), None);
        assert_eq!(step(&mut lost, Some("error"), "idle"), None);
        // lost mid-print: the recorder has told it, the feed does not repeat it, but the return is still news
        assert_eq!(step(&mut lost, Some("printing"), "offline"), None);
        assert_eq!(step(&mut lost, Some("offline"), "printing"), Some("online"));
        // a printer that is already off when Stella Scope starts is not "went offline", and its return is news
        assert_eq!(step(&mut lost, None, "offline"), None);
        assert_eq!(step(&mut lost, Some("offline"), "idle"), Some("online"));
    }

    #[test]
    fn a_metre_of_petg_weighs_about_three_grams() {
        // 1.75 mm: 2.405 cm³ per metre; PETG 1.27 g/cm³ is 3.05 g
        let g = grams(1000.0, "PETG");
        assert!((g - 3.054).abs() < 0.01, "{g}");
        assert!(grams(1000.0, "PLA") < g && grams(1000.0, "ASA") < grams(1000.0, "PLA"));
        assert_eq!(density("Sunlu PETG"), 1.27);
        assert_eq!(density("mystery"), 1.24);
    }
}
