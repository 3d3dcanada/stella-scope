//! Stella-eye: print failure detection for Stella Scope. The model is Obico's open failure detector (a one-class YOLOv2, 416 x 416,
//! exported to ONNX with the box decoding inside; weights from Obico's public model bucket), run by tract, a pure-Rust ONNX engine,
//! so the same program builds for Linux, Windows and Mac with nothing to install. Its scores match ONNX Runtime's on the test
//! pictures (0.666, 0.607, 0.295); ~0.9 s a frame on one core. The scoring below is Obico's own (obico-server
//! backend/lib/prediction.py and failure_detection.py, 1st-gen hyperparameters, tuned for one frame every 10 s): each frame's score
//! is the sum of detection confidences; an exponential mean (span 12) is compared with a long rolling baseline, so a camera that
//! always shows a little "noise" does not raise alarms, and a sudden rise does. It runs on the computer (the Pi on the test printer
//! has 1 GB and would take 15 s a frame), reads Moonraker read-only, and writes its verdict to a status file that Stella Scope
//! shows and alerts on. It never pauses the printer itself. `stella-eye --model FILE --moonraker URL --snapshot URL --status FILE
//! [--state FILE] [--interval 10]` `stella-eye --model FILE --image FILE` scores one picture and prints the detections.
use anyhow::{bail, Context, Result};
use tract_onnx::prelude::*;
type Session = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SIZE: u32 = 416;
const THRESH: f32 = 0.08; // ml_api/server.py: a box counts from this confidence
const NMS: f32 = 0.45;
// FD_1ST_GEN_PARAMS, obico-server backend/config/settings.py
const EWM_SPAN: f64 = 12.0;
const ROLLING_WIN_SHORT: f64 = 310.0;
const ROLLING_WIN_LONG: f64 = 7200.0;
const THRESHOLD_LOW: f64 = 0.38;
const THRESHOLD_HIGH: f64 = 0.78;
const INIT_SAFE_FRAME_NUM: u64 = 30;
const ROLLING_MEAN_SHORT_MULTIPLE: f64 = 3.8;
const ESCALATING_FACTOR: f64 = 1.75;

struct Det {
    conf: f32,
    b: [f32; 4], // x1 y1 x2 y2, 0..1 of the frame
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}
fn new_run(was_printing: bool, old_file: &str, file: &str, old_duration: Option<f64>, duration: Option<f64>) -> bool {
    !was_printing || old_file != file || old_duration.zip(duration).is_some_and(|(a,b)| b + 2.0 < a)
}
fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn detect(session: &Session, jpeg: &[u8]) -> Result<Vec<Det>> {
    let img = image::load_from_memory(jpeg).context("the camera frame is not an image")?.to_rgb8();
    // cv2.resize INTER_LINEAR to 416 x 416 (no letterbox), RGB, CHW, 0..1: as ml_api/lib/onnx.py.
    let small = image::imageops::resize(&img, SIZE, SIZE, image::imageops::FilterType::Triangle);
    let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, SIZE as usize, SIZE as usize), |(_, c, y, x)| small[(x as u32, y as u32)][c] as f32 / 255.0).into();
    let out = session.run(tvec!(input.into()))?;
    let boxes = out[0].to_array_view::<f32>()?; // [1, N, 1, 4]
    let confs = out[1].to_array_view::<f32>()?; // [1, N, classes]
    let n = confs.shape()[1];
    let mut dets: Vec<Det> = (0..n)
        .filter_map(|i| {
            let conf = confs[[0, i, 0]];
            (conf > THRESH).then(|| Det { conf, b: [boxes[[0, i, 0, 0]], boxes[[0, i, 0, 1]], boxes[[0, i, 0, 2]], boxes[[0, i, 0, 3]]] })
        })
        .collect();
    dets.sort_by(|a, b| b.conf.total_cmp(&a.conf));
    let mut kept: Vec<Det> = Vec::new();
    for d in dets {
        if kept.iter().all(|k| iou(&k.b, &d.b) <= NMS) {
            kept.push(d);
        }
    }
    Ok(kept)
}
fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let area = |r: &[f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    inter / (area(a) + area(b) - inter).max(1e-9)
}

/// PrinterPrediction: the running means. `current` counts frames in this print, `lifetime` all.
#[derive(Default)]
struct Pred {
    p: f64,
    ewm: f64,
    short: f64,
    long: f64,
    current: u64,
    lifetime: u64,
}
impl Pred {
    fn update(&mut self, p: f64) {
        let alpha = 2.0 / (EWM_SPAN + 1.0);
        self.p = p;
        self.current += 1;
        self.lifetime += 1;
        self.ewm = p * alpha + self.ewm * (1.0 - alpha);
        let roll = |mean: f64, count: u64, win: f64| mean + (p - mean) / if win <= count as f64 { win } else { count as f64 + 1.0 };
        self.short = roll(self.short, self.current, ROLLING_WIN_SHORT);
        self.long = roll(self.long, self.lifetime, ROLLING_WIN_LONG);
    }
    fn failing(&self, sensitivity: f64, escalating: f64) -> bool {
        if self.current < INIT_SAFE_FRAME_NUM {
            return false;
        }
        let adj = (self.ewm - self.long) * sensitivity / escalating;
        if adj < THRESHOLD_LOW {
            return false;
        }
        adj > THRESHOLD_HIGH || adj > (self.short - self.long) * ROLLING_MEAN_SHORT_MULTIPLE
    }
    /// 0..1 for the gauge: under 1/3 fine, 1/3..2/3 warning, over 2/3 failing (calc_normalized_p).
    fn normalized(&self, sensitivity: f64) -> f64 {
        let scale = |v: f64, lo: f64, hi: f64, nlo: f64, nhi: f64| ((v - lo) * (nhi - nlo) / (hi - lo) + nlo).clamp(nlo, nhi);
        let warn = ((self.short - self.long) * ROLLING_MEAN_SHORT_MULTIPLE).clamp(THRESHOLD_LOW, THRESHOLD_HIGH);
        let fail = warn * ESCALATING_FACTOR;
        let p = (self.ewm - self.long) * sensitivity;
        if p > fail {
            scale(p, fail, fail * 1.5, 2.0 / 3.0, 1.0)
        } else if p > warn {
            scale(p, warn, fail, 1.0 / 3.0, 2.0 / 3.0)
        } else {
            scale(p, 0.0, warn, 0.0, 1.0 / 3.0)
        }
    }
}

fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut r = agent.get(url).call()?;
    Ok(r.body_mut().with_config().limit(20 << 20).read_to_vec()?)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model = arg(&args, "--model").context("--model FILE")?;
    let load = || -> Result<Session> {
        Ok(tract_onnx::onnx()
            .model_for_path(&model)
            .with_context(|| format!("cannot load {model}"))?
            .with_input_fact(0, f32::fact([1, 3, SIZE as usize, SIZE as usize]).into())?
            .into_optimized()?
            .into_runnable()?)
    };
    if let Some(img) = arg(&args, "--image") {
        let session = load()?;
        let t = Instant::now();
        let dets = detect(&session, &std::fs::read(&img)?)?;
        let p: f32 = dets.iter().map(|d| d.conf).sum();
        println!("{} detections, p={p:.3}, {} ms", dets.len(), t.elapsed().as_millis());
        for d in dets {
            println!("  {:.3} at [{:.3} {:.3} {:.3} {:.3}]", d.conf, d.b[0], d.b[1], d.b[2], d.b[3]);
        }
        return Ok(());
    }
    let moonraker = arg(&args, "--moonraker").context("--moonraker URL")?;
    let snapshot = arg(&args, "--snapshot").context("--snapshot URL")?;
    let status = PathBuf::from(arg(&args, "--status").context("--status FILE")?);
    let state_file = arg(&args, "--state").map(PathBuf::from);
    let interval = Duration::from_secs(arg(&args, "--interval").and_then(|v| v.parse().ok()).unwrap_or(10));
    let sensitivity: f64 = arg(&args, "--sensitivity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if !(0.5..=2.0).contains(&sensitivity) {
        bail!("--sensitivity is 0.5 (fewer alarms) to 2.0 (more)");
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    // The long baseline survives restarts, as Obico keeps lifetime_frame_num per printer.
    let mut pred = Pred::default();
    if let Some(v) = state_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        pred.long = v["long"].as_f64().unwrap_or(0.0);
        pred.lifetime = v["lifetime"].as_u64().unwrap_or(0);
    }
    let mut job = String::new();
    let mut was_printing = false;
    let mut last_duration = None;
    let mut run = 0u64;
    // The model holds about 600 MB, so it is loaded when a print starts and let go five minutes
    // after the printer stops printing (RAM on this computer is shared).
    let mut session: Option<Session> = None;
    let mut idle_since = Instant::now();
    // Leave with Stella Scope: it holds our stdin open, and when it is gone the pipe closes.
    if std::env::var_os("STELLA_WATCH_STDIN").is_some() {
        thread::spawn(|| {
            use std::io::Read;
            let mut b = [0u8; 64];
            while matches!(std::io::stdin().read(&mut b), Ok(n) if n > 0) {}
            std::process::exit(0);
        });
    }
    loop {
        let t = Instant::now();
        let ps = get(&agent, &format!("{moonraker}/printer/objects/query?print_stats=state,filename,total_duration"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| v["result"]["status"]["print_stats"].clone())
            .unwrap_or(Value::Null);
        let state = ps["state"].as_str().unwrap_or("unknown").to_owned();
        let file = ps["filename"].as_str().unwrap_or("").to_owned();
        let duration = ps["total_duration"].as_f64();
        let mut out = json!({"at":now(),"printer":state,"watching":false,"model":"obico-failure-5a6b1be1fa"});
        if state != "printing" && session.is_some() && idle_since.elapsed() > Duration::from_secs(300) {
            session = None;
        }
        if state == "printing" {
            idle_since = Instant::now();
            if session.is_none() {
                session = Some(load()?);
            }
            if new_run(was_printing, &job, &file, last_duration, duration) {
                job = file.clone();
                run += 1;
                pred.p = 0.0;
                pred.current = 0;
                pred.short = 0.0;
                pred.ewm = 0.0;
            }
            was_printing = true;
            last_duration = duration;
            match get(&agent, &snapshot).and_then(|jpeg| detect(session.as_ref().expect("loaded above"), &jpeg)) {
                Ok(dets) => {
                    let p: f64 = dets.iter().map(|d| d.conf as f64).sum();
                    pred.update(p);
                    let pause = pred.failing(sensitivity, ESCALATING_FACTOR);
                    let warn = !pause && pred.failing(sensitivity, 1.0);
                    out = json!({"at":now(),"printer":state,"watching":true,"job":job,"run":run,"frame":pred.current,
                        "p":p,"ewm":pred.ewm,"normalized":pred.normalized(sensitivity),"verdict":if pause{"failing"}else if warn{"warning"}else{"ok"},
                        "learning":pred.current < INIT_SAFE_FRAME_NUM,"ms":t.elapsed().as_millis() as u64,
                        "boxes":dets.iter().filter(|d| d.conf > 0.2).map(|d| json!({"conf":d.conf,"box":d.b})).collect::<Vec<_>>(),
                        "model":"obico-failure-5a6b1be1fa"});
                    if let Some(f) = &state_file {
                        let _ = std::fs::write(f, json!({"long":pred.long,"lifetime":pred.lifetime}).to_string());
                    }
                }
                Err(e) => out["error"] = json!(e.to_string()),
            }
        } else if matches!(state.as_str(), "standby" | "complete" | "cancelled" | "error") { was_printing = false; last_duration = None; }
        let _ = std::fs::write(&status, out.to_string());
        thread::sleep(interval.saturating_sub(t.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_filename_reprint_starts_fresh_detection() {
        assert!(super::new_run(false, "cube.gcode", "cube.gcode", Some(120.0), Some(1.0)));
        assert!(super::new_run(true, "cube.gcode", "cube.gcode", Some(120.0), Some(1.0)));
        assert!(!super::new_run(true, "cube.gcode", "cube.gcode", Some(120.0), Some(121.0)));
    }
}
