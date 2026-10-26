//! `GET /api/print/gcode?filename=<name>` — a read-only proxy of one print file, served by the
//! printer's own driver (WO-01): Moonraker from its gcodes root, PrusaLink from
//! /api/v1/files/{storage}/{path}, FlashForge by layer only (its door does not serve file contents).
//! Nothing here writes, and nothing here can name a file outside the printer's own list.
use super::{query, Response};
use crate::{
    config::Config,
    driver::open,
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
    let driver = match open(cfg) {
        Ok(d) => d,
        Err(e) => return Response::json(502, json!({"error":e})),
    };
    // Ask for the size first so an oversized file is refused before it crosses the network.
    let size = match driver.file_size(filename) {
        Ok(s) => s,
        Err(e) => return Response::json(502, json!({"error":e})),
    };
    if size > MAX_GCODE_BYTES {
        return Response::json(
            413,
            json!({"error":format!("{filename} is {size} bytes, over the {MAX_GCODE_BYTES} byte gcode limit")}),
        );
    }
    // Finish-2 F02 (CORE One L, 2026-10-19): every page that opened asked the printer for the whole print file again (4.3 MB,
    // about a minute over PrusaLink). The last few files are kept, by name and size, and one download runs at a time, so a
    // second page waits for the first one's copy instead of asking the printer in parallel.
    // ponytail: one lock for all printers' downloads; per-printer locks if a farm opens many printers' pages at once.
    match file_body(cfg, &*driver, filename, size, true) {
        Some(Ok(body)) => Response {
            code: 200,
            mime: "text/plain; charset=utf-8",
            body,
        },
        Some(Err(e)) => Response::json(502, json!({"error":e})),
        None => Response::json(502, json!({"error":"the file is not here"})),
    }
}

/// One print file's bytes through the kept copies: the last few files are kept (see `file_cache`) and one download runs at a time. With
/// `download` false a file that is not kept is not fetched (`None`): file details use that so looking at a file never makes the printer send it.
fn file_body(
    cfg: &Config,
    driver: &dyn crate::driver::Driver,
    filename: &str,
    size: usize,
    download: bool,
) -> Option<crate::Result<Vec<u8>>> {
    // ponytail: one lock for all printers' downloads; per-printer locks if a farm opens many printers' pages at once.
    static ONE_DOWNLOAD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let key = format!("{}|{}|{filename}|{size}", cfg.printer_id, cfg.moonraker_url);
    if let Some(body) = file_cache::get(&key) {
        return Some(Ok(body));
    }
    if !download {
        return None;
    }
    let _one = ONE_DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
    // a second page that waited for the lock finds the first one's copy
    if let Some(body) = file_cache::get(&key) {
        return Some(Ok(body));
    }
    Some(driver.file_bytes(filename, MAX_GCODE_BYTES).inspect(|body| file_cache::put(key, body.clone())))
}

/// The last print files read from printers (see `handle`): three, kept 30 minutes.
mod file_cache {
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    const KEEP: Duration = Duration::from_secs(1800);
    const MAX: usize = 3;
    static CACHE: Mutex<Vec<(String, Instant, Vec<u8>)>> = Mutex::new(Vec::new());
    pub fn get(key: &str) -> Option<Vec<u8>> {
        let mut c = CACHE.lock().ok()?;
        c.retain(|(_, at, _)| at.elapsed() < KEEP);
        c.iter().find(|(k, _, _)| k == key).map(|(_, _, b)| b.clone())
    }
    pub fn put(key: String, bytes: Vec<u8>) {
        let Ok(mut c) = CACHE.lock() else { return };
        c.retain(|(k, _, _)| *k != key);
        if c.len() >= MAX {
            c.remove(0);
        }
        c.push((key, Instant::now(), bytes));
    }
    #[cfg(test)]
    #[test]
    fn the_last_three_files_are_kept() {
        for i in 0..4 {
            put(format!("fc|{i}"), vec![i]);
        }
        assert_eq!(get("fc|0"), None, "the oldest went");
        assert_eq!(get("fc|3"), Some(vec![3]));
        assert_eq!(get("fc|1"), Some(vec![1]));
    }
}

/// WO-13: `GET /api/print/thumbnail?filename=<name>` — the largest slicer thumbnail for one file,
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
    // Polish WO-06 item 3 (2026-10-18): a printer that has no pictures at all (a FlashForge) answers "nothing", not a failure: a
    // 502 per picture filled the browser's console on every page that listed its files.
    if crate::driver::open(cfg).is_ok_and(|d| !d.capabilities().thumbnails) {
        return Response {
            code: 204,
            mime: "image/png",
            body: Vec::new(),
        };
    }
    // Finish-2 F07 (2026-10-19): a dry run's copy ("cube (dry run).gcode") never has a picture of its own, and asking for it was a 404 in the
    // console on every page that showed the print. The picture is the original file's; if that has none, or is not on the printer, the answer
    // is "nothing" (204), as for a printer with no pictures at all.
    let original = crate::commands::dry_run_original(filename);
    let dry = original.is_some();
    let filename = original.as_deref().unwrap_or(filename);
    let key = format!("{}|{}|{filename}", cfg.printer_id, cfg.moonraker_url);
    if let Some(bytes) = thumb_cache::get(&key) {
        return Response {
            code: 200,
            mime: "image/png",
            body: bytes,
        };
    }
    // Thumbnails are tens of kilobytes; anything past half a megabyte is not a thumbnail.
    match crate::driver::open(cfg).and_then(|d| d.file_thumbnail(filename, 512 * 1024)) {
        Ok(bytes) => {
            thumb_cache::put(key, bytes.clone());
            Response {
                code: 200,
                mime: "image/png",
                body: bytes,
            }
        }
        // Polish WO-06 (live finding): a file that has no picture, or is not on the printer, is "not found", not a failure of the printer.
        Err(e) if dry && (e.contains("no thumbnail") || e.contains("no such file")) => Response {
            code: 204,
            mime: "image/png",
            body: Vec::new(),
        },
        Err(e) if e.contains("no thumbnail") || e.contains("no such file") => {
            Response::json(404, json!({"error":e}))
        }
        Err(e) => Response::json(502, json!({"error":e})),
    }
}

/// Pictures kept for a while (finish-2 F02, 2026-10-19). A PrusaLink picture took about 1.3 s on the CORE One L, one at a
/// time through the printer, so every opening of the file sheet asked the printer for all of them again. They are kept ten
/// minutes, at most 400 (about 20 MB at the 50 kB a slicer picture weighs); a file sent again under the same name may show
/// its old picture until then. Only pictures are kept: a missing one is asked again.
mod thumb_cache {
    use std::{
        collections::HashMap,
        sync::Mutex,
        time::{Duration, Instant},
    };
    const KEEP: Duration = Duration::from_secs(600);
    const MAX: usize = 400;
    type Map = HashMap<String, (Instant, Vec<u8>)>;
    static CACHE: Mutex<Option<Map>> = Mutex::new(None);
    pub fn get(key: &str) -> Option<Vec<u8>> {
        let mut g = CACHE.lock().ok()?;
        let map = g.get_or_insert_with(HashMap::new);
        let fresh = map.get(key).map(|(at, _)| at.elapsed() < KEEP)?;
        if fresh {
            map.get(key).map(|(_, b)| b.clone())
        } else {
            map.remove(key);
            None
        }
    }
    pub fn put(key: String, bytes: Vec<u8>) {
        let Ok(mut g) = CACHE.lock() else { return };
        let map = g.get_or_insert_with(HashMap::new);
        if map.len() >= MAX {
            map.retain(|_, (at, _)| at.elapsed() < KEEP);
            let oldest = map.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone());
            if let (true, Some(k)) = (map.len() >= MAX, oldest) {
                map.remove(&k);
            }
        }
        map.insert(key, (Instant::now(), bytes));
    }
    #[cfg(test)]
    #[test]
    fn a_picture_is_kept_and_the_oldest_goes_first_when_full() {
        put("t|a".into(), vec![1]);
        assert_eq!(get("t|a"), Some(vec![1]));
        assert_eq!(get("t|missing"), None);
        for i in 0..MAX + 5 {
            put(format!("t|n{i}"), vec![2]);
        }
        let n = CACHE.lock().unwrap().as_ref().unwrap().len();
        assert!(n <= MAX, "{n} kept");
        assert_eq!(get("t|a"), None, "the oldest went first");
        assert_eq!(get(&format!("t|n{}", MAX + 4)), Some(vec![2]));
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

    /// WO-13: a binary-capable variant of `fake` for the PNG proxy test, since a PNG's magic
    /// bytes are not valid UTF-8.
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
        // Real Moonraker (2026-09-22, the cube on Stella's own card) puts thumbnails in a
        // `.thumbs/` directory beside the file; the fixture mirrors that shape exactly.
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

    // Finish-2 F07 (2026-10-19): the picture of a dry run is the original file's, asked for by the original's name; a copy has none of its own.
    #[test]
    fn a_dry_run_copy_shows_the_originals_picture() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(b"dry-run-original");
        let (url, worker) = fake_raw(vec![
            (
                "application/json",
                json!({"result":{"thumbnails":[{"width":96,"height":96,"relative_path":".thumbs/Cube_Dry-96x96.png"}]}})
                    .to_string()
                    .into_bytes(),
            ),
            ("image/png", png.clone()),
        ]);
        let response = thumbnail_handle(
            "GET",
            "/api/print/thumbnail?filename=Cube_Dry%20%28dry%20run%29.gcode",
            &cfg(url),
        );
        let requests = worker.join().unwrap();
        assert_eq!((response.code, response.body), (200, png));
        assert!(
            requests[0].starts_with("GET /server/files/metadata?filename=Cube_Dry.gcode "),
            "the original was asked for, not the copy: {:?}",
            requests[0].lines().next()
        );
    }

    #[test]
    fn a_dry_run_copy_whose_original_has_no_picture_answers_nothing_not_a_404() {
        let (url, worker) = fake(vec![(
            "application/json",
            json!({"result":{"thumbnails":[]}}).to_string(),
        )]);
        let response = thumbnail_handle(
            "GET",
            "/api/print/thumbnail?filename=Bare%20%28dry%20run%29.gcode",
            &cfg(url),
        );
        let requests = worker.join().unwrap();
        assert_eq!(response.code, 204, "nothing to show is not an error");
        assert!(response.body.is_empty());
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /server/files/metadata?filename=Bare.gcode "), "{:?}", requests[0].lines().next());
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
        // polish WO-06 (live finding): a file that simply has no picture is 404, not a failing printer (502)
        assert_eq!(response.code, 404);
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

/// NEXT review 2026-09-24 (the owner: "clicking a file should open its details, the slicer that was
/// used, its settings"). `GET /api/print/file?filename=<name>`: Moonraker's slicer-parsed metadata
/// plus the slicer's own settings block from the last 64 KB of the file. Two read-only GETs, the
/// same gate as the gcode and thumbnail proxies.
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
    let metadata = match crate::driver::open(cfg).and_then(|d| d.file_metadata(filename)) {
        Ok(m) => m,
        Err(e) => return Response::json(502, json!({"error":e})),
    };
    // The tuned slicer settings sit in the tail of a Klipper file, read from where it is stored. Another brand's printer only sends the whole
    // file (PrusaLink has no ranged read): the details use a copy already kept, or fetch it when the person presses "Read the settings"
    // (`settings=1`), so looking at a file never makes the printer send 35 MB on its own (finish-2 F06, the F02 open item).
    let mut metadata = metadata;
    let mut layers: Option<(u32, f64)> = None;
    let (tail, head, settings_state) = if cfg.driver == "moonraker" {
        (
            Moonraker::new(cfg).gcode_tail(filename, 65536).unwrap_or_default(),
            String::new(),
            "read",
        )
    } else if filename.to_ascii_lowercase().ends_with(".bgcode") {
        (String::new(), String::new(), "binary")
    } else if let Some(size) = metadata["size"].as_f64().filter(|s| *s > 0.0 && *s <= MAX_GCODE_BYTES as f64) {
        let ask = q.get("settings").is_some_and(|v| v.as_str() == "1");
        match open(cfg).ok().and_then(|d| file_body(cfg, &*d, filename, size as usize, ask)) {
            Some(Ok(body)) if body.starts_with(b"GCDE") => (String::new(), String::new(), "binary"),
            Some(Ok(body)) => {
                layers = layer_facts(&body);
                (
                    String::from_utf8_lossy(&body[body.len().saturating_sub(65536)..]).into_owned(),
                    String::from_utf8_lossy(&body[..body.len().min(4096)]).into_owned(),
                    "read",
                )
            }
            Some(Err(_)) => (String::new(), String::new(), "failed"),
            None => (String::new(), String::new(), "not_read"),
        }
    } else {
        (String::new(), String::new(), "not_read")
    };
    let settings = slicer_settings(&tail);
    if let Some(m) = metadata.as_object_mut() {
        if let Some((n, z)) = layers {
            m.entry("layer_count").or_insert(json!(n));
            m.entry("object_height").or_insert(json!(z));
        }
        for (k, v) in file_facts(&head, &tail, &settings) {
            if m.get(&k).is_none_or(|old| old.is_null()) {
                m.insert(k, v);
            }
        }
    }
    Response::json(
        200,
        json!({"filename":filename,"metadata":metadata,"settings":settings,"settings_state":settings_state}),
    )
}

/// How many layers a PrusaSlicer or OrcaSlicer text file has (its `;LAYER_CHANGE` lines) and how tall it is (the last `;Z:`), from the whole file.
pub fn layer_facts(body: &[u8]) -> Option<(u32, f64)> {
    if body.starts_with(b"GCDE") {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    let (mut n, mut z) = (0_u32, None);
    for l in text.lines() {
        if l.starts_with(";LAYER_CHANGE") {
            n += 1;
        } else if let Some(v) = l.strip_prefix(";Z:") {
            z = v.trim().parse::<f64>().ok().or(z);
        }
    }
    (n > 0).then_some((n, z.unwrap_or(0.0)))
}

/// What a PrusaSlicer or OrcaSlicer text file says about itself, for a printer that cannot say it (PrusaLink sends none of it for a plain
/// `.gcode`): the slicer from the first lines, the estimate, the filament and the layer height from the end. Missing stays missing.
pub fn file_facts(head: &str, tail: &str, settings: &serde_json::Map<String, serde_json::Value>) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    if let Some(rest) = head.lines().find_map(|l| l.trim().strip_prefix("; generated by ")) {
        let mut words = rest.split_whitespace();
        if let Some(name) = words.next() {
            out.insert("slicer".into(), json!(name));
            if let Some(v) = words.next().filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit())) {
                out.insert("slicer_version".into(), json!(v));
            }
        }
    }
    let value = |key: &str| tail.lines().find_map(|l| l.trim().strip_prefix(';').and_then(|r| r.split_once('=')).filter(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v.trim().to_owned()));
    if let Some(t) = value("estimated printing time (normal mode)") {
        let mut s = 0.0;
        for part in t.split_whitespace() {
            let Some(at) = part.find(|c: char| c.is_ascii_alphabetic()) else { continue };
            let n: f64 = part[..at].parse().unwrap_or(0.0);
            s += n * match &part[at..] {
                "d" => 86400.0,
                "h" => 3600.0,
                "m" => 60.0,
                _ => 1.0,
            };
        }
        if s > 0.0 {
            out.insert("estimated_time".into(), json!(s));
        }
    }
    let number = |text: Option<String>| text.and_then(|t| t.split([',', ';']).next().and_then(|n| n.trim().parse::<f64>().ok()));
    if let Some(mm) = number(value("filament used [mm]")) {
        out.insert("filament_total".into(), json!(mm));
    }
    if let Some(g) = number(value("total filament used [g]").or_else(|| value("filament used [g]"))) {
        out.insert("filament_weight_total".into(), json!(g));
    }
    for (from, to) in [("layer_height", "layer_height"), ("first_layer_height", "first_layer_height"), ("nozzle_diameter", "nozzle_diameter")] {
        if let Some(n) = number(settings.get(from).and_then(|v| v.as_str()).map(str::to_owned).or_else(|| value(from))) {
            out.insert(to.into(), json!(n));
        }
    }
    for (from, to) in [("first_layer_temperature", "first_layer_extr_temp"), ("first_layer_bed_temperature", "first_layer_bed_temp")] {
        if let Some(n) = number(settings.get(from).and_then(|v| v.as_str()).map(str::to_owned).or_else(|| value(from))) {
            out.insert(to.into(), json!(n));
        }
    }
    if let Some(t) = settings.get("filament_type").and_then(|v| v.as_str()).map(|s| s.split(';').next().unwrap_or("").to_owned()).filter(|s| !s.is_empty()) {
        out.insert("filament_type".into(), json!(t));
    }
    out
}

/// The slicer settings worth showing beside a file, read from its "; key = value" tail block.
/// Keys the slicer did not write are simply absent.
pub fn slicer_settings(tail: &str) -> serde_json::Map<String, serde_json::Value> {
    const KEYS: &[&str] = &[
        "printer_settings_id",
        "print_settings_id",
        "filament_settings_id",
        "layer_height",
        "initial_layer_print_height",
        "wall_loops",
        "top_shell_layers",
        "bottom_shell_layers",
        "sparse_infill_density",
        "sparse_infill_pattern",
        "enable_support",
        "support_type",
        "brim_type",
        "brim_width",
        "nozzle_temperature",
        "nozzle_temperature_initial_layer",
        "bed_temperature",
        "hot_plate_temp",
        "filament_flow_ratio",
        "pressure_advance",
        "outer_wall_speed",
        "inner_wall_speed",
        "sparse_infill_speed",
        "travel_speed",
        "default_acceleration",
        "outer_wall_acceleration",
        "fan_max_speed",
        "fan_min_speed",
        "seam_position",
        "ironing_type",
        "filament_type",
        "nozzle_diameter",
        "fill_density",
        "perimeters",
        "temperature",
        "first_layer_temperature",
        "first_layer_bed_temperature",
        "extrusion_multiplier",
        "print_settings_id",
    ];
    let mut out = serde_json::Map::new();
    for line in tail.lines() {
        let Some((k, v)) = line
            .trim()
            .strip_prefix(';')
            .and_then(|r| r.split_once('='))
        else {
            continue;
        };
        let key = k.trim().to_lowercase();
        if KEYS.contains(&key.as_str()) && !out.contains_key(&key) {
            let v = v.trim();
            out.insert(
                key,
                json!(if v.len() > 120 {
                    &v[..v.floor_char_boundary(120)]
                } else {
                    v
                }),
            );
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

    fn sliced_text() -> Vec<u8> {
        let mut t = String::from("; generated by PrusaSlicer 2.9.6 on 2026-10-17 at 14:02:00 UTC\n");
        for i in 0..6000 {
            if i % 2000 == 0 {
                t.push_str(&format!(";LAYER_CHANGE\n;Z:{:.1}\n", 0.2 + 0.2 * (i / 2000) as f64));
            }
            t.push_str(&format!("G1 X{} Y{} E0.1\n", i % 100, i % 90));
        }
        t.push_str("; filament used [mm] = 5316.21\n; filament used [g] = 15.9\n; total filament used [g] = 15.9\n; estimated printing time (normal mode) = 1h 46m 20s\n; prusaslicer_config = begin\n; layer_height = 0.25\n; first_layer_height = 0.2\n; first_layer_temperature = 250\n; first_layer_bed_temperature = 85\n; nozzle_diameter = 0.4\n; filament_type = PETG\n; printer_settings_id = Prusa CORE One L HF 0.4 nozzle\n; fill_density = 10%\n; prusaslicer_config = end\n");
        t.into_bytes()
    }

    fn prusalink_cfg(p: &crate::fakes::FakePrusaLink) -> crate::config::Config {
        crate::config::Config {
            driver: "prusalink".into(),
            moonraker_url: p.url(),
            printer_id: "t-details".into(),
            secret: crate::config::Secret([("user".to_owned(), "maker".to_owned()), ("password".to_owned(), "secret".to_owned())].into()),
            ..crate::config::Config::default()
        }
    }

    fn details(cfg: &crate::config::Config, name: &str, settings: bool) -> serde_json::Value {
        let r = super::file_handle("GET", &format!("/api/print/file?filename={name}{}", if settings { "&settings=1" } else { "" }), cfg);
        assert_eq!(r.code, 200, "{}", String::from_utf8_lossy(&r.body));
        serde_json::from_slice(&r.body).unwrap()
    }

    /// Finish-2 F06 (the F02 open item): a CORE One L's file details showed "The file carries no slicer settings block" and dashes for the time, the filament
    /// and the layer height, although PrusaSlicer writes all of it into the file. The printer says nothing about a plain .gcode (the real one answered no
    /// `meta` at all), so it is read from the file, when the person asks: looking at a file must not make the printer send it.
    #[test]
    fn a_prusalink_files_details_are_read_from_the_file_when_asked_and_the_file_is_not_fetched_unasked() {
        let p = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        let bytes = sliced_text();
        {
            let mut s = p.state.lock().unwrap();
            s.files.push(("bowl.gcode".into(), bytes.len(), 1.0));
            s.contents.insert("bowl.gcode".into(), bytes.clone());
        }
        let cfg = prusalink_cfg(&p);
        let asked_for_the_file = || p.log().iter().filter(|e| e["path"].as_str().is_some_and(|x| x.starts_with("/usb/"))).count();
        let v = details(&cfg, "bowl.gcode", false);
        assert_eq!(v["settings_state"], "not_read", "{v}");
        assert!(v["settings"].as_object().unwrap().is_empty());
        assert_eq!(v["metadata"]["size"], bytes.len(), "the size is the printer's own");
        assert_eq!(asked_for_the_file(), 0, "the printer was not asked to send the file");
        let v = details(&cfg, "bowl.gcode", true);
        assert_eq!(v["settings_state"], "read", "{v}");
        assert_eq!(v["settings"]["layer_height"], "0.25");
        assert_eq!(v["settings"]["printer_settings_id"], "Prusa CORE One L HF 0.4 nozzle");
        let m = &v["metadata"];
        assert_eq!((m["slicer"].as_str(), m["slicer_version"].as_str()), (Some("PrusaSlicer"), Some("2.9.6")), "{m}");
        assert_eq!(m["estimated_time"].as_f64(), Some(6380.0), "1h 46m 20s");
        assert_eq!((m["filament_total"].as_f64(), m["filament_weight_total"].as_f64()), (Some(5316.21), Some(15.9)));
        assert_eq!((m["layer_height"].as_f64(), m["nozzle_diameter"].as_f64(), m["filament_type"].as_str()), (Some(0.25), Some(0.4), Some("PETG")));
        assert_eq!((m["layer_count"].as_u64(), m["object_height"].as_f64()), (Some(3), Some(0.6)), "the layers and the height are counted in the file: {m}");
        assert_eq!((m["first_layer_extr_temp"].as_f64(), m["first_layer_bed_temp"].as_f64()), (Some(250.0), Some(85.0)));
        assert_eq!(asked_for_the_file(), 1, "one copy");
        // The copy is kept: looking again costs the printer nothing, and still shows the settings.
        let again = details(&cfg, "bowl.gcode", false);
        assert_eq!(again["settings_state"], "read", "{again}");
        assert_eq!(asked_for_the_file(), 1);
    }

    /// A binary G-code file keeps its settings in blocks the printer reads itself, so it is not downloaded to look for a text tail that is not there.
    #[test]
    fn a_binary_gcode_file_is_never_fetched_for_its_settings() {
        let p = crate::fakes::FakePrusaLink::start(0, "maker", "secret");
        p.state.lock().unwrap().files.push(("cube.bgcode".into(), 5000, 1.0));
        let v = details(&prusalink_cfg(&p), "cube.bgcode", true);
        assert_eq!(v["settings_state"], "binary", "{v}");
        assert!(!p.log().iter().any(|e| e["path"].as_str().is_some_and(|x| x.starts_with("/usb/"))));
    }
}

/// NEXT review 2026-09-24 (the owner: "print history and stats, very important").
/// `GET /api/print/history?limit=N`: Moonraker's own job history and totals (every print since it
/// was installed, two read-only GETs). When the printer does not answer, the prints this recorder
/// saw start (print_context) are returned instead, marked as such.
pub fn history_handle(
    method: &str,
    target: &str,
    cfg: &Config,
    db: &crate::store::Store,
    ask_printer: bool,
) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"print history is read-only; use GET"}));
    }
    let q = match query(target) {
        Ok((_, q)) => q,
        Err(e) => return Response::json(400, json!({"error":e})),
    };
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    // Finish-2 F02 (CORE One L, 2026-10-19): only Klipper keeps a job history to ask. A PrusaLink was asked Moonraker's
    // /server/history/list, which answered 404, and the page was told "cannot reach Moonraker ... set
    // STELLA_MOONRAKER_API_KEY". Every other brand's history is the prints this recorder saw end, with their results.
    if cfg.driver != "moonraker" {
        return match db.print_sessions(0.0, f64::MAX, limit) {
            Ok(rows) => {
                let jobs: Vec<serde_json::Value> = rows
                    .into_iter()
                    .map(|s| {
                        json!({"filename":s.filename,"status":if s.result == "complete" { "completed" } else { s.result.as_str() },
                            "start_time":s.start_ts,"end_time":s.end_ts,"print_duration":s.duration_s,"total_duration":s.duration_s,
                            "filament_used":s.filament_used,"metadata":{}})
                    })
                    .collect();
                Response::json(200, json!({"source":"prints this recorder saw","jobs":jobs,"totals":null}))
            }
            Err(e) => Response::json(502, json!({"error":format!("the recording could not be read: {e}")})),
        };
    }
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
            Response::json(
                200,
                json!({"source":"Moonraker job history","jobs":jobs,"totals":totals}),
            )
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
                Response::json(
                    200,
                    json!({"source":"prints this recorder saw start (the printer is not answering)",
                    "jobs":jobs,"totals":null,"printer_error":e}),
                )
            }
            Err(e2) => Response::json(
                502,
                json!({"error":format!("{e}; the recording could not be read either: {e2}")}),
            ),
        },
    }
}

/// `GET /api/print/mesh-profiles`: the meshes Klipper has saved (2026-09-25: the bed panel only
/// showed the newest adaptive mesh, a small rectangle over the last print, never the full saved one).
pub fn mesh_profiles_handle(method: &str, cfg: &Config, ask_printer: bool) -> Response {
    if method != "GET" {
        return Response::json(405, json!({"error":"mesh profiles are read-only; use GET"}));
    }
    if !ask_printer {
        return Response::json(
            200,
            json!({"profiles":[],"note":"this is a saved recording"}),
        );
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
            Response::json(
                200,
                json!({"profiles":profiles,"active":bm["profile_name"]}),
            )
        }
        Err(e) => Response::json(502, json!({"error":e})),
    }
}
