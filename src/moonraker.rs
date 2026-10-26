use crate::{config::Config, store::Event, Result};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
pub fn object_query(objects: &[String]) -> String {
    objects
        .iter()
        .map(|s| match s.split_once('=') {
            Some((name, fields)) => format!("{}={}", encode(name), encode(fields)),
            None => encode(s),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// One plain file name in Moonraker's gcodes root. Path separators and `..` are refused outright,
/// so a request can only ever name a file the printer already lists.
pub fn check_gcode_name(filename: &str) -> Result<()> {
    if filename.is_empty()
        || filename.len() > 200
        || filename.contains(['/', '\\', '?', '#'])
        || filename.contains("..")
        || filename.chars().any(char::is_control)
    {
        return Err(
            "filename must be one plain file in Moonraker's gcodes root, with no path separators; take a name from GET /api/files"
                .into(),
        );
    }
    Ok(())
}

/// WO-20 (2026-09-23): every "Moonraker did not answer at all" error starts with this, so the
/// collector can tell a printer that is off from one whose Klipper is not ready.
pub const UNREACHABLE: &str = "cannot reach Moonraker";

pub struct Moonraker {
    cfg: Config,
    agent: ureq::Agent,
    available: Vec<String>,
    missing: Vec<String>,
    next_discovery: f64,
    capture: Value,
    host: Value,
    next_host: f64,
}
impl Moonraker {
    /// The API key Moonraker wants, if any: this printer's own credentials first (farm board WO-03,
    /// `<config dir>/printers/<ref>.secret`, field `api_key`), then the process environment variable
    /// the configuration names, as it always was.
    fn api_key(&self) -> Option<String> {
        self.cfg
            .secret
            .get("api_key")
            .map(str::to_owned)
            .or_else(|| {
                std::env::var(&self.cfg.api_key_env)
                    .ok()
                    .filter(|k| !k.is_empty())
            })
    }
    pub fn new(cfg: &Config) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(cfg.request_timeout_ms)))
            .max_redirects(0)
            .proxy(None)
            .max_idle_connections(1)
            .input_buffer_size(8192)
            .output_buffer_size(8192)
            .build()
            .into();
        Self {
            cfg: cfg.clone(),
            agent,
            available: cfg.objects.clone(),
            missing: Vec::new(),
            next_discovery: 0.0,
            capture: Value::Null,
            host: Value::Null,
            next_host: 0.0,
        }
    }
    pub fn get(&self, path: &str) -> Result<Value> {
        if self.cfg.moonraker_url.trim().is_empty() {
            return Err(crate::config::NO_PRINTER.into());
        }
        let url = format!("{}{}", self.cfg.moonraker_url.trim_end_matches('/'), path);
        let mut request = self.agent.get(&url);
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let mut response=request.call().map_err(|e|format!(
            "{UNREACHABLE} at {}: {e}; is the printer on, and is this host allowed by Moonraker? For 401/403, set {}",
            self.cfg.moonraker_url,self.cfg.api_key_env))?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "Moonraker returned HTTP {}; use the direct local HTTP endpoint, not a redirect",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(self.cfg.max_response_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                format!("Moonraker response could not be read: {e}; check the connection")
            })?;
        if bytes.len() > self.cfg.max_response_bytes {
            return Err("Moonraker response exceeds max_response_bytes; reduce requested objects or increase the configured response cap".into());
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| {
            format!("Moonraker did not return valid JSON: {e}; check moonraker_url")
        })?;
        if !v["error"].is_null() {
            return Err(format!(
                "Moonraker rejected the query: {}; check object names and printer state",
                v["error"]
            ));
        }
        v.get("result").cloned().ok_or_else(||"Moonraker response has no result; check that moonraker_url points to Moonraker, not another service".into())
    }
    pub fn post(&self, path: &str, body: Option<&Value>) -> Result<Value> {
        if self.cfg.moonraker_url.trim().is_empty() {
            return Err(crate::config::NO_PRINTER.into());
        }
        let url = format!("{}{}", self.cfg.moonraker_url.trim_end_matches('/'), path);
        let mut request = self
            .agent
            .post(&url)
            .config()
            .timeout_global(Some(Duration::from_millis(self.cfg.command_timeout_ms)))
            .build();
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let encoded = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|e| format!("cannot encode fixed Moonraker command: {e}"))?;
        let response = match encoded.as_deref() {
            Some(bytes) => request.content_type("application/json").send(bytes),
            None => request.send_empty(),
        }
        .map_err(|e| format!("Moonraker command transport did not confirm an outcome: {e}"))?;
        self.command_body(response)
    }
    /// DELETE with the command timeout, for the job queue. Same failure wording as `post`:
    /// a DELETE that does not confirm leaves the outcome unknown, and nothing is retried.
    pub fn delete(&self, path: &str) -> Result<Value> {
        if self.cfg.moonraker_url.trim().is_empty() {
            return Err(crate::config::NO_PRINTER.into());
        }
        let url = format!("{}{}", self.cfg.moonraker_url.trim_end_matches('/'), path);
        let mut request = self
            .agent
            .delete(&url)
            .config()
            .timeout_global(Some(Duration::from_millis(self.cfg.command_timeout_ms)))
            .build();
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let response = request
            .call()
            .map_err(|e| format!("Moonraker command transport did not confirm an outcome: {e}"))?;
        self.command_body(response)
    }

    /// Shared tail of `post`/`delete`: bound the body, refuse a non-2xx, and unwrap `result`.
    fn command_body(&self, mut response: ureq::http::Response<ureq::Body>) -> Result<Value> {
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "Moonraker command returned HTTP {}; outcome is unknown",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(self.cfg.max_response_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                format!("Moonraker command response was interrupted: {e}; outcome is unknown")
            })?;
        if bytes.len() > self.cfg.max_response_bytes {
            return Err(
                "Moonraker command response exceeds max_response_bytes; outcome is unknown".into(),
            );
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
            format!("Moonraker command response was not valid JSON: {e}; outcome is unknown")
        })?;
        if !value["error"].is_null() {
            return Err(format!(
                "Moonraker command returned an error after submission: {}; outcome is unknown",
                value["error"]
            ));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "Moonraker command response has no result; outcome is unknown".into())
    }

    /// Download a config file as text: `GET /server/files/<root>/<path>` returns the file itself,
    /// not JSON, so this cannot go through `get`. Bounded by `max_config_bytes`.
    pub fn download_config(&self, path: &str, max_bytes: usize) -> Result<String> {
        if path.contains(['?', '#', '\r', '\n']) || path.contains("..") || path.len() > 200 {
            return Err("config path has unexpected characters; refusing the download".into());
        }
        let url = format!(
            "{}/server/files/config/{}",
            self.cfg.moonraker_url.trim_end_matches('/'),
            encode(path)
        );
        let mut request = self.agent.get(&url);
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let mut response = request
            .call()
            .map_err(|e| format!("cannot download {path} from Moonraker: {e}"))?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "downloading {path} returned HTTP {}; check that the file exists in the config root",
                response.status()
            ));
        }
        let mut buf = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("{path} could not be read: {e}"))?;
        if buf.len() > max_bytes {
            return Err(format!(
                "{path} is larger than the {max_bytes}-byte edit cap; edit it on the Pi instead"
            ));
        }
        String::from_utf8(buf).map_err(|_| format!("{path} is not UTF-8; refusing to edit it"))
    }

    /// Upload one config file through `POST /server/files/upload`, multipart/form-data with
    /// `root=config` and `path=`. The boundary is fixed and asserted not to occur in the body,
    /// so the encoding is exact rather than probabilistic.
    pub fn upload_config(&self, path: &str, content: &str) -> Result<Value> {
        if path.contains(['?', '#', '\r', '\n', '"']) || path.contains("..") || path.len() > 200 {
            return Err("config path has unexpected characters; refusing the upload".into());
        }
        const BOUNDARY: &str = "StellaScopeConfigBoundary7f3a1c";
        if content.contains(BOUNDARY) {
            return Err(
                "config file contains the upload boundary marker; refusing to upload it".into(),
            );
        }
        let mut body = String::new();
        for (name, value) in [("root", "config"), ("path", path)] {
            body.push_str(&format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            ));
        }
        body.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n{content}\r\n--{BOUNDARY}--\r\n",
            path.rsplit('/').next().unwrap_or(path)
        ));
        let url = format!(
            "{}/server/files/upload",
            self.cfg.moonraker_url.trim_end_matches('/')
        );
        let mut request = self
            .agent
            .post(&url)
            .config()
            .timeout_global(Some(Duration::from_millis(self.cfg.command_timeout_ms)))
            .build();
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let response = request
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .send(body.as_bytes())
            .map_err(|e| format!("config upload did not confirm an outcome: {e}"))?;
        self.command_body(response)
    }

    /// Upload a G-code file to the printer's `gcodes` root (`POST /server/files/upload`, multipart,
    /// `print=false`): the queue (WO-08) uploads, then starts through the command route so its checks
    /// apply. The name is a plain file name; the boundary is fixed and refused if it occurs in the body.
    pub fn upload_gcode(&self, name: &str, bytes: &[u8]) -> Result<Value> {
        if name.is_empty()
            || name.contains(['/', '\\', '?', '#', '\r', '\n', '"'])
            || name.contains("..")
            || name.len() > 200
        {
            return Err("the file name has characters a printer upload does not take".into());
        }
        const BOUNDARY: &str = "StellaScopeQueueBoundary5b9e04";
        if bytes
            .windows(BOUNDARY.len())
            .any(|w| w == BOUNDARY.as_bytes())
        {
            return Err("the file contains the upload boundary marker; refusing to send it".into());
        }
        let mut body: Vec<u8> = Vec::with_capacity(bytes.len() + 512);
        for (field, value) in [("root", "gcodes"), ("print", "false")] {
            body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let url = format!(
            "{}/server/files/upload",
            self.cfg.moonraker_url.trim_end_matches('/')
        );
        // A big file takes longer than a command: a minute plus a second per MB.
        let ms = self
            .cfg
            .command_timeout_ms
            .max(60_000 + (bytes.len() as u64 / 1_000_000) * 1_000);
        let mut request = self
            .agent
            .post(&url)
            .config()
            .timeout_global(Some(Duration::from_millis(ms)))
            .build();
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let response = request
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .send(&body[..])
            .map_err(|e| format!("the upload did not confirm an outcome: {e}"))?;
        // Moonraker answers an upload with {"item":{...},"action":"create_file"}, not a "result".
        let mut response = response;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("the upload reply was interrupted: {e}"))?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let why = value["error"]["message"]
                .as_str()
                .or(value["error"].as_str())
                .unwrap_or("no reason given");
            return Err(format!(
                "the printer refused the file (HTTP {status}): {why}"
            ));
        }
        if value["item"].is_null() {
            return Err("the printer answered the upload without confirming a file".into());
        }
        Ok(value)
    }

    // Live object list for per-sample queries: configfile never rides the fast path. It is
    // fetched once per connect/restart/SAVE_CONFIG (see fetch_config) instead of every sample.
    fn live_objects(&self) -> Vec<String> {
        self.available
            .iter()
            .filter(|s| !s.split('=').next().unwrap_or(s).eq("configfile"))
            .cloned()
            .collect()
    }
    pub fn discover(&mut self) {
        self.missing.clear();
        self.capture = Value::Null;
        match self.get("/printer/objects/list") {
            Ok(v) => {
                if let Some(names) = v["objects"].as_array() {
                    let (selected, notes, manifest) =
                        crate::telemetry::select(&self.cfg.objects, names, &self.cfg.telemetry);
                    self.available = selected;
                    self.missing = notes;
                    self.capture = manifest;
                } else {
                    self.available = self.cfg.objects.clone();
                    self.missing.push(
                        "Object discovery returned no list; requesting configured objects directly"
                            .into(),
                    );
                }
            }
            Err(e) => {
                self.available = self.cfg.objects.clone();
                self.missing.push(format!(
                    "Object discovery failed; requesting configured objects directly. {e}"
                ));
            }
        }
    }
    pub fn manifest(&self) -> Value {
        self.capture.clone()
    }
    // Opt-in host statistics on their own cadence; shared by the poll path and the push path.
    pub fn host_tick(&mut self, ts: f64) -> Value {
        if ts >= self.next_host {
            self.next_host = ts + self.cfg.telemetry.host_interval_ms as f64 / 1000.0;
            self.host = match self.get("/machine/proc_stats") {
                Ok(v) => crate::telemetry::host_summary(&v, ts),
                Err(e) => {
                    serde_json::json!({"captured_at":ts,"error":e,"source":"GET /machine/proc_stats"})
                }
            };
        }
        self.host.clone()
    }
    pub fn fetch_config(&mut self) -> Result<Value> {
        let v = self.get("/printer/objects/query?configfile=settings")?;
        let settings = v["status"]["configfile"]["settings"].clone();
        if !settings.is_object() {
            return Err(
                "configfile settings came back empty; the printer may be starting up".into(),
            );
        }
        Ok(settings)
    }
    pub fn sample(&mut self, ts: f64) -> Result<(Value, Vec<String>)> {
        if ts >= self.next_discovery {
            self.next_discovery = ts + self.cfg.discovery_interval_s as f64;
            self.discover();
        }
        let live = self.live_objects();
        if live.is_empty() {
            return Err(
                "none of the configured objects exist; check objects in the JSON configuration"
                    .into(),
            );
        }
        let path = format!("/printer/objects/query?{}", object_query(&live));
        let response = match self.get(&path) {
            Ok(v) => v,
            Err(e) => {
                self.next_discovery = 0.0;
                return Err(e);
            }
        };
        let status=response["status"].as_object().filter(|m|!m.is_empty())
            .ok_or("printer not answering with status data; check Klipper's ready state in Mainsail or Fluidd")?;
        let mut warnings = self.missing.clone();
        for spec in &live {
            let name = spec.split('=').next().unwrap_or(spec);
            if !status.contains_key(name) {
                warnings.push(format!(
                    "No current data for {name}; this panel will show unknown"
                ));
            }
        }
        let mut data = status.clone();
        data.insert("_scope_capture".into(), self.capture.clone());
        if self.cfg.telemetry.host_stats {
            let host = self.host_tick(ts);
            if let Some(e) = host["error"].as_str() {
                warnings.push(format!("Host statistics unavailable: {e}"));
            }
            data.insert("_scope_host".into(), host);
        }
        Ok((Value::Object(data), warnings))
    }
    // Print context (board item 6), both read-only GETs:
    //  - /server/files/metadata: Moonraker's already-parsed slicer metadata for the file.
    //  - a one-shot ranged GET of the last ~64 KB of the gcode to read Orca's settings block
    //    (Orca writes its tuned values as "; key = value" lines near the end of the file).
    pub fn file_metadata(&self, filename: &str) -> Result<Value> {
        if filename.contains(['?', '#', '\r', '\n']) || filename.len() > 200 {
            return Err(
                "print filename has unexpected characters; refusing the metadata request".into(),
            );
        }
        let v = self.get(&format!(
            "/server/files/metadata?filename={}",
            encode(filename)
        ))?;
        Ok(json!({
            "size": v["size"], "modified": v["modified"], "slicer": v["slicer"],
            "layer_height": v["layer_height"], "first_layer_height": v["first_layer_height"],
            "object_height": v["object_height"], "estimated_time": v["estimated_time"],
            "filament_name": v["filament_name"], "filament_type": v["filament_type"],
            "filament_total": v["filament_total"], "filament_weight_total": v["filament_weight_total"],
            "slicer_version": v["slicer_version"], "layer_count": v["layer_count"], "filament_colors": v["filament_colors"],
            "first_layer_extr_temp": v["first_layer_extr_temp"],
            "first_layer_bed_temp": v["first_layer_bed_temp"],
            "nozzle_diameter": v["nozzle_diameter"],
            "thumbnails": v["thumbnails"],
            "source": "GET /server/files/metadata (slicer-parsed)"
        }))
    }
    pub fn gcode_tail(&self, filename: &str, bytes: usize) -> Result<String> {
        if filename.contains(['?', '#', '\r', '\n'])
            || filename.contains("..")
            || filename.len() > 200
        {
            return Err(
                "print filename has unexpected characters; refusing the range request".into(),
            );
        }
        let url = format!(
            "{}/server/files/gcodes/{}",
            self.cfg.moonraker_url.trim_end_matches('/'),
            encode(filename)
        );
        let size = self.file_metadata(filename)?["size"]
            .as_f64()
            .ok_or("file size unknown; cannot build the range request")? as i64;
        let start = (size - bytes as i64).max(0);
        let mut request = self.agent.get(&url);
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let mut response = request
            .header("Range", &format!("bytes={start}-"))
            .call()
            .map_err(|e| format!("cannot read the gcode tail: {e}"))?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "gcode tail request returned HTTP {}",
                response.status()
            ));
        }
        let mut buf = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(bytes as u64)
            .read_to_end(&mut buf)
            .map_err(|e| format!("gcode tail could not be read: {e}"))?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// A whole gcode file from Moonraker's gcodes root, for the HUD's toolpath view. Read-only,
    /// one GET, capped at `max_bytes`. The name must be a plain file in the root: no path
    /// separators, so a caller cannot walk out of it.
    pub fn gcode_file(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        check_gcode_name(filename)?;
        let url = format!(
            "{}/server/files/gcodes/{}",
            self.cfg.moonraker_url.trim_end_matches('/'),
            encode(filename)
        );
        let mut request = self
            .agent
            .get(&url)
            .config()
            .timeout_global(Some(Duration::from_millis(self.cfg.command_timeout_ms)))
            .build();
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let mut response = request.call().map_err(|e| {
            format!("cannot read {filename} from Moonraker: {e}; is the printer on?")
        })?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "Moonraker returned HTTP {} for {filename}; check the name against GET /api/files",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{filename} could not be read in full: {e}"))?;
        if bytes.len() > max_bytes {
            return Err(format!(
                "{filename} is larger than the {max_bytes} byte gcode limit; slice a smaller file"
            ));
        }
        Ok(bytes)
    }

    /// WO-13: the largest slicer thumbnail for one gcode file, PNG bytes straight from Moonraker.
    /// Thumbnails live in the same gcodes directory as the file, and the file names we accept are
    /// already single plain names (check_gcode_name refuses separators), so the thumbnail's own
    /// relative path is accepted only as one plain `.png` name — anything with a separator or a
    /// `..` is refused rather than resolved. The PNG magic is verified before the bytes are
    /// returned, so a swapped or truncated file is never handed to the HUD as an image.
    pub fn file_thumbnail(&self, filename: &str, max_bytes: usize) -> Result<Vec<u8>> {
        check_gcode_name(filename)?;
        let meta = self.get(&format!(
            "/server/files/metadata?filename={}",
            encode(filename)
        ))?;
        let best = meta["thumbnails"]
            .as_array()
            .and_then(|list| {
                list.iter()
                    .filter(|t| {
                        t["width"].as_i64().is_some_and(|w| w > 0)
                            && t["height"].as_i64().is_some_and(|h| h > 0)
                            && t["relative_path"].is_string()
                    })
                    .max_by_key(|t| {
                        t["width"].as_i64().unwrap_or(0) * t["height"].as_i64().unwrap_or(0)
                    })
            })
            .ok_or("that file has no thumbnail; slice it with thumbnail output enabled")?;
        let thumb = best["relative_path"]
            .as_str()
            .ok_or("that file has no thumbnail; slice it with thumbnail output enabled")?;
        // Moonraker puts slicer thumbnails in a `.thumbs/` directory beside the file (real
        // metadata for the 2026-09-22 cube: `.thumbs/Cube_...-300x300.png`), so subdirectory
        // components are expected. What is never acceptable is traversal: no `..`, no absolute
        // path, no separators that are not `/`, and it must end in .png.
        if thumb.len() > 250
            || thumb.contains("..")
            || thumb.starts_with('/')
            || thumb.contains(['\\', '?', '#', '\r', '\n', '\0'])
            || !thumb.to_ascii_lowercase().ends_with(".png")
        {
            return Err("thumbnail name is not a plain .png in the gcodes root".into());
        }
        let encoded_path = thumb.split('/').map(encode).collect::<Vec<_>>().join("/");
        let url = format!(
            "{}/server/files/gcodes/{}",
            self.cfg.moonraker_url.trim_end_matches('/'),
            encoded_path
        );
        let mut request = self.agent.get(&url);
        if let Some(key) = self.api_key() {
            request = request.header("X-Api-Key", &key);
        }
        let mut response = request.call().map_err(|e| {
            format!(
                "cannot read the thumbnail for {filename} from Moonraker: {e}; is the printer on?"
            )
        })?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(format!(
                "Moonraker returned HTTP {} for the thumbnail of {filename}",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("the thumbnail for {filename} could not be read: {e}"))?;
        if bytes.len() > max_bytes {
            return Err(format!(
                "the thumbnail for {filename} is larger than the {max_bytes} byte proxy limit"
            ));
        }
        if bytes.len() < 8 || bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
            return Err("the thumbnail file is not a PNG; refusing to serve it as one".into());
        }
        Ok(bytes)
    }

    /// Full print context for one file: slicer metadata + Orca's tuned settings from the tail.
    /// Two read-only GETs, done once per observed print start.
    pub fn print_context(&self, filename: &str) -> Result<(Value, Value)> {
        let metadata = self.file_metadata(filename)?;
        let tail = self.gcode_tail(filename, 65536).unwrap_or_default();
        let mut orca = serde_json::Map::new();
        for line in tail.lines().rev() {
            let Some(rest) = line.trim().strip_prefix(';') else {
                continue;
            };
            let Some((k, v)) = rest.split_once('=') else {
                continue;
            };
            let key = k.trim().to_lowercase();
            if matches!(
                key.as_str(),
                "filament_flow_ratio"
                    | "pressure_advance"
                    | "nozzle_temperature"
                    | "bed_temperature"
                    | "precise_outer_wall"
                    | "outer_wall_acceleration"
                    | "default_acceleration"
                    | "outer_wall_speed"
                    | "inner_wall_speed"
                    | "sparse_infill_speed"
            ) {
                orca.insert(key, json!(v.trim()));
            }
        }
        Ok((metadata, Value::Object(orca)))
    }

    pub fn console(&self) -> Result<(Vec<Event>, bool)> {
        let result = self.get("/server/gcode_store?count=100")?;
        let list = result["gcode_store"]
            .as_array()
            .ok_or("Moonraker returned no console history; check its gcode_store API")?;
        let mut events = Vec::with_capacity(list.len());
        for item in list {
            let ts = item["time"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("console contains an invalid timestamp; check Moonraker's clock")?;
            let message = item["message"]
                .as_str()
                .ok_or("console message is not text; inspect Moonraker's response")?;
            let kind = item["type"]
                .as_str()
                .ok_or("console message has no type; inspect Moonraker's response")?;
            events.push(Event::new(ts, kind, message));
        }
        events.sort_by(|a, b| a.ts.total_cmp(&b.ts));
        Ok((events, list.len() >= 100))
    }
}
