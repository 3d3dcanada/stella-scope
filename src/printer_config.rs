//! L2: the one config value the user changes by hand, edited through Stella Scope instead.
//! `[heater_fan hotend_fan] heater_temp` decides when the hotend fan spins up. On the test printer
//! it was moved 30 -> 50 on 2026-09-15 because a 60 C bed in the enclosure kept the nozzle at 30
//! and the fan cycled while probing. Anything outside 30..60 is a refusal, not a slider. the test
//! printer's `printer.cfg` contains the section TWICE: the live one at line 381 and a fully
//! commented-out reference copy at line 825 (`#heater_temp: 50.0`). The edit therefore takes the
//! first section whose `heater_temp` is not commented out, and rewrites only the number on that
//! one line — the trailing `# 2026-09-15: was 30.0...` comment is the own and is kept verbatim.

use crate::{moonraker::Moonraker, Result};
use serde_json::Value;

pub const MIN_C: f64 = 30.0;
pub const MAX_C: f64 = 60.0;
pub const FILE: &str = "printer.cfg";
const SECTION: &str = "heater_fan hotend_fan";
/// Printer.cfg on the test printer is 43 KB / 1383 lines. 1 MB leaves room to grow and still
/// refuses a mistaken root, so a bad path cannot pull an arbitrary file into memory.
pub const MAX_CONFIG_BYTES: usize = 1_048_576;

/// The `heater_temp` line inside `[heater_fan hotend_fan]`: where it is and what it says now.
#[derive(Debug)]
pub struct Located {
    /// Zero-based index into the file's lines.
    pub index: usize,
    pub celsius: f64,
    /// Everything after the value on that line, comment included, kept byte-exact.
    pub trailer: String,
    /// The indentation and key exactly as written, e.g. `heater_temp: `.
    pub head: String,
}

/// Find the live `heater_temp` line. Section headers are `[name]`; a line starting with `#` or `;`
/// is a comment and never matches, which is what skips the reference copy further down the file.
pub fn locate(text: &str) -> Result<Located> {
    let mut in_section = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_section = trimmed
                .trim_start_matches('[')
                .split(']')
                .next()
                .is_some_and(|name| name.trim() == SECTION);
            continue;
        }
        if !in_section || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("heater_temp") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        // Split the value from whatever follows it (whitespace, then usually a `#` comment).
        let value_start = line.len() - rest.len();
        let value_len = rest
            .trim_start()
            .find(|c: char| c.is_whitespace())
            .unwrap_or(rest.trim_start().len());
        let lead = rest.len() - rest.trim_start().len();
        let value = &rest.trim_start()[..value_len];
        let celsius = value.parse::<f64>().map_err(|_| {
            format!("[{SECTION}] heater_temp is {value:?}, which is not a number; edit it on the Pi")
        })?;
        return Ok(Located {
            index,
            celsius,
            trailer: rest[lead + value_len..].to_owned(),
            head: line[..value_start + lead].to_owned(),
        });
    }
    Err(format!(
        "no uncommented heater_temp found inside [{SECTION}] in {FILE}; nothing was changed"
    ))
}

/// Rewrite only the located line. Returns the whole new file, preserving the original's final
/// newline exactly: `lines()` drops it, so it is restored from the source.
pub fn rewrite(text: &str, found: &Located, celsius: f64) -> String {
    let mut out: Vec<String> = text.lines().map(str::to_owned).collect();
    out[found.index] = format!("{}{}{}", found.head, format_c(celsius), found.trailer);
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Klipper writes this value as `50.0`. Keep one decimal so the file style does not drift.
fn format_c(celsius: f64) -> String {
    format!("{celsius:.1}")
}

/// A unified diff of the one changed line, with three lines of context either side.
pub fn diff(before: &str, after: &str) -> String {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let changed: Vec<usize> = (0..old.len().max(new.len()))
        .filter(|i| old.get(*i) != new.get(*i))
        .collect();
    if changed.is_empty() {
        return String::new();
    }
    let start = changed[0].saturating_sub(3);
    let end = (changed[changed.len() - 1] + 4).min(old.len().max(new.len()));
    let mut out = format!("--- a/{FILE}\n+++ b/{FILE}\n");
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        start + 1,
        end - start,
        start + 1,
        end - start
    ));
    for i in start..end {
        match (old.get(i), new.get(i)) {
            (Some(a), Some(b)) if a == b => out.push_str(&format!(" {a}\n")),
            (a, b) => {
                if let Some(a) = a {
                    out.push_str(&format!("-{a}\n"));
                }
                if let Some(b) = b {
                    out.push_str(&format!("+{b}\n"));
                }
            }
        }
    }
    out
}

pub fn bounds_error(celsius: f64) -> Option<String> {
    (!celsius.is_finite() || !(MIN_C..=MAX_C).contains(&celsius)).then(|| {
        format!(
            "hotend fan heater_temp must be {MIN_C:.0} to {MAX_C:.0} °C; {FILE} was not changed"
        )
    })
}

/// Current value, for the editor to open on.
pub fn read(moonraker: &Moonraker) -> Result<Value> {
    let text = moonraker.download_config(FILE, MAX_CONFIG_BYTES)?;
    let found = locate(&text)?;
    Ok(serde_json::json!({
        "heater_temp": found.celsius,
        "min": MIN_C,
        "max": MAX_C,
        "file": FILE,
        "line": found.index + 1,
        "source": format!("GET /server/files/config/{FILE}")
    }))
}

/// Preview: download once, locate, bound, rewrite in memory. Returns the diff plus both versions,
/// so apply uploads exactly the bytes that were previewed and the file is fetched only once.
pub fn preview(moonraker: &Moonraker, celsius: f64) -> Result<(String, String, String)> {
    if let Some(error) = bounds_error(celsius) {
        return Err(error);
    }
    let text = moonraker.download_config(FILE, MAX_CONFIG_BYTES)?;
    let found = locate(&text)?;
    let after = rewrite(&text, &found, celsius);
    if after == text {
        return Err(format!(
            "heater_temp is already {}; {FILE} was not changed",
            format_c(celsius)
        ));
    }
    Ok((diff(&text, &after), text, after))
}

/// Apply: save the dated backup beside the file FIRST, then upload the edit, then FIRMWARE_RESTART.
/// Backup before edit, never after — a failed upload must still leave a copy of what was there.
pub fn apply(moonraker: &Moonraker, original: &str, edited: &str, stamp: &str) -> Result<Value> {
    let backup = format!("{FILE}.before-{stamp}");
    moonraker
        .upload_config(&backup, original)
        .map_err(|e| format!("backup {backup} could not be saved, so nothing was edited: {e}"))?;
    moonraker.upload_config(FILE, edited).map_err(|e| {
        format!("{FILE} upload did not confirm; the backup {backup} is on the Pi: {e}")
    })?;
    let restarted = moonraker.post(
        "/printer/gcode/script",
        Some(&serde_json::json!({"script": "FIRMWARE_RESTART"})),
    );
    Ok(serde_json::json!({
        "status": "completed",
        "file": FILE,
        "backup": backup,
        // FIRMWARE_RESTART tears down the connection Moonraker is answering on, so a transport
        // error here is the expected shape of success. Report it rather than calling it a failure.
        "firmware_restart": match restarted {
            Ok(_) => "FIRMWARE_RESTART accepted".to_owned(),
            Err(e) => format!("FIRMWARE_RESTART was sent; Moonraker did not confirm, which is normal because the restart drops the connection: {e}"),
        },
        "message": format!("{FILE} was uploaded and FIRMWARE_RESTART was sent; the physical result was not independently observed")
    }))
}

/// `printer.cfg.before-2026-09-20T14-33-05Z`: sortable, no colons, safe as a filename.
pub fn stamp(now: f64) -> String {
    let secs = now as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (mut year, mut left) = (1970_i64, days);
    loop {
        let len = if leap(year) { 366 } else { 365 };
        if left < len {
            break;
        }
        left -= len;
        year += 1;
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while left >= months[month] {
        left -= months[month];
        month += 1;
    }
    format!(
        "{year:04}-{:02}-{:02}T{:02}-{:02}-{:02}Z",
        month + 1,
        left + 1,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test printer's real shape: the live section, then an unrelated section, then the
    /// commented-out reference copy that also says `heater_temp`.
    const CFG: &str = "[extruder]\nsensor_type: PT1000\n\n[heater_fan hotend_fan]\npin: EBBCan: PA1\nheater: extruder\nheater_temp: 50.0    # 2026-09-15: was 30.0 (Klipper default 50). Bed 60 kept the enclosed nozzle at 30 C\n\n[controller_fan controller_fan]\nheater_temp: 45.0\n\n#[heater_fan hotend_fan]\n#heater: extruder\n#heater_temp: 50.0\n";

    #[test]
    fn diff_touches_only_the_heater_temp_line_inside_the_section() {
        let found = locate(CFG).unwrap();
        assert_eq!(found.celsius, 50.0);
        assert_eq!(found.index, 6); // the live line, not the commented copy at the end
        let after = rewrite(CFG, &found, 45.0);

        // The one and only difference between the two files is that line.
        let changed: Vec<(usize, &str, &str)> = CFG
            .lines()
            .zip(after.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i, a, b))
            .collect();
        assert_eq!(changed.len(), 1, "exactly one line changes: {changed:?}");
        assert_eq!(changed[0].0, 6);
        assert_eq!(CFG.lines().count(), after.lines().count());

        // The trailing comment survives byte for byte; only the number moved.
        assert_eq!(
            changed[0].2,
            "heater_temp: 45.0    # 2026-09-15: was 30.0 (Klipper default 50). Bed 60 kept the enclosed nozzle at 30 C"
        );
        // The commented reference copy and the controller_fan section are untouched.
        assert!(after.contains("#heater_temp: 50.0"));
        assert!(after.contains("[controller_fan controller_fan]\nheater_temp: 45.0"));
        assert!(after.ends_with('\n'));

        let unified = diff(CFG, &after);
        assert!(unified.starts_with("--- a/printer.cfg\n+++ b/printer.cfg\n@@ "));
        let minus: Vec<&str> = unified
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .collect();
        let plus: Vec<&str> = unified
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .collect();
        assert_eq!(minus.len(), 1);
        assert_eq!(plus.len(), 1);
        assert!(minus[0].contains("heater_temp: 50.0"));
        assert!(plus[0].contains("heater_temp: 45.0"));
    }

    #[test]
    fn bounds_refuse_outside_30_to_60() {
        for bad in [29.9, 60.1, 0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(bounds_error(bad).is_some(), "{bad} should be refused");
        }
        for good in [30.0, 45.0, 50.0, 60.0] {
            assert!(bounds_error(good).is_none(), "{good} should be allowed");
        }
    }

    #[test]
    fn a_section_with_no_live_heater_temp_is_a_refusal_not_a_guess() {
        let only_commented = "[heater_fan hotend_fan]\npin: PA1\n#heater_temp: 50.0\n";
        assert!(locate(only_commented).unwrap_err().contains("uncommented"));
        let elsewhere = "[controller_fan controller_fan]\nheater_temp: 45.0\n";
        assert!(locate(elsewhere).is_err());
    }

    #[test]
    fn stamp_is_a_sortable_filename() {
        // Checked against `date -u -d @<epoch>`, not against this function.
        assert_eq!(stamp(1_789_948_800.0), "2026-09-21T00-00-00Z");
        assert_eq!(stamp(1_789_689_600.0), "2026-09-18T00-00-00Z");
        assert_eq!(stamp(0.0), "1970-01-01T00-00-00Z");
        assert_eq!(stamp(1_789_948_800.0 + 3661.0), "2026-09-21T01-01-01Z");
        // Sortable as text, and legal in a filename on every filesystem Moonraker writes to.
        assert!(stamp(1_789_948_800.0) > stamp(1_789_689_600.0));
        assert!(!stamp(1_789_948_800.0).contains(':'));
    }
}
