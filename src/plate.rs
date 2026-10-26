//! Is the plate empty? (farm board WO-08.) Compares a camera frame with the frame the person saved of the
//! empty plate, inside the four corners they marked, and says which part differs.
//!
//! It is a comparison, not a recogniser: it knows "this region looks different from the empty plate",
//! not what the thing is. That is what is wanted (a part left behind, a purge blob, a dropped tool) and
//! it does not depend on the failure watch model, which is an owner add-on.
//!
//! Lighting tolerance: both images are reduced to grey at a working size, each is normalised to zero mean
//! and unit spread over the region (so a brighter room, a different exposure or a colour cast does not
//! register), and a cell is compared against the best of nine one-pixel shifts of the reference (so a
//! printer's vibration or a nudged camera does not either).
type Result<T> = std::result::Result<T, String>;
use image::{imageops::FilterType, DynamicImage, GrayImage};

/// Working size of both images. A 1280x720 frame is a 6.7x reduction, which also averages sensor noise.
pub const W: u32 = 192;
pub const H: u32 = 108;
/// How a comparison is judged. `Default` is what the service uses; `tools/plate-eval.rs` sweeps these
/// on the evaluation set (the table is in the WO-08 STOP-REPORT).
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// The grid the differences are found and reported on.
    pub cols: u32,
    pub rows: u32,
    /// A cell differs when its mean normalised difference is above this.
    pub cell_z: f64,
    /// The reference is tried at every shift of up to this many working pixels.
    pub shift: i32,
    /// The plate is not clear when at least this many cells differ, or this share of the region's cells.
    pub min_cells: u32,
    pub share: f64,
    /// A differing cell counts only when it touches at least this many others (itself included): a
    /// stray cell of sensor noise or a speck of dust is not a part.
    pub min_cluster: u32,
}
impl Default for Tuning {
    fn default() -> Self {
        Self {
            cols: 48,
            rows: 27,
            cell_z: 0.6,
            shift: 3,
            // Finish-2 F02 (CORE One L, 2026-10-19): the 25 mm test vase left on the plate showed as 3 cells at its own place and
            // nothing anywhere else, and 8 cells were needed, so it was "clear". A part left behind is printed onto; a false
            // alarm only asks a person. Any cluster of min_cluster cells now counts.
            min_cells: 3,
            share: 0.0,
            min_cluster: 3,
        }
    }
}
/// The grid the service reports and the page draws (the default tuning's).
pub const COLS: u32 = 48;
pub const ROWS: u32 = 27;

#[derive(Clone, Debug)]
pub struct Gray {
    pub px: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad(pub [[f64; 2]; 4]);
impl Quad {
    pub fn whole() -> Self {
        Self([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
    }
    pub fn valid(&self) -> bool {
        let inside = self
            .0
            .iter()
            .all(|p| p[0] >= 0.0 && p[0] <= 1.0 && p[1] >= 0.0 && p[1] <= 1.0);
        // Shoelace area: a sliver or a bow-tie is not a plate.
        let mut a = 0.0;
        for i in 0..4 {
            let (p, q) = (self.0[i], self.0[(i + 1) % 4]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        inside && a.abs() / 2.0 > 0.05
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        // Even-odd rule over the four edges.
        let mut inside = false;
        for i in 0..4 {
            let (a, b) = (self.0[i], self.0[(i + 1) % 4]);
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
        inside
    }
}

/// Decode a JPEG or PNG frame and reduce it to the working grey image.
pub fn prepare(bytes: &[u8]) -> Result<Gray> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("the camera frame could not be read: {e}"))?;
    Ok(reduce(&img))
}

pub fn reduce(img: &DynamicImage) -> Gray {
    let g: GrayImage = image::imageops::resize(&img.to_luma8(), W, H, FilterType::Triangle);
    Gray { px: g.into_raw() }
}

/// Re-hydrate a stored working image.
pub fn from_raw(px: Vec<u8>) -> Result<Gray> {
    if px.len() != (W * H) as usize {
        return Err(
            "the saved empty-plate frame is not the size this version expects; capture it again"
                .into(),
        );
    }
    Ok(Gray { px })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub clear: bool,
    /// Share of the region's cells that differ, 0 to 1.
    pub score: f64,
    pub cells_different: u32,
    pub cells_in_region: u32,
    /// Bounding box of the cells that differ, normalised: left, top, right, bottom.
    pub bbox: Option<[f64; 4]>,
    /// The cells that differ as (column, row) in a COLS x ROWS grid.
    pub cells: Vec<[u32; 2]>,
}

/// The region's level and spread, by median and median absolute deviation: a big bright part on the
/// plate must not drag the level and inflate the spread, which would make every other cell look changed.
fn stats(g: &Gray, mask: &[bool]) -> (f64, f64) {
    let mut hist = [0_u32; 256];
    let mut n = 0_u32;
    for (i, &m) in mask.iter().enumerate() {
        if m {
            hist[usize::from(g.px[i])] += 1;
            n += 1;
        }
    }
    let nth = |h: &[u32; 256], k: u32| -> usize {
        let mut acc = 0;
        for (v, &c) in h.iter().enumerate() {
            acc += c;
            if acc > k {
                return v;
            }
        }
        255
    };
    let median = nth(&hist, n / 2);
    let mut dev = [0_u32; 256];
    for (v, &c) in hist.iter().enumerate() {
        dev[v.abs_diff(median)] += c;
    }
    let mad = nth(&dev, n / 2);
    // A flat region (a dark plate in dim light) must not turn noise into signal: a floor on the spread.
    (median as f64, (1.4826 * mad as f64).max(4.0))
}

/// Compare `now` with `empty` inside `quad`. `sensitivity` scales the per-cell threshold (1.0 is the
/// default; 0.7 notices less, 1.4 notices more).
pub fn compare(empty: &Gray, now: &Gray, quad: &Quad, sensitivity: f64) -> Verdict {
    compare_with(empty, now, quad, sensitivity, &Tuning::default())
}

pub fn compare_with(
    empty: &Gray,
    now: &Gray,
    quad: &Quad,
    sensitivity: f64,
    t: &Tuning,
) -> Verdict {
    let mask: Vec<bool> = (0..(W * H) as usize)
        .map(|i| {
            quad.contains(
                ((i as u32 % W) as f64 + 0.5) / f64::from(W),
                ((i as u32 / W) as f64 + 0.5) / f64::from(H),
            )
        })
        .collect();
    let (me, se) = stats(empty, &mask);
    let (mn, sn) = stats(now, &mask);
    let z = |g: &Gray, m: f64, s: f64, x: i32, y: i32| -> f64 {
        let (x, y) = (
            x.clamp(0, W as i32 - 1) as usize,
            y.clamp(0, H as i32 - 1) as usize,
        );
        (f64::from(g.px[y * W as usize + x]) - m) / s
    };
    let (cols, rows) = (t.cols, t.rows);
    let (cw, ch) = (
        f64::from(W) / f64::from(cols),
        f64::from(H) / f64::from(rows),
    );
    let threshold = t.cell_z / sensitivity.clamp(0.3, 3.0);
    let mut in_region = 0_u32;
    let mut flagged = vec![false; (cols * rows) as usize];
    for cy in 0..rows {
        for cx in 0..cols {
            let (x0, x1) = ((f64::from(cx) * cw) as i32, (f64::from(cx + 1) * cw) as i32);
            let (y0, y1) = ((f64::from(cy) * ch) as i32, (f64::from(cy + 1) * ch) as i32);
            // Pixels of this cell that are inside the region.
            let mut px = Vec::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    if mask[(y as u32 * W + x as u32) as usize] {
                        px.push((x, y));
                    }
                }
            }
            if px.len() * 10 < ((x1 - x0).max(1) * (y1 - y0).max(1)) as usize * 4 {
                continue; // under 40 % inside: the edge of the region, not part of it
            }
            in_region += 1;
            // The best of every small shift of the reference: a nudged camera is not a part.
            let mut best = f64::MAX;
            for dy in -t.shift..=t.shift {
                for dx in -t.shift..=t.shift {
                    let mut sum = 0.0;
                    for &(x, y) in &px {
                        sum += (z(now, mn, sn, x, y) - z(empty, me, se, x + dx, y + dy)).abs();
                    }
                    best = best.min(sum / px.len() as f64);
                }
            }
            if best > threshold {
                flagged[(cy * cols + cx) as usize] = true;
            }
        }
    }
    // Keep only clusters (8-neighbour) of at least `min_cluster` cells.
    let mut seen = vec![false; flagged.len()];
    let mut cells = Vec::new();
    let (mut l, mut top, mut r, mut bot) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for start in 0..flagged.len() {
        if !flagged[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        let mut comp = Vec::new();
        seen[start] = true;
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (cx, cy) = ((i as u32 % cols) as i32, (i as u32 / cols) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                        continue;
                    }
                    let j = (ny as u32 * cols + nx as u32) as usize;
                    if flagged[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if comp.len() as u32 >= t.min_cluster {
            for i in comp {
                let (cx, cy) = (i as u32 % cols, i as u32 / cols);
                cells.push([cx, cy]);
                l = l.min(f64::from(cx) / f64::from(cols));
                r = r.max(f64::from(cx + 1) / f64::from(cols));
                top = top.min(f64::from(cy) / f64::from(rows));
                bot = bot.max(f64::from(cy + 1) / f64::from(rows));
            }
        }
    }
    let different = cells.len() as u32;
    let score = if in_region == 0 {
        0.0
    } else {
        f64::from(different) / f64::from(in_region)
    };
    let allowed = t
        .min_cells
        .max((f64::from(in_region) * t.share).ceil() as u32);
    let clear = different < allowed;
    Verdict {
        clear,
        score,
        cells_different: different,
        cells_in_region: in_region,
        bbox: (different > 0).then_some([l, top, r, bot]),
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a gate: compares two real camera frames with a region, for live evidence (finish-2 F02). PLATE_EMPTY, PLATE_NOW
    /// (JPEG paths), PLATE_REGION (JSON, four [x,y] in 0..1). `cargo test --release real_frames -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_frames_compared() {
        let read = |k: &str| prepare(&std::fs::read(std::env::var(k).unwrap()).unwrap()).unwrap();
        let r: Vec<[f64; 2]> = serde_json::from_str(&std::env::var("PLATE_REGION").unwrap()).unwrap();
        let v = compare(&read("PLATE_EMPTY"), &read("PLATE_NOW"), &Quad([r[0], r[1], r[2], r[3]]), 1.0);
        let loose = compare_with(&read("PLATE_EMPTY"), &read("PLATE_NOW"), &Quad([r[0], r[1], r[2], r[3]]), 1.0, &Tuning { min_cells: 1, share: 0.0, min_cluster: 1, cell_z: std::env::var("CELL_Z").ok().and_then(|s| s.parse().ok()).unwrap_or(0.6), ..Tuning::default() });
        eprintln!("LOOSE cells_different={} box={:?}", loose.cells_different, loose.bbox);
        eprintln!(
            "VERDICT clear={} score={:.3} cells_different={} of {} box={:?}",
            v.clear, v.score, v.cells_different, v.cells_in_region, v.bbox
        );
    }

    /// A plate-like scene: a mid-grey base with a deterministic fine texture and a soft gradient.
    fn scene(seed: u32, gain: f64, offset: f64) -> Gray {
        let mut px = vec![0_u8; (W * H) as usize];
        let mut s = seed;
        for y in 0..H {
            for x in 0..W {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                let tex = f64::from((s >> 24) as u8) / 255.0 * 40.0 - 20.0;
                let grad = f64::from(x) / f64::from(W) * 20.0;
                px[(y * W + x) as usize] =
                    (((100.0 + tex + grad) * gain) + offset).clamp(0.0, 255.0) as u8;
            }
        }
        Gray { px }
    }
    fn with_block(mut g: Gray, x0: u32, y0: u32, w: u32, h: u32, v: u8) -> Gray {
        for y in y0..(y0 + h).min(H) {
            for x in x0..(x0 + w).min(W) {
                g.px[(y * W + x) as usize] = v;
            }
        }
        g
    }

    #[test]
    fn the_same_plate_in_other_light_is_clear() {
        let empty = scene(7, 1.0, 0.0);
        let q = Quad::whole();
        for (gain, off) in [(1.0, 0.0), (1.3, 10.0), (0.7, -8.0), (1.6, 0.0)] {
            // Same texture (same seed), different exposure.
            let v = compare(&empty, &scene(7, gain, off), &q, 1.0);
            assert!(v.clear, "gain {gain} offset {off}: {v:?}");
            assert_eq!(v.cells_different, 0, "gain {gain}");
        }
    }

    #[test]
    fn a_part_left_on_the_plate_is_not_clear_and_is_located() {
        let empty = scene(7, 1.0, 0.0);
        let now = with_block(scene(7, 1.1, 4.0), 80, 40, 28, 24, 215);
        let v = compare(&empty, &now, &Quad::whole(), 1.0);
        assert!(!v.clear, "{v:?}");
        let b = v.bbox.unwrap();
        // The block sits at x 80..108 of 192, y 40..64 of 108: the box must cover it and not the plate.
        assert!(
            b[0] <= 80.0 / 192.0 + 0.05 && b[2] >= 108.0 / 192.0 - 0.05,
            "{b:?}"
        );
        assert!(
            b[1] <= 40.0 / 108.0 + 0.1 && b[3] >= 64.0 / 108.0 - 0.1,
            "{b:?}"
        );
        assert!(
            b[2] - b[0] < 0.4 && b[3] - b[1] < 0.5,
            "the box is the part, not the plate: {b:?}"
        );
    }

    /// Finish-2 F02: a small part (the 25 mm test vase was 3 cells on the CORE One L's camera) is not clear either.
    #[test]
    fn a_small_part_is_not_clear() {
        let empty = scene(7, 1.0, 0.0);
        let now = with_block(scene(7, 1.0, 0.0), 100, 50, 9, 9, 20);
        let v = compare(&empty, &now, &Quad::whole(), 1.0);
        assert!(!v.clear, "{v:?}");
        assert!(v.cells_different <= 9, "only the part: {v:?}");
    }

    #[test]
    fn a_part_outside_the_marked_region_does_not_count() {
        let empty = scene(7, 1.0, 0.0);
        let now = with_block(scene(7, 1.0, 0.0), 5, 5, 30, 20, 230);
        let inner = Quad([[0.4, 0.2], [0.95, 0.2], [0.95, 0.95], [0.4, 0.95]]);
        assert!(compare(&empty, &now, &inner, 1.0).clear);
        assert!(!compare(&empty, &now, &Quad::whole(), 1.0).clear);
    }

    #[test]
    fn a_nudged_camera_is_not_a_part() {
        let empty = scene(7, 1.0, 0.0);
        let mut moved = scene(7, 1.0, 0.0);
        // Shift the whole image one pixel to the right.
        for y in 0..H {
            for x in (1..W).rev() {
                moved.px[(y * W + x) as usize] = moved.px[(y * W + x - 1) as usize];
            }
        }
        let v = compare(&empty, &moved, &Quad::whole(), 1.0);
        assert!(v.clear, "{v:?}");
    }

    #[test]
    fn a_region_must_be_a_real_shape() {
        assert!(Quad::whole().valid());
        assert!(
            !Quad([[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 0.0]]).valid(),
            "flat"
        );
        assert!(
            !Quad([[0.1, 0.1], [1.2, 0.1], [1.0, 0.9], [0.1, 0.9]]).valid(),
            "outside the frame"
        );
        assert!(
            !Quad([[0.5, 0.5], [0.52, 0.5], [0.52, 0.52], [0.5, 0.52]]).valid(),
            "a dot"
        );
    }

    #[test]
    fn a_frame_that_is_not_an_image_is_refused_in_words() {
        assert!(prepare(b"not a jpeg")
            .unwrap_err()
            .contains("could not be read"));
        assert!(from_raw(vec![0; 10]).is_err());
    }
}
