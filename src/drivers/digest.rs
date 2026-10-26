//! HTTP Digest authentication, MD5 and qop=auth (RFC 7616 / 2617), for PrusaLink. Written here because
//! nothing in the dependency tree does it and one hash is not worth a crate (farm board WO-04). MD5 is
//! the digest the protocol names; nothing in this service uses it for anything else.
use std::fmt::Write;

pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x67452301_u32,
        0xefcdab89_u32,
        0x98badcfe_u32,
        0x10325476_u32,
    );
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (mut f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0_u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn md5_hex(s: &str) -> String {
    let mut out = String::with_capacity(32);
    for b in md5(s.as_bytes()) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// What the printer asked for in its `WWW-Authenticate: Digest ...` header.
#[derive(Clone, Debug, PartialEq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    /// qop=auth offered (the only one used). Without it the older RFC 2069 response is made.
    pub qop_auth: bool,
}

/// Parse `Digest realm="x", nonce="y", qop="auth", opaque="z", algorithm=MD5`. Anything but MD5 (or
/// absent, which means MD5) is refused: no other algorithm is built here.
pub fn parse_challenge(header: &str) -> Option<Challenge> {
    let rest = header
        .trim()
        .strip_prefix("Digest")
        .or_else(|| header.trim().strip_prefix("digest"))?
        .trim();
    let mut fields = std::collections::BTreeMap::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = rest[start..i].trim().to_ascii_lowercase();
        i += 1;
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let s = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let v = rest[s..i].to_owned();
            i += 1;
            v
        } else {
            let s = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            rest[s..i].trim().to_owned()
        };
        fields.insert(key, value);
    }
    if let Some(alg) = fields.get("algorithm") {
        if !alg.eq_ignore_ascii_case("md5") {
            return None;
        }
    }
    Some(Challenge {
        realm: fields.get("realm")?.clone(),
        nonce: fields.get("nonce")?.clone(),
        opaque: fields.get("opaque").cloned(),
        qop_auth: fields
            .get("qop")
            .is_some_and(|q| q.split(',').any(|x| x.trim() == "auth")),
    })
}

/// The `Authorization` value for one request. `cnonce` is passed in so a test can pin it.
pub fn authorization(
    c: &Challenge,
    user: &str,
    pass: &str,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let ha1 = md5_hex(&format!("{user}:{}:{pass}", c.realm));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let mut out = format!(
        "Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\"",
        c.realm, c.nonce
    );
    if c.qop_auth {
        let response = md5_hex(&format!("{ha1}:{}:{nc:08x}:{cnonce}:auth:{ha2}", c.nonce));
        let _ = write!(
            out,
            ", algorithm=MD5, response=\"{response}\", qop=auth, nc={nc:08x}, cnonce=\"{cnonce}\""
        );
    } else {
        let response = md5_hex(&format!("{ha1}:{}:{ha2}", c.nonce));
        let _ = write!(out, ", algorithm=MD5, response=\"{response}\"");
    }
    if let Some(o) = &c.opaque {
        let _ = write!(out, ", opaque=\"{o}\"");
    }
    out
}

/// A digest session: the last challenge and a counter, so the next request is authorised without a
/// round trip. A 401 teaches it the printer's new nonce.
pub struct Session {
    user: String,
    pass: String,
    challenge: Option<Challenge>,
    nc: u32,
}
impl Session {
    pub fn new(user: &str, pass: &str) -> Self {
        Self {
            user: user.to_owned(),
            pass: pass.to_owned(),
            challenge: None,
            nc: 0,
        }
    }
    pub fn authorize(&mut self, method: &str, uri: &str) -> Option<String> {
        let c = self.challenge.clone()?;
        self.nc += 1;
        let mut raw = [0_u8; 8];
        getrandom::fill(&mut raw).ok()?;
        let cnonce: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        Some(authorization(
            &c, &self.user, &self.pass, method, uri, self.nc, &cnonce,
        ))
    }
    /// Learn from a `WWW-Authenticate` header. False when it is not a Digest MD5 challenge.
    pub fn learn(&mut self, header: &str) -> bool {
        match parse_challenge(header) {
            Some(c) => {
                self.challenge = Some(c);
                self.nc = 0;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vectors() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex("message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    /// RFC 2617 section 3.5 example, to the letter.
    #[test]
    fn rfc_2617_example() {
        let c = parse_challenge(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .unwrap();
        assert!(c.qop_auth);
        let header = authorization(
            &c,
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            1,
            "0a4f113b",
        );
        assert!(
            header.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{header}"
        );
        assert!(header.contains("nc=00000001") && header.contains(r#"cnonce="0a4f113b""#));
        assert!(header.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
    }

    #[test]
    fn only_md5_is_accepted_and_the_secret_is_never_in_the_header() {
        assert!(
            parse_challenge(r#"Digest realm="x", nonce="y", algorithm=SHA-256, qop="auth""#)
                .is_none()
        );
        assert!(parse_challenge(r#"Basic realm="x""#).is_none());
        assert!(
            parse_challenge(r#"Digest realm="Printer API", nonce="abc", algorithm=MD5"#).is_some()
        );
        let mut s = Session::new("maker", "hunter2");
        assert!(
            s.authorize("GET", "/api/v1/status").is_none(),
            "no challenge yet"
        );
        assert!(s.learn(r#"Digest realm="Printer API", nonce="n1", qop="auth""#));
        let h = s.authorize("GET", "/api/v1/status").unwrap();
        assert!(h.starts_with("Digest username=\"maker\"") && !h.contains("hunter2"));
        let h2 = s.authorize("GET", "/api/v1/status").unwrap();
        assert!(h2.contains("nc=00000002"), "the counter moves: {h2}");
    }
}
