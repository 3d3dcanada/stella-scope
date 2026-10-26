//! Stella for phones (2026-09-25). The app keeps a list of printers (each a Stella Scope link
//! id, scanned from the desktop's QR code). Opening one starts a stella-link tunnel on a loopback
//! port fixed for that printer and loads Stella Scope through it; pairing and every control check
//! happen on the desktop exactly as on the home network.
use std::{collections::HashMap, sync::Mutex};
use tauri::{Manager, State};

#[derive(Default)]
struct Open(Mutex<HashMap<String, Option<u16>>>);

fn reserve(open: &mut HashMap<String, Option<u16>>, id: &str) -> Result<Option<u16>, String> {
    match open.get(id) {
        Some(Some(port)) => Ok(Some(*port)),
        Some(None) => Err("this printer is already connecting".into()),
        None => { open.insert(id.to_owned(), None); Ok(None) }
    }
}

#[tauri::command]
async fn open_link(app: tauri::AppHandle, id: String, open: State<'_, Open>) -> Result<u16, String> {
    let id = id.trim().trim_start_matches("stella://link/").to_owned();
    let remote = stella_link::parse_id(&id).map_err(|e| e.to_string())?;
    { let mut links = open.0.lock().map_err(|e| e.to_string())?;if let Some(port) = reserve(&mut links, &id)? { return Ok(port); } }
    let result = async {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let key = stella_link::load_or_create_key(&dir.join("link.key")).map_err(|e| e.to_string())?;
        let t = stella_link::Tunnel::start_on(key, remote, stella_link::port_for(&remote)).await.map_err(|e| e.to_string())?;
        Ok::<u16, String>(t.port)
    }.await;
    let mut links = open.0.lock().map_err(|e| e.to_string())?;
    match result { Ok(port) => { links.insert(id, Some(port)); Ok(port) }, Err(error) => { links.remove(&id); Err(error) } }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_printer_starts_one_tunnel() {
        let mut open = std::collections::HashMap::new();
        assert_eq!(super::reserve(&mut open, "printer"), Ok(None));
        assert!(super::reserve(&mut open, "printer").is_err());
        open.insert("printer".to_owned(), Some(41000));
        assert_eq!(super::reserve(&mut open, "printer"), Ok(Some(41000)));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .manage(Open::default())
        .invoke_handler(tauri::generate_handler![open_link]);
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init()).plugin(tauri_plugin_haptics::init());
    builder
        .run(tauri::generate_context!())
        .expect("Stella could not start");
}
