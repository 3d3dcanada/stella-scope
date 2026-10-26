// L1: resonance capture, Welch PSD and input-shaper fitting, computed here rather than read
// back from Klippain Shake&Tune's PNGs. The maths is a line-by-line port of Klipper
// `klippy/extras/shaper_calibrate.py` + `shaper_defs.py` (v0.13.0-535 as installed on Stella
// 2026-09-20), so the numbers this returns are comparable with SHAPER_CALIBRATE's own output.
//
// Deviations from that source, deliberate, and why:
//  * BOARD-4 asks for a 0..200 Hz / 0.5 Hz display grid. Klipper's bin width is fs/nfft
//    (1.5625 Hz at 3200 Hz), so the PSD is COMPUTED on Klipper's native bins — the shaper fit
//    uses those untouched — and only the JSON `psd` block is resampled onto the 0.5 Hz grid.
//    Fitting on a resampled PSD would change the answer; displaying a resampled one does not.
//  * `peak_hz` is refined by three-point parabolic interpolation on log power, so the reported
//    peak is not quantised to a 1.5625 Hz bin. Klipper never reports a peak frequency at all.
//  * `max_smoothing` is always unset (Klipper's default), so `fit_shaper`'s early-return on
//    excessive smoothing is not ported. If a max_smoothing knob is ever wanted, that branch
//    comes back with it.
//  * The window is Kaiser(beta=6), exactly as Klipper's `_psd` uses, NOT the Hann that BOARD-4's
//    parenthetical names. Matching Klipper matters more than matching the parenthetical: the
//    whole point is that these numbers can be checked against SHAPER_CALIBRATE. The window in
//    force is reported as `psd.window` in the JSON so nobody has to guess.
//
// Capture rides Moonraker's Klippy bridge (`ws://host:7125/klippysocket`), where each websocket
// message is one complete Klippy API JSON document — Moonraker appends and strips the 0x03
// terminator itself (moonraker/components/websockets.py, BridgeSocket). The subscription is
// Klipper's mux endpoint `adxl345/dump_adxl345`; batches arrive as the `response_template` we
// supplied with `params.data` = [[time, ax, ay, az], ...] (klippy/extras/bulk_sensor.py).
use crate::{
    api::Response, config::Config, moonraker::Moonraker, store::db_error, ws::WsClient, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------------------------
// Constants, all from Klipper
// ---------------------------------------------------------------------------------------------

/// shaper_calibrate.MIN_FREQ
const MIN_FREQ: f64 = 5.0;
/// shaper_calibrate.MAX_FREQ — also BOARD-4's display ceiling.
const MAX_FREQ: f64 = 200.0;
/// shaper_calibrate.WINDOW_T_SEC
const WINDOW_T_SEC: f64 = 0.5;
/// shaper_calibrate.MAX_SHAPER_FREQ
const MAX_SHAPER_FREQ: f64 = 150.0;
/// shaper_calibrate.TEST_DAMPING_RATIOS
const TEST_DAMPING_RATIOS: [f64; 3] = [0.075, 0.1, 0.15];
/// shaper_defs.DEFAULT_DAMPING_RATIO
const DEFAULT_DAMPING_RATIO: f64 = 0.1;
/// shaper_defs.SHAPER_VIBRATION_REDUCTION
const SHAPER_VIBRATION_REDUCTION: f64 = 20.0;
/// shaper_calibrate.fit_shaper's default `freq_step`.
const FREQ_STEP: f64 = 0.2;
/// find_shaper_max_accel.TARGET_SMOOTHING
const TARGET_SMOOTHING: f64 = 0.12;
/// Klipper's default `square_corner_velocity`, used when the printer does not answer.
const DEFAULT_SCV: f64 = 5.0;
/// BOARD-4's display bin width.
const BIN_HZ: f64 = 0.5;

/// Klipper's AUTOTUNE_SHAPERS, in `shaper_defs.INPUT_SHAPERS` order (zvd is not autotuned), with
/// each shaper's `min_freq` — chosen upstream so projected max_accel is about 1500.
const SHAPERS: &[(&str, f64)] = &[
    ("zv", 21.0),
    ("mzv", 23.0),
    ("ei", 29.0),
    ("2hump_ei", 39.0),
    ("3hump_ei", 48.0),
];

/// One leg is capped here rather than by trusting the printer to stop: 3200 Hz * 375 s.
const MAX_SAMPLES_PER_LEG: usize = 1_200_000;
/// Raw samples are written in chunks of this many rows, as f32 quads.
const RAW_CHUNK: usize = 32_768;

// ---------------------------------------------------------------------------------------------
// FFT
// ---------------------------------------------------------------------------------------------

/// In-place iterative radix-2 Cooley-Tukey FFT. `re.len()` must be a power of two and equal
/// `im.len()`. Written out rather than pulled in: the only transform this binary ever needs is
/// one power-of-two complex forward FFT, and a crate for that is a dependency for forty lines.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    if n < 2 {
        return;
    }
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (angle.cos(), angle.sin());
        let mut start = 0usize;
        while start < n {
            let (mut cr, mut ci) = (1.0_f64, 0.0_f64);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next_r = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next_r;
            }
            start += len;
        }
        len <<= 1;
    }
}

/// Modified Bessel function of the first kind, order zero — the Kaiser window's kernel. Power
/// series; the argument never exceeds beta (6.0) here, so it converges in a handful of terms.
fn bessel_i0(x: f64) -> f64 {
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let half = x * 0.5;
    for k in 1..60 {
        term *= (half / k as f64) * (half / k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// `numpy.kaiser(n, beta)`.
fn kaiser(n: usize, beta: f64) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    let alpha = (n - 1) as f64 / 2.0;
    let denominator = bessel_i0(beta);
    (0..n)
        .map(|i| {
            let r = (i as f64 - alpha) / alpha;
            bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / denominator
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Welch PSD
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Psd {
    pub freq: Vec<f64>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub sum: Vec<f64>,
    pub fs: f64,
    pub nfft: usize,
    pub windows: usize,
}

/// `ShaperCalibrate._psd`: Welch's method with a Kaiser(6) window, 50 % overlap, per-window
/// detrend, one-sided scaling with the interior bins doubled.
fn welch(x: &[f64], fs: f64, nfft: usize, window: &[f64], scale: f64) -> Vec<f64> {
    let overlap = nfft / 2;
    let step = nfft - overlap;
    let n_windows = if x.len() > overlap {
        (x.len() - overlap) / step
    } else {
        0
    };
    let bins = nfft / 2 + 1;
    let mut acc = vec![0.0_f64; bins];
    let mut re = vec![0.0_f64; nfft];
    let mut im = vec![0.0_f64; nfft];
    for w in 0..n_windows {
        let start = w * step;
        let slice = &x[start..start + nfft];
        let mean = slice.iter().sum::<f64>() / nfft as f64;
        for i in 0..nfft {
            re[i] = (slice[i] - mean) * window[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        for (bin, acc) in acc.iter_mut().enumerate().take(bins) {
            let power = re[bin] * re[bin] + im[bin] * im[bin];
            // One-sided: double everything but DC and the unpaired Nyquist bin.
            let doubled = if bin > 0 && bin + 1 < bins { 2.0 } else { 1.0 };
            *acc += power * scale / fs * doubled;
        }
    }
    if n_windows == 0 {
        return acc;
    }
    for value in acc.iter_mut() {
        *value /= n_windows as f64;
    }
    acc
}

/// `ShaperCalibrate.calc_freq_response`. `data` rows are [time, ax, ay, az] exactly as the
/// adxl345 dump endpoint delivers them.
pub fn calc_psd(data: &[[f64; 4]]) -> Result<Psd> {
    let n = data.len();
    if n < 64 {
        return Err(format!(
            "only {n} accelerometer samples arrived; the sweep produced no usable data"
        ));
    }
    let span = data[n - 1][0] - data[0][0];
    if !(span.is_finite()) || span <= 0.0 {
        return Err(
            "accelerometer timestamps do not advance; the capture is unusable, not merely noisy"
                .into(),
        );
    }
    let fs = n as f64 / span;
    if !(100.0..=20_000.0).contains(&fs) {
        return Err(format!(
            "derived sample rate {fs:.0} Hz is outside 100..20000 Hz; check the accelerometer wiring"
        ));
    }
    // Klipper: M = 1 << int(SAMPLING_FREQ * WINDOW_T_SEC - 1).bit_length()
    let k = (fs * WINDOW_T_SEC - 1.0).max(0.0) as u64;
    let nfft = if k == 0 {
        1usize
    } else {
        1usize << (64 - k.leading_zeros())
    };
    if n <= nfft {
        return Err(format!(
            "{n} samples is not more than the {nfft}-point analysis window; run a longer sweep"
        ));
    }
    let window = kaiser(nfft, 6.0);
    let scale = 1.0 / window.iter().map(|w| w * w).sum::<f64>();
    let column = |c: usize| -> Vec<f64> { data.iter().map(|row| row[c]).collect() };
    let px = welch(&column(1), fs, nfft, &window, scale);
    let py = welch(&column(2), fs, nfft, &window, scale);
    let pz = welch(&column(3), fs, nfft, &window, scale);
    let bins = nfft / 2 + 1;
    let freq: Vec<f64> = (0..bins).map(|i| i as f64 * fs / nfft as f64).collect();
    let sum: Vec<f64> = (0..bins).map(|i| px[i] + py[i] + pz[i]).collect();
    let overlap = nfft / 2;
    Ok(Psd {
        freq,
        x: px,
        y: py,
        z: pz,
        sum,
        fs,
        nfft,
        windows: (n - overlap) / (nfft - overlap),
    })
}

/// `CalibrationData.normalize_to_frequencies`, applied to the copy the shaper fit consumes.
fn normalize_to_frequencies(freq: &[f64], psd: &mut [f64]) {
    for (f, p) in freq.iter().zip(psd.iter_mut()) {
        *p /= f + 0.1;
        if *f < 2.0 * MIN_FREQ {
            *p *= (-(2.0 * MIN_FREQ / (f + 0.1)).powi(2) + 1.0).exp();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shaper definitions — shaper_defs.py
// ---------------------------------------------------------------------------------------------

/// Horner evaluation of Klipper's `_get_shaper_from_expansion_coeffs`. Note `k = len(a[0])`
/// upstream: the 3hump table's first `a` row has three entries where the rest have four (an
/// upstream typo, `3.29160 -1.44380` with no comma), so Klipper evaluates a *quadratic* in the
/// damping ratio for that shaper and ignores every fourth coefficient. That quirk is reproduced
/// here on purpose — matching Klipper is the point, and "fixing" it would change the answer.
fn expansion(freq: f64, dr: f64, t: &[&[f64]], a: &[&[f64]]) -> (Vec<f64>, Vec<f64>) {
    let tau = 1.0 / freq;
    let k = a[0].len();
    let mut out_t = Vec::with_capacity(t.len());
    let mut out_a = Vec::with_capacity(a.len());
    for i in 0..t.len() {
        let mut u = t[i][k - 1];
        let mut v = a[i][k - 1];
        for j in 0..k - 1 {
            u = u * dr + t[i][k - j - 2];
            v = v * dr + a[i][k - j - 2];
        }
        out_t.push(u * tau);
        out_a.push(v);
    }
    (out_a, out_t)
}

/// (A, T) impulse amplitudes and times for one shaper at one frequency, per `shaper_defs.py`.
fn shaper_impulses(name: &str, freq: f64, dr: f64) -> (Vec<f64>, Vec<f64>) {
    let df = (1.0 - dr * dr).sqrt();
    let t_d = 1.0 / (freq * df);
    match name {
        "zv" => {
            let k = (-dr * std::f64::consts::PI / df).exp();
            (vec![1.0, k], vec![0.0, 0.5 * t_d])
        }
        "mzv" => {
            let k = (-0.75 * dr * std::f64::consts::PI / df).exp();
            let a1 = 1.0 - 1.0 / 2.0_f64.sqrt();
            let a2 = (2.0_f64.sqrt() - 1.0) * k;
            let a3 = a1 * k * k;
            (vec![a1, a2, a3], vec![0.0, 0.375 * t_d, 0.75 * t_d])
        }
        "ei" => {
            let v = 1.0 / SHAPER_VIBRATION_REDUCTION;
            let a1 = (0.24968 + 0.24961 * v)
                + ((0.80008 + 1.23328 * v) + (0.49599 + 3.17316 * v) * dr) * dr;
            let a3 = (0.25149 + 0.21474 * v)
                + ((-0.83249 + 1.41498 * v) + (0.85181 - 4.90094 * v) * dr) * dr;
            let a2 = 1.0 - a1 - a3;
            let t2 = 0.4999
                + (((0.46159 + 8.57843 * v) * v)
                    + (((4.26169 - 108.644 * v) * v) + ((1.75601 + 336.989 * v) * v) * dr) * dr)
                    * dr;
            (vec![a1, a2, a3], vec![0.0, t2 * t_d, t_d])
        }
        "2hump_ei" => expansion(
            freq,
            dr,
            &[
                &[0.0, 0.0, 0.0, 0.0],
                &[0.49890, 0.16270, -0.54262, 6.16180],
                &[0.99748, 0.18382, -1.58270, 8.17120],
                &[1.49920, -0.09297, -0.28338, 1.85710],
            ],
            &[
                &[0.16054, 0.76699, 2.26560, -1.22750],
                &[0.33911, 0.45081, -2.58080, 1.73650],
                &[0.34089, -0.61533, -0.68765, 0.42261],
                &[0.15997, -0.60246, 1.00280, -0.93145],
            ],
        ),
        "3hump_ei" => expansion(
            freq,
            dr,
            &[
                &[0.0, 0.0, 0.0, 0.0],
                &[0.49974, 0.23834, 0.44559, 12.4720],
                &[0.99849, 0.29808, -2.36460, 23.3990],
                &[1.49870, 0.10306, -2.01390, 17.0320],
                &[1.99960, -0.28231, 0.61536, 5.40450],
            ],
            &[
                // Three entries, not four — see `expansion`. Upstream reads `3.29160 -1.44380`.
                &[0.11275, 0.76632, 3.29160 - 1.44380],
                &[0.23698, 0.61164, -2.57850, 4.85220],
                &[0.30008, -0.19062, -2.14560, 0.13744],
                &[0.23775, -0.73297, 0.46885, -2.08650],
                &[0.11244, -0.45439, 0.96382, -1.46000],
            ],
        ),
        other => unreachable!("unknown shaper {other}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Shaper fitting — shaper_calibrate.py
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ShaperFit {
    pub name: &'static str,
    pub freq: f64,
    pub vibrs: f64,
    pub smoothing: f64,
    pub score: f64,
    pub max_accel: f64,
}

/// `ShaperCalibrate._estimate_shaper`, evaluated for one test frequency across all bins.
fn estimate_shaper(a: &[f64], t: &[f64], dr: f64, freqs: &[f64]) -> Vec<f64> {
    let inv_d = 1.0 / a.iter().sum::<f64>();
    let t_last = *t.last().unwrap_or(&0.0);
    freqs
        .iter()
        .map(|f| {
            let omega = 2.0 * std::f64::consts::PI * f;
            let damping = dr * omega;
            let omega_d = omega * (1.0 - dr * dr).sqrt();
            let (mut s, mut c) = (0.0_f64, 0.0_f64);
            for i in 0..a.len() {
                let w = a[i] * (-damping * (t_last - t[i])).exp();
                s += w * (omega_d * t[i]).sin();
                c += w * (omega_d * t[i]).cos();
            }
            (s * s + c * c).sqrt() * inv_d
        })
        .collect()
}

/// `ShaperCalibrate._estimate_remaining_vibrations`.
fn remaining_vibrations(a: &[f64], t: &[f64], dr: f64, freqs: &[f64], psd: &[f64]) -> f64 {
    let vals = estimate_shaper(a, t, dr, freqs);
    let threshold = psd.iter().cloned().fold(f64::MIN, f64::max) / SHAPER_VIBRATION_REDUCTION;
    let remaining: f64 = vals
        .iter()
        .zip(psd)
        .map(|(v, p)| (v * p - threshold).max(0.0))
        .sum();
    let all: f64 = psd.iter().map(|p| (p - threshold).max(0.0)).sum();
    if all <= 0.0 {
        0.0
    } else {
        remaining / all
    }
}

/// `ShaperCalibrate._get_shaper_smoothing`.
fn shaper_smoothing(a: &[f64], t: &[f64], accel: f64, scv: f64) -> f64 {
    let half_accel = accel * 0.5;
    let inv_d = 1.0 / a.iter().sum::<f64>();
    let ts: f64 = a.iter().zip(t).map(|(a, t)| a * t).sum::<f64>() * inv_d;
    let (mut offset_90, mut offset_180) = (0.0_f64, 0.0_f64);
    for i in 0..a.len() {
        if t[i] >= ts {
            offset_90 += a[i] * (scv + half_accel * (t[i] - ts)) * (t[i] - ts);
        }
        offset_180 += a[i] * half_accel * (t[i] - ts) * (t[i] - ts);
    }
    offset_90 *= inv_d * 2.0_f64.sqrt();
    offset_180 *= inv_d;
    offset_90.max(offset_180)
}

/// `ShaperCalibrate.find_shaper_max_accel` — `_bisect` on the smoothing target.
fn find_shaper_max_accel(a: &[f64], t: &[f64], scv: f64) -> f64 {
    let ok = |accel: f64| shaper_smoothing(a, t, accel, scv) <= TARGET_SMOOTHING;
    if !ok(1e-9) {
        return 0.0;
    }
    let (mut left, mut right) = (1.0_f64, 1.0_f64);
    while !ok(left) {
        right = left;
        left *= 0.5;
    }
    if right == left {
        while ok(right) {
            right *= 2.0;
        }
    }
    while right - left > 1e-8 {
        let middle = (left + right) * 0.5;
        if ok(middle) {
            left = middle;
        } else {
            right = middle;
        }
    }
    left
}

/// `ShaperCalibrate.fit_shaper` for a single dataset. Returns `(selected, results)`, where
/// `results` is in the order Klipper appends them: descending test frequency.
fn fit_shaper(
    name: &'static str,
    min_freq: f64,
    freq_bins: &[f64],
    psd: &[f64],
    scv: f64,
) -> Option<(ShaperFit, Vec<ShaperFit>)> {
    let freq_end = MAX_SHAPER_FREQ;
    let freq_start = min_freq.min(freq_end - 1e-7);
    let mut test_freqs = Vec::new();
    let mut f = freq_start;
    while f < freq_end {
        test_freqs.push(f);
        f += FREQ_STEP;
    }
    if test_freqs.is_empty() {
        return None;
    }
    // Klipper narrows the PSD to `freq_bins <= max_freq` inside the loop; do it once.
    let cut: Vec<(f64, f64)> = freq_bins
        .iter()
        .zip(psd)
        .filter(|(f, _)| **f <= MAX_FREQ)
        .map(|(f, p)| (*f, *p))
        .collect();
    if cut.is_empty() {
        return None;
    }
    let band_freqs: Vec<f64> = cut.iter().map(|(f, _)| *f).collect();
    let band_psd: Vec<f64> = cut.iter().map(|(_, p)| *p).collect();

    let mut best: Option<ShaperFit> = None;
    let mut results: Vec<ShaperFit> = Vec::with_capacity(test_freqs.len());
    for test_freq in test_freqs.iter().rev() {
        let (a, t) = shaper_impulses(name, *test_freq, DEFAULT_DAMPING_RATIO);
        let smoothing = shaper_smoothing(&a, &t, 5000.0, scv);
        let max_accel = find_shaper_max_accel(&a, &t, scv);
        // Exact damping is unknown; pessimise over the three test ratios.
        let mut vibrs = 0.0_f64;
        for dr in TEST_DAMPING_RATIOS {
            vibrs = vibrs.max(remaining_vibrations(&a, &t, dr, &band_freqs, &band_psd));
        }
        let score = smoothing * (vibrs.powf(1.5) + vibrs * 0.2 + 0.01);
        let fit = ShaperFit {
            name,
            freq: *test_freq,
            vibrs,
            smoothing,
            score,
            max_accel,
        };
        if best.as_ref().is_none_or(|b| b.vibrs > fit.vibrs) {
            best = Some(fit.clone());
        }
        results.push(fit);
    }
    let best = best?;
    // "Not much worse than the best, but much less smoothing."
    let mut selected = best.clone();
    for res in results.iter().rev() {
        if res.vibrs < best.vibrs * 1.1 + 0.0005 && res.score < selected.score {
            selected = res.clone();
        }
    }
    Some((selected, results))
}

/// `ShaperCalibrate.find_best_shaper`. Returns the recommendation and the per-shaper table
/// BOARD-4 asks for, one row per shaper in Klipper's own order.
pub fn find_best_shaper(
    freq_bins: &[f64],
    psd: &[f64],
    scv: f64,
) -> Option<(ShaperFit, Vec<ShaperFit>)> {
    let mut best: Option<ShaperFit> = None;
    let mut all: Vec<ShaperFit> = Vec::new();
    for (name, min_freq) in SHAPERS {
        let Some((selected, results)) = fit_shaper(name, *min_freq, freq_bins, psd, scv) else {
            continue;
        };
        let mut shaper = selected;
        let take = match best.as_ref() {
            None => true,
            Some(b) => {
                shaper.score * 1.2 < b.score
                    || (shaper.score * 1.05 < b.score && shaper.smoothing * 1.1 < b.smoothing)
            }
        };
        if take {
            best = Some(shaper.clone());
        }
        for s in results.iter().rev() {
            let b = best.as_ref().expect("best is set by the first shaper");
            if s.vibrs < b.vibrs && s.smoothing < b.smoothing {
                best = Some(s.clone());
                shaper = s.clone();
            }
        }
        all.push(shaper);
    }
    let mut best = best?;
    // zv is the least smoothing but the weakest; prefer a real shaper if it is clearly better.
    if best.name == "zv" {
        if let Some(tuned) = all
            .iter()
            .find(|s| s.name != "zv" && s.vibrs * 1.1 < best.vibrs)
        {
            best = tuned.clone();
        }
    }
    Some((best, all))
}

// ---------------------------------------------------------------------------------------------
// Display grid, peaks, belts similarity
// ---------------------------------------------------------------------------------------------

/// The 0..200 Hz / 0.5 Hz grid BOARD-4 specifies for the JSON `psd` block.
fn display_grid() -> Vec<f64> {
    let n = (MAX_FREQ / BIN_HZ) as usize + 1;
    (0..n).map(|i| i as f64 * BIN_HZ).collect()
}

/// `numpy.interp`: linear interpolation, clamped at both ends. `xs` must ascend.
fn interp(grid: &[f64], xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(grid.len());
    let mut i = 0usize;
    for g in grid {
        if xs.is_empty() {
            out.push(0.0);
            continue;
        }
        while i + 2 < xs.len() && xs[i + 1] < *g {
            i += 1;
        }
        if *g <= xs[0] {
            out.push(ys[0]);
        } else if *g >= xs[xs.len() - 1] {
            out.push(ys[ys.len() - 1]);
        } else {
            let span = xs[i + 1] - xs[i];
            let f = if span > 0.0 { (*g - xs[i]) / span } else { 0.0 };
            out.push(ys[i] + (ys[i + 1] - ys[i]) * f);
        }
    }
    out
}

/// Local maxima of the PSD above 5 Hz, refined to sub-bin accuracy and sorted by power.
/// Parabolic interpolation on log power: the Kaiser main lobe is near-Gaussian, so the log
/// domain is where a three-point fit is unbiased. Klipper reports no peak at all, so this is
/// ours; it is what makes `peak_hz` finer than the 1.5625 Hz native bin.
fn peaks(freq: &[f64], psd: &[f64], limit: usize) -> Vec<(f64, f64)> {
    let max = psd.iter().cloned().fold(f64::MIN, f64::max);
    if !(max > 0.0) {
        return Vec::new();
    }
    let mut found: Vec<(f64, f64)> = Vec::new();
    for i in 1..psd.len().saturating_sub(1) {
        if freq[i] < MIN_FREQ || freq[i] > MAX_FREQ {
            continue;
        }
        if psd[i] <= psd[i - 1] || psd[i] < psd[i + 1] || psd[i] < max * 0.05 {
            continue;
        }
        let (y1, y2, y3) = (
            psd[i - 1].max(1e-300).ln(),
            psd[i].max(1e-300).ln(),
            psd[i + 1].max(1e-300).ln(),
        );
        let denominator = y1 - 2.0 * y2 + y3;
        let delta = if denominator.abs() > 1e-12 {
            (0.5 * (y1 - y3) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let df = freq[i] - freq[i - 1];
        found.push((freq[i] + delta * df, psd[i]));
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    found.truncate(limit);
    found
}

/// Normalised zero-lag cross-correlation of two PSDs over the shared display grid — the number
/// Shake&Tune shows as belt similarity. 1.0 means the two belts respond identically.
pub fn similarity(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let (mut ab, mut aa, mut bb) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..n {
        // Below MIN_FREQ the sweep never excited anything; including it would inflate every
        // comparison towards 1.0 by correlating two copies of the same noise floor.
        if (i as f64) * BIN_HZ < MIN_FREQ {
            continue;
        }
        ab += a[i] * b[i];
        aa += a[i] * a[i];
        bb += b[i] * b[i];
    }
    if aa <= 0.0 || bb <= 0.0 {
        return 0.0;
    }
    ab / (aa * bb).sqrt()
}

/// Everything BOARD-4's `GET /api/resonance/<id>` needs for one leg.
struct Leg {
    name: &'static str,
    psd: Psd,
    grid_sum: Vec<f64>,
}

fn analyse(name: &'static str, data: &[[f64; 4]]) -> Result<Leg> {
    let psd = calc_psd(data)?;
    let grid = display_grid();
    let grid_sum = interp(&grid, &psd.freq, &psd.sum);
    Ok(Leg {
        name,
        psd,
        grid_sum,
    })
}

// ---------------------------------------------------------------------------------------------
// Capture over the Klippy bridge
// ---------------------------------------------------------------------------------------------

/// One capture at a time, process-wide. The command service's own busy permit is released as
/// soon as the HTTP response returns, which for this command is before the sweep has run.
static BUSY: AtomicBool = AtomicBool::new(false);

struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// The G-code each leg runs, and the label its samples are stored under. `belts` is the
/// Shake&Tune pair: both belts pulled together, then in opposition.
pub fn legs(axis: &str) -> Result<Vec<(&'static str, &'static str)>> {
    Ok(match axis {
        "X" => vec![("x", "TEST_RESONANCES AXIS=X OUTPUT=raw_data")],
        "Y" => vec![("y", "TEST_RESONANCES AXIS=Y OUTPUT=raw_data")],
        "belts" => vec![
            ("a", "TEST_RESONANCES AXIS=1,1 OUTPUT=raw_data"),
            ("b", "TEST_RESONANCES AXIS=1,-1 OUTPUT=raw_data"),
        ],
        _ => return Err("resonance_capture axis must be X, Y or belts".into()),
    })
}

/// Klipper's adxl345 dump batches: `{<template fields>, "params":{"data":[[t,x,y,z],...]}}`.
fn collect_batch(value: &Value, out: &mut Vec<[f64; 4]>) {
    let Some(rows) = value["params"]["data"].as_array() else {
        return;
    };
    for row in rows {
        let Some(row) = row.as_array() else { continue };
        if row.len() < 4 {
            continue;
        }
        let mut quad = [0.0_f64; 4];
        let mut ok = true;
        for (i, slot) in quad.iter_mut().enumerate() {
            match row[i].as_f64() {
                Some(v) if v.is_finite() => *slot = v,
                _ => ok = false,
            }
        }
        if ok && out.len() < MAX_SAMPLES_PER_LEG {
            out.push(quad);
        }
    }
}

/// Subscribe to the accelerometer, run `script` on the printer, and keep reading batches until
/// the script returns. Returns the raw rows exactly as Klipper sent them.
///
/// `host`/`port` are passed rather than derived so the whole path is exercisable against a fake
/// bridge in tests; the live caller hands it `ws_source::host_port(cfg)`.
pub fn capture_leg(
    cfg: &Config,
    host: &str,
    port: u16,
    script: &str,
    sensor: &str,
) -> Result<Vec<[f64; 4]>> {
    let mut client = WsClient::connect(
        host,
        port,
        "/klippysocket",
        Duration::from_millis(cfg.request_timeout_ms.max(2000)),
    )?;
    client.send_text(
        &json!({
            "id": 1,
            "method": "adxl345/dump_adxl345",
            "params": {"sensor": sensor, "response_template": {"method": "stella_adxl"}}
        })
        .to_string(),
    )?;
    // Wait for the subscription to be accepted before moving anything.
    let subscribe_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = client.read_text_until(subscribe_deadline).map_err(|e| {
            format!("the accelerometer subscription was not confirmed: {e}; nothing was moved")
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if value["id"] == json!(1) {
            if let Some(message) = value["error"]["message"].as_str() {
                return Err(format!(
                    "Klipper refused the {sensor} dump subscription: {message}; nothing was moved"
                ));
            }
            if value["result"]["header"].is_array() {
                break;
            }
            return Err(format!(
                "Klipper answered the {sensor} dump subscription without a header; nothing was moved"
            ));
        }
    }

    // The sweep runs on its own thread so batches keep draining while it blocks; a full
    // TEST_RESONANCES is ~130 s and the socket would otherwise back up behind it.
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    let sweep_cfg = cfg.clone();
    let sweep_script = script.to_owned();
    let sweep = std::thread::spawn(move || {
        let outcome = Moonraker::new(&sweep_cfg)
            .post(
                "/printer/gcode/script",
                Some(&json!({"script": sweep_script})),
            )
            .map(|_| ());
        let _ = tx.send(outcome);
    });

    let mut samples: Vec<[f64; 4]> = Vec::new();
    let hard_stop = Instant::now() + Duration::from_millis(cfg.command_timeout_ms + 30_000);
    let mut sweep_result: Option<Result<()>> = None;
    let mut drain_until: Option<Instant> = None;
    loop {
        if sweep_result.is_none() {
            if let Ok(outcome) = rx.try_recv() {
                // Klipper batches on a 0.1 s timer; give the tail of the sweep time to arrive.
                drain_until = Some(Instant::now() + Duration::from_millis(600));
                sweep_result = Some(outcome);
            }
        }
        if let Some(until) = drain_until {
            if Instant::now() >= until {
                break;
            }
        }
        if Instant::now() >= hard_stop {
            sweep_result.get_or_insert(Err(
                "the resonance sweep did not report an outcome before the capture cap".into(),
            ));
            break;
        }
        match client.read_text_until(Instant::now() + Duration::from_millis(200)) {
            Ok(text) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    collect_batch(&value, &mut samples);
                }
            }
            Err(e) if e.contains("timed out") => {}
            Err(e) => {
                let _ = sweep.join();
                return Err(format!("the accelerometer stream stopped mid-sweep: {e}"));
            }
        }
    }
    let _ = sweep.join();
    match sweep_result {
        Some(Err(e)) => Err(format!("the resonance sweep did not complete: {e}")),
        _ if samples.is_empty() => Err(
            "the sweep completed but no accelerometer samples arrived; check the ADXL345 wiring"
                .into(),
        ),
        _ => Ok(samples),
    }
}

// ---------------------------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------------------------

fn db_path(database: &str) -> String {
    format!("{database}.resonance.sqlite3")
}

fn open(database: &str) -> Result<Connection> {
    let connection = Connection::open(db_path(database)).map_err(db_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(db_error)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS resonance_run (
               id INTEGER PRIMARY KEY,
               ts REAL NOT NULL,
               axis TEXT NOT NULL,
               status TEXT NOT NULL CHECK(status IN ('running','complete','failed')),
               finished_ts REAL,
               error TEXT,
               result TEXT
             );
             CREATE TABLE IF NOT EXISTS resonance_raw (
               run_id INTEGER NOT NULL,
               leg TEXT NOT NULL,
               seq INTEGER NOT NULL,
               samples INTEGER NOT NULL,
               data BLOB NOT NULL,
               PRIMARY KEY(run_id, leg, seq)
             );",
        )
        .map_err(db_error)?;
    Ok(connection)
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Raw samples are kept as f32 quads: 16 bytes a row, so a 130 s sweep is about 6.7 MB. f64
/// would double that for precision the ADXL345 does not have (13-bit ADC, ~4 mg per count).
fn encode_raw(rows: &[[f64; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows.len() * 16);
    for row in rows {
        for value in row {
            out.extend_from_slice(&(*value as f32).to_le_bytes());
        }
    }
    out
}

fn store_raw(connection: &Connection, run_id: i64, leg: &str, rows: &[[f64; 4]]) -> Result<()> {
    for (seq, chunk) in rows.chunks(RAW_CHUNK).enumerate() {
        connection
            .execute(
                "INSERT OR REPLACE INTO resonance_raw(run_id,leg,seq,samples,data) VALUES(?1,?2,?3,?4,?5)",
                params![run_id, leg, seq as i64, chunk.len() as i64, encode_raw(chunk)],
            )
            .map_err(db_error)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Run orchestration
// ---------------------------------------------------------------------------------------------

/// The printer's live square_corner_velocity, which the smoothing model needs. A missing answer
/// is reported as the Klipper default rather than silently assumed: the value lands in the JSON.
fn fetch_scv(cfg: &Config) -> (f64, bool) {
    match Moonraker::new(cfg).get("/printer/objects/query?toolhead=square_corner_velocity") {
        Ok(v) => match v["status"]["toolhead"]["square_corner_velocity"].as_f64() {
            Some(scv) if scv.is_finite() && scv > 0.0 => (scv, true),
            _ => (DEFAULT_SCV, false),
        },
        Err(_) => (DEFAULT_SCV, false),
    }
}

/// Build the stored result document from the finished legs.
fn result_json(axis: &str, legs: &[Leg], scv: f64, scv_measured: bool) -> Value {
    let grid = display_grid();
    // For X/Y there is one leg. For belts the shaper table is fitted on leg A, which is the
    // Shake&Tune convention: the recommendation belongs to the machine, not to one belt.
    let primary = &legs[0];
    let mut fit_psd = primary.psd.sum.clone();
    normalize_to_frequencies(&primary.psd.freq, &mut fit_psd);
    let fitted = find_best_shaper(&primary.psd.freq, &fit_psd, scv);
    let shapers: Vec<Value> = fitted
        .as_ref()
        .map(|(_, all)| {
            all.iter()
                .map(|s| {
                    json!({
                        "type": s.name,
                        "freq": (s.freq * 10.0).round() / 10.0,
                        "vibrations_pct": (s.vibrs * 1000.0).round() / 10.0,
                        "smoothing": (s.smoothing * 10000.0).round() / 10000.0,
                        "max_accel": (s.max_accel / 100.0).round() * 100.0,
                        "score": s.score
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let recommended = fitted
        .as_ref()
        .map(|(best, _)| {
            json!({
                "type": best.name,
                "freq": (best.freq * 10.0).round() / 10.0,
                "max_accel": (best.max_accel / 100.0).round() * 100.0
            })
        })
        .unwrap_or(Value::Null);
    let found = peaks(&primary.psd.freq, &primary.psd.sum, 6);
    let peak_hz = found
        .first()
        .map(|(hz, _)| (hz * 100.0).round() / 100.0)
        .map(Value::from)
        .unwrap_or(Value::Null);

    let belts = if legs.len() == 2 {
        json!({
            "a": legs[0].grid_sum,
            "b": legs[1].grid_sum,
            "similarity": (similarity(&legs[0].grid_sum, &legs[1].grid_sum) * 10000.0).round() / 10000.0
        })
    } else {
        Value::Null
    };

    json!({
        "axis": axis,
        "psd": {
            "freq": grid,
            "x": interp(&grid, &primary.psd.freq, &primary.psd.x),
            "y": interp(&grid, &primary.psd.freq, &primary.psd.y),
            "z": interp(&grid, &primary.psd.freq, &primary.psd.z),
            "sum": primary.grid_sum,
            "window": "kaiser6",
            "bin_hz": BIN_HZ,
            "native_bin_hz": primary.psd.fs / primary.psd.nfft as f64,
            "sample_rate_hz": (primary.psd.fs * 10.0).round() / 10.0,
            "nfft": primary.psd.nfft,
            "welch_windows": primary.psd.windows
        },
        "shapers": shapers,
        "recommended": recommended,
        "peak_hz": peak_hz,
        "peaks": found.iter().map(|(hz,p)| json!({"hz":(hz*100.0).round()/100.0,"power":p})).collect::<Vec<_>>(),
        "belts": belts,
        "legs": legs.iter().map(|l| json!({"name":l.name,"samples":l.psd.windows})).collect::<Vec<_>>(),
        "square_corner_velocity": scv,
        "square_corner_velocity_measured": scv_measured,
        "method": "Welch PSD (Kaiser beta=6, 50% overlap) and input-shaper fitting ported from Klipper shaper_calibrate.py; not read from Shake&Tune output"
    })
}

fn finish(database: &str, run_id: i64, outcome: Result<Value>) {
    let Ok(connection) = open(database) else {
        eprintln!("resonance run {run_id}: cannot reopen the store to record the outcome");
        return;
    };
    let write = match outcome {
        Ok(result) => connection.execute(
            "UPDATE resonance_run SET status='complete', finished_ts=?2, result=?3 WHERE id=?1",
            params![run_id, now(), result.to_string()],
        ),
        Err(error) => connection.execute(
            "UPDATE resonance_run SET status='failed', finished_ts=?2, error=?3 WHERE id=?1",
            params![run_id, now(), error],
        ),
    };
    if let Err(e) = write {
        eprintln!("resonance run {run_id}: outcome could not be saved: {e}");
    }
}

fn run_capture(cfg: &Config, run_id: i64, axis: &str) -> Result<Value> {
    let (host, port) = crate::ws_source::host_port(cfg)?;
    let (scv, scv_measured) = fetch_scv(cfg);
    let mut analysed = Vec::new();
    for (leg, script) in legs(axis)? {
        let rows = capture_leg(cfg, &host, port, script, "adxl345")?;
        let connection = open(&cfg.database)?;
        store_raw(&connection, run_id, leg, &rows)?;
        analysed.push(analyse(leg, &rows)?);
    }
    Ok(result_json(axis, &analysed, scv, scv_measured))
}

/// Create the run row and start the sweep on a background thread. Returns the run id at once:
/// a full TEST_RESONANCES is about 130 s per leg, and `belts` runs two of them, so the operator
/// polls `GET /api/resonance/<id>` rather than holding an HTTP request open for four minutes.
pub fn start(cfg: &Config, axis: &str) -> Result<i64> {
    // Refuse a bad axis before a run row exists, so a rejected request leaves no trace.
    legs(axis)?;
    if BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(
            "a resonance capture is already running; wait for it to finish before starting another"
                .into(),
        );
    }
    let guard = BusyGuard;
    let connection = open(&cfg.database)?;
    connection
        .execute(
            "INSERT INTO resonance_run(ts,axis,status) VALUES(?1,?2,'running')",
            params![now(), axis],
        )
        .map_err(db_error)?;
    let run_id = connection.last_insert_rowid();
    drop(connection);
    let database = cfg.database.clone();
    let owned = cfg.clone();
    let axis = axis.to_owned();
    let spawned = std::thread::Builder::new()
        .name(format!("resonance-{run_id}"))
        .spawn(move || {
            let _guard = guard; // released when the sweep ends, not when the HTTP request does
            let outcome = run_capture(&owned, run_id, &axis);
            finish(&owned.database, run_id, outcome);
        });
    if let Err(e) = spawned {
        // The closure was dropped with the failed spawn, so its BusyGuard has already cleared
        // the flag; the run row is marked failed so it never sits at 'running' forever.
        let message = format!("cannot start the resonance capture thread: {e}");
        finish(&database, run_id, Err(message.clone()));
        return Err(message);
    }
    Ok(run_id)
}

// ---------------------------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------------------------

fn summary_row(
    id: i64,
    ts: f64,
    axis: String,
    status: String,
    error: Option<String>,
    result: Option<String>,
) -> Value {
    let parsed: Option<Value> = result.and_then(|r| serde_json::from_str(&r).ok());
    json!({
        "id": id,
        "ts": ts,
        "axis": axis,
        "status": status,
        "error": error,
        "peak_hz": parsed.as_ref().map(|v| v["peak_hz"].clone()).unwrap_or(Value::Null),
        "recommended": parsed.as_ref().map(|v| v["recommended"].clone()).unwrap_or(Value::Null),
        // belt health over time (2026-10-26): how alike the two belts answer, from a belts run
        "belts_similarity": parsed.as_ref().map(|v| v["belts"]["similarity"].clone()).unwrap_or(Value::Null)
    })
}

pub fn list(cfg: &Config) -> Result<Value> {
    let connection = open(&cfg.database)?;
    let mut statement = connection
        .prepare(
            "SELECT id,ts,axis,status,error,result FROM resonance_run ORDER BY id DESC LIMIT 100",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok(summary_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error)?;
    Ok(json!({"runs": rows}))
}

pub fn detail(cfg: &Config, id: i64) -> Result<Option<Value>> {
    let connection = open(&cfg.database)?;
    let row = connection
        .query_row(
            "SELECT ts,axis,status,error,result FROM resonance_run WHERE id=?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((ts, axis, status, error, result)) = row else {
        return Ok(None);
    };
    let mut out = result
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .unwrap_or_else(|| {
            json!({"psd":Value::Null,"shapers":[],"belts":Value::Null,"peaks":[],"peak_hz":Value::Null,"recommended":Value::Null})
        });
    let map = out
        .as_object_mut()
        .ok_or("stored resonance result is not an object")?;
    map.insert("id".into(), json!(id));
    map.insert("ts".into(), json!(ts));
    map.insert("axis".into(), json!(axis));
    map.insert("status".into(), json!(status));
    map.insert("error".into(), json!(error));
    Ok(Some(out))
}

pub fn handle(method: &str, path: &str, cfg: &Config) -> Response {
    if method != "GET" {
        return Response::json(
            405,
            json!({"error":"resonance results are read-only; start a capture with POST /api/commands"}),
        );
    }
    let result = if path == "/api/resonance" {
        list(cfg).map(Some)
    } else {
        match path.trim_start_matches("/api/resonance/").parse::<i64>() {
            Ok(id) => detail(cfg, id),
            Err(_) => {
                return Response::json(
                    400,
                    json!({"error":"resonance id must be an integer from GET /api/resonance"}),
                )
            }
        }
    };
    match result {
        Ok(Some(value)) => Response::json(200, value),
        Ok(None) => Response::json(
            404,
            json!({"error":"no resonance run with that id; list them with GET /api/resonance"}),
        ),
        Err(e) => Response::json(500, json!({"error":e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::atomic::AtomicU64,
        thread,
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn temp(label: &str) -> String {
        format!(
            "/tmp/stella-resonance-{}-{}-{label}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn cleanup(database: &str) {
        for suffix in [
            ".resonance.sqlite3",
            ".resonance.sqlite3-wal",
            ".resonance.sqlite3-shm",
        ] {
            let _ = std::fs::remove_file(format!("{database}{suffix}"));
        }
    }

    /// A 45 Hz resonance with a weaker 78 Hz companion, on a light broadband floor. Sampled at
    /// the ADXL345's 3200 Hz for 6 s, which is 19200 rows — the shape a real leg produces.
    fn synthetic(peak_hz: f64, seconds: f64) -> Vec<[f64; 4]> {
        let fs = 3200.0_f64;
        let n = (fs * seconds) as usize;
        // Deterministic xorshift noise: a fixed seed keeps the assertion reproducible.
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut noise = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 11) as f64 / (1u64 << 53) as f64) - 0.5
        };
        (0..n)
            .map(|i| {
                let t = i as f64 / fs;
                let main = 900.0 * (2.0 * std::f64::consts::PI * peak_hz * t).sin();
                let second = 300.0 * (2.0 * std::f64::consts::PI * 78.0 * t).sin();
                [
                    t,
                    main + 40.0 * noise(),
                    second + 40.0 * noise(),
                    30.0 * noise(),
                ]
            })
            .collect()
    }

    #[test]
    fn fft_matches_a_direct_dft() {
        let n = 16;
        let signal: Vec<f64> = (0..n)
            .map(|i| (i as f64 * 0.7).sin() + 0.3 * (i as f64 * 2.1).cos())
            .collect();
        let mut re = signal.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        for k in 0..n {
            let (mut dr, mut di) = (0.0_f64, 0.0_f64);
            for (t, s) in signal.iter().enumerate() {
                let angle = -2.0 * std::f64::consts::PI * (k * t) as f64 / n as f64;
                dr += s * angle.cos();
                di += s * angle.sin();
            }
            assert!((re[k] - dr).abs() < 1e-9, "bin {k} real {} vs {dr}", re[k]);
            assert!((im[k] - di).abs() < 1e-9, "bin {k} imag {} vs {di}", im[k]);
        }
    }

    #[test]
    fn kaiser_window_matches_numpy() {
        // numpy.kaiser(8, 6.0), taken from numpy on this machine 2026-09-20.
        let expected = [
            0.014873337104763207,
            0.19978195130647008,
            0.591312182347899,
            0.9454284905890261,
            0.9454284905890261,
            0.591312182347899,
            0.19978195130647008,
            0.014873337104763207,
        ];
        let got = kaiser(8, 6.0);
        assert_eq!(got.len(), expected.len());
        for (a, b) in got.iter().zip(expected) {
            assert!((a - b).abs() < 1e-12, "kaiser {a} vs {b}");
        }
        // scipy.special.i0 for the same run.
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-15);
        assert!((bessel_i0(1.0) - 1.2660658777520082).abs() < 1e-14);
        assert!((bessel_i0(6.0) - 67.23440697647797).abs() < 1e-11);
    }

    #[test]
    fn psd_finds_the_planted_peak_and_the_grid_is_the_contract() {
        let data = synthetic(45.0, 6.0);
        let psd = calc_psd(&data).unwrap();
        // 3200 Hz: nfft = 1 << (3200*0.5 - 1).bit_length() = 2048, exactly Klipper's choice.
        assert_eq!(psd.nfft, 2048);
        assert!((psd.fs - 3200.0).abs() < 2.0, "derived rate {} Hz", psd.fs);
        let found = peaks(&psd.freq, &psd.sum, 6);
        assert!(!found.is_empty(), "no peak found in a planted spectrum");
        assert!(
            (found[0].0 - 45.0).abs() < 0.5,
            "peak came back at {:.3} Hz, not 45 Hz +/- 0.5",
            found[0].0
        );
        // The weaker 78 Hz companion is found too, and ranks below the 45 Hz one.
        assert!(
            found.iter().any(|(hz, _)| (hz - 78.0).abs() < 0.5),
            "78 Hz companion missing from {found:?}"
        );
        // Display grid: 0..200 Hz inclusive at 0.5 Hz, as BOARD-4 specifies.
        let grid = display_grid();
        assert_eq!(grid.len(), 401);
        assert_eq!(grid[0], 0.0);
        assert_eq!(grid[400], 200.0);
        assert!((grid[1] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn shaper_table_is_sensible_for_a_45_hz_machine() {
        let data = synthetic(45.0, 6.0);
        let psd = calc_psd(&data).unwrap();
        let mut fit = psd.sum.clone();
        normalize_to_frequencies(&psd.freq, &mut fit);
        let (best, all) = find_best_shaper(&psd.freq, &fit, DEFAULT_SCV).unwrap();
        // Every autotuned shaper is present, once, in Klipper's order.
        assert_eq!(
            all.iter().map(|s| s.name).collect::<Vec<_>>(),
            vec!["zv", "mzv", "ei", "2hump_ei", "3hump_ei"]
        );
        for s in &all {
            assert!(
                s.freq >= 20.0 && s.freq <= MAX_SHAPER_FREQ,
                "{} fitted at {} Hz",
                s.name,
                s.freq
            );
            assert!(
                (0.0..=1.0).contains(&s.vibrs),
                "{} vibrations {}",
                s.name,
                s.vibrs
            );
            assert!(
                s.smoothing > 0.0 && s.smoothing < 1.0,
                "{} smoothing {}",
                s.name,
                s.smoothing
            );
            assert!(
                s.max_accel > 200.0 && s.max_accel < 200_000.0,
                "{} max_accel {}",
                s.name,
                s.max_accel
            );
        }
        // A single sharp 45 Hz resonance must be cancelled near its own frequency, and the
        // recommendation has to leave almost no vibration behind.
        assert!(
            (best.freq - 45.0).abs() < 12.0,
            "recommended {} at {:.1} Hz for a 45 Hz peak",
            best.name,
            best.freq
        );
        assert!(
            best.vibrs < 0.05,
            "recommended {} leaves {:.1}% vibration",
            best.name,
            best.vibrs * 100.0
        );
        assert!(SHAPERS.iter().any(|(n, _)| *n == best.name));
    }

    #[test]
    fn shaper_impulses_match_klipper_by_hand() {
        // zv at 50 Hz, damping 0.1: df = sqrt(0.99), K = exp(-0.1*pi/df), t_d = 1/(50*df).
        let df = (1.0_f64 - 0.01).sqrt();
        let k = (-0.1 * std::f64::consts::PI / df).exp();
        let t_d = 1.0 / (50.0 * df);
        let (a, t) = shaper_impulses("zv", 50.0, 0.1);
        assert!((a[0] - 1.0).abs() < 1e-12 && (a[1] - k).abs() < 1e-12);
        assert!((t[0]).abs() < 1e-12 && (t[1] - 0.5 * t_d).abs() < 1e-12);
        // Every shaper: amplitudes sum to something positive, times ascend from zero.
        for (name, _) in SHAPERS {
            let (a, t) = shaper_impulses(name, 50.0, DEFAULT_DAMPING_RATIO);
            assert_eq!(a.len(), t.len(), "{name}");
            assert!(a.iter().sum::<f64>() > 0.0, "{name}");
            assert!(t[0].abs() < 1e-12, "{name} does not start at t=0");
            assert!(
                t.windows(2).all(|w| w[1] > w[0]),
                "{name} times not ascending"
            );
        }
        // The 3hump table's short first row is upstream's; k must follow it to 3.
        let (a, _) = shaper_impulses("3hump_ei", 50.0, 0.1);
        assert_eq!(a.len(), 5);
    }

    #[test]
    fn belts_similarity_is_one_for_identical_and_falls_apart_for_shifted() {
        let a = calc_psd(&synthetic(45.0, 6.0)).unwrap();
        let grid = display_grid();
        let same = interp(&grid, &a.freq, &a.sum);
        assert!(
            (similarity(&same, &same) - 1.0).abs() < 1e-9,
            "identical PSDs must correlate at 1.0"
        );
        let b = calc_psd(&synthetic(70.0, 6.0)).unwrap();
        let shifted = interp(&grid, &b.freq, &b.sum);
        let score = similarity(&same, &shifted);
        assert!(
            (0.0..0.6).contains(&score),
            "a 45 Hz belt against a 70 Hz belt scored {score}"
        );
    }

    #[test]
    fn interp_is_linear_and_clamps_at_both_ends() {
        let xs = [0.0, 1.0, 2.0];
        let ys = [0.0, 10.0, 20.0];
        let got = interp(&[-1.0, 0.0, 0.5, 1.5, 2.0, 3.0], &xs, &ys);
        assert_eq!(got, vec![0.0, 0.0, 5.0, 15.0, 20.0, 20.0]);
    }

    #[test]
    fn axis_legs_are_the_shake_and_tune_pair() {
        assert_eq!(legs("X").unwrap().len(), 1);
        assert_eq!(
            legs("Y").unwrap()[0].1,
            "TEST_RESONANCES AXIS=Y OUTPUT=raw_data"
        );
        let belts = legs("belts").unwrap();
        assert_eq!(belts.len(), 2);
        assert!(belts[0].1.contains("AXIS=1,1"));
        assert!(belts[1].1.contains("AXIS=1,-1"));
        assert!(legs("Z").is_err());
        assert!(legs("x").is_err());
    }

    #[test]
    fn calc_psd_refuses_data_it_cannot_honestly_analyse() {
        assert!(calc_psd(&[]).unwrap_err().contains("no usable data"));
        let flat: Vec<[f64; 4]> = (0..4000).map(|_| [1.0, 0.0, 0.0, 0.0]).collect();
        assert!(calc_psd(&flat).unwrap_err().contains("do not advance"));
        // Enough span, far too few samples for a 2048-point window.
        let sparse: Vec<[f64; 4]> = (0..500)
            .map(|i| [i as f64 / 3200.0, 0.0, 0.0, 0.0])
            .collect();
        assert!(calc_psd(&sparse).unwrap_err().contains("longer sweep"));
    }

    // -------------------------------------------------------------------------------------
    // Fake Klippy bridge + fake Moonraker: the capture path end to end with nothing moving.
    // -------------------------------------------------------------------------------------

    fn ws_read_client_frame(stream: &mut TcpStream) -> String {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).unwrap();
        let masked = head[1] & 0x80 != 0;
        let len7 = (head[1] & 0x7F) as usize;
        let len = match len7 {
            126 => {
                let mut b = [0u8; 2];
                stream.read_exact(&mut b).unwrap();
                u16::from_be_bytes(b) as usize
            }
            127 => {
                let mut b = [0u8; 8];
                stream.read_exact(&mut b).unwrap();
                u64::from_be_bytes(b) as usize
            }
            n => n,
        };
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask).unwrap();
        }
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).unwrap();
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        String::from_utf8(payload).unwrap()
    }

    fn ws_write_server_frame(stream: &mut TcpStream, text: &str) {
        let payload = text.as_bytes();
        let mut frame = vec![0x81u8];
        if payload.len() < 126 {
            frame.push(payload.len() as u8);
        } else if payload.len() <= 0xFFFF {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        stream.write_all(&frame).unwrap();
        stream.flush().unwrap();
    }

    /// A stand-in for Moonraker's `/klippysocket`: completes the upgrade, answers the
    /// `adxl345/dump_adxl345` subscription with a header, then pushes `batches` of samples.
    fn fake_bridge(
        peak_hz: f64,
        seconds: f64,
        reject: Option<&'static str>,
    ) -> (String, u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
                )
                .unwrap();
            let request = ws_read_client_frame(&mut stream);
            if let Some(message) = reject {
                ws_write_server_frame(
                    &mut stream,
                    &json!({"id":1,"error":{"message":message}}).to_string(),
                );
                thread::sleep(Duration::from_millis(200));
                return request;
            }
            ws_write_server_frame(
                &mut stream,
                &json!({"id":1,"result":{"header":["time","x_acceleration","y_acceleration","z_acceleration"]}}).to_string(),
            );
            // Klipper pushes batches on a 0.1 s timer; send the whole planted signal in
            // batch-sized pieces, with the template method the client asked for.
            let data = synthetic(peak_hz, seconds);
            for chunk in data.chunks(1600) {
                let rows: Vec<Value> = chunk
                    .iter()
                    .map(|r| json!([r[0], r[1], r[2], r[3]]))
                    .collect();
                ws_write_server_frame(
                    &mut stream,
                    &json!({"method":"stella_adxl","params":{"data":rows,"errors":0,"overflows":0}})
                        .to_string(),
                );
            }
            // Stay open long enough for the fake sweep to return and the drain to run.
            thread::sleep(Duration::from_millis(1500));
            request
        });
        ("127.0.0.1".to_string(), address.port(), worker)
    }

    /// A stand-in for Moonraker's HTTP side: answers every POST with `{"result":"ok"}` after a
    /// short delay, standing in for the sweep's run time, and records what was asked.
    fn fake_moonraker(delay_ms: u64, replies: usize) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 2048];
                let header_end = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_owned())
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                seen.push(String::from_utf8_lossy(&bytes).into_owned());
                thread::sleep(Duration::from_millis(delay_ms));
                let body = r#"{"result":"ok"}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
            seen
        });
        (format!("http://{address}"), worker)
    }

    #[test]
    fn capture_leg_subscribes_runs_the_sweep_and_returns_real_samples() {
        let (host, port, bridge) = fake_bridge(45.0, 2.0, None);
        let (url, http) = fake_moonraker(300, 1);
        let cfg = Config {
            moonraker_url: url,
            request_timeout_ms: 2000,
            command_timeout_ms: 10_000,
            ..Config::default()
        };
        let rows = capture_leg(
            &cfg,
            &host,
            port,
            "TEST_RESONANCES AXIS=X OUTPUT=raw_data",
            "adxl345",
        )
        .unwrap();
        assert!(rows.len() > 6000, "only {} samples captured", rows.len());
        // The subscription is Klipper's mux endpoint, with our response template.
        let request: Value = serde_json::from_str(&bridge.join().unwrap()).unwrap();
        assert_eq!(request["method"], "adxl345/dump_adxl345");
        assert_eq!(request["params"]["sensor"], "adxl345");
        assert_eq!(
            request["params"]["response_template"]["method"],
            "stella_adxl"
        );
        // The sweep went out as an ordinary gcode script POST, exactly once.
        let posts = http.join().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].starts_with("POST /printer/gcode/script "));
        assert!(posts[0].ends_with(r#"{"script":"TEST_RESONANCES AXIS=X OUTPUT=raw_data"}"#));
        // And the captured stream analyses back to the planted 45 Hz peak.
        let psd = calc_psd(&rows).unwrap();
        let found = peaks(&psd.freq, &psd.sum, 3);
        assert!(
            (found[0].0 - 45.0).abs() < 0.5,
            "captured peak {:?}",
            found[0]
        );
    }

    #[test]
    fn a_refused_subscription_never_moves_the_printer() {
        let (host, port, bridge) = fake_bridge(45.0, 1.0, Some("Unknown sensor 'adxl345'"));
        // One reply slot, which must go unused: nothing may be POSTed.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let cfg = Config {
            moonraker_url: format!("http://{}", listener.local_addr().unwrap()),
            request_timeout_ms: 2000,
            ..Config::default()
        };
        let error =
            capture_leg(&cfg, &host, port, "TEST_RESONANCES AXIS=X", "adxl345").unwrap_err();
        assert!(error.contains("refused"), "{error}");
        assert!(error.contains("nothing was moved"), "{error}");
        assert!(listener.accept().is_err(), "a sweep was sent anyway");
        bridge.join().unwrap();
    }

    #[test]
    fn a_run_stores_raw_samples_and_serves_both_endpoints() {
        let database = temp("run");
        let connection = open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO resonance_run(ts,axis,status) VALUES(?1,'X','running')",
                params![now()],
            )
            .unwrap();
        let run_id = connection.last_insert_rowid();
        let rows = synthetic(45.0, 2.0);
        store_raw(&connection, run_id, "x", &rows).unwrap();
        let stored: i64 = connection
            .query_row(
                "SELECT SUM(samples) FROM resonance_raw WHERE run_id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored as usize, rows.len(), "every raw sample is kept");
        drop(connection);

        let cfg = Config {
            database: database.clone(),
            ..Config::default()
        };
        // A run still in flight is visible and honestly labelled.
        let running = detail(&cfg, run_id).unwrap().unwrap();
        assert_eq!(running["status"], "running");
        assert_eq!(running["peak_hz"], Value::Null);

        // Finish it with a real analysis of the stored signal.
        let leg = analyse("x", &rows).unwrap();
        finish(&database, run_id, Ok(result_json("X", &[leg], 5.0, true)));

        let listed = list(&cfg).unwrap();
        let first = &listed["runs"][0];
        assert_eq!(first["id"], run_id);
        assert_eq!(first["axis"], "X");
        assert_eq!(first["status"], "complete");
        assert!((first["peak_hz"].as_f64().unwrap() - 45.0).abs() < 0.5);
        assert!(first["recommended"]["type"].is_string());
        assert!(first["recommended"]["max_accel"].as_f64().unwrap() > 0.0);

        let full = detail(&cfg, run_id).unwrap().unwrap();
        assert_eq!(full["psd"]["freq"].as_array().unwrap().len(), 401);
        assert_eq!(full["psd"]["sum"].as_array().unwrap().len(), 401);
        assert_eq!(full["psd"]["x"].as_array().unwrap().len(), 401);
        assert_eq!(full["psd"]["window"], "kaiser6");
        assert_eq!(full["shapers"].as_array().unwrap().len(), 5);
        assert!(full["belts"].is_null(), "a single axis has no belt pair");
        assert!(!full["peaks"].as_array().unwrap().is_empty());

        assert!(detail(&cfg, run_id + 999).unwrap().is_none());
        assert_eq!(handle("POST", "/api/resonance", &cfg).code, 405);
        assert_eq!(handle("GET", "/api/resonance/not-a-number", &cfg).code, 400);
        assert_eq!(
            handle("GET", &format!("/api/resonance/{run_id}"), &cfg).code,
            200
        );
        assert_eq!(handle("GET", "/api/resonance/424242", &cfg).code, 404);
        cleanup(&database);
    }

    #[test]
    fn a_belts_run_carries_both_legs_and_a_similarity() {
        let a = analyse("a", &synthetic(45.0, 2.0)).unwrap();
        let b = analyse("b", &synthetic(45.0, 2.0)).unwrap();
        let value = result_json("belts", &[a, b], 5.0, true);
        assert_eq!(value["belts"]["a"].as_array().unwrap().len(), 401);
        assert_eq!(value["belts"]["b"].as_array().unwrap().len(), 401);
        let score = value["belts"]["similarity"].as_f64().unwrap();
        assert!(score > 0.9, "two matched belts scored {score}");

        let mismatched = result_json(
            "belts",
            &[
                analyse("a", &synthetic(45.0, 2.0)).unwrap(),
                analyse("b", &synthetic(72.0, 2.0)).unwrap(),
            ],
            5.0,
            true,
        );
        let bad = mismatched["belts"]["similarity"].as_f64().unwrap();
        assert!(bad < 0.6, "belts resonating 27 Hz apart scored {bad}");
    }
}
