// Motion for machines whose models carry it in their own parts (polish board WO-02, 2026-10-18).
//
//   cargo run --release --example printer-motion-parts -- MODELS OUT.json
//     MODELS  a folder laid out like the project's pages, holding for each machine below
//             `<dir>/model.glb.gz` and `<dir>/machine_profile.json` (the release zip's `machines/<id>/`
//             and the community repository's `site/ender3/`)
//     OUT     web/printer-motion-parts.json
//
// 3D Print Rig registers motion for ten machines in its `motion-profiles` folder (web/printer-motion.json).
// The Voron V2.4, Trident, V0.2, Micron and the Ender-3 are not there: their models say what moves in each
// part's own record (`extras.motion` and `extras.group` in the GLB) and their machine profile gives the
// travel and where the nozzle is in the model's pose. This reads both and writes the same kind of profile
// (`groups` of part keys, `motions`, `axes`, a `calibration`), pinned to the model's SHA-256, so the farm
// draws these machines moving. Nothing is guessed: a part with no record stays with the frame, a model whose
// hash is not the catalogue's is refused, and a machine whose model has no gantry (the Trident's base) moves
// only what the model has. The Ender-3's nozzle and plate are measured from its geometry.
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

#[path = "../src/sha256.rs"]
mod sha256;

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The unpacked GLB bytes (the system gzip does the unpacking).
fn gunzip(p: &Path) -> Vec<u8> {
    let o = Command::new("gzip")
        .arg("-dc")
        .arg(p)
        .output()
        .unwrap_or_else(|e| panic!("gzip: {e}"));
    assert!(o.status.success(), "{}: gzip failed", p.display());
    o.stdout
}

struct Part {
    key: String,
    motion: String,
    group: String,
    min: [f64; 3],
    max: [f64; 3],
}

/// Every part of a GLB: its key, its own motion and group, and its bounding box in millimetres (the files
/// are in metres with the world coordinates baked into the vertices).
fn parts(glb: &[u8]) -> Vec<Part> {
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON", "not a GLB");
    let json: Value = serde_json::from_slice(&glb[20..20 + len]).expect("GLB json");
    let mut out = Vec::new();
    for n in json["nodes"].as_array().expect("nodes") {
        let Some(mi) = n["mesh"].as_u64() else {
            continue;
        };
        let mesh = &json["meshes"][mi as usize];
        assert!(
            n["matrix"].is_null() && n["rotation"].is_null() && n["scale"].is_null(),
            "a node has more than a translation; the bounds would be wrong"
        );
        let e = &mesh["extras"];
        let key = e["part_key"]
            .as_str()
            .or_else(|| n["extras"]["part_key"].as_str())
            .unwrap_or("")
            .to_owned();
        if key.is_empty() {
            continue;
        }
        let acc = &json["accessors"][mesh["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .expect("POSITION") as usize];
        // A node may move its part (a translation, in metres) from where the vertices put it.
        let v = |a: &Value| -> [f64; 3] {
            [0, 1, 2].map(|i| {
                (a[i].as_f64().expect("accessor bounds")
                    + n["translation"][i].as_f64().unwrap_or(0.0))
                    * 1000.0
            })
        };
        out.push(Part {
            key,
            motion: e["motion"].as_str().unwrap_or("").to_owned(),
            group: e["group"].as_str().unwrap_or("").to_owned(),
            min: v(&acc["min"]),
            max: v(&acc["max"]),
        });
    }
    out
}

fn finish(id: &str, cat: &Value, notices: &[&str], scope: &str) -> Map<String, Value> {
    let row = cat["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in the catalogue"));
    let mut p = Map::new();
    p.insert("model_sha256".into(), row["pin"]["model_sha256"].clone());
    p.insert("notices".into(), json!(notices));
    p.insert("scope".into(), json!(scope));
    p
}

const SCOPE: &str = "Movement preview from the live position; read from the model's own part records. Firmware, belts and collisions are not simulated.";

/// A machine whose parts say what moves: `rules` maps a part's `extras.motion` to (group, motion spec).
#[allow(clippy::too_many_arguments)]
fn from_records(
    id: &str,
    dir: &Path,
    cat: &Value,
    limits_key: &str,
    rules: &[(&str, &str, Value)],
    notices: &[&str],
) -> (String, Value) {
    let glb = gunzip(&dir.join("model.glb.gz"));
    let want = cat["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap()["pin"]["model_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        sha256::hex(&glb),
        want,
        "{id}: the model is not the one the catalogue pins"
    );
    let prof = read_json(&dir.join("machine_profile.json"));
    let limits = &prof[limits_key];
    let rf = &prof["display_reference_xyz_mm"];
    let ax = |name: &str, i: usize| -> [f64; 2] {
        let r = rf[i].as_f64().expect("reference");
        [
            limits[name][0].as_f64().expect("limit") - r,
            limits[name][1].as_f64().expect("limit") - r,
        ]
    };
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut motions = Map::new();
    motions.insert("fixed".into(), json!([0, 0, 0]));
    for (motion, group, m) in rules {
        motions.insert((*group).into(), m.clone());
        groups.entry((*group).into()).or_default();
        let _ = motion;
    }
    for part in parts(&glb) {
        if let Some((_, group, _)) = rules.iter().find(|(m, _, _)| *m == part.motion) {
            groups.get_mut(*group).unwrap().insert(part.key); // a part in two nodes (a motor and its shaft) is one key
        }
    }
    for (g, keys) in &groups {
        assert!(!keys.is_empty(), "{id}: no part moves as {g}");
    }
    let mut p = finish(id, cat, notices, SCOPE);
    p.insert(
        "axes".into(),
        json!({"x":ax("X", 0),"y":ax("Y", 1),"z":ax("Z", 2)}),
    );
    p.insert("groups".into(), json!(groups));
    p.insert("motions".into(), Value::Object(motions));
    p.insert("calibration".into(), json!({"offset_mm":[-rf[0].as_f64().unwrap(), -rf[1].as_f64().unwrap(), -rf[2].as_f64().unwrap()],"sign":[1,1,1],"source":[0,1,2]}));
    p.insert("calibration_basis".into(), json!(format!("The maker's machine profile: the nozzle is at X {:.1}, Y {:.1}, Z {:.1} mm over the bed in the model's pose, so reported 0 is that far back along each axis.", rf[0].as_f64().unwrap(), rf[1].as_f64().unwrap(), rf[2].as_f64().unwrap())));
    (id.to_owned(), Value::Object(p))
}

/// The Ender-3: the maker's profile names the groups and the motion; the plate and the nozzle are measured.
fn ender(dir: &Path, cat: &Value) -> (String, Value) {
    let id = "ender3_stock_220";
    let glb = gunzip(&dir.join("model.glb.gz"));
    let want = cat["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap()["pin"]["model_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        sha256::hex(&glb),
        want,
        "{id}: the model is not the one the catalogue pins"
    );
    let prof = read_json(&dir.join("machine_profile.json"));
    let all = parts(&glb);
    let in_group = |g: &'static str| all.iter().filter(move |p| p.group == *g);
    // Display frame: Y up, millimetres. The plate is the bed part whose top is highest; the nozzle tip is the
    // lowest point of the head, and the tip's part is centred on the nozzle's axis.
    let plate = in_group(&"bed")
        .max_by(|a, b| {
            a.max[1]
                .total_cmp(&b.max[1])
                .then((a.max[0] - a.min[0]).total_cmp(&(b.max[0] - b.min[0])))
        })
        .expect("a bed part");
    let tip = in_group(&"head")
        .min_by(|a, b| a.min[1].total_cmp(&b.min[1]))
        .expect("a head part");
    let (px, pz, top) = (
        (plate.min[0] + plate.max[0]) / 2.0,
        (plate.min[2] + plate.max[2]) / 2.0,
        plate.max[1],
    );
    let (nx, nz, ny) = (
        (tip.min[0] + tip.max[0]) / 2.0,
        (tip.min[2] + tip.max[2]) / 2.0,
        tip.min[1],
    );
    // The uprights stand toward the back of the base, so the front is the side the beam is not on.
    let span = |g: &'static str| -> (f64, f64) {
        let v: Vec<&Part> = in_group(&g).collect();
        (
            v.iter().map(|p| p.min[2]).fold(f64::MAX, f64::min),
            v.iter().map(|p| p.max[2]).fold(f64::MIN, f64::max),
        )
    };
    let (b, f) = (span("beam"), span("fixed"));
    assert!(
        (b.0 + b.1) / 2.0 < (f.0 + f.1) / 2.0,
        "the front of the Ender-3 is not +Z in this model; check the sign of Y"
    );
    // The plate is 235 mm and the print area 220 mm, centred on it. Y 0 is the front edge, +Z is the front.
    let area = 220.0;
    let off = [px - area / 2.0 - nx, -(area / 2.0) - (pz - nz), top - ny];
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for part in &all {
        if prof["motions"].get(&part.group).is_some() {
            groups
                .entry(part.group.clone())
                .or_default()
                .insert(part.key.clone());
        }
    }
    let travel_z = prof["axes"]["z"][1].as_f64().unwrap();
    let mut p = finish(
        id,
        cat,
        &[
            "Axis values are displacements from the model's pose. Groups and motion are the maker's, from the machine profile; the travel is widened to the 220 mm print area and the nozzle reaching the plate, measured from the model.",
            "The bed moves toward the front as the nozzle goes toward the back: Y is the bed's travel along the model's Z.",
        ],
        SCOPE,
    );
    p.insert(
        "axes".into(),
        json!({"x":[off[0], off[0] + area],"y":[off[1], off[1] + area],"z":[off[2], travel_z]}),
    );
    p.insert("basis".into(), prof["basis"].clone());
    p.insert("groups".into(), json!(groups));
    p.insert("motions".into(), prof["motions"].clone());
    p.insert(
        "calibration".into(),
        json!({"offset_mm":off,"sign":[1,1,1],"source":[0,1,2]}),
    );
    p.insert("calibration_basis".into(), json!(format!(
        "Measured from the model (mm, Y up): plate {:.1} wide centred at X {:.1}, Z {:.1}, top {:.1}; nozzle tip at X {:.1}, Z {:.1}, height {:.1}, so {:.1} mm over the plate. The 220 mm print area is centred on the plate; Y 0 is its front edge (the +Z side).",
        plate.max[0] - plate.min[0], px, pz, top, nx, nz, ny, ny - top)));
    (id.to_owned(), Value::Object(p))
}

/// The Trident R2 gantry and X carriage modules (F03, 2026-10-19): the base model has no gantry or toolhead, and 3D Print Rig keeps
/// them as two modules whose parts say what moves (`extras.motion`): `y` is the X beam and what rides on it, on its Y rails, `xy` is
/// the carriage that moves with X as well, `fixed` stays. Their part keys are added to the Trident's groups `gantry` and `head`, so a
/// printer that chooses the module option moves them (the keys name nothing in a Trident without it). Each file is refused unless it
/// is the one the catalogue pins.
fn with_trident_modules((id, mut v): (String, Value), models: &Path, extra: &Value) -> (String, Value) {
    let (mut gantry, mut head): (BTreeSet<String>, BTreeSet<String>) = (BTreeSet::new(), BTreeSet::new());
    for module in ["trident_r2_gantry_300", "trident_r2_sb_carriage"] {
        let glb = gunzip(&models.join(format!("modules/{module}.glb.gz")));
        let want = extra["parts"][module]["pin"]["model_sha256"].as_str().unwrap_or_else(|| panic!("{module} is not in the extra catalogue"));
        assert_eq!(sha256::hex(&glb), want, "{module}: the module is not the one the catalogue pins");
        for part in parts(&glb) {
            match part.motion.as_str() {
                "y" => gantry.insert(part.key),
                "xy" => head.insert(part.key),
                "fixed" => continue,
                other => panic!("{module}: a part moves as {other:?}, which is not understood"),
            };
        }
    }
    assert!(!gantry.is_empty() && !head.is_empty(), "the Trident modules moved nothing");
    v["groups"]["gantry"] = json!(gantry);
    v["groups"]["head"] = json!(head);
    v["motions"]["gantry"] = json!([0, "y", 0]);
    v["motions"]["head"] = json!(["x", "y", 0]);
    v["notices"] = json!([
        "Axis values are displacements from the model's pose: the bed drops as Z rises.",
        "The base model has no gantry or toolhead (3D Print Rig keeps them as separate modules). With the option 'Trident R2 gantry and X carriage' chosen, the X beam moves with Y and the X carriage with X and Y, as the parts of those files say. The toolhead that hangs from the carriage is not drawn."
    ]);
    (id, v)
}

/// The Annex K1, K2 and K3 (F03, 2026-10-19). 3D Print Rig registered their motion (`web/printer-motion.json`) but their files are not in the display's frame: each part is
/// placed by a matrix of its own, and the maker's profile says the assembly is turned about X for upright viewing (`viewer_orientation`, 0, a quarter or a half turn). That
/// turn is the profile's `orient` (display = orient * file), which the page applies to every part. The motion is written in one of two frames: the K2 and K3 list Z on the
/// second component (the display's up, so no basis is needed), the K1 on the third (a Z-up CAD frame, the project's default basis). Their panels and doors are solid in the
/// file, so the profile names them `see_through` with the rig's own words. The groups, motions and travel are the rig's, unchanged; the file is refused unless it is the one
/// the catalogue pins.
fn annex(id: &str, dir: &Path, cat: &Value, rig: &Value) -> (String, Value) {
    let glb = gunzip(&dir.join("model.glb.gz"));
    let row = cat["machines"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("{id} is not in the catalogue"));
    assert_eq!(sha256::hex(&glb), row["pin"]["model_sha256"].as_str().unwrap(), "{id}: the model is not the one the catalogue pins");
    let prof = read_json(&dir.join("machine_profile.json"));
    let turn = prof["viewer_orientation"]["viewer_root_rotation_x_radians"].as_f64().unwrap_or(0.0);
    let q = (turn / std::f64::consts::FRAC_PI_2).round();
    assert!((turn - q * std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{id}: a turn of {turn} rad is not a quarter turn");
    let (c, sn): (i64, i64) = match q.rem_euclid(4.0) as i32 {
        0 => (1, 0),
        1 => (0, 1),
        2 => (-1, 0),
        _ => (0, -1),
    };
    let mut p = rig["profiles"][id].clone();
    assert_eq!(p["model_sha256"], row["pin"]["model_sha256"], "{id}: the rig's registration is for another file");
    // The frame the motion is written in: where Z is on the head's or the bed's motion.
    let z_at = ["head", "bed"].iter().find_map(|g| (0..3).find(|&k| match &p["motions"][g][k] { Value::String(x) => x == "z", Value::Object(o) => o.contains_key("z"), _ => false }));
    p["basis"] = match z_at {
        Some(2) => json!([[1, 0, 0], [0, 0, 1], [0, -1, 0]]),
        Some(1) => json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]]),
        other => panic!("{id}: Z is on component {other:?} of the motion, which is not understood"),
    };
    p["orient"] = json!([[1, 0, 0], [0, c, -sn], [0, sn, c]]);
    p["see_through"] = json!({"match":"panel|door","except":"clip|hinge|latch|handle|mount|bracket|spacer"});
    p["notices"] = json!([
        "Axis values are displacements from the model's pose, in the frame the rig registered the motion in; the file is turned upright by the rig's own rotation.",
        "Side panels and doors are drawn nearly clear. Firmware, belts and chains keep their shape in the model; nothing here is certified."
    ]);
    (id.to_owned(), p)
}

/// The SnakeOil XY IDEX (F03, 2026-10-19): two carriages on one beam, each a group of the file's own part records (`head`, `head2`), the beam (`beam`, which
/// moves with Y) and the bed (`bed`). The maker's profile drives the second carriage by its own axis, U; while that tool prints the printer's X is where it
/// is, so here it follows X like the first (the demo parks whichever tool is not printing at its end of the beam: `tools` and the second carriage's rest offset
/// are measured by tools/printer-print-frames.rs from the two nozzles). Travel, motions and the basis are the maker's. The file is refused unless it is the
/// one the catalogue pins.
fn idex(id: &str, dir: &Path, cat: &Value) -> (String, Value) {
    let glb = gunzip(&dir.join("model.glb.gz"));
    let want = cat["machines"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("{id} is not in the catalogue"))["pin"]["model_sha256"].as_str().unwrap().to_owned();
    assert_eq!(sha256::hex(&glb), want, "{id}: the model is not the one the catalogue pins");
    let prof = read_json(&dir.join("machine_profile.json"));
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for part in parts(&glb) {
        if ["bed", "beam", "head", "head2"].contains(&part.group.as_str()) {
            groups.entry(part.group.clone()).or_default().insert(part.key);
        }
    }
    for g in ["bed", "beam", "head", "head2"] {
        assert!(groups.get(g).is_some_and(|k| !k.is_empty()), "{id}: no part moves as {g}");
    }
    let mut head2 = prof["motions"]["head2"].clone();
    for c in head2.as_array_mut().unwrap() {
        if c == "u" {
            *c = json!("x");
        }
    }
    let mut p = Map::new();
    p.insert("model_sha256".into(), json!(want));
    p.insert("axes".into(), json!({"x":prof["axes"]["x"],"y":prof["axes"]["y"],"z":prof["axes"]["z"]}));
    p.insert("basis".into(), prof["basis"].clone());
    p.insert("groups".into(), json!(groups));
    p.insert("motions".into(), json!({"fixed":[0,0,0],"head":prof["motions"]["head"],"head2":head2,"beam":prof["motions"]["beam"],"bed":prof["motions"]["bed"]}));
    p.insert("notices".into(), json!([
        "Axis values are displacements from the model's pose, the maker's. Two carriages share the beam: the one whose tool is printing follows the head's position, the other waits at its end of the beam.",
        "Belts and cables keep their shape in the model; nothing here is certified."
    ]));
    p.insert("scope".into(), json!(SCOPE));
    (id.to_owned(), Value::Object(p))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (models, out) = (Path::new(&a[0]), Path::new(&a[1]));
    let cat = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("web/printer-models.json"));
    let z = |s: &str| json!(s);
    let neg = |c: &str| json!({ c: -1 });
    let mut profiles = Map::new();
    let mut add = |(id, v): (String, Value)| {
        println!(
            "{id}: {} groups, axes {}",
            v["groups"].as_object().map_or(0, |g| g.len()),
            v["axes"]
        );
        profiles.insert(id, v);
    };
    // The two printed R2 sizes share one structure (the 300 is the base of the toolchanger machine).
    for id in ["voron_v24_350_printed", "voron_v24_300_printed"] {
        add(from_records(
            id,
            &models.join(format!("machines/{id}")),
            &cat,
            "display_limits_mm",
            &[("z_gantry", "z_gantry", json!([0, 0, z("z")])), ("yz", "x_beam", json!([0, z("y"), z("z")])), ("xyz", "head", json!([z("x"), z("y"), z("z")]))],
            &["Axis values are displacements from the model's pose: the gantry climbs with Z, the X beam also moves with Y, the toolhead with X, Y and Z.", "Belts and cables keep their shape in the model."],
        ));
    }
    let extra = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("web/printer-models-extra.json"));
    add(with_trident_modules(
        from_records(
            "voron_trident_300",
            &models.join("machines/voron_trident_300"),
            &cat,
            "display_limits_mm",
            &[("z", "bed", json!([0, 0, neg("z")]))],
            &[],
        ),
        models,
        &extra,
    ));
    add(from_records(
        "voron_v02_120",
        &models.join("machines/voron_v02_120"),
        &cat,
        "display_limits_mm",
        &[("y", "x_beam", json!([0, z("y"), 0])), ("xy", "head", json!([z("x"), z("y"), 0])), ("z_bed", "bed", json!([0, 0, neg("z")]))],
        &["Axis values are displacements from the model's pose: the bed drops as Z rises, the X beam moves with Y, the toolhead with X and Y.", "Belts and cables keep their shape in the model."],
    ));
    add(from_records(
        "micron_plus_r1_180",
        &models.join("machines/micron_plus_r1_180"),
        &cat,
        "nominal_review_limits_mm",
        &[("z_gantry", "z_gantry", json!([0, 0, z("z")])), ("yz_beam", "x_beam", json!([0, z("y"), z("z")])), ("xyz_head", "head", json!([z("x"), z("y"), z("z")]))],
        &["Axis values are displacements from the model's pose: the gantry climbs with Z, the X beam also moves with Y, the toolhead with X, Y and Z.", "The maker lists the machine's calibration as pending; travel is the nominal review limits."],
    ));
    add(ender(&models.join("site/ender3"), &cat));
    add(idex("snakeoil_xy_idex", &models.join("site/snakeidex"), &cat));
    let rig = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("web/printer-motion.json"));
    for id in ["annex_k1_assembly", "annex_k2_assembly", "annex_k3_assembly"] {
        add(annex(id, &models.join(format!("machines/{id}")), &cat, &rig));
    }
    let doc = json!({"schema":"stella-scope.printer-motion-parts.v1","source":"Read from the part records inside each model file and the maker's machine profile by tools/printer-motion-parts.rs","profiles":profiles});
    fs::write(out, serde_json::to_vec(&doc).unwrap()).unwrap();
}
