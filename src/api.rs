pub mod history;
pub mod investigation;
pub mod print;
pub mod signals;
pub mod workspace;
use crate::{
    config::Config,
    model::{self, CollectorStatus},
    store::{sample_json, Store},
    Result,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct Response {
    pub code: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
}
impl Response {
    pub fn json(code: u16, value: Value) -> Self {
        Self {
            code,
            mime: "application/json; charset=utf-8",
            body: value.to_string().into_bytes(),
        }
    }
    pub fn text(code: u16, mime: &'static str, body: &str) -> Self {
        Self {
            code,
            mime,
            body: body.as_bytes().to_vec(),
        }
    }
}

pub fn asset(method: &str, path: &str) -> Option<Response> {
    if method != "GET" {
        return Some(Response::json(
            405,
            json!({"error":"read-only service; only GET is allowed"}),
        ));
    }
    match path {
        "/classic-hud" | "/service" | "/service/machine" | "/service/signals" | "/service/electronics" | "/service/parts" | "/service/diagnostics" | "/service/maintenance" | "/service/timeline" | "/service/thermal" | "/service/leveling" | "/service/assembly" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/service.html"))),
        // Stella Scope NEXT: the rebuilt HUD, beside the old one until the user approves the switch.
        // NEXT is the app. "/" opens it; the previous HUD stays at /classic-hud.
        "/" | "/index.html" | "/next" | "/next/" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/next.html"))),
        "/next.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/next.css"))),
        "/next.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next.js"))),
        "/next-ui.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next-ui.js"))),
        "/next/lab" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/next-lab.html"))),
        "/next-lab.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next-lab.js"))),
        "/next-lab.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/next-lab.css"))),
        "/next-pages.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next-pages.js"))),
        "/pair" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/pair.html"))),
        "/pair.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/pair.js"))),
        "/manifest.webmanifest" => Some(Response::text(200,"application/manifest+json",include_str!("../web/manifest.webmanifest"))),
        "/next-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next-core.js"))),
        "/next-gesture.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/next-gesture.js"))),
        "/next-icons.svg" => Some(Response::text(200,"image/svg+xml",include_str!("../web/next-icons.svg"))),
        "/service.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service.js"))),
        "/service-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service-core.js"))),
        "/service-scene.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service-scene.js"))),
        "/service-dials.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service-dials.js"))),
        "/hud.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/hud.js"))),
        "/hud-scene.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/hud-scene.js"))),
        "/hud-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/hud-core.js"))),
        "/hud-graphs.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/hud-graphs.js"))),
        "/hud-gcode.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/hud-gcode.js"))),
        "/hud.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/hud.css"))),
        "/vendor/STLLoader.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/vendor/STLLoader.js"))),
        "/assembly.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/assembly.js"))),
        "/assembly.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/assembly.css"))),
        "/vendor/three.module.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/vendor/three.module.js"))),
        "/vendor/three.core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/vendor/three.core.js"))),
        "/vendor/OrbitControls.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/vendor/OrbitControls.js"))),
        // Self-hosted variable fonts (Geist + Geist Mono, latin subset) — no CDN, works offline.
        "/fonts/geist.woff2" => Some(Response{code:200,mime:"font/woff2",body:include_bytes!("../web/fonts/geist.woff2").to_vec()}),
        "/fonts/archivo.woff2" => Some(Response{code:200,mime:"font/woff2",body:include_bytes!("../web/fonts/archivo.woff2").to_vec()}),
        "/fonts/plex-mono.woff2" => Some(Response{code:200,mime:"font/woff2",body:include_bytes!("../web/fonts/plex-mono.woff2").to_vec()}),
        "/fonts/plex-mono-bold.woff2" => Some(Response{code:200,mime:"font/woff2",body:include_bytes!("../web/fonts/plex-mono-bold.woff2").to_vec()}),
        "/fonts/geist-mono.woff2" => Some(Response{code:200,mime:"font/woff2",body:include_bytes!("../web/fonts/geist-mono.woff2").to_vec()}),
        "/service.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/service.css"))),
        "/service-catalog.json" => Some(Response::text(200,"application/json; charset=utf-8",include_str!("../web/service-catalog.json"))),
        // The 3D3D wave mark, as sourced from the brand assets — never recoloured.
        "/logo.svg" => Some(Response::text(200,"image/svg+xml",include_str!("../web/logo.svg"))),

        "/workspace" | "/workspace.html" | "/workspace/printer" | "/workspace/motion" | "/workspace/motors" | "/workspace/thermal" | "/workspace/leveling" | "/workspace/events" | "/workspace/recording" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/workspace.html"))),
        "/workspace.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace.js"))),
        "/workspace-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace-core.js"))),
        "/workspace-scene.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace-scene.js"))),
        "/workspace.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/workspace.css"))),
        "/classic" => Some(Response::text(200,"text/html; charset=utf-8",&include_str!("../web/index.html").replace("<div class=\"header-right\">","<div class=\"header-right\"><a class=\"pill\" href=\"/investigate\">Investigate a fault</a>"))),
        "/app.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/app.js"))),
        "/style.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/style.css"))),
        "/investigate" | "/investigate.html" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/investigate.html"))),
        "/investigate.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/investigate.js"))),
        "/investigate.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/investigate.css"))),
        "/favicon.ico" => Some(Response::text(204,"image/x-icon","")),
        _ => None,
    }
}

fn decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err("incomplete URL escape; use a complete percent-encoded query".into());
            }
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            match (h, l) {
                (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                _ => return Err("invalid URL escape; use two hexadecimal digits after %".into()),
            }
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "query is not UTF-8; encode the URL as UTF-8".into())
}

fn query(target: &str) -> Result<(&str, BTreeMap<String, String>)> {
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let mut out = BTreeMap::new();
    for item in q.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = item
            .split_once('=')
            .ok_or("query values need key=value form")?;
        let k = decode(k)?;
        let v = decode(v)?;
        if out.insert(k.clone(), v).is_some() {
            return Err(format!(
                "duplicate query parameter {k}; provide it only once"
            ));
        }
    }
    Ok((path, out))
}
fn number(q: &BTreeMap<String, String>, key: &str, default: Option<f64>) -> Result<f64> {
    let n = match q.get(key) {
        Some(s) => s
            .parse::<f64>()
            .map_err(|_| format!("{key} must be a Unix timestamp in seconds"))?,
        None => {
            default.ok_or_else(|| format!("missing {key}; provide a Unix timestamp in seconds"))?
        }
    };
    if !n.is_finite() || n < 0.0 {
        return Err(format!("{key} must be a finite non-negative timestamp"));
    }
    Ok(n)
}

pub fn handle(
    method: &str,
    target: &str,
    db: &Store,
    cfg: &Config,
    status: &CollectorStatus,
) -> Response {
    let path = target.split('?').next().unwrap_or(target);
    if let Some(response) = asset(method, path) {
        return response;
    }
    if path == "/api/signal-history" {
        return signals::handle(method, target, db, cfg, status);
    }
    if path == "/api/assets" || path.starts_with("/api/assets/") {
        return crate::assets::handle(method, path, &cfg.database);
    }
    // A replay instance shows a recording and never asks the printer for a print file. The recording
    // keeps each print's slicer metadata and Orca settings (print_context), not its G-code or
    // thumbnails, so the answer is a plain "not stored" and Moonraker is not contacted.
    if status.mode == "replay" && ["/api/print/gcode", "/api/print/thumbnail", "/api/print/file"].contains(&path) {
        let what = if path.ends_with("gcode") { "G-code" } else if path.ends_with("file") { "File details" } else { "Thumbnail" };
        return Response::json(
            404,
            json!({"error":format!("{what} not stored in the recording"),"stored":false}),
        );
    }
    if path == "/api/print/gcode" {
        return print::handle(method, target, cfg);
    }
    if path == "/api/print/thumbnail" {
        return print::thumbnail_handle(method, target, cfg);
    }
    if path == "/api/camera" {
        if status.mode == "replay" {
            return Response::json(404, json!({"error":"a saved recording has no camera"}));
        }
        return match crate::camera::list(cfg) {
            Ok(v) => Response::json(200, v),
            Err(e) => Response::json(502, json!({"error":e})),
        };
    }
    if path == "/api/print/history" {
        return print::history_handle(method, target, cfg, db, status.mode != "replay");
    }
    if path == "/api/print/mesh-profiles" {
        return print::mesh_profiles_handle(method, cfg, status.mode != "replay");
    }
    if path == "/api/print/file" {
        return print::file_handle(method, target, cfg);
    }
    if path == "/api/workspace" {
        return workspace::handle(method, target, db, cfg, status);
    }
    // L1: resonance runs. Results live in their own SQLite file beside the recording, so the
    // handler takes the config rather than the recorder's read-only Store.
    if path == "/api/resonance" || path.starts_with("/api/resonance/") {
        return crate::resonance::handle(method, path, cfg);
    }
    // L7: histories, each row carrying evidence pointers /api/at?ts= can resolve.
    if path.starts_with("/api/history/") {
        return history::handle(method, target, db, cfg, status);
    }
    if path == "/api/eye" {
        return Response::json(200, crate::remote::eye(cfg));
    }
    if path == "/api/tick" {
        return workspace::tick_handle(method, db, cfg, status);
    }
    if path == "/api/trends" {
        return workspace::trends_handle(method, target, db, cfg, status);
    }
    if ["/api/investigation", "/api/export"].contains(&path) {
        return investigation::handle(method, target, db, cfg, status);
    }
    if ![
        "/api/meta",
        "/api/live",
        "/api/at",
        "/api/series",
        "/api/events",
        "/api/qgl",
    ]
    .contains(&path)
    {
        return Response::json(
            404,
            json!({"error":"page not found; open / for Stella Scope"}),
        );
    }
    let (path, q) = match query(target) {
        Ok(v) => v,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let clock = model::now();
    let get_window = || -> Result<(f64, f64)> {
        let from = number(&q, "from", Some(0.0))?;
        let to = number(&q, "to", Some(clock))?;
        if from > to {
            return Err("from must not be later than to; reverse the time window".into());
        }
        Ok((from, to))
    };
    let window = match get_window() {
        Ok(w) => w,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let cursor = if path == "/api/at" {
        match number(&q, "ts", None) {
            Ok(t) => t,
            Err(e) => return Response::json(400, json!({"error":e})),
        }
    } else {
        clock
    };
    let points = match q.get("points") {
        None => 600.min(cfg.series_max_points),
        Some(s) => match s.parse::<usize>() {
            Ok(n) if (2..=cfg.series_max_points).contains(&n) => n,
            _ => {
                return Response::json(
                    400,
                    json!({"error":format!("points must be an integer from 2 to {}",cfg.series_max_points)}),
                )
            }
        },
    };
    let result = (|| -> Result<Value> {
        match path {
            "/api/meta" => {
                let (first, last) = db.bounds()?;
                let g = db.context_at(last.unwrap_or(clock), "scope_geometry")?;
                Ok(json!({"service":"stella-scope","version":env!("CARGO_PKG_VERSION"),"read_only":true,
                    "instance":{"moonraker_url":cfg.moonraker_url,"database":cfg.database},
                    "first_ts":first,"last_ts":last,"server_time":clock,"collector":status,
                    "series_max_points":cfg.series_max_points,"investigation_limits":cfg.investigation,
                    "geometry_recorded":g.is_some(),"geometry":g.unwrap_or_else(||model::geometry(cfg)),
                    "printer":{"name":cfg.printer_name,"model":cfg.printer_model}}))
            }
            "/api/live" | "/api/at" => {
                let sample = db.at(if path == "/api/live" {
                    f64::MAX
                } else {
                    cursor
                })?;
                let sample_ts = sample.as_ref().map(|(t, _)| *t);
                let at = if path == "/api/live" && status.mode == "replay" {
                    sample_ts.unwrap_or(cursor)
                } else {
                    cursor
                };
                let g = db.context_at(at, "scope_geometry")?;
                Ok(
                    json!({"sample":sample_json(sample),"requested_ts":at,"server_time":clock,
                    "age_seconds":sample_ts.map(|ts|(at-ts).max(0.0)),"collector":status,
                    "recorded_health":db.context_at(at,"scope_health")?,
                    "geometry_recorded":g.is_some(),"geometry":g.unwrap_or_else(||model::geometry(cfg))}),
                )
            }
            "/api/series" => Ok(
                json!({"from":window.0,"to":window.1,"points":db.series(window.0,window.1,points)?,
                "method":"indexed representative samples; brief peaks can be omitted at wide zoom"}),
            ),
            "/api/events" => Ok(
                json!({"events":db.events(window.0,window.1,cfg.events_limit)?,"limit":cfg.events_limit}),
            ),
            "/api/qgl" => {
                let rows = db.rounds(window.0, window.1, cfg.qgl_limit)?;
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let g = db.context_at(row.ts, "scope_geometry")?;
                    let tolerance = g
                        .as_ref()
                        .and_then(|v| v["qgl_tolerance_mm"].as_f64())
                        .unwrap_or(cfg.qgl_tolerance_mm);
                    let mut v = serde_json::to_value(&row).map_err(|e| e.to_string())?;
                    v["tolerance_mm"] = json!(tolerance);
                    v["pass"] = json!(row.range_mm <= tolerance);
                    out.push(v);
                }
                Ok(json!({"rounds":out,"limit":cfg.qgl_limit}))
            }
            _ => Err("unknown API route".into()),
        }
    })();
    match result {
        Ok(v) => Response::json(200, v),
        Err(e) => Response::json(500, json!({"error":e})),
    }
}
