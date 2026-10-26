//! Finding printers on the local network, and testing a connection before it is saved (farm board WO-06).
//!
//! Everything here is read-only. A host is first asked, with a short timeout, whether the few ports
//! printers use are open; only an open port gets a request: Moonraker `GET /server/info`, PrusaLink
//! `GET /api/version` (it answers 401 with a digest challenge, or its name), FlashForge `~M115` on 8899
//! (take control, ask who it is, release). Nothing is sent that changes a printer.
use crate::{
    config::{self, Config},
    driver::{self, Capabilities},
    Result,
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

/// The ports each brand answers on. Tests give the fakes' own.
#[derive(Clone, Debug)]
pub struct Ports {
    pub moonraker: Vec<u16>,
    pub prusalink: Vec<u16>,
    pub flashforge: Vec<u16>,
    /// RTSP cameras (a Prusa Buddy3D is its own device on 554), offered to the Prusa found beside them.
    pub rtsp: Vec<u16>,
}
impl Default for Ports {
    fn default() -> Self {
        Self {
            moonraker: vec![7125, 80],
            prusalink: vec![80],
            flashforge: vec![8899],
            rtsp: vec![554],
        }
    }
}

/// This machine's own /24, found without sending a packet: asking the OS which address it would use to
/// reach the internet. `None` on a machine with no route; the caller then asks for a subnet.
pub fn local_subnet() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1: nothing is sent, a UDP connect only picks a route
    match s.local_addr().ok()? {
        SocketAddr::V4(a) => {
            let o = a.ip().octets();
            private(*a.ip()).then(|| format!("{}.{}.{}", o[0], o[1], o[2]))
        }
        SocketAddr::V6(_) => None,
    }
}

fn private(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_loopback()
}

/// `192.168.1` -> the 254 hosts; `192.168.1.139` -> that one host. Only a private network is ever scanned.
pub fn hosts_of(subnet: &str) -> Result<Vec<String>> {
    let text = subnet.trim().trim_end_matches(".0/24");
    let parts: Vec<&str> = text.split('.').collect();
    if !(3..=4).contains(&parts.len()) || parts.iter().any(|p| p.parse::<u8>().is_err()) {
        return Err(
            "type a network such as 192.168.1, or one address such as 192.168.1.139".into(),
        );
    }
    let probe: Ipv4Addr = format!("{}.1", parts[..3].join("."))
        .parse()
        .map_err(|_| "type a network such as 192.168.1")?;
    if !private(probe) {
        return Err(
            "only a private network (192.168.x.x, 10.x.x.x, 172.16-31.x.x) is scanned".into(),
        );
    }
    if parts.len() == 4 {
        return Ok(vec![text.to_owned()]);
    }
    Ok((1..=254)
        .map(|d| format!("{}.{d}", parts.join(".")))
        .collect())
}

/// `Silent` (no answer within the timeout) means the host is probably not there, so its other ports are
/// not tried; `Closed` (refused at once) means a host that is up with that port shut.
#[derive(PartialEq)]
enum Reach {
    Open,
    Closed,
    Silent,
}

fn reach(host: &str, port: u16, ms: u64) -> Reach {
    let Some(addr) = (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    else {
        return Reach::Silent;
    };
    match TcpStream::connect_timeout(&addr, Duration::from_millis(ms)) {
        Ok(_) => Reach::Open,
        Err(e)
            if e.kind() == std::io::ErrorKind::TimedOut
                || e.kind() == std::io::ErrorKind::WouldBlock =>
        {
            Reach::Silent
        }
        Err(_) => Reach::Closed,
    }
}

/// One short HTTP exchange, bounded: (status, www-authenticate header, body up to 16 KiB).
fn http_get(host: &str, port: u16, path: &str, ms: u64) -> Option<(u16, String, String)> {
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(ms)).ok()?;
    let t = Duration::from_millis((ms * 4).max(1200));
    let _ = s.set_read_timeout(Some(t));
    let _ = s.set_write_timeout(Some(t));
    write!(
        s,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut buf = Vec::new();
    let _ = s.take(16 * 1024).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let auth = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .starts_with("www-authenticate:")
                .then(|| l[17..].trim().to_owned())
        })
        .unwrap_or_default();
    Some((status, auth, body.to_owned()))
}

fn probe_moonraker(host: &str, port: u16, ms: u64) -> Option<Value> {
    let (status, _, body) = http_get(host, port, "/server/info", ms)?;
    let address = if port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    };
    let v: Value = serde_json::from_str(&body).ok()?;
    if status == 401 || status == 403 {
        return Some(
            json!({"driver":"moonraker","brand":"Klipper","address":address,"host":host,"name":Value::Null,
            "detail":"Moonraker asks for an API key","needs":["api_key"]}),
        );
    }
    let r = &v["result"];
    r["moonraker_version"].as_str()?;
    let info = http_get(host, port, "/printer/info", ms)
        .and_then(|(_, _, b)| serde_json::from_str::<Value>(&b).ok())
        .unwrap_or(Value::Null);
    let name = info["result"]["hostname"].as_str().map(str::to_owned);
    // A machine presented as Moonraker by a bridge (Kobra X through KX-Bridge) is named as itself.
    let bridge = crate::driver::BRIDGES.iter().find(|b| {
        [info["result"]["cpu_info"].as_str(), name.as_deref()]
            .iter()
            .flatten()
            .any(|n| b.signs.iter().any(|s| n.to_ascii_lowercase().contains(s)))
    });
    Some(
        json!({"driver":"moonraker","brand":bridge.map_or("Klipper", |b| b.brand),"model":bridge.map(|b| b.model),"address":address,"host":host,"name":name,
        "detail":format!("Moonraker {}, Klipper {}", r["moonraker_version"].as_str().unwrap_or("?"), r["klippy_state"].as_str().unwrap_or("?")),
        "needs":[]}),
    )
}

fn probe_prusalink(host: &str, port: u16, ms: u64) -> Option<Value> {
    let (status, auth, body) = http_get(host, port, "/api/version", ms)?;
    let address = if port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    };
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let text = v["text"].as_str().unwrap_or("");
    let digest = auth.to_ascii_lowercase().starts_with("digest");
    let looks = text.to_ascii_lowercase().contains("prusalink")
        || (status == 401 && digest && auth.to_ascii_lowercase().contains("printer api"));
    if !looks {
        return None;
    }
    Some(
        json!({"driver":"prusalink","brand":"PrusaLink","address":address,"host":host,
        "name":v["hostname"].as_str(),"detail":if text.is_empty() {"PrusaLink, login needed".to_owned()} else {text.to_owned()},
        "needs":["user","password"]}),
    )
}

fn probe_flashforge(host: &str, port: u16, ms: u64) -> Option<Value> {
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(ms)).ok()?;
    let t = Duration::from_millis((ms * 4).max(1500));
    let _ = s.set_read_timeout(Some(t));
    let _ = s.set_write_timeout(Some(t));
    let mut ask = |c: &str| -> String {
        let _ = s.write_all(format!("{c}\r\n").as_bytes());
        let mut buf = vec![0_u8; 4096];
        let mut out = Vec::new();
        let deadline = Instant::now() + t;
        while Instant::now() < deadline {
            match s.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&out).trim_end().ends_with("ok") {
                        break;
                    }
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let took = ask("~M601 S1").to_ascii_lowercase();
    if took.contains("control failed") {
        // Another program (FlashPrint, or this service's own driver) holds the one control session.
        return Some(
            json!({"driver":"flashforge","brand":"FlashForge","address":format!("http://{host}:8898"),"host":host,
            "name":Value::Null,"detail":"answers on the control port; another program is using it right now",
            "needs":["check_code"],"tcp_port":port}),
        );
    }
    if !took.contains("control success") {
        return None;
    }
    let info = ask("~M115");
    let _ = ask("~M602");
    let get = |k: &str| {
        info.find(k).and_then(|i| {
            info[i + k.len()..]
                .trim_start()
                .lines()
                .next()
                .map(|l| l.trim().to_owned())
        })
    };
    let machine = get("Machine Type:")?;
    let serial = get("SN:")
        .map(|s| s.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    Some(
        json!({"driver":"flashforge","brand":"FlashForge","address":format!("http://{host}:8898"),"host":host,
        "name":get("Machine Name:"),"model":machine,"serial":serial,
        "mac":get("Mac Address:").map(|s| s.split_whitespace().next().unwrap_or("").to_owned()),
        "detail":"TCP control answers; the check code is on the printer's panel","needs":["check_code"],"tcp_port":port}),
    )
}

/// An RTSP camera that serves `/live` (the Buddy3D path): one OPTIONS request, nothing is streamed.
fn probe_rtsp(host: &str, port: u16, ms: u64) -> Option<Value> {
    let addr: std::net::SocketAddr = format!("{host}:{port}").parse().ok()?;
    let url = if port == 554 {
        format!("rtsp://{host}/live")
    } else {
        format!("rtsp://{host}:{port}/live")
    };
    // Finish-2 F06 (2026-10-19): the Buddy3D was found on one search of the CORE One L's network and missed on another, a few minutes apart: it answers a
    // DESCRIBE slowly (or not at all) while a page or the plate check has its picture open, and 600 ms was all it was given. Only an address with the
    // RTSP port open gets here (a camera, one per search), so it is given 2.5 s and a second try.
    for _ in 0..2 {
        let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(ms.max(600))) else { continue };
        let _ = s.set_read_timeout(Some(Duration::from_millis(2500)));
        if s.write_all(format!("DESCRIBE {url} RTSP/1.0\r\nCSeq: 1\r\nAccept: application/sdp\r\n\r\n").as_bytes()).is_err() {
            continue;
        }
        let mut buf = [0_u8; 512];
        let Ok(n) = s.read(&mut buf) else { continue };
        if String::from_utf8_lossy(&buf[..n]).starts_with("RTSP/1.0 200") {
            return Some(json!({"driver":"camera","kind":"rtsp","address":url,"host":host}));
        }
    }
    None
}

/// `host[:port]` of an address with the scheme and path taken off.
fn authority(address: &str) -> String {
    address
        .trim_start_matches("http://")
        .trim_start_matches("rtsp://")
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .to_owned()
}

/// Scan `hosts`. Bounded concurrency (32 threads); a host that does not answer costs one timeout, not
/// one per port. Printers already in the farm are marked.
#[cfg(test)]
pub fn scan(
    hosts: &[String],
    ports: &Ports,
    added: &[(String, String, String)],
    timeout_ms: u64,
) -> Vec<Value> {
    scan_all(hosts, ports, added, timeout_ms).0
}

/// `scan`, and the RTSP cameras it saw as well (finish-2 F06, 2026-10-19): a camera is not a printer, but a person looking for
/// "the camera found beside my printer" is told which addresses answered, whether or not a Prusa sits beside them.
pub fn scan_all(
    hosts: &[String],
    ports: &Ports,
    added: &[(String, String, String)],
    timeout_ms: u64,
) -> (Vec<Value>, Vec<String>) {
    // (port, probe) in the order tried; Moonraker also answers on 80 behind Mainsail or Fluidd.
    type Probe = fn(&str, u16, u64) -> Option<Value>;
    let mut plan: Vec<(u16, Probe)> = Vec::new();
    plan.extend(
        ports
            .moonraker
            .iter()
            .map(|&p| (p, probe_moonraker as Probe)),
    );
    plan.extend(
        ports
            .prusalink
            .iter()
            .map(|&p| (p, probe_prusalink as Probe)),
    );
    plan.extend(
        ports
            .flashforge
            .iter()
            .map(|&p| (p, probe_flashforge as Probe)),
    );
    plan.extend(ports.rtsp.iter().map(|&p| (p, probe_rtsp as Probe)));
    let plan = &plan;
    let mut hits: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = hosts
            .chunks(hosts.len().div_ceil(32).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out: Vec<Value> = Vec::new();
                    for h in chunk {
                        let before = out.len();
                        for &(port, probe) in plan.iter() {
                            match reach(h, port, timeout_ms) {
                                Reach::Silent => break,
                                Reach::Closed => {}
                                Reach::Open => {
                                    if let Some(hit) = probe(h, port, timeout_ms) {
                                        // One line per brand per host: Moonraker on 7125 and on 80 is one printer.
                                        if !out[before..]
                                            .iter()
                                            .any(|o| o["driver"] == hit["driver"])
                                        {
                                            out.push(hit);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    // Cameras are not printers: take them out and offer the one beside a Prusa as its camera.
    let cameras: Vec<String> = hits
        .iter()
        .filter(|h| h["driver"] == "camera")
        .filter_map(|h| h["address"].as_str().map(str::to_owned))
        .collect();
    hits.retain(|h| h["driver"] != "camera");
    for h in hits.iter_mut().filter(|h| h["driver"] == "prusalink") {
        if cameras.len() == 1 {
            h["camera_hint"] = json!(cameras[0]);
        }
        if !cameras.is_empty() {
            h["cameras"] = json!(cameras);
        }
    }
    for h in hits.iter_mut() {
        let (addr, host) = (
            authority(h["address"].as_str().unwrap_or("")),
            h["host"].as_str().unwrap_or("").to_owned(),
        );
        let flash = h["driver"] == "flashforge";
        let found = added.iter().find(|(a, _, driver)| {
            let a = authority(a);
            let same = if flash {
                a.split(':').next() == Some(host.as_str())
            } else {
                a == addr
                    || (a == host && addr.ends_with(":80"))
                    || (addr == host && a.ends_with(":80"))
            };
            same && Some(driver.as_str()) == h["driver"].as_str()
        });
        h["added"] = json!(found.is_some());
        h["added_id"] = found.map(|(_, id, _)| json!(id)).unwrap_or(Value::Null);
    }
    hits.sort_by(|a, b| {
        let key = |v: &Value| {
            let h = v["host"].as_str().unwrap_or("");
            (
                h.split('.')
                    .filter_map(|p| p.parse::<u16>().ok())
                    .collect::<Vec<_>>(),
                v["address"].as_str().unwrap_or("").to_owned(),
            )
        };
        key(a).cmp(&key(b))
    });
    (hits, cameras)
}

/// Plain-words capability list for the connect step: what this printer will and will not do here.
/// Finish-2 F06 (2026-10-19): what only Klipper has (typing G-code, macros, gantry level and bed mesh) is not listed as "not available" on
/// another brand, where it was never a thing; and on another brand a camera is added by its address, so it is never listed as missing.
pub fn describe(c: &Capabilities) -> Vec<Value> {
    // (key, words, on, only a Klipper printer has it)
    [
        ("heat", "Set nozzle and bed temperatures", c.heat, false),
        ("pause_resume", "Pause and resume a print", c.pause_resume, false),
        ("cancel", "Cancel a print", c.cancel, false),
        ("start_print", "Start a file", c.start_print, false),
        ("upload", "Send files to the printer", c.upload, false),
        ("extrude", "Push and pull filament by a length", c.extrude, false),
        ("filament", "Load and unload filament", c.filament, false),
        ("jog", "Move the head by hand", c.jog, false),
        ("home", "Home the axes", c.home, false),
        ("fan", "Part fan", c.fan, false),
        ("speed", "Speed and flow factors", c.speed, false),
        ("console", "Type G-code", c.console, true),
        ("macros", "Run macros", c.macros, true),
        ("level", "Gantry level and bed mesh", c.level, true),
        ("live_xy", "Live head position", c.live_xy, false),
        ("camera", "Camera", c.camera, false),
        ("emergency_stop", "Emergency stop", c.emergency_stop, false),
    ]
    .iter()
    .filter(|(k, _, on, klipper_only)| *on || c.klipper || !(*klipper_only || *k == "camera"))
    .map(|(k, l, on, _)| json!({"key":k,"label":l,"on":on}))
    .collect()
}

/// Try a connection without saving anything. An unreachable printer or a wrong login is a normal
/// answer (`ok:false` with the reason), not an error.
pub fn test(
    base: &Config,
    driver_kind: &str,
    address: &str,
    credentials: &std::collections::BTreeMap<String, String>,
    camera: Option<&str>,
) -> Value {
    let kind = if driver_kind.is_empty() {
        "moonraker"
    } else {
        driver_kind
    };
    if !driver::KINDS.contains(&kind) {
        return json!({"ok":false,"error":format!("unknown brand {kind}")});
    }
    let url = address.trim().trim_end_matches('/');
    if !url.starts_with("http://") || url.len() <= 7 || url.contains(['?', '#', '@', ' ']) {
        return json!({"ok":false,"error":"the address must look like http://192.168.1.139"});
    }
    if let Some(c) = camera.filter(|c| !c.is_empty()) {
        if !config::camera_ok(c) {
            return json!({"ok":false,"error":"the camera address must be rtsp:// or http:// on this network"});
        }
    }
    let id = format!("test-{:08x}", {
        let mut b = [0_u8; 4];
        let _ = getrandom::fill(&mut b);
        u32::from_le_bytes(b)
    });
    let mut cfg = base.clone();
    cfg.printers.clear();
    cfg.printer_id = id.clone();
    cfg.driver = kind.to_owned();
    cfg.moonraker_url = url.to_owned();
    cfg.secret = config::Secret(credentials.clone());
    cfg.camera_url = camera.filter(|c| !c.is_empty()).map(str::to_owned);
    cfg.request_timeout_ms = cfg.request_timeout_ms.clamp(1500, 4000);
    let result = (|| -> Result<Value> {
        let mut d = driver::open(&cfg)?;
        // Name the machine first: a bridged printer's capabilities depend on what it is.
        let model = d.model_hint();
        if model.is_some() {
            cfg.printer_model = model.clone();
            crate::drivers::forget(&id);
            d = driver::open(&cfg)?;
        }
        let (sample, warnings) = d.sample(crate::model::now())?;
        // A wrong check code still reads through the printer's other channel, so the refusal of the first is what says so.
        if let Some(w) = warnings
            .iter()
            .find(|w| w.contains("check code is different"))
        {
            return Err(w
                .trim_start_matches("Live data is limited: ")
                .trim_end_matches("; the printer is still read")
                .to_owned());
        }
        let caps = d.capabilities();
        let job = &sample["print_stats"];
        Ok(json!({
            "ok":true,"brand":d.brand(),"model":model,"state":job["state"],"file":job["filename"],
            "temps":{"nozzle":sample["extruder"]["temperature"],"bed":sample["heater_bed"]["temperature"]},
            "capabilities":describe(&caps),"warnings":warnings,
        }))
    })();
    crate::drivers::forget(&id);
    match result {
        Ok(v) => v,
        Err(e) => json!({"ok":false,"error":plain_error(kind, url, &e)}),
    }
}

/// What the connection test says when it fails, in words a person can act on (finish-2 F06, 2026-10-19). The drivers' own sentences carry the
/// socket error ("io: Connection refused (os error 111)") and, for a Prusa address that is really a Klipper printer, "its firmware may be too old".
/// The first words stay the driver's (a log, a test and a support request still find them); what the person can do about it comes after.
pub fn plain_error(kind: &str, url: &str, e: &str) -> String {
    let host = url.trim_start_matches("http://").trim_end_matches('/');
    let lower = e.to_ascii_lowercase();
    if lower.contains("cannot reach") && (lower.contains("refused") || lower.contains("timed out") || lower.contains("unreachable") || lower.contains("lookup") || lower.contains("no route") || lower.contains("os error")) {
        return format!("Nothing answered at {host}: cannot reach the printer. Check that it is on and on the same network as this computer, and that the address is the one on the printer's own screen.");
    }
    if kind == "prusalink" && lower.contains("firmware may be too old") {
        return format!("{host} answers, but not like a Prusa with PrusaLink. Check that you picked the right brand (Klipper, Prusa or FlashForge). If it is a Prusa, its firmware may be too old for PrusaLink's newer interface: update it first.");
    }
    e.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeFlashForge, FakeMoonraker, FakePrusaLink};

    #[test]
    fn only_a_private_subnet_is_scanned_and_it_is_254_hosts() {
        let h = hosts_of("192.168.1").unwrap();
        assert_eq!(
            (h.len(), h[0].as_str(), h[253].as_str()),
            (254, "192.168.1.1", "192.168.1.254")
        );
        assert_eq!(hosts_of("192.168.1.0/24").unwrap().len(), 254);
        assert!(hosts_of("8.8.8").is_err());
        assert!(hosts_of("1.1.1").is_err());
        assert!(hosts_of("192.168").is_err());
        assert!(hosts_of("192.168.1.x").is_err());
        assert_eq!(hosts_of("192.168.1.139").unwrap(), vec!["192.168.1.139"]);
        assert!(hosts_of("8.8.8.8").is_err());
        assert!(hosts_of("10.0.0").is_ok() && hosts_of("172.16.5").is_ok());
    }

    #[test]
    fn the_three_brands_are_found_named_and_the_added_one_is_marked() {
        let (m, p, f) = (
            FakeMoonraker::start(0),
            FakePrusaLink::start(0, "maker", "secret"),
            FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000"),
        );
        let port = |a: &str| a.rsplit(':').next().unwrap().parse::<u16>().unwrap();
        let ports = Ports {
            moonraker: vec![port(&m.addr)],
            prusalink: vec![port(&p.addr)],
            flashforge: vec![f.tcp_port()],
            rtsp: vec![],
        };
        let hosts = vec!["127.0.0.1".to_owned(), "127.0.0.2".to_owned()];
        let hits = scan(
            &hosts,
            &ports,
            &[
                (m.url(), "stella".into(), "moonraker".into()),
                (f.http_url(), "other".into(), "prusalink".into()),
            ],
            400,
        );
        let by = |d: &str| {
            hits.iter()
                .find(|h| h["driver"] == d)
                .unwrap_or_else(|| panic!("{d} not found in {hits:?}"))
                .clone()
        };
        let klipper = by("moonraker");
        assert_eq!(klipper["brand"], "Klipper");
        assert_eq!(klipper["name"], "fake-voron");
        assert_eq!(klipper["added"], true);
        assert_eq!(klipper["added_id"], "stella");
        let prusa = by("prusalink");
        assert_eq!(
            prusa["needs"],
            json!(["user", "password"]),
            "it answered 401 with a digest challenge"
        );
        assert_eq!(prusa["added"], false);
        let ff = by("flashforge");
        assert_eq!(
            ff["added"], false,
            "a Klipper at the same host is not this FlashForge"
        );
        assert_eq!(ff["serial"], "SNFAKE5M00001");
        assert_eq!(ff["needs"], json!(["check_code"]));
        assert!(ff["model"].as_str().unwrap().contains("Adventurer 5M"));
        // The fake released control: the printer is usable by someone else straight after.
        assert!(!f.state.lock().unwrap().controlled);
        // Nothing but reads was sent to Moonraker.
        assert!(
            m.log().iter().all(|e| e["method"] == "GET"),
            "{:?}",
            m.log()
        );
    }

    #[test]
    fn a_connection_test_tells_what_works_and_what_is_wrong_in_plain_words() {
        let base = Config::default();
        let p = FakePrusaLink::start(0, "maker", "secret");
        let creds = |u: &str, pw: &str| {
            [
                ("user".to_owned(), u.to_owned()),
                ("password".to_owned(), pw.to_owned()),
            ]
            .into()
        };
        let ok = test(
            &base,
            "prusalink",
            &p.url(),
            &creds("maker", "secret"),
            None,
        );
        assert_eq!(ok["ok"], true, "{ok}");
        assert_eq!(ok["brand"], "PrusaLink");
        let caps = ok["capabilities"].as_array().unwrap();
        let on = |k: &str| {
            caps.iter().find(|c| c["key"] == k).unwrap()["on"]
                .as_bool()
                .unwrap()
        };
        // heat goes as a one-line file while the printer is idle (prusalink COMMAND_FILE); a hand jog still does not
        assert!(on("pause_resume") && on("heat") && !on("jog"));
        let bad = test(&base, "prusalink", &p.url(), &creds("maker", "wrong"), None);
        assert_eq!(bad["ok"], false);
        assert!(bad["error"].as_str().unwrap().contains("login"));
        assert!(
            !bad.to_string().contains("wrong"),
            "the password is never echoed"
        );
        // Nothing was saved: the registry holds no test driver afterwards.
        let off = test(
            &base,
            "prusalink",
            "http://127.0.0.1:9",
            &creds("maker", "secret"),
            None,
        );
        assert_eq!(off["ok"], false);
        assert!(off["error"].as_str().unwrap().contains("cannot reach"));
        assert_eq!(
            test(&base, "prusalink", "ftp://x", &creds("a", "b"), None)["ok"],
            false
        );
        assert_eq!(
            test(
                &base,
                "bambu",
                "http://127.0.0.1",
                &Default::default(),
                None
            )["ok"],
            false
        );
        assert_eq!(
            test(
                &base,
                "prusalink",
                &p.url(),
                &creds("maker", "secret"),
                Some("rtsp://8.8.8.8/x")
            )["ok"],
            false
        );
        // FlashForge: a wrong-case check code reads as the check code, not as a dead printer.
        let f = FakeFlashForge::start(0, 0, "SNFAKE5M00001", "0fake000");
        let ff = |code: &str| -> std::collections::BTreeMap<String, String> {
            [
                ("serial", "SNFAKE5M00001"),
                ("check_code", code),
                ("tcp_port", &f.tcp_port().to_string()),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
        };
        let good = test(&base, "flashforge", &f.http_url(), &ff("0fake000"), None);
        assert_eq!(good["ok"], true, "{good}");
        // The TCP door still answers for a wrong check code, so the HTTP door's refusal is what is reported.
        let upper = test(&base, "flashforge", &f.http_url(), &ff("0FAKE000"), None);
        assert_eq!(upper["ok"], false, "{upper}");
        let why = upper["error"].as_str().unwrap();
        assert!(
            why.contains("case sensitive")
                && !why.contains("0FAKE000")
                && !why.contains("0fake000"),
            "{why}"
        );
        // Nothing was left held: control is released after every test.
        assert!(!f.state.lock().unwrap().controlled);
    }

    /// Finish-2 F06 (2026-10-19): the connection test told a Prusa owner that it could not "run macros", "type G-code" or do a "gantry level and bed mesh", and
    /// that a camera was "not available" when a Prusa's camera is added by its address. Those are listed on a Klipper printer, where they are real.
    #[test]
    fn what_only_klipper_has_is_not_listed_as_missing_on_another_brand() {
        let keys = |c: &Capabilities| -> Vec<String> { describe(c).iter().map(|v| v["key"].as_str().unwrap().to_owned()).collect() };
        let prusa = Capabilities { pause_resume: true, cancel: true, start_print: true, upload: true, ..Capabilities::none() };
        let k = keys(&prusa);
        for gone in ["console", "macros", "level", "camera"] {
            assert!(!k.contains(&gone.to_owned()), "{gone} is not a thing to miss on a Prusa: {k:?}");
        }
        assert!(k.contains(&"heat".to_owned()) && k.contains(&"jog".to_owned()), "what a Prusa lacks and could have is still said: {k:?}");
        let klipper = Capabilities { klipper: true, ..Capabilities::none() };
        let k = keys(&klipper);
        for here in ["console", "macros", "level", "camera"] {
            assert!(k.contains(&here.to_owned()), "{here} is listed on a Klipper printer: {k:?}");
        }
    }

    /// The camera search: a camera that answers slowly is still found, and a camera with no Prusa beside it is reported on its own.
    #[test]
    fn a_slow_camera_is_found_and_reported_even_with_no_printer_beside_it() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            // the first connection is answered after a pause longer than the old 600 ms; a Buddy3D busy with another viewer does that
            for _ in 0..2 {
                let Ok((mut c, _)) = listener.accept() else { return };
                let mut buf = [0_u8; 512];
                let n = c.read(&mut buf).unwrap_or(0);
                if String::from_utf8_lossy(&buf[..n]).starts_with("DESCRIBE ") {
                    std::thread::sleep(std::time::Duration::from_millis(900));
                    let _ = c.write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n");
                    return;
                }
            }
        });
        let ports = Ports { moonraker: vec![], prusalink: vec![], flashforge: vec![], rtsp: vec![port] };
        let (printers, cameras) = scan_all(&["127.0.0.1".to_owned()], &ports, &[], 300);
        assert!(printers.is_empty(), "a camera is not a printer: {printers:?}");
        assert_eq!(cameras, vec![format!("rtsp://127.0.0.1:{port}/live")]);
        let _ = server.join();
    }

    /// The walk typed an address nothing answers on and a Klipper address given as a Prusa; both read as socket errors and "firmware too old".
    #[test]
    fn a_failed_connection_test_says_what_to_check_not_the_socket_error() {
        let base = Config::default();
        let none = test(&base, "prusalink", "http://127.0.0.1:9", &Default::default(), None);
        let why = none["error"].as_str().unwrap();
        assert!(why.starts_with("Nothing answered at 127.0.0.1:9") && why.contains("cannot reach the printer") && why.contains("on the same network"), "{why}");
        assert!(!why.contains("os error") && !why.contains("io:"), "no socket words: {why}");
        // a Klipper printer's address, given as a Prusa
        let klipper = crate::fakes::FakeMoonraker::start(0);
        let wrong = test(&base, "prusalink", &klipper.url(), &[("user".to_owned(), "maker".to_owned()), ("password".to_owned(), "x".to_owned())].into(), None);
        let why = wrong["error"].as_str().unwrap();
        assert!(why.contains("not like a Prusa with PrusaLink") && why.contains("right brand") && why.contains("firmware may be too old"), "{why}");
        // a wrong login keeps its own sentence
        let p = FakePrusaLink::start(0, "maker", "secret");
        let bad = test(&base, "prusalink", &p.url(), &[("user".to_owned(), "maker".to_owned()), ("password".to_owned(), "nope".to_owned())].into(), None);
        assert!(bad["error"].as_str().unwrap().contains("refused the login"), "{bad}");
    }
}
