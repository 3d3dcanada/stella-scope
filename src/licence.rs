//! The farm licence (farm board WO-09). Stella Scope is free for up to five printers. Beyond that a farm
//! licence is asked for, on trust: nothing is disabled, nothing phones home, there is no account. A
//! licence is a short piece of text that a person pastes in and that is checked here, offline, against
//! a public key compiled into the program.
//!
//! Format, one line: `SFL1.<payload>.<signature>`, both parts base64url without padding. The payload is
//! JSON, `{"name":"...","printers":N,"issued":"YYYY-MM-DD"}`, and the signature is Ed25519 over the
//! payload's exact bytes. Changing one character of the name or the count breaks the signature.
//!
//! The private key is never in this repository: `tools/farm-licence.rs` makes and uses it from a path
//! outside any git work tree, named by an environment variable.
use serde_json::{json, Value};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

/// Printers that need no licence and are never nagged about.
pub const FREE_PRINTERS: usize = 5;

/// One trial, per farm, 30 days from the day the sixth printer is added.
pub const TRIAL_SECONDS: f64 = 30.0 * 86400.0;

/// The public key licences are checked against (Ed25519, 32 bytes), made by `farm-licence keygen` on
/// 2026-10-12. Its private half is in `~/.config/stella-scope/farm-licence.key` on the build machine,
/// outside the repository. Replacing it means making a new key pair and rebuilding.
#[cfg(not(feature = "test-licence-key"))]
pub const PUBLIC_KEY: [u8; 32] = [
    159, 66, 103, 158, 123, 66, 160, 18, 25, 136, 166, 181, 88, 163, 53, 133, 206, 18, 209, 17, 21,
    20, 47, 226, 113, 46, 226, 7, 5, 226, 236, 92,
];

/// The test build's key (see Cargo.toml, feature `test-licence-key`): 64 hex digits from STELLA_TEST_LICENCE_PUBKEY, read at build time.
#[cfg(feature = "test-licence-key")]
pub const PUBLIC_KEY: [u8; 32] = test_key(env!("STELLA_TEST_LICENCE_PUBKEY"));
#[cfg(feature = "test-licence-key")]
const fn test_key(hex: &str) -> [u8; 32] {
    const fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("STELLA_TEST_LICENCE_PUBKEY must be hex digits"),
        }
    }
    let b = hex.as_bytes();
    assert!(b.len() == 64, "STELLA_TEST_LICENCE_PUBKEY must be 64 hex digits");
    let mut out = [0_u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(b[2 * i]) << 4) | nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Licence {
    pub name: String,
    pub printers: u32,
    pub issued: String,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0_u32, 0);
    for ch in s.bytes() {
        let v = B64
            .iter()
            .position(|&b| b == ch)
            .ok_or("the licence has a character that does not belong in it")?
            as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Check a licence's text against `key`. The reasons are in words a person can act on.
pub fn verify_with(text: &str, key: &[u8; 32]) -> Result<Licence> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parts = text.split('.');
    let (Some("SFL1"), Some(payload), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(
            "This is not a Stella Scope farm licence. It starts SFL1. and has two parts after it."
                .into(),
        );
    };
    let payload_bytes = b64_decode(payload)?;
    let sig_bytes: [u8; 64] = b64_decode(sig)?.try_into().map_err(|_| {
        "The licence is cut short or damaged: its signature is the wrong length.".to_owned()
    })?;
    let vk = VerifyingKey::from_bytes(key).map_err(|_| "This copy of Stella Scope has no licence key built in yet, so it cannot check a licence.".to_owned())?;
    vk.verify(&payload_bytes, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| {
            "This licence does not check out: it was changed, or it was not made for Stella Scope."
                .to_owned()
        })?;
    let v: Value = serde_json::from_slice(&payload_bytes)
        .map_err(|_| "The licence is signed but unreadable.".to_owned())?;
    let name = v["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty() && n.len() <= 120)
        .ok_or("The licence has no name in it.")?
        .to_owned();
    let printers = v["printers"]
        .as_u64()
        .filter(|&p| (1..=100_000).contains(&p))
        .ok_or("The licence does not say how many printers it covers.")? as u32;
    let issued = v["issued"].as_str().unwrap_or("").to_owned();
    Ok(Licence {
        name,
        printers,
        issued,
    })
}

/// The program checks with `verify_with`; this one is for `tools/farm-licence.rs` and the tests.
#[allow(dead_code)]
pub fn verify(text: &str) -> Result<Licence> {
    verify_with(text, &PUBLIC_KEY)
}

/// Make a licence. Used by `tools/farm-licence.rs` and the tests; the program itself only verifies.
/// `expiry` is optional: a perpetual licence has none (the normal case, see docs/FARM-LICENCE-TERMS.md).
#[allow(dead_code)]
pub fn issue(
    seed: &[u8; 32],
    name: &str,
    printers: u32,
    issued: &str,
    expiry: Option<&str>,
) -> String {
    use ed25519_dalek::{Signer, SigningKey};
    let mut payload = json!({"name":name,"printers":printers,"issued":issued});
    if let Some(e) = expiry {
        payload
            .as_object_mut()
            .unwrap()
            .insert("expiry".to_owned(), json!(e));
    }
    let payload = payload.to_string();
    let sig = SigningKey::from_bytes(seed).sign(payload.as_bytes());
    format!(
        "SFL1.{}.{}",
        b64_encode(payload.as_bytes()),
        b64_encode(&sig.to_bytes())
    )
}

#[allow(dead_code)]
pub fn public_key_of(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}

/// Where a farm of `count` printers stands, for the pages: whether a licence is wanted, whether a
/// trial is running, whether the saved licence covers the farm, and whether printers beyond the free
/// five are locked. `now` and `trial_started` are epoch seconds so tests can inject the clock.
pub fn status_with_clock(
    saved: Option<&str>,
    count: usize,
    key: &[u8; 32],
    now: f64,
    trial_started: Option<f64>,
) -> Value {
    let parsed = saved.map(|t| verify_with(t, key));
    let (licence, problem) = match parsed {
        Some(Ok(l)) => (Some(l), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let covered = licence
        .as_ref()
        .is_some_and(|l| l.printers as usize >= count);
    let free = count <= FREE_PRINTERS;
    let trial_active =
        trial_started.is_some_and(|started| now >= started && now < started + TRIAL_SECONDS);
    let trial_expired = trial_started.is_some_and(|started| now >= started + TRIAL_SECONDS);
    let (state, wanted, locked, trial, note) = if free {
        (
            "free",
            false,
            false,
            false,
            format!("Free up to {FREE_PRINTERS} printers. This farm has {count}."),
        )
    } else if covered {
        (
            "licensed",
            false,
            false,
            false,
            format!("Licensed to cover {count} printers."),
        )
    } else if licence.is_some() {
        (
            "too_few",
            true,
            false,
            false,
            format!(
                "The licence covers {} printers; this farm has {count}.",
                licence.as_ref().map_or(0, |l| l.printers)
            ),
        )
    } else if trial_active {
        let days_left = ((trial_started.unwrap_or(0.0) + TRIAL_SECONDS - now) / 86400.0).max(0.0);
        ("trial", false, false, true, format!("Trial running: {days_left:.0} day(s) left before a farm licence is required beyond {FREE_PRINTERS} printers."))
    } else if trial_expired {
        (
            "locked",
            true,
            true,
            false,
            "The trial has ended. A farm licence turns printers beyond the free five back on."
                .to_owned(),
        )
    } else {
        ("wanted", true, false, false, format!("This farm has {count} printers: a farm licence is asked for beyond {FREE_PRINTERS}. Nothing is switched off while the trial runs."))
    };
    let (trial_expires, trial_days_left) = match trial_started {
        Some(started) => (
            Some(started + TRIAL_SECONDS),
            if trial_active {
                Some(((started + TRIAL_SECONDS - now) / 86400.0).max(0.0))
            } else {
                None
            },
        ),
        None => (None, None),
    };
    json!({
        "printers":count,"free_printers":FREE_PRINTERS,"state":state,
        "wanted":wanted,"locked":locked,"trial":trial,
        "licence":licence.as_ref().map(|l| json!({"name":l.name,"printers":l.printers,"issued":l.issued})),
        "problem":problem,
        "note":note,
        "trial_started":trial_started,
        "trial_expires":trial_expires,
        "trial_days_left":trial_days_left,
    })
}

/// Where a farm of `count` printers stands, using the system clock and no trial. The pre-WO-04
/// behaviour: free to five, otherwise a licence is wanted. Tests only; the program passes its own clock.
#[cfg(test)]
pub fn status(saved: Option<&str>, count: usize, key: &[u8; 32]) -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    status_with_clock(saved, count, key, now, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [9; 32];

    #[test]
    fn base64url_round_trips_every_length() {
        for n in 0..40 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(
                b64_decode(&b64_encode(&bytes)).unwrap(),
                bytes,
                "length {n}"
            );
        }
        assert!(b64_decode("not valid!").is_err());
    }

    #[test]
    fn a_valid_licence_says_who_and_how_many() {
        let key = public_key_of(&SEED);
        let text = issue(&SEED, "Maple Street Print Farm", 12, "2026-10-12", None);
        let l = verify_with(&text, &key).unwrap();
        assert_eq!(
            (l.name.as_str(), l.printers, l.issued.as_str()),
            ("Maple Street Print Farm", 12, "2026-10-12")
        );
        // Pasted with a line break or a trailing space still checks.
        assert!(verify_with(&format!("  {}\n", text.replace('.', ".\n")), &key).is_ok());
    }

    #[test]
    fn a_changed_licence_does_not_check_out() {
        let key = public_key_of(&SEED);
        let text = issue(&SEED, "Maple Street Print Farm", 12, "2026-10-12", None);
        // More printers, the same signature.
        let forged = issue(&SEED, "Maple Street Print Farm", 120, "2026-10-12", None);
        let mixed = format!(
            "SFL1.{}.{}",
            forged.split('.').nth(1).unwrap(),
            text.split('.').nth(2).unwrap()
        );
        assert!(verify_with(&mixed, &key)
            .unwrap_err()
            .contains("does not check out"));
        // One character of the name.
        let mut t = text.clone().into_bytes();
        let i = t.len() / 3;
        t[i] = if t[i] == b'A' { b'B' } else { b'A' };
        assert!(verify_with(&String::from_utf8(t).unwrap(), &key).is_err());
        // Signed by someone else's key.
        let foreign = issue(&OTHER, "Maple Street Print Farm", 12, "2026-10-12", None);
        assert!(verify_with(&foreign, &key)
            .unwrap_err()
            .contains("does not check out"));
        // Not a licence at all, and cut short.
        assert!(verify_with("hello", &key)
            .unwrap_err()
            .contains("not a Stella Scope farm licence"));
        assert!(verify_with(&text[..text.len() - 6], &key).is_err());
        assert!(verify_with("", &key).is_err());
    }

    #[test]
    fn the_compiled_in_key_is_a_real_key_and_a_test_key_does_not_pass_it() {
        assert!(
            PUBLIC_KEY != [0; 32],
            "run farm-licence keygen and paste the public key into src/licence.rs"
        );
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&PUBLIC_KEY).is_ok());
        // A licence made with a throwaway key is not accepted by the built-in one.
        let text = issue(&SEED, "x", 6, "2026-10-12", None);
        assert!(verify(&text).unwrap_err().contains("does not check out"));
    }

    #[test]
    fn five_printers_are_free_the_sixth_is_asked_about_and_nothing_stops() {
        let key = public_key_of(&SEED);
        let s = |saved: Option<&str>, n| status(saved, n, &key);
        assert_eq!(s(None, 1)["state"], "free");
        assert_eq!(s(None, 5)["state"], "free");
        assert_eq!(s(None, 5)["wanted"], false, "no licence and no nag at five");
        assert_eq!(s(None, 6)["state"], "wanted");
        assert_eq!(s(None, 6)["wanted"], true);
        let l8 = issue(&SEED, "Shop", 8, "2026-10-12", None);
        assert_eq!(s(Some(&l8), 6)["state"], "licensed");
        assert_eq!(s(Some(&l8), 8)["state"], "licensed");
        let nine = s(Some(&l8), 9);
        assert_eq!(
            (nine["state"].clone(), nine["wanted"].clone()),
            (json!("too_few"), json!(true))
        );
        assert!(nine["note"]
            .as_str()
            .unwrap()
            .contains("covers 8 printers; this farm has 9"));
        let bad = s(Some("SFL1.AAAA.BBBB"), 7);
        assert_eq!(bad["state"], "wanted");
        assert!(bad["problem"].is_string());
        assert_eq!(
            s(Some(&l8), 3)["state"],
            "free",
            "a licence under the free count changes nothing"
        );
    }

    #[test]
    fn trial_runs_for_thirty_days_then_locks() {
        let key = public_key_of(&SEED);
        let started = 1_000_000_000.0;
        let expires = started + TRIAL_SECONDS;
        let s = |saved, n, now| status_with_clock(saved, n, &key, now, Some(started));
        // Sixth printer added: trial starts now, active for 30 days.
        assert_eq!(s(None, 6, started)["state"], "trial");
        assert_eq!(s(None, 6, started)["trial"], true);
        assert_eq!(s(None, 6, started)["locked"], false);
        let days_left = s(None, 6, started)["trial_days_left"].as_f64().unwrap();
        assert!((days_left - 30.0).abs() < 0.01, "days_left {days_left}");
        assert_eq!(
            s(None, 6, started)["trial_expires"].as_f64().unwrap(),
            expires
        );
        // One day in: still trial, 29 days left.
        assert_eq!(s(None, 6, started + 86400.0)["state"], "trial");
        let d = s(None, 6, started + 86400.0)["trial_days_left"]
            .as_f64()
            .unwrap();
        assert!((d - 29.0).abs() < 0.01, "days_left {d}");
        // On the expiry instant: locked.
        assert_eq!(s(None, 6, expires)["state"], "locked");
        assert_eq!(s(None, 6, expires)["locked"], true);
        assert_eq!(s(None, 6, expires)["trial"], false);
        // Past expiry: locked.
        assert_eq!(s(None, 6, expires + 1.0)["state"], "locked");
        // Before the trial started: wanted (trial not running yet).
        assert_eq!(s(None, 6, started - 1.0)["state"], "wanted");
        // A valid licence covers the farm during and after the trial.
        let l8 = issue(&SEED, "Shop", 8, "2026-10-12", None);
        assert_eq!(s(Some(&l8), 8, expires + 100.0)["state"], "licensed");
        assert_eq!(s(Some(&l8), 8, expires + 100.0)["locked"], false);
        // Five printers stay free even when a trial has been started and expired.
        assert_eq!(s(None, 5, expires + 100.0)["state"], "free");
        assert_eq!(s(None, 5, expires + 100.0)["locked"], false);
    }

    #[test]
    fn five_printers_never_locked_without_a_trial() {
        let key = public_key_of(&SEED);
        let now = 1_000_000_000.0;
        let s = |n| status_with_clock(None, n, &key, now, None);
        assert_eq!(s(1)["state"], "free");
        assert_eq!(s(5)["state"], "free");
        assert_eq!(s(5)["locked"], false);
        assert_eq!(s(5)["wanted"], false);
    }

    #[test]
    fn a_tampered_licence_is_refused_and_an_existing_one_still_verifies() {
        let key = public_key_of(&SEED);
        let text = issue(&SEED, "Shop", 8, "2026-10-12", None);
        let mut t = text.clone().into_bytes();
        let i = t.len() / 3;
        t[i] = if t[i] == b'A' { b'B' } else { b'A' };
        let now = 1_000_000_000.0;
        let bad = status_with_clock(Some(&String::from_utf8_lossy(&t)), 8, &key, now, None);
        assert_eq!(bad["state"], "wanted");
        assert!(bad["problem"].is_string());
        let good = status_with_clock(Some(&text), 8, &key, now, None);
        assert_eq!(good["state"], "licensed");
    }

    #[test]
    fn an_expiry_dated_licence_verifies_and_tampering_expiry_breaks_it() {
        let key = public_key_of(&SEED);
        let text = issue(&SEED, "Shop", 8, "2026-10-12", Some("2027-01-01"));
        // It verifies against the right key.
        let good = status_with_clock(Some(&text), 8, &key, 1_000_000_000.0, None);
        assert_eq!(good["state"], "licensed");
        // Tampering anywhere in the payload (including the expiry) breaks the signature.
        let mut t = text.clone().into_bytes();
        let i = t.len() / 3;
        t[i] = if t[i] == b'A' { b'B' } else { b'A' };
        let bad = status_with_clock(
            Some(&String::from_utf8_lossy(&t)),
            8,
            &key,
            1_000_000_000.0,
            None,
        );
        assert_eq!(bad["state"], "wanted");
        assert!(bad["problem"].is_string());
    }
}
