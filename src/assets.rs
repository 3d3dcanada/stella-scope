use crate::api::Response;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SCHEMA: &str = "stella-scope.assets.v1";
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_GEOMETRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn library(database: &str) -> PathBuf {
    PathBuf::from(format!("{database}.assets"))
}

fn manifest(database: &str) -> Result<Value, String> {
    let root = library(database);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("managed asset library must be a directory, not a symlink or file".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"schema":SCHEMA,"assets":[]}));
        }
        Err(error) => return Err(format!("cannot inspect managed asset library: {error}")),
        Ok(_) => {}
    }
    let path = root.join("manifest.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"schema":SCHEMA,"assets":[]}));
        }
        Err(error) => return Err(format!("cannot inspect managed asset manifest: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(
            "managed asset manifest must be a regular file, not a symlink or directory".into(),
        );
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("managed asset manifest exceeds the 8 MiB read limit".into());
    }
    let value: Value = serde_json::from_slice(
        &fs::read(&path).map_err(|e| format!("cannot read managed asset manifest: {e}"))?,
    )
    .map_err(|e| format!("managed asset manifest is invalid JSON: {e}"))?;
    if value.get("schema").and_then(Value::as_str) != Some(SCHEMA)
        || !value.get("assets").is_some_and(Value::is_array)
    {
        return Err("managed asset manifest has an unsupported schema".into());
    }
    Ok(value)
}

fn contained_file(root: &Path, id: &str, filename: &str, max_bytes: u64) -> Option<PathBuf> {
    let root_metadata = fs::symlink_metadata(root).ok()?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return None;
    }
    let root = fs::canonicalize(root).ok()?;
    let directory = root.join(id);
    let directory_metadata = fs::symlink_metadata(&directory).ok()?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return None;
    }
    let directory = fs::canonicalize(directory).ok()?;
    if !directory.starts_with(&root) {
        return None;
    }
    let file = directory.join(filename);
    let file_metadata = fs::symlink_metadata(&file).ok()?;
    if file_metadata.file_type().is_symlink()
        || !file_metadata.is_file()
        || file_metadata.len() > max_bytes
    {
        return None;
    }
    let file = fs::canonicalize(file).ok()?;
    (file.starts_with(&directory) && file.starts_with(&root)).then_some(file)
}

fn asset<'a>(manifest: &'a Value, id: &str) -> Option<&'a Value> {
    manifest["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset.get("id").and_then(Value::as_str) == Some(id))
}

fn response_error(code: u16, message: impl Into<String>) -> Response {
    Response::json(code, json!({"error":message.into()}))
}

pub fn handle(method: &str, path: &str, database: &str) -> Response {
    if method != "GET" {
        return response_error(405, "read-only service; only GET is allowed");
    }
    let mut data = match manifest(database) {
        Ok(value) => value,
        Err(error) => return response_error(500, error),
    };
    let root = library(database);
    if path == "/api/assets" {
        let Some(assets) = data.get_mut("assets").and_then(Value::as_array_mut) else {
            return response_error(500, "managed asset manifest has an invalid assets list");
        };
        for entry in assets {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                return response_error(
                    500,
                    "managed asset manifest contains an asset without an id",
                );
            };
            if !valid_id(id) {
                return response_error(500, "managed asset manifest contains an invalid asset id");
            }
            let Some(format) = entry.get("format").and_then(Value::as_str) else {
                return response_error(
                    500,
                    "managed asset manifest contains an asset without a format",
                );
            };
            if !["stl", "step"].contains(&format) {
                return response_error(
                    500,
                    "managed asset manifest contains an unsupported format",
                );
            }
            entry["availability"] = json!({
                "geometry": if contained_file(&root, id, "geometry.json", MAX_GEOMETRY_BYTES).is_some() { "available" } else { "missing" },
                "original": if contained_file(&root, id, &format!("original.{format}"), MAX_SOURCE_BYTES).is_some() { "available" } else { "missing" },
            });
        }
        return Response::json(200, data);
    }

    let Some(rest) = path.strip_prefix("/api/assets/") else {
        return response_error(404, "asset route not found");
    };
    let Some((id, item)) = rest.split_once('/') else {
        return response_error(404, "asset route must end in geometry or original");
    };
    if !valid_id(id) || rest.matches('/').count() != 1 {
        return response_error(
            400,
            "asset id must match [a-z0-9_-]{1,64} and may not contain path separators",
        );
    }
    let Some(entry) = asset(&data, id) else {
        return response_error(404, format!("managed asset {id} was not found"));
    };
    let filename = match item {
        "geometry" => "geometry.json".to_owned(),
        "original" => {
            let Some(format) = entry.get("format").and_then(Value::as_str) else {
                return response_error(500, "managed asset manifest entry has no format");
            };
            if !["stl", "step"].contains(&format) {
                return response_error(
                    500,
                    "managed asset manifest entry has an unsupported format",
                );
            }
            format!("original.{format}")
        }
        _ => return response_error(404, "asset route must end in geometry or original"),
    };
    let max_bytes = if item == "geometry" {
        MAX_GEOMETRY_BYTES
    } else {
        MAX_SOURCE_BYTES
    };
    let Some(file) = contained_file(&root, id, &filename, max_bytes) else {
        return response_error(
            404,
            format!("managed {item} file for {id} is missing or unsafe"),
        );
    };
    match fs::read(file) {
        Ok(body) => Response {
            code: 200,
            mime: if item == "geometry" {
                "application/json; charset=utf-8"
            } else {
                "application/octet-stream"
            },
            body,
        },
        Err(error) => response_error(
            500,
            format!("cannot read managed {item} file for {id}: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!(
            "stella-assets-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("test.sqlite3").to_string_lossy().into_owned();
        (root, database)
    }

    fn body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn empty_manifest_and_method_boundary() {
        let (root, database) = fixture();
        let response = handle("GET", "/api/assets", &database);
        assert_eq!(response.code, 200);
        assert_eq!(body(&response), json!({"schema":SCHEMA,"assets":[]}));
        assert_eq!(handle("POST", "/api/assets", &database).code, 405);
        for path in [
            "/service/assembly",
            "/assembly.js",
            "/assembly.css",
            "/vendor/three.module.js",
            "/vendor/three.core.js",
            "/vendor/OrbitControls.js",
        ] {
            let response = crate::api::asset("GET", path).unwrap();
            assert_eq!(response.code, 200, "{path}");
            assert!(!response.body.is_empty(), "{path}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serves_exact_managed_files_and_reports_missing_state() {
        let (root, database) = fixture();
        let library = library(&database);
        fs::create_dir_all(library.join("part-v1")).unwrap();
        fs::write(
            library.join("part-v1/geometry.json"),
            br#"{"root":{},"meshes":[]}"#,
        )
        .unwrap();
        fs::write(library.join("part-v1/original.stl"), b"stl").unwrap();
        fs::write(
            library.join("manifest.json"),
            serde_json::to_vec(&json!({"schema":SCHEMA,"assets":[
                {"id":"part-v1","format":"stl"},
                {"id":"missing-v1","format":"step"}
            ]}))
            .unwrap(),
        )
        .unwrap();

        let listing = handle("GET", "/api/assets", &database);
        assert_eq!(listing.code, 200);
        let listing = body(&listing);
        assert_eq!(
            listing["assets"][0]["availability"],
            json!({"geometry":"available","original":"available"})
        );
        assert_eq!(
            listing["assets"][1]["availability"],
            json!({"geometry":"missing","original":"missing"})
        );
        let geometry = handle("GET", "/api/assets/part-v1/geometry", &database);
        assert_eq!(geometry.code, 200);
        assert_eq!(geometry.mime, "application/json; charset=utf-8");
        let original = handle("GET", "/api/assets/part-v1/original", &database);
        assert_eq!(original.code, 200);
        assert_eq!(original.mime, "application/octet-stream");
        assert_eq!(original.body, b"stl");
        assert_eq!(
            handle("GET", "/api/assets/missing-v1/geometry", &database).code,
            404
        );
        assert_eq!(
            handle("GET", "/api/assets/missing-v1/original", &database).code,
            404
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_traversal_and_symlinks() {
        let (root, database) = fixture();
        let library = library(&database);
        fs::create_dir_all(library.join("linked-v1")).unwrap();
        fs::write(
            library.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schema":SCHEMA,"assets":[{"id":"linked-v1","format":"stl"}]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            handle("GET", "/api/assets/../geometry", &database).code,
            400
        );
        assert_eq!(
            handle("GET", "/api/assets/%2e%2e/geometry", &database).code,
            400
        );
        assert_eq!(
            handle(
                "GET",
                "/api/assets/linked-v1/../../manifest.json",
                &database
            )
            .code,
            400
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", library.join("linked-v1/original.stl"))
                .unwrap();
            assert_eq!(
                handle("GET", "/api/assets/linked-v1/original", &database).code,
                404
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_library() {
        let (root, database) = fixture();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("manifest.json"),
            serde_json::to_vec(&json!({"schema":SCHEMA,"assets":[]})).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, library(&database)).unwrap();
        assert_eq!(handle("GET", "/api/assets", &database).code, 500);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_oversized_manifest() {
        let (root, database) = fixture();
        let library = library(&database);
        fs::create_dir_all(&library).unwrap();
        let manifest = fs::File::create(library.join("manifest.json")).unwrap();
        manifest.set_len(MAX_MANIFEST_BYTES + 1).unwrap();
        assert_eq!(handle("GET", "/api/assets", &database).code, 500);
        fs::remove_dir_all(root).unwrap();
    }
}
