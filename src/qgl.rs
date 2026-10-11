use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Round {
    pub ts: f64,
    pub run_id: i64,
    pub round: i64,
    pub range_mm: f64,
    pub z0: Option<f64>,
    pub z1: Option<f64>,
    pub z2: Option<f64>,
    pub z3: Option<f64>,
}

#[derive(Clone, Default)]
pub struct Parser {
    pub run_id: i64,
    corrections: [Option<f64>; 4],
}

fn after_number(line: &str, marker: &str) -> Option<f64> {
    let s = line.split_once(marker)?.1.split_whitespace().next()?;
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

impl Parser {
    pub fn new(last_run: i64) -> Self {
        Self {
            run_id: last_run,
            ..Self::default()
        }
    }
    pub fn feed(&mut self, ts: f64, text: &str) -> Vec<Round> {
        let mut out = Vec::new();
        for raw in text.lines() {
            let line = raw.trim().trim_start_matches("//").trim();
            if line.contains("Gantry-relative probe points:")
                || line.contains("Making the following Z adjustments:")
            {
                self.corrections = [None; 4];
            }
            for (i, name) in ["stepper_z", "stepper_z1", "stepper_z2", "stepper_z3"]
                .iter()
                .enumerate()
            {
                if let Some(rest) = line.strip_prefix(name) {
                    if let Some(value) = rest.trim_start().strip_prefix('=') {
                        self.corrections[i] = value
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<f64>().ok())
                            .filter(|v| v.is_finite());
                    }
                }
            }
            if let Some((_, rest)) = line.split_once("Retries:") {
                let round = rest
                    .trim()
                    .split('/')
                    .next()
                    .and_then(|n| n.trim().parse::<i64>().ok());
                let range = after_number(line, "Probed points range:");
                if let (Some(round), Some(range_mm)) = (round, range) {
                    if round < 0 || range_mm < 0.0 {
                        continue;
                    }
                    if round == 0 || self.run_id == 0 {
                        self.run_id += 1;
                    }
                    out.push(Round {
                        ts,
                        run_id: self.run_id,
                        round,
                        range_mm,
                        z0: self.corrections[0],
                        z1: self.corrections[1],
                        z2: self.corrections[2],
                        z3: self.corrections[3],
                    });
                    // Missing console fragments once reused the previous round's adjustments.
                    self.corrections = [None; 4];
                }
            }
            if line.contains("Retries aborting:") {
                self.corrections = [None; 4];
            }
        }
        out
    }
}
