//! `GET /api/print/gcode?filename=<name>` — a read-only proxy of one file from Moonraker's gcodes
//! root, so the HUD's toolpath view can parse a print without talking to the printer itself.
//! Nothing here writes, and nothing here can name a file outside that root.
use super::{query, Response};
use crate::{
    config::Config,
    moonraker::{check_gcode_name, Moonraker},
};
use serde_json::json;

/// Refused above this size: a 64 MB gcode file is already ~1.5 million moves, and the browser has
/// to hold the parsed toolpath as well as the text.
pub const MAX_GCODE_BYTES: usize = 64 * 1024 * 1024;

pub fn handle(method: &str, target: &str, cfg: &Config) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"the gcode proxy is read-only; use GET"}),
        );
    }
    let q = match query(target) {
        Ok((_, q)) => q,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let Some(filename) = q.get("filename") else {
        return Response::json(
            400,
            json!({"error":"missing filename; name one file from GET /api/files"}),
        );
    };
    if let Err(e) = check_gcode_name(filename) {
        return Response::json(400, json!({"error":e}));
    }
    let moonraker = Moonraker::new(cfg);
    // Ask for the size first so an oversized file is refused before it crosses the network.
    let size = match moonraker.file_metadata(filename) {
        Ok(meta) => meta["size"].as_f64(),
        Err(e) => return Response::json(502, json!({"error":e})),
    };
    if size.is_some_and(|bytes| bytes > MAX_GCODE_BYTES as f64) {
        return Response::json(
            413,
            json!({"error":format!("{filename} is {} bytes, over the {MAX_GCODE_BYTES} byte gcode limit",size.unwrap_or_default())}),
        );
    }
    match moonraker.gcode_file(filename, MAX_GCODE_BYTES) {
        Ok(body) => Response {
            code: 200,
            mime: "text/plain; charset=utf-8",
            body,
        },
        Err(e) => Response::json(502, json!({"error":e})),
    }
}

/// `GET /api/print/thumbnail?filename=<name>` — the largest slicer thumbnail for one file,
/// proxied read-only from Moonraker so the Files modal can show what a print looks like. Same
/// gate as the gcode proxy: nothing writes, and nothing can name a file outside the gcodes root.
pub fn thumbnail_handle(method: &str, target: &str, cfg: &Config) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"the thumbnail proxy is read-only; use GET"}),
        );
    }
    let q = match query(target) {
        Ok((_, q)) => q,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let Some(filename) = q.get("filename") else {
        return Response::json(
            400,
            json!({"error":"missing filename; name one file from GET /api/files"}),
        );
    };
    if let Err(e) = check_gcode_name(filename) {
        return Response::json(400, json!({"error":e}));
    }
    // Thumbnails are tens of kilobytes; anything past half a megabyte is not a thumbnail.
    match Moonraker::new(cfg).file_thumbnail(filename, 512 * 1024) {
        Ok(bytes) => Response {
            code: 200,
            mime: "image/png",
            body: bytes,
        },
        Err(e) => Response::json(502, json!({"error":e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    /// A fake Moonraker that answers a fixed list of replies in order, like the one in
    /// `commands.rs`, but able to send a non-JSON body for the file itself.
    fn fake(replies: Vec<(&'static str, String)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for (mime, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
            }
            requests
        });
        (format!("http://{address}"), worker)
    }

    fn cfg(url: String) -> Config {
        Config {
            moonraker_url: url,
            request_timeout_ms: 1000,
            command_timeout_ms: 2000,
            ..Config::default()
        }
    }
    fn error(response: &Response) -> String {
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn proxies_the_file_with_its_length_and_asks_moonraker_for_the_right_path() {
        let gcode = ";LAYER_CHANGE\n;Z:0.2\nG1 X10 Y10 E1\n".to_owned();
        let (url, worker) = fake(vec![
            (
                "application/json",
                json!({"result":{"size":gcode.len()}}).to_string(),
            ),
            ("text/plain", gcode.clone()),
        ]);
        let response = handle(
            "GET",
            "/api/print/gcode?filename=Voron_Design_Cube_v7%20copy.gcode",
            &cfg(url),
        );
        let requests = worker.join().unwrap();
        assert_eq!(response.code, 200);
        assert_eq!(response.mime, "text/plain; charset=utf-8");
        assert_eq!(response.body, gcode.as_bytes());
        assert!(
            requests[0].starts_with(
                "GET /server/files/metadata?filename=Voron_Design_Cube_v7%20copy.gcode "
            ),
            "metadata request was {:?}",
            requests[0].lines().next()
        );
        assert!(
            requests[1].starts_with("GET /server/files/gcodes/Voron_Design_Cube_v7%20copy.gcode "),
            "file request was {:?}",
            requests[1].lines().next()
        );
    }

    #[test]
    fn refuses_path_separators_and_traversal_without_calling_moonraker() {
        for name in [
            "..%2Fprinter.cfg",
            "sub%2Fpart.gcode",
            "..%5Cprinter.cfg",
            "a..b.gcode",
            "",
        ] {
            // moonraker_url points at a closed port: reaching it at all would fail the test.
            let response = handle(
                "GET",
                &format!("/api/print/gcode?filename={name}"),
                &cfg("http://127.0.0.1:1".into()),
            );
            assert_eq!(response.code, 400, "{name} was not refused");
            assert!(error(&response).contains("plain file"), "{name}");
        }
    }

    #[test]
    fn refuses_a_file_over_the_limit_before_downloading_it() {
        let (url, worker) = fake(vec![(
            "application/json",
            json!({"result":{"size":MAX_GCODE_BYTES as u64 + 1}}).to_string(),
        )]);
        let response = handle("GET", "/api/print/gcode?filename=huge.gcode", &cfg(url));
        let requests = worker.join().unwrap();
        assert_eq!(response.code, 413);
        assert!(error(&response).contains("over the"));
        assert_eq!(requests.len(), 1, "the file itself must not be fetched");
    }

    #[test]
    fn missing_filename_and_wrong_method_are_refused() {
        let c = cfg("http://127.0.0.1:1".into());
        let missing = handle("GET", "/api/print/gcode", &c);
        assert_eq!(missing.code, 400);
        assert!(error(&missing).contains("missing filename"));
        let posted = handle("POST", "/api/print/gcode?filename=a.gcode", &c);
        assert_eq!(posted.code, 405);
    }

    #[test]
    fn an_unreachable_printer_is_a_bad_gateway() {
        let response = handle(
            "GET",
            "/api/print/gcode?filename=a.gcode",
            &cfg("http://127.0.0.1:1".into()),
        );
        assert_eq!(response.code, 502);
    }

    /// A binary-capable variant of `fake` for the PNG proxy test, since a PNG's magic bytes
    /// are not valid UTF-8.
    fn fake_raw(
        replies: Vec<(&'static str, Vec<u8>)>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for (mime, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (format!("http://{address}"), worker)
    }

    #[test]
    fn thumbnail_proxies_the_largest_png_and_names_the_right_file() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(b"test-thumbnail-bytes");
        // Real Moonraker (2026-09-22, the cube on a real printer\'s card) puts thumbnails
        // in a `.thumbs/` directory beside the file; the fixture mirrors that shape
        // exactly.
        let (url, worker) = fake_raw(vec![
            (
                "application/json",
                json!({"result":{"thumbnails":[
                    {"width":32,"height":32,"relative_path":".thumbs/cube-32x32.png"},
                    {"width":96,"height":96,"relative_path":".thumbs/cube-96x96.png"}]}})
                .to_string()
                .into_bytes(),
            ),
            ("image/png", png.clone()),
        ]);
        let response =
            thumbnail_handle("GET", "/api/print/thumbnail?filename=cube.gcode", &cfg(url));
        let requests = worker.join().unwrap();
        assert_eq!(response.code, 200);
        assert_eq!(response.mime, "image/png");
        assert_eq!(response.body, png);
        assert!(
            requests[0].starts_with("GET /server/files/metadata?filename=cube.gcode "),
            "metadata request was {:?}",
            requests[0].lines().next()
        );
        assert!(
            requests[1].starts_with("GET /server/files/gcodes/.thumbs/cube-96x96.png "),
            "the largest thumbnail must be fetched, request was {:?}",
            requests[1].lines().next()
        );
    }

    #[test]
    fn thumbnail_refuses_bad_names_and_verbs_without_calling_moonraker() {
        let c = cfg("http://127.0.0.1:1".into());
        assert_eq!(
            thumbnail_handle("GET", "/api/print/thumbnail?filename=..%2Fprinter.cfg", &c).code,
            400
        );
        assert_eq!(
            thumbnail_handle("POST", "/api/print/thumbnail?filename=a.gcode", &c).code,
            405
        );
        assert_eq!(
            thumbnail_handle("GET", "/api/print/thumbnail", &c).code,
            400
        );
    }

    #[test]
    fn a_file_without_thumbnails_says_so_and_fetches_nothing_else() {
        let (url, worker) = fake(vec![(
            "application/json",
            json!({"result":{"thumbnails":[]}}).to_string(),
        )]);
        let response =
            thumbnail_handle("GET", "/api/print/thumbnail?filename=bare.gcode", &cfg(url));
        let requests = worker.join().unwrap();
        assert_eq!(response.code, 502);
        assert!(error(&response).contains("no thumbnail"));
        assert_eq!(requests.len(), 1, "the png itself must not be fetched");
    }

    #[test]
    fn a_thumbnail_that_traverses_or_is_not_a_png_is_refused() {
        // Subdirectories are fine (real Moonraker uses `.thumbs/`); traversal and non-png are not.
        for bad in [
            "cube.gcode",
            "..%2Fescape.png",
            "../escape.png",
            "/abs.png",
            "sub\\back.png",
        ] {
            let (url, worker) = fake(vec![(
                "application/json",
                json!({"result":{"thumbnails":[{"width":96,"height":96,"relative_path":bad}]}})
                    .to_string(),
            )]);
            let response =
                thumbnail_handle("GET", "/api/print/thumbnail?filename=cube.gcode", &cfg(url));
            worker.join().unwrap();
            assert_eq!(response.code, 502, "{bad}");
            assert!(error(&response).contains("plain .png"), "{bad}");
        }
    }
}

/// `GET /api/print/file?filename=<name>`: Moonraker's slicer-parsed
/// metadata plus the slicer's own settings block from the last 64 KB of the file. Two read-only
/// GETs, the same gate as the gcode and thumbnail proxies.
pub fn file_handle(method: &str, target: &str, cfg: &Config) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"file details are read-only; use GET"}));
    }
    let q = match query(target) {
        Ok((_, q)) => q,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let Some(filename) = q.get("filename") else {
        return Response::json(
            400,
            json!({"error":"missing filename; name one file from GET /api/files"}),
        );
    };
    if let Err(e) = check_gcode_name(filename) {
        return Response::json(400, json!({"error":e}));
    }
    let moonraker = Moonraker::new(cfg);
    let metadata = match moonraker.file_metadata(filename) {
        Ok(m) => m,
        Err(e) => return Response::json(502, json!({"error":e})),
    };
    let tail = moonraker.gcode_tail(filename, 65536).unwrap_or_default();
    Response::json(
        200,
        json!({"filename":filename,"metadata":metadata,"settings":slicer_settings(&tail)}),
    )
}

/// The slicer settings worth showing beside a file, read from its "; key = value" tail block.
/// Keys the slicer did not write are simply absent.
pub fn slicer_settings(tail: &str) -> serde_json::Map<String, serde_json::Value> {
    const KEYS: &[&str] = &[
        "printer_settings_id", "print_settings_id", "filament_settings_id", "layer_height",
        "initial_layer_print_height", "wall_loops", "top_shell_layers", "bottom_shell_layers",
        "sparse_infill_density", "sparse_infill_pattern", "enable_support", "support_type",
        "brim_type", "brim_width", "nozzle_temperature", "nozzle_temperature_initial_layer",
        "bed_temperature", "hot_plate_temp", "filament_flow_ratio", "pressure_advance",
        "outer_wall_speed", "inner_wall_speed", "sparse_infill_speed", "travel_speed",
        "default_acceleration", "outer_wall_acceleration", "fan_max_speed", "fan_min_speed",
        "seam_position", "ironing_type", "filament_type", "nozzle_diameter",
        "fill_density", "perimeters", "temperature", "first_layer_temperature",
        "first_layer_bed_temperature", "extrusion_multiplier", "print_settings_id",
    ];
    let mut out = serde_json::Map::new();
    for line in tail.lines() {
        let Some((k, v)) = line.trim().strip_prefix(';').and_then(|r| r.split_once('=')) else {
            continue;
        };
        let key = k.trim().to_lowercase();
        if KEYS.contains(&key.as_str()) && !out.contains_key(&key) {
            let v = v.trim();
            out.insert(key, json!(if v.len() > 120 { &v[..v.floor_char_boundary(120)] } else { v }));
        }
    }
    out
}

#[cfg(test)]
mod file_detail_tests {
    #[test]
    fn slicer_settings_reads_the_tail_block_and_nothing_else() {
        let s = super::slicer_settings(
            "G1 X1\n; layer_height = 0.2\n; wall_loops = 3\n; secret_key = no\n;sparse_infill_density = 15%\n",
        );
        assert_eq!(s["layer_height"], "0.2");
        assert_eq!(s["wall_loops"], "3");
        assert_eq!(s["sparse_infill_density"], "15%");
        assert!(!s.contains_key("secret_key"));
    }
}

/// `GET /api/print/history?limit=N`: Moonraker's own job history and
/// totals (every print since it was installed, two read-only GETs). When the printer does not
/// answer, the prints this recorder saw start (print_context) are returned instead, marked as
/// such.
pub fn history_handle(method: &str, target: &str, cfg: &Config, db: &crate::store::Store, ask_printer: bool) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"print history is read-only; use GET"}));
    }
    let q = match query(target) {
        Ok((_, q)) => q,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let limit = q.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(100).clamp(1, 500);
    let moonraker = Moonraker::new(cfg);
    // A replay never asks the printer anything.
    let listed = if ask_printer {
        moonraker.get(&format!("/server/history/list?limit={limit}&order=desc"))
    } else {
        Err("this is a saved recording".to_owned())
    };
    match listed {
        Ok(list) => {
            let jobs: Vec<serde_json::Value> = list["jobs"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|j| {
                            json!({"filename":j["filename"],"status":j["status"],"start_time":j["start_time"],
                                "end_time":j["end_time"],"print_duration":j["print_duration"],"total_duration":j["total_duration"],
                                "filament_used":j["filament_used"],"metadata":{"filament_type":j["metadata"]["filament_type"],
                                "filament_name":j["metadata"]["filament_name"],"estimated_time":j["metadata"]["estimated_time"],
                                "layer_height":j["metadata"]["layer_height"],"slicer":j["metadata"]["slicer"]}})
                        })
                        .collect()
                })
                .unwrap_or_default();
            let totals = moonraker
                .get("/server/history/totals")
                .map(|t| t["job_totals"].clone())
                .unwrap_or(serde_json::Value::Null);
            Response::json(200, json!({"source":"Moonraker job history","jobs":jobs,"totals":totals}))
        }
        Err(e) => match db.print_context(0.0, f64::MAX, limit) {
            Ok(rows) => {
                let jobs: Vec<serde_json::Value> = rows
                    .into_iter().rev()
                    .map(|(ts, filename, meta, _)| {
                        json!({"filename":filename,"status":"recorded start","start_time":ts,"end_time":null,
                            "print_duration":null,"filament_used":null,
                            "metadata":{"filament_type":meta["filament_type"],"estimated_time":meta["estimated_time"]}})
                    })
                    .collect();
                Response::json(200, json!({"source":"prints this recorder saw start (the printer is not answering)",
                    "jobs":jobs,"totals":null,"printer_error":e}))
            }
            Err(e2) => Response::json(502, json!({"error":format!("{e}; the recording could not be read either: {e2}")})),
        },
    }
}

/// `GET /api/print/mesh-profiles`: the meshes Klipper has saved.
pub fn mesh_profiles_handle(method: &str, cfg: &Config, ask_printer: bool) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"mesh profiles are read-only; use GET"}));
    }
    if !ask_printer {
        return Response::json(200, json!({"profiles":[],"note":"this is a saved recording"}));
    }
    match Moonraker::new(cfg).get("/printer/objects/query?bed_mesh=profiles,profile_name") {
        Ok(v) => {
            let bm = &v["status"]["bed_mesh"];
            let profiles: Vec<serde_json::Value> = bm["profiles"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(name, p)| {
                            let grid = p["points"].as_array()?.clone();
                            let mp = &p["mesh_params"];
                            Some(json!({"name":name,"grid":grid,"rows":grid.len(),
                                "cols":grid.first().and_then(|r| r.as_array()).map(|r| r.len()).unwrap_or(0),
                                "min":[mp["min_x"],mp["min_y"]],"max":[mp["max_x"],mp["max_y"]]}))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Response::json(200, json!({"profiles":profiles,"active":bm["profile_name"]}))
        }
        Err(e) => Response::json(502, json!({"error":e})),
    }
}
