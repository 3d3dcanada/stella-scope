//! NEXT review 2026-09-24: "Stella does have a camera ... in Mainsail it's very, very slow."
//!
//! Mainsail's default "MJPEG-Streamer (adaptive)" mode does not stream at all: it fetches one
//! snapshot, waits for it to draw, then asks for the next, so every frame pays a full round trip.
//! Stella Scope reads the camera's list from Moonraker and passes its MJPEG image bytes to an
//! `<img>` on the page. Upstream HTTP headers are never served under Stella's origin.
//!
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

const MAX_STREAMS: usize = 8;
const MAX_HEADER: usize = 16 * 1024;
static STREAMS: AtomicUsize = AtomicUsize::new(0);

fn safe_content_type(bytes: &[u8], snapshot: bool) -> Result<(String, usize)> {
    let end = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or("camera response has no complete headers")?
        + 4;
    let head =
        std::str::from_utf8(&bytes[..end]).map_err(|_| "camera response headers are not UTF-8")?;
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
        let (name, value) = line
            .split_once(':')
            .ok_or("camera returned a malformed response header")?;
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-encoding")
        {
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
        if !matches!(
            media.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        ) {
            return Err("camera snapshot is not a supported image".into());
        }
        return Ok((media, end));
    }
    if media != "multipart/x-mixed-replace" {
        return Err("camera stream is not MJPEG".into());
    }
    let boundary = parts
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .unwrap_or("")
        .trim_matches('"');
    if boundary.is_empty()
        || boundary.len() > 70
        || !boundary
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err("camera stream has an invalid MJPEG boundary".into());
    }
    Ok((
        format!("multipart/x-mixed-replace; boundary={boundary}"),
        end,
    ))
}

/// The printer's host name or address, from `moonraker_url`.
fn printer_host(cfg: &Config) -> Result<String> {
    let rest = cfg
        .moonraker_url
        .split("://")
        .nth(1)
        .ok_or("moonraker_url has no scheme; expected http://host:port")?;
    let hostport = rest.split('/').next().unwrap_or(rest);
    let host = hostport
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(hostport);
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
    let (hostport, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    let (h, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>().map_err(|_| "camera URL has a bad port")?,
        ),
        None => (hostport, 80),
    };
    // A bridge that listens on every interface reports its camera as 0.0.0.0 ("this machine"): from
    // the service's side that is the printer's own address, so it is read as that and nothing else.
    if h != host && h != "0.0.0.0" {
        return Err(format!(
            "camera URL points at {h}, not the printer ({host}); refused"
        ));
    }
    Ok((port, path))
}

/// A driver camera as the hub reads it: an RTSP address through ffmpeg, or MJPEG over HTTP.
fn hub_source(c: &crate::driver::CameraSource) -> Result<crate::camera_hub::Source> {
    use crate::camera_hub::Source;
    if c.kind == "rtsp" {
        return Ok(Source::Rtsp(c.url.clone()));
    }
    let rest = c
        .url
        .strip_prefix("http://")
        .ok_or("only http:// and rtsp:// cameras are supported")?;
    let (hostport, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>().map_err(|_| "camera URL has a bad port")?,
        ),
        None => (hostport, 80),
    };
    // An mjpg-streamer camera (the AD5M's) has a snapshot beside its stream: read that, one still at a time (F05).
    if path.contains("action=stream") {
        return Ok(Source::Snapshots {
            host: host.to_owned(),
            port,
            path: path.replace("action=stream", "action=snapshot"),
        });
    }
    Ok(Source::Mjpeg {
        host: host.to_owned(),
        port,
        path,
    })
}

/// `GET /api/camera`: the cameras Moonraker knows, with what the page needs to show them. Another brand
/// lists what its driver offers, in the same shape.
pub fn list(cfg: &Config) -> Result<Value> {
    if cfg.driver != "moonraker" {
        let cams = crate::driver::open(cfg)?.cameras()?;
        let rows: Vec<Value> = cams
            .iter()
            .map(|c| json!({"name":c.name,"service":c.kind,"target_fps":6,"target_fps_idle":1,"stream_url":Value::Null,
                "snapshot_url":Value::Null,"flip_horizontal":false,"flip_vertical":false,"rotation":0,"aspect_ratio":"16:9"}))
            .collect();
        return Ok(json!({"cameras":rows,
            "note":"Read once from the camera and shared by every viewer; an RTSP camera is converted to frames here."}));
    }
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
    if cfg.driver != "moonraker" {
        return pipe_driver(client, cfg, name.as_deref(), snapshot);
    }
    let cams = match list(cfg) {
        Ok(v) => v,
        Err(e) => return fail(&mut client, 502, &e),
    };
    let cam = cams["cameras"].as_array().and_then(|all| {
        all.iter()
            .find(|c| {
                name.as_deref()
                    .is_none_or(|n| c["name"].as_str() == Some(n))
            })
            .cloned()
    });
    let Some(cam) = cam else {
        return fail(
            &mut client,
            404,
            "no camera by that name; list them with GET /api/camera",
        );
    };
    let url = cam[if snapshot {
        "snapshot_url"
    } else {
        "stream_url"
    }]
    .as_str()
    .unwrap_or("");
    let (port, path) = match resolve(cfg, url) {
        Ok(v) => v,
        Err(e) => return fail(&mut client, 400, &e),
    };
    // Finish-2 F05: a stream of a camera that has a snapshot address is read one still at a time through the shared hub
    // (camera_hub::SNAPSHOT_FPS), never at the camera's full rate.
    if !snapshot {
        if let (Some(snap), Ok(host)) = (
            cam["snapshot_url"].as_str().filter(|s| !s.is_empty()),
            printer_host(cfg),
        ) {
            if let Ok((port, path)) = resolve(cfg, snap) {
                return serve_hub(
                    client,
                    crate::camera_hub::Source::Snapshots { host, port, path },
                    false,
                );
            }
        }
    }
    if !snapshot
        && STREAMS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < MAX_STREAMS).then_some(n + 1)
            })
            .is_err()
    {
        return fail(
            &mut client,
            503,
            "eight camera streams are already open; close one",
        );
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
    let addr = match (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    {
        Some(a) => a,
        None => return fail(&mut client, 502, "the printer's address does not resolve"),
    };
    let mut upstream = match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(e) => return fail(&mut client, 502, &format!("camera not answering: {e}")),
    };
    let _ = upstream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(10)));
    if write!(
        upstream,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return fail(&mut client, 502, "camera refused the request");
    }
    let mut buf = [0u8; 4096];
    let mut head = Vec::new();
    let (mime, body_start) = loop {
        match upstream.read(&mut buf) {
            Ok(0) | Err(_) => {
                return fail(&mut client, 502, "camera response ended before its headers")
            }
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

/// A camera another brand's driver offers, through the shared hub: one read of the camera however many
/// viewers, and at most `MAX_STREAMS` streams open at once.
fn pipe_driver(mut client: TcpStream, cfg: &Config, name: Option<&str>, snapshot: bool) {
    let cams = match crate::driver::open(cfg).and_then(|d| d.cameras()) {
        Ok(c) => c,
        Err(e) => return fail(&mut client, 502, &e),
    };
    let Some(cam) = cams
        .iter()
        .find(|c| name.is_none_or(|n| n.is_empty() || c.name == n))
    else {
        return fail(
            &mut client,
            404,
            "this printer has no camera set up; add its address in the printer's settings",
        );
    };
    let source = match hub_source(cam) {
        Ok(s) => s,
        Err(e) => return fail(&mut client, 400, &e),
    };
    serve_hub(client, source, snapshot)
}

/// One picture from the printer's first camera, as JPEG bytes (the timelapse recorder's frame, 2026-10-26). A Klipper camera is read by its
/// snapshot address when it has one, else one frame of its stream; another brand's camera through its driver, as the pages see it.
pub fn still(cfg: &Config) -> Result<Vec<u8>> {
    use crate::camera_hub::Source;
    let source = if cfg.driver != "moonraker" {
        let cams = crate::driver::open(cfg)?.cameras()?;
        hub_source(cams.first().ok_or("this printer has no camera set up")?)?
    } else {
        let cams = list(cfg)?;
        let cam = cams["cameras"].as_array().and_then(|a| a.first()).cloned().ok_or("this printer has no camera")?;
        let host = printer_host(cfg)?;
        match cam["snapshot_url"].as_str().filter(|u| !u.is_empty()) {
            Some(u) => {
                let (port, path) = resolve(cfg, u)?;
                Source::Snapshots { host, port, path }
            }
            None => {
                let (port, path) = resolve(cfg, cam["stream_url"].as_str().unwrap_or(""))?;
                Source::Mjpeg { host, port, path }
            }
        }
    };
    crate::camera_hub::snapshot(source, Duration::from_secs(8))
}

/// One camera through the shared hub: a still, or an MJPEG stream of the hub's frames.
fn serve_hub(mut client: TcpStream, source: crate::camera_hub::Source, snapshot: bool) {
    use crate::camera_hub::{self, Msg};
    let _ = client.set_write_timeout(Some(Duration::from_secs(10)));
    let headers = "Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; sandbox\r\n";
    if snapshot {
        return match camera_hub::snapshot(source, Duration::from_secs(8)) {
            Ok(jpeg) => {
                let _ = write!(client, "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n{headers}\r\n", jpeg.len());
                let _ = client.write_all(&jpeg);
            }
            Err(e) => fail(&mut client, 502, &e),
        };
    }
    if STREAMS
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < MAX_STREAMS).then_some(n + 1)
        })
        .is_err()
    {
        return fail(
            &mut client,
            503,
            "eight camera streams are already open; close one",
        );
    }
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            STREAMS.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let _slot = Slot;
    let rx = match camera_hub::subscribe(source) {
        Ok(rx) => rx,
        Err(e) => return fail(&mut client, 502, &e),
    };
    let first = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Msg::Frame(f)) => f,
        Ok(Msg::Error(e)) => return fail(&mut client, 502, &e),
        Err(_) => return fail(&mut client, 504, "the camera did not send a frame in time"),
    };
    if write!(client, "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=stellaframe\r\n{headers}\r\n").is_err() {
        return;
    }
    let mut frame = Some(first);
    loop {
        if let Some(f) = frame.take() {
            let head = format!(
                "--stellaframe\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                f.len()
            );
            if client.write_all(head.as_bytes()).is_err()
                || client.write_all(&f).is_err()
                || client.write_all(b"\r\n").is_err()
            {
                return;
            }
        }
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Msg::Frame(f)) => frame = Some(f),
            Ok(Msg::Error(_)) | Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    /// our KX-Bridge reports its webcam as http://0.0.0.0:7125/...: that is the printer's own address, and
    /// only that; another host is still refused.
    #[test]
    fn a_bridge_that_reports_its_camera_as_0_0_0_0_is_read_as_the_printer_itself() {
        let cfg = super::Config {
            moonraker_url: "http://127.0.0.1:7125".into(),
            ..super::Config::default()
        };
        assert_eq!(
            super::resolve(&cfg, "http://0.0.0.0:7125/api/camera/snapshot").unwrap(),
            (7125, "/api/camera/snapshot".to_owned())
        );
        assert!(super::resolve(&cfg, "http://192.168.1.50:7125/x")
            .unwrap_err()
            .contains("refused"));
        assert_eq!(
            super::resolve(&cfg, "/webcam/?action=snapshot").unwrap().1,
            "/webcam/?action=snapshot"
        );
    }

    /// An MJPEG camera relayed through the service: two viewers, one upstream connection to the
    /// camera, whole JPEGs in the multipart the page reads.
    #[test]
    fn two_viewers_share_one_upstream_read_of_an_mjpeg_camera() {
        let cam = crate::fakes::FakeMjpeg::start(0, 20);
        let url = cam.stream_url();
        let source = || crate::camera_hub::Source::Mjpeg {
            host: "127.0.0.1".into(),
            port: cam.addr.rsplit(':').next().unwrap().parse().unwrap(),
            path: "/?action=stream".into(),
        };
        let a = crate::camera_hub::subscribe(source()).unwrap();
        let b = crate::camera_hub::subscribe(source()).unwrap();
        for rx in [&a, &b] {
            match rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap() {
                crate::camera_hub::Msg::Frame(f) => assert_eq!(&f[..2], [0xFF, 0xD8]),
                crate::camera_hub::Msg::Error(e) => panic!("{e}"),
            }
        }
        assert_eq!(
            cam.clients.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "one connection to the camera: {url}"
        );
    }

    #[test]
    fn camera_urls_stay_on_the_printer() {
        let mut cfg = crate::config::Config::default();
        cfg.moonraker_url = "http://192.168.1.131:7125".into();
        assert_eq!(
            super::resolve(&cfg, "/webcam/?action=stream").unwrap(),
            (80, "/webcam/?action=stream".into())
        );
        assert_eq!(
            super::resolve(&cfg, "http://192.168.1.131:8080/?action=stream").unwrap(),
            (8080, "/?action=stream".into())
        );
        assert!(super::resolve(&cfg, "http://evil.example/cam").is_err());
        assert!(super::resolve(&cfg, "https://192.168.1.131/cam").is_err());
    }

    #[test]
    fn camera_headers_never_pass_through() {
        let jpeg = b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nSet-Cookie: evil=1\r\n\r\nJFIF";
        assert_eq!(
            super::safe_content_type(jpeg, true).unwrap(),
            ("image/jpeg".into(), jpeg.len() - 4)
        );
        let mjpeg =
            b"HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame_1\r\n\r\n";
        assert_eq!(
            super::safe_content_type(mjpeg, false).unwrap().0,
            "multipart/x-mixed-replace; boundary=frame_1"
        );
        for bad in [
            b"HTTP/1.1 302 Found\r\nContent-Type: image/jpeg\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<script>".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nTransfer-Encoding: chunked\r\n\r\n"
                .as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Type: text/html\r\n\r\n"
                .as_slice(),
        ] {
            assert!(super::safe_content_type(bad, true).is_err());
        }
    }
}
