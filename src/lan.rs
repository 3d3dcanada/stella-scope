//! Phone access (2026-09-24: "a way to be able to use this on my phone ... over the network").
//!
//! Off unless `lan_port` is set. The loopback phone listener answers only paired devices: a device pairs
//! by typing a six-digit code that the desktop shows for five minutes, and receives a random device
//! key in an HttpOnly, SameSite=Strict cookie. Every other request through the encrypted link without a
//! known key gets the pairing page, nothing else. Direct home-network HTTP is not supported.
//!
//! The control routes stay exactly as strict as on the desktop. A paired request must be
//! same-origin (its Origin, when sent, equals its own Host); the server then hands it to the
//! control service as the loopback listener would see it, so the control token, the exact-Host
//! check, the tiers and the reviews all still apply. Keys are stored beside the recording in
//! `<database>.devices.json` (mode 600) and can be revoked from the desktop.
//!
//! 2026-09-27: at most three phones, each bound to its own phone. A phone that pairs through the
//! link has its link key (the `X-Stella-Peer` header stella-link sets) stored with its device key;
//! later requests must arrive over that same phone key. `<database>.link.allow` tells stella-link
//! which phone keys are paired and until when a pairing window is open, so nobody else can hold a
//! connection (link/src/lib.rs `admission`).
use crate::{config::Config, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CODE_LIFETIME: Duration = Duration::from_secs(300);
const MAX_DEVICES: usize = 3;
static SAVE_ID: AtomicU64 = AtomicU64::new(0);

struct Pending {
    code: String,
    until: Instant,
    tries: u32,
}
fn pending() -> &'static Mutex<Option<Pending>> {
    static P: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| format!("no system randomness for pairing: {e}"))?;
    Ok(buf)
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
fn store_path(cfg: &Config) -> String {
    format!("{}.devices.json", cfg.database)
}
fn load(cfg: &Config) -> Vec<Value> {
    fs::read_to_string(store_path(cfg))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}
fn save(cfg: &Config, devices: &[Value]) -> Result<()> {
    write_private(
        &store_path(cfg),
        &Value::Array(devices.to_vec()).to_string(),
    )
    .map_err(|error| format!("cannot store paired devices: {error}"))?;
    write_allow(cfg)
}
pub fn allow_path(cfg: &Config) -> String {
    format!("{}.link.allow", cfg.database)
}
/// Rewrites stella-link's admission file from the paired devices and the open pairing window.
pub fn write_allow(cfg: &Config) -> Result<()> {
    let peers: Vec<Value> = load(cfg)
        .iter()
        .filter_map(|d| {
            d["peer"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(|p| json!(p))
        })
        .collect();
    let until = pending()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|p| p.until))
        .and_then(|u| u.checked_duration_since(Instant::now()))
        .map(|left| now() + left.as_secs_f64())
        .unwrap_or(0.0);
    write_private(
        &allow_path(cfg),
        &json!({"peers":peers,"pairing_until":until}).to_string(),
    )
    .map_err(|error| format!("cannot update the phone link's admission list: {error}"))
}
fn write_private(path: &str, text: &str) -> std::io::Result<()> {
    let temp = format!(
        "{path}.tmp-{}-{}",
        std::process::id(),
        SAVE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write = (|| -> std::io::Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// The device key from a Cookie header, if it names a paired device and, when that device paired
/// through the link, the request came over the same phone key.
pub fn device_from_cookie(
    cfg: &Config,
    cookie: Option<&String>,
    peer: Option<&String>,
) -> Option<String> {
    let key = cookie?
        .split(';')
        .find_map(|c| c.trim().strip_prefix("stella_device="))?
        .to_owned();
    load(cfg)
        .iter()
        .any(|d| {
            d["key"].as_str().is_some_and(|k| same(k, &key))
                && d["peer"]
                    .as_str()
                    .is_none_or(|p| peer.is_some_and(|q| q == p))
        })
        .then_some(key)
}

/// Desktop: start a pairing window and show its code.
pub fn new_code(cfg: &Config) -> Result<Value> {
    let r = random_bytes(4)?;
    let n = u32::from_le_bytes([r[0], r[1], r[2], r[3]]) % 1_000_000;
    let code = format!("{n:06}");
    *pending().lock().map_err(|_| "pairing state unavailable")? = Some(Pending {
        code: code.clone(),
        until: Instant::now() + CODE_LIFETIME,
        tries: 0,
    });
    write_allow(cfg)?;
    Ok(json!({"code":code,"expires_in_s":CODE_LIFETIME.as_secs()}))
}

/// Phone: trade the code for a device key. Five wrong tries end the window.
pub fn pair(
    cfg: &Config,
    body: &[u8],
    agent: Option<&String>,
    peer: Option<&String>,
) -> Result<String> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "send {\"code\":\"123456\"}")?;
    let code = v["code"].as_str().unwrap_or("").trim().to_owned();
    let mut guard = pending().lock().map_err(|_| "pairing state unavailable")?;
    let Some(p) = guard.as_mut() else {
        return Err("no pairing is open; press Pair a phone on the desktop first".into());
    };
    if Instant::now() > p.until {
        *guard = None;
        drop(guard);
        let _ = write_allow(cfg);
        return Err("that code has expired; start pairing again on the desktop".into());
    }
    if !same(&p.code, &code) {
        p.tries += 1;
        if p.tries >= 5 {
            *guard = None;
            drop(guard);
            let _ = write_allow(cfg);
            return Err("too many wrong codes; start pairing again on the desktop".into());
        }
        return Err("that code is not the one on the desktop".into());
    }
    let mut devices = load(cfg);
    if devices.len() >= MAX_DEVICES {
        return Err(format!(
            "{MAX_DEVICES} phones are paired already; remove one on the desktop first"
        ));
    }
    *guard = None;
    drop(guard);
    let key: String = random_bytes(24)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let name: String = agent
        .map(|a| {
            if a.contains("Android") {
                "Android phone"
            } else if a.contains("iPhone") {
                "iPhone"
            } else if a.contains("iPad") {
                "iPad"
            } else {
                "Browser"
            }
        })
        .unwrap_or("Browser")
        .to_owned();
    let id: String = random_bytes(4)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut device = json!({"id":id,"key":key,"name":name,"paired_at":now()});
    if let Some(peer) = peer.filter(|p| p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit())) {
        device["peer"] = json!(peer.to_ascii_lowercase());
    }
    devices.push(device);
    save(cfg, &devices)?;
    Ok(key)
}

/// Desktop: the paired devices (keys never leave the store).
pub fn devices(cfg: &Config) -> Value {
    let list: Vec<Value> = load(cfg)
        .iter()
        .map(|d| json!({"id":d["id"],"name":d["name"],"paired_at":d["paired_at"]}))
        .collect();
    json!({"devices":list,"enabled":cfg.lan_port.is_some(),"max":MAX_DEVICES})
}
pub fn revoke(cfg: &Config, body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "send {\"id\":\"...\"}")?;
    let id = v["id"]
        .as_str()
        .ok_or("send the id of the device to remove")?;
    let mut list = load(cfg);
    let before = list.len();
    list.retain(|d| d["id"].as_str() != Some(id));
    if list.len() == before {
        return Err("no paired device with that id".into());
    }
    save(cfg, &list)?;
    Ok(devices(cfg))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_device_pairs_once_with_the_desktop_code_and_can_be_removed() {
        let mut cfg = crate::config::Config::default();
        cfg.database = format!("/tmp/stella-lan-test-{}", std::process::id());
        cfg.lan_port = Some(7422);
        let _ = std::fs::remove_file(super::store_path(&cfg));
        assert!(
            super::pair(&cfg, br#"{"code":"000000"}"#, None, None).is_err(),
            "no window open"
        );
        let code = super::new_code(&cfg).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_owned();
        let allow = std::fs::read_to_string(super::allow_path(&cfg)).unwrap();
        assert!(
            !allow.contains("\"pairing_until\":0"),
            "an open window is announced to the link: {allow}"
        );
        let wrong = if code == "111111" { "222222" } else { "111111" };
        assert!(super::pair(
            &cfg,
            format!(r#"{{"code":"{wrong}"}}"#).as_bytes(),
            None,
            None
        )
        .is_err());
        let phone = "ab".repeat(32);
        let key = super::pair(
            &cfg,
            format!(r#"{{"code":"{code}"}}"#).as_bytes(),
            Some(&"Mozilla Android".into()),
            Some(&phone),
        )
        .unwrap();
        assert!(
            super::pair(
                &cfg,
                format!(r#"{{"code":"{code}"}}"#).as_bytes(),
                None,
                None
            )
            .is_err(),
            "a code works once"
        );
        let allow = std::fs::read_to_string(super::allow_path(&cfg)).unwrap();
        assert!(
            allow.contains(&phone) && allow.contains("\"pairing_until\":0"),
            "paired phone admitted, window closed: {allow}"
        );
        let cookie = format!("theme=dark; stella_device={key}");
        assert_eq!(
            super::device_from_cookie(&cfg, Some(&cookie), Some(&phone)),
            Some(key.clone())
        );
        assert_eq!(
            super::device_from_cookie(&cfg, Some(&cookie), None),
            None,
            "bound to its phone"
        );
        assert_eq!(
            super::device_from_cookie(&cfg, Some(&cookie), Some(&"cd".repeat(32))),
            None,
            "another phone's key fails"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(super::store_path(&cfg))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
        assert_eq!(
            super::device_from_cookie(&cfg, Some(&"stella_device=nope".to_owned()), Some(&phone)),
            None
        );
        let id = super::devices(&cfg)["devices"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            !super::devices(&cfg).to_string().contains(&key),
            "keys never leave the store"
        );
        super::revoke(&cfg, format!(r#"{{"id":"{id}"}}"#).as_bytes()).unwrap();
        assert_eq!(
            super::device_from_cookie(&cfg, Some(&cookie), Some(&phone)),
            None
        );
        assert!(
            !std::fs::read_to_string(super::allow_path(&cfg))
                .unwrap()
                .contains(&phone),
            "revoked phone leaves the link's list"
        );
        // Three phones at most.
        for n in 0..3 {
            let code = super::new_code(&cfg).unwrap()["code"]
                .as_str()
                .unwrap()
                .to_owned();
            let result = super::pair(
                &cfg,
                format!(r#"{{"code":"{code}"}}"#).as_bytes(),
                None,
                Some(&format!("{n}").repeat(64)),
            );
            assert!(result.is_ok(), "phone {n}");
        }
        let code = super::new_code(&cfg).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(super::pair(
            &cfg,
            format!(r#"{{"code":"{code}"}}"#).as_bytes(),
            None,
            Some(&"e".repeat(64))
        )
        .unwrap_err()
        .contains("3 phones"));
        let _ = std::fs::remove_file(super::store_path(&cfg));
        let _ = std::fs::remove_file(super::allow_path(&cfg));
    }
}
