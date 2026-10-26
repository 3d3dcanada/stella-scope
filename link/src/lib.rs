//! Stella Scope anywhere (2026-09-25: "access it on their phone anywhere as long as their
//! computer and their printer is on ... safely and securely").
//!
//! One idea: a TCP pipe over iroh. The desktop runs [`serve`]: an iroh endpoint with a key kept on
//! disk, so its id never changes. Every stream a phone opens is joined to Stella Scope's phone
//! listener (the LAN port), so the pairing code, the device cookie and every control check still
//! apply exactly as they do on the home network. The phone runs [`Tunnel`]: a listener on its own
//! loopback, and each connection the app's web view makes there travels as one iroh stream.
//!
//! iroh dials by public key, punches through NAT when it can and falls back to a relay when it
//! cannot. QUIC with TLS 1.3 end to end: a relay only forwards packets it cannot read.
use anyhow::{Context, Result};
use iroh::{endpoint::presets, Endpoint, EndpointAddr, EndpointId, SecretKey};
use std::{future::{Future, IntoFuture}, io::Write, net::SocketAddr, path::{Path, PathBuf}, str::FromStr, sync::Arc, time::Duration};
use tokio::{
    io::{copy, AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
};

/// The application protocol both sides speak. A change here is a breaking change.
pub const ALPN: &[u8] = b"stella-scope/link/1";
// Handshakes in flight, and phones that are not paired yet (only while a pairing code is showing).
const MAX_HANDSHAKES: usize = 16;
const MAX_UNPAIRED: usize = 2;
// Streams at once per connection, so one phone cannot starve another.
const MAX_FORWARDED_STREAMS: usize = 8;
const MAX_LOCAL_STREAMS: usize = 16;
const MAX_HTTP_HEADER: usize = 16 * 1024;
// A request over the stream caps waits this long for a slot, then is dropped. Dropping at once
// broke page loads: a browser opens up to six connections per page, two of them long-lived.
const SLOT_WAIT: Duration = Duration::from_secs(15);

// Browser cookies are shared across ports. Each printer gets a distinct browser cookie name;
// the phone tunnel sends only that printer's key under the desktop listener's expected name.
fn browser_cookie(remote: &EndpointId) -> String {
    format!("stella_device_{remote}")
}

async fn read_http_head<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 { return Err(std::io::ErrorKind::UnexpectedEof.into()); }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            if end + 4 > MAX_HTTP_HEADER { return Err(std::io::ErrorKind::InvalidData.into()); }
            return Ok(bytes);
        }
        if bytes.len() > MAX_HTTP_HEADER { return Err(std::io::ErrorKind::InvalidData.into()); }
    }
}

fn rewrite_http_head(bytes: &[u8], browser_name: &str, response: bool) -> std::io::Result<Vec<u8>> {
    let end = bytes.windows(4).position(|s| s == b"\r\n\r\n").ok_or(std::io::ErrorKind::InvalidData)?;
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let mut lines = text.split("\r\n");
    let mut out = String::new();
    out.push_str(lines.next().ok_or(std::io::ErrorKind::InvalidData)?);
    out.push_str("\r\n");
    let mut own = None;
    for line in lines {
        if !response && line.get(..7).is_some_and(|name| name.eq_ignore_ascii_case("Cookie:")) {
            for cookie in line[7..].split(';') {
                if let Some((name, value)) = cookie.trim().split_once('=') {
                    if name == browser_name && value.len() == 48 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
                        own = Some(value.to_owned());
                    }
                }
            }
            continue;
        }
        if response && line.get(..11).is_some_and(|name| name.eq_ignore_ascii_case("Set-Cookie:")) {
            if let Some(value) = line[11..].trim_start().strip_prefix("stella_device=") {
                out.push_str(&format!("Set-Cookie: {browser_name}={value}\r\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    if let Some(value) = own { out.push_str(&format!("Cookie: stella_device={value}\r\n")); }
    out.push_str("\r\n");
    let mut result = out.into_bytes();
    result.extend_from_slice(&bytes[end + 4..]);
    Ok(result)
}

// A client may keep its request half open after the HTTP response closes. Drop that copy task
// when the response finishes so one client cannot hold a forwarding slot indefinitely.
async fn copy_until_response(up: impl Future<Output = ()>, down: impl Future<Output = ()>) {
    tokio::pin!(up, down);
    tokio::select! {
        _ = &mut up => down.await,
        _ = &mut down => (),
    }
}

/// The endpoint key, created once (mode 600) and reused, so the id a phone paired with stays valid.
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    let read_key = || -> Result<SecretKey> {
        let bytes = std::fs::read(path).with_context(|| format!("cannot read the link key at {}", path.display()))?;
        let arr: [u8; 32] = bytes.as_slice().try_into().context("the link key file is not 32 bytes")?;
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?; }
        Ok(SecretKey::from_bytes(&arr))
    };
    match read_key() { Ok(key) => return Ok(key), Err(_) if !path.exists() => (), Err(error) => return Err(error) }
    let key = SecretKey::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return read_key(),
        Err(e) => return Err(e).with_context(|| format!("cannot create the link key at {}", path.display())),
    };
    file.write_all(&key.to_bytes())?;
    file.sync_all()?;
    Ok(key)
}

pub fn parse_id(text: &str) -> Result<EndpointId> {
    EndpointId::from_str(text.trim()).map_err(|e| anyhow::anyhow!("that is not a Stella link id: {e}"))
}

/// Desktop side. Runs until the process ends. `on_status` hears the id once bound and each time the
/// relay connection comes up. `allow` is the admission file Stella Scope keeps (see [`Admission`]);
/// without one every peer is admitted (the old behaviour, for a hand-run test).
pub async fn serve(key: SecretKey, forward: SocketAddr, allow: Option<PathBuf>, on_status: impl Fn(String) + Send + 'static) -> Result<()> {
    let ep = Endpoint::builder(presets::N0).secret_key(key).alpns(vec![ALPN.to_vec()]).bind().await?;
    serve_endpoint(ep, forward, allow, on_status).await
}

/// Who may hold a connection (2026-09-27: "all four slots aren't always full ... binding it to
/// one phone"). Stella Scope writes `{"peers":[<phone link ids>],"pairing_until":<unix s>}`: a paired
/// phone always gets in; any other phone only while the desktop shows a pairing code, and then at
/// most [`MAX_UNPAIRED`] at once. Everyone else is closed right after the handshake, so knowing a
/// printer's link id is no longer enough to fill its slots.
#[derive(Debug, PartialEq)]
pub enum Admission { Paired, Pairing, Refused }

pub fn admission(allow: Option<&Path>, peer: &EndpointId) -> Admission {
    let Some(path) = allow else { return Admission::Paired };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if text.contains(&format!("\"{peer}\"")) { return Admission::Paired; }
    let until = text.split("\"pairing_until\":").nth(1)
        .and_then(|t| t.split(|c: char| c != '.' && !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<f64>().ok()).unwrap_or(0.0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    if until > now { Admission::Pairing } else { Admission::Refused }
}

// The desktop listener learns which phone key a request came over, so a device key is bound to the
// phone that paired it. A client-sent header of the same name is removed first.
fn peer_head(bytes: &[u8], peer: &EndpointId) -> std::io::Result<Vec<u8>> {
    let end = bytes.windows(4).position(|s| s == b"\r\n\r\n").ok_or(std::io::ErrorKind::InvalidData)?;
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let mut out = String::new();
    for line in text.split("\r\n") {
        if line.get(..14).is_some_and(|n| n.eq_ignore_ascii_case("x-stella-peer:")) { continue; }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("X-Stella-Peer: {peer}\r\n\r\n"));
    let mut result = out.into_bytes();
    result.extend_from_slice(&bytes[end + 4..]);
    Ok(result)
}

type PeerCount = Arc<std::sync::Mutex<std::collections::HashMap<EndpointId, usize>>>;
struct Leave(PeerCount, EndpointId);
impl Drop for Leave {
    fn drop(&mut self) {
        if let Ok(mut map) = self.0.lock() {
            if let Some(n) = map.get_mut(&self.1) { *n -= 1; if *n == 0 { map.remove(&self.1); } }
        }
    }
}

/// [`serve`] on an endpoint already bound (the local end-to-end check binds one without relays).
pub async fn serve_endpoint(ep: Endpoint, forward: SocketAddr, allow: Option<PathBuf>, on_status: impl Fn(String) + Send + 'static) -> Result<()> {
    let id = ep.id();
    on_status(format!(r#"{{"id":"{id}","online":false}}"#));
    {
        let ep = ep.clone();
        tokio::spawn(async move {
            ep.online().await;
            let relay = ep.addr().relay_urls().next().map(|u| u.to_string()).unwrap_or_default();
            on_status(format!(r#"{{"id":"{}","online":true,"relay":"{relay}"}}"#, ep.id()));
        });
    }
    let allow = Arc::new(allow);
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    let unpaired = Arc::new(Semaphore::new(MAX_UNPAIRED));
    let per_peer: PeerCount = Default::default();
    while let Some(incoming) = ep.accept().await {
        let Ok(handshake) = handshakes.clone().try_acquire_owned() else { continue };
        let (allow, unpaired, per_peer) = (allow.clone(), unpaired.clone(), per_peer.clone());
        tokio::spawn(async move {
            let Ok(Ok(conn)) = tokio::time::timeout(Duration::from_secs(5), incoming.into_future()).await else { return };
            drop(handshake);
            let peer = conn.remote_id();
            let refuse = |conn: &iroh::endpoint::Connection| conn.close(1u32.into(), b"this phone is not paired with this printer");
            // A phone that is pairing holds one of the shared unpaired slots while it stays.
            let mut _unpaired_slot = None;
            match admission(allow.as_deref(), &peer) {
                Admission::Refused => return refuse(&conn),
                Admission::Pairing => match unpaired.try_acquire_owned() {
                    Ok(slot) => _unpaired_slot = Some(slot),
                    Err(_) => return refuse(&conn),
                },
                Admission::Paired => {}
            }
            // Two connections per phone at most (a redial overlaps the old one).
            let admitted = per_peer.lock().is_ok_and(|mut map| {
                let n = map.entry(peer).or_insert(0);
                *n < 2 && { *n += 1; true }
            });
            if !admitted { return refuse(&conn); }
            let _leave = Leave(per_peer.clone(), peer);
            let streams = Arc::new(Semaphore::new(MAX_FORWARDED_STREAMS));
            while let Ok(Ok((mut send, mut recv))) = tokio::time::timeout(Duration::from_secs(60), conn.accept_bi()).await {
                // Revoked, or the pairing window closed without this phone pairing: cut it off.
                if admission(allow.as_deref(), &peer) == Admission::Refused { refuse(&conn); break; }
                let streams = streams.clone();
                tokio::spawn(async move {
                    let Ok(Ok(_stream_slot)) = tokio::time::timeout(SLOT_WAIT, streams.acquire_owned()).await else { return };
                    let Ok(Ok(head)) = tokio::time::timeout(Duration::from_secs(10), read_http_head(&mut recv)).await else { return };
                    let Ok(head) = peer_head(&head, &peer) else { return };
                    let Ok(tcp) = TcpStream::connect(forward).await else { return };
                    let (mut r, mut w) = tcp.into_split();
                    let up = async {
                        if w.write_all(&head).await.is_ok() { let _ = copy(&mut recv, &mut w).await; }
                        let _ = w.shutdown().await;
                    };
                    let down = async {
                        let _ = copy(&mut r, &mut send).await;
                        let _ = send.finish();
                    };
                    copy_until_response(up, down).await;
                });
            }
        });
    }
    Ok(())
}

/// Phone side: a loopback port whose connections are carried to one paired Stella Scope.
pub struct Tunnel {
    pub port: u16,
    pub me: EndpointId,
}

impl Tunnel {
    pub async fn start(key: SecretKey, remote: EndpointId) -> Result<Tunnel> {
        Self::start_on(key, remote, 0).await
    }
    /// The same, on a chosen loopback port (the app keeps one port per printer for stable URLs;
    /// cookies are isolated by the header mapping above, because browser cookies ignore ports).
    /// Falls back to any available port when that one is occupied.
    pub async fn start_on(key: SecretKey, remote: EndpointId, port: u16) -> Result<Tunnel> {
        let ep = Endpoint::builder(presets::N0).secret_key(key).bind().await?;
        Self::start_endpoint(ep, EndpointAddr::new(remote), port).await
    }
    /// [`Tunnel::start_on`] with a bound endpoint and a full address (the local end-to-end check).
    pub async fn start_endpoint(ep: Endpoint, remote: EndpointAddr, port: u16) -> Result<Tunnel> {
        let me = ep.id();
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind("127.0.0.1:0").await?,
        };
        let port = listener.local_addr()?.port();
        // Dial once now, so the app can say "that computer is not answering" instead of loading an
        // empty page.
        let first = tokio::time::timeout(Duration::from_secs(25), ep.connect(remote.clone(), ALPN))
            .await
            .map_err(|_| anyhow::anyhow!("the computer did not answer within 25 s"))?
            .map_err(|e| anyhow::anyhow!("could not reach the computer: {e}"))?;
        let conn: Arc<Mutex<Option<iroh::endpoint::Connection>>> = Arc::new(Mutex::new(Some(first)));
        let local_streams = Arc::new(Semaphore::new(MAX_LOCAL_STREAMS));
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else { continue };
                let (ep, conn, remote, local_streams) = (ep.clone(), conn.clone(), remote.clone(), local_streams.clone());
                let browser_name = browser_cookie(&remote.id);
                tokio::spawn(async move {
                    // Browsers open spare connections ahead of need. The link stream is opened only
                    // when a request arrives, so a spare opened while the printer was refusing (or
                    // before the network changed) is not tied to a dead connection when it is used.
                    let (mut r, mut w) = tcp.into_split();
                    let Ok(Ok(first)) = tokio::time::timeout(Duration::from_secs(120), read_http_head(&mut r)).await else { return };
                    let Ok(first) = rewrite_http_head(&first, &browser_name, false) else { return };
                    let Ok(Ok(_local_slot)) = tokio::time::timeout(SLOT_WAIT, local_streams.acquire_owned()).await else { return };
                    // One QUIC connection shared by every request; redialled when it drops.
                    let streams = {
                        let mut guard = conn.lock().await;
                        let mut tries = 0;
                        loop {
                            if guard.is_none() {
                                match tokio::time::timeout(Duration::from_secs(20), ep.connect(remote.clone(), ALPN)).await {
                                    Ok(Ok(c)) => *guard = Some(c),
                                    _ => return,
                                }
                            }
                            match tokio::time::timeout(Duration::from_secs(20), guard.as_ref().unwrap().open_bi()).await {
                                Ok(Ok(s)) => break s,
                                Ok(Err(_)) if tries == 0 => {
                                    *guard = None;
                                    tries += 1;
                                }
                                _ => return,
                            }
                        }
                    };
                    let (mut send, mut recv) = streams;
                    let up = async {
                        if send.write_all(&first).await.is_err() { return; }
                        let _ = copy(&mut r, &mut send).await;
                        let _ = send.finish();
                    };
                    let down = async {
                        let Ok(Ok(first)) = tokio::time::timeout(Duration::from_secs(30), read_http_head(&mut recv)).await else { let _ = w.shutdown().await; return; };
                        let Ok(first) = rewrite_http_head(&first, &browser_name, true) else { let _ = w.shutdown().await; return; };
                        if w.write_all(&first).await.is_err() { return; }
                        let _ = copy(&mut recv, &mut w).await;
                        let _ = w.shutdown().await;
                    };
                    copy_until_response(up, down).await;
                });
            }
        });
        Ok(Tunnel { port, me })
    }
}

/// A steady loopback port for one printer id, 41000..=58999.
pub fn port_for(id: &EndpointId) -> u16 {
    let b = id.as_bytes();
    41000 + (u16::from_le_bytes([b[0], b[1]]) % 18000)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn response_end_releases_a_request_that_never_half_closes() {
        use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
        struct Release(Arc<AtomicBool>);
        impl Drop for Release { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
        let released = Arc::new(AtomicBool::new(false));
        let marker = released.clone();
        let (ready, wait) = tokio::sync::oneshot::channel::<()>();
        let up = async move {
            let _guard = Release(marker);
            let _ = ready.send(());
            std::future::pending::<()>().await;
        };
        super::copy_until_response(up, async move { let _ = wait.await; }).await;
        assert!(released.load(Ordering::SeqCst));
        let completed = Arc::new(AtomicBool::new(false));
        let marker = completed.clone();
        super::copy_until_response(async {}, async move { marker.store(true, Ordering::SeqCst); }).await;
        assert!(completed.load(Ordering::SeqCst));
    }

    #[test]
    fn one_printers_cookie_is_the_only_credential_forwarded() {
        let a = "a".repeat(48);
        let b = "b".repeat(48);
        let request = format!("GET /api/meta HTTP/1.1\r\nHost: 127.0.0.1:41000\r\nCookie: theme=dark; stella_device_first={a}; stella_device_second={b}\r\n\r\n");
        let first = String::from_utf8(super::rewrite_http_head(request.as_bytes(), "stella_device_first", false).unwrap()).unwrap();
        assert!(first.contains(&format!("Cookie: stella_device={a}\r\n")));
        assert!(!first.contains(&b));
        assert!(!first.contains("theme=dark"));
        let second = String::from_utf8(super::rewrite_http_head(request.as_bytes(), "stella_device_second", false).unwrap()).unwrap();
        assert!(second.contains(&format!("Cookie: stella_device={b}\r\n")));
        assert!(!second.contains(&a));
        let response = format!("HTTP/1.1 200 OK\r\nSet-Cookie: stella_device={a}; Path=/; HttpOnly; SameSite=Strict\r\nContent-Length: 1\r\n\r\nx");
        let rewritten = String::from_utf8(super::rewrite_http_head(response.as_bytes(), "stella_device_first", true).unwrap()).unwrap();
        assert!(rewritten.contains(&format!("Set-Cookie: stella_device_first={a}; Path=/; HttpOnly; SameSite=Strict")));
        assert!(rewritten.ends_with("\r\n\r\nx"));
    }

    #[tokio::test]
    async fn fragmented_http_headers_keep_the_body() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, mut reader) = tokio::io::duplex(128);
        tokio::spawn(async move {
            writer.write_all(b"POST /api/pair HTTP/1.1\r\nHost: 127.0.0.1:41000\r\n").await.unwrap();
            writer.write_all(b"Content-Length: 2\r\n\r\n{}").await.unwrap();
        });
        let bytes = super::read_http_head(&mut reader).await.unwrap();
        assert!(bytes.ends_with(b"\r\n\r\n{}"));
    }

    #[test]
    fn only_paired_phones_or_an_open_pairing_window_are_admitted() {
        use super::{admission, Admission};
        let phone = iroh::SecretKey::generate().public();
        let other = iroh::SecretKey::generate().public();
        let path = std::env::temp_dir().join(format!("stella-link-allow-test-{}", std::process::id()));
        assert_eq!(admission(None, &other), Admission::Paired, "no file: legacy open");
        std::fs::write(&path, format!(r#"{{"peers":["{phone}"],"pairing_until":0}}"#)).unwrap();
        assert_eq!(admission(Some(&path), &phone), Admission::Paired);
        assert_eq!(admission(Some(&path), &other), Admission::Refused);
        let soon = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() + 300.0;
        std::fs::write(&path, format!(r#"{{"peers":[],"pairing_until":{soon}}}"#)).unwrap();
        assert_eq!(admission(Some(&path), &other), Admission::Pairing);
        assert_eq!(admission(Some(&path), &phone), Admission::Pairing, "a revoked phone is a stranger again");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(admission(Some(&path), &phone), Admission::Refused, "a missing file refuses");
    }

    #[test]
    fn peer_header_is_set_by_the_link_and_never_by_the_client() {
        let peer = iroh::SecretKey::generate().public();
        let head = b"GET /api/meta HTTP/1.1\r\nHost: 127.0.0.1:41000\r\nx-stella-peer: forged\r\n\r\nbody";
        let out = String::from_utf8(super::peer_head(head, &peer).unwrap()).unwrap();
        assert!(!out.contains("forged"));
        assert!(out.contains(&format!("X-Stella-Peer: {peer}\r\n\r\nbody")));
    }

    #[test]
    fn link_key_is_private_and_stable() {
        let path = std::env::temp_dir().join(format!("stella-link-key-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let first = super::load_or_create_key(&path).unwrap();
        assert_eq!(first.to_bytes(), super::load_or_create_key(&path).unwrap().to_bytes());
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            super::load_or_create_key(&path).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
        std::fs::remove_file(path).unwrap();
    }
}
