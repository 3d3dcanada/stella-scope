// Plate check evaluation (farm board WO-08): `cargo run --release --example plate-eval -- PHOTO.jpg`.
// There are no Stella camera frames of an empty and a loaded plate on this machine, so the set is made
// from one REAL phone photo of a textured plate with real prints on it: plain plate patches are the
// "empty" frames; the same patches with lighting, noise, blur and shifts are "still empty"; a real print
// cropped from the photo pasted at several sizes is "something there". The threshold is chosen on it and
// must be confirmed on the real camera (the live pass).
// The service's plate module, whole; this tool uses only part of it.
#[allow(dead_code)]
#[path = "../src/plate.rs"]
mod plate;

use image::{imageops, DynamicImage, GenericImageView, GrayImage, Luma};

fn lcg(s: &mut u32) -> f64 {
    *s = s.wrapping_mul(1664525).wrapping_add(1013904223);
    f64::from(*s >> 8) / f64::from(1_u32 << 24)
}
fn gauss(s: &mut u32) -> f64 {
    let (a, b) = (lcg(s).max(1e-9), lcg(s));
    (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
}
fn adjust(g: &GrayImage, gain: f64, off: f64, noise: f64, seed: u32) -> GrayImage {
    let mut s = seed;
    let mut o = g.clone();
    for p in o.pixels_mut() {
        let v = f64::from(p.0[0]) * gain + off + gauss(&mut s) * noise;
        *p = Luma([v.clamp(0.0, 255.0) as u8]);
    }
    o
}
fn shifted(g: &GrayImage, dx: i32, dy: i32) -> GrayImage {
    let (w, h) = g.dimensions();
    GrayImage::from_fn(w, h, |x, y| *g.get_pixel((x as i32 - dx).clamp(0, w as i32 - 1) as u32, (y as i32 - dy).clamp(0, h as i32 - 1) as u32))
}
fn jpeg_roundtrip(g: &GrayImage, q: u8) -> GrayImage {
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
    enc.encode_image(&DynamicImage::ImageLuma8(g.clone())).unwrap();
    image::load_from_memory(&buf).unwrap().to_luma8()
}

struct Case {
    name: String,
    expect_clear: bool,
    img: GrayImage,
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: plate-eval PHOTO.jpg");
    let photo = image::open(&path).expect("photo").to_luma8();
    let (pw, ph) = photo.dimensions();
    // Plain-plate patches, as fractions of the photo (checked by eye on the source: no print in them).
    let patches = [("band-top", 0.0, 0.08, 1.0, 0.23), ("band-bottom", 0.0, 0.80, 1.0, 0.99)];
    // Prints cropped from the photo.
    let prints = [("round-tag", 0.10, 0.46, 0.43, 0.75), ("white-tag", 0.45, 0.45, 0.80, 0.78)];
    let crop = |(x0, y0, x1, y1): (f64, f64, f64, f64)| -> GrayImage {
        imageops::crop_imm(&photo, (x0 * f64::from(pw)) as u32, (y0 * f64::from(ph)) as u32, ((x1 - x0) * f64::from(pw)) as u32, ((y1 - y0) * f64::from(ph)) as u32).to_image()
    };
    let mut cases: Vec<(String, GrayImage, Vec<Case>)> = Vec::new();
    for (pname, x0, y0, x1, y1) in patches {
        let base = crop((x0, y0, x1, y1));
        let (bw, bh) = base.dimensions();
        let mut list = Vec::new();
        let mut seed = 11;
        for (gain, off) in [(1.0, 0.0), (0.6, 0.0), (0.8, -6.0), (1.25, 8.0), (1.6, 0.0), (0.45, 4.0)] {
            seed += 1;
            list.push(Case { name: format!("same, gain {gain} offset {off}"), expect_clear: true, img: adjust(&base, gain, off, 0.0, seed) });
        }
        for n in [3.0, 6.0, 10.0] {
            seed += 1;
            list.push(Case { name: format!("same, sensor noise {n}"), expect_clear: true, img: adjust(&base, 1.0, 0.0, n, seed) });
        }
        list.push(Case { name: "same, blurred 1.2".into(), expect_clear: true, img: imageops::blur(&base, 1.2) });
        list.push(Case { name: "same, JPEG q50".into(), expect_clear: true, img: jpeg_roundtrip(&base, 50) });
        for d in [bw as i32 / 192, bw as i32 / 96] {
            list.push(Case { name: format!("same, camera nudged {d} px"), expect_clear: true, img: shifted(&base, d, d / 2) });
        }
        list.push(Case { name: "same, dim room + noise + JPEG".into(), expect_clear: true, img: jpeg_roundtrip(&adjust(&base, 0.55, 2.0, 5.0, 99), 60) });
        // Something there: a real print pasted at a share of the patch's area, at three places.
        for (tname, tx0, ty0, tx1, ty1) in prints {
            let tag = crop((tx0, ty0, tx1, ty1));
            for share in [0.015_f64, 0.04, 0.10, 0.25] {
                for (px, py, plabel) in [(0.2, 0.2, "left"), (0.5, 0.35, "middle"), (0.75, 0.5, "right")] {
                    let area = f64::from(bw) * f64::from(bh) * share;
                    let (tw0, th0) = tag.dimensions();
                    let k = (area / (f64::from(tw0) * f64::from(th0))).sqrt();
                    let (tw, th) = (((f64::from(tw0) * k) as u32).max(2), ((f64::from(th0) * k) as u32).max(2));
                    if tw >= bw || th >= bh {
                        continue;
                    }
                    let small = imageops::resize(&tag, tw, th, imageops::FilterType::Triangle);
                    let mut frame = adjust(&base, 1.1, 3.0, 3.0, 5 + (share * 1000.0) as u32);
                    imageops::overlay(&mut frame, &small, (px * f64::from(bw - tw)) as i64, (py * f64::from(bh - th)) as i64);
                    list.push(Case { name: format!("{tname} {:.1}% of the region, {plabel}", share * 100.0), expect_clear: false, img: frame });
                }
            }
        }
        // A purge blob: plate-coloured but lighter, small.
        let mut blob = adjust(&base, 1.0, 0.0, 2.0, 77);
        let (cx, cy) = (bw as f64 * 0.6, bh as f64 * 0.5);
        for y in 0..bh {
            for x in 0..bw {
                let (dx, dy) = ((f64::from(x) - cx) / (f64::from(bw) * 0.03), (f64::from(y) - cy) / (f64::from(bh) * 0.12));
                if dx * dx + dy * dy < 1.0 {
                    let p = blob.get_pixel_mut(x, y);
                    p.0[0] = (f64::from(p.0[0]) + 70.0).min(255.0) as u8;
                }
            }
        }
        list.push(Case { name: "purge blob, 1.1% of the region, +70 levels".into(), expect_clear: false, img: blob });
        cases.push((pname.to_owned(), base, list));
    }
    println!("{:<14} {:<48} {:>7} {:>6} {:>7} {:>8}", "patch", "case", "expect", "cells", "share", "verdict");
    let quad = plate::Quad::whole();
    let mut rows = Vec::new();
    for (pname, base, list) in &cases {
        let empty = plate::reduce(&DynamicImage::ImageLuma8(base.clone()));
        for c in list {
            let now = plate::reduce(&DynamicImage::ImageLuma8(c.img.clone()));
            let v = plate::compare(&empty, &now, &quad, 1.0);
            let ok = v.clear == c.expect_clear;
            println!("{:<14} {:<48} {:>7} {:>6} {:>6.1}% {:>8}{}", pname, c.name, if c.expect_clear { "clear" } else { "NOT" }, v.cells_different, v.score * 100.0, if v.clear { "clear" } else { "NOT" }, if ok { "" } else { "   <-- WRONG" });
            rows.push((pname.clone(), c.name.clone(), c.expect_clear, empty.clone(), now));
        }
    }
    // Sweep the judgement: grid, per-cell threshold, shift tolerance and how many cells make it "not clear".
    println!("\ntuning sweep: false alarms on the {} still-empty frames / misses on the {} frames with a part", rows.iter().filter(|r| r.2).count(), rows.iter().filter(|r| !r.2).count());
    let mut best: Vec<(String, usize, usize)> = Vec::new();
    for (cols, rws) in [(24_u32, 14_u32), (48, 27), (64, 36)] {
        for cell_z in [0.35, 0.45, 0.6, 0.8] {
            for shift in [1_i32, 2, 3] {
                for (min_cells, share, min_cluster) in [(4_u32, 0.004_f64, 1_u32), (4, 0.004, 3), (6, 0.005, 3), (8, 0.006, 3), (8, 0.006, 4), (12, 0.008, 4), (8, 0.006, 1)] {
                    let t = plate::Tuning { cols, rows: rws, cell_z, shift, min_cells, share, min_cluster };
                    let (mut fa, mut miss) = (0, 0);
                    for (_, _, expect_clear, empty, now) in &rows {
                        let v = plate::compare_with(empty, now, &quad, 1.0, &t);
                        if *expect_clear && !v.clear {
                            fa += 1;
                        }
                        if !*expect_clear && v.clear {
                            miss += 1;
                        }
                    }
                    best.push((format!("grid {cols}x{rws} cell_z {cell_z} shift {shift} min_cells {min_cells} share {share} min_cluster {min_cluster}"), fa, miss));
                }
            }
        }
    }
    best.sort_by_key(|(_, fa, miss)| (*fa, *miss));
    for (label, fa, miss) in best.iter().take(14) {
        println!("  false alarms {fa:>2}  misses {miss:>2}   {label}");
    }
    let d = plate::Tuning::default();
    let (mut fa, mut miss) = (0, 0);
    for (_, _, expect_clear, empty, now) in &rows {
        let v = plate::compare(empty, now, &quad, 1.0);
        if *expect_clear && !v.clear {
            fa += 1;
        }
        if !*expect_clear && v.clear {
            miss += 1;
        }
    }
    println!("\nthe service default ({}x{}, cell_z {}, shift {}, min_cells {}, share {}, min_cluster {}): false alarms {fa}, misses {miss}", d.cols, d.rows, d.cell_z, d.shift, d.min_cells, d.share, d.min_cluster);
    let _ = GenericImageView::dimensions(&photo);
}
