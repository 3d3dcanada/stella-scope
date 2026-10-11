// BLTouch evidence parser: turns the console lines Klipper prints while probing into recorded
// probe points and Z-offset change events. Every format below was taken from a real
// printer\'s recording on 2026-09-17 (GET /server/gcode_store), not from documentation.

#[derive(Clone, Debug, PartialEq)]
pub struct ProbePoint {
    pub ts: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub context: String,
}

fn number(token: &str) -> Option<f64> {
    token
        .trim_end_matches(',')
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

// One probe point per Klipper line: "probe: at X,Y bed will contact at z=Z".
fn probe_at(message: &str) -> Option<(f64, f64, f64)> {
    let rest = message.split("probe: at ").nth(1)?;
    let mut parts = rest.split_whitespace();
    // "27.050,24.282" is a single comma-joined token in Klipper's console line.
    let (xs, ys) = parts.next()?.split_once(',')?;
    let x = number(xs)?;
    let y = number(ys)?;
    let z = parts.find(|t| t.starts_with("z="))?;
    let z = number(z.trim_start_matches("z="))?;
    Some((x, y, z))
}

/// Latest probe point embedded in a console line, if any.
pub fn point_from(message: &str) -> Option<(f64, f64, f64)> {
    if !message.contains("probe: at ") {
        return None;
    }
    probe_at(message)
}

/// Z-offset change evidence: returns a short description when the line changes the effective
/// Z offset. Recorded as its own event kind so the log is one table on the Leveling page.
pub fn offset_change(message: &str) -> Option<&str> {
    let m = message.trim_start_matches("// ").trim();
    let lower = m.to_lowercase();
    if lower.starts_with("set_gcode_offset")
        || lower.starts_with("z_offset_apply_probe")
        || lower.starts_with("save_config")
    {
        return Some(m);
    }
    if lower.starts_with("bltouch") && lower.contains("z_offset") {
        return Some(m);
    }
    None
}

/// What procedure a probe point belongs to, derived from the console evidence seen so far.
pub fn context_for(
    recent_commands: &[String],
    qgl_round: Option<u64>,
    printing: Option<&str>,
) -> String {
    let mut ctx = "";
    for c in recent_commands.iter().rev() {
        let l = c.to_lowercase();
        if l.contains("quad_gantry_level") {
            ctx = "gantry levelling";
            break;
        }
        if l.contains("bed_mesh") {
            ctx = "bed mesh";
            break;
        }
        if l.contains("g28") {
            ctx = "homing";
            break;
        }
        if l.contains("probe_accuracy") || l.contains("\nprobe\n") {
            ctx = "probe accuracy";
            break;
        }
    }
    let step = match (ctx, qgl_round) {
        ("gantry levelling", Some(r)) => format!("gantry levelling round {}", r + 1),
        (c, _) if c.is_empty() => printing
            .map(|f| format!("print {}", short_name(f)))
            .unwrap_or_else(|| "unclassified probing".into()),
        (c, _) => c.to_string(),
    };
    step
}

fn short_name(file: &str) -> String {
    file.rsplit('/')
        .next()
        .unwrap_or(file)
        .chars()
        .take(24)
        .collect()
}

// Latest known homing-style zero reference: the test printer homes Z with the BLTouch
// at nozzle X207.95 Y185.72, i.e. the probe reads bed centre 175,175.
pub fn is_zero_reference(x: f64, y: f64, context: &str) -> bool {
    context == "homing" && (x - 175.0).abs() <= 1.0 && (y - 175.0).abs() <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_stella_lines() {
        let (x, y, z) =
            point_from("// probe: at 27.050,24.282 bed will contact at z=-1.951000").unwrap();
        assert_eq!((x, y, z), (27.050, 24.282, -1.951));
        let (_, _, z2) =
            point_from("// probe: at 297.050,319.282 bed will contact at z=0.447750").unwrap();
        assert!((z2 - 0.44775).abs() < 1e-9);
        assert!(point_from("// Probe samples exceed tolerance. Retrying...").is_none());
        assert!(point_from("// Gantry-relative probe points:").is_none());
    }
    #[test]
    fn offset_changes_from_real_commands() {
        assert!(offset_change("SET_GCODE_OFFSET Z=0.01").is_some());
        assert!(offset_change("Z_OFFSET_APPLY_PROBE").is_some());
        assert!(offset_change("SAVE_CONFIG").is_some());
        assert!(offset_change("bltouch: z_offset: 1.973").is_some());
        assert!(offset_change("M117 Hello").is_none());
        assert!(offset_change("// probe: at 1,2 bed will contact at z=3").is_none());
    }
    #[test]
    fn context_and_zero_reference() {
        assert_eq!(context_for(&["G28".into()], None, None), "homing");
        assert_eq!(
            context_for(&["G28".into(), "QUAD_GANTRY_LEVEL".into()], Some(1), None),
            "gantry levelling round 2"
        );
        assert_eq!(
            context_for(&[], None, Some("dir/flowrate_0.2mm_PLA.gcode")),
            "print flowrate_0.2mm_PLA.gcode"
        );
        assert!(is_zero_reference(175.02, 174.98, "homing"));
        assert!(!is_zero_reference(175.0, 175.0, "bed mesh"));
    }
}
