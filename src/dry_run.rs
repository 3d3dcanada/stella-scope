//! Dry-run transform (finish board WO-02): a print file turned into a no-extrusion move file so a
//! printer can run a job for real, every move, without printing anything.
//!
//! Pure over bytes: no printer, no network, no crate.
//!
//! - Every G0/G1/G2/G3 move: the E word removed, so nothing extrudes. Absolute (M82) and relative
//!   (M83) extrusion are handled the same way — there is no E left to act on; retracts are E moves
//!   too, so they disappear with the rest.
//! - A line that is only extrusion (no X/Y/Z) is dropped.
//! - Z is lifted by `lift_mm` (default 30 mm) on every Z target, so the head runs above the plate.
//!   `max_z` optionally refuses the transform when the lifted Z would exceed the printer's Z travel.
//! - Heater commands (M104/M109/M140/M190) are removed unless `heat` is true; fans, homing (G28),
//!   levelling (G29), the Prusa printer-check lines (M862.x) and everything else are kept unchanged.
//! - A Klipper start macro (PRINT_START / START_PRINT) heats and purges inside the printer, where the
//!   file cannot reach it, so it is replaced by G28 (plus its own BED/EXTRUDER temperatures when heat
//!   is on). An end macro (PRINT_END / END_PRINT) is dropped: it usually retracts.
//! - Only absolute Z targets are lifted; a relative Z move (G91, a Z hop) keeps its own length.

/// A move line's words, scanned without allocating.
struct Words {
    has_xyz: bool,
    has_e: bool,
    /// Read by the lift test only; the transform needs just to know a Z word is there.
    #[cfg_attr(not(test), allow(dead_code))]
    z: Option<f64>,
}

fn scan_move(line: &str) -> Option<Words> {
    // The command letter: G or M, possibly after leading whitespace and a line number (N..).
    let code = line.trim_start();
    let code = code
        .strip_prefix(|c: char| c.is_ascii_digit())
        .unwrap_or(code);
    let letter = code.chars().next()?;
    if letter != 'G' && letter != 'M' {
        return None;
    }
    let rest = &code[1..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n = num.parse::<u32>().ok()?;
    if letter != 'G' || !matches!(n, 0 | 1 | 2 | 3) {
        return None;
    }
    // After the command number, scan words. Stop at a comment (; or //) or a checksum (*).
    let start = rest[num.len()..].as_ptr() as usize - rest.as_ptr() as usize;
    let tail = &rest[start..];
    let tail = tail.split([';', '*']).next().unwrap_or(tail);
    let mut has_xyz = false;
    let mut has_e = false;
    let mut z = None;
    for word in tail.split_ascii_whitespace() {
        if word.is_empty() {
            continue;
        }
        let mut ch = word.chars();
        let letter = ch.next()?;
        let value: Option<f64> = ch.as_str().parse().ok();
        match letter {
            'X' | 'Y' => has_xyz = true,
            'Z' => {
                has_xyz = true;
                z = value;
            }
            'E' => has_e = true,
            _ => {}
        }
    }
    Some(Words { has_xyz, has_e, z })
}

/// The dry-run recipe.
pub struct Recipe {
    /// Millimetres added to every Z target. Default 30.
    pub lift_mm: f64,
    /// When false, heater set commands are dropped; when true, temperatures run as normal.
    pub heat: bool,
    /// When set, the transform is refused if any lifted Z would exceed it (the printer's Z travel).
    pub max_z: Option<f64>,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            lift_mm: 30.0,
            heat: false,
            max_z: None,
        }
    }
}

/// Is this line a heater set command that drops out of a dry run (unless heat is on)?
fn is_heater(line: &str) -> bool {
    let code = line.trim_start();
    let code = code
        .strip_prefix(|c: char| c.is_ascii_digit())
        .unwrap_or(code);
    let letter = match code.chars().next() {
        Some(l) => l,
        None => return false,
    };
    if letter != 'M' {
        return false;
    }
    let rest = &code[1..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    matches!(num.as_str(), "104" | "109" | "140" | "190")
}

/// Is this line an extrusion-only move (G1 E.. with no X/Y/Z) that should be dropped?
fn is_extrusion_only(line: &str) -> bool {
    let Some(w) = scan_move(line) else {
        return false;
    };
    !w.has_xyz && w.has_e
}

/// Transform one print file into its dry-run copy. Every line is passed through unchanged except
/// the moves and heaters named above; the file's own structure, comments and macros are untouched.
/// A Klipper start or end macro on this line: Some(true) start, Some(false) end.
fn print_macro(line: &str) -> Option<bool> {
    let head = line.split(|c: char| c.is_whitespace() || c == ';').next()?.to_ascii_uppercase();
    match head.trim_start_matches('_') {
        "PRINT_START" | "START_PRINT" => Some(true),
        "PRINT_END" | "END_PRINT" => Some(false),
        _ => None,
    }
}

/// `BED=85 EXTRUDER=250` style parameters of a macro line (first of the names given).
fn macro_temp(line: &str, names: &[&str]) -> Option<f64> {
    line.split_ascii_whitespace().find_map(|w| {
        let (k, v) = w.split_once('=')?;
        names
            .iter()
            .any(|n| k.eq_ignore_ascii_case(n))
            .then(|| v.parse().ok())
            .flatten()
    })
}

/// Polish WO-07 (2026-10-19, live pass): a Prusa binary G-code file (.bgcode, magic "GCDE") is not text. Read as text it
/// would come out as garbage and be uploaded to the printer under the same name, so a dry run refuses it.
pub fn is_binary_gcode(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GCDE")
}

pub fn transform(input: &str, recipe: &Recipe) -> Result<String, String> {
    let mut max_z: Option<f64> = None;
    let mut absolute = true;
    let out: String = input
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return Ok(line.to_owned());
            }
            // Finish-2 F02 (CORE One L, 2026-10-19): the bowl's dry run, sliced for PETG, stopped at 0 % on the printer's own
            // question about the filament (PLA was loaded) until the owner pressed OK beside it. Nothing is extruded in a dry run, so
            // the file stops naming a filament type and any filament on the printer will do. Printer settings are untouched.
            if trimmed
                .strip_prefix(';')
                .is_some_and(|c| c.trim_start().starts_with("filament_type"))
            {
                return Ok(format!("; dry run: {} dropped (nothing is extruded)", trimmed.trim_start_matches(';').trim()));
            }
            let code = trimmed.split(';').next().unwrap_or("").trim();
            match code.split_ascii_whitespace().next() {
                Some("G90") => absolute = true,
                Some("G91") => absolute = false,
                _ => {}
            }
            match print_macro(code) {
                Some(true) => {
                    let mut lines = Vec::new();
                    if recipe.heat {
                        let bed = macro_temp(code, &["BED", "BED_TEMP"]);
                        let tool = macro_temp(code, &["EXTRUDER", "EXTRUDER_TEMP", "HOTEND", "NOZZLE"]);
                        if let Some(b) = bed {
                            lines.push(format!("M140 S{b}"));
                        }
                        if let Some(t) = tool {
                            lines.push(format!("M104 S{t}"));
                        }
                        lines.push("G28".to_owned());
                        if let Some(b) = bed {
                            lines.push(format!("M190 S{b}"));
                        }
                        if let Some(t) = tool {
                            lines.push(format!("M109 S{t}"));
                        }
                    } else {
                        lines.push("G28".to_owned());
                    }
                    lines.push(format!("; dry run: {code} replaced (it heats and purges)"));
                    return Ok(lines.join("\n"));
                }
                Some(false) => return Ok(format!("; dry run: {code} dropped")),
                None => {}
            }
            // Heater commands drop out unless heat is requested.
            if is_heater(trimmed) {
                return Ok(if recipe.heat {
                    line.to_owned()
                } else {
                    String::new()
                });
            }
            // Extrusion-only moves drop out entirely.
            if is_extrusion_only(trimmed) {
                return Ok(String::new());
            }
            if let Some(_w) = scan_move(trimmed) {
                let mut kept: Vec<String> = Vec::new();
                for word in trimmed.split_ascii_whitespace() {
                    if word.is_empty() {
                        continue;
                    }
                    let mut ch = word.chars();
                    let letter = match ch.next() {
                        Some(l) => l,
                        None => continue,
                    };
                    if letter == 'E' {
                        continue; // no extrusion in a dry run
                    }
                    if letter == 'Z' && absolute {
                        if let Some(v) = ch.as_str().parse::<f64>().ok() {
                            let lifted = v + recipe.lift_mm;
                            if let Some(max) = recipe.max_z {
                                if lifted > max {
                                    return Err(format!(
                                        "dry run lift of {lift:.1} mm on a {max:.1} mm Z axis would exceed the printer's travel",
                                        lift = lifted,
                                        max = max
                                    ));
                                }
                            }
                            max_z = Some(max_z.map_or(lifted, |m| m.max(lifted)));
                            kept.push(format!("Z{lifted}"));
                            continue;
                        }
                    }
                    kept.push(word.to_owned());
                }
                return Ok(kept.join(" "));
            }
            Ok(line.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_gcode_is_recognised_and_text_is_not() {
        assert!(is_binary_gcode(b"GCDE\x01\x00\x00\x00"));
        assert!(!is_binary_gcode(b"; generated by PrusaSlicer\nG28\n"));
        assert!(!is_binary_gcode(b""));
    }

    #[test]
    fn klipper_start_macro_becomes_homing_and_end_macro_goes() {
        let file = "PRINT_START BED=85 EXTRUDER=250\nG1 X10 Y10 Z0.2 E1\nPRINT_END\n";
        let off = transform(file, &Recipe::default()).unwrap();
        assert!(off.starts_with("G28\n"), "{off}");
        assert!(!off.contains("\nPRINT_START") && !off.contains("\nPRINT_END"), "{off}");
        assert!(!off.lines().any(|l| l.starts_with('M') && is_heater(l)), "no heat: {off}");
        let on = transform(file, &Recipe { heat: true, ..Default::default() }).unwrap();
        for want in ["M140 S85", "M104 S250", "G28", "M190 S85", "M109 S250"] {
            assert!(on.lines().any(|l| l == want), "{want} in {on}");
        }
        assert!(!on.lines().any(|l| l.starts_with("PRINT_START")), "the purge never runs");
    }

    #[test]
    fn a_relative_z_move_is_not_lifted() {
        let file = "G1 Z0.2\nG91\nG1 Z0.4\nG90\nG1 Z10\n";
        let out = transform(file, &Recipe::default()).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "G1 Z30.2");
        assert_eq!(lines[2], "G1 Z0.4", "a hop keeps its own length");
        assert_eq!(lines[4], "G1 Z40");
    }

    /// A realistic PrusaSlicer-style slice for a CORE One L: header, printer check, homing,
    /// levelling, heat, moves with absolute extrusion and a retract, end g-code.
    fn prusa() -> &'static str {
        "; generated by PrusaSlicer 2.9.6\n\
         ; printer_model = COREONEL\n\
         ; nozzle_diameter = 0.4\n\
         ; filament_type = PETG\n\
         M862.1 P\"Prusa CORE One L\" S1\n\
         G90\n\
         M82\n\
         M104 S215\n\
         M109 S215\n\
         M140 S60\n\
         M190 S60\n\
         G28\n\
         G29\n\
         G1 Z0.2 F3000\n\
         G1 X10 Y10 E0.5 F1200\n\
         G1 X20 Y10 E1.0\n\
         G1 X20 Y20 E-0.2\n\
         G1 E-1.0 F900\n\
         G1 X30 Y30 E0.8\n\
         M104 S0\n\
         M140 S0\n\
         "
    }

    /// A realistic Orca-style slice for Stella/AD5M: relative extrusion, purge and wipe moves.
    fn orca() -> &'static str {
        "; generated by OrcaSlicer 1.9\n\
         ; printer_model = Voron 2.4 R2 350\n\
         M83\n\
         G1 Z0.2\n\
         G1 X0 Y0 E5 F1200\n\
         G1 X100 Y0 E10\n\
         ; purge line\n\
         G1 X0 Y150 E8\n\
         G1 X100 Y150 E10\n\
         G1 E-5 F900\n\
         "
    }

    #[test]
    fn no_line_extrudes_after_the_transform() {
        for (name, src) in [("prusa", prusa()), ("orca", orca())] {
            let out = transform(src, &Recipe::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
            for line in out.lines() {
                let t = line.trim();
                if t.starts_with('G') || t.starts_with('M') {
                    // A remaining E word on a move would extrude.
                    assert!(
                        !t.split_ascii_whitespace().any(|w| {
                            let mut c = w.chars();
                            matches!(c.next(), Some('E')) && c.as_str().parse::<f64>().is_ok()
                        }),
                        "{name}: extrusion left in '{t}'"
                    );
                }
            }
        }
    }

    #[test]
    fn extrusion_only_moves_are_dropped_but_moves_are_kept() {
        let out = transform(orca(), &Recipe::default()).unwrap();
        // The retract G1 E-5 is gone.
        assert!(!out.lines().any(|l| l.trim() == "G1 E-5 F900"));
        // The X/Y moves survive (E removed).
        assert!(out.lines().any(|l| l.contains("X100 Y0")));
        assert!(out.lines().any(|l| l.contains("X100 Y150")));
    }

    #[test]
    fn z_is_lifted_and_max_z_equals_original_plus_lift() {
        let out = transform(
            prusa(),
            &Recipe {
                lift_mm: 30.0,
                ..Default::default()
            },
        )
        .unwrap();
        let mut max_z: Option<f64> = None;
        for line in out.lines() {
            let w = match scan_move(line.trim()) {
                Some(w) => w,
                None => continue,
            };
            if let Some(z) = w.z {
                max_z = Some(max_z.map_or(z, |m| m.max(z)));
            }
        }
        let want = 0.2 + 30.0;
        assert!(
            (max_z.unwrap() - want).abs() < 0.001,
            "max Z {max_z:?}, want {want}"
        );
        // Every lifted Z is at least 30 above where it was.
        for line in out.lines() {
            if let Some(w) = scan_move(line.trim()) {
                if let Some(z) = w.z {
                    assert!(z >= 30.0, "lifted Z {z} in '{line}'");
                }
            }
        }
    }

    #[test]
    fn homing_levelling_and_prusa_checks_survive_heat_off() {
        let out = transform(prusa(), &Recipe::default()).unwrap();
        assert!(out.contains("M862.1"), "prusa printer check kept");
        // finish-2 F02: no filament question on the printer for a file that extrudes nothing
        assert!(!out.lines().any(|l| l.trim_start_matches(';').trim().starts_with("filament_type")), "filament type gone");
        assert!(out.contains("; dry run: filament_type = PETG dropped"));
        assert!(out.contains("G28"), "homing kept");
        assert!(out.contains("G29"), "levelling kept");
        assert!(!out.contains("M104 S215"), "heat dropped");
        assert!(!out.contains("M140 S60"), "bed heat dropped");
        assert!(!out.contains("M109 S215"), "wait-for-heat dropped");
        assert!(!out.contains("M190 S60"), "wait-for-bed dropped");
        assert!(
            !out.contains("M104 S0"),
            "cool-down set is a heater command, dropped too"
        );
    }

    #[test]
    fn heat_on_keeps_temperatures() {
        let out = transform(
            prusa(),
            &Recipe {
                heat: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.contains("M104 S215"), "nozzle heat kept");
        assert!(out.contains("M140 S60"), "bed heat kept");
    }

    #[test]
    fn lift_exceeding_z_travel_is_refused() {
        let err = transform(
            prusa(),
            &Recipe {
                lift_mm: 30.0,
                max_z: Some(20.0),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("20.0") || err.contains("travel"), "{err}");
        // Within travel: fine.
        transform(
            prusa(),
            &Recipe {
                lift_mm: 5.0,
                max_z: Some(20.0),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn default_lift_is_thirty_mm() {
        let r = Recipe::default();
        assert_eq!(r.lift_mm, 30.0);
        assert!(!r.heat);
        assert!(r.max_z.is_none());
    }
}
