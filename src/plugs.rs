//! Smart plugs (2026-10-13: "definitely should build smart plugs"): read a printer's power and
//! switch it, on this network only, never through anyone's cloud. Three local protocols:
//! Shelly (Gen2+ RPC, and Gen1 HTTP), Tasmota (HTTP commands) and TP-Link Kasa (the classic TCP 9999
//! protocol). A plug is set per printer as `shelly://host`, `tasmota://host` or `kasa://host`.
//! Plugs that only talk to their maker's cloud (Amazon Smart Plug, most Tuya plugs) cannot be driven
//! from here, and the settings say so instead of pretending.

use crate::Result;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Shelly,
    Tasmota,
    Kasa,
}

/// What a plug says now. Power and energy only when the plug measures them.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub on: bool,
    pub watts: Option<f64>,
    pub kwh: Option<f64>,
}

impl Reading {
    pub fn to_json(&self) -> Value {
        json!({"on":self.on,"watts":self.watts,"kwh":self.kwh})
    }
}

/// `shelly://192.168.1.50` -> (Shelly, "192.168.1.50"). A host is letters, digits, dots, dashes and an
/// optional port; nothing else (no path, no credentials), so a plug address can only ever reach a plug.
pub fn parse(spec: &str) -> Result<(Kind, String)> {
    let (scheme, host) = spec
        .trim()
        .split_once("://")
        .ok_or("a plug is shelly://, tasmota:// or kasa:// and its address")?;
    let kind = match scheme.to_ascii_lowercase().as_str() {
        "shelly" => Kind::Shelly,
        "tasmota" => Kind::Tasmota,
        "kasa" => Kind::Kasa,
        other => return Err(format!("{other} plugs are not supported here: Shelly, Tasmota and TP-Link Kasa work on the local network")),
    };
    let host = host.trim_end_matches('/');
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host, None),
    };
    let name_ok = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let port_ok = port.is_none_or(|p| p.parse::<u16>().is_ok_and(|n| n > 0));
    if !name_ok || !port_ok {
        return Err(
            "the plug's address is a host name or IP address, with an optional :port".into(),
        );
    }
    Ok((kind, host.to_owned()))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(2500)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into()
}

fn get_json(url: &str) -> Result<(u16, Value)> {
    let mut r = agent()
        .get(url)
        .call()
        .map_err(|e| format!("the plug did not answer: {e}"))?;
    let code = r.status().as_u16();
    let text = r
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Ok((code, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Kasa's framing: a 4-byte length, then the JSON XORed with a running key starting at 171.
fn kasa_encrypt(text: &str) -> Vec<u8> {
    let mut key = 171_u8;
    let mut out = (text.len() as u32).to_be_bytes().to_vec();
    for b in text.bytes() {
        key ^= b;
        out.push(key);
    }
    out
}
fn kasa_decrypt(bytes: &[u8]) -> String {
    let mut key = 171_u8;
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        out.push(key ^ b);
        key = b;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn kasa(host: &str, request: &Value) -> Result<Value> {
    let addr: SocketAddr = if host.contains(':') {
        host.parse()
    } else {
        format!("{host}:9999").parse()
    }
    .map_err(|_| "a Kasa plug is reached by its IP address")?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(2500))
        .map_err(|e| format!("the plug did not answer: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(2500)));
    s.write_all(&kasa_encrypt(&request.to_string()))
        .map_err(|e| e.to_string())?;
    let mut len = [0_u8; 4];
    s.read_exact(&mut len).map_err(|_| {
        "the Kasa plug closed the connection; newer Kasa firmware only talks to its app"
    })?;
    let n = u32::from_be_bytes(len) as usize;
    if n > 64 * 1024 {
        return Err("the Kasa plug's reply is too large".into());
    }
    let mut body = vec![0_u8; n];
    s.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_str(&kasa_decrypt(&body))
        .map_err(|_| "the Kasa plug's reply is not JSON".into())
}

fn tasmota_power(v: &Value) -> Option<bool> {
    let p = v["POWER"].as_str().or_else(|| v["POWER1"].as_str())?;
    Some(p.eq_ignore_ascii_case("ON"))
}

/// Read a plug.
pub fn read(spec: &str) -> Result<Reading> {
    let (kind, host) = parse(spec)?;
    match kind {
        Kind::Shelly => {
            let (code, v) = get_json(&format!("http://{host}/rpc/Switch.GetStatus?id=0"))?;
            if code == 200 && v["output"].is_boolean() {
                return Ok(Reading {
                    on: v["output"] == true,
                    watts: v["apower"].as_f64(),
                    kwh: v["aenergy"]["total"].as_f64().map(|wh| wh / 1000.0),
                });
            }
            // Gen1: /status, relays[0] and meters[0] (energy in watt-minutes)
            let (code, v) = get_json(&format!("http://{host}/status"))?;
            if code == 401 {
                return Err(
                    "the Shelly asks for a login; turn its restricted login off for this network"
                        .into(),
                );
            }
            let on = v["relays"][0]["ison"]
                .as_bool()
                .ok_or("that address is not a Shelly plug")?;
            Ok(Reading {
                on,
                watts: v["meters"][0]["power"].as_f64(),
                kwh: v["meters"][0]["total"].as_f64().map(|wm| wm / 60_000.0),
            })
        }
        Kind::Tasmota => {
            let (_, p) = get_json(&format!("http://{host}/cm?cmnd=Power"))?;
            let on = tasmota_power(&p)
                .ok_or("that address is not a Tasmota plug (or it asks for a password)")?;
            let (_, e) = get_json(&format!("http://{host}/cm?cmnd=Status%208"))?;
            let energy = &e["StatusSNS"]["ENERGY"];
            Ok(Reading {
                on,
                watts: energy["Power"].as_f64(),
                kwh: energy["Total"].as_f64(),
            })
        }
        Kind::Kasa => {
            let v = kasa(
                &host,
                &json!({"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}),
            )?;
            let on = v["system"]["get_sysinfo"]["relay_state"]
                .as_i64()
                .ok_or("that address is not a Kasa plug")?
                == 1;
            let rt = &v["emeter"]["get_realtime"];
            let watts = rt["power_mw"]
                .as_f64()
                .map(|mw| mw / 1000.0)
                .or_else(|| rt["power"].as_f64());
            let kwh = rt["total_wh"]
                .as_f64()
                .map(|wh| wh / 1000.0)
                .or_else(|| rt["total"].as_f64());
            Ok(Reading { on, watts, kwh })
        }
    }
}

/// Switch a plug, then read it back.
pub fn set(spec: &str, on: bool) -> Result<Reading> {
    let (kind, host) = parse(spec)?;
    match kind {
        Kind::Shelly => {
            let (code, _) = get_json(&format!("http://{host}/rpc/Switch.Set?id=0&on={on}"))?;
            if code != 200 {
                let (code, _) = get_json(&format!(
                    "http://{host}/relay/0?turn={}",
                    if on { "on" } else { "off" }
                ))?;
                if code != 200 {
                    return Err(format!("the Shelly answered {code}"));
                }
            }
        }
        Kind::Tasmota => {
            let (_, v) = get_json(&format!(
                "http://{host}/cm?cmnd=Power%20{}",
                if on { "On" } else { "Off" }
            ))?;
            if tasmota_power(&v) != Some(on) {
                return Err("the Tasmota plug did not switch".into());
            }
        }
        Kind::Kasa => {
            let v = kasa(
                &host,
                &json!({"system":{"set_relay_state":{"state":i32::from(on)}}}),
            )?;
            if v["system"]["set_relay_state"]["err_code"].as_i64() != Some(0) {
                return Err("the Kasa plug did not switch".into());
            }
        }
    }
    read(spec)
}

// ---- what every plug said last (the fleet watch fills it) ----------------------------------------

fn cache() -> &'static Mutex<HashMap<String, Value>> {
    static C: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read a printer's plug now and remember it.
pub fn refresh(printer: &str, spec: &str) -> Value {
    let v = match read(spec) {
        Ok(r) => json!({"ok":true,"at":crate::model::now(),"spec":spec,"reading":r.to_json()}),
        Err(e) => json!({"ok":false,"at":crate::model::now(),"spec":spec,"error":e}),
    };
    if let Ok(mut c) = cache().lock() {
        c.insert(printer.to_owned(), v.clone());
    }
    v
}

pub fn remember(printer: &str, spec: &str, r: &Reading) {
    if let Ok(mut c) = cache().lock() {
        c.insert(
            printer.to_owned(),
            json!({"ok":true,"at":crate::model::now(),"spec":spec,"reading":r.to_json()}),
        );
    }
}

/// The last thing a printer's plug said, if it has one.
pub fn status(printer: &str, spec: Option<&str>) -> Value {
    let Some(spec) = spec else { return Value::Null };
    cache()
        .lock()
        .ok()
        .and_then(|c| c.get(printer).filter(|v| v["spec"] == spec).cloned())
        .unwrap_or_else(|| json!({"ok":null,"spec":spec}))
}

// ---- finding plugs -------------------------------------------------------------------------------

fn open(host: &str, port: u16) -> bool {
    format!("{host}:{port}")
        .parse::<SocketAddr>()
        .ok()
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

/// Look for Shelly, Tasmota and Kasa plugs on these hosts. Reads only.
pub fn discover(hosts: &[String]) -> Vec<Value> {
    let mut found: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = hosts
            .chunks(hosts.len().div_ceil(32).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    for h in chunk {
                        if open(h, 80) {
                            if let Ok((200, v)) = get_json(&format!("http://{h}/shelly")) {
                                if v["mac"].is_string() || v["type"].is_string() || v["model"].is_string() {
                                    let name = v["name"].as_str().or_else(|| v["model"].as_str()).or_else(|| v["type"].as_str()).unwrap_or("Shelly");
                                    out.push(json!({"kind":"shelly","spec":format!("shelly://{h}"),"host":h,"name":name}));
                                    continue;
                                }
                            }
                            if let Ok((200, v)) = get_json(&format!("http://{h}/cm?cmnd=Status")) {
                                if v["Status"].is_object() {
                                    let name = v["Status"]["DeviceName"].as_str().or_else(|| v["Status"]["FriendlyName"][0].as_str()).unwrap_or("Tasmota");
                                    out.push(json!({"kind":"tasmota","spec":format!("tasmota://{h}"),"host":h,"name":name}));
                                    continue;
                                }
                            }
                        }
                        if open(h, 9999) {
                            if let Ok(v) = kasa(h, &json!({"system":{"get_sysinfo":{}}})) {
                                let s = &v["system"]["get_sysinfo"];
                                if s.is_object() {
                                    out.push(json!({"kind":"kasa","spec":format!("kasa://{h}"),"host":h,"name":s["alias"].as_str().or_else(|| s["model"].as_str()).unwrap_or("Kasa")}));
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
    found.sort_by(|a, b| a["host"].as_str().cmp(&b["host"].as_str()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{serve_http, Rsp};
    use std::{
        net::TcpListener,
        sync::{atomic::AtomicBool, Arc},
    };

    fn http_plug(
        handler: impl Fn(&str) -> Rsp + Send + Sync + 'static,
    ) -> (String, Arc<AtomicBool>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        serve_http(
            l,
            stop.clone(),
            Arc::new(move |req, _| Some(handler(&req.target))),
        );
        (addr, stop)
    }

    #[test]
    fn plug_addresses_reach_only_a_plug() {
        assert_eq!(
            parse("shelly://192.168.1.50").unwrap(),
            (Kind::Shelly, "192.168.1.50".into())
        );
        assert_eq!(parse("KASA://plug-3.local:9999").unwrap().0, Kind::Kasa);
        for bad in [
            "amazon://x",
            "shelly://",
            "shelly://a/b",
            "tasmota://u:p@host",
            "shelly://h:0",
            "http://host",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_shelly_and_a_tasmota_are_read_and_switched() {
        let on = Arc::new(Mutex::new(true));
        let s = on.clone();
        let (shelly, _a) = http_plug(move |t| {
            let mut o = s.lock().unwrap();
            if let Some(q) = t.strip_prefix("/rpc/Switch.Set?id=0&on=") {
                *o = q == "true";
                return Rsp::json(json!({"was_on":!*o}));
            }
            assert!(t.starts_with("/rpc/Switch.GetStatus"), "{t}");
            Rsp::json(
                json!({"id":0,"output":*o,"apower":if *o {182.5} else {0.0},"aenergy":{"total":12345.0}}),
            )
        });
        let r = read(&format!("shelly://{shelly}")).unwrap();
        assert_eq!(
            r,
            Reading {
                on: true,
                watts: Some(182.5),
                kwh: Some(12.345)
            }
        );
        let r = set(&format!("shelly://{shelly}"), false).unwrap();
        assert!(!r.on && r.watts == Some(0.0));

        let t_on = Arc::new(Mutex::new(false));
        let t = t_on.clone();
        let (tas, _b) = http_plug(move |q| {
            let mut o = t.lock().unwrap();
            match q {
                "/cm?cmnd=Power%20On" => *o = true,
                "/cm?cmnd=Power%20Off" => *o = false,
                "/cm?cmnd=Status%208" => {
                    return Rsp::json(json!({"StatusSNS":{"ENERGY":{"Power":95,"Total":3.21}}}))
                }
                _ => {}
            }
            Rsp::json(json!({"POWER":if *o {"ON"} else {"OFF"}}))
        });
        assert!(!read(&format!("tasmota://{tas}")).unwrap().on);
        let r = set(&format!("tasmota://{tas}"), true).unwrap();
        assert_eq!(
            r,
            Reading {
                on: true,
                watts: Some(95.0),
                kwh: Some(3.21)
            }
        );
    }

    #[test]
    fn kasa_framing_round_trips_and_a_kasa_plug_answers() {
        let msg = r#"{"system":{"get_sysinfo":{}}}"#;
        let e = kasa_encrypt(msg);
        assert_eq!(
            u32::from_be_bytes([e[0], e[1], e[2], e[3]]) as usize,
            msg.len()
        );
        assert_eq!(kasa_decrypt(&e[4..]), msg);
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten().take(1) {
                let mut len = [0_u8; 4];
                s.read_exact(&mut len).unwrap();
                let mut b = vec![0_u8; u32::from_be_bytes(len) as usize];
                s.read_exact(&mut b).unwrap();
                assert!(kasa_decrypt(&b).contains("get_sysinfo"));
                let reply = json!({"system":{"get_sysinfo":{"relay_state":1,"alias":"Stella"}},"emeter":{"get_realtime":{"power_mw":181000,"total_wh":5400}}});
                s.write_all(&kasa_encrypt(&reply.to_string())).unwrap();
            }
        });
        let r = read(&format!("kasa://{addr}")).unwrap();
        assert_eq!(
            r,
            Reading {
                on: true,
                watts: Some(181.0),
                kwh: Some(5.4)
            }
        );
    }
}
