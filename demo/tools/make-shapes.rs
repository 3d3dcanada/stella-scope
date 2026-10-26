// The demo printers' sample shapes (polish board WO-02, 2026-10-18), made for Stella Scope, no outside model.
//
//   rustc -O --edition 2021 demo/tools/make-shapes.rs -o /tmp/make-shapes && /tmp/make-shapes OUTDIR
//
// Writes twist-vase.stl, twist-bowl.stl (the two shapes that shipped first, so they can be sliced again),
// duo-half-a.stl and duo-half-b.stl (the duo about the origin, for thumbnails),
// small-twist-vase.stl (for 120 mm machines) and duo-twist.3mf (two halves of one twisted column, each its
// own object on its own extruder, for IDEX machines and toolchangers). No dependency: the 3MF is a stored zip
// written here. The shapes and the slicer settings are in demo/README.md.
//
// A shape is a stack of rings. Ring t (0..1) has radius r(t) = r0 + (r1 - r0) t + bulge sin(pi t); each point
// at angle a is pushed out by 1 + depth cos(lobes a) and the ring is turned by twist * t. The ends are closed.
// The duo is the lobed column cut along its axis: half A takes angles 0..pi and half B pi..2pi, each moved
// 0.4 mm off the cut plane so the two objects never touch.
use std::{collections::HashMap, f64::consts::PI, fs, path::Path};

struct Shape {
    lobes: f64,
    r0: f64,
    r1: f64,
    h: f64,
    twist_deg: f64,
    depth: f64,
    bulge: f64,
    rings: usize,
    pts: usize,
}

type V = [f64; 3];

/// Closed triangle mesh of a swept loop. `loop_at(t)` is the loop at height fraction t, counter-clockwise from
/// above, and `centre_at(t)` a point the loop is star-shaped around (the fan for the caps). The loop of a
/// full ring has the centre inside it; a half ring has it on its straight cut.
fn sweep(
    shape: &Shape,
    rings: usize,
    loop_at: &dyn Fn(f64) -> Vec<[f64; 2]>,
    centre_at: &dyn Fn(f64) -> [f64; 2],
    at: [f64; 2],
) -> Vec<[V; 3]> {
    let turn = shape.twist_deg.to_radians();
    let place = |p: [f64; 2], t: f64| -> V {
        let (s, c) = (turn * t).sin_cos();
        [
            at[0] + p[0] * c - p[1] * s,
            at[1] + p[0] * s + p[1] * c,
            shape.h * t,
        ]
    };
    let ts: Vec<f64> = (0..=rings).map(|i| i as f64 / rings as f64).collect();
    let loops: Vec<Vec<V>> = ts
        .iter()
        .map(|&t| loop_at(t).into_iter().map(|p| place(p, t)).collect())
        .collect();
    let centres: Vec<V> = ts.iter().map(|&t| place(centre_at(t), t)).collect();
    let n = loops[0].len();
    let mut tris = Vec::new();
    for i in 0..rings {
        for k in 0..n {
            let (a0, b0) = (loops[i][k], loops[i][(k + 1) % n]);
            let (a1, b1) = (loops[i + 1][k], loops[i + 1][(k + 1) % n]);
            tris.push([a0, b0, b1]);
            tris.push([a0, b1, a1]);
        }
    }
    // The fan does not use the centre when it is already a vertex of the loop (the half rings).
    let fan = |ring: usize, up: bool, tris: &mut Vec<[V; 3]>| {
        let c = centres[ring];
        let l = &loops[ring];
        for k in 0..n {
            let (a, b) = (l[k], l[(k + 1) % n]);
            if same(a, c) || same(b, c) {
                continue;
            }
            tris.push(if up { [c, a, b] } else { [c, b, a] });
        }
    };
    fan(0, false, &mut tris);
    fan(rings, true, &mut tris);
    tris
}

fn same(a: V, b: V) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 && (a[2] - b[2]).abs() < 1e-9
}

fn radius(s: &Shape, t: f64) -> f64 {
    s.r0 + (s.r1 - s.r0) * t + s.bulge * (PI * t).sin()
}

fn full(s: &Shape) -> Vec<[V; 3]> {
    let loop_at = |t: f64| -> Vec<[f64; 2]> {
        (0..s.pts)
            .map(|k| {
                let a = 2.0 * PI * k as f64 / s.pts as f64;
                let r = radius(s, t) * (1.0 + s.depth * (s.lobes * a).cos());
                [r * a.cos(), r * a.sin()]
            })
            .collect()
    };
    sweep(s, s.rings, &loop_at, &|_| [0.0, 0.0], [0.0, 0.0])
}

/// One half of the column: `from` is 0 or pi. The half is moved `gap` mm off the cut plane.
fn half(s: &Shape, from: f64, gap: f64, at: [f64; 2]) -> Vec<[V; 3]> {
    let dy = if from == 0.0 { gap } else { -gap };
    let loop_at = |t: f64| -> Vec<[f64; 2]> {
        let mut v: Vec<[f64; 2]> = (0..=s.pts)
            .map(|k| {
                let a = from + PI * k as f64 / s.pts as f64;
                let r = radius(s, t) * (1.0 + s.depth * (s.lobes * a).cos());
                [r * a.cos(), r * a.sin() + dy]
            })
            .collect();
        v.push([0.0, dy]); // the centre sits on the cut and closes the loop
        v
    };
    sweep(s, s.rings, &loop_at, &|_| [0.0, dy], at)
}

/// Every edge must be used once each way, or the slicer will complain.
fn check_closed(name: &str, tris: &[[V; 3]]) {
    let key = |v: V| {
        (
            (v[0] * 1e5).round() as i64,
            (v[1] * 1e5).round() as i64,
            (v[2] * 1e5).round() as i64,
        )
    };
    let mut edges: HashMap<(_, _), i32> = HashMap::new();
    for t in tris {
        for i in 0..3 {
            let (a, b) = (key(t[i]), key(t[(i + 1) % 3]));
            *edges.entry((a, b)).or_default() += 1;
        }
    }
    let bad = edges
        .iter()
        .filter(|(&(a, b), &n)| n != 1 || edges.get(&(b, a)) != Some(&1))
        .count();
    assert_eq!(bad, 0, "{name}: {bad} edges are not shared once each way");
    let vol: f64 = tris
        .iter()
        .map(|t| {
            let (a, b, c) = (t[0], t[1], t[2]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum();
    assert!(
        vol > 0.0,
        "{name}: the triangles face inwards (volume {vol})"
    );
    println!("{name}: {} triangles, volume {:.0} mm3", tris.len(), vol);
}

fn stl(path: &Path, name: &str, tris: &[[V; 3]]) {
    let mut out = vec![0u8; 80];
    let label = name.as_bytes();
    out[..label.len().min(80)].copy_from_slice(&label[..label.len().min(80)]);
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for t in tris {
        let (u, v) = (
            [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]],
            [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]],
        );
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        for x in [n[0] / l, n[1] / l, n[2] / l] {
            out.extend_from_slice(&(x as f32).to_le_bytes());
        }
        for p in t {
            for x in p {
                out.extend_from_slice(&(*x as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&[0, 0]);
    }
    fs::write(path, out).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A zip of stored (uncompressed) entries.
fn zip(path: &Path, files: &[(&str, Vec<u8>)]) {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let (crc, off) = (crc32(data), out.len() as u32);
        let head = |central: bool| -> Vec<u8> {
            let mut h = Vec::new();
            h.extend_from_slice(if central {
                &[0x50, 0x4b, 1, 2, 20, 0][..]
            } else {
                &[0x50, 0x4b, 3, 4][..]
            });
            h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]); // version, flags, stored, time, date
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                h.extend_from_slice(&[0; 6]); // comment length, disk number, internal attributes
                h.extend_from_slice(&[0; 4]); // external attributes
                h.extend_from_slice(&off.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend_from_slice(&head(false));
        out.extend_from_slice(data);
        central.extend_from_slice(&head(true));
    }
    let (cd_off, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    fs::write(path, out).unwrap();
}

/// A PrusaSlicer 3MF of objects, each on its own extruder (1-based), vertices already where they print.
fn threemf(path: &Path, objects: &[(&str, usize, Vec<[V; 3]>)]) {
    let mut model = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" xmlns:slic3rpe=\"http://schemas.slic3r.org/3mf/2017/06\">\n <metadata name=\"slic3rpe:Version3mf\">1</metadata>\n <resources>\n");
    let mut config = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    let mut build = String::new();
    for (i, (name, extruder, tris)) in objects.iter().enumerate() {
        let id = i + 1;
        let mut index: HashMap<(i64, i64, i64), usize> = HashMap::new();
        let (mut verts, mut faces) = (String::new(), String::new());
        let mut count = 0usize;
        for t in tris {
            let mut ids = [0usize; 3];
            for (j, p) in t.iter().enumerate() {
                let k = (
                    (p[0] * 1e4).round() as i64,
                    (p[1] * 1e4).round() as i64,
                    (p[2] * 1e4).round() as i64,
                );
                ids[j] = *index.entry(k).or_insert_with(|| {
                    verts += &format!(
                        "    <vertex x=\"{:.4}\" y=\"{:.4}\" z=\"{:.4}\"/>\n",
                        p[0], p[1], p[2]
                    );
                    count += 1;
                    count - 1
                });
            }
            faces += &format!(
                "    <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n",
                ids[0], ids[1], ids[2]
            );
        }
        model += &format!("  <object id=\"{id}\" type=\"model\">\n   <mesh>\n    <vertices>\n{verts}    </vertices>\n    <triangles>\n{faces}    </triangles>\n   </mesh>\n  </object>\n");
        build += &format!("  <item objectid=\"{id}\" printable=\"1\"/>\n");
        config += &format!(
            " <object id=\"{id}\" instances_count=\"1\">\n  <metadata type=\"object\" key=\"name\" value=\"{name}\"/>\n  <metadata type=\"object\" key=\"extruder\" value=\"{extruder}\"/>\n  <volume firstid=\"0\" lastid=\"{}\">\n   <metadata type=\"volume\" key=\"name\" value=\"{name}\"/>\n   <metadata type=\"volume\" key=\"volume_type\" value=\"ModelPart\"/>\n   <metadata type=\"volume\" key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n  </volume>\n </object>\n",
            tris.len() - 1
        );
    }
    model += &format!(" </resources>\n <build>\n{build} </build>\n</model>\n");
    config += "</config>\n";
    let types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n</Types>\n";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n</Relationships>\n";
    zip(
        path,
        &[
            ("[Content_Types].xml", types.as_bytes().to_vec()),
            ("_rels/.rels", rels.as_bytes().to_vec()),
            ("3D/3dmodel.model", model.into_bytes()),
            ("Metadata/Slic3r_PE_model.config", config.into_bytes()),
        ],
    );
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: make-shapes OUTDIR");
    let dir = Path::new(&out);
    fs::create_dir_all(dir).unwrap();
    let vase = Shape {
        lobes: 6.0,
        r0: 34.0,
        r1: 26.0,
        h: 110.0,
        twist_deg: 120.0,
        depth: 0.12,
        bulge: 12.0,
        rings: 80,
        pts: 96,
    };
    let bowl = Shape {
        lobes: 5.0,
        r0: 30.0,
        r1: 55.0,
        h: 55.0,
        twist_deg: 72.0,
        depth: 0.14,
        bulge: 4.0,
        rings: 80,
        pts: 96,
    };
    let small = Shape {
        lobes: 5.0,
        r0: 22.0,
        r1: 18.0,
        h: 60.0,
        twist_deg: 90.0,
        depth: 0.10,
        bulge: 6.0,
        rings: 60,
        pts: 96,
    };
    let duo = Shape {
        lobes: 3.0,
        r0: 28.0,
        r1: 24.0,
        h: 70.0,
        twist_deg: 180.0,
        depth: 0.12,
        bulge: 4.0,
        rings: 70,
        pts: 48,
    };
    for (file, name, s) in [
        ("twist-vase.stl", "twist_vase", &vase),
        ("twist-bowl.stl", "twist_bowl", &bowl),
        ("small-twist-vase.stl", "small_twist_vase", &small),
    ] {
        let t = full(s);
        check_closed(name, &t);
        stl(&dir.join(file), name, &t);
    }
    // The duo is placed where it prints: centred on 125,125 (a 250 mm bed), half A on extruder 1.
    let c = [125.0, 125.0];
    let (a, b) = (half(&duo, 0.0, 0.4, c), half(&duo, PI, 0.4, c));
    check_closed("duo_half_a", &a);
    check_closed("duo_half_b", &b);
    threemf(
        &dir.join("duo-twist.3mf"),
        &[("duo half A", 1, a), ("duo half B", 2, b)],
    );
    // The same two halves about the origin, for the picture the thumbnail tool draws.
    stl(
        &dir.join("duo-half-a.stl"),
        "duo_half_a",
        &half(&duo, 0.0, 0.4, [0.0, 0.0]),
    );
    stl(
        &dir.join("duo-half-b.stl"),
        "duo_half_b",
        &half(&duo, PI, 0.4, [0.0, 0.0]),
    );
}
