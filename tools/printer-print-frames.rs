// Where a print sits on a machine's plate (polish board WO-03, 2026-10-18).
//
//   cargo run --release --example printer-print-frames -- MODELS OUT.json [PRUSA_OUT.json]
//   cargo run --release --example printer-print-frames -- MODELS PRUSA_OUT.json --prusa-only
//     MODELS  a folder laid out like the project's pages: `machines/<id>/model.glb.gz` (and `machine_profile.json`
//             where the maker wrote one), `site/ender3/`, `site/sv08-v2/model.glb.gz`,
//             `site/doomcube-v103/doomcube2_350_reference/model.glb.gz`, and `prusa-core-one-l/model.glb.gz`
//     OUT     web/printer-print-frames.json
//     PRUSA_OUT  web/printer-print-frames-prusa.json (F09, 2026-10-19): the Prusa machines added after the rig's, kept in a file of their own
//             like printer-models-extra.json so that measuring one never rewrites the others. `prusa-core-one/model.glb.gz`,
//             `prusa-mk3s/model.glb.gz` and `prusa-mini/model.glb.gz` are read from MODELS too (the repository's own `models/` copies, one folder each).
//             With --prusa-only the rig's machines are not measured at all, so its files need not be to hand.
//
// A print file speaks in the printer's own coordinates: X0 Y0 is a corner of the plate, Z0 the plate's top. To draw
// the part growing on a model, the page needs to know where that corner is in the model's frame and which way the
// printer's axes run. This reads that off each model: the maker's own measurements where the maker wrote them
// (`nozzle_tip_mm` minus `display_reference_xyz_mm` is the printer's origin in the model's CAD frame, and it agrees
// with `bed_surface_min_xy_mm` and `bed_top_world_z_mm`), and the plate itself where the maker did not (the slab at
// the top of the bed, as wide as the machine's build size). Nothing is guessed: a model whose hash is not the
// catalogue's pin is refused, and the way each number was found is written beside it.
//
// The front of a machine is +Z in the model file (the project's own convention, and the rig's "front" view), so
// printer Y runs from the front toward -Z. A model that says otherwise in its profile is not in this table.
use serde_json::{json, Map, Value};
use std::{fs, path::Path, process::Command};

#[path = "../src/sha256.rs"]
mod sha256;

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

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
    group: String,
    min: [f64; 3],
    max: [f64; 3],
}

/// Every part of a GLB with its bounding box in the display frame (Y up), millimetres.
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
        let acc = &json["accessors"][mesh["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .expect("POSITION") as usize];
        let v = |a: &Value| -> [f64; 3] {
            [0, 1, 2].map(|i| {
                (a[i].as_f64().expect("accessor bounds")
                    + n["translation"][i].as_f64().unwrap_or(0.0))
                    * 1000.0
            })
        };
        out.push(Part {
            key,
            group: e["group"].as_str().unwrap_or("").to_owned(),
            min: v(&acc["min"]),
            max: v(&acc["max"]),
        });
    }
    out
}

/// F03 (2026-10-19): the same for a file whose nodes place their parts with a 4 x 4 matrix (the Annex K1, K2 and K3: shared meshes, one matrix a node, parts named
/// `p00634__name` with no part key) and whose frame is not the display's: `orient` (three rows, display = orient * file) turns it upright. The box of each part is
/// its mesh's box through the node's matrix and then the orientation, in millimetres.
fn parts_oriented(glb: &[u8], orient: Option<[[f64; 3]; 3]>) -> Vec<Part> {
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json: Value = serde_json::from_slice(&glb[20..20 + len]).expect("GLB json");
    let o = orient.unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let mut out = Vec::new();
    for n in json["nodes"].as_array().expect("nodes") {
        let Some(mi) = n["mesh"].as_u64() else { continue };
        let mesh = &json["meshes"][mi as usize];
        assert!(n["rotation"].is_null() && n["scale"].is_null(), "a node has a rotation or scale of its own, which this does not read");
        let m: [f64; 16] = match n["matrix"].as_array() {
            Some(a) => std::array::from_fn(|i| a[i].as_f64().unwrap()),
            None => {
                let t = |i: usize| n["translation"][i].as_f64().unwrap_or(0.0);
                [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, t(0), t(1), t(2), 1.0]
            }
        };
        let name = n["name"].as_str().unwrap_or("");
        let key = mesh["extras"]["part_key"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| name.split_once("__").filter(|(k, _)| k.starts_with('p') && k[1..].bytes().all(|b| b.is_ascii_digit())).map(|(k, _)| k.to_owned()))
            .unwrap_or_default();
        let acc = &json["accessors"][mesh["primitives"][0]["attributes"]["POSITION"].as_u64().expect("POSITION") as usize];
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for &x in &[0, 1] {
            for &y in &[0, 1] {
                for &z in &[0, 1] {
                    let p = [
                        acc[if x == 0 { "min" } else { "max" }][0].as_f64().unwrap(),
                        acc[if y == 0 { "min" } else { "max" }][1].as_f64().unwrap(),
                        acc[if z == 0 { "min" } else { "max" }][2].as_f64().unwrap(),
                    ];
                    let f: [f64; 3] = std::array::from_fn(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]);
                    for r in 0..3 {
                        let d = (o[r][0] * f[0] + o[r][1] * f[1] + o[r][2] * f[2]) * 1000.0;
                        lo[r] = lo[r].min(d);
                        hi[r] = hi[r].max(d);
                    }
                }
            }
        }
        out.push(Part { key, group: mesh["extras"]["group"].as_str().unwrap_or("").to_owned(), min: lo, max: hi });
    }
    out
}

/// The tip of a tool: the centre of the lowest faces of the parts that are drawn (the file's display frame, millimetres). A tool
/// has its own nozzle, a few millimetres from the stock one; a toolchanger's firmware makes up the difference, so the model
/// has to as well. Vertices within a quarter millimetre of the lowest point are the tip.
fn lowest_tip(glb: &[u8], hidden: &std::collections::HashSet<String>) -> [f64; 3] {
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json: Value = serde_json::from_slice(&glb[20..20 + len]).expect("GLB json");
    let bin = 20 + len + 8; // the BIN chunk's data
    let mut verts: Vec<[f64; 3]> = Vec::new();
    for n in json["nodes"].as_array().expect("nodes") {
        let Some(mi) = n["mesh"].as_u64() else { continue };
        let mesh = &json["meshes"][mi as usize];
        let key = mesh["extras"]["part_key"].as_str().or_else(|| n["extras"]["part_key"].as_str()).unwrap_or("");
        if hidden.contains(key) {
            continue;
        }
        for prim in mesh["primitives"].as_array().unwrap() {
            let acc = &json["accessors"][prim["attributes"]["POSITION"].as_u64().unwrap() as usize];
            let bv = &json["bufferViews"][acc["bufferView"].as_u64().unwrap() as usize];
            let at = bin + bv["byteOffset"].as_u64().unwrap_or(0) as usize + acc["byteOffset"].as_u64().unwrap_or(0) as usize;
            let t = |i: usize| n["translation"][i].as_f64().unwrap_or(0.0);
            for v in 0..acc["count"].as_u64().unwrap() as usize {
                let f = |c: usize| f32::from_le_bytes(glb[at + v * 12 + c * 4..at + v * 12 + c * 4 + 4].try_into().unwrap()) as f64;
                verts.push([(f(0) + t(0)) * 1000.0, (f(1) + t(1)) * 1000.0, (f(2) + t(2)) * 1000.0]);
            }
        }
    }
    let lo = verts.iter().map(|v| v[1]).fold(f64::MAX, f64::min);
    let tip: Vec<&[f64; 3]> = verts.iter().filter(|v| v[1] <= lo + 0.25).collect();
    let n = tip.len() as f64;
    [tip.iter().map(|v| v[0]).sum::<f64>() / n, lo, tip.iter().map(|v| v[2]).sum::<f64>() / n]
}

fn catalogue_row<'a>(cat: &'a Value, id: &str) -> &'a Value {
    cat["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in the catalogue"))
}

/// The machine's basis as the page reads it: the catalogue row, else the registered motion, else CAD Z up.
fn basis_of(cat: &Value, motion: &Value, id: &str) -> [[f64; 3]; 3] {
    let b = catalogue_row(cat, id)["basis"]
        .as_array()
        .cloned()
        .or_else(|| motion["profiles"][id]["basis"].as_array().cloned())
        .unwrap_or_else(|| {
            vec![
                json!([1, 0, 0]),
                json!([0, 0, 1]),
                json!([0, -1, 0]),
            ]
        });
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = b[r][c].as_f64().unwrap();
        }
    }
    out
}

/// The model's frame from the display frame: the basis maps the model's frame into the display, so its
/// transpose (it is a signed permutation) takes a display vector back.
fn to_frame(b: &[[f64; 3]; 3], d: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|c| (0..3).map(|r| b[r][c] * d[r]).sum::<f64>())
}

fn r3(v: [f64; 3]) -> Value {
    json!(v.map(|x| (x * 1000.0).round() / 1000.0))
}

fn pin_ok(glb: &[u8], cat: &Value, id: &str) {
    assert_eq!(
        sha256::hex(glb),
        catalogue_row(cat, id)["pin"]["model_sha256"].as_str().unwrap(),
        "{id}: the model is not the one the catalogue pins"
    );
}

/// The plate: slabs (thin, as wide as the build size, within 10 % of it) at the top of the bed; the highest top
/// is the plate's top, and of the slabs within a millimetre of it the squarest one gives the centre (a tab on a
/// sheet does not move the middle).
///
/// `lower` is how much shorter than the build size a slab may be (1 for every machine but the SnakeOil XY IDEX, whose plate is 254 x 243 for a 250 x 240 build).
/// F03 (2026-10-19): a sheet with a tab is longer than the bed it lies on (the Rook's PEI is 134 mm for a 120 mm bed), so `slack` is how much
/// longer than the build size a slab may be (1.1 for every machine measured before, 1.15 for the Rook), and `within` how far below the
/// highest top the squarest slab may lie (1 mm before; the Satsuma's heater plate lies 9.5 mm under the sheet that has the tab).
fn plate(all: &[Part], size: f64, slack: f64, within: f64, lower: f64) -> (f64, f64, f64, f64, f64) {
    let slabs: Vec<&Part> = all
        .iter()
        .filter(|p| {
            let (w, d, h) = (p.max[0] - p.min[0], p.max[2] - p.min[2], p.max[1] - p.min[1]);
            w >= size * lower && d >= size * lower && w <= size * slack && d <= size * slack && h < 10.0
        })
        .collect();
    let top = slabs.iter().map(|p| p.max[1]).fold(f64::MIN, f64::max);
    let best = slabs
        .iter()
        .filter(|p| p.max[1] >= top - within)
        .min_by(|a, b| {
            let sq = |p: &Part| ((p.max[0] - p.min[0]) - (p.max[2] - p.min[2])).abs();
            sq(a).total_cmp(&sq(b))
        })
        .unwrap_or_else(|| panic!("no plate of {size} mm found"));
    (
        (best.min[0] + best.max[0]) / 2.0,
        (best.min[2] + best.max[2]) / 2.0,
        top,
        best.max[0] - best.min[0],
        best.max[2] - best.min[2],
    )
}

/// The lowest part of a group, and the middle of that part: where the nozzle tip is (display frame).
fn tip(all: &[Part], keys: &[String]) -> Option<[f64; 3]> {
    all.iter()
        .filter(|p| keys.contains(&p.key))
        .min_by(|a, b| a.min[1].total_cmp(&b.min[1]))
        .map(|p| {
            [
                (p.min[0] + p.max[0]) / 2.0,
                p.min[1],
                (p.min[2] + p.max[2]) / 2.0,
            ]
        })
}

fn keys_of(motion: &Value, id: &str, group: &str) -> Vec<String> {
    motion["profiles"][id]["groups"][group]
        .as_array()
        .map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// Axes of the printer in the display frame: X to the right, Y toward the back (-Z), Z up.
const DISPLAY_AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];

struct Found {
    /// A machine measured in F03: the calibration is derived for whichever of the head and the bed carries each axis.
    generic: bool,
    id: &'static str,
    origin_display: [f64; 3],
    plate: [f64; 2],
    nozzle_display: Option<[f64; 3]>,
    /// The second carriage's nozzle (the SnakeOil XY IDEX).
    nozzle2_display: Option<[f64; 3]>,
    how: String,
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let prusa_only = a.iter().any(|x| x == "--prusa-only");
    a.retain(|x| x != "--prusa-only");
    let models = Path::new(&a[0]);
    let (out, prusa_out) = if prusa_only { (Path::new(&a[1]), Some(Path::new(&a[1]))) } else { (Path::new(&a[1]), a.get(2).map(Path::new)) };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cat = read_json(&root.join("web/printer-models.json"));
    // The machines added by hand beside the 3D Print Rig catalogue (the CORE One L).
    let extra = read_json(&root.join("web/printer-models-extra.json"));
    if let (Some(m), Some(x)) = (cat["machines"].as_array_mut(), extra["machines"].as_array()) {
        m.extend(x.iter().cloned());
    }
    let mut motion = read_json(&root.join("web/printer-motion.json"));
    for extra in ["web/printer-motion-extra.json", "web/printer-motion-parts.json"] {
        let x = read_json(&root.join(extra));
        for (k, v) in x["profiles"].as_object().unwrap() {
            motion["profiles"][k] = v.clone();
        }
    }
    let mut found: Vec<Found> = Vec::new();

    if !prusa_only {
    // The maker's own numbers (machine_profile.json): the printer's origin in the CAD frame (Z up) is where the
    // nozzle tip is in the model's pose minus where the maker says the nozzle is over the bed in that pose.
    for id in [
        "voron_v24_350_printed",
        "voron_v24_300_printed",
        "voron_v02_120",
        "micron_plus_r1_180",
    ] {
        let dir = models.join(format!("machines/{id}"));
        let glb = gunzip(&dir.join("model.glb.gz"));
        pin_ok(&glb, &cat, id);
        let prof = read_json(&dir.join("machine_profile.json"));
        let n = |k: &str, i: usize| prof[k][i].as_f64().unwrap_or_else(|| panic!("{id}: {k}"));
        let tipc = [0, 1, 2].map(|i| n("nozzle_tip_mm", i));
        let reference = [0, 1, 2].map(|i| n("display_reference_xyz_mm", i));
        let origin = [0, 1, 2].map(|i| tipc[i] - reference[i]);
        // The plate says the same: its top is the bed's top, and (where the maker wrote it) its corner.
        let top = prof["bed_top_world_z_mm"].as_f64().unwrap();
        assert!((origin[2] - top).abs() < 0.05, "{id}: the plate top {top} and the nozzle say different heights ({})", origin[2]);
        if let Some(c) = prof["bed_surface_min_xy_mm"].as_array() {
            for i in 0..2 {
                assert!((origin[i] - c[i].as_f64().unwrap()).abs() < 0.05, "{id}: the plate corner and the nozzle disagree");
            }
        }
        // CAD frame (Z up, Y toward the back) into the display frame the table is written in: the machine's basis.
        let b = basis_of(&cat, &motion, id);
        let disp = [0, 1, 2].map(|r| (0..3).map(|c| b[r][c] * origin[c]).sum::<f64>());
        let nozzle = [0, 1, 2].map(|r| (0..3).map(|c| b[r][c] * tipc[c]).sum::<f64>());
        let s = prof["build_surface_mm"][0].as_f64().unwrap_or_else(|| catalogue_row(&cat, id)["size_mm"].as_f64().unwrap());
        found.push(Found {
            generic: false,
            id: Box::leak(id.to_owned().into_boxed_str()),
            origin_display: disp,
            plate: [s, s],
            nozzle_display: Some(nozzle),
            nozzle2_display: None,
            how: format!("The maker's machine profile: the nozzle tip is at CAD {:.2}, {:.2}, {:.2} and is {:.2}, {:.2}, {:.2} mm over the bed in the model's pose, so printer 0,0,0 is the plate's corner at CAD {:.2}, {:.2}, {:.2}; the plate top ({top:.2}) agrees.", tipc[0], tipc[1], tipc[2], reference[0], reference[1], reference[2], origin[0], origin[1], origin[2]),
        });
    }

    // Machines whose maker wrote no origin: the plate is measured from the model.
    // (id, folder, the group whose lowest part is the nozzle, slack, within, generic: see `plate` and `Found`)
    let measured: [(&str, &str, &str, f64, f64, bool); 13] = [
        ("ender3_stock_220", "site/ender3", "head", 1.1, 1.0, false),
        ("voron_trident_300", "machines/voron_trident_300", "", 1.1, 1.0, false),
        ("sovol_sv08_350", "site/sv08-v2", "head", 1.1, 1.0, false),
        ("doomcube2_350_reference", "site/doomcube-v103/doomcube2_350_reference", "", 1.1, 1.0, false),
        // F03 (2026-10-19): the other machines with registered motion.
        ("fysetc_v24_250_pro", "machines/fysetc_v24_250_pro", "head", 1.1, 1.0, true),
        ("rook_mk2_120", "site/requested-v1/rook", "head", 1.15, 1.5, true),
        ("satsuma180_v10", "site/requested-v1/satsuma", "head", 1.1, 12.0, true),
        ("the100_v11_165", "site/requested-v1/the100", "head", 1.1, 1.0, true),
        ("tictac_21_120", "site/requested-v1/tictac", "head", 1.1, 1.0, true),
        // The Annex machines (matrix nodes, turned upright by their profile's `orient`; parts_oriented).
        ("annex_k1_assembly", "machines/annex_k1_assembly", "head", 1.1, 2.0, true),
        ("annex_k2_assembly", "machines/annex_k2_assembly", "head", 1.1, 2.0, true),
        ("annex_k3_assembly", "machines/annex_k3_assembly", "head", 1.1, 1.0, true),
        // The SnakeOil XY IDEX: two carriages (head, head2) on one beam; the second's nozzle and the dock-ends are measured below.
        ("snakeoil_xy_idex", "site/snakeidex", "head", 1.1, 1.0, true),
    ];
    for (id, dir, head, slack, within, generic) in measured {
        let glb = gunzip(&models.join(dir).join("model.glb.gz"));
        pin_ok(&glb, &cat, id);
        let orient: Option<[[f64; 3]; 3]> = motion["profiles"][id]["orient"].as_array().map(|r| std::array::from_fn(|i| std::array::from_fn(|j| r[i][j].as_f64().unwrap())));
        let all = if orient.is_some() { parts_oriented(&glb, orient) } else { parts(&glb) };
        let size = catalogue_row(&cat, id)["size_mm"].as_f64().unwrap();
        let (cx, cz, top, w, d) = plate(&all, size, slack, within, if id == "snakeoil_xy_idex" { 0.95 } else { 1.0 });
        let nozzle = if head.is_empty() { None } else { tip(&all, &keys_of(&motion, id, head)) };
        found.push(Found {
            generic,
            id,
            // The print area is the machine's build size, centred on the plate; Y0 is its front edge (+Z).
            origin_display: [cx - size / 2.0, top, cz + size / 2.0],
            plate: [size, size],
            nozzle_display: nozzle,
            nozzle2_display: if head.is_empty() { None } else { tip(&all, &keys_of(&motion, id, "head2")) },
            how: format!("Measured from the model (mm, display frame, Y up): the plate is a {w:.1} x {d:.1} slab centred at X {cx:.1}, Z {cz:.1}, top {top:.1}; the {size:.0} mm print area is centred on it and Y0 is its front edge (the +Z side, the project's convention).{}", nozzle.map_or(String::new(), |n| format!(" The nozzle tip is at X {:.1}, Z {:.1}, height {:.1}, so {:.1} mm over the plate.", n[0], n[2], n[1], n[1] - top))),
        });
    }

    }

    // The CORE One L: Prusa's CAD, with a calibration measured when the machine was added (printer-motion-extra.json).
    // Its calibration is the authority for where printer 0,0,0 is: the nozzle at the pose plus what the head moves at
    // reported zero, less what the bed moved. The nozzle is read off the model (the lowest part of its own "head" group)
    // when the file is given, and is the registered one (CAD -14.5, 80, 449) otherwise.
    {
        let id = "prusa_core_one_l";
        let b = basis_of(&cat, &motion, id);
        let prof = &motion["profiles"][id];
        let nozzle_cad = [-14.5, 80.0, 449.0];
        if let Some(p) = Some(models.join("prusa-core-one-l/model.glb.gz")).filter(|p| p.is_file()) {
            // The registered nozzle (Nextruder, picked by its part names when the machine was added) is the one used: the
            // lowest part of the head group is not the nozzle on this machine (something in the group reaches below
            // it). What the model can confirm is the sheet: its top is the Z0 the calibration touches.
            let glb = gunzip(&p);
            pin_ok(&glb, &cat, id);
            let all = parts(&glb);
            let sheet = all
                .iter()
                .filter(|p| p.group == "bed" && p.max[0] - p.min[0] > 250.0 && p.max[2] - p.min[2] > 250.0)
                .map(|p| p.max[1])
                .fold(f64::MIN, f64::max);
            println!("CORE One L: the bed sheet's top is {sheet:.2} in the model, the registered Z0 is {:.2} (nozzle {:.1} less its 21 mm)", nozzle_cad[2] - 21.0, nozzle_cad[2]);
            assert!((sheet - (nozzle_cad[2] - 21.0)).abs() < 1.0, "the CORE One L sheet is not where the calibration touches it");
        }
        let cal = &prof["calibration"];
        let axes_at_zero: [f64; 3] = [0, 1, 2].map(|i| {
            let raw = cal["offset_mm"][i].as_f64().unwrap();
            let (lo, hi) = (prof["axes"][["x", "y", "z"][i]][0].as_f64().unwrap(), prof["axes"][["x", "y", "z"][i]][1].as_f64().unwrap());
            raw.clamp(lo.min(hi), lo.max(hi))
        });
        let comp = |spec: &Value| -> f64 {
            match spec {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                Value::String(s) => ["x", "y", "z"].iter().position(|a| a == s).map_or(0.0, |i| axes_at_zero[i]),
                Value::Object(o) => o.iter().next().map_or(0.0, |(k, f)| ["x", "y", "z"].iter().position(|a| a == k).map_or(0.0, |i| axes_at_zero[i]) * f.as_f64().unwrap_or(1.0)),
                _ => 0.0,
            }
        };
        let disp_of = |name: &str| -> [f64; 3] { [0, 1, 2].map(|c| comp(&prof["motions"][name][c])) };
        let (head, bed) = (disp_of("head"), disp_of("bed"));
        let origin: [f64; 3] = [0, 1, 2].map(|c| nozzle_cad[c] + head[c] - bed[c]);
        let disp = [0, 1, 2].map(|r| (0..3).map(|c| b[r][c] * origin[c]).sum::<f64>());
        let nozzle = [0, 1, 2].map(|r| (0..3).map(|c| b[r][c] * nozzle_cad[c]).sum::<f64>());
        found.push(Found {
            generic: false,
            id,
            origin_display: disp,
            plate: [300.0, 300.0],
            nozzle_display: Some(nozzle),
            nozzle2_display: None,
            how: format!("From Prusa's CAD and the registered calibration (X0 is the sheet's left edge and Y0 its front edge, Z0 the sheet touching the nozzle): the Nextruder nozzle is at CAD {:.1}, {:.1}, {:.1} in the pose, the head moves {:.1}, {:.1} and the bed {:.1} to reported zero, so printer 0,0,0 is CAD {:.1}, {:.1}, {:.1}. X runs along CAD +Y and Y along CAD -X.", nozzle_cad[0], nozzle_cad[1], nozzle_cad[2], head[0], head[1], bed[2], origin[0], origin[1], origin[2]),
        });
    }

    let mut machines = Map::new();
    let mut prusa_machines = Map::new();
    for f in &found {
        let b = basis_of(&cat, &motion, f.id);
        let origin = to_frame(&b, f.origin_display);
        let axes: Vec<Value> = DISPLAY_AXES.iter().map(|a| r3(to_frame(&b, *a))).collect();
        // The printer's axes are the CORE One L's own, not the display convention (its X runs along CAD +Y).
        let axes = if f.id == "prusa_core_one_l" {
            vec![json!([0, 1, 0]), json!([-1, 0, 0]), json!([0, 0, 1])]
        } else {
            axes
        };
        let prof = &motion["profiles"][f.id];
        let bed_has_parts = prof["groups"]["bed"].as_array().is_some_and(|a| !a.is_empty())
            // The CORE One L's groups come from its model's own part records.
            || f.id == "prusa_core_one_l";
        let group_is_bed = bed_has_parts && prof["motions"].get("bed").is_some();
        let group = if group_is_bed { json!("bed") } else { Value::Null };
        let mut m = Map::new();
        m.insert("model_sha256".into(), catalogue_row(&cat, f.id)["pin"]["model_sha256"].clone());
        m.insert("origin_mm".into(), r3(origin));
        m.insert("axes".into(), Value::Array(axes));
        m.insert("group".into(), group);
        m.insert("plate_mm".into(), json!(f.plate));
        m.insert("nozzle_mm".into(), f.nozzle_display.map_or(Value::Null, |n| r3(to_frame(&b, n))));
        m.insert("basis".into(), json!(f.how));
        // A machine whose registration holds no calibration of its own starts at the low end of each travel, which puts the
        // nozzle wherever the CAD pose left it. With the plate and the nozzle both measured, the calibration that puts the
        // nozzle on the plate at printer 0,0,0 is a measurement, and the travel is widened to the print area (as the
        // Ender-3's was in polish WO-02): the part and the head then agree.
        // F03 (2026-10-19): whichever of the head and the bed carries an axis (the Rook's, Satsuma's and THE 100's bed carries Z, TicTac's carries Y), the
        // axis value that puts the nozzle at the printer coordinate P is `sign * P + offset` with, for factor k of the carrier's motion and s the
        // direction of the printer axis along that component: the head gives sign k*s and offset k*(origin - nozzle), the bed (the nozzle stands
        // still and the plate moves by k times the axis) gives sign -k*s and offset k*(nozzle - origin). A group's rest offset (the Satsuma's bed
        // starts 4 mm low) is in the pose before any axis moves it, so it is part of where the nozzle and the plate stand at axis zero.
        if let (true, Some(n), None) = (f.generic, f.nozzle_display, prof.get("calibration")) {
            let n_frame = to_frame(&b, n);
            let find = |group: &str, name: &str| -> Option<(usize, f64)> {
                let g = prof["motions"].get(group)?;
                (0..3).find_map(|c| match &g[c] {
                    Value::String(x) if x == name => Some((c, 1.0)),
                    Value::Object(o) => o.get(name).and_then(Value::as_f64).map(|k| (c, k)),
                    _ => None,
                })
            };
            let rest = |group: &str| -> [f64; 3] { [0, 1, 2].map(|c| prof["rest_offsets_mm"][group][c].as_f64().unwrap_or(0.0)) };
            let (rest_head, rest_bed) = (rest("head"), if group_is_bed { rest("bed") } else { [0.0; 3] });
            let (mut off, mut sign, mut by) = ([0.0_f64; 3], [1.0_f64; 3], Vec::new());
            let zspan = (prof["axes"]["z"][1].as_f64().unwrap() - prof["axes"]["z"][0].as_f64().unwrap()).max(1.0);
            let ext = [f.plate[0], f.plate[1], zspan];
            let mut axes = Map::new();
            for (i, name) in ["x", "y", "z"].iter().enumerate() {
                let dir = to_frame(&b, DISPLAY_AXES[i]);
                let (carrier, c, k) = match (find("head", name), find("bed", name)) {
                    (Some((c, k)), _) => ("head", c, k),
                    (None, Some((c, k))) => ("bed", c, k),
                    _ => panic!("{}: neither the head nor the bed moves with {name}", f.id),
                };
                let s = dir[c];
                assert!((s.abs() - 1.0).abs() < 1e-9 && k.abs() == 1.0, "{}: printer {name} does not run along component {c} of the motion", f.id);
                let gap = (origin[c] + rest_bed[c]) - (n_frame[c] + rest_head[c]);   // from the nozzle to the plate's origin, both as the pose stands at axis zero
                (sign[i], off[i]) = if carrier == "head" { (k * s, k * gap) } else { (-k * s, -k * gap) };
                let ends = [off[i], sign[i] * ext[i] + off[i]];
                axes.insert((*name).into(), json!([ends[0].min(ends[1]), ends[0].max(ends[1])]));
                by.push(carrier);
            }
            // Two carriages (the SnakeOil XY IDEX): the second tool's nozzle stands where it stands in the model, so while it prints its group follows the head's
            // position plus the difference between the two nozzles (the rest offset); whichever tool is not printing waits at its end of the X travel (X 0 for the
            // first, the plate's far edge for the second). Both nozzles are the lowest part of their carriage.
            let mut extra = Map::new();
            if prof["motions"].get("head2").is_some() {
                let n2 = f.nozzle2_display.expect("the second carriage has a nozzle");
                let n2_frame = to_frame(&b, n2);
                let rest: [f64; 3] = [0, 1, 2].map(|c| n_frame[c] - n2_frame[c]);
                extra.insert("rest_offsets_mm".into(), json!({"head2": rest.map(|v| (v * 1000.0).round() / 1000.0)}));
                let a = |p: f64| sign[0] * p + off[0];
                extra.insert("tools".into(), json!([{"group":"head","park":{"x":a(0.0)}},{"group":"head2","park":{"x":a(f.plate[0])}}]));
            }
            m.insert("profile".into(), json!({
                "axes":axes,
                "calibration":{"offset_mm":off,"sign":sign,"source":[0,1,2]},
                "calibration_basis":format!("Measured from the model: the nozzle tip is {:.1} mm over the plate, {:.1} mm from its left edge and {:.1} mm from its front edge in the model's pose. X is carried by the {}, Y by the {} and Z by the {}; reported 0,0,0 puts the nozzle at the plate's front left corner, and the travel is the {:.0} mm print area and the machine's registered Z travel.", n[1] - f.origin_display[1], n[0] - f.origin_display[0], f.origin_display[2] - n[2], by[0], by[1], by[2], f.plate[0]),
            }));
            if let Some(Value::Object(pr)) = m.get_mut("profile") {
                pr.extend(extra);
            }
        } else         if let (Some(n), true, Some(head)) = (f.nozzle_display, prof.get("calibration").is_none(), prof["motions"].get("head")) {
            let n_frame = to_frame(&b, n);
            let want = [0, 1, 2].map(|c| origin[c] - n_frame[c]);
            // Frame component c of the head's motion is: nothing, an axis name, or {axis: factor}.
            let mut off = [0.0_f64; 3];
            for c in 0..3 {
                let (name, k) = match &head[c] {
                    Value::String(s) => (s.clone(), 1.0),
                    Value::Object(o) => o.iter().next().map_or((String::new(), 0.0), |(k, f)| (k.clone(), f.as_f64().unwrap_or(1.0))),
                    _ => (String::new(), 0.0),
                };
                if let Some(i) = ["x", "y", "z"].iter().position(|a| *a == name) {
                    off[i] = want[c] * k;
                }
            }
            let hi_z = prof["axes"]["z"][1].as_f64().unwrap_or(off[2] + f.plate[0]);
            let axes = json!({"x":[off[0], off[0] + f.plate[0]],"y":[off[1], off[1] + f.plate[1]],"z":[off[2], hi_z.max(off[2] + 1.0)]});
            m.insert("profile".into(), json!({
                "axes":axes,
                "calibration":{"offset_mm":off,"sign":[1,1,1],"source":[0,1,2]},
                "calibration_basis":format!("Measured from the model: the nozzle tip is {:.1} mm over the plate, {:.1} mm from its left edge and {:.1} mm from its front edge in the model's pose, so reported 0,0,0 is that far back along each axis; the travel is widened to the {:.0} mm print area.", n[1] - f.origin_display[1], n[0] - f.origin_display[0], f.origin_display[2] - n[2], f.plate[0]),
            }));
        }
        println!("{}: origin {} group {} nozzle {}", f.id, m["origin_mm"], m["group"], m["nozzle_mm"]);
        machines.insert(f.id.to_owned(), Value::Object(m));
    }
    // F09 (2026-10-19): the other Prusa machines whose converted CAD ships in the build. Like the CORE One L, the calibration in
    // printer-motion-extra.json is the authority for where printer 0,0,0 is: the nozzle at the pose plus what the head moves at reported zero, less what
    // the bed moves; the model's own plate (`sheet`, the part whose top the nozzle touches at Z0) must be where that puts the plate's top.
    // (id, folder, nozzle tip in the CAD frame, print area, the printer's X, Y and Z as vectors in the CAD frame, the part key of the sheet)
    // The tip is the lowest point of the hotend part (tools/printer-cad glbtool: `lowest`), the sheet the part named in each model's notice.
    type PrusaFrame = (&'static str, &'static str, [f64; 3], [f64; 2], [[f64; 3]; 3], &'static str);
    let prusa: [PrusaFrame; 3] = [
        ("prusa_core_one", "prusa-core-one", [-1.17, -57.94, 329.0], [250.0, 220.0], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], "p00403"),
        ("prusa_mk3s", "prusa-mk3s", [-21.66, -47.47, 179.78], [250.0, 210.0], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], "p00043"),
        ("prusa_mini", "prusa-mini", [-41.3, 59.7, -93.4], [180.0, 180.0], [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]], "p00089"),
    ];
    for (id, dir, nozzle_cad, plate_mm, axes, sheet) in prusa {
        let glb = gunzip(&models.join(dir).join("model.glb.gz"));
        pin_ok(&glb, &cat, id);
        let b = basis_of(&cat, &motion, id);
        let prof = &motion["profiles"][id];
        let all = parts(&glb);
        let sheet_top = all.iter().find(|p| p.key == sheet).unwrap_or_else(|| panic!("{id}: no part {sheet}")).max[1];
        let cal = &prof["calibration"];
        let axes_at_zero: [f64; 3] = [0, 1, 2].map(|i| {
            let raw = cal["offset_mm"][i].as_f64().unwrap();
            let (lo, hi) = (prof["axes"][["x", "y", "z"][i]][0].as_f64().unwrap(), prof["axes"][["x", "y", "z"][i]][1].as_f64().unwrap());
            raw.clamp(lo.min(hi), lo.max(hi))
        });
        let comp = |spec: &Value| -> f64 {
            match spec {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                Value::String(s) => ["x", "y", "z"].iter().position(|a| a == s).map_or(0.0, |i| axes_at_zero[i]),
                Value::Object(o) => o.iter().next().map_or(0.0, |(k, f)| ["x", "y", "z"].iter().position(|a| a == k).map_or(0.0, |i| axes_at_zero[i]) * f.as_f64().unwrap_or(1.0)),
                _ => 0.0,
            }
        };
        let disp_of = |name: &str| -> [f64; 3] { [0, 1, 2].map(|c| comp(&prof["motions"][name][c])) };
        // The head and the gantry move with the head's axes; what carries the nozzle at reported zero is the head group, and the plate is carried by the bed.
        let (head, bed) = (disp_of("head"), disp_of("bed"));
        let origin: [f64; 3] = [0, 1, 2].map(|c| nozzle_cad[c] + head[c] - bed[c]);
        let disp = [0, 1, 2].map(|r| (0..3).map(|c| b[r][c] * origin[c]).sum::<f64>());
        assert!((sheet_top - disp[1]).abs() < 0.2, "{id}: the model's sheet top is {sheet_top:.2} and the calibration touches {:.2}", disp[1]);
        println!("{id}: the sheet {sheet} top is {sheet_top:.2} in the model, the calibration touches {:.2}", disp[1]);
        let mut m = Map::new();
        m.insert("model_sha256".into(), catalogue_row(&cat, id)["pin"]["model_sha256"].clone());
        m.insert("origin_mm".into(), r3(origin));
        m.insert("axes".into(), json!(axes));
        m.insert("group".into(), json!("bed"));
        m.insert("plate_mm".into(), json!(plate_mm));
        m.insert("nozzle_mm".into(), r3(nozzle_cad));
        m.insert("basis".into(), json!(format!("From the model's CAD and its registered calibration (X0 is the plate's left edge, Y0 its front edge, Z0 the sheet touching the nozzle): the nozzle tip is at CAD {:.2}, {:.2}, {:.2} in the pose (the lowest point of the hotend), the head moves {:.2}, {:.2}, {:.2} and the bed {:.2}, {:.2}, {:.2} to reported zero, so printer 0,0,0 is CAD {:.2}, {:.2}, {:.2} and the sheet's top ({sheet_top:.2}) is there. The margins of the plate on the sheet are the CORE One L's, measured in F02; here they are estimates.", nozzle_cad[0], nozzle_cad[1], nozzle_cad[2], head[0], head[1], head[2], bed[0], bed[1], bed[2], origin[0], origin[1], origin[2])));
        prusa_machines.insert(id.to_owned(), Value::Object(m));
    }

    // The nozzle of each tool of the StealthChanger bank, from the tool's own file: the centre of its lowest drawn faces, in the file's
    // display frame (the page puts the file at the registered place; `bank_tools` does the same sum). The files are the library's
    // `parts/<id>/model.glb.gz`, each checked against the catalogue's pin.
    let mut tools = Map::new();
    let parts_dir = models.join("parts");
    if parts_dir.is_dir() {
        let bank = cat["options"].as_array().unwrap().iter().find(|o| o["id"] == "stealthchanger_bank").expect("the StealthChanger bank is in the catalogue");
        for (tid, t) in bank["attach"]["tools"].as_object().unwrap() {
            let head = t["active"].as_array().unwrap().iter().find(|a| a["role"] == "head").expect("a tool has a head part");
            let part = head["part"].as_str().unwrap();
            let glb = gunzip(&parts_dir.join(part).join("model.glb.gz"));
            assert_eq!(sha256::hex(&glb), cat["parts"][part]["pin"]["model_sha256"].as_str().unwrap(), "{part}: not the file the catalogue pins");
            let hidden: std::collections::HashSet<String> = head["hidden_keys"].as_array().unwrap().iter().filter_map(|k| k.as_str().map(str::to_owned)).collect();
            let tip = lowest_tip(&glb, &hidden);
            println!("tool {tid}: tip in the file at X {:.2}, Y(up) {:.2}, Z {:.2}", tip[0], tip[1], tip[2]);
            tools.insert(tid.clone(), json!({"part":part,"tip_file_mm":r3(tip),"basis":"The centre of the lowest faces (within 0.25 mm of the lowest point) of the parts of the tool's head file that are drawn, in the file's display frame, millimetres."}));
        }
    }
    let doc = json!({
        "schema":"stella-scope.printer-print-frames.v1",
        "tools":tools,
        "source":"Read from each model file and the maker's machine profile by tools/printer-print-frames.rs",
        "about":"Where printer X0 Y0 Z0 is in the model's frame (millimetres, the frame the motion table is written in), the printer's axes in that frame, and the motion group that carries the plate. A print file is drawn at origin + x * axes[0] + y * axes[1] + z * axes[2].",
        "machines":machines,
    });
    if let Some(p) = prusa_out {
        let extra = json!({
            "schema":"stella-scope.printer-print-frames.v1",
            "source":"Read from each model file and the registered calibration by tools/printer-print-frames.rs (F09, 2026-10-19)",
            "about":"The Prusa machines whose converted CAD ships in the build: where printer X0 Y0 Z0 is in the model's frame, the printer's axes in that frame and the motion group that carries the plate, in the form of printer-print-frames.json.",
            "machines":prusa_machines,
        });
        fs::write(p, serde_json::to_vec_pretty(&extra).unwrap()).unwrap();
    }
    if !prusa_only {
        fs::write(out, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    }
}
