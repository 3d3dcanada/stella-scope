use crate::{
    api::{self, Response},
    collector::Shared,
    commands,
    config::Config,
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

// SSE registry: one entry per open /api/tick/stream connection. The collector broadcasts the
// newest tick payload to every listener the moment a sample is stored; dead listeners drop out.
// Round-4 item 4: each stream pins a worker for its lifetime, so concurrency is capped —
// two windows is the realistic maximum; a third gets 503 and the client polls /api/tick.
const MAX_SSE_STREAMS: usize = 2;
fn sse_registry() -> &'static std::sync::Mutex<Vec<SseSender<String>>> {
    static R: std::sync::OnceLock<std::sync::Mutex<Vec<SseSender<String>>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}
// The channel carries whole SSE frames, so a named event and the default tick event go down the
// same path. The tick keeps its unnamed frame: naming it would move it off EventSource.onmessage
// and break every page already listening.
fn sse_send(frame: String) {
    if let Ok(mut list) = sse_registry().lock() {
        list.retain(|tx| tx.send(frame.clone()).is_ok());
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
pub fn sse_broadcast(line: &str) {
    sse_send(sse_frame(None, line));
}
/// L8: console lines as they are recorded, as a named `console` event.
pub fn sse_broadcast_event(event: &str, line: &str) {
    sse_send(sse_frame(Some(event), line));
}
fn sse_register() -> Option<SseReceiver<String>> {
    let (tx, rx) = sse_channel();
    let Ok(mut list) = sse_registry().lock() else {
        return Some(rx);
    };
    if list.len() >= MAX_SSE_STREAMS {
        return None;
    }
    list.push(tx);
    Some(rx)
}
fn sse_stream(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Some(rx) = sse_register() else {
        let _ = write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{{\"error\":\"SSE stream limit reached; polling /api/tick instead\"}}");
        return;
    };
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

fn send_with_cookie(stream: &mut TcpStream, response: Response, cookie: &str) -> std::io::Result<()> {
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

#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream, cfg: &Config) -> Result<HttpRequest> {
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
    let takes_body = matches!(method.as_str(), "POST" | "PUT");
    let content_length = match headers.get("content-length") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "Content-Length must be a non-negative decimal integer".to_owned())?,
        None if takes_body => return Err(format!("{method} requires Content-Length")),
        None => 0,
    };
    if content_length > MAX_COMMAND_BODY {
        return Err(format!(
            "request body exceeds the {MAX_COMMAND_BODY}-byte command limit"
        ));
    }
    if !takes_body && content_length != 0 {
        return Err("request bodies are accepted only for POST and PUT command requests".into());
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
    })
}

pub fn request_line(stream: &mut TcpStream, cfg: &Config) -> Result<(String, String)> {
    let request = read_request(stream, cfg)?;
    Ok((request.method, request.target))
}

/// Phone access (src/lan.rs): what a request from the network listener may do. Pages without a
/// paired device get the pairing page, data gets 401; a paired control request is checked for
/// same-origin here and then handed on exactly as the loopback listener would present it.
fn lan_gate(request: &mut HttpRequest, cfg: &Config, path: &str, peer: &mut Option<IpAddr>) -> Option<Response> {
    if path == "/pair.js" || path == "/pair" {
        return api::asset("GET", path);
    }
    if request.method == "POST" && path == "/api/pair" {
        let host = request.headers.get("host").cloned().unwrap_or_default();
        if host.is_empty() || request.headers.get("origin") != Some(&format!("http://{host}")) {
            return Some(Response::json(403, json!({"error":"pairing requires the phone page's origin"})));
        }
        return None; // answered by the caller with its cookie
    }
    if crate::lan::device_from_cookie(cfg, request.headers.get("cookie"), request.headers.get("x-stella-peer")).is_none() {
        if path.starts_with("/api/") {
            return Some(Response::json(401, json!({"error":"this device is not paired; open / and enter the code from the desktop"})));
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
        return Some(Response::json(403, json!({"error":"pairing is managed from the desktop"})));
    }
    let host = request.headers.get("host").cloned().unwrap_or_default();
    if let Some(origin) = request.headers.get("origin") {
        if origin != &format!("http://{host}") {
            return Some(Response::json(403, json!({"error":"cross-site request refused"})));
        }
    }
    let loopback = match cfg.bind.parse::<IpAddr>() {
        Ok(IpAddr::V6(a)) => format!("[{a}]:{}", cfg.port),
        _ => format!("{}:{}", cfg.bind, cfg.port),
    };
    request.headers.insert("host".into(), loopback.clone());
    if request.headers.contains_key("origin") {
        request.headers.insert("origin".into(), format!("http://{loopback}"));
    }
    *peer = Some(IpAddr::from([127, 0, 0, 1]));
    None
}

fn pairing_routes(request: &HttpRequest, cfg: &Config, path: &str, lan: bool) -> Option<(Response, Option<String>)> {
    if lan && request.method == "POST" && path == "/api/pair" {
        return Some(match crate::lan::pair(cfg, &request.body, request.headers.get("user-agent"), request.headers.get("x-stella-peer")) {
            Ok(key) => (Response::json(200, json!({"paired":true})), Some(format!("stella_device={key}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict"))),
            Err(e) => (Response::json(403, json!({"error":e})), None),
        });
    }
    if lan && path.starts_with("/api/eye/switch") {
        return Some((Response::json(403, json!({"error":"the failure watch is switched from the desktop"})), None));
    }
    if lan && path.starts_with("/api/notify") {
        return Some((Response::json(403, json!({"error":"WhatsApp alerts are set up from the desktop"})), None));
    }
    if lan || !(path.starts_with("/api/pair") || path.starts_with("/api/notify") || path == "/api/eye/switch") {
        return None;
    }
    // Desktop side: a POST must come from the desktop page itself.
    let own = format!("http://{}:{}", cfg.bind, cfg.port);
    if request.method == "POST" && request.headers.get("origin") != Some(&own) {
        return Some((Response::json(403, json!({"error":"this change comes from the Stella Scope window on the computer only"})), None));
    }
    let r = match (request.method.as_str(), path) {
        ("GET", "/api/pair/devices") => Ok(crate::lan::devices(cfg)),
        ("GET", "/api/pair/remote") => Ok(crate::remote::status(cfg)),
        ("POST", "/api/pair/new") if cfg.lan_port.is_none() => Err("phone access is off: set lan_port in stella-scope.json".to_owned()),
        ("POST", "/api/pair/new") => crate::lan::new_code(cfg).map(|v| crate::remote::with_pair_qr(cfg, v)),
        ("POST", "/api/pair/revoke") => crate::lan::revoke(cfg, &request.body),
        ("GET", "/api/notify") => Ok(crate::notify::settings(cfg)),
        ("POST", "/api/notify") => crate::notify::save(cfg, &request.body),
        ("POST", "/api/eye/switch") => crate::remote::eye_switch(cfg, &request.body),
        ("POST", "/api/notify/test") => Ok(json!({"queued":crate::notify::send("Stella: this is a test message from Stella Scope. WhatsApp alerts are working.")})),
        _ => Err("unknown pairing route".to_owned()),
    };
    Some((match r { Ok(v) => Response::json(200, v), Err(e) => Response::json(400, json!({"error":e})) }, None))
}

fn desktop_host(request: &HttpRequest, cfg: &Config) -> bool {
    request.headers.get("host") == Some(&cfg.host())
}

fn connection(mut stream: TcpStream, cfg: &Config, state: &Shared, controls: &commands::Service, lan: bool) {
    if stream
        .set_write_timeout(Some(Duration::from_millis(cfg.http_timeout_ms)))
        .is_err()
    {
        return;
    }
    let mut peer = stream.peer_addr().ok().map(|address| address.ip());
    let mut allow = "GET";
    let response = match read_request(&mut stream, cfg) {
        Err(e) => Response::json(
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
        ),
        Ok(mut request) => {
            if !lan && !desktop_host(&request, cfg) {
                let _ = send(&mut stream, Response::json(403, json!({"error":"Host does not match the configured desktop listener"})), "GET");
                return;
            }
            let target = request.target.clone();
            let path = target.split('?').next().unwrap_or(&target);
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
                let _ = send(&mut stream, Response::json(200, json!({"desktop":!lan,"phone":lan})), "GET");
                return;
            }
            if let Some(methods) = commands::routed(path) {
                allow = match methods {
                    ["GET"] => "GET",
                    ["POST"] => "POST",
                    ["GET", "POST"] => "GET, POST",
                    ["GET", "PUT"] => "GET, PUT",
                    _ => "GET",
                };
                controls.handle(
                    &request.method,
                    path,
                    &request.headers,
                    &request.body,
                    peer.unwrap_or(IpAddr::from([0, 0, 0, 0])),
                )
            } else if request.method == "GET" && path == "/api/tick/stream" {
                sse_stream(stream);
                return;
            } else if request.method == "GET" && (path == "/api/camera/stream" || path == "/api/camera/snapshot") {
                // A replay has no camera; a live stream runs until the page lets it go.
                let replay = state.lock().map(|s| s.mode == "replay").unwrap_or(true);
                if replay {
                    Response::json(404, json!({"error":"a saved recording has no camera"}))
                } else {
                    crate::camera::pipe(stream, cfg, &request.target, path.ends_with("snapshot"));
                    return;
                }
            } else if let Some(r) = api::asset(&request.method, path) {
                r
            } else {
                match Store::open(&cfg.database, true) {
                    Err(e) => Response::json(500, json!({"error":e})),
                    Ok(db) => match state.lock() {
                        Ok(status) => {
                            let snapshot = status.clone();
                            drop(status);
                            api::handle(&request.method, &request.target, &db, cfg, &snapshot)
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
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn serve(listener: TcpListener, lan: Option<TcpListener>, cfg: Config, state: Shared) -> Result<()> {
    let mode = state
        .lock()
        .map_err(|_| "collector status unavailable while initializing controls".to_owned())?
        .mode
        .clone();
    // One control service for both listeners: one token, one busy flag, one command audit.
    let controls = Arc::new(commands::Service::new(&cfg, &mode)?);
    let cfg = Arc::new(cfg);
    let active = Arc::new(AtomicUsize::new(0));
    let phone_active = Arc::new(AtomicUsize::new(0));
    if let Some(lan) = lan {
        let (cfg, state, controls, active) = (cfg.clone(), state.clone(), controls.clone(), active.clone());
        let phone_active = phone_active.clone();
        thread::Builder::new()
            .name("stella-lan".into())
            .spawn(move || accept(lan, cfg, state, controls, active, phone_active, true))
            .map_err(|e| format!("cannot start the phone listener: {e}"))?;
    }
    accept(listener, cfg, state, controls, active, phone_active, false);
    Ok(())
}

fn accept(listener: TcpListener, cfg: Arc<Config>, state: Shared, controls: Arc<commands::Service>, active: Arc<AtomicUsize>, phone_active: Arc<AtomicUsize>, lan: bool) {
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
            if phone_active.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < cap).then_some(n + 1)).is_err() {
                // Take the request off the socket first (50 ms at most): closing with it unread
                // resets the connection and the client never sees the 503.
                let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                let _ = stream.read(&mut [0u8; 8192]);
                let _ = send(&mut stream, Response::json(503, json!({"error":"phone reader slots are busy; retry shortly"})), "GET");
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
        let cfg = cfg.clone();
        let state = state.clone();
        let controls = controls.clone();
        if let Err(e) = thread::Builder::new()
            .name("stella-http".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let _permit = permit;
                let _phone_permit = phone_permit;
                connection(stream, &cfg, &state, &controls, lan);
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
        let rebound = parse("GET /api/live HTTP/1.1\r\nHost: attacker.example:7420\r\n\r\n").unwrap();
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
