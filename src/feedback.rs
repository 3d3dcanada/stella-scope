//! Feedback to 3D3D from inside the app (2026-10-27: "We're supposed to have feedback in all of our apps anyway, so people can give us
//! feedback"). Only when the person presses Send: what they typed, the app's version and system, and, if they leave the box ticked, how many
//! printers of which kind. No recording, no file name, no address of anything. It goes to 3d3d.ca's intake, where every enquiry lands, as
//! one multipart field the way the site's own forms send it, and the reply is the reference number the site gives.
use serde_json::{json, Value};
use std::time::Duration;

const FEEDBACK_URL: &str = "https://3d3d.ca/api/intake";
const MESSAGE_MIN: usize = 10;
const MESSAGE_MAX: usize = 3000;

/// The intake address, or `STELLA_FEEDBACK_URL` (tests point it at a local server).
fn url() -> String {
    std::env::var("STELLA_FEEDBACK_URL").unwrap_or_else(|_| FEEDBACK_URL.to_owned())
}

/// What is sent, built from the page's request: checked here, so nothing malformed ever leaves the computer.
/// `printers` is the farm's printers as (driver) names; it is only sent when `include_setup` is true.
pub fn payload(v: &Value, printers: &[String]) -> Result<Value, String> {
    let text = |k: &str, max: usize| -> Result<String, String> {
        let s = v[k].as_str().unwrap_or("").trim().to_owned();
        if s.chars().count() > max {
            return Err(format!("{k} is longer than {max} characters"));
        }
        Ok(s)
    };
    let message = text("message", MESSAGE_MAX)?;
    if message.chars().count() < MESSAGE_MIN {
        return Err(format!("write at least {MESSAGE_MIN} characters"));
    }
    let name = text("name", 80)?;
    let email = text("email", 254)?;
    if !email.is_empty() && !valid_email(&email) {
        return Err("that email address does not look right".into());
    }
    let mut answers = json!({
        "app": "stella-scope",
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "message": message,
        "name": name,
        "email": email,
    });
    if v["include_setup"].as_bool().unwrap_or(true) {
        answers["setup"] = json!(setup_line(printers));
    }
    Ok(json!({"journey":"app-feedback","journeyName":"Stella Scope feedback","answers":answers}))
}

/// "4 printers: 2 moonraker, 1 prusalink, 1 flashforge" (demo printers are left out: they say nothing about the person's farm).
pub fn setup_line(printers: &[String]) -> String {
    let real: Vec<&String> = printers.iter().filter(|d| d.as_str() != "demo").collect();
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for d in &real {
        match kinds.iter_mut().find(|(k, _)| k == *d) {
            Some((_, n)) => *n += 1,
            None => kinds.push(((*d).clone(), 1)),
        }
    }
    let list = kinds
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    match real.len() {
        0 => "no printers yet".into(),
        1 => format!("1 printer: {list}"),
        n => format!("{n} printers: {list}"),
    }
}

fn valid_email(s: &str) -> bool {
    let Some((user, host)) = s.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !s.chars().any(char::is_whitespace)
}

/// The body the site's intake reads: multipart with one field, `payload`, holding the JSON.
pub fn multipart(payload: &Value) -> (String, Vec<u8>) {
    let boundary = format!("stella{:016x}", rand_u64());
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"payload\"\r\n\r\n{payload}\r\n--{boundary}--\r\n"
    );
    (
        format!("multipart/form-data; boundary={boundary}"),
        body.into_bytes(),
    )
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

/// Send it. The reply is the site's reference (3D3D-XXXXXX), or why it was not taken, in words.
pub fn send(payload: &Value) -> Result<String, String> {
    let (content_type, body) = multipart(payload);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut r = agent
        .post(&url())
        .header("Content-Type", &content_type)
        .header(
            "User-Agent",
            concat!("StellaScope/", env!("CARGO_PKG_VERSION")),
        )
        .send(&body[..])
        .map_err(|e| {
            format!(
                "3d3d.ca could not be reached ({e}). Check the internet connection and try again."
            )
        })?;
    let status = r.status().as_u16();
    let reply: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap_or_default())
        .unwrap_or(Value::Null);
    match (status, reply["ref"].as_str()) {
        (200, Some(id)) if id.starts_with("3D3D-") && id.len() <= 16 => Ok(id.to_owned()),
        (429, _) => {
            Err("too many messages from this connection in the last hour; try again later".into())
        }
        _ => Err(format!(
            "3d3d.ca did not take it (HTTP {status}{}). Write to info@3d3d.ca instead.",
            reply["error"]
                .as_str()
                .map(|e| format!(", {e}"))
                .unwrap_or_default()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn what_is_sent_is_what_the_person_typed_and_the_app_and_nothing_else() {
        let p = payload(&json!({"message":"  The heat wheel is great  ","email":"a@b.ca","name":"","include_setup":true}), &["moonraker".into(), "demo".into(), "prusalink".into(), "moonraker".into()]).unwrap();
        assert_eq!(p["journey"], "app-feedback");
        let a = &p["answers"];
        assert_eq!(a["message"], "The heat wheel is great");
        assert_eq!(a["app"], "stella-scope");
        assert_eq!(a["setup"], "3 printers: 2 moonraker, 1 prusalink");
        let keys: Vec<&String> = a.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 7, "{keys:?}");
        let quiet = payload(
            &json!({"message":"No setup line please","include_setup":false}),
            &["moonraker".into()],
        )
        .unwrap();
        assert!(quiet["answers"].get("setup").is_none());
    }

    #[test]
    fn a_short_long_or_malformed_message_never_leaves_the_computer() {
        assert!(payload(&json!({"message":"short"}), &[]).is_err());
        assert!(payload(&json!({"message":"x".repeat(3001)}), &[]).is_err());
        assert!(payload(
            &json!({"message":"long enough here","email":"not an email"}),
            &[]
        )
        .is_err());
        assert!(payload(
            &json!({"message":"long enough here","name":"n".repeat(81)}),
            &[]
        )
        .is_err());
        assert_eq!(setup_line(&["demo".into()]), "no printers yet");
    }

    /// The whole trip against a stand-in intake: one multipart field named payload, the reference read back.
    #[test]
    fn it_posts_one_payload_field_and_reads_back_the_reference() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let seen = std::thread::spawn(move || {
            let (mut s, _) = server.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if String::from_utf8_lossy(&buf).contains("--\r\n") {
                    break;
                }
            }
            let reply = "{\"ref\":\"3D3D-ABC123\"}";
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        // SAFETY: tests that read STELLA_FEEDBACK_URL run only here.
        unsafe { std::env::set_var("STELLA_FEEDBACK_URL", format!("http://{addr}/api/intake")) };
        let p = payload(&json!({"message":"The farm wall is clear"}), &[]).unwrap();
        assert_eq!(send(&p).unwrap(), "3D3D-ABC123");
        let req = seen.join().unwrap();
        assert!(req.starts_with("POST /api/intake "), "{req}");
        assert!(req.contains("multipart/form-data; boundary=stella"));
        assert!(req.contains("name=\"payload\"\r\n\r\n{\"answers\""));
        assert!(req.contains("The farm wall is clear"));
    }
}
