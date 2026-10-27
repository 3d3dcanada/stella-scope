use crate::{
    api::{self, Response},
    commands,
    config::Config,
    farm::{self, Farm, Printer},
    store::Store,
    Result,
};
use serde_json::json;
use std::sync::mpsc::{channel as sse_channel, Receiver as SseReceiver, Sender as SseSender};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

// SSE registry: one entry per open /api/tick/stream connection, tagged with the printer it watches.
// The collector broadcasts the newest tick payload to every listener of its own printer the moment a
// sample is stored; dead listeners drop out. Each stream pins a worker for its lifetime, so
// concurrency is capped: two windows per printer is the realistic maximum, six in all; one more gets
// 503 and the client polls /api/tick.
const MAX_SSE_STREAMS_PER_PRINTER: usize = 2;
const MAX_SSE_STREAMS: usize = 6;
fn sse_registry() -> &'static std::sync::Mutex<Vec<(String, SseSender<String>)>> {
    static R: std::sync::OnceLock<std::sync::Mutex<Vec<(String, SseSender<String>)>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}
// The channel carries whole SSE frames, so a named event and the default tick event go down the
// same path. The tick keeps its unnamed frame: naming it would move it off EventSource.onmessage
// and break every page already listening.
fn sse_send(printer: &str, frame: String) {
    if let Ok(mut list) = sse_registry().lock() {
        list.retain(|(id, tx)| id != printer || tx.send(frame.clone()).is_ok());
    }
}
/// The tick frame stays unnamed so it keeps arriving on `EventSource.onmessage`, which is what
/// web/service.js already listens on. A named event would silently stop reaching it.
fn sse_frame(event: Option<&str>, line: &str) -> String {
    match event {
        Some(event) => format!("event: {event}\ndata: {line}\n\n"),
        None => format!("data: {line}\n\n"),
    }
}
pub fn sse_broadcast(printer: &str, line: &str) {
    sse_send(printer, sse_frame(None, line));
}
/// L8: console lines as they are recorded, as a named `console` event.
pub fn sse_broadcast_event(printer: &str, event: &str, line: &str) {
    sse_send(printer, sse_frame(Some(event), line));
}
/// The newest page wins (finish-2 F01, 2026-10-19). A closed tab's stream stays listed until a broadcast to
/// it fails, and a printer that sends nothing (an idle one, the placeholder of a service with no printer)
/// never broadcasts, so a reload or two filled the slots and every later page was refused with a 503. At the
/// limit the oldest stream is let go; if its page is still open it falls back to polling, as it would have.
fn sse_register(printer: &str) -> SseReceiver<String> {
    let (tx, rx) = sse_channel();
    let Ok(mut list) = sse_registry().lock() else {
        return rx;
    };
    if list.iter().filter(|(id, _)| id == printer).count() >= MAX_SSE_STREAMS_PER_PRINTER {
        if let Some(i) = list.iter().position(|(id, _)| id == printer) {
            list.remove(i);
        }
    }
    if list.len() >= MAX_SSE_STREAMS {
        list.remove(0);
    }
    list.push((printer.to_owned(), tx));
    rx
}
fn sse_stream(mut stream: TcpStream, printer: &str) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let rx = sse_register(printer);
    if write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n").is_err() {
        return;
    }
    let _ = stream.flush();
    loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(frame) => {
                if stream
                    .write_all(frame.as_bytes())
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    break;
                }
            }
            // let go for a newer page (sse_register): end here, and that page polls
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {
                if write!(stream, ": keep-alive\n\n")
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

fn send_with_cookie(
    stream: &mut TcpStream,
    response: Response,
    cookie: &str,
) -> std::io::Result<()> {
    write!(stream,"HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nSet-Cookie: {cookie}\r\n\r\n",
        response.code,response.mime,response.body.len())?;
    stream.write_all(&response.body)
}

fn send(stream: &mut TcpStream, response: Response, allow: &str) -> std::io::Result<()> {
    let reason = match response.code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    write!(stream,"HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nAllow: {}\r\n\r\n",
        response.code,reason,response.mime,response.body.len(),allow)?;
    stream.write_all(&response.body)
}

const MAX_COMMAND_BODY: usize = 4096;
const RESTORE_MAX_BODY: usize = 512 * 1024;
const PRESETS_MAX_BODY: usize = 16 * 1024;

#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    /// A queue upload that was written straight to this file instead of into `body`.
    spool: Option<std::path::PathBuf>,
}

/// What a queue upload must show before its bytes are accepted onto the disk: the desktop's own Host
/// and Origin and the farm token. Without these the body is refused unread.
struct UploadGate {
    dir: std::path::PathBuf,
    /// Where a model the person downloaded from its author is spooled (WO-10).
    models: std::path::PathBuf,
    host: String,
    token: Option<String>,
    /// Keys a slicer's OctoPrint upload is taken with (fleet::slicer_keys): its own upload key, and
    /// the farm key only when the owner allowed it.
    upload_keys: Vec<String>,
}
/// A sliced file can be tens of megabytes (a 52 MB one is in our folders); this is the ceiling.
const QUEUE_UPLOAD_MAX: usize = 300 * 1024 * 1024;
/// The largest published 3D model is 80 MB packed.
const MODEL_UPLOAD_MAX: usize = 400 * 1024 * 1024;

/// `/api/printer-models/<id>/upload`: a model file the person downloaded, streamed to disk like a queue file.
fn is_model_upload(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or("");
    path.strip_prefix("/api/printer-models/").is_some_and(|r| {
        r.strip_suffix("/upload")
            .is_some_and(|id| !id.is_empty() && !id.contains('/'))
    })
}

#[cfg(test)]
fn read_request(stream: &mut TcpStream, cfg: &Config) -> Result<HttpRequest> {
    read_request_gated(stream, cfg, None)
}

fn read_request_gated(
    stream: &mut TcpStream,
    cfg: &Config,
    gate: Option<&UploadGate>,
) -> Result<HttpRequest> {
    let begin = Instant::now();
    let timeout = Duration::from_millis(cfg.http_timeout_ms);
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 1024];
    loop {
        let remaining = timeout
            .checked_sub(begin.elapsed())
            .ok_or("request timed out; reconnect and send the complete request promptly")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("request could not be read: {e}; reconnect to the service"))?;
        if n == 0 {
            return Err("connection closed before the headers were complete".into());
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(header_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            if header_end + 4 > cfg.max_request_bytes {
                return Err(
                    "request headers exceed max_request_bytes; send a smaller request".into(),
                );
            }
            break;
        }
        if bytes.len() > cfg.max_request_bytes {
            return Err("request headers exceed max_request_bytes; send a smaller request".into());
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("request headers are incomplete")?;
    let text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "request headers must be UTF-8 or ASCII".to_owned())?;
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or("missing request line")?;
    let pieces = line.split_whitespace().collect::<Vec<_>>();
    if pieces.len() != 3
        || !["HTTP/1.0", "HTTP/1.1"].contains(&pieces[2])
        || !pieces[1].starts_with('/')
        || pieces[1].starts_with("//")
        || pieces[1].chars().any(char::is_control)
    {
        return Err("invalid HTTP request; use an origin-relative URL such as /api/live".into());
    }
    let method = pieces[0].to_owned();
    let target = pieces[1].to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err("folded request headers are not supported".into());
        }
        let (name, value) = line
            .split_once(':')
            .ok_or("malformed request header; expected name:value")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("request header name contains unsupported characters".into());
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim().to_owned();
        if value.chars().any(char::is_control) {
            return Err("request header value contains control characters".into());
        }
        if headers.insert(name.clone(), value).is_some() {
            return Err(format!("duplicate {name} request header"));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err("Transfer-Encoding is unsupported; send a bounded Content-Length".into());
    }
    // PUT joins POST as a body-carrying verb for PUT /api/presets; every other verb is still
    // refused a body outright.
    let takes_body = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
    let content_length = match headers.get("content-length") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "Content-Length must be a non-negative decimal integer".to_owned())?,
        None if takes_body => return Err(format!("{method} requires Content-Length")),
        None => 0,
    };
    // A file for the queue is streamed to disk, and only for a request that proves it is the desktop's.
    // WO-15 (OctoPrint-compatible upload): a slicer posting to /api/files/local holds an API key in
    // `X-Api-Key` instead of Origin; it gets the same disk spool, the key checked before a byte is kept.
    let model_upload = method == "POST" && is_model_upload(&target);
    let octo_upload = method == "POST" && target.split('?').next() == Some("/api/files/local");
    if method == "POST"
        && (target.split('?').next() == Some("/api/queue/upload") || model_upload || octo_upload)
    {
        let Some(gate) = gate else {
            return Err("request body exceeds the command limit: files are accepted only from the desktop window".into());
        };
        let token_ok = |given: Option<&String>| match (&gate.token, given) {
            (Some(e), Some(g)) => {
                e.len() == g.len()
                    && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
            }
            _ => false,
        };
        let key_ok = |given: Option<&String>| {
            given.is_some_and(|g| {
                gate.upload_keys.iter().any(|e| {
                    e.len() == g.len()
                        && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
                })
            })
        };
        let valid = if octo_upload {
            key_ok(headers.get("x-api-key"))
        } else {
            headers.get("host") == Some(&gate.host)
                && headers.get("origin") == Some(&format!("http://{}", gate.host))
                && token_ok(headers.get("x-stella-control-token"))
        };
        if !valid {
            return Err(if octo_upload {
                "the upload was refused: the slicer's API key did not match (Stella Scope, More, Slicer upload has the key) or slicer uploads are off".into()
            } else {
                "the upload was refused: it must come from the Stella Scope window on the computer"
                    .into()
            });
        }
        let limit = if model_upload {
            MODEL_UPLOAD_MAX
        } else {
            QUEUE_UPLOAD_MAX
        };
        if content_length == 0 || content_length > limit {
            return Err(format!(
                "request body exceeds the {} MB file limit",
                limit / (1024 * 1024)
            ));
        }
        let mut tag = [0_u8; 8];
        let _ = getrandom::fill(&mut tag);
        let spool_dir = if model_upload {
            &gate.models
        } else {
            &gate.dir
        };
        let _ = std::fs::create_dir_all(spool_dir);
        let file = spool_dir.join(format!(
            "{}.part",
            tag.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        let mut out =
            std::fs::File::create(&file).map_err(|e| format!("cannot keep the upload: {e}"))?;
        let first = &bytes[(header_end + 4).min(bytes.len())..];
        let mut got = first.len().min(content_length);
        let fail = |file: &std::path::Path, e: String| {
            let _ = std::fs::remove_file(file);
            Err(e)
        };
        if out.write_all(&first[..got]).is_err() {
            return fail(&file, "cannot write the upload".into());
        }
        let mut chunk = vec![0_u8; 256 * 1024];
        let started = Instant::now();
        while got < content_length {
            if started.elapsed() > Duration::from_secs(900) {
                return fail(&file, "the upload took too long".into());
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
            let take = chunk.len().min(content_length - got);
            match stream.read(&mut chunk[..take]) {
                Ok(0) | Err(_) => {
                    return fail(
                        &file,
                        "connection closed before the file was complete".into(),
                    )
                }
                Ok(n) => {
                    if out.write_all(&chunk[..n]).is_err() {
                        return fail(&file, "cannot write the upload".into());
                    }
                    got += n;
                }
            }
        }
        return Ok(HttpRequest {
            method,
            target,
            headers,
            body: Vec::new(),
            spool: Some(file),
        });
    }
    // A command is small. A farm backup is not: a farm of ten printers is more than 6 KB, and restoring it was refused as "too large" (finish-2 F06), so the one
    // route that takes a backup takes up to half a megabyte (no secret is in one, and nothing else gets the larger limit).
    // rc9: the presets carry the standard lists (40 materials, plates, maintenance jobs, the sheet log), more than 4 KB.
    let route = target.split('?').next().unwrap_or("");
    let body_limit = if route == "/api/farm/restore" { RESTORE_MAX_BODY } else if route.ends_with("/api/presets") { PRESETS_MAX_BODY } else { MAX_COMMAND_BODY };
    if content_length > body_limit {
        return Err(format!(
            "request body exceeds the {body_limit}-byte command limit"
        ));
    }
    if !takes_body && content_length != 0 {
        return Err("request bodies are accepted only for POST and PUT command requests, and PATCH for printers".into());
    }
    let total = header_end + 4 + content_length;
    while bytes.len() < total {
        let remaining = timeout
            .checked_sub(begin.elapsed())
            .ok_or("request timed out while reading the body")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let take = buf.len().min(total - bytes.len());
        let n = stream
            .read(&mut buf[..take])
            .map_err(|e| format!("request body could not be read: {e}"))?;
        if n == 0 {
            return Err("connection closed before the request body was complete".into());
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    if bytes.len() > total {
        return Err(
            "request contains bytes beyond the declared body; reconnect and send one request"
                .into(),
        );
    }
    Ok(HttpRequest {
        method,
        target,
        headers,
        body: bytes[header_end + 4..total].to_vec(),
        spool: None,
    })
}

#[cfg(test)]
pub fn request_line(stream: &mut TcpStream, cfg: &Config) -> Result<(String, String)> {
    let request = read_request(stream, cfg)?;
    Ok((request.method, request.target))
}

/// Phone access (src/lan.rs): what a request from the network listener may do. Pages without a
/// paired device get the pairing page, data gets 401; a paired control request is checked for
/// same-origin here and then handed on exactly as the loopback listener would present it.
fn lan_gate(
    request: &mut HttpRequest,
    cfg: &Config,
    path: &str,
    peer: &mut Option<IpAddr>,
) -> Option<Response> {
    if path == "/pair.js" || path == "/pair" {
        return api::asset("GET", path);
    }
    if request.method == "POST" && path == "/api/pair" {
        let host = request.headers.get("host").cloned().unwrap_or_default();
        if host.is_empty() || request.headers.get("origin") != Some(&format!("http://{host}")) {
            return Some(Response::json(
                403,
                json!({"error":"pairing requires the phone page's origin"}),
            ));
        }
        return None; // answered by the caller with its cookie
    }
    if crate::lan::device_from_cookie(
        cfg,
        request.headers.get("cookie"),
        request.headers.get("x-stella-peer"),
    )
    .is_none()
    {
        if path.starts_with("/api/") {
            return Some(Response::json(
                401,
                json!({"error":"this device is not paired; open / and enter the code from the desktop"}),
            ));
        }
        if let Some(r) = api::asset("GET", path) {
            if r.mime.starts_with("text/html") {
                return api::asset("GET", "/pair");
            }
            return Some(r);
        }
        return api::asset("GET", "/pair");
    }
    if path.starts_with("/api/pair/") {
        return Some(Response::json(
            403,
            json!({"error":"pairing is managed from the desktop"}),
        ));
    }
    let host = request.headers.get("host").cloned().unwrap_or_default();
    if let Some(origin) = request.headers.get("origin") {
        if origin != &format!("http://{host}") {
            return Some(Response::json(
                403,
                json!({"error":"cross-site request refused"}),
            ));
        }
    }
    let loopback = match cfg.bind.parse::<IpAddr>() {
        Ok(IpAddr::V6(a)) => format!("[{a}]:{}", cfg.port),
        _ => format!("{}:{}", cfg.bind, cfg.port),
    };
    request.headers.insert("host".into(), loopback.clone());
    if request.headers.contains_key("origin") {
        request
            .headers
            .insert("origin".into(), format!("http://{loopback}"));
    }
    *peer = Some(IpAddr::from([127, 0, 0, 1]));
    None
}

fn pairing_routes(
    request: &HttpRequest,
    cfg: &Config,
    path: &str,
    lan: bool,
) -> Option<(Response, Option<String>)> {
    if lan && request.method == "POST" && path == "/api/pair" {
        return Some(
            match crate::lan::pair(
                cfg,
                &request.body,
                request.headers.get("user-agent"),
                request.headers.get("x-stella-peer"),
            ) {
                Ok(key) => (
                    Response::json(200, json!({"paired":true})),
                    Some(format!(
                        "stella_device={key}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict"
                    )),
                ),
                Err(e) => (Response::json(403, json!({"error":e})), None),
            },
        );
    }
    if lan && path.starts_with("/api/eye/switch") {
        return Some((
            Response::json(
                403,
                json!({"error":"the failure watch is switched from the desktop"}),
            ),
            None,
        ));
    }
    if lan && path.starts_with("/api/notify") {
        return Some((
            Response::json(
                403,
                json!({"error":"WhatsApp alerts are set up from the desktop"}),
            ),
            None,
        ));
    }
    if lan
        || !(path.starts_with("/api/pair")
            || path.starts_with("/api/notify")
            || path == "/api/eye/switch")
    {
        return None;
    }
    // Desktop side: a POST must come from the desktop page itself.
    let own = format!("http://{}:{}", cfg.bind, cfg.port);
    if request.method == "POST" && request.headers.get("origin") != Some(&own) {
        return Some((
            Response::json(
                403,
                json!({"error":"this change comes from the Stella Scope window on the computer only"}),
            ),
            None,
        ));
    }
    let r = match (request.method.as_str(), path) {
        ("GET", "/api/pair/devices") => Ok(crate::lan::devices(cfg)),
        ("GET", "/api/pair/remote") => Ok(crate::remote::status(cfg)),
        ("POST", "/api/pair/new") if cfg.lan_port.is_none() => {
            Err("Phone access is switched off. Turn it on from the Phone page, then close Stella Scope and open it again.".to_owned())
        }
        ("POST", "/api/pair/new") => {
            crate::lan::new_code(cfg).map(|v| crate::remote::with_pair_qr(cfg, v))
        }
        ("POST", "/api/pair/revoke") => crate::lan::revoke(cfg, &request.body),
        ("GET", "/api/notify") => Ok(crate::notify::settings(cfg)),
        ("POST", "/api/notify") => crate::notify::save(cfg, &request.body),
        ("POST", "/api/eye/switch") => crate::remote::eye_switch(cfg, &request.body),
        ("POST", "/api/notify/events") => crate::notify::save_events(cfg, &request.body),
        ("POST", "/api/notify/ntfy") => crate::notify::save_ntfy(cfg, &request.body),
        ("POST", "/api/notify/webhook") => crate::notify::save_webhook(cfg, &request.body),
        ("POST", "/api/notify/test") => Ok(
            json!({"queued":crate::notify::event("test", "", "", "Stella: this is a test message from Stella Scope. Alerts are working.")}),
        ),
        _ => Err("unknown pairing route".to_owned()),
    };
    Some((
        match r {
            Ok(v) => Response::json(200, v),
            Err(e) => Response::json(400, json!({"error":e})),
        },
        None,
    ))
}

fn desktop_host(request: &HttpRequest, cfg: &Config) -> bool {
    request.headers.get("host") == Some(&cfg.host())
}

/// Farm-level routes, the ones that belong to no one printer: the list, and adding, changing and removing
/// printers. Writes keep the rules of every other control write: the loopback desktop only, exact Host
/// and Origin, JSON, and the control token; none of it is reachable over the phone link.
fn farm_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str, lan: bool) -> Option<Response> {
    if path != "/api/printers" && !path.starts_with("/api/printers/") {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    if request.method == "GET" && path == "/api/printers" {
        let mut v = farm.summary();
        // the spool on each printer (2026-10-13): what is loaded, shown on its tile and page
        let loaded = crate::fleet::loaded(farm);
        if let Some(rows) = v["printers"].as_array_mut() {
            for row in rows {
                let spool = row["id"]
                    .as_str()
                    .and_then(|id| loaded.get(id))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                row["spool"] = spool;
            }
        }
        if lan {
            v["token"] = serde_json::Value::Null;
            v["writable"] = json!(false);
        }
        return Some(Response::json(200, v));
    }
    if lan {
        return fail(403, "printers are added, renamed and removed from the desktop window, not over the phone link");
    }
    if !matches!(request.method.as_str(), "POST" | "PATCH" | "DELETE") {
        return fail(
            405,
            "use GET to list printers, POST to add one, PATCH or DELETE /api/printers/<id>",
        );
    }
    let host = farm.base.host();
    if request.headers.get("origin") != Some(&format!("http://{host}")) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    if request.method != "DELETE"
        && request
            .headers
            .get("content-type")
            .is_none_or(|v| v != "application/json")
    {
        return fail(415, "Content-Type must be exactly application/json");
    }
    let supplied = request.headers.get("x-stella-control-token");
    let valid = match (farm.token.as_ref(), supplied) {
        (Some(e), Some(g)) => {
            e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    };
    if !valid {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    let target_id = path
        .strip_prefix("/api/printers/")
        .filter(|s| !s.is_empty() && !s.contains('/'));
    let row = |id: &str| {
        farm.summary()["printers"]
            .as_array()
            .and_then(|a| a.iter().find(|p| p["id"] == id).cloned())
    };
    match (request.method.as_str(), target_id) {
        // Read-only network scan: which printers answer, so adding one is a pick, not typing an address.
        ("POST", Some("discover")) => {
            let v: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
            let subnet =
                match v["subnet"].as_str().filter(|s| !s.trim().is_empty()) {
                    Some(s) => s.trim().to_owned(),
                    None => match crate::discover::local_subnet() {
                        Some(s) => s,
                        None => return fail(
                            400,
                            "this machine's network was not found; type a subnet such as 192.168.1",
                        ),
                    },
                };
            let hosts = match crate::discover::hosts_of(&subnet) {
                Ok(h) => h,
                Err(e) => return fail(400, &e),
            };
            static SCANNING: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if SCANNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
                return fail(429, "a search is already running; wait for it to finish");
            }
            let added: Vec<(String, String, String)> = farm
                .visible()
                .iter()
                .map(|p| {
                    (
                        p.entry.address.clone(),
                        p.id.clone(),
                        if p.entry.driver.is_empty() {
                            "moonraker".to_owned()
                        } else {
                            p.entry.driver.clone()
                        },
                    )
                })
                .collect();
            let started = std::time::Instant::now();
            let (found, cameras) =
                crate::discover::scan_all(&hosts, &crate::discover::Ports::default(), &added, 300);
            SCANNING.store(false, std::sync::atomic::Ordering::Release);
            Some(Response::json(
                200,
                json!({"subnet":subnet,"scanned":hosts.len(),"seconds":started.elapsed().as_secs_f64(),"printers":found,"cameras":cameras}),
            ))
        }
        // Try a connection with what was typed; nothing is saved.
        ("POST", Some("test")) => {
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid JSON: {e}")),
            };
            let text = |k: &str| {
                v[k].as_str()
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default()
            };
            let credentials: BTreeMap<String, String> = v["credentials"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, x)| x.as_str().map(|t| (k.clone(), t.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            let camera = text("camera");
            Some(Response::json(
                200,
                crate::discover::test(
                    &farm.base,
                    &text("driver"),
                    &text("address"),
                    &credentials,
                    Some(camera.as_str()),
                ),
            ))
        }
        ("POST", None) => {
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid printer JSON: {e}")),
            };
            let text = |k: &str| {
                v[k].as_str()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
            };
            let pair = |k: &str| -> Option<[f64; 2]> {
                let a = v[k].as_array()?;
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
            };
            let entry = crate::config::PrinterEntry {
                id: text("id").unwrap_or_default(),
                name: text("name").unwrap_or_default(),
                driver: text("driver").unwrap_or_default(),
                address: text("address").unwrap_or_default(),
                credentials_ref: None,
                database: None,
                model_asset: text("model_asset"),
                model: text("model"),
                accent: text("accent"),
                camera: text("camera"),
                bed_size_mm: pair("bed_size_mm"),
                probe_offset_mm: pair("probe_offset_mm"),
                nozzle_mm: v["nozzle_mm"].as_f64(),
                slicer_model: text("slicer_model"),
                material: text("material"),
                cad_model: text("cad_model")
                    .filter(|m| crate::printer_models::machine(m).is_some()),
                cad_calibration: None,
                cad_options: None,
                demo_start_at: v["demo_start_at"].as_f64().filter(|f| {
                    (0.0..=1.0).contains(f) && text("driver").as_deref() == Some("demo")
                }),
                demo_paused: v["demo_paused"].as_bool().unwrap_or(false)
                    && text("driver").as_deref() == Some("demo"),
                demo_since: (text("driver").as_deref() == Some("demo")).then(crate::model::now),
                plug: match text("plug") {
                    Some(p) => match crate::plugs::parse(&p) {
                        Ok(_) => Some(p),
                        Err(e) => return fail(400, &e),
                    },
                    None => None,
                },
            };
            let secret: Option<BTreeMap<String, String>> = v["credentials"].as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, x)| x.as_str().map(|t| (k.clone(), t.to_owned())))
                    .collect()
            });
            match farm.add(entry, secret) {
                Ok(p) => Some(Response::json(
                    200,
                    json!({"printer":row(&p.id),"count":farm.list().len()}),
                )),
                Err(e) => fail(400, &e),
            }
        }
        // The demo farm, added and removed in one press (polish WO-02). Only demo printers are touched.
        // F03 (2026-10-19): the press also fetches the demo's open models, one press and no second button, when the page says the licences
        // were shown (`accept_licence`, as for every fetch). Only the demo printers' models are asked for.
        ("POST", Some("demo-farm")) => {
            let accepted = serde_json::from_slice::<serde_json::Value>(&request.body).is_ok_and(|b| b["accept_licence"] == true);
            match farm.add_demo_farm() {
                Ok(mut v) => {
                    if accepted {
                        let lib = farm.library();
                        let want: Vec<(String, Option<serde_json::Value>)> = farm.list().iter().filter(|p| p.entry.driver == "demo").filter_map(|p| p.entry.cad_model.clone().map(|m| (m, p.entry.cad_options.clone()))).collect();
                        let open = crate::printer_models::fetch_selection(&crate::printer_models::wanted(&want, &lib), &[]);
                        v["fetch"] = if open.is_empty() {
                            crate::printer_models::fetch_status(&lib)
                        } else {
                            crate::printer_models::start_fetch(&lib, &open, crate::printer_models::Source::Published, true).unwrap_or_else(|e| json!({"state":"error","error":e}))
                        };
                    }
                    Some(Response::json(200, v))
                }
                Err(e) => fail(400, &e),
            }
        }
        ("DELETE", Some("demo-farm")) => match farm.remove_demo_farm() {
            Ok(v) => Some(Response::json(200, v)),
            Err(e) => fail(400, &e),
        },
        ("PATCH", Some(id)) => {
            let mut v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid printer JSON: {e}")),
            };
            // the login is typed here, kept apart from the other settings (never in the configuration file)
            if let Some(c) = v.as_object_mut().and_then(|o| o.remove("credentials")) {
                let secret: BTreeMap<String, String> = c.as_object().map(|o| o.iter().filter_map(|(k, x)| x.as_str().map(|t| (k.clone(), t.to_owned()))).collect()).unwrap_or_default();
                if let Err(e) = farm.set_login(id, secret) {
                    return fail(400, &e);
                }
                if v.as_object().is_some_and(|o| o.is_empty()) {
                    return Some(Response::json(200, json!({"printer":row(id)})));
                }
            }
            match farm.patch(id, &v) {
                Ok(_) => Some(Response::json(200, json!({"printer":row(id)}))),
                Err(e) => fail(400, &e),
            }
        }
        ("DELETE", Some(id)) => {
            let delete_data = request
                .target
                .split_once('?')
                .is_some_and(|(_, q)| q.split('&').any(|kv| kv == "delete_data=1"));
            match farm.remove(id, delete_data) {
                Ok(v) => Some(Response::json(200, v)),
                Err(e) => fail(400, &e),
            }
        }
        _ => fail(
            405,
            "use POST /api/printers, PATCH /api/printers/<id> or DELETE /api/printers/<id>",
        ),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(v) => {
                    out.push(v);
                    i += 3;
                }
                None => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(target: &str, key: &str) -> Option<String> {
    target
        .split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(percent_decode))
}

/// The farm's alarm feed, spools, analytics and backup (2026-10-13). Reading the feed, the spools and
/// the analytics is open to any paired device; changing anything, and the backup (it holds every
/// printer's address), is the desktop window's, with the farm token, like adding a printer.
fn fleet_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str, lan: bool) -> Option<Response> {
    let route = path == "/api/alerts/feed"
        || path == "/api/spools"
        || path.starts_with("/api/spools/")
        || path == "/api/analytics"
        || path == "/api/farm/backup"
        || path == "/api/farm/restore"
        || path == "/api/slicer"
        || path.starts_with("/api/plugs/");
    if !route {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    let num = |key: &str| query_param(&request.target, key).and_then(|v| v.parse::<f64>().ok());
    match (request.method.as_str(), path) {
        ("GET", "/api/alerts/feed") => {
            let printer = query_param(&request.target, "printer").filter(|p| !p.is_empty());
            return Some(Response::json(
                200,
                crate::notify::feed(
                    num("after").unwrap_or(0.0).max(0.0) as u64,
                    printer.as_deref(),
                    num("limit").unwrap_or(200.0) as usize,
                ),
            ));
        }
        ("GET", "/api/spools") => return Some(Response::json(200, crate::fleet::spools(farm))),
        ("GET", "/api/analytics") => {
            return Some(Response::json(
                200,
                crate::fleet::analytics(farm, num("days").unwrap_or(30.0)),
            ))
        }
        _ => {}
    }
    if lan {
        return fail(403, "spools, backups and restores are handled from the Stella Scope window on the computer, not over the phone link");
    }
    if request.method == "GET" && path == "/api/slicer" {
        if !desktop_host(request, &farm.base) {
            return fail(
                403,
                "the slicer key is read from the Stella Scope window on the computer",
            );
        }
        return Some(Response::json(200, crate::fleet::slicer_settings(farm)));
    }
    if request.method == "GET" && path == "/api/farm/backup" {
        if !desktop_host(request, &farm.base) {
            return fail(
                403,
                "the backup is read from the Stella Scope window on the computer",
            );
        }
        return Some(Response::json(200, crate::fleet::backup(farm)));
    }
    if !matches!(request.method.as_str(), "POST" | "PATCH" | "DELETE") {
        return fail(405, "use GET to read, POST, PATCH or DELETE to change");
    }
    let host = farm.base.host();
    if request.headers.get("origin") != Some(&format!("http://{host}")) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    if request.method != "DELETE"
        && request
            .headers
            .get("content-type")
            .is_none_or(|v| v != "application/json")
    {
        return fail(415, "Content-Type must be exactly application/json");
    }
    let valid = match (
        farm.token.as_ref(),
        request.headers.get("x-stella-control-token"),
    ) {
        (Some(e), Some(g)) => {
            e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    };
    if !valid {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    let r = if path == "/api/plugs/discover" && request.method == "POST" {
        let v: serde_json::Value =
            serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
        let subnet = v["subnet"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_owned())
            .or_else(crate::discover::local_subnet);
        match subnet
            .ok_or_else(|| "say which network to search, like 192.168.1.0/24".to_owned())
            .and_then(|s| crate::discover::hosts_of(&s).map(|h| (s, h)))
        {
            Ok((subnet, hosts)) => {
                let started = std::time::Instant::now();
                Ok(
                    json!({"subnet":subnet,"plugs":crate::plugs::discover(&hosts),"seconds":started.elapsed().as_secs_f64()}),
                )
            }
            Err(e) => Err(e),
        }
    } else if let Some(id) = path
        .strip_prefix("/api/plugs/")
        .filter(|s| !s.contains('/') && request.method == "POST")
    {
        crate::fleet::plug_switch(farm, id, &request.body)
    } else if path == "/api/slicer" && request.method == "POST" {
        crate::fleet::slicer_write(farm, &request.body)
    } else if path == "/api/farm/restore" && request.method == "POST" {
        crate::fleet::restore(farm, &request.body)
    } else if path == "/api/spools" || path.starts_with("/api/spools/") {
        let id = path
            .strip_prefix("/api/spools/")
            .filter(|s| !s.is_empty() && !s.contains('/'));
        crate::fleet::spools_write(farm, &request.method, id, &request.body)
    } else {
        Err("unknown farm route".into())
    };
    Some(match r {
        Ok(v) => Response::json(200, v),
        Err(e) => Response::json(400, json!({"error":e})),
    })
}

/// The queue and its jobs (farm board WO-08). Reading the queue is open to any paired device; changing
/// it is the desktop window's, with the farm token, like adding a printer.
fn queue_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str, lan: bool) -> Option<Response> {
    let route = path == "/api/queue"
        || path == "/api/queue/upload"
        || path == "/api/queue/reorder"
        || path == "/api/queue/jobs"
        || path.starts_with("/api/queue/jobs/");
    if !route {
        return None;
    }
    let drop_spool = || {
        if let Some(f) = &request.spool {
            let _ = std::fs::remove_file(f);
        }
    };
    let fail = |code: u16, e: &str| {
        drop_spool();
        Some(Response::json(code, json!({"error":e})))
    };
    let Some(q) = farm.queue() else {
        return fail(503, &farm.queue_error());
    };
    if request.method == "GET" && path == "/api/queue" {
        return Some(match crate::queue_flow::overview(farm, &q) {
            Ok(v) => Response::json(200, v),
            Err(e) => Response::json(500, json!({"error":e})),
        });
    }
    if lan {
        return fail(403, "the queue is changed from the Stella Scope window on the computer, not over the phone link");
    }
    let host = farm.base.host();
    if request.headers.get("origin") != Some(&format!("http://{host}")) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    let supplied = request.headers.get("x-stella-control-token");
    let valid = match (farm.token.as_ref(), supplied) {
        (Some(e), Some(g)) => {
            e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    };
    if !valid {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    let json_body = || -> std::result::Result<serde_json::Value, String> {
        if request
            .headers
            .get("content-type")
            .is_none_or(|v| v != "application/json")
        {
            return Err("Content-Type must be exactly application/json".into());
        }
        serde_json::from_slice(&request.body).map_err(|e| format!("invalid JSON: {e}"))
    };
    let queue_json = |r: crate::Result<crate::queue::Job>| match r {
        Ok(j) => {
            let profiles: Vec<_> = farm
                .visible()
                .iter()
                .map(|p| crate::queue_flow::profile(p))
                .collect();
            Some(Response::json(
                200,
                json!({"job":crate::queue_flow::job_json(&j, &profiles)}),
            ))
        }
        Err(e) => Some(Response::json(400, json!({"error":e}))),
    };
    match (request.method.as_str(), path) {
        ("POST", "/api/queue/upload") => {
            let Some(file) = request.spool.clone() else {
                return fail(400, "no file arrived");
            };
            let name = query_param(&request.target, "name").unwrap_or_default();
            let copies = query_param(&request.target, "copies").and_then(|c| c.parse().ok()).unwrap_or(1);
            queue_json(q.add_upload(&file, &name, copies))
        }
        ("POST", "/api/queue/jobs") => {
            // A file that is already on a printer: it runs there, and its facts come from the printer.
            let v = match json_body() {
                Ok(v) => v,
                Err(e) => return fail(400, &e),
            };
            let (Some(pid), Some(file)) = (v["printer"].as_str(), v["file"].as_str()) else {
                return fail(400, "say which printer and which file");
            };
            let Some(printer) = farm.get(pid) else {
                return fail(404, &format!("no printer called {pid}"));
            };
            let mut meta = crate::gcode_meta::Meta::default();
            if let Ok(m) = crate::driver::open(&printer.cfg).and_then(|d| d.file_metadata(file)) {
                meta.slicer = m["slicer"].as_str().map(str::to_owned);
                meta.nozzle_mm = m["nozzle_diameter"].as_f64();
                if let Some(t) = m["filament_type"].as_str() {
                    meta.filament = t.split(';').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect();
                }
                meta.max_height_mm = m["object_height"].as_f64();
            }
            queue_json(q.add_on_printer(pid, file, &meta, v["copies"].as_u64().unwrap_or(1) as u32))
        }
        ("POST", "/api/queue/reorder") => {
            let v = match json_body() {
                Ok(v) => v,
                Err(e) => return fail(400, &e),
            };
            let ids: Vec<i64> = v["order"].as_array().map(|a| a.iter().filter_map(serde_json::Value::as_i64).collect()).unwrap_or_default();
            Some(match q.reorder(&ids) {
                Ok(()) => Response::json(200, json!({"ok":true})),
                Err(e) => Response::json(400, json!({"error":e})),
            })
        }
        ("PATCH", p) if p.starts_with("/api/queue/jobs/") => {
            let Ok(id) = p["/api/queue/jobs/".len()..].parse::<i64>() else {
                return fail(400, "job ids are numbers");
            };
            let v = match json_body() {
                Ok(v) => v,
                Err(e) => return fail(400, &e),
            };
            queue_json(q.patch(id, &v))
        }
        ("DELETE", p) if p.starts_with("/api/queue/jobs/") => {
            let Ok(id) = p["/api/queue/jobs/".len()..].parse::<i64>() else {
                return fail(400, "job ids are numbers");
            };
            Some(match q.remove(id) {
                Ok(()) => Response::json(200, json!({"removed":id})),
                Err(e) => Response::json(400, json!({"error":e})),
            })
        }
        _ => fail(405, "use GET /api/queue, POST /api/queue/upload, POST /api/queue/jobs, POST /api/queue/reorder, PATCH or DELETE /api/queue/jobs/<id>"),
    }
}

/// WO-15 (2026-10-13), the owner: "better than Watchtower, by far": slicers speak OctoPrint's little upload
/// handshake, and Watchtower offers nothing like opening the farm to them. `GET /api/version` answers
/// the host check, and `POST /api/files/local` (X-Api-Key: the farm control token) drops the file into
/// the farm queue, which routes it to the one printer it fits. `print=true` is accepted but never
/// armed on its own: starting a job stays an in-app action, because the plate check stands between a
/// file and a machine.
fn octoprint_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str) -> Option<Response> {
    if request.method == "GET" && path == "/api/version" {
        // A slicer's "Test" button sends its API key to this address and trusts a 200. A key that is wrong (or slicer uploads that are off) used
        // to read "works" here and fail only at the first upload (finish-2 F06): a key that was sent is checked, as OctoPrint itself does.
        if let Some(given) = request.headers.get("x-api-key") {
            let ok = crate::fleet::slicer_keys(farm).iter().any(|e| {
                e.len() == given.len() && e.bytes().zip(given.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
            });
            if !ok {
                return Some(Response::json(
                    403,
                    json!({"error":"the slicer's API key did not match (Stella Scope, More, Slicer upload has the key) or slicer uploads are off"}),
                ));
            }
        }
        return Some(Response::json(
            200,
            json!({
                "api":"0.1","server":env!("CARGO_PKG_VERSION"),
                "text":"Stella Scope farm queue (OctoPrint-compatible upload)"
            }),
        ));
    }
    if !(request.method == "POST" && path == "/api/files/local") {
        return None;
    }
    let drop_spool = || {
        if let Some(f) = &request.spool {
            let _ = std::fs::remove_file(f);
        }
    };
    let fail = |code: u16, e: &str| {
        drop_spool();
        Some(Response::json(code, json!({"error":e})))
    };
    let Some(q) = farm.queue() else {
        return fail(503, &farm.queue_error());
    };
    let Some(spool) = &request.spool else {
        return fail(400, "no file arrived");
    };
    let Some(ct) = request.headers.get("content-type") else {
        return fail(400, "a slicer's OctoPrint upload is multipart/form-data");
    };
    let body = match std::fs::read(spool) {
        Ok(b) => b,
        Err(e) => return fail(500, &format!("cannot read the spooled upload: {e}")),
    };
    let (name, bytes, _select, _print) = match octoprint_multipart(&body, ct) {
        Ok(v) => v,
        Err(e) => return fail(400, &e),
    };
    // The spool holds the whole multipart envelope; the queue wants the file's bytes alone.
    let solo = spool.with_extension("file");
    if let Err(e) = std::fs::write(&solo, &bytes) {
        return fail(500, &format!("cannot keep the upload: {e}"));
    }
    let Some(job) = (match q.add_upload(&solo, &name, 1) {
        Ok(j) => Some(j),
        Err(e) => {
            return fail(
                400,
                &format!("the file was not accepted into the queue: {e}"),
            )
        }
    }) else {
        return None;
    };
    let profiles: Vec<_> = farm
        .visible()
        .iter()
        .map(|p| crate::queue_flow::profile(p))
        .collect();
    let fits = profiles
        .iter()
        .filter(|p| crate::queue::fit(&job.needs(), false, p).ok)
        .count();
    Some(Response::json(
        201,
        json!({
            "files":{"local":{"name":job.file_name,"path":job.file_name,"display":name,"origin":"local"}},
            "done":true,"effectiveSelect":false,"effectivePrint":false,
            "job":crate::queue_flow::job_json(&job, &profiles),
            "note":format!("file accepted into the farm queue; {fits} printer(s) fit it. Starting a print stays at the farm: select/print from the app after the plate check.")
        }),
    ))
}

/// Pull one `file` part and the `select`/`print` flags out of a multipart/form-data body, with no
/// new crate: boundary from the Content-Type, parts split on it, the file's slice taken between its
/// part head and the closing CRLF.
fn octoprint_multipart(body: &[u8], content_type: &str) -> Result<(String, Vec<u8>, bool, bool)> {
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .map(str::trim)
        .map(str::to_owned)
        .filter(|b| !b.is_empty())
        .ok_or("multipart/form-data without a boundary")?;
    let marker = format!("--{boundary}").into_bytes();
    let mut name = String::new();
    let mut bytes = Vec::new();
    let mut want_print = false;
    let mut want_select = false;
    let mut at = 0;
    while let Some(pos) = find_subslice(&body[at..], &marker) {
        let start = at + pos + marker.len();
        let end = find_subslice(&body[start..], &marker)
            .map(|p| start + p)
            .unwrap_or(body.len());
        let part = &body[start..end];
        at = end;
        let Some(head_end) = find_subslice(part, b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&part[..head_end]).into_owned();
        let content = &part[head_end + 4..part.len().saturating_sub(2)]; // trailing CRLF
        let lhead = head.to_lowercase();
        if lhead.contains("name=\"file\"") {
            let dn = head
                .split("filename=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or("upload.gcode");
            name = dn.rsplit(['/', '\\']).next().unwrap_or(dn).to_owned();
            bytes = content.to_vec();
        } else if lhead.contains("name=\"print\"") {
            want_print = content == b"true";
        } else if lhead.contains("name=\"select\"") {
            want_select = content == b"true";
        }
    }
    if bytes.is_empty() {
        return Err("no file part in the upload".into());
    }
    Ok((name, bytes, want_select, want_print))
}
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One printer's 3D model: what it is, how it sits and where it stands now (`/api/p/<id>/model`), and the
/// calibration that would make the model read a position the person chose (`/api/p/<id>/model/match`).
/// Read-only; the calibration is saved through the farm's printer settings.
fn model_route(
    request: &HttpRequest,
    farm: &Arc<Farm>,
    printer: &Arc<Printer>,
    path: &str,
) -> Response {
    let lib = farm.library();
    let now = crate::model::now();
    if path == "/api/model" {
        // ?ts=: the model as the recorder had it then (a rewound page; finish-2 F02).
        let recorded = query_param(&request.target, "ts")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .and_then(|ts| {
                let db = crate::store::Store::open(&printer.cfg.database, true).ok()?;
                crate::api::workspace::recorded_tick(&db, ts)
            });
        let mut v = crate::farm::model_json_at(printer, &lib, now, recorded.as_ref());
        v["recorded"] = json!(recorded.is_some());
        v["id"] = json!(printer.id);
        v["default"] = json!(farm.first().id == printer.id);
        // The pose tickers ask again every half second and have the scene already: ?scene=0 leaves it out.
        if query_param(&request.target, "scene").as_deref() == Some("0") {
            v["scene"] = serde_json::Value::Null;
        }
        return Response::json(200, v);
    }
    let fail = |e: &str| Response::json(400, json!({"error":e}));
    let Some(id) = printer.entry.cad_model.as_deref() else {
        return fail("this printer has no model");
    };
    let Some(profile) = crate::printer_models::profile(id) else {
        return fail("no movement is registered for this model, so there is nothing to match");
    };
    let num = |k: &str| {
        query_param(&request.target, k)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let (Some(x), Some(y), Some(z)) = (num("x"), num("y"), num("z")) else {
        return fail("give the model's x, y and z in millimetres");
    };
    let current = printer
        .entry
        .cad_calibration
        .as_ref()
        .and_then(|v| crate::printer_models::Calibration::from_value(v).ok())
        .unwrap_or_else(|| crate::printer_models::Calibration::starting(profile));
    let position = crate::farm::position_of(printer, now).0;
    if position.iter().any(Option::is_none) {
        return fail(
            "the printer has not reported a position yet, so the model cannot be matched to it",
        );
    }
    let m = crate::printer_models::matched(profile, &current, position, [x, y, z]);
    Response::json(
        200,
        json!({"calibration":m.to_value(),"pose":crate::printer_models::pose(profile, &m, position)}),
    )
}

/// The model library (`/api/printer-models`): the catalogue is open to read, a model's file is served to
/// any page, and installing or removing one is the desktop window's with the farm token.
fn models_route(
    request: &HttpRequest,
    farm: &Arc<Farm>,
    path: &str,
    lan: bool,
) -> Option<Response> {
    let rest = path.strip_prefix("/api/printer-models")?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let drop_spool = || {
        if let Some(f) = &request.spool {
            let _ = std::fs::remove_file(f);
        }
    };
    let fail = |code: u16, e: &str| {
        drop_spool();
        Some(Response::json(code, json!({"error":e})))
    };
    let lib = farm.library();
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if request.method == "GET" {
        return match parts.as_slice() {
            [] => {
                let mut v = crate::printer_models::list(&lib);
                v["source"] = json!(farm.model_source());
                v["library_has"] = json!(lib.installed_ids());
                Some(Response::json(200, v))
            }
            // Polish WO-03: the models this farm's printers draw that are not here yet, what can be fetched on one press, and
            // where a fetch stands.
            ["wanted"] => {
                let want: Vec<(String, Option<serde_json::Value>)> = farm.list().iter().filter_map(|p| p.entry.cad_model.clone().map(|m| (m, p.entry.cad_options.clone()))).collect();
                let mut v = crate::printer_models::wanted(&want, &lib);
                v["fetch"] = crate::printer_models::fetch_status(&lib);
                // F03: what each printer's picture waits for, so a tile can say whether its model is arriving.
                let by_printer: Vec<(String, String, Option<serde_json::Value>)> = farm.list().iter().filter_map(|p| p.entry.cad_model.clone().map(|m| (p.id.clone(), m, p.entry.cad_options.clone()))).collect();
                v["needs"] = crate::printer_models::needs(&by_printer, &lib);
                Some(Response::json(200, v))
            }
            // F03: the open licences of the demo farm's models in one line, shown before the button that fetches them is pressed.
            ["demo-farm"] => {
                let want: Vec<(String, Option<serde_json::Value>)> = crate::demo::demo_farm().into_iter().filter_map(|e| e.cad_model.clone().map(|m| (m, e.cad_options.clone()))).collect();
                Some(Response::json(200, crate::printer_models::licence_line(&want)))
            }
            ["fetch-open"] => Some(Response::json(200, crate::printer_models::fetch_status(&lib))),
            // Polish WO-01: the options (toolchangers, toolheads, add-ons) and their files.
            ["options"] => Some(Response::json(
                200,
                crate::printer_models::options_list(&lib),
            )),
            ["options", id, "scene"] => Some(match crate::printer_models::option(id) {
                Some(o) => Response::json(200, crate::printer_models::option_scene(o, &lib)),
                None => Response::json(
                    404,
                    json!({"error":format!("{id} is not an option in the catalogue")}),
                ),
            }),
            ["options", id] => Some(match crate::printer_models::option(id) {
                Some(o) => Response::json(200, crate::printer_models::option_detail(o, &lib)),
                None => Response::json(
                    404,
                    json!({"error":format!("{id} is not an option in the catalogue")}),
                ),
            }),
            ["parts", id, "file"] => Some(match lib.part_file(id).map(|(p, _)| std::fs::read(p)) {
                Some(Ok(body)) => Response {
                    code: 200,
                    mime: "application/octet-stream",
                    body,
                },
                Some(Err(e)) => {
                    Response::json(500, json!({"error":format!("cannot read the file: {e}")}))
                }
                None => Response::json(404, json!({"error":"this file is not installed"})),
            }),
            // Finish-2 F09 (2026-10-19): who is credited for every model this build carries (About), and every Prusa machine with its defaults and,
            // where no 3D model draws it, why.
            ["credits"] => Some(Response::json(200, crate::printer_models::credits())),
            ["prusa"] => Some(Response::json(200, crate::prusa_machines::json())),
            // Polish WO-06 item 1: the maker's notice for a model (finish-2 F09: read in its sheet and in About; the models Prusa shares are in the build).
            [id, "notice"] => Some(match crate::printer_models::machine(id).and_then(|m| crate::printer_models::notice_text(m).map(|t| (m, t))) {
                Some((m, text)) => Response::json(200, json!({"id":id,"label":m["label"],"text":text,"licence":m["licence"]})),
                None => Response::json(404, json!({"error":format!("{id} has no notice")})),
            }),
            [id, "options"] => Some(match crate::printer_models::machine(id) {
                Some(_) => Response::json(200, crate::printer_models::options_for(id, &lib)),
                None => Response::json(
                    404,
                    json!({"error":format!("{id} is not a model in the catalogue")}),
                ),
            }),
            [id] => Some(match crate::printer_models::machine(id) {
                Some(m) => {
                    let mut v = crate::printer_models::view(m, &lib);
                    v["record"] = lib.record(id).unwrap_or(serde_json::Value::Null);
                    Response::json(200, v)
                }
                None => Response::json(
                    404,
                    json!({"error":format!("{id} is not a model in the catalogue")}),
                ),
            }),
            [id, "thumb"] => Some(match crate::printer_models::thumb_bytes(id) {
                Some(body) => Response {
                    code: 200,
                    mime: "image/webp",
                    body,
                },
                None => Response::json(
                    404,
                    json!({"error":"no picture of this machine comes with Stella Scope"}),
                ),
            }),
            [id, "motion"] => Some(match crate::printer_models::profile(id) {
                Some(p) => Response::json(200, p.clone()),
                None => Response::json(
                    404,
                    json!({"error":"no movement is registered for this model"}),
                ),
            }),
            [id, "file"] => Some(match lib.model_file(id).map(|(p, _)| std::fs::read(p)) {
                Some(Ok(body)) => Response {
                    code: 200,
                    mime: "application/octet-stream",
                    body,
                },
                Some(Err(e)) => {
                    Response::json(500, json!({"error":format!("cannot read the model: {e}")}))
                }
                None => Response::json(404, json!({"error":"this model is not installed"})),
            }),
            _ => Some(Response::json(404, json!({"error":"no such model route"}))),
        };
    }
    if lan {
        return fail(403, "models are installed from the Stella Scope window on the computer, not over the phone link");
    }
    if request.headers.get("origin") != Some(&format!("http://{}", farm.base.host())) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    let valid = match (
        farm.token.as_ref(),
        request.headers.get("x-stella-control-token"),
    ) {
        (Some(e), Some(g)) => {
            e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    };
    if !valid {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    let result = match (request.method.as_str(), parts.as_slice()) {
        ("POST", [id, "install"]) => {
            if request.headers.get("content-type").is_none_or(|v| v != "application/json") {
                return fail(415, "Content-Type must be exactly application/json");
            }
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid JSON: {e}")),
            };
            let text = v["source"].as_str().map(str::to_owned).or_else(|| farm.model_source()).unwrap_or_default();
            crate::printer_models::Source::parse(&text).and_then(|src| {
                let r = lib.install_from(id, &src, v["accept_licence"].as_bool().unwrap_or(false))?;
                farm.set_model_source(&text);
                Ok(r)
            })
        }
        ("POST", [id, "upload"]) => {
            let Some(file) = request.spool.clone() else {
                return fail(400, "no file arrived");
            };
            let accepted = query_param(&request.target, "accept").is_some_and(|a| a == "1");
            let name = query_param(&request.target, "name").unwrap_or_default();
            let origin = format!("a file you downloaded{}", if name.is_empty() { String::new() } else { format!(" ({name})") });
            let r = match std::fs::File::open(&file) {
                Ok(mut f) => lib.install_file(id, &mut f, &origin, accepted),
                Err(e) => Err(format!("cannot read the upload: {e}")),
            };
            drop_spool();
            r
        }
        // Polish WO-03: one press fetches every open model the farm's printers draw, from the addresses their authors publish
        // them at, each checked against its pinned SHA-256; the page shows the licences first and says so in the request.
        ("POST", ["fetch-open"]) => {
            if request.headers.get("content-type").is_none_or(|v| v != "application/json") {
                return fail(415, "Content-Type must be exactly application/json");
            }
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid JSON: {e}")),
            };
            if v["accept_licence"].as_bool() != Some(true) {
                return fail(400, "read the licences of these models and confirm before they are fetched");
            }
            let want: Vec<(String, Option<serde_json::Value>)> = farm.list().iter().filter_map(|p| p.entry.cad_model.clone().map(|m| (m, p.entry.cad_options.clone()))).collect();
            // `ids` names the models whose notice was read (the CORE One L): exactly those are fetched, on this one press.
            let asked: Vec<String> = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
            let open = crate::printer_models::fetch_selection(&crate::printer_models::wanted(&want, &lib), &asked);
            if open.is_empty() {
                Ok(crate::printer_models::fetch_status(&lib))
            } else {
                crate::printer_models::start_fetch(&lib, &open, crate::printer_models::Source::Published, true)
            }
        }
        ("POST", ["options", id, "install"]) => {
            if request.headers.get("content-type").is_none_or(|v| v != "application/json") {
                return fail(415, "Content-Type must be exactly application/json");
            }
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid JSON: {e}")),
            };
            // No source named and none saved: take each file from the address its author publishes it at.
            let text = v["source"].as_str().map(str::to_owned).or_else(|| farm.model_source()).unwrap_or_else(|| "published".into());
            crate::printer_models::Source::parse(&text).and_then(|src| {
                let r = lib.install_option(id, &src, v["accept_licence"].as_bool().unwrap_or(false))?;
                farm.set_model_source(&text);
                Ok(r)
            })
        }
        ("DELETE", ["options", id]) => {
            if let Some(p) = farm.list().iter().find(|p| p.entry.cad_options.as_ref().is_some_and(|s| s.as_array().is_some_and(|a| a.iter().any(|c| c["option"] == *id)))) {
                return fail(400, &format!("{} uses this option; take it off that printer first", p.entry.name));
            }
            lib.remove_option(id).map(|()| json!({"removed":id}))
        }
        ("DELETE", [id]) => {
            if let Some(p) = farm.list().iter().find(|p| p.entry.cad_model.as_deref() == Some(*id)) {
                return fail(400, &format!("{} uses this model; choose another model for it, or none, first", p.entry.name));
            }
            lib.remove(id).map(|()| json!({"removed":id}))
        }
        _ => return fail(405, "use GET /api/printer-models, GET /api/printer-models/<id>[/file], POST .../install or .../upload, DELETE /api/printer-models/<id>"),
    };
    Some(match result {
        Ok(v) => Response::json(200, v),
        Err(e) => Response::json(400, json!({"error":e})),
    })
}

/// The farm licence: reading its state is open (the pages show a note), saving or removing one is the
/// desktop window's with the farm token, like every other change to the farm.
fn licence_route(
    request: &HttpRequest,
    farm: &Arc<Farm>,
    path: &str,
    lan: bool,
) -> Option<Response> {
    if path != "/api/licence" {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    if request.method == "GET" {
        return Some(Response::json(200, farm.licence_status()));
    }
    if lan {
        return fail(
            403,
            "the licence is entered in the Stella Scope window on the computer",
        );
    }
    if request.headers.get("origin") != Some(&format!("http://{}", farm.base.host())) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    let valid = match (
        farm.token.as_ref(),
        request.headers.get("x-stella-control-token"),
    ) {
        (Some(e), Some(g)) => {
            e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    };
    if !valid {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    match request.method.as_str() {
        "POST" => {
            if request
                .headers
                .get("content-type")
                .is_none_or(|v| v != "application/json")
            {
                return fail(415, "Content-Type must be exactly application/json");
            }
            let v: serde_json::Value = match serde_json::from_slice(&request.body) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("invalid JSON: {e}")),
            };
            match farm.save_licence(v["text"].as_str().unwrap_or("")) {
                Ok(s) => Some(Response::json(200, s)),
                Err(e) => fail(400, &e),
            }
        }
        "DELETE" => match farm.remove_licence() {
            Ok(s) => Some(Response::json(200, s)),
            Err(e) => fail(400, &e),
        },
        _ => fail(405, "use GET, POST or DELETE /api/licence"),
    }
}

/// `POST /api/farm/phone-access`: switch phone access on in the configuration file (the page's "Turn on phone access"). Same rules as every
/// write to the farm: the desktop window only, its Origin and the farm's control token.
fn phone_access_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str, lan: bool) -> Option<Response> {
    if path != "/api/farm/phone-access" {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    if request.method != "POST" {
        return fail(405, "use POST /api/farm/phone-access");
    }
    if lan {
        return fail(403, "phone access is switched on in the Stella Scope window on the computer");
    }
    if request.headers.get("origin") != Some(&format!("http://{}", farm.base.host())) {
        return fail(403, "Origin does not match the configured loopback listener");
    }
    let valid = match (farm.token.as_ref(), request.headers.get("x-stella-control-token")) {
        (Some(e), Some(g)) => e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0,
        _ => false,
    };
    if !valid {
        return fail(401, "missing or invalid Stella control token");
    }
    match farm.enable_phone_access() {
        Ok(v) => Some(Response::json(200, v)),
        Err(e) => fail(400, &e),
    }
}

/// `POST /api/feedback`: what the person typed on the Feedback page, sent to 3D3D (feedback.rs). The desktop window only, with its Origin
/// and the farm's control token, like every other thing that leaves the computer on a press.
fn feedback_route(request: &HttpRequest, farm: &Arc<Farm>, path: &str, lan: bool) -> Option<Response> {
    if path != "/api/feedback" {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    if request.method != "POST" {
        return fail(405, "use POST /api/feedback");
    }
    if lan {
        return fail(403, "feedback is sent from the Stella Scope window on the computer");
    }
    if request.headers.get("origin") != Some(&format!("http://{}", farm.base.host())) {
        return fail(403, "Origin does not match the configured loopback listener");
    }
    let valid = match (farm.token.as_ref(), request.headers.get("x-stella-control-token")) {
        (Some(e), Some(g)) => e.len() == g.len() && e.bytes().zip(g.bytes()).fold(0_u8, |d, (a, b)| d | (a ^ b)) == 0,
        _ => false,
    };
    if !valid {
        return fail(401, "missing or invalid Stella control token");
    }
    if request.headers.get("content-type").is_none_or(|v| v != "application/json") {
        return fail(415, "Content-Type must be exactly application/json");
    }
    let v: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(e) => return fail(400, &format!("invalid JSON: {e}")),
    };
    let drivers: Vec<String> = farm.list().iter().map(|p| p.entry.driver.clone()).collect();
    let payload = match crate::feedback::payload(&v, &drivers) {
        Ok(p) => p,
        Err(e) => return fail(400, &e),
    };
    match crate::feedback::send(&payload) {
        Ok(reference) => Some(Response::json(200, json!({"ref":reference,"sent":payload["answers"]}))),
        Err(e) => fail(502, &e),
    }
}

/// One printer's plate: the picture of the empty plate, the check, "plate cleared" and starting the next
/// file (`/api/p/<id>/plate...` and `/api/p/<id>/queue/start`). Writes carry the printer's own control
/// token, so a paired phone can do them; the same safety as every other control.
fn plate_route(
    request: &HttpRequest,
    farm: &Arc<Farm>,
    printer: &Arc<Printer>,
    path: &str,
) -> Option<Response> {
    let route =
        path == "/api/plate" || path.starts_with("/api/plate/") || path == "/api/queue/start";
    if !route {
        return None;
    }
    let fail = |code: u16, e: &str| Some(Response::json(code, json!({"error":e})));
    let Some(q) = farm.queue() else {
        return fail(503, &farm.queue_error());
    };
    let id = printer.id.as_str();
    match (request.method.as_str(), path) {
        ("GET", "/api/plate") => {
            let plate = q.plate(id);
            let flow = q.flow(id);
            return Some(Response::json(
                200,
                json!({
                    "has_reference":plate.is_some(),
                    "region":plate.as_ref().map(|p| p.region.clone()),
                    "sensitivity":plate.as_ref().map(|p| p.sensitivity),
                    "captured_at":plate.as_ref().map(|p| p.captured_at),
                    "awaiting_clear":flow.awaiting_clear,"released":flow.released,"auto_start":flow.auto_start,
                    "checks":q.checks(id, 10),
                    "grid":[crate::plate::COLS, crate::plate::ROWS],
                }),
            ));
        }
        ("GET", "/api/plate/reference.jpg") => {
            return Some(match q.plate(id) {
                Some(p) => Response {
                    code: 200,
                    mime: "image/jpeg",
                    body: p.jpeg,
                },
                None => Response::json(
                    404,
                    json!({"error":"no empty-plate picture is saved for this printer"}),
                ),
            });
        }
        ("GET", "/api/plate/frame") => {
            let n = query_param(&request.target, "id").and_then(|v| v.parse::<i64>().ok());
            return Some(match n.and_then(|n| q.check_frame(n, id)) {
                Some(b) => Response {
                    code: 200,
                    mime: "image/jpeg",
                    body: b,
                },
                None => Response::json(404, json!({"error":"no such frame"})),
            });
        }
        _ => {}
    }
    // Everything else changes something: the printer's own door.
    if !printer.controls_enabled() {
        return fail(
            503,
            "controls are off, so the plate flow and the queue cannot send anything",
        );
    }
    let host = farm.base.host();
    if request.headers.get("origin") != Some(&format!("http://{host}")) {
        return fail(
            403,
            "Origin does not match the configured loopback listener",
        );
    }
    if request
        .headers
        .get("content-type")
        .is_none_or(|v| v != "application/json")
    {
        return fail(415, "Content-Type must be exactly application/json");
    }
    if !printer
        .controls
        .token_ok(request.headers.get("x-stella-control-token"))
    {
        return fail(401, "this page is not allowed to control the printer; reload it and try again");
    }
    let v: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(e) => return fail(400, &format!("invalid JSON: {e}")),
    };
    let ok = |r: crate::Result<serde_json::Value>| match r {
        Ok(v) => Some(Response::json(200, v)),
        Err(e) => Some(Response::json(400, json!({"error":e}))),
    };
    match (request.method.as_str(), path) {
        ("POST", "/api/plate/reference") => ok(crate::queue_flow::capture_reference(farm, &q, printer)),
        ("PATCH", "/api/plate") => {
            let mut flow = q.flow(id);
            if let Some(a) = v["auto_start"].as_bool() {
                flow.auto_start = a;
                if let Err(e) = q.set_flow(id, flow) {
                    return fail(400, &e);
                }
            }
            if v.get("forget").and_then(serde_json::Value::as_bool) == Some(true) {
                q.forget_plate(id);
            }
            if let Some(r) = v.get("region") {
                let ok_shape = r.as_array().is_some_and(|a| a.len() == 4 && a.iter().all(|p| p.as_array().is_some_and(|c| c.len() == 2 && c.iter().all(|n| n.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x))))));
                if !ok_shape {
                    return fail(400, "mark the plate with four corners, each between 0 and 1");
                }
                let q4 = crate::plate::Quad([0, 1, 2, 3].map(|i| [r[i][0].as_f64().unwrap_or(0.0), r[i][1].as_f64().unwrap_or(0.0)]));
                if !q4.valid() {
                    return fail(400, "the four corners do not mark a plate; drag them to its corners");
                }
                if let Err(e) = q.save_region(id, r, v["sensitivity"].as_f64()) {
                    return fail(400, &e);
                }
            } else if let Some(s) = v["sensitivity"].as_f64() {
                let region = q.plate(id).map(|p| p.region).unwrap_or(serde_json::Value::Null);
                if let Err(e) = q.save_region(id, &region, Some(s)) {
                    return fail(400, &e);
                }
            }
            Some(Response::json(200, json!({"ok":true})))
        }
        ("POST", "/api/plate/cleared") => ok(crate::queue_flow::plate_cleared(farm, &q, printer, v["override"].as_bool() == Some(true))),
        ("POST", "/api/queue/start") => ok(crate::queue_flow::start_next(farm, &q, printer, v["job"].as_i64(), v["override"].as_bool() == Some(true), v["dry_run"].as_bool() == Some(true))),
        _ => fail(405, "use GET or PATCH /api/plate, POST /api/plate/reference, POST /api/plate/cleared, POST /api/queue/start"),
    }
}

fn connection(mut stream: TcpStream, farm: &Arc<Farm>, lan: bool) {
    let cfg: &Config = &farm.base;
    if stream
        .set_write_timeout(Some(Duration::from_millis(cfg.http_timeout_ms)))
        .is_err()
    {
        return;
    }
    let mut peer = stream.peer_addr().ok().map(|address| address.ip());
    let mut allow = "GET";
    let gate = (!lan).then(|| UploadGate {
        dir: farm
            .queue()
            .map(|q| q.incoming())
            .unwrap_or_else(|| farm.models_dir()),
        models: farm.models_dir(),
        host: cfg.host(),
        token: farm.token.clone(),
        upload_keys: crate::fleet::slicer_keys(farm),
    });
    // 2026-10-18 (polish WO-06): a request refused before its body was read.
    let mut refused = false;
    let response = match read_request_gated(&mut stream, cfg, gate.as_ref()) {
        Err(e) => {
            refused = true;
            Response::json(
                if e.contains("headers exceed") {
                    431
                } else if e.contains("body exceeds") {
                    413
                } else if e.contains("requires Content-Length") {
                    411
                } else {
                    400
                },
                json!({"error":e}),
            )
        }
        Ok(mut request) => {
            if !lan && !desktop_host(&request, cfg) {
                let _ = send(
                    &mut stream,
                    Response::json(
                        403,
                        json!({"error":"Host does not match the configured desktop listener"}),
                    ),
                    "GET",
                );
                return;
            }
            let full_path = request
                .target
                .split('?')
                .next()
                .unwrap_or(&request.target)
                .to_owned();
            let path = full_path.as_str();
            if lan {
                if let Some(r) = lan_gate(&mut request, cfg, path, &mut peer) {
                    if let Err(e) = send(&mut stream, r, "GET") {
                        eprintln!("HTTP response was interrupted: {e}");
                    }
                    return;
                }
            }
            if let Some((r, cookie)) = pairing_routes(&request, cfg, path, lan) {
                let result = match cookie {
                    Some(c) => send_with_cookie(&mut stream, r, &c),
                    None => send(&mut stream, r, "GET, POST"),
                };
                if let Err(e) = result {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if request.method == "GET" && path == "/api/capabilities" {
                let _ = send(
                    &mut stream,
                    Response::json(200, json!({"desktop":!lan,"phone":lan})),
                    "GET",
                );
                return;
            }
            if let Some(r) = farm_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "GET, POST, PATCH, DELETE") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = licence_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "GET, POST, DELETE") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = feedback_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "POST") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = phone_access_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "POST") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = models_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "GET, POST, DELETE") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            // WO-15: OctoPrint-compatible upload for slicers (OrcaSlicer/PrusaSlicer "OctoPrint"
            // host type), before the per-printer prefix or the old single-printer /api will swallow
            // /api/files/local with a "page not found".
            if let Some(r) = octoprint_route(&request, farm, path) {
                if let Err(e) = send(&mut stream, r, "GET, POST") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = fleet_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "GET, POST, PATCH, DELETE") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            if let Some(r) = queue_route(&request, farm, path, lan) {
                if let Err(e) = send(&mut stream, r, "GET, POST, PATCH, DELETE") {
                    eprintln!("HTTP response was interrupted: {e}");
                }
                return;
            }
            // A printer's own page: /p/<id> is the same NEXT app, which reads the id from its URL.
            // A phone's home is the farm when this service has more than one printer (farm board WO-11);
            // a single printer opens straight to its own page as before.
            let page = if path == "/p" || path.starts_with("/p/") {
                "/"
            } else if lan && path == "/" && request.method == "GET" && farm.visible().len() > 1 {
                "/farm"
            // No printer a person can see (a fresh install): the farm, with its demo card and Add a printer, never
            // the hidden placeholder's page stuck at "Connecting to the printer" (finish-2 F01).
            } else if path == "/" && request.method == "GET" && farm.visible().is_empty() {
                "/farm"
            } else {
                path
            };
            // Which printer? /api/p/<id>/rest means the printer <id> and the route /api/rest; no prefix
            // means the first printer, exactly as before there was a farm.
            let (printer, target): (Arc<Printer>, String) = match farm::split_prefix(
                &request.target,
            ) {
                Some((id, rewritten)) => match farm.get(id) {
                    Some(p) => (p, rewritten),
                    None => {
                        let _ = send(
                            &mut stream,
                            Response::json(
                                404,
                                json!({"error":format!("no printer called {id}; GET /api/printers lists them")}),
                            ),
                            "GET",
                        );
                        return;
                    }
                },
                None => (farm.first(), request.target.clone()),
            };
            let path = if page != path {
                page
            } else {
                target.split('?').next().unwrap_or(&target)
            };
            let cfg = &printer.cfg;
            if let Some(methods) = commands::routed(path) {
                allow = match methods {
                    ["GET"] => "GET",
                    ["POST"] => "POST",
                    ["GET", "POST"] => "GET, POST",
                    ["GET", "PUT"] => "GET, PUT",
                    _ => "GET",
                };
                printer.controls.handle(
                    &request.method,
                    path,
                    &request.headers,
                    &request.body,
                    peer.unwrap_or(IpAddr::from([0, 0, 0, 0])),
                )
            } else if let Some(r) = plate_route(&request, farm, &printer, path) {
                allow = "GET, POST, PATCH";
                r
            } else if request.method == "GET"
                && (path == "/api/model" || path == "/api/model/match")
            {
                allow = "GET";
                model_route(&request, farm, &printer, path)
            } else if request.method == "GET" && path == "/api/tick/stream" {
                sse_stream(stream, &printer.id);
                return;
            } else if request.method == "GET"
                && (path == "/api/camera/stream" || path == "/api/camera/snapshot")
            {
                // A replay has no camera; a live stream runs until the page lets it go.
                let replay = printer
                    .state
                    .lock()
                    .map(|s| s.mode == "replay")
                    .unwrap_or(true);
                if replay {
                    Response::json(404, json!({"error":"a saved recording has no camera"}))
                } else {
                    crate::camera::pipe(stream, cfg, &target, path.ends_with("snapshot"));
                    return;
                }
            } else if let Some(r) = api::asset(&request.method, path) {
                r
            } else {
                match Store::open(&cfg.database, true) {
                    Err(e) => Response::json(500, json!({"error":e})),
                    Ok(db) => match printer.state.lock() {
                        Ok(status) => {
                            let snapshot = status.clone();
                            drop(status);
                            api::handle(&request.method, &target, &db, cfg, &snapshot)
                        }
                        Err(_) => Response::json(
                            500,
                            json!({"error":"collector status unavailable; restart Stella Scope without deleting the recording"}),
                        ),
                    },
                }
            }
        }
    };
    if let Err(e) = send(&mut stream, response, allow) {
        eprintln!("HTTP response was interrupted: {e}");
    }
    if refused {
        close_politely(&mut stream);
    }
}

/// A refused upload is answered before its body is read. Closing a socket that still holds unread
/// bytes sends a reset, and a reset can destroy the answer on its way: a slicer saw "connection
/// reset by peer" instead of the reason (polish WO-06 item 2, the 1-in-15 failure of the OctoPrint
/// upload test). So: finish our side of the conversation, then read away what is still arriving, for
/// at most three seconds, so the close is a quiet one.
fn close_politely(stream: &mut TcpStream) {
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let end = Instant::now() + Duration::from_secs(3);
    let mut sink = vec![0_u8; 64 * 1024];
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn serve(listener: TcpListener, lan: Option<TcpListener>, farm: Arc<Farm>) -> Result<()> {
    let active = Arc::new(AtomicUsize::new(0));
    let phone_active = Arc::new(AtomicUsize::new(0));
    if let Some(lan) = lan {
        let (farm, active) = (farm.clone(), active.clone());
        let phone_active = phone_active.clone();
        thread::Builder::new()
            .name("stella-lan".into())
            .spawn(move || accept(lan, farm, active, phone_active, true))
            .map_err(|e| format!("cannot start the phone listener: {e}"))?;
    }
    accept(listener, farm, active, phone_active, false);
    Ok(())
}

fn accept(
    listener: TcpListener,
    farm: Arc<Farm>,
    active: Arc<AtomicUsize>,
    phone_active: Arc<AtomicUsize>,
    lan: bool,
) {
    let cfg = farm.base.clone();
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot accept HTTP connection: {e}; check system file limits");
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        let phone_permit = if lan {
            let cap = cfg.max_http_connections.saturating_sub(4).min(8);
            if phone_active
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    (n < cap).then_some(n + 1)
                })
                .is_err()
            {
                // Take the request off the socket first (50 ms at most): closing with it unread
                // resets the connection and the client never sees the 503.
                let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                let _ = stream.read(&mut [0u8; 8192]);
                let _ = send(
                    &mut stream,
                    Response::json(
                        503,
                        json!({"error":"phone reader slots are busy; retry shortly"}),
                    ),
                    "GET",
                );
                continue;
            }
            Some(Permit(phone_active.clone()))
        } else {
            None
        };
        if active
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n < cfg.max_http_connections {
                    Some(n + 1)
                } else {
                    None
                }
            })
            .is_err()
        {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(cfg.http_timeout_ms)));
            let _ = send(
                &mut stream,
                Response::json(
                    503,
                    json!({"error":"all reader slots are busy; close extra tabs and retry shortly"}),
                ),
                "GET",
            );
            continue;
        }
        let permit = Permit(active.clone());
        let farm = farm.clone();
        if let Err(e) = thread::Builder::new()
            .name("stella-http".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let _permit = permit;
                let _phone_permit = phone_permit;
                connection(stream, &farm, lan);
            })
        {
            eprintln!(
                "cannot start HTTP worker: {e}; reduce max_http_connections or free system memory"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, thread};

    /// Finish-2 F01: a reload never meets a 503. The third page on one printer lets the first go (it ends and
    /// polls); the newest gets the next frame. (The registry is shared with every test in this process, so
    /// only what other tests cannot disturb is asserted.)
    #[test]
    fn the_newest_page_gets_the_stream_and_the_oldest_is_let_go() {
        let id = "sse-newest-wins";
        let first = sse_register(id);
        let _second = sse_register(id);
        let third = sse_register(id);
        assert!(matches!(
            first.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ));
        sse_broadcast(id, "{}");
        assert!(third.recv_timeout(Duration::from_millis(50)).is_ok());
    }

    fn parse(request: &str) -> Result<HttpRequest> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = request.as_bytes().to_vec();
        let worker = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(&request).unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        let result = read_request(&mut stream, &Config::default());
        worker.join().unwrap();
        result
    }

    #[test]
    fn tick_frames_stay_unnamed_and_console_frames_are_named() {
        // web/service.js listens on EventSource.onmessage, which only fires for unnamed events.
        // Naming the tick would take the HUD live readout off the air without any error.
        let tick = sse_frame(None, r#"{"schema":"stella-scope.tick.v1"}"#);
        assert_eq!(tick, "data: {\"schema\":\"stella-scope.tick.v1\"}\n\n");
        assert!(!tick.contains("event:"));

        let console = sse_frame(Some("console"), r#"{"lines":[]}"#);
        assert_eq!(console, "event: console\ndata: {\"lines\":[]}\n\n");
        // Both frames end with the blank line that terminates an SSE event.
        assert!(tick.ends_with("\n\n") && console.ends_with("\n\n"));
    }

    #[test]
    fn put_carries_a_body_and_other_verbs_still_do_not() {
        let body = r#"{"materials":[],"steps":[1],"fan":[50]}"#;
        let parsed = parse(&format!(
            "PUT /api/presets HTTP/1.1\r\nHost: 127.0.0.1:7420\r\nOrigin: http://127.0.0.1:7420\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .unwrap();
        assert_eq!(parsed.method, "PUT");
        assert_eq!(parsed.body, body.as_bytes());
        assert!(
            parse("PUT /api/presets HTTP/1.1\r\nHost: 127.0.0.1:7420\r\n\r\n")
                .unwrap_err()
                .contains("PUT requires Content-Length")
        );
        for verb in ["GET", "DELETE", "HEAD"] {
            assert!(parse(&format!(
                "{verb} /api/presets HTTP/1.1\r\nHost: a\r\nContent-Length: 1\r\n\r\nx"
            ))
            .unwrap_err()
            .contains("only for POST and PUT"));
        }
    }

    #[test]
    fn bounded_parser_reads_exact_json_body() {
        let body = r#"{"id":"home","confirm":true}"#;
        let request = format!(
            "POST /api/commands HTTP/1.1\r\nHost: 127.0.0.1:7420\r\nOrigin: http://127.0.0.1:7420\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let parsed = parse(&request).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.target, "/api/commands");
        assert_eq!(parsed.body, body.as_bytes());
        assert_eq!(parsed.headers["origin"], "http://127.0.0.1:7420");
    }

    #[test]
    fn desktop_data_requires_its_exact_host() {
        let cfg = Config::default();
        let good = parse("GET /api/live HTTP/1.1\r\nHost: 127.0.0.1:7420\r\n\r\n").unwrap();
        let rebound =
            parse("GET /api/live HTTP/1.1\r\nHost: attacker.example:7420\r\n\r\n").unwrap();
        assert!(desktop_host(&good, &cfg));
        assert!(!desktop_host(&rebound, &cfg));
    }

    #[test]
    fn bounded_parser_rejects_body_and_header_ambiguity() {
        assert!(
            parse("POST /api/commands HTTP/1.1\r\nHost: 127.0.0.1:7420\r\n\r\n")
                .unwrap_err()
                .contains("Content-Length")
        );
        assert!(parse(
            "POST /api/commands HTTP/1.1\r\nHost: 127.0.0.1:7420\r\nContent-Length: 4097\r\n\r\n"
        )
        .unwrap_err()
        .contains("body exceeds"));
        assert!(parse("POST /api/commands HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n")
            .unwrap_err().contains("duplicate"));
        assert!(parse(
            "POST /api/commands HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .unwrap_err()
        .contains("Transfer-Encoding"));
        assert!(
            parse("GET /api/controls HTTP/1.1\r\nHost: a\r\nContent-Length: 1\r\n\r\nx")
                .unwrap_err()
                .contains("only for POST")
        );
        assert!(
            parse("POST /api/commands HTTP/1.1\r\nHost: a\r\nContent-Length: 1\r\n\r\n{}")
                .unwrap_err()
                .contains("beyond the declared body")
        );
    }
}

/// Published HTTPS download page, shown by the desktop pairing flow.
pub const APP_PAGE: &str = "https://3d3d.ca/software/stella#android";
