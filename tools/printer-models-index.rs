// Printer-model catalogue builder (farm board WO-10, 2026-10-12).
//
//   cargo run --release --example printer-models-index -- RIG_SRC MIRROR OUT.json [TABLES.md]
//     RIG_SRC  a checkout of github.com/Psych0h3ad/3d-print-rig (read only), e.g.
//              ~/claude-reference/3d-print-rig-2026-10-12/src
//     MIRROR   the git --mirror + release mirror, e.g. /media/.../3d-print-rig-2026-10-18 (its viewer-models.zip
//              must be the one site/ASSET_BUNDLE.json pins)
//     OUT      web/printer-models.json (read first: what is new or changed is told against it)
//     TABLES   the inventory tables of docs/PRINT-RIG-INVENTORY.md
//
// Reads the 67 machines of `site/assets/machines/index.json`, finds each machine's model files in the
// upstream registries (`site/*_ASSETS.json`) and in the release zips of the mirror, classifies the
// licence from the per-machine NOTICE files (community models) and the documented family terms, and
// writes one catalogue. Nothing is downloaded and nothing in RIG_SRC or MIRROR is changed.
//
// Polish WO-01 (2026-10-18): the catalogue also carries the project's options (toolchangers, toolheads, add-ons)
// and their files, read from the model bundle's registrations (`TOOLCHANGER_BANK.json`, `MACHINE_MODS.json`, the
// component lists, ...) and the notices, each file pinned by its stored and unpacked SHA-256. See the options
// section below.
//
// The licence CLASS decides what a Stella Scope build may carry (`src/printer_models.rs`, `may_bundle`):
// gpl, mit, cc-by and cc-by-sa may be bundled with their notice; nc, mixed, unlicensed and unknown never
// are. Where an assembly joins parts under different terms the stricter one is used.
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[path = "../src/sha256.rs"]
mod sha256;

/// base64url without padding, the alphabet `licence::b64_decode` reads.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn run(cmd: &str, args: &[&str]) -> String {
    let o = Command::new(cmd).args(args).output().unwrap_or_else(|e| panic!("{cmd}: {e}"));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A small JSON file that sits beside a machine's model (its manifest or profile), read from the mirror.
fn companion(mirror: &Path, loc: &Value, name: &str) -> Option<Value> {
    let text = match loc["kind"].as_str()? {
        "registry" => {
            let m = loc["mirror"].as_object()?;
            let dir = m["path"].as_str()?.rsplit_once('/')?.0;
            let g = mirror.join(m["git"].as_str()?);
            run("git", &["-C", g.to_str()?, "show", &format!("HEAD:{dir}/{name}")])
        }
        "release_zip" => {
            let zip = mirror.join(loc["mirror"]["zip"].as_str()?);
            let entry = loc["mirror"]["entry"].as_str()?.rsplit_once('/')?.0.to_owned() + "/" + name;
            run("unzip", &["-p", zip.to_str()?, &entry])
        }
        _ => return None,
    };
    serde_json::from_str(&text).ok()
}

/// "License: GPL-3.0" from a NOTICE.txt, or the first licence-looking phrase.
fn notice_licence(text: &str) -> Option<String> {
    for l in text.lines() {
        if let Some(r) = l.strip_prefix("License:").or_else(|| l.strip_prefix("Licence:")) {
            return Some(r.trim().trim_end_matches('.').to_owned());
        }
        if let Some(i) = l.find("GPL-3.0") {
            if l.contains("Copyright") {
                return Some(l[i..i + 7].to_owned());
            }
        }
    }
    None
}

fn class_of(licence: &str) -> &'static str {
    let l = licence.to_ascii_lowercase();
    // 2026-10-18 (polish WO-01): the option files carry their own terms, in English and in Japanese; a
    // statement that no licence was found is a class of its own, never read as the licence it names in passing.
    if ["not found", "not stated", "no explicit", "no unrelated", "表記なし", "確認できず", "明記なし"].iter().any(|p| l.contains(p)) {
        "unlicensed"
    } else if l.contains("annex engineering license") || l.contains("eula") || l.contains("personal use only") || l.contains("personal-use") || l.contains("(mixed)") || l.starts_with("mixed") || l.contains("component-specific") {
        "mixed"
    } else if l.contains("nc") && (l.contains("cc-by-nc") || l.contains("cc by-nc") || l.contains("cc by nc") || l.contains("noncommercial")) {
        "nc"
    } else if l.contains("cc0") {
        "cc0"
    } else if l.contains("gpl") {
        "gpl"
    } else if l.contains("cc-by-sa") || l.contains("cc by-sa") || l.contains("by-sa") {
        "cc-by-sa"
    } else if l.contains("cc-by") || l.contains("cc by") {
        "cc-by"
    } else if l.starts_with("mit") || l.contains("(mit") || l.contains(" mit") {
        "mit"
    } else if l.contains("cern-ohl") {
        "cc-by-sa"
    } else {
        "unlicensed"
    }
}

/// Stricter first: the class a set of parts carries is the strictest of theirs.
const STRICT: [&str; 9] = ["unlicensed", "mixed", "nc", "ocl", "cc-by-sa", "gpl", "cc-by", "mit", "cc0"];
fn strictest<'a>(classes: impl Iterator<Item = &'a str>) -> &'static str {
    classes.filter_map(|c| STRICT.iter().position(|s| *s == c)).min().map_or("unlicensed", |i| STRICT[i])
}

// ------------------------------------------------------------------------------------------ options
// Polish board WO-01 (2026-10-18). The catalogue now carries the options of the 3D Print Rig: toolchangers,
// toolheads and add-ons. Every option is a set of PARTS (one GLB each, pinned by SHA-256 with its own licence)
// and, for a machine it fits, a placement taken from the rig's own registrations (`TOOLCHANGER_BANK.json`,
// `MACHINE_HEAD_REGISTRATIONS.json`, `MACHINE_MODS.json`, ...). The arithmetic that puts a head on a machine is
// a port of the rig's `installedHeadPlan` and `bankPlan` (site/viewer/machine-head-model.mjs,
// changer-bank-model.mjs, MIT), done once here so the service never has to read the rig's JavaScript.
use std::{cell::RefCell, process::Stdio};

#[path = "../src/gzip.rs"]
mod gzip;

const PAGES: &str = "https://psych0h3ad.github.io";
const BUNDLE_REL: &str = "releases/3d-print-rig/viewer-trident-deck-103/viewer-models.zip";

fn st(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn n3(v: &Value) -> [f64; 3] {
    [0, 1, 2].map(|i| v[i].as_f64().unwrap_or(0.0))
}
fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn j3(a: [f64; 3]) -> Value {
    json!([a[0], a[1], a[2]])
}

/// A readable label from an id when the rig's own label is missing or not in plain English.
fn humanise(id: &str) -> String {
    let t = id.replace(['_', '-'], " ");
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
/// A label for a file from its id: the rig names its modules `changer_sc_dock_xol`, which reads as
/// "StealthChanger dock for xol" here.
fn part_label(id: &str) -> String {
    for (prefix, words) in [
        ("changer_sc_bank_fasteners", "StealthChanger bank fasteners"),
        ("changer_sc_core_", "StealthChanger shuttle "),
        ("changer_sc_dock_", "StealthChanger dock for "),
        ("changer_sc_back_", "StealthChanger backplate for "),
        ("changer_sc_", "StealthChanger "),
        ("changer_tap_", "TapChanger "),
        ("changer_yudx_", "YUDX "),
        ("head_sc_", "StealthChanger backplate "),
        ("head_indx_", "INDX head "),
        ("indx_passive_tool_", "INDX passive tool "),
        ("indx_", "INDX "),
        ("fixed_sc_", "Fixed-gantry StealthChanger "),
        ("madmax_", "MadMax "),
        ("component_", ""),
        ("head_", "Head "),
        ("v0mod_", "V0 "),
    ] {
        if let Some(rest) = id.strip_prefix(prefix) {
            let tail = rest.replace(['_', '-'], " ");
            return if words.is_empty() { humanise(&tail) } else if words.ends_with(' ') { format!("{words}{tail}") } else { words.to_owned() };
        }
    }
    humanise(id)
}

fn english(label: &str, id: &str) -> String {
    let plain = |c: char| c.is_ascii() || "·–—’×µ°é".contains(c);
    let head: String = label.chars().take_while(|c| plain(*c)).collect();
    let head = head.trim_end_matches([' ', '·', '/', '-']).trim().to_owned();
    if head.chars().count() >= 4 {
        head
    } else {
        humanise(id)
    }
}

/// The few phrases the rig states in Japanese inside a licence line, in plain English.
fn anglicise(t: &str) -> String {
    t.replace("配布条件の明記を取得資料で確認できず", "No redistribution terms are stated in the source the rig obtained")
        .replace("メーカー配布資料：再配布条件の明記なし", "Manufacturer download; no redistribution terms are stated")
        .replace("（別条件）", " (separate terms)")
        .replace("（部品別例外あり）", " (some parts have their own terms)")
}

#[derive(Clone)]
struct Lic {
    texts: Vec<String>,
    author: String,
    source: String,
    basis: String,
}
impl Lic {
    fn class(&self) -> &'static str {
        if self.texts.is_empty() {
            "unlicensed"
        } else {
            strictest(self.texts.iter().map(|t| class_of(t)))
        }
    }
    fn text(&self) -> String {
        if self.texts.is_empty() {
            "not stated by the author".into()
        } else {
            anglicise(&self.texts.join(" + "))
        }
    }
    fn merge(&mut self, o: &Lic) {
        for t in &o.texts {
            if !self.texts.contains(t) {
                self.texts.push(t.clone());
            }
        }
        if self.author.is_empty() {
            self.author = o.author.clone();
        }
        if self.source.is_empty() {
            self.source = o.source.clone();
        }
        if !self.basis.contains(&o.basis) {
            self.basis = format!("{}; {}", self.basis, o.basis);
        }
    }
    fn json(&self) -> Value {
        json!({"class":self.class(),"text":self.text(),"author":self.author,"basis":self.basis,"source":self.source})
    }
}

/// The licence statements a module's own metadata carries: `license`, `source_license`, and every source record's.
fn meta_lic(meta: &Value, basis: &str) -> Lic {
    let mut texts: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        let t = t.trim();
        if !t.is_empty() && !texts.iter().any(|x| x == t) {
            texts.push(t.to_owned());
        }
    };
    push(st(&meta["license"]));
    push(st(&meta["source_license"]));
    for r in meta["source_records"].as_array().into_iter().flatten() {
        push(st(&r["license"]));
    }
    let repo = [&meta["source_repository"], &meta["source_records"][0]["repository"]].iter().map(|v| st(v)).find(|s| !s.is_empty()).unwrap_or("").to_owned();
    let repo = repo.trim_start_matches("https://github.com/").to_owned();
    let author = if !st(&meta["author"]).is_empty() {
        st(&meta["author"]).to_owned()
    } else if !st(&meta["manufacturer"]).is_empty() {
        st(&meta["manufacturer"]).to_owned()
    } else {
        repo.split('/').next().unwrap_or("").to_owned()
    };
    let source = [&meta["source_url"], &meta["source_records"][0]["source_url"], &meta["source_records"][0]["url"]]
        .iter()
        .map(|v| st(v))
        .find(|s| s.starts_with("http"))
        .map(str::to_owned)
        .unwrap_or_else(|| if repo.contains('/') { format!("https://github.com/{repo}") } else { String::new() });
    Lic { texts, author, source, basis: basis.to_owned() }
}

/// The rig's bundle (`viewer-models.zip` of the pinned release), read through `unzip`.
struct Bundle {
    zip: std::path::PathBuf,
    pins: BTreeMap<String, (u64, String)>,
    cache: RefCell<BTreeMap<String, Option<Value>>>,
}
impl Bundle {
    fn open(mirror: &Path, site: &Path) -> Bundle {
        let zip = mirror.join(BUNDLE_REL);
        let pin = read_json(&site.join("ASSET_BUNDLE.json"));
        let size = fs::metadata(&zip).unwrap_or_else(|e| panic!("{}: {e}", zip.display())).len();
        assert_eq!(Some(size), pin["bytes"].as_u64(), "the mirror's viewer-models.zip is not the one site/ASSET_BUNDLE.json pins");
        let b = Bundle { zip, pins: BTreeMap::new(), cache: RefCell::new(BTreeMap::new()) };
        let assets = b.json("VIEWER_ASSETS.json");
        let pins = assets["files"].as_array().unwrap().iter().map(|f| (st(&f["path"]).to_owned(), (f["bytes"].as_u64().unwrap_or(0), st(&f["sha256"]).to_owned()))).collect();
        Bundle { pins, ..b }
    }
    fn raw(&self, name: &str) -> String {
        run("unzip", &["-p", self.zip.to_str().unwrap(), name])
    }
    fn json(&self, name: &str) -> Value {
        serde_json::from_str(&self.raw(name)).unwrap_or_else(|e| panic!("{name} in the bundle: {e}"))
    }
    fn json_opt(&self, name: &str) -> Option<Value> {
        self.cache.borrow_mut().entry(name.to_owned()).or_insert_with(|| serde_json::from_str(&self.raw(name)).ok()).clone()
    }
    /// SHA-256 of an entry once unpacked (the pin `Library::store` checks a gzipped model against).
    fn decoded_sha(&self, entry: &str) -> String {
        let mut child = Command::new("unzip").args(["-p", self.zip.to_str().unwrap(), entry]).stdout(Stdio::piped()).spawn().expect("unzip");
        let mut h = sha256::Sha256::new();
        let src = child.stdout.take().unwrap();
        gzip::gunzip(src, |c| {
            h.update(c);
            Ok(())
        })
        .unwrap_or_else(|e| panic!("{entry}: {e}"));
        let _ = child.wait();
        h.hex()
    }
}

/// One section of the rig's `docs/THIRD_PARTY_NOTICES.md`: the commits it names and the licence it states.
struct Notice {
    title: String,
    commits: Vec<String>,
    licence: String,
    author: String,
    url: String,
}
/// The notices are the rig's own per-source licence record; a model's metadata names its source commit, and the
/// section that names the same commit says under what terms it was taken.
fn parse_notices(text: &str) -> Vec<Notice> {
    let mut out = Vec::new();
    for sec in text.split("\n## ").skip(1) {
        let title = sec.lines().next().unwrap_or("").trim().to_owned();
        let commits: Vec<String> = sec
            .split(|c: char| !c.is_ascii_hexdigit())
            .filter(|w| w.len() == 40)
            .map(str::to_owned)
            .collect();
        let field = |keys: &[&str]| -> String {
            sec.lines()
                .find_map(|l| keys.iter().find_map(|k| l.strip_prefix(k)))
                .map(|r| r.trim().trim_start_matches(['：', ':']).trim().to_owned())
                .unwrap_or_default()
        };
        let licence = field(&["ライセンス：", "ライセンス:", "License:", "Licence:"]);
        if licence.is_empty() || commits.is_empty() {
            continue;
        }
        let url = field(&["配布元・参考元：", "配布元・参考元:"]);
        out.push(Notice { title, commits, licence, author: field(&["作者：", "作者:"]), url: if url.starts_with("http") { url.split('/').take(5).collect::<Vec<_>>().join("/") } else { String::new() } });
    }
    out
}
fn commits_of(meta: &Value) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in [&meta["source_commit"], &meta["commit"]].into_iter().chain(meta["source_records"].as_array().into_iter().flatten().map(|r| &r["commit"])) {
        if st(c).len() == 40 && !v.iter().any(|x| x == st(c)) {
            v.push(st(c).to_owned());
        }
    }
    v
}


struct Ctx {
    b: Bundle,
    notices: Vec<Notice>,
    parts: BTreeMap<String, Value>,
    lic: BTreeMap<String, Lic>,
    lic_item: BTreeMap<String, Lic>,
    assets: BTreeMap<String, Value>,
    missing: Vec<String>,
}
impl Ctx {
    /// The licence the rig's component lists state for a part, by its id or module name.
    fn component_lic(&self, id: &str) -> Option<Lic> {
        self.lic.get(id).cloned()
    }

    /// A part by its asset id: built once from the registry descriptor (a bundle module, or a file the rig hosts
    /// on its pages), pinned, licensed. `hint` is a licence the caller knows (the registry's own).
    fn part(&mut self, id: &str, hint: Option<&Lic>) -> Option<String> {
        if self.parts.contains_key(id) {
            return Some(id.to_owned());
        }
        let Some(d) = self.assets.get(id).cloned() else {
            self.missing.push(id.to_owned());
            return None;
        };
        let (row, mut lic);
        if d["files"]["model.glb"].is_object() {
            // A file the rig hosts on its pages (a companion repo), pinned in the registry itself.
            let f = &d["files"]["model.glb"];
            let base = st(&d["external"]["base_url"]);
            let upstream = format!("{}{}", base, st(&f["path"]));
            lic = hint.cloned().unwrap_or(Lic { texts: vec![], author: String::new(), source: String::new(), basis: "registry".into() });
            if let Some(c) = self.component_lic(id).or_else(|| self.lic_item.get(st(&d["component_id"])).cloned()) {
                lic.merge(&c);
            }
            row = json!({"id":id,"label":part_label(id),"kind":"part","pin":{"stored_model_sha256":f["sha256"],"model_sha256":f["decoded_sha256"]},
                "location":{"kind":"registry","upstream":upstream},"bytes":f["bytes"],"parts":d["parts"]});
        } else {
            let glb = st(&d["glb"]);
            let stored = format!("{glb}.gz");
            let Some((bytes, sha)) = self.b.pins.get(&stored).cloned() else {
                self.missing.push(format!("{id} (no pin for {stored})"));
                return None;
            };
            let meta_name = st(&d["meta"]);
            let meta = self.b.json_opt(meta_name).unwrap_or(Value::Null);
            lic = meta_lic(&meta, &format!("viewer-models.zip {meta_name}"));
            for c in commits_of(&meta) {
                for n in self.notices.iter().filter(|n| n.commits.contains(&c)) {
                    lic.merge(&Lic { texts: vec![n.licence.clone()], author: n.author.clone(), source: n.url.clone(), basis: format!("docs/THIRD_PARTY_NOTICES.md, '{}'", n.title) });
                }
            }
            if let Some(h) = hint {
                lic.merge(h);
            }
            if let Some(c) = self.component_lic(id) {
                lic.merge(&c);
            }
            let decoded = self.b.decoded_sha(&stored);
            row = json!({"id":id,"label":part_label(id),"kind":"part","pin":{"stored_model_sha256":sha,"model_sha256":decoded},
                "location":{"kind":"release_zip","upstream":format!("{PAGES}/3d-print-rig/{stored}"),"mirror":{"zip":BUNDLE_REL,"entry":stored}},
                "bytes":bytes,"parts":meta["parts"].as_array().map(Vec::len)});
        }
        let mut row = row;
        row["licence"] = lic.json();
        self.parts.insert(id.to_owned(), row);
        Some(id.to_owned())
    }

    fn class_of_parts(&self, ids: &[String]) -> Lic {
        let mut it = ids.iter().filter_map(|i| self.parts.get(i));
        let first = it.next();
        let mut lic = Lic { texts: vec![], author: String::new(), source: String::new(), basis: "strictest of its parts".into() };
        for p in first.into_iter().chain(it) {
            let l = &p["licence"];
            lic.merge(&Lic { texts: vec![st(&l["text"]).to_owned()], author: st(&l["author"]).to_owned(), source: st(&l["source"]).to_owned(), basis: String::new() });
        }
        lic.basis = "strictest of its parts' own statements".into();
        lic
    }
}

fn option_row(ctx: &Ctx, id: &str, kind: &str, category: &str, label: &str, parts: Vec<String>, attach: Value, fits: Map<String, Value>, scope: &str, source: Value) -> Value {
    let mut parts = parts;
    parts.sort();
    parts.dedup();
    let lic = ctx.class_of_parts(&parts);
    let bytes: u64 = parts.iter().filter_map(|p| ctx.parts[p]["bytes"].as_u64()).sum();
    json!({"id":id,"kind":kind,"category":category,"label":label,"licence":lic.json(),"parts":parts,"bytes":bytes,
        "attach":attach,"fits":Value::Object(fits),"scope":scope,"source":source})
}

/// The machine's head registration: where the rig's own data says a head's rail interface sits.
fn target_of<'a>(reg: &'a Value, machine: &str) -> Option<&'a Value> {
    let t = &reg["machines"][machine];
    (t["origin_mm"].is_array() && t["belt_width_mm"].is_number()).then_some(t)
}

/// A head variant as the rig installs it on a machine (`installedHeadPlan`), in the variant's own frame: the
/// machine's shift is added at placement time, so one tool entry serves every machine that takes it.
fn head_plan(v: &Value) -> (String, [f64; 3], Vec<Value>, Vec<Value>) {
    let base = if st(&v["base_asset"]).is_empty() { st(&v["toolhead"]).to_owned() } else { st(&v["base_asset"]).to_owned() };
    let hidden = if v["base_hidden_keys"].is_array() {
        v["base_hidden_keys"].as_array().unwrap().clone()
    } else if st(&v["toolhead"]) == "xol" {
        v["hidden_xol_keys"].as_array().cloned().unwrap_or_default()
    } else {
        v["removed_stock_keys"].as_array().cloned().unwrap_or_default()
    };
    let modules = v["modules"].as_array().map(|m| m.iter().filter(|m| st(&m["id"]) != "trident_r2_gantry_350").cloned().collect()).unwrap_or_default();
    (base, n3(&v["head_translation_mm"]), hidden, modules)
}

fn label_of(list: &Value, id: &str) -> String {
    list.as_array().into_iter().flatten().find(|r| st(&r["id"]) == id).map(|r| english(st(&r["label"]), id)).unwrap_or_else(|| humanise(id))
}

/// StealthChanger and INDX banks (`TOOLCHANGER_BANK.json`): frame-mounted docks with registered parked heads.
fn build_banks(ctx: &mut Ctx, heads: &Value, reg: &Value, bank: &Value, out: &mut Vec<Value>) {
    let bank_machines: Vec<(&String, &Value)> = bank["machines"].as_object().unwrap().iter().filter(|(k, _)| k.as_str() != "toolhead").collect();
    // ---- StealthChanger
    {
        let mut tools = Map::new();
        let mut fits = Map::new();
        let mut parts: Vec<String> = Vec::new();
        let fixtures: Vec<String> = bank["fixture_assets"].as_array().unwrap().iter().filter_map(|f| ctx.part(st(f), None)).collect();
        parts.extend(fixtures.iter().cloned());
        for (mid, m) in &bank_machines {
            if m["bank_permitted"] == false {
                continue;
            }
            let Some(t) = target_of(reg, mid) else { continue };
            let belt = t["belt_width_mm"].as_u64().unwrap_or(6);
            let anchor = reg["sources"][format!("stealthchanger_{belt}")]["origin_mm"].as_array().map(|_| n3(&reg["sources"][format!("stealthchanger_{belt}")]["origin_mm"])).unwrap_or_else(|| n3(&reg["sources"]["stealthchanger"]["origin_mm"]));
            let shift = sub3(n3(&t["origin_mm"]), anchor);
            let mut ids: Vec<String> = Vec::new();
            for p in bank["profiles"].as_array().unwrap() {
                let sel = p["selection"].as_object().unwrap();
                let Some(v) = heads["variants"].as_array().unwrap().iter().find(|v| {
                    st(&v["mount"]) == "stealthchanger" && st(&v["gantry"]) == format!("sc_standard_{belt}") && v["belt_width_mm"].as_u64() == Some(belt) && sel.iter().all(|(k, x)| v[k] == *x)
                }) else { continue };
                let tid = format!("{}_{}_{}{}", st(&v["toolhead"]), st(&v["extruder"]), st(&v["hotend"]), if belt == 6 { String::new() } else { format!("_{belt}mm") });
                ids.push(tid.clone());
                if tools.contains_key(&tid) {
                    continue;
                }
                let (base, bt, hidden, modules) = head_plan(v);
                let base_part = ctx.part(&base, None).unwrap_or_else(|| panic!("head part {base}"));
                parts.push(base_part.clone());
                let dock = ctx.part(st(&p["dock"]), None).unwrap_or_else(|| panic!("dock {}", st(&p["dock"])));
                parts.push(dock.clone());
                assert!(p["dock_part_offsets_mm"].as_object().is_none_or(|o| o.is_empty()), "the rig registers a dock part offset; this port does not apply it");
                let fwd = if belt == 9 { bank["nine_mm_forward_mm"].as_f64().unwrap_or(0.0) } else { 0.0 };
                let park = add3(n3(&p["park_translation_mm"]), [0.0, fwd, 0.0]);
                let mut active = vec![json!({"part":base_part,"translation_mm":j3(bt),"hidden_keys":hidden,"role":"head"})];
                let mut parked = vec![json!({"part":base_part,"translation_mm":j3(add3(bt, park)),"hidden_keys":hidden,"role":"tool"})];
                for mo in &modules {
                    if st(&mo["role"]) == "dock" {
                        continue;
                    }
                    let mp = ctx.part(st(&mo["id"]), None).unwrap_or_else(|| panic!("module {}", st(&mo["id"])));
                    parts.push(mp.clone());
                    let tr = n3(&mo["translation_mm"]);
                    active.push(json!({"part":mp,"translation_mm":j3(tr),"hidden_keys":mo["hidden_keys"].as_array().cloned().unwrap_or_default(),"role":mo["role"]}));
                    // A parked tool carries no shuttle, rail, belt keeper or gantry: only the tool's own modules.
                    if st(&mo["role"]) == "tool" {
                        parked.push(json!({"part":mp,"translation_mm":j3(add3(tr, park)),"hidden_keys":mo["hidden_keys"].as_array().cloned().unwrap_or_default(),"role":"tool"}));
                    }
                }
                tools.insert(tid.clone(), json!({"label":format!("{} / {} / {}", label_of(&heads["toolheads"], st(&v["toolhead"])), label_of(&heads["extruders"], st(&v["extruder"])), label_of(&heads["hotends"], st(&v["hotend"]))),
                    "belt_width_mm":belt,"dock":{"part":dock,"hidden_keys":p["dock_hidden_keys"]},"active":active,"parked":parked,"native_park_body_intersections":p["native_park_body_intersections"].as_array().map(Vec::len)}));
            }
            if ids.is_empty() {
                continue;
            }
            // The rig's own order, so the first tool is the one it offers first.
            let ids: Vec<String> = { let mut seen = Vec::new(); for i in ids { if !seen.contains(&i) { seen.push(i); } } seen };
            fits.insert((*mid).clone(), json!({"capacity":m["capacity"],"pitch_mm":bank["pitch_mm"],"anchor_mm":[m["center_x_mm"],m["translation_mm"][1],m["translation_mm"][2]],
                "head_shift_mm":j3(shift),"stock_head_keys":t["stock_head_keys"],"tools":ids,"datum":m["datum"]}));
        }
        let blocked: Vec<&String> = bank_machines.iter().filter(|(_, m)| m["bank_permitted"] == false).map(|(k, _)| *k).collect();
        out.push(option_row(ctx, "stealthchanger_bank", "toolchanger", "tool bank", "StealthChanger tool bank (frame-mounted docks, parked heads)", parts,
            json!({"kind":"bank","system":"stealthchanger","fixture_parts":fixtures,"tools":tools,"not_offered_on":blocked,
                "not_offered_because":"The standard dock collides with the bed of a Trident at the nozzle plane, and a Trident has no head-side Z move for the original exchange path (CHANGER_BANK.md)."}),
            fits, "A CAD arrangement preview: retaining hardware, cable paths, door fit, travel sweeps and automatic docking are not certified (3D Print Rig docs/CHANGER_BANK.md).",
            json!([{"repository":"DraftShift/StealthChanger","commit":"50e3c769297b273ac390fb33b455aa7f4dfe3099","license":"GPL-3.0"},{"repository":"DraftShift/ModularDock","commit":"597fad34f3a1233ed1c182ec221a8da9ba68ac33","license":"GPL-3.0"}])));
    }
    // ---- INDX
    {
        let ix = &bank["indx"];
        let mut tools = Map::new();
        let mut fits = Map::new();
        let mut parts: Vec<String> = Vec::new();
        let dock = ctx.part(st(&ix["dock_asset"]), None).expect("indx dock");
        let parked_part = ctx.part(st(&ix["parked_asset"]), None).expect("indx parked tool");
        parts.push(dock.clone());
        parts.push(parked_part.clone());
        for (mid, m) in ix["machines"].as_object().unwrap().iter().filter(|(k, _)| k.as_str() != "toolhead") {
            if m["bank_permitted"] == false {
                continue;
            }
            let Some(t) = target_of(reg, mid) else { continue };
            let anchor = n3(&reg["sources"]["indx"]["origin_mm"]);
            let shift = sub3(n3(&t["origin_mm"]), anchor);
            let mut ids = Vec::new();
            for o in ix["tool_options"].as_array().unwrap() {
                let Some(v) = heads["variants"].as_array().unwrap().iter().find(|v| st(&v["toolhead"]) == "indx" && st(&v["hotend"]) == st(&o["id"]) && st(&v["cooling"]) == "4010") else { continue };
                let tid = st(&o["id"]).to_owned();
                ids.push(tid.clone());
                if tools.contains_key(&tid) {
                    continue;
                }
                let (base, bt, hidden, modules) = head_plan(v);
                let base_part = ctx.part(&base, None).unwrap_or_else(|| panic!("indx head {base}"));
                parts.push(base_part.clone());
                let mut active = vec![json!({"part":base_part,"translation_mm":j3(bt),"hidden_keys":hidden,"role":"head"})];
                for mo in &modules {
                    let mp = ctx.part(st(&mo["id"]), None).unwrap_or_else(|| panic!("indx module {}", st(&mo["id"])));
                    parts.push(mp.clone());
                    active.push(json!({"part":mp,"translation_mm":j3(n3(&mo["translation_mm"])),"hidden_keys":mo["hidden_keys"].as_array().cloned().unwrap_or_default(),"role":mo["role"]}));
                }
                // The rig adds the rail fasteners for every INDX head at the machine's shift.
                let rf = ctx.part("head_indx_rail_fasteners", None).expect("indx rail fasteners");
                parts.push(rf.clone());
                active.push(json!({"part":rf,"translation_mm":[0.0,0.0,0.0],"hidden_keys":[],"role":"tool"}));
                tools.insert(tid, json!({"label":st(&o["label"]),"dock":{"part":dock,"hidden_keys":[]},"active":active,
                    "parked":[{"part":parked_part,"translation_mm":[0.0,0.0,0.0],"hidden_keys":[],"role":"tool"}]}));
            }
            if ids.is_empty() {
                continue;
            }
            let fx = ctx.part(st(&m["fixture_asset"]), None).unwrap_or_else(|| panic!("indx crossbar {}", st(&m["fixture_asset"])));
            parts.push(fx.clone());
            fits.insert(mid.clone(), json!({"capacity":m["capacity"],"pitch_mm":ix["pitch_mm"],"anchor_mm":[m["center_x_mm"],m["translation_mm"][1],m["translation_mm"][2]],
                "head_shift_mm":j3(shift),"stock_head_keys":t["stock_head_keys"],"tools":ids,"fixture":{"part":fx,"translation_mm":m["translation_mm"]},"datum":m["datum"]}));
        }
        out.push(option_row(ctx, "indx_bank", "toolchanger", "tool bank", "Bondtech INDX tool bank (passive tools on 2020 docks)", parts,
            json!({"kind":"bank","system":"indx","fixture_parts":[],"tools":tools,"reference_geometry_only":ix["reference_geometry_only"]}),
            fits, "Original 2020 docks, magnets, springs and passive tools at 41 mm pitch; an extra front 2020 crossbar is needed. End fastening, covers, full travel and automatic docking are not verified (3D Print Rig docs/INDX.md).",
            json!([{"repository":"BondtechAB","license":"GPL-3.0"}])));
    }
}

/// Add-ons that sit on a machine at a registered place: `accessories[]` of MACHINE_MODS, REQUESTED_MOD_INSTALLATIONS
/// and the Trident internal spool all use `{id, label, module(s), translation_mm, hidden_stock_keys}`.
fn build_addons(ctx: &mut Ctx, site: &Path, out: &mut Vec<Value>) {
    // option id -> (label, parts, fits, scope, source, extra attach)
    struct Acc {
        label: String,
        parts: Vec<String>,
        fits: Map<String, Value>,
        scope: String,
        source: Value,
        hint: Option<Lic>,
        group: Option<String>,
    }
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    let mut put = |ctx: &mut Ctx, id: &str, label: &str, machine: &str, modules: Vec<String>, tr: Value, hide: Value, fit_extra: Value, scope: &str, source: Value, hint: Option<Lic>, group: Option<String>| {
        let a = accs.entry(id.to_owned()).or_insert_with(|| Acc { label: label.to_owned(), parts: vec![], fits: Map::new(), scope: scope.to_owned(), source, hint: hint.clone(), group: group.clone() });
        let mut place = Vec::new();
        for (i, mname) in modules.iter().enumerate() {
            if let Some(p) = ctx.part(mname, hint.as_ref()) {
                a.parts.push(p.clone());
                place.push(json!({"part":p,"translation_mm":if tr.is_array() { tr.clone() } else { tr[i].clone() },"hidden_keys":[]}));
            }
        }
        let mut fit = json!({"place":place,"hide_machine_keys":hide,"exclusive_group":group});
        if let (Some(o), Some(e)) = (fit.as_object_mut(), fit_extra.as_object()) {
            o.extend(e.clone());
        }
        a.fits.insert(machine.to_owned(), fit);
    };
    // MACHINE_MODS: handles, skirts and the Disco on a Stick, per machine.
    let mm = ctx.b.json("MACHINE_MODS.json");
    for (c, v) in mm["assets"].as_object().unwrap() {
        ctx.assets.entry(c.clone()).or_insert_with(|| v.clone());
    }
    for (mid, m) in mm["machines"].as_object().unwrap() {
        if m["disco"].is_object() {
            let d = &m["disco"];
            let id = "disco_on_a_stick_xxl_350";
            ctx.assets.entry(id.to_owned()).or_insert_with(|| json!({"meta":d["meta"],"glb":d["glb"]}));
            let mut extra = json!({});
            if d["side_translation_mm"].is_object() {
                extra["side_translation_mm"] = d["side_translation_mm"].clone();
            }
            put(ctx, "disco_on_a_stick", "Disco on a Stick (LED bar)", mid, vec![id.into()], d["translation_mm"].clone(), json!([]), extra, "Static CAD at the registered place; lighting is not simulated (3D Print Rig docs/DISCO).", json!([{"license":"see part"}]), None, None);
        }
        for a in m["accessories"].as_array().into_iter().flatten() {
            let mods: Vec<String> = if a["modules"].is_array() { a["modules"].as_array().unwrap().iter().map(|x| st(x).to_owned()).collect() } else { vec![st(&a["module"]).to_owned()] };
            let hint = (!st(&a["license"]).is_empty()).then(|| Lic { texts: vec![st(&a["license"]).to_owned()], author: String::new(), source: st(&a["source_url"]).to_owned(), basis: "MACHINE_MODS.json accessory".into() });
            put(ctx, st(&a["id"]), &english(st(&a["label"]), st(&a["id"])), mid, mods, a["translation_mm"].clone().as_array().map_or(json!([0.0, 0.0, 0.0]), |_| a["translation_mm"].clone()), a["hidden_stock_keys"].clone(), json!({}),
                "Static CAD at the registered place (3D Print Rig MACHINE_MODS.json); fit with every head and travel is not certified.", json!([{"url":a["source_url"],"version":a["source_version"]}]), hint, a["exclusive_group"].as_str().map(str::to_owned));
        }
    }
    // REQUESTED_MOD_INSTALLATIONS: mods hosted by the large-Voron companion repo.
    let rq = read_json(&site.join("REQUESTED_MOD_INSTALLATIONS.json"));
    for (c, v) in rq["assets"].as_object().unwrap() {
        ctx.assets.entry(c.clone()).or_insert_with(|| v.clone());
    }
    for (mid, m) in rq["machines"].as_object().unwrap() {
        for a in m["accessories"].as_array().into_iter().flatten() {
            let id = st(&a["id"]);
            let hint = ctx.lic_item.get(id).cloned();
            let extra = json!({"requires_gantry":a["gantry_ids"],"conflicts":a["conflicts"],"checked_with":a["compatible_selection"]});
            put(ctx, id, &english(st(&a["label"]), id), mid, vec![st(&a["module"]).to_owned()], a["translation_mm"].clone(), json!([]), extra,
                "Geometry preview at the registered place; whole-machine fit is not certified (3D Print Rig REQUESTED_MOD_INSTALLATIONS.json).", json!([{"registry":"REQUESTED_MOD_INSTALLATIONS.json"}]), hint, None);
        }
    }
    // The Trident internal spool holder.
    let sp = read_json(&site.join("TRIDENT_INTERNAL_SPOOL.json"));
    for (c, v) in sp["assets"].as_object().unwrap() {
        ctx.assets.entry(c.clone()).or_insert_with(|| v.clone());
    }
    let sp_lic = Lic { texts: vec![st(&sp["source"]["license"]).to_owned()], author: "elcrni".into(), source: st(&sp["source"]["repository"]).to_owned(), basis: format!("TRIDENT_INTERNAL_SPOOL.json source (commit {})", st(&sp["source"]["commit"])) };
    for (mid, m) in sp["machines"].as_object().unwrap() {
        put(ctx, "trident_internal_spool", "Trident internal spool holder (elcrni)", mid, vec![st(&m["module"]).to_owned()], m["translation_mm"].clone(), json!([]),
            json!({"whole_machine_certified":sp["whole_machine_certified"],"filament_route_registered":sp["filament_route_registered"]}),
            "Rigid holder and guides only: the filament route and the whole machine are not certified (3D Print Rig TRIDENT_INTERNAL_SPOOL.json).", json!([sp["source"].clone()]), Some(sp_lic.clone()), None);
    }
    for (id, a) in accs {
        let attach = json!({"kind":"place","exclusive_group":a.group});
        out.push(option_row(ctx, &id, "addon", "add-on", &a.label, a.parts, attach, a.fits, &a.scope, a.source));
        let _ = a.hint;
    }
}

/// Standalone toolchanger systems (`TOOLCHANGER_CONFIGURATIONS.json`): shuttle, backplate and dock modules at their registered offsets.
fn build_changer_systems(ctx: &mut Ctx, heads: &Value, out: &mut Vec<Value>) {
    let tc = ctx.b.json("TOOLCHANGER_CONFIGURATIONS.json");
    for (c, v) in tc["assets"].as_object().unwrap() {
        ctx.assets.entry(c.clone()).or_insert_with(|| v.clone());
    }
    for v in tc["variants"].as_array().unwrap() {
        let id = st(&v["id"]);
        let mut place = Vec::new();
        let mut parts = Vec::new();
        for m in v["modules"].as_array().unwrap() {
            let Some(p) = ctx.part(st(&m["id"]), None) else { continue };
            parts.push(p.clone());
            place.push(json!({"part":p,"translation_mm":m["translation_mm"],"role":m["role"],"hidden_keys":m["hidden_keys"].as_array().cloned().unwrap_or_default()}));
        }
        let system = st(&v["system"]);
        let label = format!("{} · {} · {}", match system { "stealthchanger" => "StealthChanger", "tapchanger" => "TapChanger", "yudx" => "YUDX", "fixed_gantry_sc" => "Fixed-gantry StealthChanger", s => s }, st(&v["interface"]).replace('_', " "), if st(&v["toolhead"]).is_empty() { humanise(id.trim_start_matches(system).trim_start_matches('_')) } else { label_of(&heads["toolheads"], st(&v["toolhead"])) });
        out.push(option_row(ctx, id, "toolchanger", "changer system", &label, parts,
            json!({"kind":"standalone","system":system,"interface":v["interface"],"toolhead":v["toolhead"],"place":place,"native_fit":v["native_fit"]}),
            Map::new(), "A reference assembly shown on its own. Installation on a whole printer is not registered in the rig for this system (docs/STEALTHCHANGER_MOUNTS.md, CHANGER_BANK.md).",
            json!(tc["sources"])));
    }
}

/// The rig's component lists: hotends, extruders, carriages, gantries, electronics and mods, each with the licence
/// and source the rig states for it. They are shown on their own (no machine registers their installation).
fn build_components(ctx: &mut Ctx, site: &Path, out: &mut Vec<Value>) {
    let lib = ctx.b.json("COMPONENT_LIBRARY.json");
    let add = read_json(&site.join("COMPONENT_ADDITIONS.json"));
    for (c, v) in lib["assets"].as_object().unwrap().iter().chain(add["assets"].as_object().unwrap()) {
        ctx.assets.entry(c.clone()).or_insert_with(|| v.clone());
    }
    let mut seen = std::collections::BTreeSet::new();
    for (list, from) in [(&lib["items"], "COMPONENT_LIBRARY.json"), (&add["items"], "COMPONENT_ADDITIONS.json")] {
        for it in list.as_array().unwrap() {
            let id = st(&it["id"]);
            let module = st(&it["module"]);
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let hint = Lic { texts: if st(&it["license"]).is_empty() { vec![] } else { vec![st(&it["license"]).to_owned()] }, author: st(&it["manufacturer"]).to_owned(), source: if st(&it["source_url"]).starts_with("http") { st(&it["source_url"]).to_owned() } else { String::new() }, basis: format!("{from} item {id}") };
            let Some(p) = ctx.part(module, Some(&hint)) else { continue };
            let cat = st(&it["kind"]);
            let (kind, category) = match cat {
                "hotend" | "extruder" | "carriage" | "electronics" => ("toolhead", cat),
                _ => ("addon", if cat.is_empty() { "mod" } else { cat }),
            };
            out.push(option_row(ctx, &if id.starts_with("component_") { id.to_owned() } else { format!("component_{id}") }, kind, category, &english(st(&it["label"]), id), vec![p],
                json!({"kind":"standalone","place":[{"part":module,"translation_mm":[0.0,0.0,0.0]}],"views":it["views"].as_array().map(Vec::len)}),
                Map::new(), "A source part shown on its own; the rig registers no installation of it on a printer.", json!([{"manufacturer":it["manufacturer"],"source":it["source_url"],"version":it["source_version"],"licence_url":it["license_url"]}])));
        }
    }
}

/// Toolhead assemblies (`TOOLHEAD_CONFIGURATIONS.json` base assets): one option per head file, with the machines
/// the rig's own installation rules (`machineHeadVariants`) accept it on. Stella draws only the heads a bank uses.
fn build_toolheads(ctx: &mut Ctx, heads: &Value, reg: &Value, out: &mut Vec<Value>) {
    let mut users: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for v in heads["variants"].as_array().unwrap() {
        users.entry(st(&v["toolhead"]).to_owned()).or_default().push(v);
    }
    let machines: Vec<(&String, &Value)> = reg["machines"].as_object().unwrap().iter().filter(|(_, t)| t["belt_width_mm"].is_number()).collect();
    // The rig's rule for what is installed on a machine (machine-head-model.mjs, machineHeadVariants), for 6 and 9 mm belts.
    let installed = |v: &Value, belt: u64| -> bool {
        let (th, mount, src) = (st(&v["toolhead"]), st(&v["mount"]), st(&v["registration_source"]));
        v["belt_width_mm"].as_u64() == Some(belt)
            && ((mount == "fixed" && ["trident_r2", "siboor_awd"].contains(&st(&v["gantry"])) && st(&v["carriage"]) == "standard" && ["stealthburner", "xol"].contains(&th))
                || (th == "crowncooler" && src == "crowncooler" && belt == 6)
                || (th == "a4t" && mount == "fixed" && st(&v["carriage"]) == "a4t_xol" && src == format!("a4t_xol_carriage_{belt}") && reg["sources"][src].is_object())
                || (th == "yavoth" && mount == "fixed" && st(&v["carriage"]) == "yavoth_mgn12" && ["source", "rear5"].contains(&st(&v["cooling"])) && src == format!("yavoth_mgn12_{belt}_{}", st(&v["cooling"])) && reg["sources"][src].is_object())
                || (th == "sphinx" && (src == "sphinx_voron" || (src.starts_with("sphinx_v3_") && reg["sources"][src].is_object())) && belt == 6)
                || (["filamatrix", "madmax"].contains(&mount) && ["filamatrix", "madmax_xol"].contains(&src) && belt == 6)
                || (mount == "stealthchanger" && st(&v["gantry"]) == format!("sc_standard_{belt}") && ["stealthburner", "xol", "jabberwocky"].contains(&th))
                || (th == "indx" && belt == 6))
    };
    for (family_id, vs) in users {
        let mut bases: Vec<String> = vs.iter().map(|v| if st(&v["base_asset"]).is_empty() { st(&v["toolhead"]).to_owned() } else { st(&v["base_asset"]).to_owned() }).collect();
        bases.sort();
        bases.dedup();
        let parts: Vec<String> = bases.iter().filter_map(|b| ctx.part(b, None)).collect();
        if parts.is_empty() {
            continue;
        }
        let mut fits = Map::new();
        for (mid, t) in &machines {
            let belt = t["belt_width_mm"].as_u64().unwrap_or(6);
            let n = vs.iter().filter(|v| installed(v, belt)).count();
            if n > 0 {
                fits.insert((*mid).clone(), json!({"registered_variants":n,"belt_width_mm":belt}));
            }
        }
        let label = format!("{} toolhead", label_of(&heads["toolheads"], &family_id));
        out.push(option_row(ctx, &format!("head_{family_id}"), "toolhead", "head family", &label, parts,
            json!({"kind":"registered","toolhead":family_id,"variants":vs.len(),"base_files":bases.len(),"drawn_by_stella":false,
                "note":"The rig registers this head, with each hotend and extruder it offers, on the machines listed in fits. Stella Scope draws a head only as part of a tool bank for now."}),
            fits, "Installed-head registrations are the rig's own (MACHINE_HEAD_REGISTRATIONS.json); head clearance, wiring and full travel are not certified.",
            json!([])));
    }
}


/// Build every option and its parts from the rig's registrations; also the E3NG toolchanger, a machine of the
/// rig that is not among the 67 of `site/assets/machines/index.json`.
fn build_options(rig: &Path, mirror: &Path) -> (Vec<Value>, BTreeMap<String, Value>, Value, Option<Value>, Vec<String>) {
    let site = rig.join("site");
    let b = Bundle::open(mirror, &site);
    let mut heads = b.json("TOOLHEAD_CONFIGURATIONS.json");
    let additions = read_json(&site.join("HEAD_ADDITIONS.json"));
    for k in ["toolheads", "variants", "hotends", "extruders", "probes"] {
        if let (Some(h), Some(a)) = (heads[k].as_array_mut(), additions[k].as_array()) {
            h.extend(a.iter().cloned());
        }
    }
    if let (Some(h), Some(a)) = (heads["assets"].as_object_mut(), additions["assets"].as_object()) {
        h.extend(a.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let reg = b.json("MACHINE_HEAD_REGISTRATIONS.json");
    let bank = b.json("TOOLCHANGER_BANK.json");
    let mut assets: BTreeMap<String, Value> = BTreeMap::new();
    for src in [&heads["assets"], &heads["base_assets"], &bank["assets"], &reg["assets"]] {
        for (k, v) in src.as_object().into_iter().flatten() {
            assets.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let notices = parse_notices(&fs::read_to_string(rig.join("docs/THIRD_PARTY_NOTICES.md")).expect("docs/THIRD_PARTY_NOTICES.md"));
    let (mut lic, mut lic_item) = (BTreeMap::new(), BTreeMap::new());
    let lib = b.json("COMPONENT_LIBRARY.json");
    let add = read_json(&site.join("COMPONENT_ADDITIONS.json"));
    for (list, from) in [(&lib["items"], "COMPONENT_LIBRARY.json"), (&add["items"], "COMPONENT_ADDITIONS.json")] {
        for it in list.as_array().unwrap() {
            let l = Lic {
                texts: if st(&it["license"]).is_empty() { vec![] } else { vec![st(&it["license"]).to_owned()] },
                author: st(&it["manufacturer"]).to_owned(),
                source: if st(&it["source_url"]).starts_with("http") { st(&it["source_url"]).to_owned() } else { String::new() },
                basis: format!("{from} item {}", st(&it["id"])),
            };
            if !l.texts.is_empty() {
                lic.insert(st(&it["module"]).to_owned(), l.clone());
                lic_item.insert(st(&it["id"]).to_owned(), l);
            }
        }
    }
    let mut ctx = Ctx { b, notices, parts: BTreeMap::new(), lic, lic_item, assets, missing: vec![] };
    let mut options = Vec::new();
    build_banks(&mut ctx, &heads, &reg, &bank, &mut options);
    build_addons(&mut ctx, &site, &mut options);
    build_changer_systems(&mut ctx, &heads, &mut options);
    build_components(&mut ctx, &site, &mut options);
    build_toolheads(&mut ctx, &heads, &reg, &mut options);
    let e3ng = e3ng_row(&ctx);
    let pin = read_json(&site.join("ASSET_BUNDLE.json"));
    let info = json!({"url":pin["url"],"bytes":pin["bytes"],"sha256":pin["sha256"],"tag":"viewer-trident-deck-103","pages":format!("{PAGES}/3d-print-rig/")});
    (options, ctx.parts, info, e3ng, ctx.missing)
}

/// The E3NG hotend changer and its dedicated gantry (`machines/e3ng_toolchanger` of the bundle). The rig says it is
/// "not the complete printer"; it is licensed CC BY-NC-SA 4.0 by its author, so Stella Scope never carries it.
fn e3ng_row(ctx: &Ctx) -> Option<Value> {
    let entry = "machines/e3ng_toolchanger/model.glb.gz";
    let (bytes, stored) = ctx.b.pins.get(entry)?.clone();
    let man = ctx.b.json_opt("machines/e3ng_toolchanger/assembly_manifest.json")?;
    let decoded = ctx.b.decoded_sha(entry);
    let lic = Lic {
        texts: vec![st(&man["source_license"]).to_owned()],
        author: "emna2881 (E3NG-Toolchanger), designed by RH3D".into(),
        source: format!("https://github.com/{}", st(&man["source_repository"])),
        basis: format!("machines/e3ng_toolchanger/assembly_manifest.json source_license; docs/THIRD_PARTY_NOTICES.md 'E3NG hotend changer' (commit {})", st(&man["source_commit"])),
    };
    let mut files = Map::new();
    files.insert("model.glb.gz".into(), json!({"path":entry,"bytes":bytes,"sha256":stored}));
    files.insert("assembly_manifest.json".into(), json!({"path":"machines/e3ng_toolchanger/assembly_manifest.json"}));
    Some(json!({"id":"e3ng_toolchanger","label":"E3NG hotend changer · 230 / Native CAD (changer and gantry, not the complete printer)","family":"e3ng","size_mm":230,"thumbnail":null,"thumbnail_sha256":null,
        "licence":lic.json(),"pin":{"model_sha256":decoded,"stored_model_sha256":stored,"decoded_sha256_registry":null,"stored_bytes":bytes},
        "location":{"kind":"release_zip","mirror":{"zip":BUNDLE_REL,"entry":entry,"bytes":bytes},"upstream":format!("{PAGES}/3d-print-rig/{entry}")},
        "files":files,"motion":false,"triangles":man["triangles"],"parts":man["parts"].as_array().map(Vec::len),"basis":null,"origin_mm":null,
        "note":"The rig draws this reference with a selector (one of two gantries, one to six tools, board and extruder choices). Stella Scope shows every part of the file at once, so alternates overlap; it is a reference, not a drawing of one build."}))
}

/// The inventory tables for docs/PRINT-RIG-INVENTORY.md, and what is new or changed against the last catalogue.
fn inventory_md(doc: &Value, prev: Option<&Value>) -> String {
    let mut s = String::new();
    let machines = doc["machines"].as_array().unwrap();
    let opts = doc["options"].as_array().unwrap();
    s += &format!("## Machines ({})\n\n| id | label | family | bed mm | licence class | motion | model file | options that fit |\n|---|---|---|---|---|---|---|---|\n", machines.len());
    for m in machines {
        let loc = &m["location"];
        let file = match loc["kind"].as_str() {
            Some("registry") => loc["upstream"].as_str().unwrap_or("").trim_start_matches("https://psych0h3ad.github.io/").to_owned(),
            Some("release_zip") => format!("viewer-models.zip {}", loc["mirror"]["entry"].as_str().unwrap_or("")),
            Some("maker") => "maker's own CAD".into(),
            _ => "no model file published".into(),
        };
        s += &format!("| `{}` | {} | {} | {} | {} | {} | `{}` | {} |\n", st(&m["id"]), st(&m["label"]).replace('|', "/"), st(&m["family"]), m["size_mm"], st(&m["licence"]["class"]), if m["motion"] == true { "yes" } else { "no" }, file, m["fits"].as_array().map_or(0, Vec::len));
    }
    for (kind, title) in [("toolchanger", "Toolchangers"), ("toolhead", "Toolheads"), ("addon", "Add-ons")] {
        let rows: Vec<&Value> = opts.iter().filter(|o| st(&o["kind"]) == kind).collect();
        s += &format!("\n## {title} ({})\n\n| id | what | category | licence class | files | MB | fits (machines) | drawn by Stella |\n|---|---|---|---|---|---|---|---|\n", rows.len());
        for o in rows {
            let drawn = matches!(st(&o["attach"]["kind"]), "bank" | "place") && o["fits"].as_object().is_some_and(|f| !f.is_empty());
            s += &format!("| `{}` | {} | {} | {} | {} | {:.1} | {} | {} |\n", st(&o["id"]), st(&o["label"]).replace('|', "/"), st(&o["category"]), st(&o["licence"]["class"]), o["parts"].as_array().map_or(0, Vec::len), o["bytes"].as_f64().unwrap_or(0.0) / 1048576.0, o["fits"].as_object().map_or(0, Map::len), if drawn { "yes" } else { "no" });
        }
    }
    s += "\n## New or changed since the 2026-10-12 catalogue\n\n";
    match prev {
        None => s += "No earlier catalogue was given.\n",
        Some(p) => {
            let old: BTreeMap<&str, &Value> = p["machines"].as_array().into_iter().flatten().map(|m| (st(&m["id"]), m)).collect();
            let mut changes = 0;
            for m in machines {
                match old.get(st(&m["id"])) {
                    None => {
                        s += &format!("- NEW machine `{}` ({}, licence class {}).\n", st(&m["id"]), st(&m["label"]), st(&m["licence"]["class"]));
                        changes += 1;
                    }
                    Some(o) => {
                        let mut diff = Vec::new();
                        for k in ["pin", "licence", "size_mm", "label", "family", "motion", "triangles", "parts", "basis", "origin_mm"] {
                            if o[k] != m[k] {
                                diff.push(k);
                            }
                        }
                        if o["location"]["kind"] != m["location"]["kind"] || o["location"]["mirror"] != m["location"]["mirror"] {
                            diff.push("location");
                        }
                        if o["location"]["upstream"] != m["location"]["upstream"] {
                            diff.push("upstream address");
                        }
                        if !diff.is_empty() {
                            s += &format!("- CHANGED `{}`: {}.\n", st(&m["id"]), diff.join(", "));
                            changes += 1;
                        }
                    }
                }
            }
            for id in old.keys() {
                if !machines.iter().any(|m| st(&m["id"]) == *id) {
                    s += &format!("- REMOVED machine `{id}`.\n");
                    changes += 1;
                }
            }
            if changes == 0 {
                s += "No machine was added, removed or changed.\n";
            }
            s += &format!("- {} options and {} parts are new: the earlier catalogue carried machines only.\n", opts.len(), doc["parts"].as_object().map_or(0, Map::len));
        }
    }
    s
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (rig, mirror, out) = (Path::new(&a[0]), Path::new(&a[1]), Path::new(&a[2]));
    let site = rig.join("site");
    let index = read_json(&site.join("assets/machines/index.json"));
    let machines = index["machines"].as_array().expect("machines");

    // Upstream registries: machine id -> (registry file, base url, files)
    let mut reg: BTreeMap<String, (String, String, Value)> = BTreeMap::new();
    for e in fs::read_dir(&site).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.ends_with("_ASSETS.json") || name == "EVA_INSTALLATIONS.json") {
            continue;
        }
        let d = read_json(&e.path());
        let base = d["base_url"].as_str().unwrap_or("").to_owned();
        // A machine entry holds its files directly or, in EVA_INSTALLATIONS.json, under `asset`
        // (which then carries its own base_url).
        let mut put = |id: &str, v: &Value| {
            let (files, b) = if v["files"].is_object() { (&v["files"], None) } else { (&v["asset"]["files"], v["asset"]["base_url"].as_str()) };
            if files.is_object() {
                reg.insert(id.to_owned(), (name.clone(), b.unwrap_or(&base).to_owned(), files.clone()));
            }
        };
        match &d["machines"] {
            Value::Object(m) => m.iter().for_each(|(id, v)| put(id, v)),
            Value::Array(m) => m.iter().for_each(|v| put(v["id"].as_str().unwrap_or(""), v)),
            _ => {
                if let Some(id) = d["machine_id"].as_str() {
                    put(id, &d);
                }
            }
        }
    }

    // Release zips: machines/<id>/model.glb.gz inside viewer-models.zip and friends.
    let mut in_zip: BTreeMap<String, (String, u64)> = BTreeMap::new();
    let zips = run("find", &[mirror.join("releases").to_str().unwrap(), "-name", "*.zip"]);
    for z in zips.lines() {
        let listing = run("unzip", &["-Z", z]);
        for l in listing.lines() {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if let Some(name) = cols.last() {
                if let Some(rest) = name.strip_prefix("machines/") {
                    if let Some((id, f)) = rest.split_once('/') {
                        if f == "model.glb.gz" {
                            let bytes = cols.get(3).and_then(|b| b.parse::<u64>().ok()).unwrap_or(0);
                            in_zip.insert(id.to_owned(), (z.strip_prefix(mirror.to_str().unwrap()).unwrap_or(z).trim_start_matches('/').to_owned(), bytes));
                        }
                    }
                }
            }
        }
    }

    // The community-models repo (bare): which of the registry paths exist, and each NOTICE.
    let cm = mirror.join("git/3d-print-rig-community-models.git");
    // Every git repo of the mirror, with the model files it holds: (repo dir name, paths under site/).
    // Polish WO-06 (2026-10-18): the 247Zero lives in its own repository now (its address in the community models' pages answers 404).
    let trees: Vec<(String, Vec<String>)> = ["3d-print-rig-community-models", "3d-print-rig-large-voron-models", "3d-print-rig-247zero-models"]
        .iter()
        .map(|r| {
            let g = mirror.join(format!("git/{r}.git"));
            ((*r).to_owned(), run("git", &["-C", g.to_str().unwrap(), "ls-tree", "-r", "--name-only", "HEAD"]).lines().map(str::to_owned).collect())
        })
        .collect();
    let notice_dirs = [
        ("ender3", "ender3"), ("mercury235", "mercury235"), ("mercury370", "mercury370"), ("proosa", "proosa"), ("sboom", "sboom"),
        ("snake180", "snake180"), ("snake3s", "snake3s"), ("snakeidex", "snakeidex"), ("stinger-v3", "stinger"), ("sv08-v2", "sv08"),
        ("vminion", "vminion"), ("vz330", "vz330"), ("aether", "aether"), ("requested-v1/rook", "requested-rook"),
        ("requested-v1/satsuma", "requested-satsuma"), ("requested-v1/the100", "requested-the100"), ("requested-v1/tictac", "requested-tictac"),
    ];
    let motion_dir = site.join("viewer/motion-profiles");

    let mut rows: Vec<Value> = Vec::new();
    let mut motion = Map::new();
    for m in machines {
        let id = m["id"].as_str().unwrap();
        let family = m["family"].as_str().unwrap_or("");
        let pin = &m["model_pins"][0];

        // Where its files are.
        let mut files = Map::new();
        let mut location = json!({"kind":"none"});
        if let Some((regfile, base, f)) = reg.get(id) {
            let model = &f["model.glb"];
            let path = model["path"].as_str().unwrap_or("");
            // The pages repo is `<host>/<repo>/<rel>`; the same file is `site/<rel>` in the mirror's git repo.
            let upstream = format!("{base}{path}");
            let rel = upstream.strip_prefix("https://psych0h3ad.github.io/").and_then(|r| r.split_once('/')).map_or(path, |(_, r)| r).to_owned();
            let in_mirror = trees.iter().find(|(_, t)| t.iter().any(|x| x == &format!("site/{rel}"))).map(|(r, _)| r.clone());
            // Polish WO-03 (2026-10-18): the registry names the community-models pages as the base of five machines whose files are in
            // the large-voron-models repository (the DoomCube 2, three Tridents, the V2.4 1000), where those pages answer 404. A file's
            // published address is the pages of the repository that holds it.
            let upstream = match &in_mirror {
                Some(r) => format!("{PAGES}/{r}/{rel}"),
                None => upstream,
            };
            for (n, v) in f.as_object().unwrap() {
                files.insert(n.clone(), json!({"path":v["path"],"bytes":v["bytes"],"sha256":v["sha256"],"decoded_sha256":v["decoded_sha256"]}));
            }
            location = json!({"kind":"registry","registry":regfile,"upstream":upstream,
                "mirror": match in_mirror { Some(r) => json!({"git":format!("git/{r}.git"),"path":format!("site/{rel}")}), None => Value::Null }});
            // Polish WO-06 (2026-10-18): a registry that names no `model.glb` holds the machine as a multi-file glTF (the Crossant 235:
            // model.gltf and seven geometry bins), which the installer, one file in and one file out, does not read. The address of its
            // folder answers 404, so the machine is listed with no file rather than with an address that cannot be fetched.
            // F03 (2026-10-19): and the "from the author" link it carried was that folder, which answers 404 as well: it is the author's repository page,
            // which answers and holds the files.
            if path.is_empty() {
                location = json!({"kind":"none","upstream":"https://github.com/Psych0h3ad/3d-print-rig-crossant-models","why":"published only as a multi-file glTF (model.gltf and its geometry bins), which Stella Scope does not read yet"});
            }
        } else if let Some((zip, bytes)) = in_zip.get(id) {
            for n in ["model.glb.gz", "assembly_manifest.json", "machine_profile.json"] {
                files.insert(n.to_owned(), json!({"path":format!("machines/{id}/{n}")}));
            }
            location = json!({"kind":"release_zip","mirror":{"zip":zip,"entry":format!("machines/{id}/model.glb.gz"),"bytes":bytes},
                "upstream":format!("{PAGES}/3d-print-rig/machines/{id}/model.glb.gz")});
        }

        // Licence.
        let notice = notice_dirs.iter().find_map(|(dir, lic)| {
            let path = location["upstream"].as_str().unwrap_or("");
            (path.contains(&format!("/{dir}/")) || location["mirror"]["path"].as_str().is_some_and(|p| p.contains(&format!("/{dir}/")))).then_some(*lic)
        });
        let basis: String;
        let mut source_url = String::new();
        let (licence, author): (String, String) = if let Some(l) = notice {
            let t = run("git", &["-C", cm.to_str().unwrap(), "show", &format!("HEAD:licenses/{l}/NOTICE.txt")]);
            basis = format!("community-models licenses/{l}/NOTICE.txt");
            let lic = notice_licence(&t).unwrap_or_else(|| "not stated".into());
            // Author: line when there is one; else the notice's own first lines carry the credit.
            let auth = t.lines().find_map(|x| x.strip_prefix("Author:")).map(|a| a.trim().to_owned()).unwrap_or_else(|| {
                t.lines().find(|l| l.starts_with("Copyright")).or_else(|| t.lines().next()).unwrap_or("").trim().to_owned()
            });
            let auth = auth.split(". ").next().unwrap_or(&auth).split(" \u{2014} ").next().unwrap_or(&auth).trim_end_matches('.').trim().to_owned();
            source_url = t.lines().find_map(|x| x.strip_prefix("Source:")).map(|u| u.trim().to_owned()).unwrap_or_default();
            (lic, auth)
        } else if id.starts_with("ratrig_vcore") {
            basis = "RATRIG_ASSETS.json source.license_basis: Rat Rig Wiki contentLicense=ccbyncsa".into();
            ("CC-BY-NC-SA-4.0".into(), "Rat Rig".into())
        } else if id.starts_with("annex_") {
            basis = "site/licenses/annex_license/LICENSE.md".into();
            ("ANNEX ENGINEERING HARDWARE AND SOFTWARE EULA (non-commercial terms)".into(), "Annex Engineering".into())
        } else if id == "crossant_235_v06_leadscrew" {
            basis = "CROSSANT_ASSETS.json license; THIRD_PARTY_NOTICES Crossant-235".into();
            ("CC-BY-NC-SA-4.0".into(), "Pole Engineering and contributors".into())
        } else if id == "positron_v322" {
            basis = "site/licenses/positron/NOTICE.txt".into();
            ("CC-BY-SA-4.0".into(), "Positron 3D Team, LDO Motors, Kralyn".into())
        } else if id == "remorph_beta1_307" {
            basis = "site/licenses/remorph/NOTICE.md (CERN-OHL-W-2.0, with a Phaetus component under personal-use terms)".into();
            ("CERN-OHL-W-2.0 and Phaetus personal-use terms (mixed)".into(), "Lex Reman".into())
        } else if id == "fysetc_v24_250_pro" {
            basis = "its own assembly_manifest.json: 'Repository published reference; explicit CAD license not found'".into();
            ("not stated by the author".into(), "FYSETC".into())
        } else if id.starts_with("siboor_") {
            basis = "docs/THIRD_PARTY_NOTICES.md (SIBOOR sections): redistribution terms not stated".into();
            ("not stated by the author".into(), "Lzhikai (SIBOOR)".into())
        } else if id == "doomcube2_350_reference" {
            basis = "docs/DOOMCUBE.md: GPL-3.0 at the pinned revision".into();
            ("GPL-3.0".into(), "DoomCube authors".into())
        } else if matches!(family, "v24" | "trident" | "v0" | "micron") {
            basis = "docs/THIRD_PARTY_NOTICES.md: VoronDesign (V2.4 R2, Trident R2, Voron-0), PrintersForAnts (Micron), LDO_AWD, GPL-3.0; embedded components keep their own terms".into();
            ("GPL-3.0".into(), if family == "micron" { "PrintersForAnts".into() } else { "VoronDesign".into() })
        } else if id == "247zero_beta2_120" {
            basis = "no licence statement found in the catalogue or notices".into();
            ("not stated in the catalogue".into(), "".into())
        } else {
            basis = "no licence statement found in the catalogue or notices".into();
            ("not stated in the catalogue".into(), "".into())
        };
        // Heavily-assembled Voron machines carry third-party parts with their own terms: say so, keep gpl.
        let class = if licence.starts_with("not stated") { "unlicensed" } else { class_of(&licence) };
        let motion_file = motion_dir.join(format!("{id}.json"));
        let has_motion = motion_file.exists();
        if has_motion {
            let p = read_json(&motion_file);
            let mut keep = Map::new();
            for k in ["model_sha256", "axes", "motions", "groups", "basis", "origin_mm", "rest_offsets_mm", "scope", "notices"] {
                if !p[k].is_null() {
                    keep.insert(k.to_owned(), p[k].clone());
                }
            }
            motion.insert(id.to_owned(), Value::Object(keep));
        }
        let (manifest, profile) = (companion(mirror, &location, "assembly_manifest.json"), companion(mirror, &location, "machine_profile.json"));
        let triangles = manifest.as_ref().and_then(|m| m["triangles"].as_u64());
        let parts = manifest.as_ref().and_then(|m| m["parts"].as_array().map(Vec::len));
        let frame_basis = profile.as_ref().map(|p| p["basis"].clone()).filter(|b| !b.is_null());
        let origin = profile.as_ref().map(|p| p["origin_mm"].clone()).filter(|b| !b.is_null());
        let (bytes, decoded) = (files.get("model.glb").and_then(|f| f["bytes"].as_u64()), files.get("model.glb").and_then(|f| f["decoded_sha256"].as_str().map(str::to_owned)));
        rows.push(json!({
            "id": id, "label": m["label"], "family": family, "size_mm": m["size"], "thumbnail": m["thumbnail"],
            "thumbnail_sha256": m["thumbnail_sha256"],
            "licence": {"class": class, "text": licence, "author": author, "basis": basis, "source": source_url},
            "pin": {"model_sha256": pin["model_sha256"], "stored_model_sha256": pin["stored_model_sha256"], "decoded_sha256_registry": decoded, "stored_bytes": bytes},
            "location": location, "files": files, "motion": has_motion,
            "triangles": triangles, "parts": parts, "basis": frame_basis, "origin_mm": origin,
        }));
    }

    // Options (polish WO-01): toolchangers, toolheads and add-ons, with their files, pins, licences and placements.
    let prev: Option<Value> = fs::read(out).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let (options, parts, bundle, e3ng, missing) = build_options(rig, mirror);
    assert!(missing.is_empty(), "options name files the bundle does not describe: {missing:?}");
    rows.extend(e3ng);
    for r in rows.iter_mut() {
        let id = r["id"].as_str().unwrap().to_owned();
        let fits: Vec<&str> = options.iter().filter(|o| o["fits"].get(&id).is_some()).map(|o| o["id"].as_str().unwrap()).collect();
        r["fits"] = json!(fits);
    }
    let doc = json!({"schema":"stella-scope.printer-models.v1","source":"https://github.com/Psych0h3ad/3d-print-rig","built":"2026-10-18","bundle":bundle,"machines":rows,"options":options,"parts":parts});
    fs::write(out, serde_json::to_vec(&doc).unwrap()).unwrap();
    if let Some(inv) = a.get(3) {
        fs::write(inv, inventory_md(&doc, prev.as_ref())).unwrap();
    }
    // Registered motion: one file, id -> profile. Registrations are the rig's MIT-licensed data.
    let mot = json!({"schema":"stella-scope.printer-motion.v1","source":"https://github.com/Psych0h3ad/3d-print-rig site/viewer/motion-profiles (MIT)","profiles":Value::Object(motion)});
    fs::write(out.with_file_name("printer-motion.json"), serde_json::to_vec(&mot).unwrap()).unwrap();

    // Thumbnails travel with a build only for licence classes that allow it, each checked against its pin.
    let mut thumbs = Map::new();
    for r in doc["machines"].as_array().unwrap() {
        let class = r["licence"]["class"].as_str().unwrap();
        if !matches!(class, "gpl" | "mit" | "cc-by" | "cc-by-sa") {
            continue;
        }
        let (id, name) = (r["id"].as_str().unwrap(), r["thumbnail"].as_str().unwrap());
        let bytes = fs::read(site.join("assets/machines").join(name)).unwrap_or_else(|e| panic!("thumbnail {name}: {e}"));
        assert_eq!(Some(sha256::hex(&bytes).as_str()), r["thumbnail_sha256"].as_str(), "thumbnail {name} does not match its pin");
        thumbs.insert(id.to_owned(), json!(base64(&bytes)));
    }
    let th = json!({"schema":"stella-scope.printer-thumbs.v1","note":"Only machines whose licence class allows redistribution.","thumbs":Value::Object(thumbs)});
    fs::write(out.with_file_name("printer-thumbs.json"), serde_json::to_vec(&th).unwrap()).unwrap();

    // The table for the STOP-REPORT.
    println!("{:<30} {:<12} {:<9} {:<15} {:<8} {:<7} {}", "id", "family", "size", "licence class", "motion", "mirror", "model file");
    for r in doc["machines"].as_array().unwrap() {
        let loc = &r["location"];
        let where_ = match loc["kind"].as_str() {
            Some("registry") => loc["mirror"]["path"].as_str().map(|p| format!("community repo {p}")).unwrap_or_else(|| format!("upstream only {}", loc["upstream"].as_str().unwrap_or(""))),
            Some("release_zip") => format!("zip {}", loc["mirror"]["entry"].as_str().unwrap_or("")),
            _ => "no model file found".into(),
        };
        println!("{:<30} {:<12} {:<9} {:<15} {:<8} {:<7} {}", r["id"].as_str().unwrap(), r["family"].as_str().unwrap(), r["size_mm"], r["licence"]["class"].as_str().unwrap(), if r["motion"] == true { "yes" } else { "no" },
            if loc["mirror"].is_null() { "no" } else { "yes" }, where_);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licence_words_in_english_and_japanese_become_the_right_class() {
        for (text, class) in [
            ("GPL-3.0", "gpl"),
            ("GPL-3.0; embedded component terms retained", "gpl"),
            ("CC-BY-SA-4.0", "cc-by-sa"),
            ("CC0-1.0", "cc0"),
            ("MIT; Molex connector exceptions retained", "mit"),
            ("MIT（Molex sub-assemblyには原文中の別条件が適用）", "mit"),
            ("CC-BY-NC-SA-4.0", "nc"),
            ("CC BY-NC-ND 4.0", "nc"),
            ("CC BY-NC (author Printables license)", "nc"),
            ("CC BY-NC-SA 4.0（部品別例外あり）", "nc"),
            ("ANNEX ENGINEERING HARDWARE AND SOFTWARE EULA（別条件）", "mixed"),
            ("Component-specific terms; see licenses/yavoth_assembly/NOTICE.md", "mixed"),
            ("Manufacturer published CAD; explicit repository license not found", "unlicensed"),
            ("配布条件の明記を取得資料で確認できず", "unlicensed"),
            ("メーカー配布資料：再配布条件の明記なし", "unlicensed"),
            ("取得したSTEPの再配布ライセンス表記なし。VORONやビューアーのGPLをこの基板へ適用しません。", "unlicensed"),
            ("No explicit CAD redistribution terms in the acquired source; no unrelated project license is applied.", "unlicensed"),
            ("a licence nobody has heard of", "unlicensed"),
            // "limited" and "permit" are not MIT.
            ("limited permit", "unlicensed"),
        ] {
            assert_eq!(class_of(text), class, "{text}");
        }
    }

    #[test]
    fn the_strictest_class_of_a_set_wins() {
        assert_eq!(strictest(["gpl", "nc", "mit"].into_iter()), "nc");
        assert_eq!(strictest(["gpl", "unlicensed"].into_iter()), "unlicensed");
        assert_eq!(strictest(["cc0", "mit", "gpl"].into_iter()), "gpl");
        assert_eq!(strictest(["gpl", "cc-by-sa"].into_iter()), "cc-by-sa");
        assert_eq!(strictest(std::iter::empty()), "unlicensed", "no statement is no licence");
    }

    #[test]
    fn a_notice_names_its_commit_and_licence() {
        let c = "c00b13ef851d38fef6e295e18296650e1ca50d7e";
        let text = format!("# Notices\n\n## Xol Toolhead\n\n作者：Armchair-Heavy-Industries\n配布元・参考元：https://github.com/Armchair-Heavy-Industries/Xol-Toolhead/tree/{c}\n版：{c}\nライセンス：CC BY-NC-SA 4.0（部品別例外あり）\n\n## No licence here\n\n版：{c}\n");
        let n = parse_notices(&text);
        assert_eq!(n.len(), 1, "a section without a licence line says nothing");
        assert_eq!((n[0].title.as_str(), n[0].commits.clone()), ("Xol Toolhead", vec![c.to_owned(), c.to_owned()]));
        assert!(n[0].licence.starts_with("CC BY-NC-SA 4.0") && n[0].author == "Armchair-Heavy-Industries");
        assert_eq!(n[0].url, "https://github.com/Armchair-Heavy-Industries/Xol-Toolhead");
        assert_eq!(commits_of(&json!({"source_commit":c,"source_records":[{"commit":c},{"commit":"short"}]})), vec![c.to_owned()]);
        assert_eq!(anglicise("CC BY-NC-SA 4.0（部品別例外あり）"), "CC BY-NC-SA 4.0 (some parts have their own terms)");
        assert_eq!(english("Sturdy Handles · M5原設計 / 左右", "x"), "Sturdy Handles · M5");
        assert_eq!(english("", "sturdy_handles_m5"), "Sturdy handles m5");
        assert_eq!(part_label("changer_sc_dock_xol"), "StealthChanger dock for xol");
        assert_eq!(part_label("changer_sc_bank_fasteners"), "StealthChanger bank fasteners");
        assert_eq!(part_label("component_revo_voron"), "Revo voron");
        assert_eq!(part_label("xol"), "Xol");
    }

    fn write(p: &Path, b: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }
    fn gz(raw: &[u8], at: &Path) {
        write(at, &[]);
        let raw_file = at.with_extension("raw");
        fs::write(&raw_file, raw).unwrap();
        let out = Command::new("gzip").args(["-n", "-c"]).arg(&raw_file).output().unwrap().stdout;
        fs::write(at, out).unwrap();
        fs::remove_file(raw_file).unwrap();
    }

    /// Round trip on a small rig of our own making: bundle, registrations and notices go in, the catalogue comes out,
    /// and what comes out is what the registrations say (placements, pins, licences), the same on every run.
    #[test]
    fn a_small_rig_goes_in_and_the_same_catalogue_comes_out_every_time() {
        if Command::new("zip").arg("-v").output().is_err() {
            eprintln!("skipped: no zip command");
            return;
        }
        let dir = std::env::temp_dir().join(format!("rig-fixture-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let (bundle, rig, mirror) = (dir.join("bundle"), dir.join("rig"), dir.join("mirror"));
        let commit = "c00b13ef851d38fef6e295e18296650e1ca50d7e";
        // Files of the bundle: three modules and one base head, gzipped.
        let files = [("modules/dock_a.glb.gz", b"dock a".as_slice()), ("modules/fasteners.glb.gz", b"fasteners"), ("modules/head_a_back.glb.gz", b"back"), ("modules/core.glb.gz", b"core"), ("heada.glb.gz", b"head a"),
            ("modules/indx_dock.glb.gz", b"indx dock")];
        for (name, raw) in files {
            gz(raw, &bundle.join(name));
        }
        let pins: Vec<Value> = files.iter().map(|(n, _)| { let b = fs::read(bundle.join(n)).unwrap(); json!({"path":n,"bytes":b.len(),"sha256":sha256::hex(&b)}) }).collect();
        let j = |name: &str, v: Value| write(&bundle.join(name), v.to_string().as_bytes());
        j("VIEWER_ASSETS.json", json!({"files":pins}));
        j("modules/dock_a.json", json!({"id":"dock_a","license":"GPL-3.0","parts":[1,2]}));
        j("modules/fasteners.json", json!({"id":"fasteners","license":"GPL-3.0","parts":[1]}));
        j("modules/head_a_back.json", json!({"id":"head_a_back","license":"GPL-3.0","parts":[1]}));
        j("modules/core.json", json!({"id":"core","license":"GPL-3.0","parts":[1]}));
        j("modules/indx_dock.json", json!({"id":"indx_dock","license":"GPL-3.0","parts":[1]}));
        j("HEADA.json", json!({"id":"heada","source_commit":commit,"parts":[1,2,3]}));
        j("TOOLHEAD_CONFIGURATIONS.json", json!({
            "toolheads":[{"id":"xol","label":"Xol"}],"extruders":[{"id":"e","label":"Sherpa"}],"hotends":[{"id":"h","label":"Rapido"}],
            "assets":{"dock_a":{"meta":"modules/dock_a.json","glb":"modules/dock_a.glb"},"head_a_back":{"meta":"modules/head_a_back.json","glb":"modules/head_a_back.glb"},"core":{"meta":"modules/core.json","glb":"modules/core.glb"}},
            "base_assets":{"xol":{"meta":"HEADA.json","glb":"heada.glb"}},
            "variants":[{"id":"head__sc__standard_6__xol__h__e","mount":"stealthchanger","gantry":"sc_standard_6","belt_width_mm":6,"toolhead":"xol","extruder":"e","hotend":"h",
                "head_translation_mm":[1.0,2.0,3.0],"hidden_xol_keys":["k1","k2"],
                "modules":[{"id":"core","translation_mm":[0,0,0],"role":"shuttle"},{"id":"head_a_back","translation_mm":[0,0.5,0],"role":"tool"},{"id":"dock_a","translation_mm":[-150,0,0],"role":"dock"}]}]}));
        j("MACHINE_HEAD_REGISTRATIONS.json", json!({"sources":{"stealthchanger_6":{"origin_mm":[10,20,30]},"indx":{"origin_mm":[0,0,0]}},"assets":{},
            "machines":{"m1":{"origin_mm":[11,22,33],"belt_width_mm":6,"stock_head_keys":["s1","s2"]}}}));
        j("TOOLCHANGER_BANK.json", json!({"pitch_mm":90,"nine_mm_forward_mm":5.6,"fixture_assets":["fasteners"],
            "assets":{"fasteners":{"meta":"modules/fasteners.json","glb":"modules/fasteners.glb"},"indx_dock":{"meta":"modules/indx_dock.json","glb":"modules/indx_dock.glb"}},
            "profiles":[{"selection":{"toolhead":"xol","extruder":"e","hotend":"h"},"dock":"dock_a","park_translation_mm":[0,100,50],"dock_hidden_keys":["d7"],"dock_part_offsets_mm":{},"native_park_body_intersections":[]}],
            "machines":{"m1":{"capacity":2,"center_x_mm":0.5,"translation_mm":[0,-200,300],"datum":"a datum"},"toolhead":{"capacity":1},"blocked":{"capacity":1,"bank_permitted":false}},
            "indx":{"pitch_mm":41,"tool_options":[],"dock_asset":"indx_dock","parked_asset":"indx_dock","machines":{}}}));
        j("MACHINE_MODS.json", json!({"assets":{},"machines":{}}));
        j("COMPONENT_LIBRARY.json", json!({"items":[],"assets":{}}));
        j("TOOLCHANGER_CONFIGURATIONS.json", json!({"assets":{},"variants":[],"sources":[]}));
        let zip = dir.join("viewer-models.zip");
        write(&zip, &[]);
        fs::remove_file(&zip).unwrap();
        assert!(Command::new("zip").current_dir(&bundle).args(["-q", "-r", zip.to_str().unwrap(), "."]).status().unwrap().success());
        let placed = mirror.join(BUNDLE_REL);
        fs::create_dir_all(placed.parent().unwrap()).unwrap();
        fs::copy(&zip, &placed).unwrap();
        let site = rig.join("site");
        write(&site.join("ASSET_BUNDLE.json"), json!({"bytes":fs::metadata(&zip).unwrap().len(),"sha256":"x","url":"u"}).to_string().as_bytes());
        for (n, v) in [("REQUESTED_MOD_INSTALLATIONS.json", json!({"assets":{},"machines":{}})), ("TRIDENT_INTERNAL_SPOOL.json", json!({"assets":{},"machines":{},"source":{}})),
            ("COMPONENT_ADDITIONS.json", json!({"items":[],"assets":{}})), ("HEAD_ADDITIONS.json", json!({}))] {
            write(&site.join(n), v.to_string().as_bytes());
        }
        write(&rig.join("docs/THIRD_PARTY_NOTICES.md"), format!("# Notices\n\n## Xol\n\n作者：Someone\n配布元・参考元：https://github.com/x/y/tree/{commit}\n版：{commit}\nライセンス：CC BY-NC-SA 4.0（部品別例外あり）\n").as_bytes());

        let (options, parts, info, e3ng, missing) = build_options(&rig, &mirror);
        assert!(missing.is_empty() && e3ng.is_none(), "{missing:?}");
        assert_eq!(info["bytes"], fs::metadata(&zip).unwrap().len());
        // Same input, same output, byte for byte.
        let again = build_options(&rig, &mirror);
        assert_eq!(serde_json::to_string(&options).unwrap(), serde_json::to_string(&again.0).unwrap());
        assert_eq!(serde_json::to_string(&parts).unwrap(), serde_json::to_string(&again.1).unwrap());

        // Every part is pinned by what is in the file, both ways.
        for (id, p) in &parts {
            let stored = fs::read(bundle.join(st(&p["location"]["mirror"]["entry"]))).unwrap();
            assert_eq!(p["pin"]["stored_model_sha256"], sha256::hex(&stored).as_str(), "{id}");
            let raw = Command::new("gzip").arg("-dc").arg(bundle.join(st(&p["location"]["mirror"]["entry"]))).output().unwrap().stdout;
            assert_eq!(p["pin"]["model_sha256"], sha256::hex(&raw).as_str(), "{id}");
            assert!(st(&p["location"]["upstream"]).starts_with("https://psych0h3ad.github.io/3d-print-rig/"));
        }
        // The bank: where the registrations put it. m1 only: the pseudo-machine and the blocked one are not offered.
        let bank = options.iter().find(|o| o["id"] == "stealthchanger_bank").expect("the bank");
        assert_eq!(bank["fits"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["m1"]);
        let fit = &bank["fits"]["m1"];
        assert_eq!((fit["capacity"].clone(), fit["pitch_mm"].clone(), fit["anchor_mm"].clone()), (json!(2), json!(90), json!([0.5, -200, 300])));
        assert_eq!(fit["head_shift_mm"], json!([1.0, 2.0, 3.0]), "the machine's origin minus the changer's registered origin");
        assert_eq!(fit["stock_head_keys"], json!(["s1", "s2"]));
        let tool = &bank["attach"]["tools"]["xol_e_h"];
        assert_eq!(tool["dock"], json!({"part":"dock_a","hidden_keys":["d7"]}));
        assert_eq!(tool["active"][0], json!({"part":"xol","translation_mm":[1.0,2.0,3.0],"hidden_keys":["k1","k2"],"role":"head"}));
        assert_eq!(tool["active"].as_array().unwrap().len(), 3, "base, shuttle and backplate; the dock is not on the gantry");
        // Parked: the head and its backplate only, moved by the park offset; no shuttle.
        assert_eq!(tool["parked"].as_array().unwrap().len(), 2);
        assert_eq!(tool["parked"][0]["translation_mm"], json!([1.0, 102.0, 53.0]));
        assert_eq!(tool["parked"][1]["translation_mm"], json!([0.0, 100.5, 50.0]));
        // Licences: the Xol head is named by the notice of its commit as CC BY-NC-SA, so the bank is non-commercial
        // though its docks are GPL, and nothing in it is open as a whole.
        assert_eq!(parts["dock_a"]["licence"]["class"], "gpl");
        assert_eq!(parts["xol"]["licence"]["class"], "nc");
        assert!(parts["xol"]["licence"]["text"].as_str().unwrap().contains("some parts have their own terms"));
        assert_eq!(bank["licence"]["class"], "nc");
        let fam = options.iter().find(|o| o["id"] == "head_xol").expect("the Xol family");
        assert_eq!((fam["attach"]["kind"].clone(), fam["fits"]["m1"]["registered_variants"].clone()), (json!("registered"), json!(1)));
        // Nothing is made up: no machine is invented from the options, and every option lists parts that exist.
        for o in &options {
            for p in o["parts"].as_array().unwrap() {
                assert!(parts.contains_key(st(p)), "{} names {p}", o["id"]);
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
