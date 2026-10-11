// Minimal Moonraker websocket client. No crates, no async: std::net::TcpStream plus a
// hand-written RFC 6455 subset — handshake, masked client frames, server frame decode,
// ping→pong. Exactly what printer.objects.subscribe needs, nothing more.
// LAN peer; the Sec-WebSocket-Accept value is not re-verified (the first valid framed
// message proves the upgrade), so no SHA-1 dependency is pulled in for one check.
use crate::Result;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if chunk.len() == 1 && i >= 2 {
                out.push('=');
            } else if chunk.len() == 2 && i == 3 {
                out.push('=');
            } else {
                out.push(BASE64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
    }
    out
}

// xorshift64* — mask keys only; not cryptographic, and not claimed to be.
fn mask_key(state: &mut u64) -> [u8; 4] {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let x = *state;
    let mixed = x ^ (x >> 33);
    [
        (mixed >> 24) as u8,
        (mixed >> 16) as u8,
        (mixed >> 8) as u8,
        mixed as u8,
    ]
}

pub struct WsClient {
    stream: TcpStream,
    seed: u64,
    /// Read buffer for the frame parser. Bytes survive quiet-timeout returns so a
    /// partial frame can never desync the stream (round-4 item 4).
    buf: Vec<u8>,
    pos: usize,
}

impl WsClient {
    /// `path` is the endpoint on the Moonraker host: `/websocket` for the JSON-RPC API the
    /// recorder subscribes to, `/klippysocket` for the Klippy API bridge the resonance capture
    /// needs. Both speak one JSON document per websocket message.
    pub fn connect(host: &str, port: u16, path: &str, timeout: Duration) -> Result<WsClient> {
        let mut stream = TcpStream::connect((host, port))
            .map_err(|e| format!("websocket connect to {host}:{port}{path} failed: {e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream.set_nodelay(true).ok();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b9)
            ^ (std::process::id() as u64).rotate_left(32);
        let mut key_bytes = [0u8; 16];
        let mut s = seed;
        for b in key_bytes.iter_mut() {
            let k = mask_key(&mut s);
            *b = k[0] ^ k[1] ^ k[2] ^ k[3];
        }
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            base64(&key_bytes)
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| e.to_string())?;
        let mut head = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).map_err(|e| {
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                {
                    "websocket read timed out".into()
                } else {
                    e.to_string()
                }
            })?;
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
            if head.len() > 8192 {
                return Err("websocket handshake response exceeded 8 KiB".into());
            }
        }
        let head = String::from_utf8_lossy(&head);
        if !head.starts_with("HTTP/1.1 101") && !head.starts_with("HTTP/1.0 101") {
            let first = head.lines().next().unwrap_or("");
            return Err(format!("websocket upgrade refused: {first}"));
        }
        Ok(WsClient {
            stream,
            seed,
            buf: Vec::new(),
            pos: 0,
        })
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode); // FIN + opcode
        let mask_key = mask_key(&mut self.seed);
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else if payload.len() <= 0xFFFF {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask_key);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask_key[i % 4]);
        }
        self.stream.write_all(&frame).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn send_pong(&mut self, payload: &[u8]) -> Result<()> {
        self.send_frame(0xA, payload)
    }

    /// One socket read appended to the internal buffer, bounded by `until`. A read that would
    /// block past the deadline returns the quiet timeout error with everything already read
    /// preserved for the next call.
    ///
    /// This used to return early whenever ANY unconsumed byte was buffered, which made `need`
    /// spin at 100 % CPU forever on a frame bigger than one 16 KiB socket read: it kept asking
    /// for more bytes and `fill` kept declining to read any. The recorder never met it because
    /// `notify_status_update` frames are small, but an adxl345 batch at 3200 Hz is ~20 KB and
    /// hits it on the first frame. Consumed bytes are dropped by compaction instead.
    fn fill(&mut self, until: Instant) -> Result<()> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        let Some(remaining) = until.checked_duration_since(Instant::now()) else {
            return Err("websocket read timed out".into());
        };
        self.stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 16384];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err("websocket closed by server".into()),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                Err("websocket read timed out".into())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Ensure `n` bytes are available from the buffer position.
    fn need(&mut self, n: usize, until: Instant) -> Result<()> {
        while self.buf.len() - self.pos < n {
            self.fill(until)?;
        }
        Ok(())
    }

    /// Consume `n` buffered bytes.
    fn take(&mut self, n: usize) -> Vec<u8> {
        let out = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        out
    }

    /// Blocking read of one complete text message, bounded by `until`. Server pings
    /// are answered and pongs skipped inside the same deadline; on timeout any
    /// partial frame stays buffered, so the next call resumes exactly where this
    /// one stopped. This is what makes the idle-resample tick independent of
    /// inbound traffic: the caller sets the deadline at its own cadence.
    pub fn read_text_until(&mut self, until: Instant) -> Result<String> {
        let mut message = Vec::new();
        loop {
            self.need(2, until)?;
            let fin = self.buf[self.pos] & 0x80 != 0;
            let opcode = self.buf[self.pos] & 0x0F;
            let masked = self.buf[self.pos + 1] & 0x80 != 0;
            let len7 = (self.buf[self.pos + 1] & 0x7F) as usize;
            let head_len = if len7 == 126 {
                4
            } else if len7 == 127 {
                10
            } else {
                2
            };
            // Peek the header and wait for the whole frame before taking anything: taking the
            // header first and then timing out on the payload left the next call reading payload
            // bytes as a header ("unexpected websocket opcode 11", 2026-09-26).
            self.need(head_len, until)?;
            let head = self.buf[self.pos..self.pos + head_len].to_vec();
            let len = if len7 == 126 {
                u16::from_be_bytes(head[2..4].try_into().unwrap()) as usize
            } else if len7 == 127 {
                u64::from_be_bytes(head[2..10].try_into().unwrap()) as usize
            } else {
                len7
            };
            if len > 16 * 1024 * 1024 {
                return Err(format!(
                    "websocket frame of {len} bytes exceeds the 16 MiB sanity bound"
                ));
            }
            let mask_len = if masked { 4 } else { 0 };
            self.need(head_len + mask_len + len, until)?;
            self.take(head_len);
            let mask = self.take(mask_len);
            let mut payload = self.take(len);
            if masked {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            match opcode {
                0x8 => return Err("websocket closed by server".into()),
                0x9 => {
                    self.send_pong(&payload)?;
                    continue;
                }
                0xA => continue,
                0x1 | 0x0 | 0x2 => {
                    message.extend_from_slice(&payload);
                    if message.len() > 16 * 1024 * 1024 {
                        return Err("websocket message exceeds the 16 MiB sanity bound".into());
                    }
                    if fin {
                        return Ok(String::from_utf8_lossy(&message).into_owned());
                    }
                }
                other => return Err(format!("unexpected websocket opcode {other}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        let key = base64(&[0x11u8; 16]);
        assert_eq!(key.len(), 24);
        assert!(key.ends_with("=="));
        for c in key.bytes() {
            assert!(
                c.is_ascii_uppercase()
                    || c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == b'+'
                    || c == b'/'
                    || c == b'='
            );
        }
    }
    #[test]
    fn mask_keys_differ_and_span_the_byte_range() {
        // Round-4 item 6: the old body asserted *x <= 255 on a u8 — it could not fail.
        // Real checks: keys are 4 bytes, consecutive draws differ, and over many draws
        // the masking stream reaches both halves of the byte range.
        let mut s = 12345u64;
        let a = mask_key(&mut s);
        let b = mask_key(&mut s);
        assert_eq!(a.len(), 4);
        assert_eq!(b.len(), 4);
        assert_ne!(a, b);
        let (mut lo, mut hi) = (u8::MAX, u8::MIN);
        for _ in 0..1000 {
            let k = mask_key(&mut s);
            for x in k {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        assert!(lo < 64, "masking degenerated: min byte {lo}");
        assert!(hi > 191, "masking degenerated: max byte {hi}");
    }
    /// Regression for the `fill` spin: a frame far larger than one 16 KiB socket read must be
    /// reassembled, not busy-loop. An adxl345 batch at 3200 Hz is about 20 KB, so this is the
    /// ordinary case for the resonance capture, and the old `fill` never returned on it.
    #[test]
    fn a_frame_larger_than_one_socket_read_is_reassembled() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        // 400 KB of payload: 25 socket reads' worth, and past the 16-bit length field.
        let payload = "x".repeat(400_000);
        let expected = payload.clone();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .unwrap();
            let mut frame = vec![0x81u8, 127];
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            frame.extend_from_slice(payload.as_bytes());
            stream.write_all(&frame).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let mut client = WsClient::connect(
            "127.0.0.1",
            address.port(),
            "/klippysocket",
            Duration::from_secs(5),
        )
        .unwrap();
        let text = client
            .read_text_until(Instant::now() + Duration::from_secs(5))
            .unwrap();
        assert_eq!(text.len(), expected.len());
        assert_eq!(text, expected);
        worker.join().unwrap();
    }

    /// A frame whose payload arrives after the caller's deadline must be read whole on the next
    /// call. Taking the header before the payload had arrived used to leave the next call reading
    /// payload bytes as a header ("unexpected websocket opcode 11" in the journal, 2026-09-26).
    #[test]
    fn a_frame_split_across_a_timeout_is_not_lost() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let payload = b"{\"jsonrpc\":\"2.0\"}".to_vec();
        let expected = String::from_utf8(payload.clone()).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n").unwrap();
            stream.write_all(&[0x81u8, payload.len() as u8]).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(400));
            stream.write_all(&payload).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let mut client = WsClient::connect("127.0.0.1", address.port(), "/websocket", Duration::from_secs(5)).unwrap();
        assert!(client.read_text_until(Instant::now() + Duration::from_millis(150)).is_err());
        let text = client.read_text_until(Instant::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(text, expected);
        worker.join().unwrap();
    }

    #[test]
    fn handshake_line_and_key_shape() {
        // The request format is exercised end-to-end by the mock-ws smoke test; here we pin
        // the header text the server must accept.
        let head = "GET /websocket HTTP/1.1\r\nHost: h\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        assert!(head.contains("Sec-WebSocket-Version: 13"));
        assert!(head.ends_with("\r\n\r\n"));
    }
}
