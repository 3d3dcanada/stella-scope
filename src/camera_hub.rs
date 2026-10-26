//! One upstream per camera, shared by every viewer (farm board WO-04/05).
//!
//! The CORE One's camera is H.264 over RTSP and the AD5M's is MJPEG over HTTP; a browser page wants
//! JPEG frames from its own origin. A hub reads the camera once (ffmpeg for RTSP, a plain HTTP read for
//! MJPEG), cuts the byte stream into whole JPEG frames and hands each to every subscriber. The first
//! viewer starts the upstream; when the last has gone for five seconds it is closed. No frame is kept
//! beyond the newest, so a slow viewer drops frames rather than holding memory.
use crate::Result;
use std::{
    collections::HashMap,
    io::Read,
    process::{Command, Stdio},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

pub enum Msg {
    Frame(Arc<Vec<u8>>),
    Error(String),
}

pub(crate) struct Shared {
    subs: Mutex<Vec<SyncSender<Msg>>>,
    last: Mutex<Option<(Instant, Arc<Vec<u8>>)>>,
}

fn hubs() -> &'static Mutex<HashMap<String, Arc<Shared>>> {
    static H: OnceLock<Mutex<HashMap<String, Arc<Shared>>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Shared {
    /// Deliver to everyone; true while anyone is still listening.
    fn publish(&self, frame: Arc<Vec<u8>>) -> bool {
        if let Ok(mut l) = self.last.lock() {
            *l = Some((Instant::now(), frame.clone()));
        }
        let Ok(mut subs) = self.subs.lock() else {
            return false;
        };
        subs.retain(|tx| match tx.try_send(Msg::Frame(frame.clone())) {
            Ok(()) => true,
            // A viewer that is behind skips this frame; one that is gone is dropped.
            Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        });
        !subs.is_empty()
    }
    fn fail(&self, why: String) {
        if let Ok(mut subs) = self.subs.lock() {
            for tx in subs.drain(..) {
                let _ = tx.try_send(Msg::Error(why.clone()));
            }
        }
    }
}

/// Cuts a byte stream into whole JPEG images, SOI (FFD8) to EOI (FFD9). Inside JPEG scan data an FF is
/// always followed by 00 or an RSTn, so FFD9 is only ever the end of an image.
#[derive(Default)]
pub struct JpegSplitter {
    buf: Vec<u8>,
}
impl JpegSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(start) = self.buf.windows(2).position(|w| w == [0xFF, 0xD8]) else {
                // Keep a trailing FF in case the marker is split across reads.
                let keep = usize::from(self.buf.last() == Some(&0xFF));
                let drop = self.buf.len() - keep;
                self.buf.drain(..drop);
                break;
            };
            if start > 0 {
                self.buf.drain(..start);
            }
            let Some(end) = self.buf.windows(2).skip(2).position(|w| w == [0xFF, 0xD9]) else {
                break;
            };
            let end = end + 2 + 2;
            out.push(self.buf.drain(..end).collect());
        }
        if self.buf.len() > 8 * 1024 * 1024 {
            self.buf.clear(); // not a JPEG stream; do not grow without bound
        }
        out
    }
}

/// ffmpeg's arguments for an RTSP camera: TCP transport, no audio, MJPEG frames on stdout at a rate a
/// browser can show and a LAN can carry.
pub fn rtsp_args(url: &str, fps: u32) -> Vec<String> {
    [
        "-nostdin",
        "-loglevel",
        "error",
        "-rtsp_transport",
        "tcp",
        "-i",
        url,
        "-an",
        "-vf",
        "scale='min(1280,iw)':-2",
        "-r",
        &fps.to_string(),
        "-q:v",
        "6",
        "-f",
        "image2pipe",
        "-c:v",
        "mjpeg",
        "pipe:1",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

fn run_ffmpeg(url: String, shared: Arc<Shared>) {
    let mut child = match Command::new("ffmpeg")
        .args(rtsp_args(&url, 6))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            shared.fail(format!(
                "this camera is RTSP and needs ffmpeg to be shown here; install ffmpeg ({e})"
            ));
            return;
        }
    };
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };
    let mut split = JpegSplitter::default();
    let mut buf = vec![0_u8; 64 * 1024];
    let mut idle_since: Option<Instant> = None;
    loop {
        match out.read(&mut buf) {
            Ok(0) | Err(_) => {
                shared.fail(
                    "the camera stream ended; check that its RTSP stream is switched on".into(),
                );
                break;
            }
            Ok(n) => {
                for frame in split.push(&buf[..n]) {
                    if shared.publish(Arc::new(frame)) {
                        idle_since = None;
                    } else {
                        idle_since.get_or_insert_with(Instant::now);
                    }
                }
                if idle_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
                    break;
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// An MJPEG-over-HTTP camera read from the printer's own address.
fn run_http(host: String, port: u16, path: String, shared: Arc<Shared>) {
    use std::io::Write;
    use std::net::{TcpStream, ToSocketAddrs};
    let addr = match (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    {
        Some(a) => a,
        None => return shared.fail("the camera's address does not resolve".into()),
    };
    let mut up = match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(e) => return shared.fail(format!("camera not answering: {e}")),
    };
    let _ = up.set_read_timeout(Some(Duration::from_secs(10)));
    if write!(
        up,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return shared.fail("camera refused the request".into());
    }
    let mut split = JpegSplitter::default();
    let mut buf = vec![0_u8; 64 * 1024];
    let mut idle_since: Option<Instant> = None;
    loop {
        match up.read(&mut buf) {
            Ok(0) | Err(_) => {
                shared.fail("the camera stream ended".into());
                return;
            }
            Ok(n) => {
                for frame in split.push(&buf[..n]) {
                    if shared.publish(Arc::new(frame)) {
                        idle_since = None;
                    } else {
                        idle_since.get_or_insert_with(Instant::now);
                    }
                }
                if idle_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
                    return;
                }
            }
        }
    }
}

pub enum Source {
    Rtsp(String),
    Mjpeg {
        host: String,
        port: u16,
        path: String,
    },
    /// A camera read one still at a time (its snapshot address), at most `SNAPSHOT_FPS` a second.
    Snapshots {
        host: String,
        port: u16,
        path: String,
    },
}

impl Source {
    fn key(&self) -> String {
        match self {
            Source::Rtsp(u) => u.clone(),
            Source::Mjpeg { host, port, path } | Source::Snapshots { host, port, path } => {
                format!("http://{host}:{port}{path}")
            }
        }
    }
}

/// Finish-2 F05 (2026-10-19): Stella's C920 sends its full-rate MJPEG stream, about 340 kB a frame, and that one stream
/// alone starved the CORE One L's PrusaLink on our 2.4 GHz Wi-Fi (status reads timed out at 4 s; the farm showed the CORE
/// One L going offline and back six times a minute with the Cameras sheet open). A camera that has a snapshot address is
/// read one still at a time instead, the next asked only when the last has arrived, at most this many a second: a slow
/// link slows the pictures, never the printers. ponytail: fixed rate; a per-camera setting if a farm wants smoother video.
pub const SNAPSHOT_FPS: f64 = 3.0;

fn run_snapshots(host: String, port: u16, path: String, shared: Arc<Shared>) {
    use std::io::Write;
    use std::net::{TcpStream, ToSocketAddrs};
    let Some(addr) = (host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return shared.fail("the camera's address does not resolve".into());
    };
    let mut failures = 0;
    let mut idle_since: Option<Instant> = None;
    loop {
        let started = Instant::now();
        let frame = (|| -> std::result::Result<Vec<u8>, String> {
            let mut up = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
                .map_err(|e| format!("camera not answering: {e}"))?;
            let _ = up.set_read_timeout(Some(Duration::from_secs(10)));
            write!(up, "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                .map_err(|_| "camera refused the request".to_owned())?;
            let mut body = Vec::new();
            // A camera may end the connection with a reset once the picture is out; a whole picture is kept either way.
            let read = (&mut up).take(4 * 1024 * 1024).read_to_end(&mut body);
            match (JpegSplitter::default().push(&body).pop(), read) {
                (Some(f), _) => Ok(f),
                (None, Err(e)) => Err(format!("the camera stopped sending: {e}")),
                (None, Ok(_)) => Err("the camera did not send a picture".to_owned()),
            }
        })();
        match frame {
            Ok(f) => {
                failures = 0;
                if shared.publish(Arc::new(f)) {
                    idle_since = None;
                } else {
                    idle_since.get_or_insert_with(Instant::now);
                }
            }
            Err(e) => {
                failures += 1;
                if failures >= 3 {
                    return shared.fail(e);
                }
            }
        }
        if idle_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            return;
        }
        let gap = Duration::from_secs_f64(1.0 / SNAPSHOT_FPS);
        if let Some(rest) = gap.checked_sub(started.elapsed()) {
            thread::sleep(rest);
        }
    }
}

/// Subscribe to a camera, starting its upstream if nobody else has. The receiver gets whole JPEG frames
/// (newest first on a slow link: at most two wait).
pub fn subscribe(source: Source) -> Result<Receiver<Msg>> {
    subscribe_with(source.key(), move |shared| match source {
        Source::Rtsp(u) => run_ffmpeg(u, shared),
        Source::Mjpeg { host, port, path } => run_http(host, port, path, shared),
        Source::Snapshots { host, port, path } => run_snapshots(host, port, path, shared),
    })
}

/// The seam tests use: any function that publishes frames can be a source.
pub(crate) fn subscribe_with(
    key: String,
    run: impl FnOnce(Arc<Shared>) + Send + 'static,
) -> Result<Receiver<Msg>> {
    let (tx, rx) = sync_channel::<Msg>(2);
    let mut map = hubs()
        .lock()
        .map_err(|_| "camera hubs poisoned".to_owned())?;
    if let Some(shared) = map.get(&key) {
        if shared
            .subs
            .lock()
            .map(|s| {
                !s.is_empty()
                    || shared
                        .last
                        .lock()
                        .map(|l| {
                            l.as_ref()
                                .is_some_and(|(t, _)| t.elapsed() < Duration::from_secs(6))
                        })
                        .unwrap_or(false)
            })
            .unwrap_or(false)
        {
            shared
                .subs
                .lock()
                .map_err(|_| "camera hub poisoned".to_owned())?
                .push(tx);
            return Ok(rx);
        }
    }
    let shared = Arc::new(Shared {
        subs: Mutex::new(vec![tx]),
        last: Mutex::new(None),
    });
    map.insert(key.clone(), shared.clone());
    drop(map);
    let hub_key = key;
    thread::Builder::new()
        .name("camera-hub".into())
        .spawn(move || {
            run(shared.clone());
            if let Ok(mut m) = hubs().lock() {
                if m.get(&hub_key).is_some_and(|s| Arc::ptr_eq(s, &shared)) {
                    m.remove(&hub_key);
                }
            }
        })
        .map_err(|e| format!("cannot start the camera reader: {e}"))?;
    Ok(rx)
}

/// One frame: the newest the hub has if it is fresh, else wait for the next (up to `wait`).
pub fn snapshot(source: Source, wait: Duration) -> Result<Vec<u8>> {
    let key = source.key();
    if let Some(shared) = hubs().lock().ok().and_then(|m| m.get(&key).cloned()) {
        if let Some((t, f)) = shared.last.lock().ok().and_then(|l| l.clone()) {
            if t.elapsed() < Duration::from_millis(1500) {
                return Ok((*f).clone());
            }
        }
    }
    let rx = subscribe(source)?;
    match rx.recv_timeout(wait) {
        Ok(Msg::Frame(f)) => Ok((*f).clone()),
        Ok(Msg::Error(e)) => Err(e),
        Err(_) => Err("the camera did not send a frame in time".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn jpeg(tag: u8) -> Vec<u8> {
        vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, tag, tag, 0xFF, 0x00, 0x12, 0xFF, 0xD9,
        ]
    }

    #[test]
    fn frames_are_cut_whole_across_any_read_boundary() {
        let (a, b, c) = (jpeg(1), jpeg(2), jpeg(3));
        let stream: Vec<u8> = [
            b"--frame\r\nContent-Type: image/jpeg\r\n\r\n".as_slice(),
            &a,
            b"\r\n--frame\r\n",
            &b,
            &c,
        ]
        .concat();
        for step in [1_usize, 2, 3, 5, 7, 64] {
            let mut s = JpegSplitter::default();
            let mut got = Vec::new();
            for chunk in stream.chunks(step) {
                got.extend(s.push(chunk));
            }
            assert_eq!(
                got,
                vec![a.clone(), b.clone(), c.clone()],
                "read size {step}"
            );
        }
        // Garbage with no image never grows without bound.
        let mut s = JpegSplitter::default();
        for _ in 0..40 {
            s.push(&vec![0x41; 1024 * 1024]);
        }
        assert!(s.buf.len() <= 8 * 1024 * 1024 + 1024 * 1024);
    }

    /// Real ffmpeg output through the real splitter: five frames in, five whole JPEGs out. Skipped on a
    /// machine without ffmpeg (the service says so plainly there).
    #[test]
    fn real_ffmpeg_frames_split_into_whole_jpegs() {
        let Ok(out) = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x180:rate=5",
                "-frames:v",
                "5",
                "-q:v",
                "6",
                "-f",
                "image2pipe",
                "-c:v",
                "mjpeg",
                "pipe:1",
            ])
            .output()
        else {
            return;
        };
        assert!(out.status.success());
        let mut s = JpegSplitter::default();
        let mut frames = Vec::new();
        for chunk in out.stdout.chunks(1500) {
            frames.extend(s.push(chunk));
        }
        assert_eq!(frames.len(), 5);
        for f in &frames {
            assert_eq!(&f[..2], [0xFF, 0xD8]);
            assert_eq!(&f[f.len() - 2..], [0xFF, 0xD9]);
        }
    }

    #[test]
    fn rtsp_goes_through_ffmpeg_over_tcp_and_the_url_is_one_argument() {
        let a = rtsp_args("rtsp://192.168.1.132:554/live", 6);
        assert_eq!(&a[..2], ["-nostdin", "-loglevel"]);
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert_eq!(a[i + 1], "rtsp://192.168.1.132:554/live");
        assert!(a.windows(2).any(|w| w == ["-rtsp_transport", "tcp"]));
        // A URL with spaces or shell characters is still a single argument: there is no shell.
        let weird = rtsp_args("rtsp://h/a b;rm -rf", 6);
        assert_eq!(
            weird[weird.iter().position(|x| x == "-i").unwrap() + 1],
            "rtsp://h/a b;rm -rf"
        );
    }

    #[test]
    fn many_viewers_share_one_upstream_and_it_ends_when_they_leave() {
        static STARTS: AtomicUsize = AtomicUsize::new(0);
        let key = format!("test://{}", std::process::id());
        let source = |shared: Arc<Shared>| {
            STARTS.fetch_add(1, Ordering::Relaxed);
            for i in 0..2000_u32 {
                if !shared.publish(Arc::new(vec![(i % 250) as u8; 4])) && i > 3 {
                    thread::sleep(Duration::from_millis(1));
                }
                thread::sleep(Duration::from_millis(2));
                if shared.subs.lock().unwrap().is_empty() && i > 20 {
                    break;
                }
            }
        };
        let a = subscribe_with(key.clone(), source).unwrap();
        let b = subscribe_with(key.clone(), |_| {
            panic!("a second viewer must not start a second upstream")
        })
        .unwrap();
        for rx in [&a, &b] {
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                Msg::Frame(f) => assert_eq!(f.len(), 4),
                Msg::Error(e) => panic!("{e}"),
            }
        }
        assert_eq!(STARTS.load(Ordering::Relaxed), 1);
        drop((a, b));
        let start = Instant::now();
        while hubs().lock().unwrap().contains_key(&key) {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "the hub should close after the last viewer left"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Finish-2 F05: a camera read by snapshots sends whole pictures to its viewer and is asked at most SNAPSHOT_FPS times a
    /// second (Stella's full-rate C920 stream starved the CORE One L's Wi-Fi).
    #[test]
    fn a_snapshot_camera_is_asked_a_few_times_a_second_and_its_pictures_arrive() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = asked.clone();
        thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { continue };
                counted.fetch_add(1, Ordering::SeqCst);
                // read the whole request, so closing the socket is a clean end and not a reset
                let mut req = Vec::new();
                let mut b = [0_u8; 256];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => req.extend_from_slice(&b[..k]),
                    }
                }
                let body = jpeg(7);
                let _ = write!(s, "HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
        });
        let rx = subscribe(Source::Snapshots { host: "127.0.0.1".into(), port, path: "/?action=snapshot".into() }).unwrap();
        let start = Instant::now();
        let mut frames = 0;
        while start.elapsed() < Duration::from_secs(3) {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Msg::Frame(f)) => {
                    assert_eq!(*f, jpeg(7));
                    frames += 1;
                }
                Ok(Msg::Error(e)) => panic!("the reader failed: {e}"),
                Err(_) => {}
            }
        }
        let n = asked.load(Ordering::SeqCst);
        // a loaded machine (the whole suite in parallel) slows the pictures; the cap is what must hold
        assert!(frames >= 1 && n >= 2, "{frames} pictures, asked {n} times in 3 s");
        assert!(n <= (3.0 * SNAPSHOT_FPS) as usize + 1, "the camera was asked {n} times in 3 s");
    }
}
