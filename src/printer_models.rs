//! Real CAD library for printers (farm board WO-10).
//!
//! Where the models come from: the 67 machines of 3D Print Rig (github.com/Psych0h3ad/3d-print-rig),
//! each a gzipped GLB of the author's real CAD with a SHA-256 the project publishes. The catalogue
//! (`web/printer-models.json`, built by `tools/printer-models-index.rs`) maps every machine to its file,
//! its pins and its licence class. Nothing here makes up a model: a printer with no real CAD has none.
//!
//! What a build may carry: the licence class decides. `gpl`, `mit`, `cc-by` and `cc-by-sa` models may travel
//! with a build, with their notice; `nc` (non-commercial), `mixed` and `unlicensed` models never do. They are
//! fetched on the person's request from a source they name, shown with their licence and author, and kept in
//! their own library. Every file is checked against the published SHA-256 as it arrives and as it unpacks,
//! and a file that does not match is refused and not kept.
//!
//! Options (polish WO-01, 2026-10-18): the same project's toolchangers, toolheads and add-ons. An option is a set
//! of parts (one pinned, licensed file each) kept under `parts/`; for a machine it fits, the catalogue holds the
//! place the project registers for it, and `scene` serves a machine with its chosen options as one list of files
//! to draw. An option is refused on a machine it does not fit (`check_selection`).
//!
//! Motion: the project registers which parts move with X, Y and Z for ten machines (`web/printer-motion.json`,
//! the project's own MIT-licensed data). A printer's calibration says how its reported position becomes
//! the model's axis values; `pose` does that arithmetic once, here, and the page only draws the result.
//! Six more (Voron V2.4 300 and 350, Trident 300, V0.2, Micron+ 180, Ender-3) are read from the part records in
//! their own models by `tools/printer-motion-parts.rs` into `web/printer-motion-parts.json`. A machine with
//! several tools (IDEX, toolchanger) lists them in its profile's `tools`: `pose_for_tool` moves the active
//! one with the head and parks the others at their docks.
use crate::{gzip, sha256::Sha256};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

type Result<T> = std::result::Result<T, String>;

const CATALOGUE: &str = include_str!("../web/printer-models.json");
const MOTION: &str = include_str!("../web/printer-motion.json");
const THUMBS: &str = include_str!("../web/printer-thumbs.json");
/// Machines added by hand from their maker's own CAD (the Prusa CORE One L, 2026-10-13), beside the
/// 3D Print Rig catalogue, so a rebuild of that catalogue never drops them.
const EXTRA: &str = include_str!("../web/printer-models-extra.json");
const MOTION_EXTRA: &str = include_str!("../web/printer-motion-extra.json");
/// Motion read from the part records inside the models of machines 3D Print Rig registers there rather
/// than in its motion-profiles folder (Voron V2.4 300 and 350, Trident 300, V0.2, Micron+ 180, Ender-3), made by
/// `tools/printer-motion-parts.rs` (polish WO-02, 2026-10-18).
const MOTION_PARTS: &str = include_str!("../web/printer-motion-parts.json");
/// Where a print file's X0 Y0 Z0 sits on each machine's model (polish WO-03, 2026-10-18), made by
/// `tools/printer-print-frames.rs` from the model files and the makers' machine profiles.
const PRINT_FRAMES: &str = include_str!("../web/printer-print-frames.json");
/// The same for the Prusa machines whose converted CAD ships in the build (F09, 2026-10-19), made by the same tool into a file of its own.
const PRINT_FRAMES_PRUSA: &str = include_str!("../web/printer-print-frames-prusa.json");

/// The models this build carries (finish-2 F09, 2026-10-19; the owner: "we can share it on the application ... every Prusa printer should be available").
/// Each is the converted CAD of a machine, gzipped, under the open licence its author gives (Prusa Research's Open Community License for the CORE One and
/// the CORE One L, passed on free of charge with Prusa's notice; the community assemblies of the MK3S+ and the MINI under the licences their rows say),
/// checked against the catalogue's pins when it is put in the person's library. `tools/printer-cad/build-model.sh` makes each from its STEP file.
const BUNDLED: [(&str, &[u8]); 4] = [
    ("prusa_core_one_l", include_bytes!("../models/prusa-core-one-l.glb.gz")),
    ("prusa_core_one", include_bytes!("../models/prusa-core-one.glb.gz")),
    ("prusa_mk3s", include_bytes!("../models/prusa-mk3s.glb.gz")),
    ("prusa_mini", include_bytes!("../models/prusa-mini.glb.gz")),
];
/// What a person reads about each carried model: who made it, under what terms and what was done to the file (shown in the model's own sheet and
/// in About). The two Prusa notices end with the Open Community License text.
const NOTICES: [(&str, &str); 4] = [
    ("prusa_core_one_l", include_str!("../docs/notices/prusa_core_one_l.txt")),
    ("prusa_core_one", include_str!("../docs/notices/prusa_core_one.txt")),
    ("prusa_mk3s", include_str!("../docs/notices/prusa_mk3s.txt")),
    ("prusa_mini", include_str!("../docs/notices/prusa_mini.txt")),
];
/// The largest model file taken in (the biggest published is 80 MB packed).
const MAX_MODEL_BYTES: u64 = 400 * 1024 * 1024;
/// The largest an unpacked model may be.
const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;

fn parsed(cell: &'static OnceLock<Value>, text: &'static str) -> &'static Value {
    cell.get_or_init(|| {
        serde_json::from_str(text).expect("the built-in model catalogue is valid JSON")
    })
}

pub fn catalogue() -> &'static Value {
    static C: OnceLock<Value> = OnceLock::new();
    C.get_or_init(|| {
        let mut c: Value =
            serde_json::from_str(CATALOGUE).expect("the built-in model catalogue is valid JSON");
        let extra: Value =
            serde_json::from_str(EXTRA).expect("the extra model catalogue is valid JSON");
        if let (Some(m), Some(x)) = (c["machines"].as_array_mut(), extra["machines"].as_array()) {
            m.extend(x.iter().cloned());
        }
        // F03 (2026-10-19): option files and options added by hand (the Trident R2 gantry and carriage). A machine's own `fits` list
        // names the options that fit it, so each added option is listed on the machines it says it fits.
        if let (Some(p), Some(x)) = (c["parts"].as_object_mut(), extra["parts"].as_object()) {
            p.extend(x.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        for o in extra["options"].as_array().into_iter().flatten() {
            for m in o["fits"].as_object().into_iter().flat_map(|f| f.keys()) {
                let row = c["machines"].as_array_mut().and_then(|ms| ms.iter_mut().find(|r| r["id"] == m.as_str()));
                if let Some(list) = row.and_then(|r| r["fits"].as_array_mut()) {
                    list.push(o["id"].clone());
                }
            }
            if let Some(list) = c["options"].as_array_mut() {
                list.push(o.clone());
            }
        }
        c
    })
}

fn motion_table() -> &'static Value {
    static M: OnceLock<Value> = OnceLock::new();
    M.get_or_init(|| {
        let mut m: Value =
            serde_json::from_str(MOTION).expect("the built-in motion table is valid JSON");
        let extra: Value =
            serde_json::from_str(MOTION_EXTRA).expect("the extra motion table is valid JSON");
        let parts: Value =
            serde_json::from_str(MOTION_PARTS).expect("the part-record motion table is valid JSON");
        for table in [&extra, &parts] {
            if let (Some(p), Some(x)) =
                (m["profiles"].as_object_mut(), table["profiles"].as_object())
            {
                p.extend(x.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        // A registration with no calibration of its own starts at the low end of each travel, which leaves the nozzle
        // wherever the CAD pose put it. Where the plate and the nozzle were both measured (print frames), that
        // measurement is the calibration and the travel reaches the plate (polish WO-03).
        for (id, frame) in print_frames()["machines"].as_object().into_iter().flatten() {
            let (Some(over), Some(profile)) = (frame["profile"].as_object(), m["profiles"].get_mut(id.as_str())) else {
                continue;
            };
            if profile.get("calibration").is_some() {
                continue;
            }
            for (k, v) in over {
                profile[k.as_str()] = v.clone();
            }
        }
        m
    })
}

/// Where a print file's coordinates land on a machine's model: `origin_mm` is printer X0 Y0 Z0 in the model's frame
/// (the frame of its motion table, millimetres), `axes` the printer's X, Y and Z as vectors in that frame, `group`
/// the motion group that carries the plate (the bed that drops or slides) or null when the plate stands still.
/// A point (x, y, z) of a print file is at `origin + x * axes[0] + y * axes[1] + z * axes[2]`.
pub fn print_frame(id: &str) -> Option<&'static Value> {
    print_frames()["machines"].get(id).filter(|v| v.is_object())
}

fn print_frames() -> &'static Value {
    static F: OnceLock<Value> = OnceLock::new();
    F.get_or_init(|| {
        let mut f: Value = serde_json::from_str(PRINT_FRAMES).expect("the print frames are valid JSON");
        let extra: Value = serde_json::from_str(PRINT_FRAMES_PRUSA).expect("the Prusa print frames are valid JSON");
        if let (Some(m), Some(x)) = (f["machines"].as_object_mut(), extra["machines"].as_object()) {
            m.extend(x.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        f
    })
}

/// The thumbnails that travel with a build: only machines whose licence class allows it.
pub fn thumbs() -> &'static Value {
    static T: OnceLock<Value> = OnceLock::new();
    parsed(&T, THUMBS)
}

pub fn machine(id: &str) -> Option<&'static Value> {
    catalogue()["machines"]
        .as_array()?
        .iter()
        .find(|m| m["id"] == id)
}

/// The registered motion of a machine, when the project registered one.
pub fn profile(id: &str) -> Option<&'static Value> {
    motion_table()["profiles"].get(id).filter(|v| v.is_object())
}

/// The options of the catalogue (polish WO-01, 2026-10-18): toolchangers, toolheads and add-ons. Each is a set of
/// parts (one pinned, licensed GLB each) and, for a machine it fits, the place the project registers for it.
pub fn options() -> &'static [Value] {
    catalogue()["options"].as_array().map_or(&[], Vec::as_slice)
}

pub fn option(id: &str) -> Option<&'static Value> {
    options().iter().find(|o| o["id"] == id)
}

/// One file of an option, by its id.
pub fn part(id: &str) -> Option<&'static Value> {
    catalogue()["parts"].get(id).filter(|p| p.is_object())
}

/// May a build carry a model of this licence class? Only classes that allow redistribution, with notice.
/// Finish-2 F09 (2026-10-19: "we can share it on the application ... every Prusa printer should be available"): Prusa Research's Open Community
/// License is one of them. It lets the CAD be modified and the modification passed on under the same licence; what it does not allow is selling
/// machines or remixes made from it, and the converted files are passed on free of charge, with Prusa's notice and the licence text, shown in the
/// model's sheet and in About. (Whether a build that is itself sold may carry them is a question for Prusa; the report to the owner says so.)
pub fn may_bundle(class: &str) -> bool {
    matches!(class, "gpl" | "mit" | "cc-by" | "cc-by-sa" | "cc0" | "ocl")
}

/// What the licence class means, in words for the page.
pub fn class_words(class: &str) -> &'static str {
    match class {
        "gpl" => "Open licence (GPL). May be shared with its notice.",
        "mit" => "Open licence (MIT). May be shared with its notice.",
        "cc-by" => "Open licence (CC BY). May be shared with credit.",
        "cc-by-sa" => "Open licence (CC BY-SA). May be shared with credit, under the same terms.",
        "cc0" => "Public domain (CC0). May be shared.",
        "nc" => "Non-commercial licence. Downloaded from the author when you ask, never shipped with Stella Scope.",
        "ocl" => "Open Community License, from the maker (Prusa Research). Free to use and to modify, passed on under the same licence; not to be resold. Shared with its notice.",
        "none" => "There is no 3D model of this machine yet.",
        "mixed" => "Mixed terms, some non-commercial. Downloaded from the author when you ask, never shipped with Stella Scope.",
        _ => "No licence stated by the author. Downloaded from the author when you ask, never shipped with Stella Scope.",
    }
}

fn class_of(row: &Value) -> &str {
    row["licence"]["class"].as_str().unwrap_or("unlicensed")
}

/// Where a machine's files sit relative to the root of its project's published pages, when known:
/// `sv08-v2/model.glb.gz`. Taken from the upstream address in the catalogue.
fn page_path(row: &Value) -> Option<String> {
    let up = row["location"]["upstream"].as_str()?;
    let rest = up.strip_prefix("https://psych0h3ad.github.io/")?;
    Some(rest.split_once('/')?.1.to_owned())
}

/// The address the author publishes the file at, for the person to download by hand.
pub fn upstream_url(row: &Value) -> Option<String> {
    row["location"]["upstream"].as_str().map(str::to_owned)
}

/// Does this build carry the model (finish-2 F09)? It is put in the person's library when the service starts (`Library::seed_bundled`).
pub fn is_bundled(id: &str) -> bool {
    BUNDLED.iter().any(|(b, _)| *b == id)
}

/// The notice a person reads about a carried model: who made it, the terms, what was done to the file, then the licence text it is under.
pub fn notice_text(row: &Value) -> Option<String> {
    let id = row["id"].as_str()?;
    let Some((_, text)) = NOTICES.iter().find(|(n, _)| *n == id) else {
        // rc9 WO-02: the open-licence models the installer carries, unchanged as 3D Print Rig publishes them.
        if !may_bundle(class_of(row)) {
            return None;
        }
        let l = &row["licence"];
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        let mut out = format!(
            "{}\nAuthor: {}\nLicence: {}\nSource: {}\nCarried unchanged, as converted and published by 3D Print Rig ({}).\n",
            s(&row["label"]), s(&l["author"]), s(&l["text"]),
            if s(&l["source"]).is_empty() { s(&row["location"]["upstream"]) } else { s(&l["source"]) },
            catalogue()["source"].as_str().unwrap_or("")
        );
        if class_of(row) == "gpl" {
            out.push_str("\nTHE GNU GENERAL PUBLIC LICENSE, VERSION 3\n\n");
            out.push_str(GPL3_TEXT);
        }
        return Some(out);
    };
    let (title, licence) = match class_of(row) {
        "ocl" => ("THE OPEN COMMUNITY LICENSE (OCL v1), AS PUBLISHED AT https://github.com/OpenCommunityLicence/OpenCommunityLicence (TAG v1)", OCL_TEXT),
        _ if id == "prusa_mini" => ("THE GNU GENERAL PUBLIC LICENSE, VERSION 3 (A COPY OF PRUSA RESEARCH'S OWN, FROM github.com/prusa3d/Original-Prusa-MINI)", GPL3_TEXT),
        _ => ("THE GNU GENERAL PUBLIC LICENSE, VERSION 2 (A COPY OF PRUSA RESEARCH'S OWN, FROM github.com/prusa3d/Original-Prusa-i3)", GPL2_TEXT),
    };
    Some(format!("{text}\n{title}\n\n{licence}"))
}
const OCL_TEXT: &str = include_str!("../docs/licences/OCL-v1.txt");
const GPL2_TEXT: &str = include_str!("../docs/licences/GPL-2.0.txt");
const GPL3_TEXT: &str = include_str!("../docs/licences/GPL-3.0.txt");

/// Who is credited for each model the build carries, for About: the machine, the maker, the licence in words and where the CAD comes from.
pub fn credits() -> Value {
    let rows: Vec<Value> = BUNDLED
        .iter()
        .filter_map(|(id, bytes)| {
            let m = machine(id)?;
            let class = class_of(m);
            Some(json!({
                "id":id,"label":m["label"].as_str().unwrap_or(id).split(" / ").next().unwrap_or(id),
                "author":m["licence"]["author"],"licence":m["licence"]["text"],"class":class,"words":class_words(class),
                "source":m["licence"]["source"],"bytes":bytes.len(),
            }))
        })
        .collect();
    let bytes: usize = BUNDLED.iter().map(|(_, b)| b.len()).sum();
    // Finish-2 F07 (2026-10-19): About also says who to write to and credits 3D Print Rig with its MIT notice (docs/licences/3D-Print-Rig-MIT.txt).
    json!({"version":env!("CARGO_PKG_VERSION"),"licence":"AGPL-3.0","maker":"3D3D Canada","contact":"info@3d3d.ca",
        "rig":{"name":"3D Print Rig","by":"Psych0h3ad","licence":"MIT","source":"https://github.com/Psych0h3ad/3d-print-rig","notice":crate::help::RIG_NOTICE},
        "models":rows,"bytes":bytes})
}

fn pin(row: &Value) -> (String, String) {
    (
        row["pin"]["stored_model_sha256"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        row["pin"]["model_sha256"].as_str().unwrap_or("").to_owned(),
    )
}

// ---------------------------------------------------------------------------------------------- sources

/// Where a model is fetched from: a directory laid out like the project's published pages (or a checkout
/// of it, or an unpacked `viewer-models.zip`), or a plain `http://` address of one.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Dir(PathBuf),
    Http(String),
    /// The address the author publishes the file at (the catalogue row says which), over https only.
    Published,
}

impl Source {
    pub fn parse(text: &str) -> Result<Source> {
        let t = text.trim();
        if t.is_empty() {
            return Err("name a folder or a web address to get the models from, or say published to take them from their authors".into());
        }
        // Polish WO-01 (2026-10-18): "published" fetches each file from the address its author publishes it at.
        if t.eq_ignore_ascii_case("published") {
            return Ok(Source::Published);
        }
        // WO-13: https works now (ureq with rustls); the pinned SHA-256 is still checked before use.
        if t.starts_with("https://") || t.starts_with("http://") {
            return Ok(Source::Http(t.trim_end_matches('/').to_owned()));
        }
        let p = PathBuf::from(t);
        if !p.is_absolute() {
            return Err("give the folder as a full path, starting with /".into());
        }
        if !p.is_dir() {
            return Err(format!("{t} is not a folder"));
        }
        Ok(Source::Dir(p))
    }

    pub fn label(&self) -> String {
        match self {
            Source::Dir(p) => format!("folder {}", p.display()),
            Source::Http(u) => u.clone(),
            Source::Published => "the author's published address".to_owned(),
        }
    }

    /// The relative paths a machine's model may sit at, most specific first.
    fn candidates(row: &Value) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(p) = page_path(row) {
            v.push(format!("site/{p}"));
            v.push(p);
        }
        if let Some(id) = row["id"].as_str() {
            v.push(format!("machines/{id}/model.glb.gz"));
            v.push(format!("{id}/model.glb.gz"));
        }
        v
    }

    fn open(&self, row: &Value) -> Result<Box<dyn Read>> {
        match self {
            Source::Dir(root) => {
                for rel in Self::candidates(row) {
                    if rel.split('/').any(|s| s == ".." || s.is_empty()) {
                        continue;
                    }
                    let path = root.join(&rel);
                    if path.is_file() {
                        return fs::File::open(&path)
                            .map(|f| Box::new(f) as Box<dyn Read>)
                            .map_err(|e| format!("cannot read {}: {e}", path.display()));
                    }
                }
                Err(format!(
                    "{} does not hold this model (looked for {})",
                    self.label(),
                    Self::candidates(row).join(", ")
                ))
            }
            Source::Http(base) => {
                let mut last = String::new();
                for rel in Self::candidates(row) {
                    match http_get(&format!("{base}/{rel}")) {
                        Ok(r) => return Ok(r),
                        Err(e) => last = e,
                    }
                }
                Err(format!("the source did not give this model ({last})"))
            }
            Source::Published => {
                // Only a file the catalogue knows the address of: a maker's web page is not a file.
                let kind = row["location"]["kind"].as_str().unwrap_or("");
                let url = upstream_url(row).filter(|u| {
                    u.starts_with("https://") && (kind == "registry" || kind == "release_zip")
                });
                let url = url.ok_or_else(|| {
                    format!(
                        "{} has no published file address; download it from its author and install the file",
                        row["label"].as_str().unwrap_or("this model")
                    )
                })?;
                http_get(&url)
                    .map_err(|e| format!("the author's address did not give this model ({e})"))
            }
        }
    }
}

/// One GET with the library's timeouts; the body is the file. Only a 200 is a file.
fn http_get(url: &str) -> Result<Box<dyn Read>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(900)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    match agent.get(url).call() {
        Ok(r) if r.status().as_u16() == 200 => Ok(Box::new(r.into_body().into_reader())),
        Ok(r) => Err(format!("{url} answered {}", r.status().as_u16())),
        // F03 (2026-10-19): no answer at all (no network, no DNS, a refused or timed-out connection) is said so; the page tells it from an answer
        // that is not the file.
        Err(e) => Err(format!("{url}: no connection ({e})")),
    }
}

// ---------------------------------------------------------------------------------------------- library

/// The person's library of installed models: `<root>/<id>/model.glb.gz` (or `.glb`) and `install.json`.
pub struct Library {
    root: PathBuf,
    /// The open-licence models the installer carries (rc9 WO-02), read in place: never copied into `root`, never written.
    carried: Option<PathBuf>,
}

/// Where the installer put the carried models: `STELLA_MODELS_DIR`, else the Tauri resource folder beside the binary
/// (`/usr/lib/Stella Scope/models` for the deb), else `models` beside the binary. None when no folder is there.
pub fn carried_dir() -> Option<PathBuf> {
    // Tests never read an installed package's folder: what the library holds must not depend on what is installed on the machine running them.
    if cfg!(test) {
        return std::env::var_os("STELLA_MODELS_DIR").map(PathBuf::from).filter(|p| p.is_dir());
    }
    let exe = std::env::current_exe().ok();
    let dir = exe.as_deref().and_then(Path::parent);
    [
        std::env::var_os("STELLA_MODELS_DIR").map(PathBuf::from),
        dir.map(|d| d.join("../lib/Stella Scope/models")),
        Some(PathBuf::from("/usr/lib/Stella Scope/models")),
        dir.map(|d| d.join("models")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self { root, carried: None }
    }
    /// The service's library: its own files, and the carried folder (`carried_dir()`). `new` alone carries nothing (tests, a library that is not there).
    pub fn with_carried(root: PathBuf, carried: Option<PathBuf>) -> Self {
        Self { root, carried }
    }
    /// The carried file for a machine whose licence lets a build carry it; nothing for any other.
    fn carried_file(&self, id: &str) -> Option<PathBuf> {
        let row = machine(id)?;
        if !may_bundle(class_of(row)) {
            return None;
        }
        let p = self.carried.as_ref()?.join(format!("{id}.glb.gz"));
        p.is_file().then_some(p)
    }

    fn dir(&self, id: &str) -> Result<PathBuf> {
        if machine(id).is_none() {
            return Err(format!("{id} is not a model in the catalogue"));
        }
        Ok(self.root.join(id))
    }

    pub fn record(&self, id: &str) -> Option<Value> {
        if let Some(t) = self.dir(id).ok().and_then(|d| fs::read_to_string(d.join("install.json")).ok()) {
            return serde_json::from_str(&t).ok();
        }
        let p = self.carried_file(id)?;
        Some(json!({"origin":"carried with Stella Scope","bytes":fs::metadata(&p).ok()?.len(),"installed_at":null,"carried":true}))
    }

    /// The stored model file and whether it is gzipped.
    pub fn model_file(&self, id: &str) -> Option<(PathBuf, bool)> {
        let d = self.dir(id).ok()?;
        if d.join("install.json").is_file() {
            for (name, gz) in [("model.glb.gz", true), ("model.glb", false)] {
                if d.join(name).is_file() {
                    return Some((d.join(name), gz));
                }
            }
        }
        self.carried_file(id).map(|p| (p, true))
    }

    pub fn installed_ids(&self) -> Vec<String> {
        catalogue()["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str())
            .filter(|id| self.model_file(id).is_some())
            .map(str::to_owned)
            .collect()
    }

    /// Put the models this build carries in the library (finish-2 F09, 2026-10-19): each whose file is not there, or is not the one the catalogue
    /// pins. A copy the person put there themselves that has the pinned content is left as it is. The bytes are checked against the pins on the way
    /// in like any other file. Returns the ids written. Called once when the service starts, so a fresh install draws these machines with nothing
    /// to download and no file to find.
    pub fn seed_bundled(&self) -> Vec<String> {
        let mut wrote = Vec::new();
        for (id, bytes) in BUNDLED {
            let Some(row) = machine(id) else { continue };
            let (_, want_decoded) = pin(row);
            // Here already: the record says it is this model and the file is the size the record says (a cut-short copy is written again).
            let here = self.model_file(id).and_then(|(p, _)| fs::metadata(p).ok()).map(|m| m.len());
            if self.record(id).is_some_and(|r| r["decoded_sha256"] == want_decoded.as_str() && here.is_some() && r["bytes"].as_u64() == here) {
                continue;
            }
            match self.store(row, &mut std::io::Cursor::new(bytes), "bundled with Stella Scope") {
                Ok(_) => wrote.push((*id).to_owned()),
                Err(e) => eprintln!("stella-scope: could not put the {id} model in the library: {e}"),
            }
        }
        wrote
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        // A model the build carries comes back at the next start (and is the one thing that makes a CORE One L draw on a fresh install), so it is not removed.
        if is_bundled(id) {
            return Err(format!("{} ships inside Stella Scope and cannot be removed here.", machine(id).and_then(|m| m["label"].as_str()).unwrap_or(id).split(" / ").next().unwrap_or(id)));
        }
        let d = self.dir(id)?;
        if d.is_dir() {
            fs::remove_dir_all(&d).map_err(|e| format!("cannot remove {id}: {e}"))?;
        }
        Ok(())
    }

    // ---- options (polish WO-01): the files of toolchangers, toolheads and add-ons, kept under `parts/`.

    fn part_dir(&self, id: &str) -> Result<PathBuf> {
        if part(id).is_none() {
            return Err(format!("{id} is not a part in the catalogue"));
        }
        Ok(self.root.join("parts").join(id))
    }

    /// A tool or add-on file the installer carries (`<carried>/parts/<id>.glb.gz`), for open-licence parts only.
    fn carried_part(&self, id: &str) -> Option<PathBuf> {
        if !may_bundle(part(id)?["licence"]["class"].as_str().unwrap_or("unlicensed")) {
            return None;
        }
        let p = self.carried.as_ref()?.join("parts").join(format!("{id}.glb.gz"));
        p.is_file().then_some(p)
    }

    /// The stored file of a part and whether it is gzipped.
    pub fn part_file(&self, id: &str) -> Option<(PathBuf, bool)> {
        let d = self.part_dir(id).ok()?;
        if d.join("install.json").is_file() {
            for (name, gz) in [("model.glb.gz", true), ("model.glb", false)] {
                if d.join(name).is_file() {
                    return Some((d.join(name), gz));
                }
            }
        }
        self.carried_part(id).map(|p| (p, true))
    }

    /// An option is installed when every one of its files is.
    pub fn option_installed(&self, o: &Value) -> bool {
        let ids = part_ids(o);
        !ids.is_empty() && ids.iter().all(|p| self.part_file(p).is_some())
    }

    #[cfg(test)]
    pub fn installed_options(&self) -> Vec<String> {
        options()
            .iter()
            .filter(|o| self.option_installed(o))
            .filter_map(|o| o["id"].as_str())
            .map(str::to_owned)
            .collect()
    }

    /// Fetch every file of an option, each checked against its own pinned SHA-256. The licence is the strictest of
    /// its files' (the catalogue says which), confirmed before the first byte is read.
    pub fn install_option(&self, id: &str, source: &Source, accepted: bool) -> Result<Value> {
        let o = option(id).ok_or_else(|| format!("{id} is not an option in the catalogue"))?;
        gate(o, accepted)?;
        let ids = part_ids(o);
        for pid in &ids {
            if self.part_file(pid).is_some() {
                continue;
            }
            let row = part(pid).ok_or_else(|| format!("{pid} is not a part in the catalogue"))?;
            let mut reader = source.open(row).map_err(|e| format!("{pid}: {e}"))?;
            self.store_into(self.part_dir(pid)?, row, &mut reader, &source.label())
                .map_err(|e| format!("{pid}: {e}"))?;
        }
        Ok(json!({"option":id,"parts":ids.len(),"installed":true}))
    }

    /// Remove an option's files, keeping any that another installed option still needs.
    pub fn remove_option(&self, id: &str) -> Result<()> {
        let o = option(id).ok_or_else(|| format!("{id} is not an option in the catalogue"))?;
        let keep: std::collections::BTreeSet<String> = options()
            .iter()
            .filter(|x| x["id"] != id && self.option_installed(x))
            .flat_map(part_ids)
            .collect();
        for pid in part_ids(o) {
            if keep.contains(&pid) {
                continue;
            }
            let d = self.part_dir(&pid)?;
            if d.is_dir() {
                fs::remove_dir_all(&d).map_err(|e| format!("cannot remove {pid}: {e}"))?;
            }
        }
        Ok(())
    }

    /// Fetch a model from `source`, check it, keep it. `accepted` is the person's confirmation that they have
    /// read the licence, required for any model a build may not carry.
    pub fn install_from(&self, id: &str, source: &Source, accepted: bool) -> Result<Value> {
        let row = machine(id).ok_or_else(|| format!("{id} is not a model in the catalogue"))?;
        self.install_row(row, source, accepted)
    }

    pub fn install_row(&self, row: &Value, source: &Source, accepted: bool) -> Result<Value> {
        gate(row, accepted)?;
        let mut reader = source.open(row)?;
        self.store(row, &mut reader, &source.label())
    }

    /// Keep a file the person downloaded from the author themselves.
    pub fn install_file(
        &self,
        id: &str,
        reader: &mut dyn Read,
        origin: &str,
        accepted: bool,
    ) -> Result<Value> {
        let row = machine(id).ok_or_else(|| format!("{id} is not a model in the catalogue"))?;
        gate(row, accepted)?;
        self.store(row, reader, origin)
    }

    #[cfg(test)]
    pub fn install_file_row(
        &self,
        row: &Value,
        reader: &mut dyn Read,
        origin: &str,
        accepted: bool,
    ) -> Result<Value> {
        gate(row, accepted)?;
        self.store(row, reader, origin)
    }

    fn store(&self, row: &Value, reader: &mut dyn Read, origin: &str) -> Result<Value> {
        let id = row["id"].as_str().ok_or("catalogue row without an id")?;
        self.store_into(self.root.join(id), row, reader, origin)
    }

    /// Keep a checked file in `dir` (a machine's folder, or an option part's under `parts/`).
    fn store_into(
        &self,
        dir: PathBuf,
        row: &Value,
        reader: &mut dyn Read,
        origin: &str,
    ) -> Result<Value> {
        let id = row["id"].as_str().ok_or("catalogue row without an id")?;
        let label = row["label"].as_str().unwrap_or(id);
        let (want_stored, want_decoded) = pin(row);
        if want_stored.len() != 64 || want_decoded.len() != 64 {
            return Err(format!("the catalogue has no SHA-256 for {label}, so it cannot be checked and is not installed"));
        }
        fs::create_dir_all(&dir).map_err(|e| format!("cannot make the library folder: {e}"))?;
        let tmp = dir.join(format!(".incoming-{}", std::process::id()));
        let result = (|| -> Result<(Value, bool)> {
            // Take the file in, hashing as it arrives.
            let mut out =
                fs::File::create(&tmp).map_err(|e| format!("cannot write the library: {e}"))?;
            let (mut h, mut total, mut head) = (Sha256::new(), 0_u64, Vec::with_capacity(4));
            let mut buf = vec![0_u8; 64 * 1024];
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| format!("the download was interrupted: {e}"))?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > MAX_MODEL_BYTES {
                    return Err(format!(
                        "this file is larger than the {} MB limit for a model",
                        MAX_MODEL_BYTES / 1_048_576
                    ));
                }
                if head.len() < 4 {
                    head.extend_from_slice(&buf[..n.min(4 - head.len())]);
                }
                h.update(&buf[..n]);
                out.write_all(&buf[..n])
                    .map_err(|e| format!("cannot write the library: {e}"))?;
            }
            out.flush()
                .map_err(|e| format!("cannot write the library: {e}"))?;
            drop(out);
            let got = h.hex();
            let gz = head.starts_with(&[0x1f, 0x8b]);
            if !gz {
                // A plain GLB: it is the model when its own hash is the decoded pin.
                if head.as_slice() == b"glTF" && got == want_decoded {
                    return Ok((
                        json!({"stored_sha256":got,"decoded_sha256":got,"bytes":total,"unpacked_bytes":total}),
                        false,
                    ));
                }
                return Err(refusal(label, &got, &want_stored));
            }
            // A gzipped file: unpack it to a hash and compare with the decoded pin.
            let mut d = Sha256::new();
            let file = fs::File::open(&tmp).map_err(|e| e.to_string())?;
            let mut unpacked = 0_u64;
            gzip::gunzip(file, |c| {
                unpacked += c.len() as u64;
                if unpacked > MAX_UNPACKED_BYTES {
                    return Err("the unpacked model is larger than the 1 GB limit".into());
                }
                d.update(c);
                Ok(())
            })
            .map_err(|e| format!("This file could not be unpacked: {e}. Nothing was installed."))?;
            let decoded = d.hex();
            if decoded != want_decoded {
                return Err(refusal(label, &got, &want_stored));
            }
            Ok((
                json!({"stored_sha256":got,"decoded_sha256":decoded,"bytes":total,"unpacked_bytes":unpacked}),
                true,
            ))
        })();
        let (facts, gz) = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                if fs::read_dir(&dir)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = fs::remove_dir(&dir);
                }
                return Err(e);
            }
        };
        let name = if gz { "model.glb.gz" } else { "model.glb" };
        let other = if gz { "model.glb" } else { "model.glb.gz" };
        let _ = fs::remove_file(dir.join(other));
        fs::rename(&tmp, dir.join(name)).map_err(|e| format!("cannot keep the model: {e}"))?;
        let record = json!({
            "id":id,"label":label,"file":name,"origin":origin,
            "installed_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "stored_sha256":facts["stored_sha256"],"decoded_sha256":facts["decoded_sha256"],
            "bytes":facts["bytes"],"unpacked_bytes":facts["unpacked_bytes"],
            "licence":row["licence"],"source_url":upstream_url(row),
            "notice":if is_bundled(id) { "This model ships inside Stella Scope under its author's licence. Its notice is under About and under Printer models." } else { "The model keeps its author's licence. It is stored here for this installation and is not part of Stella Scope." },
        });
        fs::write(
            dir.join("install.json"),
            serde_json::to_vec_pretty(&record).unwrap_or_default(),
        )
        .map_err(|e| format!("cannot keep the model: {e}"))?;
        Ok(record)
    }
}

fn refusal(label: &str, got: &str, want: &str) -> String {
    format!(
        "This file is not the model the author published for {label}: its SHA-256 is {}... and the published one is {}.... Nothing was installed.",
        &got[..got.len().min(12)],
        &want[..want.len().min(12)]
    )
}

/// A model a build may not carry is only fetched once the person says they have read its licence.
fn gate(row: &Value, accepted: bool) -> Result<()> {
    if may_bundle(class_of(row)) || accepted {
        return Ok(());
    }
    Err(format!(
        "{} is under {}: {} Read the licence and confirm to install it.",
        row["label"].as_str().unwrap_or("This model"),
        row["licence"]["text"]
            .as_str()
            .unwrap_or("terms its author did not state"),
        class_words(class_of(row))
    ))
}

// ---------------------------------------------------------------------------------------------- views

/// A machine for the models page. Never carries file paths or pins; those stay in the catalogue.
pub fn view(row: &Value, lib: &Library) -> Value {
    let id = row["id"].as_str().unwrap_or("");
    let class = class_of(row);
    let motion = profile(id);
    json!({
        "id":id,"label":row["label"],"family":row["family"],"size_mm":row["size_mm"],
        "licence":{"class":class,"text":row["licence"]["text"],"author":row["licence"]["author"],"source":row["licence"]["source"],"words":class_words(class)},
        "bundled":may_bundle(class),
        // finish-2 F09: the build carries this model's file (it is in the library from the first start and cannot be removed)
        "in_build":is_bundled(id),
        "motion":motion.is_some(),
        "basis":basis(id),
        "axes":motion.map(|p| p["axes"].clone()),
        "triangles":row["triangles"],"parts":row["parts"],
        "fits":row["fits"],"note":row["note"],
        "has_file":row["location"]["kind"] != "none",
        // Polish WO-05: how much a fetch brings, so the page can say so before anything is fetched (null when the catalogue does not know).
        "bytes":row["pin"]["stored_bytes"].as_u64().or_else(|| row["location"]["mirror"]["bytes"].as_u64()),
        "from_author":upstream_url(row),
        "thumb":thumbs()["thumbs"].get(id).map(|_| format!("/api/printer-models/{id}/thumb")),
        "installed":lib.record(id).map(|r| json!({"installed_at":r["installed_at"],"bytes":r["bytes"],"origin":r["origin"]})),
    })
}

pub fn list(lib: &Library) -> Value {
    let rows: Vec<Value> = catalogue()["machines"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| view(m, lib))
        .collect();
    // Finish-2 F09: the Prusa machines no 3D model draws, each with the exact reason (they are not catalogue models: there is no file, no pin and no licence to show).
    let no_model: Vec<Value> = crate::prusa_machines::MACHINES
        .iter()
        .filter(|m| m.cad.is_none())
        .map(|m| json!({"id":m.id,"label":m.name,"short":m.short,"family":"prusa","why":m.no_model}))
        .collect();
    json!({"schema":"stella-scope.printer-models.v1","count":rows.len(),"source":catalogue()["source"],"models":rows,"no_model":no_model})
}

/// A thumbnail that travels with the build (open-licence machines only), as WebP bytes.
pub fn thumb_bytes(id: &str) -> Option<Vec<u8>> {
    crate::licence::b64_decode(thumbs()["thumbs"].get(id)?.as_str()?).ok()
}

/// The matrix that carries the registered motion (the CAD frame, millimetres) into the model file's frame
/// (Y up, metres, shifted to the machine's origin). Most CAD is Z up, so most machines carry
/// `[[1,0,0],[0,0,1],[0,-1,0]]`; a machine whose file says so is already Y up (identity).
pub fn basis(id: &str) -> Value {
    machine(id)
        .map(|m| m["basis"].clone())
        .filter(|b| !b.is_null())
        .or_else(|| {
            profile(id)
                .map(|p| p["basis"].clone())
                .filter(|b| !b.is_null())
        })
        .unwrap_or_else(|| json!([[1, 0, 0], [0, 0, 1], [0, -1, 0]]))
}

// ---------------------------------------------------------------------------------------------- options on a machine

/// The files of an option, by id.
fn part_ids(o: &Value) -> Vec<String> {
    o["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
        .map(str::to_owned)
        .collect()
}

fn n3(v: &Value) -> [f64; 3] {
    [0, 1, 2].map(|i| v[i].as_f64().unwrap_or(0.0))
}

fn machine_label(id: &str) -> String {
    machine(id)
        .and_then(|m| m["label"].as_str())
        .unwrap_or(id)
        .to_owned()
}

/// An option for the page: what it is, its licence, its size, whether it is here, which machines take it.
pub fn option_view(o: &Value, lib: &Library) -> Value {
    let class = class_of(o);
    let kind = o["attach"]["kind"].as_str().unwrap_or("");
    let fits: Vec<&String> = o["fits"]
        .as_object()
        .map(|f| f.keys().collect())
        .unwrap_or_default();
    json!({
        "id":o["id"],"label":o["label"],"kind":o["kind"],"category":o["category"],"system":o["attach"]["system"],
        "licence":{"class":class,"text":o["licence"]["text"],"author":o["licence"]["author"],"source":o["licence"]["source"],"words":class_words(class)},
        "bundled":may_bundle(class),
        "bytes":o["bytes"],"parts":o["parts"].as_array().map_or(0, Vec::len),
        "installed":lib.option_installed(o),
        "fits":fits,
        // Stella Scope draws a bank or a placed add-on on its machine; the rest is listed with its licence and files.
        "drawable":(kind == "bank" || kind == "place") && !fits.is_empty(),
        // An option with files placed in its own frame (a changer system, a library part) can be looked at on its own.
        "previewable":o["attach"]["place"].as_array().is_some_and(|p| !p.is_empty()),
        "scope":o["scope"],
    })
}

/// One option in full: its files with their own licences, and how it sits on each machine that takes it.
pub fn option_detail(o: &Value, lib: &Library) -> Value {
    let mut v = option_view(o, lib);
    let parts: Vec<Value> = part_ids(o)
        .iter()
        .filter_map(|p| part(p).map(|r| (p, r)))
        .map(|(p, r)| {
            let c = class_of(r);
            json!({"id":p,"label":r["label"],"class":c,"licence":r["licence"]["text"],"bytes":r["bytes"],"installed":lib.part_file(p).is_some(),"words":class_words(c)})
        })
        .collect();
    v["files"] = json!(parts);
    v["machines"] = json!(o["fits"].as_object().map(|f| f
        .keys()
        .map(|m| json!({"id":m,"label":machine_label(m)}))
        .collect::<Vec<_>>()));
    v["note"] = o["attach"]["note"].clone();
    v
}

pub fn options_list(lib: &Library) -> Value {
    let rows: Vec<Value> = options().iter().map(|o| option_view(o, lib)).collect();
    json!({"schema":"stella-scope.printer-options.v1","count":rows.len(),"options":rows})
}

/// An option on its own, without a machine: its files at the places the project shows them (a changer system's
/// shuttle, backplate and dock; a library part by itself).
pub fn option_scene(o: &Value, lib: &Library) -> Value {
    let oid = o["id"].as_str().unwrap_or("");
    let placements: Vec<Value> = o["attach"]["place"].as_array().into_iter().flatten().filter_map(|p| {
        let id = p["part"].as_str()?;
        Some(json!({"part":id,"url":format!("/api/printer-models/parts/{id}/file"),"translation_mm":p["translation_mm"],"hidden_keys":if p["hidden_keys"].is_array() { p["hidden_keys"].clone() } else { json!([]) },
            "role":p["role"],"slot":-1,"option":oid,"follows":null,"installed":lib.part_file(id).is_some()}))
    }).collect();
    let mut missing: Vec<String> = placements
        .iter()
        .filter(|p| p["installed"] == false)
        .filter_map(|p| p["part"].as_str())
        .map(str::to_owned)
        .collect();
    missing.sort();
    missing.dedup();
    json!({"machine":null,"options":[{"option":oid,"label":o["label"],"installed":lib.option_installed(o)}],"placements":placements,"hide_machine_keys":[],"missing":missing})
}

/// What a machine takes: its options, each with what can be chosen on it (tool count and choices, or a plain add-on).
pub fn options_for(machine_id: &str, lib: &Library) -> Value {
    let rows: Vec<Value> = options()
        .iter()
        .filter(|o| o["fits"].get(machine_id).is_some_and(Value::is_object))
        .map(|o| {
            let fit = &o["fits"][machine_id];
            let mut v = option_view(o, lib);
            v["capacity"] = fit["capacity"].clone();
            v["tools"] = json!(fit["tools"].as_array().map(|t| t
                .iter()
                .filter_map(|id| id.as_str())
                .map(|id| json!({"id":id,"label":o["attach"]["tools"][id]["label"]}))
                .collect::<Vec<_>>()));
            v["datum"] = fit["datum"].clone();
            v
        })
        .collect();
    json!({"machine":machine_id,"label":machine_label(machine_id),"count":rows.len(),"options":rows})
}

/// What a printer chose, checked against its machine: an option that does not fit it, a bank with too many tools,
/// two banks, two versions of one part, or an option Stella Scope does not draw on a printer is refused with the
/// reason. Returns the choices in the form the scene reads (`tools` and `active` filled in).
pub fn check_selection(machine_id: &str, input: &Value) -> Result<Value> {
    if input.is_null() {
        return Ok(json!([]));
    }
    let list = input
        .as_array()
        .ok_or("cad_options must be a list of options, or null")?;
    let mlabel = machine_label(machine_id);
    let (mut seen, mut banks, mut groups): (Vec<&str>, u32, Vec<(String, String)>) =
        (vec![], 0, vec![]);
    let mut out = Vec::new();
    for item in list {
        let id = item["option"]
            .as_str()
            .ok_or("each choice names an option")?;
        let o = option(id).ok_or_else(|| format!("{id} is not an option in the catalogue"))?;
        let label = o["label"].as_str().unwrap_or(id);
        let fit = o["fits"]
            .get(machine_id)
            .filter(|f| f.is_object())
            .ok_or_else(|| format!("{label} does not fit {mlabel}"))?;
        if seen.contains(&id) {
            return Err(format!("{label} is chosen twice"));
        }
        seen.push(id);
        match o["attach"]["kind"].as_str().unwrap_or("") {
            "bank" => {
                banks += 1;
                if banks > 1 {
                    return Err(
                        "a printer has one tool bank: choose StealthChanger or INDX, not both"
                            .into(),
                    );
                }
                let cap = fit["capacity"].as_u64().unwrap_or(1) as usize;
                let allowed: Vec<&str> = fit["tools"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str())
                    .collect();
                let tools: Vec<String> = match &item["tools"] {
                    Value::Null => allowed
                        .first()
                        .map(|t| vec![(*t).to_owned()])
                        .unwrap_or_default(),
                    Value::Array(a) => a
                        .iter()
                        .map(|t| {
                            t.as_str()
                                .map(str::to_owned)
                                .ok_or("each tool is named by its id")
                        })
                        .collect::<std::result::Result<_, _>>()?,
                    _ => return Err("tools must be a list of tool ids".into()),
                };
                if tools.is_empty() || tools.len() > cap {
                    return Err(format!("{label} takes 1 to {cap} tools on {mlabel}"));
                }
                if let Some(t) = tools.iter().find(|t| !allowed.contains(&t.as_str())) {
                    return Err(format!("{t} is not a tool {label} takes on {mlabel}"));
                }
                let active = match &item["active"] {
                    Value::Null => 0,
                    v => v
                        .as_u64()
                        .ok_or("the tool on the gantry is a dock number, 0 or more")?
                        as usize,
                };
                if active >= tools.len() {
                    return Err(format!(
                        "the tool on the gantry must be one of the {} docks",
                        tools.len()
                    ));
                }
                out.push(json!({"option":id,"tools":tools,"active":active}));
            }
            "place" => {
                if let Some(g) = fit["exclusive_group"].as_str() {
                    if let Some((_, other)) = groups.iter().find(|(x, _)| x == g) {
                        return Err(format!(
                            "{label} and {other} are two versions of the same part"
                        ));
                    }
                    groups.push((g.to_owned(), label.to_owned()));
                }
                out.push(json!({"option":id}));
            }
            _ => {
                return Err(format!(
                "{label} is listed for {mlabel} but Stella Scope does not draw it on a printer yet"
            ))
            }
        }
    }
    Ok(Value::Array(out))
}

/// The scene of a machine with its chosen options: every file to draw with the place it goes (millimetres, the
/// machine's CAD frame; the page turns them into the model's frame with the machine's own basis), the parts of the
/// machine the options replace, and the files that are not installed yet. The arithmetic is the rig's own
/// `bankPlan` and `installedHeadPlan` (site/viewer/changer-bank-model.mjs, machine-head-model.mjs, MIT), carried
/// into the catalogue by tools/printer-models-index.rs and checked against the rig's code by scripts/.
pub fn scene(machine_id: &str, selection: &Value, lib: &Library) -> Value {
    scene_with(machine_id, selection, lib, false)
}

/// The same scene for a printer whose tools change while it prints (a toolchanger that reports its active tool): every
/// tool is placed at its dock and is its own motion group (`tool0`, `tool1`...) that the pose carries from the dock to
/// the carriage and back; the carriage's half of the coupling (the shuttle) rides with the head. The machine's own head is
/// hidden only when every file is here: with the bank not installed the stock head is drawn and stays the head.
pub fn scene_swapping(machine_id: &str, selection: &Value, lib: &Library) -> Value {
    scene_with(machine_id, selection, lib, true)
}

fn scene_with(machine_id: &str, selection: &Value, lib: &Library, swap: bool) -> Value {
    let mut placements: Vec<Value> = Vec::new();
    let mut hide: Vec<Value> = Vec::new();
    let mut chosen: Vec<Value> = Vec::new();
    let add = |placements: &mut Vec<Value>,
               part_id: &str,
               t: [f64; 3],
               hidden: &Value,
               role: &str,
               slot: i64,
               option: &str,
               follows: Option<&str>| {
        placements.push(json!({"part":part_id,"url":format!("/api/printer-models/parts/{part_id}/file"),"translation_mm":[t[0], t[1], t[2]],
            "hidden_keys":if hidden.is_array() { hidden.clone() } else { json!([]) },"role":role,"slot":slot,"option":option,"follows":follows,"installed":lib.part_file(part_id).is_some()}));
    };
    let sum = |a: [f64; 3], b: [f64; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    for item in selection.as_array().into_iter().flatten() {
        let Some(o) = item["option"].as_str().and_then(option) else {
            continue;
        };
        let oid = o["id"].as_str().unwrap_or("");
        let fit = &o["fits"][machine_id];
        if !fit.is_object() {
            continue;
        }
        chosen.push(json!({"option":oid,"label":o["label"],"installed":lib.option_installed(o),"tools":item["tools"],"active":item["active"]}));
        match o["attach"]["kind"].as_str().unwrap_or("") {
            "bank" => {
                let tools: Vec<&str> = item["tools"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str())
                    .collect();
                let active = item["active"].as_u64().unwrap_or(0) as usize;
                let (anchor, pitch, shift) = (
                    n3(&fit["anchor_mm"]),
                    fit["pitch_mm"].as_f64().unwrap_or(0.0),
                    n3(&fit["head_shift_mm"]),
                );
                if fit["fixture"].is_object() {
                    add(
                        &mut placements,
                        fit["fixture"]["part"].as_str().unwrap_or(""),
                        n3(&fit["fixture"]["translation_mm"]),
                        &json!([]),
                        "fixture",
                        -1,
                        oid,
                        None,
                    );
                }
                for (slot, tid) in tools.iter().enumerate() {
                    let tool = &o["attach"]["tools"][*tid];
                    if !tool.is_object() {
                        continue;
                    }
                    let a = [
                        anchor[0] + (slot as f64 - (tools.len() as f64 - 1.0) / 2.0) * pitch,
                        anchor[1],
                        anchor[2],
                    ];
                    add(
                        &mut placements,
                        tool["dock"]["part"].as_str().unwrap_or(""),
                        a,
                        &tool["dock"]["hidden_keys"],
                        "dock",
                        slot as i64,
                        oid,
                        None,
                    );
                    for f in o["attach"]["fixture_parts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f.as_str())
                    {
                        add(
                            &mut placements,
                            f,
                            a,
                            &json!([]),
                            "fastener",
                            slot as i64,
                            oid,
                            None,
                        );
                    }
                    if swap {
                        // Every tool is at its dock; its own group carries it to the carriage and back.
                        let group = format!("tool{slot}");
                        for p in tool["parked"].as_array().into_iter().flatten() {
                            add(
                                &mut placements,
                                p["part"].as_str().unwrap_or(""),
                                sum(n3(&p["translation_mm"]), a),
                                &p["hidden_keys"],
                                "tool",
                                slot as i64,
                                oid,
                                Some(group.as_str()),
                            );
                        }
                        // The carriage's half of the coupling, once, with the head.
                        if slot == 0 {
                            for p in tool["active"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|p| p["role"] == "shuttle")
                            {
                                add(
                                    &mut placements,
                                    p["part"].as_str().unwrap_or(""),
                                    sum(n3(&p["translation_mm"]), shift),
                                    &p["hidden_keys"],
                                    "shuttle",
                                    -1,
                                    oid,
                                    Some("head"),
                                );
                            }
                        }
                        for k in fit["stock_head_keys"].as_array().into_iter().flatten() {
                            if !hide.contains(k) {
                                hide.push(k.clone());
                            }
                        }
                    } else if slot == active {
                        // The tool on the gantry replaces the machine's own head, and moves with it.
                        for p in tool["active"].as_array().into_iter().flatten() {
                            add(
                                &mut placements,
                                p["part"].as_str().unwrap_or(""),
                                sum(n3(&p["translation_mm"]), shift),
                                &p["hidden_keys"],
                                p["role"].as_str().unwrap_or("tool"),
                                slot as i64,
                                oid,
                                Some("head"),
                            );
                        }
                        for k in fit["stock_head_keys"].as_array().into_iter().flatten() {
                            if !hide.contains(k) {
                                hide.push(k.clone());
                            }
                        }
                    } else {
                        for p in tool["parked"].as_array().into_iter().flatten() {
                            add(
                                &mut placements,
                                p["part"].as_str().unwrap_or(""),
                                sum(n3(&p["translation_mm"]), a),
                                &p["hidden_keys"],
                                "tool",
                                slot as i64,
                                oid,
                                None,
                            );
                        }
                    }
                }
            }
            "place" => {
                for p in fit["place"].as_array().into_iter().flatten() {
                    add(
                        &mut placements,
                        p["part"].as_str().unwrap_or(""),
                        n3(&p["translation_mm"]),
                        &p["hidden_keys"],
                        "addon",
                        -1,
                        oid,
                        None,
                    );
                }
                for k in fit["hide_machine_keys"].as_array().into_iter().flatten() {
                    if !hide.contains(k) {
                        hide.push(k.clone());
                    }
                }
            }
            _ => {}
        }
    }
    let mut missing: Vec<String> = placements
        .iter()
        .filter(|p| p["installed"] == false)
        .filter_map(|p| p["part"].as_str())
        .map(str::to_owned)
        .collect();
    missing.sort();
    missing.dedup();
    if swap && !missing.is_empty() {
        hide.clear();
    }
    json!({"machine":machine_id,"options":chosen,"placements":placements,"hide_machine_keys":hide,"missing":missing,"swapping":swap})
}

/// One tool of a tool bank on a machine, as the pose and the demo printer need it (polish WO-03).
#[derive(Clone, Debug, PartialEq)]
pub struct BankTool {
    /// The motion group that is this tool (`tool0`...).
    pub group: String,
    /// Where the tool is on the carriage compared with at its dock (the model's frame, millimetres), less the tool's own nozzle
    /// offset: a toolchanger's firmware makes up the difference between one tool's nozzle and the next, so the nozzle of
    /// whichever tool is active is on the printer's coordinates (the tool is rigid, so one number serves all its parts).
    pub rest_mm: [f64; 3],
    /// Where this tool's nozzle is compared with the machine's own nozzle when the tool is on the carriage at the pose
    /// (measured from the tool's file, `tools/printer-print-frames.rs`; the model's frame, millimetres).
    pub offset_mm: [f64; 3],
    /// The axis values that carry the head to the dock, inside the machine's travel: the head's motion read backwards.
    pub park: [f64; 3],
    /// The printer coordinates the head stands at to pick the tool up or put it down.
    pub dock: [f64; 3],
}

/// The tools of the tool bank chosen on a machine, in dock order, for a machine with registered motion. None when no
/// bank is chosen, the machine has no head motion or a tool does not give one rigid move.
pub fn bank_tools(machine_id: &str, selection: &Value) -> Option<Vec<BankTool>> {
    let item = selection
        .as_array()?
        .iter()
        .find(|i| i["option"].as_str().and_then(option).is_some_and(|o| o["attach"]["kind"] == "bank"))?;
    let o = option(item["option"].as_str()?)?;
    let fit = o["fits"].get(machine_id).filter(|f| f.is_object())?;
    let profile = profile(machine_id)?;
    let head = profile["motions"].get("head")?;
    let cal = Calibration::starting(profile);
    let tools: Vec<&str> = item["tools"].as_array()?.iter().filter_map(Value::as_str).collect();
    let (anchor, pitch, shift) = (
        n3(&fit["anchor_mm"]),
        fit["pitch_mm"].as_f64().unwrap_or(0.0),
        n3(&fit["head_shift_mm"]),
    );
    let mut out = Vec::new();
    for (slot, tid) in tools.iter().enumerate() {
        let tool = &o["attach"]["tools"][*tid];
        let a = [
            anchor[0] + (slot as f64 - (tools.len() as f64 - 1.0) / 2.0) * pitch,
            anchor[1],
            anchor[2],
        ];
        // A part the tool has both on the carriage and at the dock: the difference is the tool's move.
        let moves: Vec<[f64; 3]> = tool["active"]
            .as_array()?
            .iter()
            .filter_map(|x| {
                let y = tool["parked"].as_array()?.iter().find(|y| y["part"] == x["part"])?;
                let (xa, ya) = (n3(&x["translation_mm"]), n3(&y["translation_mm"]));
                Some([0, 1, 2].map(|c| xa[c] + shift[c] - ya[c] - a[c]))
            })
            .collect();
        let delta = *moves.first()?;
        if moves.iter().any(|m| (0..3).any(|c| (m[c] - delta[c]).abs() > 0.01)) {
            return None;
        }
        // The tool's nozzle, measured from its file: the file's display frame into the machine's frame, then to where the
        // tool's head part sits on the carriage. Its difference from the machine's nozzle is the tool's offset.
        let head_pl = tool["active"].as_array()?.iter().find(|x| x["role"] == "head")?;
        let file_tip = n3(&print_frames()["tools"][*tid]["tip_file_mm"]);
        let b = basis(machine_id);
        let tip: [f64; 3] = [0, 1, 2].map(|c| (0..3).map(|r| b[r][c].as_f64().unwrap_or(0.0) * file_tip[r]).sum::<f64>());
        let stock = n3(&print_frame(machine_id)?["nozzle_mm"]);
        let ht = n3(&head_pl["translation_mm"]);
        let offset: [f64; 3] = [0, 1, 2].map(|c| tip[c] + ht[c] + shift[c] - stock[c]);
        let rest: [f64; 3] = [0, 1, 2].map(|c| delta[c] - offset[c]);
        // The head's motion read backwards: the axis that makes frame component c move by -rest[c].
        let mut park = [0.0_f64; 3];
        for c in 0..3 {
            let (name, factor) = match &head[c] {
                Value::String(s) => (s.as_str(), 1.0),
                Value::Object(m) => m.iter().next().map_or(("", 0.0), |(k, f)| (k.as_str(), f.as_f64().unwrap_or(1.0))),
                _ => ("", 0.0),
            };
            if let (Some(i), true) = (NAMES.iter().position(|n| *n == name), factor != 0.0) {
                park[i] = -rest[c] / factor;
            }
        }
        let mut dock = [0.0_f64; 3];
        for i in 0..3 {
            dock[cal.source[i]] = (park[i] - cal.offset_mm[i]) / cal.sign[i];
        }
        out.push(BankTool { group: format!("tool{slot}"), rest_mm: rest, offset_mm: offset, park, dock });
    }
    (!out.is_empty()).then_some(out)
}

/// What the pose arithmetic needs of a machine, for the page to repeat at render rate: the travel, the motions and
/// the tools (no part lists). With a tool bank chosen, each tool is a motion group that follows the head with its
/// rest offset and waits at its dock when it is not the active one.
pub fn pose_spec(machine_id: &str, selection: Option<&Value>) -> Option<Value> {
    let p = profile(machine_id)?;
    let mut v = json!({"axes":p["axes"],"motions":p["motions"],"rest_offsets_mm":p["rest_offsets_mm"],"tools":p["tools"]});
    if let Some(tools) = selection.and_then(|s| bank_tools(machine_id, s)) {
        let head = p["motions"]["head"].clone();
        let mut list = Vec::new();
        for t in &tools {
            v["motions"][&t.group] = head.clone();
            v["rest_offsets_mm"][&t.group] = json!(t.rest_mm);
            list.push(json!({"group":t.group,"park":{"x":t.park[0],"y":t.park[1],"z":t.park[2]}}));
        }
        v["tools"] = Value::Array(list);
    }
    Some(v)
}

// ---------------------------------------------------------------------------------------------- fetching what a farm wants

/// Can this file be fetched on one press: its author publishes it at an https address (the project's pages or release)?
fn is_published(row: &Value) -> bool {
    let kind = row["location"]["kind"].as_str().unwrap_or("");
    upstream_url(row).is_some_and(|u| u.starts_with("https://") && (kind == "registry" || kind == "release_zip"))
}

/// What a farm's printers draw that is not in the library yet (polish WO-03, 2026-10-18): one row per machine model and per
/// file of a tool bank chosen for a printer, with its size, its licence and whether it can be fetched on one press, which is
/// when its licence lets a build carry it and its author publishes it at an https address. The rest is said plainly, with
/// why. `printers` is each printer's machine and the options chosen for it.
pub fn wanted(printers: &[(String, Option<Value>)], lib: &Library) -> Value {
    let mut seen: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for (id, _) in printers {
        if seen.contains(id) || lib.model_file(id).is_some() {
            continue;
        }
        seen.push(id.clone());
        let Some(m) = machine(id) else { continue };
        let class = class_of(m);
        let published = is_published(m);
        let open = may_bundle(class) && published;
        // The machine's name without the model's description ("Prusa CORE One L / Native CAD" is the Prusa CORE One L).
        let short = m["label"].as_str().unwrap_or(id).split(" / ").next().unwrap_or(id);
        // Finish-2 F09 (2026-10-19): the models Prusa shares under its own licence ship inside the build and are in the library from the first start, so
        // nothing here is fetched from a releases page that may not exist, and no notice has to be read before a download: it is read in the model's sheet and in About.
        let why = if open {
            Value::Null
        } else if is_bundled(id) {
            json!(format!("The {short} model ships inside Stella Scope and is put in your library when the service starts. It is not there yet: restart Stella Scope."))
        } else if !published {
            json!(format!("The {short} model comes from its maker's own files and is added by hand under Printer models."))
        } else {
            json!(format!("The {short} model is under {}: {}", m["licence"]["text"].as_str().unwrap_or("terms its author did not state"), class_words(class)))
        };
        rows.push(json!({
            "kind":"model","id":id,"label":m["label"],"bytes":model_bytes(m),
            "licence":{"class":class,"text":m["licence"]["text"],"author":m["licence"]["author"],"source":m["licence"]["source"],"words":class_words(class)},
            "open":open,"why":why,
        }));
    }
    // The files of a toolchanger or add-on chosen for a printer: only those its selection draws (a bank's other tools, and
    // their licences, are not asked for).
    for (id, selection) in printers {
        let Some(sel) = selection.as_ref().filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) else {
            continue;
        };
        let scene = scene_swapping(id, sel, lib);
        for pid in scene["missing"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if seen.iter().any(|x| x == pid) {
                continue;
            }
            seen.push(pid.to_owned());
            let Some(r) = part(pid) else { continue };
            let class = class_of(r);
            let open = may_bundle(class) && is_published(r);
            let label = r["label"].as_str().unwrap_or(pid);
            rows.push(json!({
                "kind":"part","id":pid,"label":label,"bytes":r["bytes"],
                "licence":{"class":class,"text":r["licence"]["text"],"author":r["licence"]["author"],"source":r["licence"]["source"],"words":class_words(class)},
                "open":open,
                "why":if open { Value::Null } else { json!(format!("{label} is under {}: {}", r["licence"]["text"].as_str().unwrap_or("terms its author did not state"), class_words(class))) },
            }));
        }
    }
    let open_bytes: u64 = rows.iter().filter(|r| r["open"] == true).filter_map(|r| r["bytes"].as_u64()).sum();
    let open_count = rows.iter().filter(|r| r["open"] == true).count();
    json!({"models":rows,"open_count":open_count,"open_bytes":open_bytes})
}

/// What one press of the fetch takes (polish WO-06 item 1): with no ids, every open model of `wanted`; with ids, exactly those that are open.
/// Never the open set on top of an id: a model's button once pulled every open model with it.
pub fn fetch_selection(wanted: &Value, asked: &[String]) -> Vec<String> {
    wanted["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            if asked.is_empty() {
                m["open"] == true
            } else {
                asked.iter().any(|a| m["id"] == a.as_str()) && m["open"] == true
            }
        })
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect()
}

/// What each printer's picture still waits for (F03, 2026-10-19): per printer, the ids of its machine model and of the option files its
/// choices draw that are not in the library. A printer whose files are all here is not listed. The wall matches these ids with the
/// status of the fetch to say, on each tile, whether its model is waiting, arriving, failed or not asked for yet. `printers` is each
/// printer's id, machine and chosen options.
pub fn needs(printers: &[(String, String, Option<Value>)], lib: &Library) -> Value {
    let mut out = Map::new();
    for (pid, model, selection) in printers {
        let mut ids: Vec<String> = Vec::new();
        if machine(model).is_some() && lib.model_file(model).is_none() {
            ids.push(model.clone());
        }
        if let Some(sel) = selection.as_ref().filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) {
            for p in scene_swapping(model, sel, lib)["missing"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !ids.iter().any(|x| x == p) {
                    ids.push(p.to_owned());
                }
            }
        }
        if !ids.is_empty() {
            out.insert(pid.clone(), json!(ids));
        }
    }
    Value::Object(out)
}

/// The open licences of what a farm would fetch, as one line of plain words and the facts behind it (F03, 2026-10-19; our rule for
/// the demo farm: one press shows the licences in one line, then fetches). Built from the catalogue, never typed: the licences and
/// their authors are the files' own, the size is the sum of what would be fetched, and a model that needs its maker's notice read
/// first is named, not fetched. `printers` is each printer's machine and chosen options.
pub fn licence_line(printers: &[(String, Option<Value>)]) -> Value {
    // Everything counts as missing: a library that is not there.
    let none = Library::new(std::env::temp_dir().join("stella-no-such-library"));
    let w = wanted(printers, &none);
    let rows: Vec<&Value> = w["models"].as_array().into_iter().flatten().collect();
    let mut by: Vec<(String, Vec<String>)> = Vec::new();
    for r in rows.iter().filter(|r| r["open"] == true) {
        let text = r["licence"]["text"].as_str().unwrap_or("terms its author did not state").to_owned();
        // "Copyright SOVOL and the original contributors" is SOVOL; a file whose row names nobody is credited to the owner of the repository it comes from.
        let who = match r["licence"]["author"].as_str().map(|a| a.trim_start_matches("Copyright ").split(" and the original").next().unwrap_or(a).trim()).filter(|a| !a.is_empty()) {
            Some(a) => a.to_owned(),
            None => r["licence"]["source"].as_str().and_then(|u| u.strip_prefix("https://github.com/")).and_then(|u| u.split('/').next()).filter(|o| !o.is_empty()).map_or_else(|| "its author".to_owned(), str::to_owned),
        };
        match by.iter_mut().find(|(t, _)| *t == text) {
            Some((_, v)) => {
                if !v.contains(&who) {
                    v.push(who);
                }
            }
            None => by.push((text, vec![who])),
        }
    }
    // Finish-2 F09: the machines of this farm whose CAD ships inside the build need no fetch; they are credited here by name so the line is the whole of it.
    let carried: Vec<Value> = printers
        .iter()
        .filter(|(id, _)| is_bundled(id))
        .filter_map(|(id, _)| machine(id))
        .map(|m| json!({"id":m["id"],"label":m["label"],"licence":m["licence"]["text"],"author":m["licence"]["author"]}))
        .collect::<Vec<_>>();
    let mut carried_unique: Vec<Value> = Vec::new();
    for c in carried {
        if !carried_unique.contains(&c) {
            carried_unique.push(c);
        }
    }
    let carried = carried_unique;
    let groups = by.iter().map(|(t, v)| format!("{t} ({})", v.join(", "))).collect::<Vec<_>>().join(" and ");
    let bytes = w["open_bytes"].as_u64().unwrap_or(0);
    let mut line = format!(
        "Open licences: {groups}. {} MB downloaded from the authors' own sites, each file checked against the checksum its author published (SHA-256).",
        bytes.div_ceil(1_000_000)
    );
    for n in &carried {
        let name = n["label"].as_str().unwrap_or("A model").split(" / ").next().unwrap_or("A model").trim_start_matches("Prusa ");
        let first = |v: &Value, d: &str| v.as_str().and_then(|t| t.split(" (").next()).filter(|t| !t.is_empty()).unwrap_or(d).to_owned();
        line.push_str(&format!(" The {name} model is inside Stella Scope (from {}, {}): nothing is downloaded for it.", first(&n["author"], "its maker"), first(&n["licence"], "its licence")));
    }
    if !carried.is_empty() {
        line.push_str(" Who made each model and under what terms is under About.");
    }
    json!({"line":line,"files":w["open_count"],"bytes":bytes,"licences":by.iter().map(|(t, v)| json!({"licence":t,"by":v})).collect::<Vec<_>>(),"carried":carried})
}

/// The size of a machine's model file as stored, from what the catalogue knows of it.
pub fn model_bytes(row: &Value) -> Option<u64> {
    row["location"]["mirror"]["bytes"]
        .as_u64()
        .or_else(|| row["pin"]["stored_bytes"].as_u64())
        .or_else(|| row["files"]["model.glb.gz"]["bytes"].as_u64())
        .or_else(|| row["bytes"].as_u64()) // a part of a tool bank carries its size directly
}

/// A reader that counts what passes through it, so a download can be watched while it runs.
struct Counting {
    inner: Box<dyn Read>,
    seen: Arc<AtomicU64>,
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.seen.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// How many files are fetched at once (F03, 2026-10-19). A farm's models are a dozen files and 170 MB; one after the other left the
/// connection idle while each file was unpacked and checked. Three at once keep the tiles filling in, without one fetch taking the
/// whole connection or the disk for itself.
const FETCH_WORKERS: usize = 3;

/// One file of a fetch, as a tile on the wall watches it.
struct Item {
    id: String,
    label: String,
    /// "model" (a machine's file) or "part" (a file of a tool bank or an add-on).
    kind: &'static str,
    bytes: u64,
    /// queued, fetching, done or failed.
    state: &'static str,
    got: Arc<AtomicU64>,
    error: Option<String>,
}

/// A fetch of several models in the background, watched by the page. Files are in the order they were asked for; the status lists
/// them in that order whichever worker finishes first, so a page that reads it twice sees the same order.
#[derive(Default)]
struct FetchJob {
    state: &'static str,
    items: Vec<Item>,
    source: String,
    started: Option<std::time::Instant>,
    ended: Option<std::time::Instant>,
}

fn jobs() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<FetchJob>>>> {
    static J: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<FetchJob>>>>> = OnceLock::new();
    J.get_or_init(Default::default)
}

fn job_for(lib: &Library) -> Arc<Mutex<FetchJob>> {
    jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(lib.root.clone())
        .or_default()
        .clone()
}

fn lock_job(j: &Mutex<FetchJob>) -> std::sync::MutexGuard<'_, FetchJob> {
    j.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why one file was not fetched, in the words a person on the wall reads (F03, 2026-10-19): a kind for the page to choose its button
/// by, and one sentence that says what happened and what to do. "offline" is no connection at all; "gone" and "refused" are the
/// author's site answering without the file; "changed" is a file whose SHA-256 is not the one pinned (it is not kept); "unpublished" is
/// a file Stella Scope's own releases are meant to hold and do not yet.
pub fn failure_words(error: &str) -> (&'static str, String) {
    let e = error.to_ascii_lowercase();
    if e.contains("no connection") {
        return ("offline", "No connection to the author's site. Check the internet, then try again.".to_owned());
    }
    if let Some(code) = e.split("answered ").nth(1).and_then(|r| r.get(..3)).filter(|c| c.bytes().all(|b| b.is_ascii_digit())) {
        return if code == "404" {
            ("gone", "The author's site no longer has this file (it answered 404).".to_owned())
        } else {
            ("refused", format!("The author's site answered {code}, not the file. Try again later."))
        };
    }
    if e.contains("not the model the author published") {
        return ("changed", "The file from the author's site is not the one expected (its SHA-256 differs), so it was not kept.".to_owned());
    }
    if e.contains("interrupted") {
        return ("interrupted", "The download was interrupted. Try again.".to_owned());
    }
    ("other", error.chars().take(200).collect())
}

/// Where the fetch stands: idle, running (what, how many bytes of how many, and each file's own progress) or done (what arrived
/// and what did not, and how long it took).
pub fn fetch_status(lib: &Library) -> Value {
    let j = job_for(lib);
    let j = lock_job(&j);
    let total: u64 = j.items.iter().map(|m| m.bytes).sum();
    let have = |m: &Item| match m.state {
        "done" | "failed" => m.bytes,
        "fetching" => m.got.load(Ordering::Relaxed).min(m.bytes.max(1)),
        _ => 0,
    };
    let items: Vec<Value> = j
        .items
        .iter()
        .map(|m| {
            let mut v = json!({"id":m.id,"label":m.label,"kind":m.kind,"bytes":m.bytes,"got":have(m),"state":m.state});
            if let Some(e) = &m.error {
                let (kind, plain) = failure_words(e);
                v["error"] = json!(e);
                v["failure"] = json!({"kind":kind,"plain":plain});
            }
            v
        })
        .collect();
    let of = |s: &'static str| j.items.iter().filter(move |m| m.state == s);
    json!({
        "state":if j.state.is_empty() { "idle" } else { j.state },
        "total_bytes":total,
        "done_bytes":j.items.iter().map(have).sum::<u64>(),
        "count":j.items.len(),
        "current":of("fetching").next().map(|m| json!({"id":m.id,"label":m.label})),
        "finished":of("done").map(|m| m.id.clone()).collect::<Vec<_>>(),
        "failed":of("failed").map(|m| json!({"id":m.id,"error":m.error})).collect::<Vec<_>>(),
        "items":items,
        "source":j.source,
        "elapsed_s":j.started.map(|t| (j.ended.unwrap_or_else(std::time::Instant::now) - t).as_secs_f64()),
    })
}

/// Fetch the open models in `ids` from `source` in the background, three at a time, smallest first (so the most tiles are
/// drawn soonest), each checked against its pinned SHA-256 as it arrives. `accepted` is the person's confirmation that they have read
/// the licences (shown first). A fetch already running is not started twice. Returns the status.
pub fn start_fetch(lib: &Library, ids: &[String], source: Source, accepted: bool) -> Result<Value> {
    // A machine's model or one file of a tool bank (the catalogue's part rows say `"kind":"part"`).
    let mut rows: Vec<Value> = ids.iter().filter_map(|id| machine(id).or_else(|| part(id))).cloned().collect();
    rows.sort_by_key(|r| model_bytes(r).unwrap_or(0));
    start_fetch_rows(lib, rows, source, accepted)
}

fn start_fetch_rows(lib: &Library, rows: Vec<Value>, source: Source, accepted: bool) -> Result<Value> {
    let job = job_for(lib);
    {
        let mut j = lock_job(&job);
        if j.state == "running" {
            drop(j);
            return Ok(fetch_status(lib));
        }
        for r in &rows {
            gate(r, accepted)?;
        }
        *j = FetchJob {
            state: "running",
            items: rows
                .iter()
                .map(|r| Item {
                    id: r["id"].as_str().unwrap_or("").to_owned(),
                    label: r["label"].as_str().unwrap_or("").to_owned(),
                    kind: if r["kind"] == "part" { "part" } else { "model" },
                    bytes: model_bytes(r).unwrap_or(0),
                    state: "queued",
                    got: Arc::new(AtomicU64::new(0)),
                    error: None,
                })
                .collect(),
            source: source.label(),
            started: Some(std::time::Instant::now()),
            ended: None,
        };
    }
    let own = Library::with_carried(lib.root.clone(), lib.carried.clone());
    std::thread::spawn(move || {
        let job = job_for(&own);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..FETCH_WORKERS.min(rows.len()) {
                scope.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(row) = rows.get(i) else { break };
                    let (id, seen) = {
                        let mut j = lock_job(&job);
                        j.items[i].state = "fetching";
                        (j.items[i].id.clone(), j.items[i].got.clone())
                    };
                    let result = source.open(row).and_then(|r| {
                        let mut reader = Counting { inner: r, seen: seen.clone() };
                        if row["kind"] == "part" {
                            own.store_into(own.part_dir(&id)?, row, &mut reader, &source.label())
                        } else {
                            own.store(row, &mut reader, &source.label())
                        }
                    });
                    let mut j = lock_job(&job);
                    match result {
                        Ok(_) => j.items[i].state = "done",
                        Err(e) => {
                            j.items[i].state = "failed";
                            j.items[i].error = Some(e);
                        }
                    }
                });
            }
        });
        let mut j = lock_job(&job);
        j.state = "done";
        j.ended = Some(std::time::Instant::now());
    });
    Ok(fetch_status(lib))
}

// ---------------------------------------------------------------------------------------------- motion

/// How a printer's reported position becomes the model's axis values (millimetres from the CAD pose).
/// `axis value = sign * reported[source] + offset`, then held inside the model's registered travel.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub offset_mm: [f64; 3],
    pub sign: [f64; 3],
    pub source: [usize; 3],
}

impl Calibration {
    /// A start that reads right for most printers: reported 0 is the low end of the model's travel.
    /// A CAD pose never equals the printer's homing coordinates, so this is a starting point to match.
    pub fn starting(profile: &Value) -> Self {
        // A profile made from the maker's CAD can carry its measured calibration (CORE One L).
        if let Ok(c) = Self::from_value(&profile["calibration"]) {
            return c;
        }
        let lo = |a: &str| profile["axes"][a][0].as_f64().unwrap_or(0.0);
        Self {
            offset_mm: [lo("x"), lo("y"), lo("z")],
            sign: [1.0; 3],
            source: [0, 1, 2],
        }
    }

    pub fn from_value(v: &Value) -> Result<Self> {
        let nums = |k: &str| -> Result<[f64; 3]> {
            let a = v[k]
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| format!("{k} must be three numbers"))?;
            let mut o = [0.0; 3];
            for (i, n) in a.iter().enumerate() {
                o[i] = n
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| format!("{k} must be three numbers"))?;
            }
            Ok(o)
        };
        let offset_mm = nums("offset_mm")?;
        let sign = nums("sign")?;
        let src = nums("source")?;
        if offset_mm.iter().any(|x| x.abs() > 5000.0) {
            return Err("an offset is more than 5 m, which cannot be right".into());
        }
        if sign.iter().any(|s| *s != 1.0 && *s != -1.0) {
            return Err("each sign must be 1 or -1".into());
        }
        if src.iter().any(|s| ![0.0, 1.0, 2.0].contains(s)) {
            return Err("each source must be 0 (X), 1 (Y) or 2 (Z)".into());
        }
        Ok(Self {
            offset_mm,
            sign,
            source: [src[0] as usize, src[1] as usize, src[2] as usize],
        })
    }

    pub fn to_value(&self) -> Value {
        json!({"offset_mm":self.offset_mm,"sign":self.sign,"source":self.source})
    }
}

const NAMES: [&str; 3] = ["x", "y", "z"];

/// The model's pose for a reported position: the three axis values, which of them hit the end of the
/// registered travel, and the offset every motion group takes from its CAD pose.
pub fn pose(profile: &Value, cal: &Calibration, position: [Option<f64>; 3]) -> Value {
    pose_for_tool(profile, cal, position, None)
}

/// The same pose for a machine with more than one tool (IDEX, toolchanger, polish WO-02). The profile lists
/// `tools`: for each, the motion group that is that tool and where it waits (`park`, axis values at its
/// dock). The active tool follows the position; every other tool sits at its park pose. A profile with no
/// `tools`, or no active tool given, is posed exactly as before.
pub fn pose_for_tool(
    profile: &Value,
    cal: &Calibration,
    position: [Option<f64>; 3],
    tool: Option<usize>,
) -> Value {
    let mut axes = [0.0_f64; 3];
    let mut clamped = Vec::new();
    for i in 0..3 {
        let lo = profile["axes"][NAMES[i]][0].as_f64().unwrap_or(0.0);
        let hi = profile["axes"][NAMES[i]][1].as_f64().unwrap_or(0.0);
        let reported = position[cal.source[i]].unwrap_or(0.0);
        let raw = cal.sign[i] * reported + cal.offset_mm[i];
        let held = raw.clamp(lo.min(hi), lo.max(hi));
        if (held - raw).abs() > 1e-9 {
            clamped.push(NAMES[i]);
        }
        axes[i] = held;
    }
    let component = |spec: &Value, axes: &[f64; 3]| -> f64 {
        match spec {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(s) => NAMES.iter().position(|n| n == s).map_or(0.0, |i| axes[i]),
            Value::Object(o) => o.iter().next().map_or(0.0, |(k, f)| {
                NAMES.iter().position(|n| n == k).map_or(0.0, |i| axes[i])
                    * f.as_f64().unwrap_or(1.0)
            }),
            _ => 0.0,
        }
    };
    let mut groups = Map::new();
    if let Some(m) = profile["motions"].as_object() {
        for (name, spec) in m {
            let rest = &profile["rest_offsets_mm"][name];
            let v: Vec<f64> = (0..3)
                .map(|c| component(&spec[c], &axes) + rest[c].as_f64().unwrap_or(0.0))
                .collect();
            groups.insert(name.clone(), json!(v));
        }
    }
    if let (Some(active), Some(tools)) = (tool, profile["tools"].as_array()) {
        for t in tools
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != active)
            .map(|(_, t)| t)
        {
            let (Some(name), Some(park)) = (t["group"].as_str(), t["park"].as_object()) else {
                continue;
            };
            let Some(spec) = profile["motions"].get(name) else {
                continue;
            };
            // Where this tool's axes stand at its dock, held inside the travel like any other pose.
            let mut at = axes;
            for k in 0..3 {
                if let Some(v) = park.get(NAMES[k]).and_then(Value::as_f64) {
                    let lo = profile["axes"][NAMES[k]][0].as_f64().unwrap_or(0.0);
                    let hi = profile["axes"][NAMES[k]][1].as_f64().unwrap_or(0.0);
                    at[k] = v.clamp(lo.min(hi), lo.max(hi));
                }
            }
            let rest = &profile["rest_offsets_mm"][name];
            let v: Vec<f64> = (0..3)
                .map(|c| component(&spec[c], &at) + rest[c].as_f64().unwrap_or(0.0))
                .collect();
            groups.insert(name.to_owned(), json!(v));
        }
    }
    json!({
        "axes":{"x":axes[0],"y":axes[1],"z":axes[2]},
        "clamped":clamped,
        "known":position.iter().all(Option::is_some),
        "tool":tool,
        "groups":groups,
    })
}

/// "The model is right here": the calibration that gives the wanted axis values at the position the printer
/// reports now. Only the offsets change; the signs and sources are the person's.
pub fn matched(
    profile: &Value,
    cal: &Calibration,
    position: [Option<f64>; 3],
    wanted: [f64; 3],
) -> Calibration {
    let mut out = cal.clone();
    for i in 0..3 {
        let lo = profile["axes"][NAMES[i]][0].as_f64().unwrap_or(0.0);
        let hi = profile["axes"][NAMES[i]][1].as_f64().unwrap_or(0.0);
        let w = wanted[i].clamp(lo.min(hi), lo.max(hi));
        out.offset_mm[i] = w - cal.sign[i] * position[cal.source[i]].unwrap_or(0.0);
    }
    out
}

/// The path a stored model file is served from, kept out of the page's reach.
pub fn library_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("printer-models")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "stella-models-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    // `gzip -9n` of "glTF fixture model\n": the one file the install tests use as a model.
    fn fixture() -> (Vec<u8>, String, String) {
        // A stored deflate block, so the file is exactly what the test says it is.
        let body = b"glTF fixture model for the install tests\n";
        let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1];
        gz.extend_from_slice(&(body.len() as u16).to_le_bytes());
        gz.extend_from_slice(&(!(body.len() as u16)).to_le_bytes());
        gz.extend_from_slice(body);
        let mut crc = 0xffff_ffff_u32;
        for &b in body.iter() {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    0xedb8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        gz.extend_from_slice(&(!crc).to_le_bytes());
        gz.extend_from_slice(&(body.len() as u32).to_le_bytes());
        let stored = crate::sha256::hex(&gz);
        let decoded = crate::sha256::hex(body);
        (gz, stored, decoded)
    }

    fn row(class: &str, stored: &str, decoded: &str) -> Value {
        json!({"id":"fixture_machine","label":"Fixture machine","licence":{"class":class,"text":"CC-BY-NC-4.0","author":"A. Maker","source":"https://example.invalid/m"},
               "pin":{"stored_model_sha256":stored,"model_sha256":decoded},
               "location":{"kind":"registry","upstream":"https://psych0h3ad.github.io/3d-print-rig-community-models/fixture/model.glb.gz"}})
    }

    #[test]
    fn the_catalogue_maps_all_67_machines_and_every_one_is_pinned() {
        let ms = catalogue()["machines"].as_array().unwrap();
        // Finish-2 F09 (2026-10-19): the CORE One, the MK3S+ and the MINI joined the CORE One L (69 became 72): the machines whose converted CAD the build carries.
        assert_eq!(
            ms.len(),
            72,
            "67 from 3D Print Rig, the E3NG toolchanger from the rig's bundle, and the CORE One L, CORE One, MK3S+ and MINI whose CAD the build carries"
        );
        for m in ms {
            let (s, d) = pin(m);
            assert_eq!((s.len(), d.len()), (64, 64), "{} has both pins", m["id"]);
            assert!(
                [
                    "gpl",
                    "mit",
                    "cc-by",
                    "cc-by-sa",
                    "nc",
                    "mixed",
                    "ocl",
                    "unlicensed"
                ]
                .contains(&class_of(m)),
                "{}",
                m["id"]
            );
            assert!(
                !m["licence"]["text"].as_str().unwrap_or("").is_empty(),
                "{} states a licence or says it has none",
                m["id"]
            );
        }
        // Registered motion is for machines that are in the catalogue and has the three travel ranges.
        for (id, p) in motion_table()["profiles"].as_object().unwrap() {
            assert!(machine(id).is_some(), "{id}");
            assert!(
                p["axes"]["x"].is_array() && p["axes"]["y"].is_array() && p["axes"]["z"].is_array()
            );
            assert_eq!(
                p["model_sha256"],
                machine(id).unwrap()["pin"]["model_sha256"],
                "{id}: the motion is registered for exactly this model"
            );
        }
    }

    #[test]
    fn only_licence_classes_that_allow_it_may_travel_with_a_build() {
        // Finish-2 F09 (2026-10-19): "ocl" is in the first list now (it was in the second). Prusa Research's Open Community License lets the CAD be
        // converted and passed on under the same licence, free of charge, with its notice: the build carries Prusa's CORE One and CORE One L that way. A
        // machine with no model ("none") carries nothing.
        for c in ["gpl", "mit", "cc-by", "cc-by-sa", "cc0", "ocl"] {
            assert!(may_bundle(c), "{c}");
        }
        for c in ["nc", "mixed", "unlicensed", "unknown", "none", ""] {
            assert!(!may_bundle(c), "{c}");
        }
        // What this build really embeds: a thumbnail for a machine only if its class allows it.
        for (id, _) in thumbs()["thumbs"].as_object().unwrap() {
            assert!(
                may_bundle(class_of(machine(id).unwrap())),
                "{id} is embedded but its licence does not allow it"
            );
        }
        // Rat Rig, Annex, VzBot, SnakeOil, Mercury, FYSETC and the SIBOOR machines with no licence stated are kept out
        // (the SIBOOR S-BOOM carries a GPL notice and may travel).
        for m in catalogue()["machines"].as_array().unwrap() {
            let id = m["id"].as_str().unwrap();
            let kept_out = [
                "ratrig_",
                "annex_",
                "vzbot_",
                "snakeoil_",
                "mercury_",
                "fysetc_",
                "siboor_trident_",
                "siboor_v24_",
            ]
            .iter()
            .any(|p| id.starts_with(p));
            if kept_out {
                assert!(!may_bundle(class_of(m)), "{id}");
            }
        }
    }

    #[test]
    fn a_file_with_the_right_hash_installs_and_a_changed_one_is_refused_and_not_kept() {
        let (gz, stored, decoded) = fixture();
        let root = scratch("install");
        let lib = Library::new(root.clone());
        let r = row("gpl", &stored, &decoded);
        // The right file.
        let rec = lib
            .install_file_row(&r, &mut gz.as_slice(), "test", false)
            .unwrap();
        assert_eq!(rec["decoded_sha256"], decoded.as_str());
        assert_eq!(rec["licence"]["author"], "A. Maker");
        assert!(root.join("fixture_machine/model.glb.gz").is_file());
        assert!(
            !fs::read_dir(root.join("fixture_machine"))
                .unwrap()
                .any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".incoming")),
            "no temporary file is left"
        );
        // A different file (one byte of the model changed, packed again).
        let root2 = scratch("refuse");
        let lib2 = Library::new(root2.clone());
        let mut bad = gz.clone();
        let n = bad.len();
        bad[20] ^= 0x01; // inside the stored body
        let crc_fix = {
            let body = &bad[15..n - 8];
            let mut crc = 0xffff_ffff_u32;
            for &b in body {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        0xedb8_8320 ^ (crc >> 1)
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        };
        bad[n - 8..n - 4].copy_from_slice(&crc_fix.to_le_bytes());
        let e = lib2
            .install_file_row(&r, &mut bad.as_slice(), "test", false)
            .unwrap_err();
        assert!(
            e.contains("not the model the author published") && e.contains("Nothing was installed"),
            "{e}"
        );
        assert!(lib2.record("fixture_machine").is_none());
        assert!(
            !root2.join("fixture_machine").exists(),
            "a refused install leaves no folder"
        );
        // Not a model at all.
        assert!(lib2
            .install_file_row(&r, &mut &b"<html>404</html>"[..], "test", false)
            .is_err());
        // The same bytes with a different pin: refused.
        let wrong = row("gpl", &"0".repeat(64), &"1".repeat(64));
        assert!(lib2
            .install_file_row(&wrong, &mut gz.as_slice(), "test", false)
            .is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(root2);
    }

    #[test]
    fn a_model_a_build_may_not_carry_needs_the_licence_confirmed() {
        let (gz, stored, decoded) = fixture();
        let root = scratch("gate");
        let lib = Library::new(root.clone());
        let nc = row("nc", &stored, &decoded);
        let e = lib
            .install_file_row(&nc, &mut gz.as_slice(), "test", false)
            .unwrap_err();
        assert!(e.contains("CC-BY-NC-4.0") && e.contains("confirm"), "{e}");
        assert!(
            lib.record("fixture_machine").is_none(),
            "nothing was read or kept before the person agreed"
        );
        let rec = lib
            .install_file_row(&nc, &mut gz.as_slice(), "test", true)
            .unwrap();
        assert_eq!(rec["licence"]["class"], "nc");
        // An open licence does not ask.
        assert!(Library::new(scratch("gate2"))
            .install_file_row(
                &row("gpl", &stored, &decoded),
                &mut gz.as_slice(),
                "t",
                false
            )
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_source_finds_the_file_where_the_pages_keep_it() {
        let (gz, stored, decoded) = fixture();
        let src = scratch("src");
        fs::create_dir_all(src.join("fixture")).unwrap();
        fs::write(src.join("fixture/model.glb.gz"), &gz).unwrap();
        let lib = Library::new(scratch("srcinst"));
        let r = row("gpl", &stored, &decoded);
        let rec = lib
            .install_row(&r, &Source::parse(src.to_str().unwrap()).unwrap(), false)
            .unwrap();
        assert!(rec["origin"].as_str().unwrap().starts_with("folder "));
        // The same under a checkout's site/ folder.
        let src2 = scratch("src2");
        fs::create_dir_all(src2.join("site/fixture")).unwrap();
        fs::write(src2.join("site/fixture/model.glb.gz"), &gz).unwrap();
        assert!(Library::new(scratch("srcinst2"))
            .install_row(&r, &Source::parse(src2.to_str().unwrap()).unwrap(), false)
            .is_ok());
        // An empty folder says what it looked for.
        let e = Library::new(scratch("srcinst3"))
            .install_row(
                &r,
                &Source::parse(scratch("empty").to_str().unwrap()).unwrap(),
                false,
            )
            .unwrap_err();
        assert!(e.contains("does not hold this model"), "{e}");
    }

    #[test]
    fn sources_are_named_plainly_and_https_is_explained() {
        // WO-13: secure downloads work (ureq rustls); the pinned SHA-256 is checked before use.
        assert_eq!(
            Source::parse("https://example.com/x").unwrap(),
            Source::Http("https://example.com/x".into())
        );
        assert!(Source::parse("relative/dir")
            .unwrap_err()
            .contains("full path"));
        assert!(Source::parse("").is_err());
        assert_eq!(
            Source::parse("http://192.168.1.5:8080/models/").unwrap(),
            Source::Http("http://192.168.1.5:8080/models".into())
        );
    }

    #[test]
    fn calibration_math_offsets_signs_sources_and_the_ends_of_travel() {
        let p = json!({"axes":{"x":[0,115],"y":[-90,30],"z":[-110,0]},
            "motions":{"head":["x","y",0],"gantry":[0,"y",0],"bed":[0,0,{"z":-1}],"fixed":[0,0,0]},
            "rest_offsets_mm":{"bed":[0,0,-4]}});
        let c = Calibration::starting(&p);
        assert_eq!(c.offset_mm, [0.0, -90.0, -110.0]);
        // Reported 60, 60, 55: the model reads 60, -30, -55.
        let r = pose(&p, &c, [Some(60.0), Some(60.0), Some(55.0)]);
        assert_eq!(r["axes"], json!({"x":60.0,"y":-30.0,"z":-55.0}));
        assert_eq!(r["groups"]["head"], json!([60.0, -30.0, 0.0]));
        assert_eq!(r["groups"]["gantry"], json!([0.0, -30.0, 0.0]));
        // The bed takes -z (the sign in the registration) and its 4 mm rest offset.
        assert_eq!(r["groups"]["bed"], json!([0.0, 0.0, 55.0 - 4.0]));
        assert_eq!(r["groups"]["fixed"], json!([0.0, 0.0, 0.0]));
        assert_eq!(r["clamped"], json!([]));
        // Past the end of travel: held, and said.
        let r = pose(&p, &c, [Some(500.0), Some(0.0), Some(0.0)]);
        assert_eq!(r["axes"]["x"], 115.0);
        assert_eq!(r["clamped"], json!(["x"]));
        // A flipped axis and a swapped source.
        let mut flipped = c.clone();
        flipped.sign[1] = -1.0;
        flipped.offset_mm[1] = 30.0;
        assert_eq!(
            pose(&p, &flipped, [Some(0.0), Some(120.0), Some(0.0)])["axes"]["y"],
            -90.0
        );
        let mut swapped = c.clone();
        swapped.source = [1, 0, 2];
        assert_eq!(
            pose(&p, &swapped, [Some(10.0), Some(40.0), Some(0.0)])["axes"]["x"],
            40.0
        );
        // No position yet: not known, nothing drawn as if it were.
        assert_eq!(pose(&p, &c, [None, None, None])["known"], false);
    }

    #[test]
    fn matching_sets_the_offset_so_the_model_reads_what_the_person_sees() {
        let p =
            json!({"axes":{"x":[-80,80],"y":[-80,80],"z":[0,200]},"motions":{"head":["x","y",0]}});
        let c = Calibration::starting(&p);
        // The printer reports (120, 60, 5) and the model's head is at (10, -20, 0) in the CAD.
        let m = matched(
            &p,
            &c,
            [Some(120.0), Some(60.0), Some(5.0)],
            [10.0, -20.0, 0.0],
        );
        let r = pose(&p, &m, [Some(120.0), Some(60.0), Some(5.0)]);
        assert_eq!(r["axes"], json!({"x":10.0,"y":-20.0,"z":0.0}));
        // Moving 30 mm in X moves the model 30 mm in X.
        let r = pose(&p, &m, [Some(150.0), Some(60.0), Some(5.0)]);
        assert_eq!(r["axes"]["x"], 40.0);
        // With the Y axis reversed the offset follows.
        let mut rev = c.clone();
        rev.sign[1] = -1.0;
        let m = matched(
            &p,
            &rev,
            [Some(0.0), Some(60.0), Some(0.0)],
            [0.0, 20.0, 0.0],
        );
        assert_eq!(
            pose(&p, &m, [Some(0.0), Some(60.0), Some(0.0)])["axes"]["y"],
            20.0
        );
        assert_eq!(
            pose(&p, &m, [Some(0.0), Some(70.0), Some(0.0)])["axes"]["y"],
            10.0
        );
    }

    #[test]
    fn a_calibration_that_makes_no_sense_is_refused() {
        let ok = json!({"offset_mm":[0,0,0],"sign":[1,1,-1],"source":[0,1,2]});
        assert!(Calibration::from_value(&ok).is_ok());
        for bad in [
            json!({"offset_mm":[0,0],"sign":[1,1,1],"source":[0,1,2]}),
            json!({"offset_mm":[0,0,9e9],"sign":[1,1,1],"source":[0,1,2]}),
            json!({"offset_mm":[0,0,0],"sign":[2,1,1],"source":[0,1,2]}),
            json!({"offset_mm":[0,0,0],"sign":[1,1,1],"source":[0,1,3]}),
        ] {
            assert!(Calibration::from_value(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn real_registered_motion_moves_the_head_with_x() {
        let p = profile("sovol_sv08_350").expect("SV08 has registered motion");
        let c = Calibration::starting(p);
        let a = pose(p, &c, [Some(0.0), Some(0.0), Some(0.0)]);
        let b = pose(p, &c, [Some(100.0), Some(0.0), Some(0.0)]);
        let dx =
            b["groups"]["head"][0].as_f64().unwrap() - a["groups"]["head"][0].as_f64().unwrap();
        assert!(
            (dx - 100.0).abs() < 1e-9,
            "head follows X mm for mm, got {dx}"
        );
        assert_eq!(b["groups"]["fixed"], json!([0.0, 0.0, 0.0]));
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    /// 2026-10-13: the CORE One L from Prusa's own CAD sits beside the 3D Print Rig catalogue, and its
    /// measured calibration puts X0 Y0 at the sheet's front-left. (It was "never shipped (OCL)"; finish-2 F09,
    /// 2026-10-19: Prusa's licence lets the converted CAD be passed on under the same licence, so the build carries it.)
    #[test]
    fn the_core_one_l_is_in_the_catalogue_with_its_own_calibration() {
        let row = machine("prusa_core_one_l").expect("the CORE One L is listed");
        assert_eq!(class_of(row), "ocl");
        assert!(may_bundle("ocl") && is_bundled("prusa_core_one_l"));
        let p = profile("prusa_core_one_l").expect("it has motion");
        let cal = Calibration::starting(p);
        assert_eq!(cal.sign, [1.0, -1.0, 1.0]);
        // front-left at Z0: the head over the sheet's front-left corner, the bed up at the nozzle
        let front = pose(p, &cal, [Some(0.0), Some(0.0), Some(0.0)]);
        assert_eq!(front["groups"]["head"], json!([315.5, -230.0, 0.0]));
        assert_eq!(front["groups"]["bed"], json!([0.0, 0.0, 21.0]));
        // Y grows toward the back (CAD -X), Z drops the bed
        let back = pose(p, &cal, [Some(0.0), Some(300.0), Some(100.0)]);
        assert_eq!(back["groups"]["gantry"], json!([15.5, 0.0, 0.0]));
        assert_eq!(back["groups"]["bed"], json!([0.0, 0.0, -79.0]));
        assert!(
            catalogue()["machines"].as_array().unwrap().len() > 60,
            "the 3D Print Rig rows are still there"
        );
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "stella-options-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn hex64(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Polish WO-01 (2026-10-18): the catalogue lists the rig's options, each with a licence class and files that are
    /// all pinned by both SHA-256 values the installer checks.
    #[test]
    fn every_option_has_a_licence_class_and_every_part_a_pinned_hash() {
        let classes = [
            "gpl",
            "mit",
            "cc-by",
            "cc-by-sa",
            "cc0",
            "nc",
            "mixed",
            "ocl",
            "unlicensed",
        ];
        assert!(
            options().len() > 150,
            "the toolchangers, toolheads and add-ons are listed ({})",
            options().len()
        );
        let strict = |cs: Vec<&str>| -> &str {
            [
                "unlicensed",
                "mixed",
                "nc",
                "ocl",
                "cc-by-sa",
                "gpl",
                "cc-by",
                "mit",
                "cc0",
            ]
            .into_iter()
            .find(|c| cs.contains(c))
            .unwrap()
        };
        for o in options() {
            let id = o["id"].as_str().unwrap();
            assert!(classes.contains(&class_of(o)), "{id}");
            assert!(
                !o["licence"]["text"].as_str().unwrap_or("").is_empty(),
                "{id} states a licence or says it has none"
            );
            assert!(
                ["toolchanger", "toolhead", "addon"].contains(&o["kind"].as_str().unwrap()),
                "{id}"
            );
            let ps = part_ids(o);
            assert!(!ps.is_empty(), "{id} has files");
            let mut cs = Vec::new();
            for p in &ps {
                let row = part(p)
                    .unwrap_or_else(|| panic!("{id} names {p}, which is not in the catalogue"));
                let (s, d) = pin(row);
                assert!(hex64(&s) && hex64(&d), "{p} is pinned by both hashes");
                assert!(classes.contains(&class_of(row)), "{p}");
                assert!(
                    row["location"]["upstream"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("https://"),
                    "{p} says where its author publishes it"
                );
                cs.push(class_of(row));
            }
            assert_eq!(
                class_of(o),
                strict(cs),
                "{id}: an option carries the strictest class of its files"
            );
        }
        // Every file belongs to some option, and every machine's list of options is the options that list the machine.
        let used: std::collections::BTreeSet<String> =
            options().iter().flat_map(part_ids).collect();
        for p in catalogue()["parts"].as_object().unwrap().keys() {
            assert!(used.contains(p), "{p} is in no option");
        }
        for m in catalogue()["machines"].as_array().unwrap() {
            let id = m["id"].as_str().unwrap();
            let mut listed: Vec<&str> = options()
                .iter()
                .filter(|o| o["fits"].get(id).is_some())
                .filter_map(|o| o["id"].as_str())
                .collect();
            let mut said: Vec<&str> = m["fits"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            listed.sort();
            said.sort();
            if m["fits"].is_array() {
                assert_eq!(
                    listed, said,
                    "{id}: the machine and the options agree on what fits"
                );
            }
        }
        for o in options() {
            for m in o["fits"].as_object().unwrap().keys() {
                assert!(
                    machine(m).is_some(),
                    "{} fits {m}, which is not a machine in the catalogue",
                    o["id"]
                );
            }
        }
    }

    #[test]
    fn a_non_commercial_or_unlicensed_option_is_never_marked_bundled() {
        let lib = Library::new(scratch("bundled"));
        let (mut open, mut shut) = (0, 0);
        for o in options() {
            let v = option_view(o, &lib);
            let c = class_of(o);
            assert_eq!(v["bundled"], may_bundle(c), "{}", o["id"]);
            if matches!(c, "nc" | "mixed" | "unlicensed" | "ocl") {
                assert_eq!(v["bundled"], false, "{} is {c}", o["id"]);
                shut += 1;
            } else {
                open += 1;
            }
            // Nothing of an option travels in the build: the catalogue holds facts and addresses, and the only
            // pictures embedded belong to machines.
            assert!(o["parts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| thumbs()["thumbs"].get(p.as_str().unwrap()).is_none()));
        }
        assert!(
            open > 20 && shut > 20,
            "both kinds are in the catalogue ({open} open, {shut} kept out)"
        );
        // The StealthChanger bank carries the Xol head (CC BY-NC-SA by its authors), so the bank is kept out as a whole.
        let bank = option("stealthchanger_bank").unwrap();
        assert_eq!(class_of(bank), "nc");
        assert!(!option_view(bank, &lib)["bundled"].as_bool().unwrap());
        // The same bank refuses to install until the licence is confirmed, and says which licence.
        let e = lib
            .install_option("stealthchanger_bank", &Source::Dir(scratch("empty")), false)
            .unwrap_err();
        assert!(e.contains("CC BY-NC-SA") && e.contains("confirm"), "{e}");
        assert!(
            !lib.root.join("parts").exists(),
            "nothing was read or kept before the person agreed"
        );
    }

    #[test]
    fn an_option_is_refused_on_a_machine_it_does_not_fit() {
        let ok = |m: &str, sel: Value| check_selection(m, &sel);
        // A V2.4 takes the StealthChanger bank, a Trident does not (its bed meets the dock at the nozzle plane).
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank"}])
        )
        .is_ok());
        let e = ok(
            "voron_trident_300",
            json!([{"option":"stealthchanger_bank"}]),
        )
        .unwrap_err();
        assert!(e.contains("does not fit") && e.contains("Trident"), "{e}");
        // A machine with no registered place for it.
        assert!(
            ok("sovol_sv08_350", json!([{"option":"stealthchanger_bank"}]))
                .unwrap_err()
                .contains("does not fit")
        );
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"nothing_like_it"}])
        )
        .unwrap_err()
        .contains("not an option"));
        // Capacity: three docks on a 250, four on a 300; each tool must be one the bank takes; the active one is a dock.
        let tools = |n: usize| {
            json!((0..n)
                .map(|_| "xol_sherpa_mini_rapido2_uhf")
                .collect::<Vec<_>>())
        };
        assert!(ok(
            "voron_v24_250_printed",
            json!([{"option":"stealthchanger_bank","tools":tools(3)}])
        )
        .is_ok());
        assert!(ok(
            "voron_v24_250_printed",
            json!([{"option":"stealthchanger_bank","tools":tools(4)}])
        )
        .unwrap_err()
        .contains("1 to 3"));
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank","tools":tools(4)}])
        )
        .is_ok());
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank","tools":["a_tool_nobody_made"]}])
        )
        .unwrap_err()
        .contains("not a tool"));
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank","tools":tools(2),"active":2}])
        )
        .unwrap_err()
        .contains("one of the 2 docks"));
        for bad in [json!(-1), json!(0.5), json!("1")] {
            assert!(
                ok(
                    "voron_v24_300_printed",
                    json!([{"option":"stealthchanger_bank","tools":tools(2),"active":bad}])
                )
                .unwrap_err()
                .contains("dock number"),
                "{bad}"
            );
        }
        // One bank per printer, one version of a part, one choice once.
        assert!(ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank"},{"option":"indx_bank"}])
        )
        .unwrap_err()
        .contains("one tool bank"));
        assert!(ok(
            "voron_trident_300",
            json!([{"option":"stealth_skirts_cheese_grater"},{"option":"stealth_skirts_honeycomb"}])
        )
        .unwrap_err()
        .contains("two versions"));
        assert!(ok(
            "voron_trident_300",
            json!([{"option":"sturdy_handles_m5"},{"option":"sturdy_handles_m5"}])
        )
        .unwrap_err()
        .contains("twice"));
        assert!(ok(
            "voron_trident_300",
            json!([{"option":"sturdy_handles_m5"},{"option":"stealth_skirts_honeycomb"}])
        )
        .is_ok());
        // Listed for the machine but not drawn on a printer by Stella Scope: said plainly.
        let e = ok("voron_v24_300_printed", json!([{"option":"head_xol"}])).unwrap_err();
        assert!(e.contains("does not draw it"), "{e}");
        // Nothing chosen is fine, and the defaults are filled in.
        assert_eq!(ok("voron_v24_300_printed", Value::Null).unwrap(), json!([]));
        let d = ok(
            "voron_v24_300_printed",
            json!([{"option":"stealthchanger_bank"}]),
        )
        .unwrap();
        assert_eq!(
            d[0]["tools"],
            json!(["xol_sherpa_mini_rapido2_uhf"]),
            "the first tool the rig offers"
        );
        assert_eq!(d[0]["active"], 0);
    }

    /// The bank is placed as the rig's own `bankPlan` and `installedHeadPlan` place it. These numbers are what that
    /// code returns (node, the rig's changer-bank-model.mjs on its TOOLCHANGER_BANK.json and
    /// MACHINE_HEAD_REGISTRATIONS.json): see docs/work-orders/polish-2026-10-18/evidence/WO-01/bank-truth.mjs.
    #[test]
    fn the_stealthchanger_bank_is_placed_where_the_rig_places_it() {
        let lib = Library::new(scratch("scene"));
        let (xol, jw) = (
            "xol_sherpa_mini_rapido2_uhf",
            "jabberwocky_jabberwocky_drive_conch",
        );
        let sel = check_selection(
            "voron_v24_300_printed",
            &json!([{"option":"stealthchanger_bank","tools":[xol, jw, xol, jw],"active":0}]),
        )
        .unwrap();
        let s = scene("voron_v24_300_printed", &sel, &lib);
        let find = |part: &str, slot: i64, role: &str| -> [f64; 3] {
            let p = s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["part"] == part && p["slot"] == slot && p["role"] == role)
                .unwrap_or_else(|| panic!("{part} slot {slot} {role}"));
            n3(&p["translation_mm"])
        };
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        // 4 docks, 4 fastener sets, 3 parked tools of 2 files each, and the tool on the gantry (head, shuttle, backplate).
        assert_eq!(s["placements"].as_array().unwrap().len(), 4 + 4 + 6 + 3);
        assert!(near(
            find("changer_sc_dock_xol", 0, "dock"),
            [-134.99991944981247, -214.99992383691756, 279.9990417204351]
        ));
        assert!(near(
            find("changer_sc_dock_jabberwocky", 3, "dock"),
            [135.00008055018753, -214.99992383691756, 279.9990417204351]
        ));
        assert!(near(
            find("head_jabberwocky", 1, "tool"),
            [-44.99991939781176, -149.51701146623125, 343.3340667204351]
        ));
        assert!(near(
            find("head_sc_jabberwocky_back", 1, "tool"),
            [-45.255119397811754, -144.25875946623125, 342.2928667204351]
        ));
        assert!(near(
            find("xol", 2, "tool"),
            [45.10103060218824, -149.91570546623123, 9.196330720435071]
        ));
        // The tool on the gantry is the rig's installed head, and it moves with the head.
        assert!(near(
            find("xol", 0, "head"),
            [0.1004302893993379, -19.99951581543714, -213.69148390681065]
        ));
        assert!(near(
            find("changer_sc_core_standard_6", 0, "shuttle"),
            [-0.20056976260135917, -16.50611618612347, 130.40941609318938]
        ));
        let head: Vec<&Value> = s["placements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["follows"] == "head")
            .collect();
        assert_eq!(head.len(), 3);
        // The machine's own head is hidden while a tool is on the gantry; the dock keeps its four hidden parts.
        assert_eq!(s["hide_machine_keys"].as_array().unwrap().len(), 117);
        assert_eq!(
            s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["part"] == "changer_sc_dock_xol")
                .unwrap()["hidden_keys"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        // Nothing is installed in an empty library, and the page is told which files to fetch.
        assert_eq!(s["missing"].as_array().unwrap().len(), 8);
        // INDX: eight docks and seven parked tools at 41 mm, the crossbar once, the smart head on the gantry.
        let sel = check_selection("voron_v24_300_printed", &json!([{"option":"indx_bank","tools":["indx_nozzle","indx_cht_04","indx_cht_05","indx_cht_06","indx_cht_08","indx_cht_10","indx_standard_025","indx_standard_04"],"active":0}])).unwrap();
        let s = scene("voron_v24_300_printed", &sel, &lib);
        assert_eq!(
            s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["role"] == "dock")
                .count(),
            8
        );
        assert_eq!(
            s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["role"] == "tool" && p["part"] == "indx_passive_tool_parked")
                .count(),
            7
        );
        let find = |part: &str, slot: i64, role: &str| -> [f64; 3] {
            n3(&s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["part"] == part && p["slot"] == slot && p["role"] == role)
                .unwrap()["translation_mm"])
        };
        assert!(near(
            find("indx_crossbar_300", -1, "fixture"),
            [
                0.00008055018753339027,
                -206.99992383690943,
                331.5000000001056
            ]
        ));
        assert!(near(
            find("indx_dock_2020", 0, "dock"),
            [-143.49991944981247, -206.99992383690943, 331.5000000001056]
        ));
        assert!(near(
            find("indx_passive_tool_parked", 1, "tool"),
            [-102.49991944981247, -206.99992383690943, 331.5000000001056]
        ));
        assert!(near(
            find("head_indx_4010_closed", 0, "head"),
            [
                0.000030734041664981504,
                -40.70048989771558,
                127.50941879403933
            ]
        ));
    }

    /// A changer system is shown on its own at the offsets the project registers: shuttle at the origin, the dock
    /// 150 mm to its left, the tool's backplate by the three pins.
    #[test]
    fn an_option_on_its_own_is_its_files_at_the_places_the_project_shows_them() {
        let lib = Library::new(scratch("alone"));
        let o = option("stealthchanger_standard_6_xol").unwrap();
        let s = option_scene(o, &lib);
        assert!(s["machine"].is_null());
        assert_eq!(s["placements"].as_array().unwrap().len(), 3);
        let dock = s["placements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["role"] == "dock")
            .unwrap();
        assert_eq!(dock["part"], "changer_sc_dock_xol");
        assert_eq!(n3(&dock["translation_mm"]), [-150.0, 0.0, 0.0]);
        assert_eq!(
            s["missing"].as_array().unwrap().len(),
            3,
            "nothing is installed, so the page says what to fetch"
        );
        assert_eq!(option_view(o, &lib)["previewable"], true);
        assert_eq!(
            option_view(option("stealthchanger_bank").unwrap(), &lib)["previewable"],
            false,
            "a bank is drawn on its machine"
        );
        assert!(
            option_view(option("component_revo_voron").unwrap(), &lib)["previewable"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn a_placed_add_on_hides_what_it_replaces() {
        let lib = Library::new(scratch("addon"));
        let sel = check_selection(
            "voron_trident_350",
            &json!([{"option":"stealth_skirts_honeycomb"},{"option":"sturdy_handles_m5"}]),
        )
        .unwrap();
        let s = scene("voron_trident_350", &sel, &lib);
        let skirts = s["placements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| {
                p["part"]
                    .as_str()
                    .unwrap()
                    .starts_with("trident_stealth_skirts_")
            })
            .count();
        assert_eq!(skirts, 3, "the common, honeycomb and fastener parts");
        assert_eq!(
            s["hide_machine_keys"].as_array().unwrap().len(),
            171,
            "the stock skirts the option replaces"
        );
        assert!(
            s["placements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["follows"].is_null()),
            "add-ons sit on the frame"
        );
        assert_eq!(s["machine"], "voron_trident_350");
    }

    #[test]
    fn a_wrong_file_is_refused_for_a_part_and_nothing_of_it_is_kept() {
        let lib = Library::new(scratch("wrong"));
        // A folder that holds files by the right names with the wrong bytes: each is refused against its pin.
        let src = scratch("wrongsrc");
        fs::create_dir_all(src.join("modules")).unwrap();
        let o = option("sturdy_handles_m5").unwrap();
        for p in part_ids(o) {
            fs::write(
                src.join(format!("modules/{p}.glb.gz")),
                b"glTF not the published file",
            )
            .unwrap();
        }
        let e = lib
            .install_option("sturdy_handles_m5", &Source::Dir(src), false)
            .unwrap_err();
        assert!(
            e.contains("not the model the author published") && e.contains("Nothing was installed"),
            "{e}"
        );
        assert!(!lib.option_installed(o));
        for p in part_ids(o) {
            assert!(
                !lib.root.join("parts").join(&p).exists(),
                "{p} left nothing behind"
            );
        }
        // An empty folder says what it looked for, and a part nobody lists is not a part.
        let e = lib
            .install_option("sturdy_handles_m5", &Source::Dir(scratch("none")), false)
            .unwrap_err();
        assert!(e.contains("does not hold this model"), "{e}");
        assert!(lib
            .install_option("made_up", &Source::Dir(scratch("none2")), false)
            .is_err());
    }

    /// An installed option is its files; removing one keeps the files another installed option still needs.
    #[test]
    fn removing_an_option_keeps_the_files_another_option_needs() {
        let lib = Library::new(scratch("remove"));
        let (a, b) = (
            option("stealthchanger_bank").unwrap(),
            option("stealthchanger_standard_6_xol").unwrap(),
        );
        let shared: Vec<String> = part_ids(a)
            .into_iter()
            .filter(|p| part_ids(b).contains(p))
            .collect();
        assert!(
            !shared.is_empty(),
            "the bank and the Xol changer system share files"
        );
        // Put every file in place as the installer leaves it (the record and the file).
        for p in part_ids(a).into_iter().chain(part_ids(b)) {
            let d = lib.root.join("parts").join(&p);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("model.glb.gz"), b"x").unwrap();
            fs::write(d.join("install.json"), b"{}").unwrap();
        }
        assert!(lib.option_installed(a) && lib.option_installed(b));
        assert_eq!(
            lib.installed_options()
                .iter()
                .filter(|o| *o == "stealthchanger_bank" || *o == "stealthchanger_standard_6_xol")
                .count(),
            2
        );
        lib.remove_option("stealthchanger_bank").unwrap();
        assert!(!lib.option_installed(a));
        assert!(
            lib.option_installed(b),
            "the other option still has all of its files"
        );
        for p in &shared {
            assert!(lib.part_file(p).is_some(), "{p} is kept");
        }
        assert!(
            lib.part_file("changer_sc_bank_fasteners").is_none(),
            "a file only the bank used went with it"
        );
        lib.remove_option("stealthchanger_standard_6_xol").unwrap();
        assert!(!lib.option_installed(b));
        assert!(
            lib.part_file("changer_sc_core_standard_6").is_none(),
            "the shared file went when the last option that needed it did"
        );
        // A file that is the whole of another option (the Xol head) stays while that option is complete.
        assert!(
            lib.option_installed(option("head_xol").unwrap()) == lib.part_file("xol").is_some()
        );
    }

    #[test]
    fn the_authors_published_address_is_a_file_or_an_explanation() {
        assert_eq!(Source::parse("published").unwrap(), Source::Published);
        assert_eq!(Source::parse(" Published ").unwrap(), Source::Published);
        // A maker's web page (the CORE One L on Printables) is not a file Stella Scope can fetch. Since polish WO-06 the real
        // CORE One L row has a release asset, so the row here is the same one as it was: the maker's page and no file address.
        let core = machine("prusa_core_one_l").unwrap();
        let mut page_only = core.clone();
        page_only["location"] = json!({"kind":"maker","upstream":upstream_url(core).unwrap()});
        let e = match Source::Published.open(&page_only) {
            Err(e) => e,
            Ok(_) => panic!("a web page is not a file"),
        };
        assert!(e.contains("no published file address"), "{e}");
        // A catalogue part names the address the rig serves it from, over https.
        let up = upstream_url(part("changer_sc_dock_xol").unwrap()).unwrap();
        assert_eq!(
            up,
            "https://psych0h3ad.github.io/3d-print-rig/modules/changer_sc_dock_xol.glb.gz"
        );
        assert_eq!(
            page_path(part("changer_sc_dock_xol").unwrap()).unwrap(),
            "modules/changer_sc_dock_xol.glb.gz"
        );
    }

    #[test]
    fn the_e3ng_changer_is_a_machine_kept_out_of_builds() {
        let m = machine("e3ng_toolchanger").expect("the E3NG hotend changer is listed");
        assert_eq!(class_of(m), "nc");
        assert!(!may_bundle(class_of(m)));
        assert!(m["licence"]["text"]
            .as_str()
            .unwrap()
            .contains("CC-BY-NC-SA"));
        assert!(thumb_bytes("e3ng_toolchanger").is_none());
        let (s, d) = pin(m);
        assert!(hex64(&s) && hex64(&d));
    }
}

#[cfg(test)]
mod part_record_tests {
    use super::*;
    use std::collections::HashSet;

    /// Polish WO-02: the Voron V2.4, Trident, V0.2, Micron and Ender-3 are registered from the part
    /// records in their own models (tools/printer-motion-parts.rs). Each profile is for exactly the model
    /// the catalogue pins, names every part once, moves only groups it has a motion for, and reads as a
    /// calibration the page can use.
    #[test]
    fn motion_read_from_part_records_is_registered_for_these_exact_models() {
        for id in [
            "voron_v24_350_printed",
            "voron_v24_300_printed",
            "voron_trident_300",
            "voron_v02_120",
            "micron_plus_r1_180",
            "ender3_stock_220",
        ] {
            let p = profile(id).unwrap_or_else(|| panic!("{id} has no registered motion"));
            assert_eq!(
                p["model_sha256"],
                machine(id).unwrap()["pin"]["model_sha256"],
                "{id}"
            );
            let groups = p["groups"].as_object().unwrap();
            let mut seen = HashSet::new();
            for (name, keys) in groups {
                assert!(
                    p["motions"].get(name).is_some(),
                    "{id}: {name} has parts but no motion"
                );
                assert!(
                    !keys.as_array().unwrap().is_empty(),
                    "{id}: {name} holds no part"
                );
                for k in keys.as_array().unwrap() {
                    assert!(
                        seen.insert(k.as_str().unwrap().to_owned()),
                        "{id}: {k} is in two groups"
                    );
                }
            }
            let cal =
                Calibration::from_value(&p["calibration"]).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(
                Calibration::starting(p),
                cal,
                "{id}: the profile's own calibration is the start"
            );
            // The machine's own travel is in range at the start: reported 0 is the low end of each axis
            // (a bed slinger's Y runs from the front edge), so nothing is held at the first position.
            let rest = pose(p, &cal, [Some(0.0), Some(0.0), Some(0.0)]);
            assert_eq!(rest["clamped"], json!([]), "{id}: {rest}");
            assert!(
                p["notices"].as_array().is_some_and(|n| !n.is_empty()),
                "{id}"
            );
        }
    }

    #[test]
    fn a_pose_without_tools_is_what_it_always_was() {
        let p = profile("voron_v24_350_printed").unwrap();
        let cal = Calibration::starting(p);
        let at = [Some(100.0), Some(120.0), Some(30.0)];
        let (a, b) = (pose(p, &cal, at), pose_for_tool(p, &cal, at, Some(1)));
        assert_eq!(
            a["groups"], b["groups"],
            "a profile with no tools ignores the active tool"
        );
        assert_eq!(
            (a["tool"].clone(), b["tool"].clone()),
            (Value::Null, json!(1))
        );
    }
}

#[cfg(test)]
mod print_frame_tests {
    use super::*;

    fn vec3(v: &Value) -> [f64; 3] {
        [0, 1, 2].map(|i| v[i].as_f64().unwrap())
    }

    /// Where a print file's point lands in the model's frame (millimetres): the table's own rule.
    fn at(frame: &Value, p: [f64; 3]) -> [f64; 3] {
        let o = vec3(&frame["origin_mm"]);
        let mut out = o;
        for (i, k) in p.iter().enumerate() {
            let a = vec3(&frame["axes"][i]);
            for c in 0..3 {
                out[c] += k * a[c];
            }
        }
        out
    }

    #[test]
    fn every_print_frame_is_for_a_catalogue_model_and_reads_as_a_frame() {
        let all = print_frames()["machines"].as_object().unwrap();
        assert!(
            all.len() >= 9,
            "the nine demo-farm machines are measured: {}",
            all.len()
        );
        for (id, f) in all {
            let row = machine(id).unwrap_or_else(|| panic!("{id} is not in the catalogue"));
            assert_eq!(f["model_sha256"], row["pin"]["model_sha256"], "{id}");
            // The origin is a point, the axes are a signed permutation (three unit vectors, one per direction).
            vec3(&f["origin_mm"]).iter().for_each(|v| assert!(v.is_finite(), "{id}"));
            let mut used = [0.0_f64; 3];
            for i in 0..3 {
                let a = vec3(&f["axes"][i]);
                assert!((a.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-9, "{id}: axis {i} is a unit vector");
                assert!(a.iter().all(|v| v.abs() < 1e-9 || (v.abs() - 1.0).abs() < 1e-9), "{id}");
                for c in 0..3 {
                    used[c] += a[c].abs();
                }
            }
            assert_eq!(used, [1.0, 1.0, 1.0], "{id}: each frame direction is one printer axis");
            assert!(f["basis"].as_str().is_some_and(|b| b.len() > 40), "{id}: says how it was found");
            // The group that carries the plate really holds parts and moves (a bed that drops or slides).
            if let Some(g) = f["group"].as_str() {
                let p = profile(id).unwrap_or_else(|| panic!("{id}: a carried plate needs registered motion"));
                assert!(p["motions"].get(g).is_some(), "{id}: {g} has no motion");
                // The CORE One L's groups, and (F09) those of the other Prusa machines, come from their models' own part records, not a list in the profile.
                let has_parts = p["groups"][g].as_array().is_some_and(|a| !a.is_empty()) || id.starts_with("prusa_");
                assert!(has_parts, "{id}: {g} holds no parts");
            }
        }
    }

    /// The head and the part agree: the nozzle, put where the calibrated pose puts it, is on the printer's own
    /// coordinates in the plate's frame, wherever the head is. For a bed that moves, the plate goes with it.
    #[test]
    fn the_nozzle_is_on_the_part_wherever_the_head_is() {
        let mut checked = 0;
        for (id, f) in print_frames()["machines"].as_object().unwrap() {
            let (Some(p), true) = (profile(id), f["nozzle_mm"].is_array()) else {
                continue;
            };
            let cal = Calibration::starting(p);
            let n0 = vec3(&f["nozzle_mm"]);
            for pos in [[0.0, 0.0, 0.0], [10.0, 20.0, 5.0], [100.0, 100.0, 40.0], [60.0, 90.0, 0.2]] {
                let pz = pose(p, &cal, pos.map(Some));
                let group = |name: &str| pz["groups"].get(name).map_or([0.0; 3], vec3);
                let head = group("head");
                let bed = f["group"].as_str().map_or([0.0; 3], group);
                // Nozzle in the frame = where it sits in the pose + the head's move - what the plate moved (the plate is
                // the nozzle's reference), compared with the printer coordinate the file asks for.
                let nozzle: Vec<f64> = (0..3).map(|c| n0[c] + head[c] - bed[c]).collect();
                let want = at(f, pos);
                for c in 0..3 {
                    assert!(
                        (nozzle[c] - want[c]).abs() < 0.6,
                        "{id} at {pos:?}: the nozzle is at {nozzle:?} and the part at {want:?} (clamped {})",
                        pz["clamped"]
                    );
                }
            }
            checked += 1;
        }
        assert!(checked >= 7, "the machines with a measured nozzle were all checked: {checked}");
    }

    #[test]
    fn the_sv08_reaches_its_plate_with_a_calibration_measured_from_its_model() {
        let p = profile("sovol_sv08_350").unwrap();
        assert!(p["calibration_basis"].as_str().unwrap().starts_with("Measured from the model"));
        let cal = Calibration::starting(p);
        // The first layer is not held above the plate and the far corner is inside the travel.
        for pos in [[0.0, 0.0, 0.2], [350.0, 350.0, 0.2], [175.0, 175.0, 300.0]] {
            let pz = pose(p, &cal, pos.map(Some));
            assert_eq!(pz["clamped"], json!([]), "{pos:?}: {pz}");
        }
        // A machine that already had its own calibration keeps it.
        let v24 = profile("voron_v24_350_printed").unwrap();
        assert!(!v24["calibration_basis"].as_str().unwrap().starts_with("Measured from the model"));
    }
}

#[cfg(test)]
mod swapping_tests {
    use super::*;

    fn bank(machine: &str, tools: &[&str]) -> Value {
        let _ = machine;
        json!([{"option":"stealthchanger_bank","tools":tools,"active":0}])
    }

    const XOL: &str = "xol_sherpa_mini_rapido2_uhf";
    const JAB: &str = "jabberwocky_jabberwocky_drive_conch";

    /// A library holding a stand-in file for every part named (a record and a file are all the library checks).
    fn full_library(tag: &str, scene: &Value) -> (Library, PathBuf) {
        let root = std::env::temp_dir().join(format!("stella-swap-{tag}-{}", std::process::id()));
        let lib = Library::new(root.clone());
        for p in scene["placements"].as_array().unwrap() {
            let id = p["part"].as_str().unwrap();
            let dir = root.join("parts").join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("install.json"), "{}").unwrap();
            fs::write(dir.join("model.glb"), "x").unwrap();
        }
        (lib, root)
    }

    /// Each tool is one rigid body: its move from the dock to the carriage is one number for all its parts. The park
    /// values, put through the head's motion, carry the head to the dock and are inside the machine's travel.
    #[test]
    fn a_tool_banks_tools_have_one_move_each_and_park_inside_the_travel() {
        for machine in ["voron_v24_300_printed", "voron_v24_350_printed"] {
            let sel = bank(machine, &[XOL, JAB, XOL]);
            let tools = bank_tools(machine, &sel).unwrap_or_else(|| panic!("{machine}: no tools"));
            assert_eq!(tools.len(), 3);
            let p = profile(machine).unwrap();
            for t in &tools {
                for (i, n) in NAMES.iter().enumerate() {
                    let (lo, hi) = (p["axes"][n][0].as_f64().unwrap(), p["axes"][n][1].as_f64().unwrap());
                    assert!(t.park[i] >= lo && t.park[i] <= hi, "{machine} {}: park {n} {} is outside {lo}..{hi}", t.group, t.park[i]);
                }
            }
            // Two docks of the same tool differ by the pitch along X and in nothing else: the same tool, a different place.
            let (a, b) = (&tools[0], &tools[2]);
            assert!((a.rest_mm[0] - b.rest_mm[0] - 180.0).abs() < 0.01, "docks 0 and 2 are two pitches apart: {:?} {:?}", a.rest_mm, b.rest_mm);
            assert_eq!((a.rest_mm[1], a.rest_mm[2]), (b.rest_mm[1], b.rest_mm[2]));
            // No bank, no tools; a machine the bank does not fit, none either.
            assert!(bank_tools(machine, &json!([])).is_none());
            assert!(bank_tools("voron_trident_300", &sel).is_none());
        }
    }

    /// The pose with a bank: the active tool is on the carriage (it follows the head), every other tool is exactly where its
    /// dock holds it (no displacement), and the head standing at a tool's dock puts that tool exactly in its dock.
    #[test]
    fn the_active_tool_follows_the_head_and_the_others_sit_in_their_docks() {
        let machine = "voron_v24_300_printed";
        let sel = bank(machine, &[XOL, JAB, XOL]);
        let spec = pose_spec(machine, Some(&sel)).unwrap();
        let cal = Calibration::starting(profile(machine).unwrap());
        let tools = bank_tools(machine, &sel).unwrap();
        let v = |x: &Value| [0, 1, 2].map(|i| x[i].as_f64().unwrap());
        for active in 0..3 {
            let pz = pose_for_tool(&spec, &cal, [Some(150.0), Some(150.0), Some(40.0)], Some(active));
            for t in 0..3 {
                let g = v(&pz["groups"][format!("tool{t}")]);
                if t == active {
                    let head = v(&pz["groups"]["head"]);
                    for c in 0..3 {
                        assert!((g[c] - head[c] - tools[t].rest_mm[c]).abs() < 1e-9, "tool {t} follows the head plus its rest offset");
                    }
                } else {
                    assert!(g.iter().all(|x| x.abs() < 0.01), "tool {t} is in its dock when tool {active} is active: {g:?}");
                }
            }
        }
        // The head at tool t's dock: its tool is in the dock, to a hundredth of a millimetre.
        for (t, tool) in tools.iter().enumerate() {
            let pz = pose_for_tool(&spec, &cal, tool.dock.map(Some), Some(t));
            let g = v(&pz["groups"][format!("tool{t}")]);
            assert!(g.iter().all(|x| x.abs() < 0.01), "tool {t} at its dock: {g:?} (clamped {})", pz["clamped"]);
            assert_eq!(pz["clamped"], json!([]), "the dock is inside the travel");
        }
        // A machine's own pose spec, with no bank, is its profile's motion.
        let plain = pose_spec(machine, None).unwrap();
        assert!(plain["tools"].is_null() && plain["motions"]["tool0"].is_null());
    }

    /// Each tool has its own nozzle, a few millimetres from the machine's and from the next tool's. Whichever tool is active, its
    /// nozzle (read off its file, through the placement the scene gives its head part, and the group the pose moves) is on the
    /// printer's coordinates in the plate's frame, where the part grows: the part is always under the nozzle that is printing.
    #[test]
    fn the_active_tools_nozzle_is_on_the_part_whichever_tool_it_is() {
        let machine = "voron_v24_300_printed";
        let sel = bank(machine, &[XOL, JAB, XOL]);
        let empty = Library::new(std::env::temp_dir().join(format!("stella-swap-nozzle-{}", std::process::id())));
        let sc = scene_swapping(machine, &sel, &empty);
        let spec = pose_spec(machine, Some(&sel)).unwrap();
        let cal = Calibration::starting(profile(machine).unwrap());
        let frame = print_frame(machine).unwrap();
        let n3v = |v: &Value| [0, 1, 2].map(|i| v[i].as_f64().unwrap());
        let origin = n3v(&frame["origin_mm"]);
        let b = basis(machine);
        let tools = bank_tools(machine, &sel).unwrap();
        // The measured nozzles differ from the machine's by millimetres, and by tool.
        assert!((tools[1].offset_mm[2] - 9.75).abs() < 0.2 && (tools[0].offset_mm[2] - 3.47).abs() < 0.2, "{:?} {:?}", tools[0].offset_mm, tools[1].offset_mm);
        let o = option("stealthchanger_bank").unwrap();
        for active in 0..3 {
            let tid = [XOL, JAB, XOL][active];
            let head_part = o["attach"]["tools"][tid]["active"].as_array().unwrap().iter().find(|x| x["role"] == "head").unwrap()["part"].as_str().unwrap();
            let pl = sc["placements"].as_array().unwrap().iter().find(|p| p["slot"] == active as i64 && p["part"] == head_part && p["role"] == "tool").unwrap();
            let t = n3v(&pl["translation_mm"]);
            let tip_file = n3v(&print_frames()["tools"][tid]["tip_file_mm"]);
            let tip: [f64; 3] = [0, 1, 2].map(|c| (0..3).map(|r| b[r][c].as_f64().unwrap() * tip_file[r]).sum::<f64>());
            for pos in [[150.0, 150.0, 0.2], [60.0, 90.0, 20.0], [240.0, 200.0, 55.0]] {
                let pz = pose_for_tool(&spec, &cal, pos.map(Some), Some(active));
                let g = n3v(&pz["groups"][format!("tool{active}")]);
                for c in 0..3 {
                    let nozzle = tip[c] + t[c] + g[c];
                    let want = origin[c] + pos[c];
                    assert!((nozzle - want).abs() < 0.05, "tool {active} at {pos:?}: the nozzle is at {nozzle:.2} and the part at {want:.2} on axis {c}");
                }
            }
        }
        assert!(tools.iter().all(|t| t.dock[2] < 280.0 && t.dock[2] > 100.0), "the docks stay inside the machine: {:?}", tools.iter().map(|t| t.dock).collect::<Vec<_>>());
    }

    /// The swapping scene: every tool at its dock in a group of its own, the shuttle with the head, the machine's own head
    /// hidden once the bank's files are all here, and shown while they are not (the demo plays with the machine alone).
    #[test]
    fn the_swapping_scene_has_every_tool_at_its_dock_and_keeps_the_stock_head_until_the_bank_is_here() {
        let machine = "voron_v24_300_printed";
        let sel = bank(machine, &[XOL, JAB, XOL]);
        let empty = Library::new(std::env::temp_dir().join(format!("stella-swap-none-{}", std::process::id())));
        let none = scene_swapping(machine, &sel, &empty);
        assert!(!none["missing"].as_array().unwrap().is_empty(), "nothing is installed");
        assert_eq!(none["hide_machine_keys"], json!([]), "the stock head stays while the bank is not here");
        assert_eq!(none["swapping"], json!(true));
        let (lib, root) = full_library("swap", &none);
        let here = scene_swapping(machine, &sel, &lib);
        assert_eq!(here["missing"], json!([]));
        assert!(!here["hide_machine_keys"].as_array().unwrap().is_empty(), "the stock head is replaced once the bank is here");
        let follows = |p: &Value| p["follows"].as_str().unwrap_or("").to_owned();
        let tool_groups: std::collections::BTreeSet<String> = here["placements"].as_array().unwrap().iter()
            .filter(|p| p["role"] == "tool").map(follows).collect();
        assert_eq!(tool_groups, ["tool0", "tool1", "tool2"].map(String::from).into_iter().collect());
        let shuttles: Vec<&Value> = here["placements"].as_array().unwrap().iter().filter(|p| p["role"] == "shuttle").collect();
        assert_eq!(shuttles.len(), 1, "one carriage half");
        assert_eq!(shuttles[0]["follows"], "head");
        // The docks and fasteners stand still.
        assert!(here["placements"].as_array().unwrap().iter().filter(|p| p["role"] == "dock").all(|p| p["follows"].is_null()));
        // The static scene (a printer that reports no toolchanger) is what it was.
        let still = scene(machine, &sel, &lib);
        assert!(still["swapping"] == false && still["placements"].as_array().unwrap().iter().filter(|p| p["follows"] == "head").count() > 1);
        let _ = fs::remove_dir_all(root);
    }

    /// What the printer page reads to say what is drawn (web/next-models.js `placed`): the demo's bank uses some of the option's
    /// files, not all, so the option's own all-files flag stays false while every placement of it has its file. The page goes by the
    /// placements; it once printed "Drawn: ." because it trusted the flag.
    #[test]
    fn a_bank_the_demo_draws_has_every_placement_here_while_the_options_own_flag_is_false() {
        let machine = "voron_v24_300_printed";
        let sel = bank(machine, &[JAB, JAB, JAB]);
        let empty = Library::new(std::env::temp_dir().join(format!("stella-swap-flag0-{}", std::process::id())));
        let (lib, root) = full_library("flag", &scene_swapping(machine, &sel, &empty));
        let here = scene_swapping(machine, &sel, &lib);
        assert_eq!(here["missing"], json!([]));
        assert_eq!(here["options"][0]["option"], "stealthchanger_bank");
        assert_eq!(here["options"][0]["installed"], json!(false), "the option lists files the demo does not use");
        let ps = here["placements"].as_array().unwrap();
        assert!(!ps.is_empty() && ps.iter().all(|p| p["option"] == "stealthchanger_bank" && p["installed"] == json!(true)));
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;

    /// A gzip file made of one stored block (so the file is exactly what the test says) with its two pins.
    fn gz(body: &[u8]) -> (Vec<u8>, String, String) {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1];
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(body.len() as u16)).to_le_bytes());
        out.extend_from_slice(body);
        let mut crc = 0xffff_ffff_u32;
        for &b in body {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(!crc).to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        let stored = crate::sha256::hex(&out);
        (out, stored, crate::sha256::hex(body))
    }

    fn row(id: &str, stored: &str, decoded: &str, bytes: usize) -> Value {
        json!({"id":id,"label":format!("Fixture {id}"),"licence":{"class":"gpl","text":"GPL-3.0","author":"A. Maker"},
               "pin":{"stored_model_sha256":stored,"model_sha256":decoded},
               "location":{"kind":"registry","upstream":format!("https://psych0h3ad.github.io/3d-print-rig-community-models/{id}/model.glb.gz"),"mirror":{"bytes":bytes}}})
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stella-fetch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Polish WO-03 found five machines (the DoomCube 2, three Tridents, the V2.4 1000) whose published address named the
    /// community-models pages while the file is in the large-voron-models repository: it answered 404 and the one-press fetch
    /// could not get the DoomCube. A file's address is the pages of the repository that holds it (checked against the live
    /// pages by hand; this keeps the catalogue from saying otherwise).
    #[test]
    fn a_models_published_address_is_the_pages_of_the_repository_that_holds_it() {
        let mut checked = 0;
        for m in catalogue()["machines"].as_array().unwrap() {
            let (Some(git), Some(up)) = (m["location"]["mirror"]["git"].as_str(), m["location"]["upstream"].as_str()) else {
                continue;
            };
            let repo = git.strip_prefix("git/").and_then(|g| g.strip_suffix(".git")).unwrap();
            assert!(
                up.starts_with(&format!("https://psych0h3ad.github.io/{repo}/")),
                "{}: held by {repo} but published at {up}",
                m["id"]
            );
            checked += 1;
        }
        assert!(checked > 25, "the registry machines were all checked: {checked}");
        assert!(machine("doomcube2_350_reference").unwrap()["location"]["upstream"].as_str().unwrap().contains("-large-voron-models/doomcube-v103/"));
    }

    /// What a fresh install is missing after "Try the demo farm" (finish-2 F09, 2026-10-19): the build carries four models (the CORE One L, the CORE One, the
    /// MK3S+ and the MINI) and puts them in the library when the service starts, so they are not asked for; what is left is the seven open models and the seven
    /// open files of the toolchanger's tools and the Trident's gantry, fetched on one press. Nothing it asks for is non-commercial. Before the service has put the
    /// carried models in, a machine of the build is said to be on its way, never fetched from a page that may not exist.
    /// rc9 WO-02: the installer's open-licence models are read in place, never copied, and a non-commercial file in that
    /// folder is ignored (the folder is the package's, but a build may never carry one).
    #[test]
    fn carried_models_are_read_in_place_and_only_open_ones_count() {
        let root = scratch("carried-in-place").join("printer-models");
        let carried = scratch("carried-folder");
        fs::write(carried.join("voron_v24_350_printed.glb.gz"), b"gz").unwrap();
        fs::write(carried.join("tictac_21_120.glb.gz"), b"gz").unwrap();
        let lib = Library::with_carried(root.clone(), Some(carried.clone()));
        let (p, gz) = lib.model_file("voron_v24_350_printed").expect("carried model");
        assert!(gz && p.starts_with(&carried));
        assert_eq!(lib.record("voron_v24_350_printed").unwrap()["origin"], "carried with Stella Scope");
        assert!(lib.model_file("tictac_21_120").is_none(), "non-commercial is never carried");
        assert!(!root.join("voron_v24_350_printed").exists(), "nothing copied");
        assert!(lib.installed_ids().contains(&"voron_v24_350_printed".to_string()));
        let n = notice_text(machine("voron_v24_350_printed").unwrap()).unwrap();
        assert!(n.contains("VoronDesign") && n.contains("GNU GENERAL PUBLIC LICENSE"));
        fs::create_dir_all(carried.join("parts")).unwrap();
        fs::write(carried.join("parts").join("additional_doublet_v24.glb.gz"), b"gz").unwrap();
        assert!(lib.part_file("additional_doublet_v24").is_some_and(|(p, _)| p.starts_with(&carried)), "a carried tool file");
        assert!(Library::new(root).model_file("voron_v24_350_printed").is_none(), "new() carries nothing");
    }
    #[test]
    fn a_fresh_install_has_the_models_the_build_carries_and_wants_the_open_ones_and_tool_files() {
        let lib = Library::new(scratch("wanted").join("printer-models"));
        let want: Vec<(String, Option<Value>)> = crate::demo::demo_farm().into_iter().filter_map(|e| e.cad_model.clone().map(|m| (m, e.cad_options.clone()))).collect();
        assert_eq!(want.len(), 9, "eight machines and the Prusa MK3S+");
        // Before the service starts: the build's own models are not fetchable and say so.
        let before = wanted(&want, &lib);
        let core = before["models"].as_array().unwrap().iter().find(|m| m["id"] == "prusa_core_one_l").unwrap();
        assert!(core["open"] == false && core["why"].as_str().unwrap().contains("ships inside Stella Scope"), "{core}");
        // The service starts: the four are in the library, checked against their pins, and a second start writes nothing.
        let wrote = lib.seed_bundled();
        assert_eq!(wrote, ["prusa_core_one_l", "prusa_core_one", "prusa_mk3s", "prusa_mini"]);
        assert!(lib.seed_bundled().is_empty());
        let w = wanted(&want, &lib);
        let rows = w["models"].as_array().unwrap();
        let models: Vec<&Value> = rows.iter().filter(|m| m["kind"] == "model").collect();
        let parts: Vec<&Value> = rows.iter().filter(|m| m["kind"] == "part").collect();
        // F03 (2026-10-19): the Trident carries its R2 gantry and X carriage (two more open files). F09: the CORE One L and the MK3S+ are not asked for (it was 8 models, 7 files and 14 in all).
        assert_eq!((models.len(), parts.len(), w["open_count"].clone()), (7, 7, json!(14)), "{w}");
        assert!(models.iter().all(|m| m["id"] != "prusa_core_one_l" && m["id"] != "prusa_mk3s"));
        let open: Vec<&Value> = rows.iter().filter(|m| m["open"] == true).collect();
        assert!(open.iter().all(|m| ["gpl", "cc-by-sa"].contains(&m["licence"]["class"].as_str().unwrap()) && m["bytes"].as_u64().unwrap() > 1_000 && m["why"].is_null()), "no open row is under a non-commercial licence");
        let bytes: u64 = open.iter().map(|m| m["bytes"].as_u64().unwrap()).sum();
        assert_eq!(w["open_bytes"], json!(bytes));
        assert!(bytes > 140_000_000 && bytes < 190_000_000, "about 160 MB: {bytes}");
        let ids: Vec<&str> = parts.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"head_jabberwocky") && ids.contains(&"changer_sc_dock_jabberwocky") && !ids.contains(&"xol"), "{ids:?}");
        assert!(ids.contains(&"trident_r2_gantry_300") && ids.contains(&"trident_r2_sb_carriage"), "the Trident's gantry and carriage are asked for: {ids:?}");
        // A model that is already here is not asked for again, and neither is one named twice.
        let mut twice = want.clone();
        twice.extend(want.clone());
        assert_eq!(wanted(&twice, &lib)["models"].as_array().unwrap().len(), 14);
        let _ = fs::remove_dir_all(lib.root.parent().unwrap());
    }

    /// Finish-2 F09 (2026-10-19), replacing the polish WO-06 test of the CORE One L as a release asset fetched after a notice (the release was never published
    /// and its address answered 404; the owner: ship it inside the app). The four models the build carries are pinned (both hashes and the size agree with the
    /// bytes inside), under a licence a build may carry, and put in a fresh library with no manual step. Each has a notice that names the maker, the
    /// original, the licence and both pins and ends with the licence text; a person cannot remove one (it would only come back); a copy the person put there
    /// is left alone when it is the same model and mended when it is damaged; and no catalogue row points at a release asset any more.
    #[test]
    fn the_models_the_build_carries_are_pinned_put_in_a_fresh_library_and_credited() {
        assert_eq!(BUNDLED.len(), 4);
        let lib = Library::new(scratch("carried").join("printer-models"));
        for (id, bytes) in BUNDLED {
            let row = machine(id).unwrap_or_else(|| panic!("{id} is in the catalogue"));
            let (stored, decoded) = pin(row);
            assert_eq!(crate::sha256::hex(bytes), stored, "{id}: the bytes inside are the stored file the catalogue pins");
            let mut d = Sha256::new();
            let mut unpacked = 0_u64;
            gzip::gunzip(std::io::Cursor::new(bytes), |c| { unpacked += c.len() as u64; d.update(c); Ok(()) }).unwrap();
            assert_eq!(d.hex(), decoded, "{id}: unpacked, they are the model the catalogue pins");
            assert_eq!(model_bytes(row), Some(bytes.len() as u64), "{id}");
            assert!(unpacked > 4_000_000, "{id}: {unpacked}");
            assert!(may_bundle(class_of(row)) && is_bundled(id), "{id}: a class a build may carry");
            assert!(["ocl", "gpl"].contains(&class_of(row)), "{id}");
            assert_eq!(row["location"]["kind"], "bundled");
            assert!(profile(id).is_some() && print_frame(id).is_some(), "{id} moves and its part sits on its plate");
            // its notice
            let text = notice_text(row).unwrap();
            for must in [row["licence"]["author"].as_str().unwrap().split(" (").next().unwrap(), stored.as_str(), decoded.as_str(), "Prusa Research is not connected with Stella Scope", "https://www.printables.com/model/"] {
                assert!(text.contains(must), "{id}: the notice says {must:?}");
            }
            assert!(text.contains(&format!("{} bytes", bytes.len().to_string().as_bytes().rchunks(3).rev().map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(","))), "{id}: the notice gives the size");
            // it ends with the licence text it is under, whole: the OCL for Prusa's two, the GNU GPL of Prusa's own repository for the others
            let (title, body) = match id {
                "prusa_core_one_l" | "prusa_core_one" => ("THE OPEN COMMUNITY LICENSE (OCL v1)", "Open Community License (OCL v1)"),
                "prusa_mk3s" => ("THE GNU GENERAL PUBLIC LICENSE, VERSION 2", "Version 2, June 1991"),
                _ => ("THE GNU GENERAL PUBLIC LICENSE, VERSION 3", "Version 3, 29 June 2007"),
            };
            assert!(text.contains(title) && text.contains(body), "{id}: it ends with the licence text");
            assert!(text.len() > 15_000 || id == "prusa_core_one_l" || id == "prusa_core_one", "{id}: a GPL text is long, this is {}", text.len());
            assert!(!text.contains('\u{2014}'), "no em dash in authored text");
        }
        assert!(notice_text(machine("tictac_21_120").unwrap()).is_none(), "a non-commercial model has no carried notice");
        let ocl = notice_text(machine("prusa_core_one_l").unwrap()).unwrap();
        let flat = ocl.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("MUST NOT BE subject to any systematic or automated text and/or data mining"), "the OCL text is whole");
        assert!(ocl.contains("Source file: CORE_One_L_assembly.step") && ocl.contains("2f46bccb098bd8da"));
        // a fresh library: all four, with no network and no file to find
        let wrote = lib.seed_bundled();
        assert_eq!(wrote.len(), 4);
        for (id, _) in BUNDLED {
            let (path, gz) = lib.model_file(id).unwrap_or_else(|| panic!("{id} is in the library"));
            assert!(gz && path.is_file());
            let r = lib.record(id).unwrap();
            assert_eq!(r["origin"], "bundled with Stella Scope");
            assert!(r["notice"].as_str().unwrap().contains("ships inside Stella Scope"));
            assert_eq!(view(machine(id).unwrap(), &lib)["in_build"], true);
        }
        assert_eq!(lib.installed_ids().len(), 4);
        // nothing to fetch and nothing to confirm: the licence gate lets a model the build carries through, and the notice is the credit
        assert!(gate(machine("prusa_core_one_l").unwrap(), false).is_ok());
        // a person cannot remove one
        let e = lib.remove("prusa_core_one_l").unwrap_err();
        assert!(e.contains("ships inside Stella Scope") && e.contains("CORE One L"), "{e}");
        assert!(lib.model_file("prusa_core_one_l").is_some());
        // a copy the person installed themselves with the pinned content stays as it is (our own CORE One L was installed that way)
        let own = Library::new(scratch("carried-own").join("printer-models"));
        let row = machine("prusa_core_one_l").unwrap();
        own.install_file("prusa_core_one_l", &mut std::io::Cursor::new(BUNDLED[0].1), "a file you downloaded (prusa-core-one-l-model.glb.gz)", false).unwrap();
        let wrote = own.seed_bundled();
        assert_eq!(wrote, ["prusa_core_one", "prusa_mk3s", "prusa_mini"], "the CORE One L is not written again");
        assert_eq!(own.record("prusa_core_one_l").unwrap()["origin"], "a file you downloaded (prusa-core-one-l-model.glb.gz)");
        // a damaged copy is mended
        let (path, _) = own.model_file("prusa_mini").unwrap();
        fs::write(&path, b"cut short").unwrap();
        assert_eq!(own.seed_bundled(), ["prusa_mini"]);
        assert_eq!(fs::metadata(&path).unwrap().len(), BUNDLED[3].1.len() as u64);
        // no catalogue row points at a release asset any more, and nothing is fetched for the models of the build
        for m in catalogue()["machines"].as_array().unwrap() {
            assert!(m["location"]["kind"] != "release_asset" && m["location"]["asset"].is_null(), "{}", m["id"]);
        }
        let e = lib.install_from("prusa_core_one_l", &Source::Published, false);
        assert!(e.is_err(), "there is no address to fetch it from, so it is never asked of the network");
        let w = wanted(&[("prusa_core_one_l".into(), None)], &lib);
        assert_eq!(w["models"].as_array().unwrap().len(), 0, "it is already here: {w}");
        let _ = row;
        // About: who is credited for what the build carries
        let c = credits();
        assert_eq!(c["models"].as_array().unwrap().len(), 4);
        assert_eq!(c["bytes"], json!(BUNDLED.iter().map(|(_, b)| b.len()).sum::<usize>()));
        assert!(c["models"][0]["author"].as_str().unwrap().contains("Prusa Research") && c["models"][0]["licence"].as_str().unwrap().contains("Open Community License"));
        let _ = fs::remove_dir_all(lib.root.parent().unwrap());
        let _ = fs::remove_dir_all(own.root.parent().unwrap());
    }

    /// Finish-2 F09 (2026-10-19): a model the build carries is a model whose parts are already given to the motion groups (the groups a profile moves are read from
    /// each part's own record in the file), with as many parts as the catalogue says, and a registered motion that moves each group the model holds: a model that
    /// is one lump, or whose bed holds nothing, would draw a printer that does not follow the real one.
    #[test]
    fn every_model_the_build_carries_has_its_parts_in_the_groups_its_motion_moves() {
        for (id, bytes) in BUNDLED {
            let mut glb = Vec::new();
            gzip::gunzip(std::io::Cursor::new(bytes), |c| { glb.extend_from_slice(c); Ok(()) }).unwrap();
            assert_eq!(&glb[..4], b"glTF", "{id}");
            let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
            let json: Value = serde_json::from_slice(&glb[20..20 + len]).unwrap();
            let mut groups: std::collections::BTreeMap<String, usize> = Default::default();
            let nodes = json["nodes"].as_array().unwrap();
            for n in nodes {
                let g = json["meshes"][n["mesh"].as_u64().unwrap() as usize]["extras"]["group"].as_str().unwrap_or("").to_owned();
                *groups.entry(g).or_default() += 1;
            }
            let row = machine(id).unwrap();
            assert_eq!(nodes.len() as u64, row["parts"].as_u64().unwrap(), "{id}: the catalogue's part count is the file's");
            let p = profile(id).unwrap();
            for g in ["bed", "head", "gantry"] {
                assert!(groups.get(g).copied().unwrap_or(0) >= 5, "{id}: group {g} holds parts: {groups:?}");
                assert!(p["motions"].get(g).is_some(), "{id}: the profile moves {g}");
            }
            assert!(groups.get("fixed").copied().unwrap_or(0) > 20, "{id}: most of a machine stands still: {groups:?}");
            // every registered motion group is one the file has, and the file has no group the motion does not know
            for g in groups.keys() {
                assert!(p["motions"].get(g).is_some(), "{id}: the file has a group {g} the profile does not move");
            }
            // the plate is carried by the bed, and the print frame says so
            assert_eq!(print_frame(id).unwrap()["group"], "bed", "{id}");
        }
    }

    /// The fetch runs in the background: a good file lands and is checked, a wrong one is refused and not kept and the others
    /// still arrive, bytes count up to the total, and pressing again while it runs starts nothing new.
    #[test]
    fn a_fetch_lands_each_model_checked_and_a_wrong_file_is_refused_without_stopping_the_rest() {
        let root = scratch("job");
        let src = root.join("src");
        let (a, sa, da) = gz(&vec![b'a'; 60_000]);
        let (b, sb, db) = gz(&vec![b'b'; 40_000]);
        let (c, sc, _) = gz(&vec![b'c'; 30_000]);
        // A file of a tool bank (the catalogue knows it as a part) goes under parts/, checked the same way.
        let (pf, sp, dp) = gz(&vec![b'p'; 20_000]);
        fs::create_dir_all(src.join("modules")).unwrap();
        fs::write(src.join("modules/changer_sc_bank_fasteners.glb.gz"), &pf).unwrap();
        for (id, body) in [("fx_a", &a), ("fx_b", &b), ("fx_c", &c)] {
            let d = src.join(format!("machines/{id}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("model.glb.gz"), body).unwrap();
        }
        let lib = Library::new(root.join("printer-models"));
        // fx_c is pinned to an unpacked hash its file does not have.
        let mut part = row("changer_sc_bank_fasteners", &sp, &dp, pf.len());
        part["kind"] = json!("part");
        part["location"]["upstream"] = json!("https://psych0h3ad.github.io/3d-print-rig/modules/changer_sc_bank_fasteners.glb.gz");
        let rows = vec![row("fx_a", &sa, &da, a.len()), row("fx_c", &sc, &"0".repeat(64), c.len()), part, row("fx_b", &sb, &db, b.len())];
        // A model that needs a licence read first is not fetched without it.
        let mut nc = rows[0].clone();
        nc["licence"]["class"] = json!("nc");
        assert!(start_fetch_rows(&lib, vec![nc], Source::Dir(src.clone()), false).unwrap_err().contains("Read the licence"));
        assert_eq!(fetch_status(&lib)["state"], "idle", "a refused start leaves no job behind");
        // Pressing while one is running changes nothing: it says where that one stands (the state is set by hand, so the
        // test does not race the thread).
        lock_job(&job_for(&lib)).state = "running";
        let busy = start_fetch_rows(&lib, rows.clone(), Source::Dir(src.clone()), true).unwrap();
        assert_eq!((busy["state"].clone(), busy["count"].clone()), (json!("running"), json!(0)));
        lock_job(&job_for(&lib)).state = "";
        let first = start_fetch_rows(&lib, rows, Source::Dir(src), true).unwrap();
        assert_eq!((first["state"].clone(), first["count"].clone()), (json!("running"), json!(4)));
        let mut best = 0;
        let end = loop {
            let st = fetch_status(&lib);
            best = best.max(st["done_bytes"].as_u64().unwrap());
            if st["state"] == "done" {
                break st;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(end["finished"], json!(["fx_a", "changer_sc_bank_fasteners", "fx_b"]), "{end}");
        assert_eq!(end["failed"].as_array().unwrap().len(), 1);
        assert_eq!(end["failed"][0]["id"], "fx_c");
        assert!(end["failed"][0]["error"].as_str().unwrap().contains("not the model the author published"), "{end}");
        assert_eq!(end["total_bytes"], json!(a.len() + b.len() + c.len() + pf.len()));
        assert_eq!(end["done_bytes"], end["total_bytes"], "all three were read, the refused one too");
        assert!(best <= end["total_bytes"].as_u64().unwrap());
        for (id, ok) in [("fx_a", true), ("fx_b", true), ("fx_c", false)] {
            assert_eq!(lib.root.join(id).join("model.glb.gz").is_file(), ok, "{id}");
            assert_eq!(lib.root.join(id).join("install.json").is_file(), ok, "{id}");
        }
        assert!(lib.part_file("changer_sc_bank_fasteners").is_some(), "the tool file is in the library as a part");
        assert!(end["current"].is_null());
        let _ = fs::remove_dir_all(root);
    }

    /// F03 (2026-10-19): why a file was not fetched is said in plain words and a kind the tile chooses its button by. No connection at all (what
    /// `http_get` says when the address cannot be reached) is "offline"; the author's site answering without the file is "gone" or "refused"; a file whose
    /// SHA-256 is not the pinned one is "changed" and is not kept. (Finish-2 F09 removed the kind "unpublished": no model is fetched from Stella Scope's own releases now.)
    #[test]
    fn a_failed_fetch_says_what_happened_in_plain_words() {
        let offline = http_get("http://127.0.0.1:1/model.glb.gz").err().expect("nothing listens on port 1");
        assert!(offline.contains("no connection"), "{offline}");
        assert_eq!(failure_words(&offline).0, "offline");
        assert!(failure_words(&offline).1.contains("No connection") && failure_words(&offline).1.contains("try again"));
        assert_eq!(failure_words("https://x/y.glb.gz answered 404").0, "gone");
        assert_eq!(failure_words("https://x/y.glb.gz answered 503").0, "refused");
        assert!(failure_words("https://x/y.glb.gz answered 503").1.contains("503"));
        assert_eq!(failure_words(&refusal("Fixture", &"a".repeat(64), &"b".repeat(64))).0, "changed");
        assert_eq!(failure_words("the download was interrupted: connection reset").0, "interrupted");
        assert_eq!(failure_words("something else entirely").0, "other");
        // No word a farm owner would not use: no URL, no "ureq", no raw error in the plain sentence of an offline file.
        for (_, plain) in [failure_words(&offline), failure_words("https://x/y answered 404")] {
            assert!(!plain.contains("http") && !plain.to_ascii_lowercase().contains("ureq") && !plain.contains("os error"), "{plain}");
        }
    }

    /// The fetch of a farm's models reports each file on its own (queued, fetching, done or failed, with its bytes), in the order they were asked for, and
    /// says how long it took; a file that cannot be had is failed with its kind while the rest arrive. Offline is proved by a route that does not answer
    /// (a closed port on this machine), not by touching the network.
    #[test]
    fn each_file_of_a_fetch_reports_on_its_own_and_offline_is_said_per_file() {
        let root = scratch("items");
        let (a, sa, da) = gz(&vec![b'a'; 50_000]);
        let (b, sb, db) = gz(&vec![b'b'; 40_000]);   // (the test helper makes one stored block: 65,535 bytes at most)
        let src = root.join("src");
        for (id, body) in [("fx_a", &a), ("fx_b", &b)] {
            let d = src.join(format!("machines/{id}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("model.glb.gz"), body).unwrap();
        }
        let lib = Library::new(root.join("printer-models"));
        let rows = vec![row("fx_a", &sa, &da, a.len()), row("fx_missing", &sa, &da, 1_000), row("fx_b", &sb, &db, b.len())];
        start_fetch_rows(&lib, rows, Source::Dir(src), true).unwrap();
        let first = fetch_status(&lib);
        assert_eq!(first["count"], json!(3));
        assert_eq!(first["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect::<Vec<_>>(), ["fx_a", "fx_missing", "fx_b"], "in the order asked");
        let mut seen_states = std::collections::BTreeSet::new();
        let end = loop {
            let st = fetch_status(&lib);
            for i in st["items"].as_array().unwrap() {
                seen_states.insert(i["state"].as_str().unwrap().to_owned());
                assert!(i["got"].as_u64().unwrap() <= i["bytes"].as_u64().unwrap().max(1), "a file's own progress never passes its size: {i}");
            }
            if st["state"] == "done" {
                break st;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let states: Vec<&str> = end["items"].as_array().unwrap().iter().map(|i| i["state"].as_str().unwrap()).collect();
        assert_eq!(states, ["done", "failed", "done"], "{end}");
        assert_eq!(end["finished"], json!(["fx_a", "fx_b"]));
        assert_eq!(end["items"][1]["got"], end["items"][1]["bytes"], "a file that was tried counts as read");
        assert!(end["items"][1]["failure"]["plain"].as_str().unwrap().len() > 20 && end["items"][1]["failure"]["kind"].is_string(), "{end}");
        assert!(end["elapsed_s"].as_f64().unwrap() >= 0.0 && end["current"].is_null());
        // Offline: the source answers nothing at all.
        let lib2 = Library::new(root.join("printer-models-offline"));
        start_fetch_rows(&lib2, vec![row("fx_a", &sa, &da, a.len()), row("fx_b", &sb, &db, b.len())], Source::Http("http://127.0.0.1:1".into()), true).unwrap();
        let off = loop {
            let st = fetch_status(&lib2);
            if st["state"] == "done" {
                break st;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        for i in off["items"].as_array().unwrap() {
            assert_eq!((i["state"].clone(), i["failure"]["kind"].clone()), (json!("failed"), json!("offline")), "{off}");
        }
        assert!(lib2.model_file("fx_a").is_none() && lib2.model_file("fx_b").is_none(), "nothing was kept");
        let _ = fs::remove_dir_all(root);
    }

    /// What each printer's picture waits for: the demo farm on a fresh install. Every printer is listed with its model; the Trident also waits for the two
    /// files of its gantry, the toolchanger for its five tool files; one whose files are here is not listed.
    #[test]
    fn each_demo_printer_knows_what_its_picture_waits_for() {
        let root = scratch("needs");
        let lib = Library::new(root.join("printer-models"));
        let farm: Vec<(String, String, Option<Value>)> = crate::demo::demo_farm().into_iter().filter_map(|e| e.cad_model.clone().map(|m| (e.id.clone(), m, e.cad_options.clone()))).collect();
        let n = needs(&farm, &lib);
        assert_eq!(n.as_object().unwrap().len(), 9, "eight machines and the Prusa MK3S+ (finish-2 F09): {n}");
        let of = |id: &str| -> Vec<String> { n[id].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect() };
        assert_eq!(of("demo-voron-24"), ["voron_v24_350_printed"]);
        assert_eq!(of("demo-core-one-l"), ["prusa_core_one_l"]);
        assert_eq!(of("demo-mk3s"), ["prusa_mk3s"]);
        let trident = of("demo-trident");
        assert_eq!(trident[0], "voron_trident_300");
        assert!(trident.contains(&"trident_r2_gantry_300".to_owned()) && trident.contains(&"trident_r2_sb_carriage".to_owned()) && trident.len() == 3, "{trident:?}");
        let changer = of("demo-toolchanger");
        assert_eq!(changer[0], "voron_v24_300_printed");
        assert_eq!(changer.len(), 6, "its model and five tool files: {changer:?}");
        // A model that is here is no longer waited for.
        let dir = root.join("printer-models/voron_v24_350_printed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("install.json"), "{}").unwrap();
        fs::write(dir.join("model.glb.gz"), b"x").unwrap();
        assert!(needs(&farm, &lib).get("demo-voron-24").is_none());
        let _ = fs::remove_dir_all(root);
    }

    /// The open licences of the demo farm in one line (the owner: one press shows them in one line, then fetches): built from the catalogue, so it names the real
    /// licences and authors, the real size, and the CORE One L's notice; nothing in it is typed.
    #[test]
    fn the_demo_farms_open_licences_are_one_line_built_from_the_catalogue() {
        let farm: Vec<(String, Option<Value>)> = crate::demo::demo_farm().into_iter().filter_map(|e| e.cad_model.clone().map(|m| (m, e.cad_options.clone()))).collect();
        let v = licence_line(&farm);
        let line = v["line"].as_str().unwrap();
        assert!(!line.contains('\n') && line.len() > 120 && line.len() < 600, "one line of plain words: {line}");
        assert!(line.starts_with("Open licences: "), "{line}");
        for must in ["GPL-3.0", "CC-BY-SA-4.0", "VoronDesign", "PrintersForAnts", "SOVOL", "Creality3DPrinting", "SHA-256", "kinematicdigit", "Open Community License", "Prusa", "model is inside Stella Scope", "under About"] {
            assert!(line.contains(must), "the line says {must}: {line}");
        }
        assert!(!line.contains("Copyright") && !line.contains("non-commercial"), "only open files are on it: {line}");
        let bytes = v["bytes"].as_u64().unwrap();
        assert!(line.contains(&format!("{} MB", bytes.div_ceil(1_000_000))) && bytes > 150_000_000 && bytes < 190_000_000, "{bytes}");
        assert_eq!(v["files"], json!(14));
        // Finish-2 F09: the CORE One L and the MK3S+ are inside the build, credited by name on the line instead of fetched after a notice.
        assert_eq!(v["carried"].as_array().unwrap().len(), 2);
        assert_eq!(v["carried"][0]["id"], "prusa_core_one_l");
        assert!(line.contains("MK3S+ model is inside Stella Scope"), "{line}");
    }

    /// F03 item 4: the catalogue never offers a file that cannot be fetched. The live check (every address answers 200 and is the size the catalogue
    /// says) is evidence/F03/catalogue-addresses-*.tsv; this keeps the catalogue from carrying the kinds of address that failed: a folder or a page where
    /// a file should be, or a machine listed with no file whose "from the author" link is a dead page.
    #[test]
    fn no_address_in_the_catalogue_is_a_folder_or_a_page_that_is_not_a_file() {
        let mut files = 0;
        for m in catalogue()["machines"].as_array().unwrap() {
            let (id, kind) = (m["id"].as_str().unwrap(), m["location"]["kind"].as_str().unwrap_or(""));
            let Some(up) = m["location"]["upstream"].as_str() else {
                assert_eq!(kind, "none", "{id} has no address and says it has no file");
                continue;
            };
            assert!(up.starts_with("https://"), "{id}: {up}");
            if kind == "registry" || kind == "release_zip" {
                assert!(up.ends_with("/model.glb.gz") || up.ends_with(".glb.gz") || up.ends_with("/model.glb"), "{id}: a file is a .glb.gz: {up}");
                files += 1;
            } else if kind == "none" {
                // A machine with no file Stella Scope can read points the person at the author's repository, which answers.
                assert!(!up.ends_with('/'), "{id}: the author's page, not a folder of the pages site that answers 404: {up}");
                assert!(m["location"]["why"].as_str().is_some_and(|w| w.len() > 20), "{id} says why it has no file");
            }
        }
        assert!(files > 55, "{files}");
        assert_eq!(
            machine("crossant_235_v06_leadscrew").unwrap()["location"]["upstream"],
            "https://github.com/Psych0h3ad/3d-print-rig-crossant-models"
        );
        for p in catalogue()["parts"].as_object().unwrap().values() {
            let up = p["location"]["upstream"].as_str().unwrap_or("");
            assert!(up.starts_with("https://") && up.ends_with(".glb.gz") || up.ends_with(".glb"), "{}: {up}", p["id"]);
        }
    }

    /// F03 item 2: the Trident's R2 gantry and X carriage are an option of the catalogue with motion registered: the machine lists it, a printer may choose it,
    /// it is placed where the file is written (the modules are in the machine's own frame), and the Trident's profile names the parts that move: the beam
    /// with Y and the carriage with X and Y, while the bed drops with Z.
    #[test]
    fn the_trident_gantry_is_an_option_with_motion_registered() {
        let o = option("trident_r2_gantry").expect("the option is in the catalogue");
        assert_eq!((class_of(o), o["attach"]["kind"].as_str()), ("gpl", Some("place")));
        assert!(machine("voron_trident_300").unwrap()["fits"].as_array().unwrap().iter().any(|x| x == "trident_r2_gantry"));
        assert!(check_selection("voron_trident_300", &json!([{"option":"trident_r2_gantry"}])).is_ok());
        assert!(check_selection("voron_trident_350", &json!([{"option":"trident_r2_gantry"}])).is_err(), "only the 300's module is registered");
        for id in ["trident_r2_gantry_300", "trident_r2_sb_carriage"] {
            let p = part(id).unwrap();
            assert!(may_bundle(class_of(p)) && p["location"]["upstream"].as_str().unwrap().ends_with(&format!("/modules/{id}.glb.gz")), "{id}");
            assert_eq!((p["pin"]["model_sha256"].as_str().unwrap().len(), p["pin"]["stored_model_sha256"].as_str().unwrap().len()), (64, 64));
        }
        let lib = Library::new(std::env::temp_dir().join("stella-no-such-library"));
        let sc = scene("voron_trident_300", &json!([{"option":"trident_r2_gantry"}]), &lib);
        let placed: Vec<&str> = sc["placements"].as_array().unwrap().iter().map(|p| p["part"].as_str().unwrap()).collect();
        assert_eq!(placed, ["trident_r2_gantry_300", "trident_r2_sb_carriage"]);
        assert!(sc["placements"].as_array().unwrap().iter().all(|p| p["translation_mm"] == json!([0.0, 0.0, 0.0]) && p["follows"].is_null()), "placed in the machine's frame, each part in its own group by its record: {sc}");
        let p = profile("voron_trident_300").unwrap();
        for (group, spec) in [("bed", json!([0, 0, {"z": -1}])), ("gantry", json!([0, "y", 0])), ("head", json!(["x", "y", 0]))] {
            assert_eq!(p["motions"][group], spec, "{group}");
            assert!(p["groups"][group].as_array().is_some_and(|a| !a.is_empty()), "{group} holds parts");
        }
        let keys = |g: &str| -> Vec<&str> { p["groups"][g].as_array().unwrap().iter().filter_map(Value::as_str).collect() };
        assert!(keys("gantry").iter().chain(keys("head").iter()).all(|k| k.starts_with("trident_r2_")), "only the modules' own parts");
        assert!(keys("gantry").iter().all(|k| !keys("head").contains(k)), "a part is in one group");
        // A pose moves them: X and Y of the head, Y of the beam.
        let cal = Calibration::starting(p);
        let at = |x: f64, y: f64, z: f64| pose(p, &cal, [Some(x), Some(y), Some(z)]);
        let (a, b) = (at(50.0, 60.0, 5.0), at(200.0, 250.0, 90.0));
        let g = |v: &Value, name: &str| -> [f64; 3] { [0, 1, 2].map(|i| v["groups"][name][i].as_f64().unwrap()) };
        assert_eq!([g(&b, "head")[0] - g(&a, "head")[0], g(&b, "head")[1] - g(&a, "head")[1], g(&b, "head")[2]], [150.0, 190.0, 0.0]);
        assert_eq!([g(&b, "gantry")[0], g(&b, "gantry")[1] - g(&a, "gantry")[1]], [0.0, 190.0]);
        assert_eq!(g(&b, "bed")[2] - g(&a, "bed")[2], -85.0);
    }

    /// F03 item 3: every machine with registered motion has its plate measured, and its nozzle reaches the plate:
    /// the first layer is not held above it and the far corner is inside the travel, for a head that carries X, Y and Z, a bed that carries Z, and a bed that
    /// carries Y.
    #[test]
    fn every_registered_machine_with_a_readable_file_reaches_its_plate() {
        for (id, size) in [
            ("fysetc_v24_250_pro", 250.0),
            ("rook_mk2_120", 120.0),
            ("satsuma180_v10", 180.0),
            ("the100_v11_165", 165.0),
            ("tictac_21_120", 120.0),
            ("annex_k1_assembly", 250.0),
            ("annex_k2_assembly", 350.0),
            ("annex_k3_assembly", 180.0),
        ] {
            let p = profile(id).unwrap_or_else(|| panic!("{id}: no registered motion"));
            assert!(print_frame(id).is_some(), "{id} has a print frame");
            assert!(p["calibration_basis"].as_str().unwrap().starts_with("Measured from the model"), "{id}");
            let cal = Calibration::starting(p);
            for pos in [[0.0, 0.0, 0.2], [size, size, 0.2], [size / 2.0, size / 2.0, 40.0], [0.0, size, 0.2]] {
                let pz = pose(p, &cal, pos.map(Some));
                assert_eq!(pz["clamped"], json!([]), "{id} at {pos:?}: {pz}");
            }
        }
        // Static machines (no registered motion) have no print frame to draw a part on and say so in the model's own sentence (farm::model_json).
        assert!(print_frame("lh_stinger_200").is_none() && profile("lh_stinger_200").is_none());
    }

    /// F03 item 3: the Annex K1, K2 and K3 are written with matrices and with their up along -Y or -Z; their registered profile turns them upright (`orient`, a signed
    /// permutation, the quarter turns the maker's profile names), says which frame the motion is written in (`basis`: the K1's Z is on the third component, a Z-up CAD frame; the
    /// K2's and K3's on the second, the display's up) and makes their solid panels and doors nearly clear (`see_through`). These are the values the page was checked with
    /// (evidence/F03: each machine upright, its part on its plate under its head).
    #[test]
    fn the_annex_machines_are_turned_upright_by_their_profile() {
        let (cad, display) = (json!([[1, 0, 0], [0, 0, 1], [0, -1, 0]]), json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]]));
        for (id, orient, basis) in [
            ("annex_k1_assembly", json!([[1, 0, 0], [0, -1, 0], [0, 0, -1]]), &cad),
            ("annex_k2_assembly", json!([[1, 0, 0], [0, 0, -1], [0, 1, 0]]), &display),
            ("annex_k3_assembly", json!([[1, 0, 0], [0, 0, -1], [0, 1, 0]]), &display),
        ] {
            let p = profile(id).unwrap();
            assert_eq!(p["orient"], orient, "{id}");
            assert_eq!(p["basis"], *basis, "{id}");
            assert!(p["see_through"]["match"].as_str().unwrap().contains("panel") && p["see_through"]["except"].as_str().unwrap().contains("hinge"), "{id}");
            // every part is named by its node's own key (p00000), and the groups hold only such keys
            let keys: Vec<&str> = p["groups"].as_object().unwrap().values().flat_map(|g| g.as_array().unwrap().iter().filter_map(Value::as_str)).collect();
            assert!(keys.len() > 1_000 && keys.iter().all(|k| k.starts_with('p') && k[1..].bytes().all(|b| b.is_ascii_digit())), "{id}: {} keys", keys.len());
            // the plate is measured and the print frame turns with the basis: printer X is the display's X, Y the display's back, Z its up
            let f = print_frame(id).unwrap_or_else(|| panic!("{id} has a print frame"));
            assert!(f["nozzle_mm"].is_array() && f["basis"].as_str().unwrap().contains("slab"), "{id}");
        }
        // The page applies none of this to a machine that does not name it.
        assert!(profile("voron_v24_350_printed").unwrap().get("orient").is_none() && profile("rook_mk2_120").unwrap().get("see_through").is_none());
    }

    /// F03 item 5: the one IDEX machine in the catalogue whose file carries motion (the SnakeOil XY IDEX; every Rat Rig IDEX file is one static heap of 3,049 parts with no
    /// part record) has both carriages registered. The printing tool's carriage follows the head, put where its own nozzle stands (the 237 mm between the two nozzles); the other
    /// waits at its end of the X travel, as the demo printer reports its idle tool. The nozzle of whichever tool prints is on the printer's own coordinates.
    #[test]
    fn the_idex_machine_moves_both_carriages_and_parks_the_one_that_is_not_printing() {
        let p = profile("snakeoil_xy_idex").expect("registered motion");
        assert_eq!((p["motions"]["head"].clone(), p["motions"]["head2"].clone()), (json!(["x", "y", 0]), json!(["x", "y", 0])));
        let tools = p["tools"].as_array().unwrap();
        assert_eq!(tools.iter().map(|t| t["group"].as_str().unwrap()).collect::<Vec<_>>(), ["head", "head2"]);
        for g in ["head", "head2", "beam", "bed"] {
            assert!(p["groups"][g].as_array().is_some_and(|a| a.len() > 50), "{g}");
        }
        let f = print_frame("snakeoil_xy_idex").unwrap();
        let cal = Calibration::starting(p);
        let n0 = [0, 1, 2].map(|i| f["nozzle_mm"][i].as_f64().unwrap());
        let rest = [0, 1, 2].map(|i| p["rest_offsets_mm"]["head2"][i].as_f64().unwrap());
        assert!(rest[0].abs() > 200.0 && rest[0].abs() < 260.0 && rest[1].abs() < 1.0 && rest[2].abs() < 1.0, "the second nozzle is about 237 mm from the first, at the same height: {rest:?}");
        let g = |v: &Value, name: &str| -> [f64; 3] { [0, 1, 2].map(|i| v["groups"][name][i].as_f64().unwrap()) };
        for pos in [[125.0, 100.0, 5.0], [30.0, 200.0, 40.0]] {
            let (a, b) = (pose_for_tool(p, &cal, pos.map(Some), Some(0)), pose_for_tool(p, &cal, pos.map(Some), Some(1)));
            // Tool 0 prints: its carriage is the head's, the other waits at the far end; tool 1 prints: the other way round.
            let (h0, h1) = (g(&a, "head"), g(&b, "head2"));
            assert!((h0[0] - (pos[0] + cal.offset_mm[0])).abs() < 1e-6, "{h0:?}");
            assert!((h1[0] - (pos[0] + cal.offset_mm[0] + rest[0])).abs() < 1e-6, "the second carriage is the head's, moved by the nozzle difference: {h1:?}");
            let park = |t: usize| tools[t]["park"]["x"].as_f64().unwrap();
            assert!((g(&b, "head")[0] - park(0)).abs() < 1e-6 && (g(&a, "head2")[0] - (park(1) + rest[0])).abs() < 1e-6, "the idle carriage waits at its end: {a} {b}");
            // The printing nozzle is where the file asks, whichever tool it is: the first nozzle is where the frame says, the second is the first less their
            // difference, and each goes with its own carriage (the plate goes with the bed).
            let want = [0, 1, 2].map(|c| f["origin_mm"][c].as_f64().unwrap() + (0..3).map(|i| pos[i] * f["axes"][i][c].as_f64().unwrap()).sum::<f64>());
            let bed = g(&a, "bed");
            for (tool, nozzle, carriage) in [(0, n0, g(&a, "head")), (1, [0, 1, 2].map(|c| n0[c] - rest[c]), g(&b, "head2"))] {
                for c in 0..3 {
                    let at_pose = nozzle[c] + carriage[c] - bed[c];
                    assert!((at_pose - want[c]).abs() < 0.6, "tool {tool}, axis {c}: the nozzle is at {at_pose:.2} and the part at {:.2}", want[c]);
                }
            }
        }
    }
}
