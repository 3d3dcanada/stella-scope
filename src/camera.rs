//! Mainsail's default "MJPEG-Streamer (adaptive)" mode does not stream at all: it fetches one
//! snapshot, waits for it to draw, then asks for the next, so every frame pays a full round trip.
//! Stella Scope reads the camera's list from Moonraker and passes its MJPEG image bytes to an
//! `<img>` on the page. Upstream HTTP headers are never served under the test printer's origin.
//! Only the printer's own host is ever contacted, whatever the stream URL says, and at most
//! `MAX_STREAMS` streams run at once (each holds one HTTP worker while it lives).
use crate::{config::Config, moonraker::Moonraker, Result};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

const MAX_STREAMS: usize = 2;
const MAX_HEADER: usize = 16 * 1024;
static STREAMS: AtomicUsize = AtomicUsize::new(0);

fn safe_content_type(bytes: &[u8], snapshot: bool) -> Result<(String, usize)> {
    let end = bytes.windows(4).position(|part| part == b"\r\n\r\n").ok_or("camera response has no complete headers")? + 4;
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| "camera response headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    let mut words = status.split_whitespace();
    if !matches!(words.next(), Some("HTTP/1.0" | "HTTP/1.1")) || words.next() != Some("200") {
        return Err("camera did not return HTTP 200".into());
    }
    let mut content_type = None;
    for line in lines.take_while(|line| !line.is_empty()) {
        if line.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err("camera returned unsafe response headers".into());
        }
        let (name, value) = line.split_once(':').ok_or("camera returned a malformed response header")?;
        if name.eq_ignore_ascii_case("transfer-encoding") || name.eq_ignore_ascii_case("content-encoding") {
            return Err("camera response uses unsupported encoding".into());
        }
        if name.eq_ignore_ascii_case("content-type") {
            if content_type.replace(value.trim()).is_some() {
                return Err("camera returned duplicate content types".into());
            }
        }
    }
    let value = content_type.ok_or("camera response has no content type")?;
    let mut parts = value.split(';');
    let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if snapshot {
        if !matches!(media.as_str(), "image/jpeg" | "image/png" | "image/webp" | "image/gif") {
            return Err("camera snapshot is not a supported image".into());
        }
        return Ok((media, end));
    }
    if media != "multipart/x-mixed-replace" {
        return Err("camera stream is not MJPEG".into());
    }
    let boundary = parts.find_map(|part| part.trim().strip_prefix("boundary=")).unwrap_or("").trim_matches('"');
    if boundary.is_empty() || boundary.len() > 70 || !boundary.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
        return Err("camera stream has an invalid MJPEG boundary".into());
    }
    Ok((format!("multipart/x-mixed-replace; boundary={boundary}"), end))
}

/// The printer's host name or address, from `moonraker_url`.
fn printer_host(cfg: &Config) -> Result<String> {
    let rest = cfg
        .moonraker_url
        .split("://")
        .nth(1)
        .ok_or("moonraker_url has no scheme; expected http://host:port")?;
    let hostport = rest.split('/').next().unwrap_or(rest);
    let host = hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport);
    if host.is_empty() {
        return Err("moonraker_url has no host".into());
    }
    Ok(host.to_owned())
}

/// Resolve a webcam URL (relative, or absolute on the printer's host) into port and path.
/// Anything pointing at another host is refused: the proxy never becomes a way out.
pub fn resolve(cfg: &Config, url: &str) -> Result<(u16, String)> {
    let host = printer_host(cfg)?;
    if url.starts_with('/') {
        return Ok((80, url.to_owned()));
    }
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// camera URLs on the printer are proxied")?;
    let (hostport, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let (h, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "camera URL has a bad port")?),
        None => (hostport, 80),
    };
    if h != host {
        return Err(format!("camera URL points at {h}, not the printer ({host}); refused"));
    }
    Ok((port, path))
}

/// `GET /api/camera`: the cameras Moonraker knows, with what the page needs to show them.
pub fn list(cfg: &Config) -> Result<Value> {
    let v = Moonraker::new(cfg).get("/server/webcams/list")?;
    let cams: Vec<Value> = v["webcams"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|c| c["enabled"].as_bool() != Some(false))
                .map(|c| {
                    json!({"name":c["name"],"service":c["service"],"target_fps":c["target_fps"],
                        "target_fps_idle":c["target_fps_idle"],"stream_url":c["stream_url"],
                        "snapshot_url":c["snapshot_url"],"flip_horizontal":c["flip_horizontal"],
                        "flip_vertical":c["flip_vertical"],"rotation":c["rotation"],"aspect_ratio":c["aspect_ratio"]})
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({"cameras":cams,
        "note":"Shown here from the camera's own MJPEG stream, passed straight through; Mainsail's adaptive mode fetches one snapshot at a time instead."}))
}

fn fail(client: &mut TcpStream, code: u16, message: &str) {
    let body = json!({"error":message}).to_string();
    let _ = write!(client, "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
}

/// `GET /api/camera/stream?name=` and `/api/camera/snapshot?name=`: open the camera URL on the
/// printer and copy only its approved image body to the page until either side closes.
pub fn pipe(mut client: TcpStream, cfg: &Config, target: &str, snapshot: bool) {
    let name = target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix("name="))
        .map(|n| n.replace("%20", " ").replace('+', " "));
    let cams = match list(cfg) {
        Ok(v) => v,
        Err(e) => return fail(&mut client, 502, &e),
    };
    let cam = cams["cameras"].as_array().and_then(|all| {
        all.iter()
            .find(|c| name.as_deref().is_none_or(|n| c["name"].as_str() == Some(n)))
            .cloned()
    });
    let Some(cam) = cam else {
        return fail(&mut client, 404, "no camera by that name; list them with GET /api/camera");
    };
    let url = cam[if snapshot { "snapshot_url" } else { "stream_url" }].as_str().unwrap_or("");
    let (port, path) = match resolve(cfg, url) {
        Ok(v) => v,
        Err(e) => return fail(&mut client, 400, &e),
    };
    if !snapshot
        && STREAMS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < MAX_STREAMS).then_some(n + 1))
            .is_err()
    {
        return fail(&mut client, 503, "two camera streams are already open; close one");
    }
    struct Slot(bool);
    impl Drop for Slot {
        fn drop(&mut self) {
            if self.0 {
                STREAMS.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
    let _slot = Slot(!snapshot);
    let host = match printer_host(cfg) {
        Ok(h) => h,
        Err(e) => return fail(&mut client, 500, &e),
    };
    let addr = match (host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(a) => a,
        None => return fail(&mut client, 502, "the printer's address does not resolve"),
    };
    let mut upstream = match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(e) => return fail(&mut client, 502, &format!("camera not answering: {e}")),
    };
    let _ = upstream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(10)));
    if write!(upstream, "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").is_err() {
        return fail(&mut client, 502, "camera refused the request");
    }
    let mut buf = [0u8; 4096];
    let mut head = Vec::new();
    let (mime, body_start) = loop {
        match upstream.read(&mut buf) {
            Ok(0) | Err(_) => return fail(&mut client, 502, "camera response ended before its headers"),
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
        if let Some(end) = head.windows(4).position(|part| part == b"\r\n\r\n") {
            if end + 4 > MAX_HEADER {
                return fail(&mut client, 502, "camera response headers are too large");
            }
            break match safe_content_type(&head, snapshot) {
                Ok(parsed) => parsed,
                Err(e) => return fail(&mut client, 502, &e),
            };
        }
        if head.len() > MAX_HEADER {
            return fail(&mut client, 502, "camera response headers are too large");
        }
    };
    if write!(client, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; sandbox\r\n\r\n").is_err()
        || client.write_all(&head[body_start..]).is_err() {
        return;
    }
    loop {
        match upstream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if client.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn camera_urls_stay_on_the_printer() {
        let mut cfg = crate::config::Config::default();
        cfg.moonraker_url = "http://192.168.1.50:7125".into();
        assert_eq!(super::resolve(&cfg, "/webcam/?action=stream").unwrap(), (80, "/webcam/?action=stream".into()));
        assert_eq!(super::resolve(&cfg, "http://192.168.1.50:8080/?action=stream").unwrap(), (8080, "/?action=stream".into()));
        assert!(super::resolve(&cfg, "http://evil.example/cam").is_err());
        assert!(super::resolve(&cfg, "https://192.168.1.50/cam").is_err());
    }

    #[test]
    fn camera_headers_never_pass_through() {
        let jpeg = b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nSet-Cookie: evil=1\r\n\r\nJFIF";
        assert_eq!(super::safe_content_type(jpeg, true).unwrap(), ("image/jpeg".into(), jpeg.len() - 4));
        let mjpeg = b"HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame_1\r\n\r\n";
        assert_eq!(super::safe_content_type(mjpeg, false).unwrap().0, "multipart/x-mixed-replace; boundary=frame_1");
        for bad in [
            b"HTTP/1.1 302 Found\r\nContent-Type: image/jpeg\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<script>".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Type: text/html\r\n\r\n".as_slice(),
        ] {
            assert!(super::safe_content_type(bad, true).is_err());
        }
    }
}
