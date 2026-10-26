//! The G-code follower (finish board WO-01): one position computation for every view.
//!
//! Pure signal mapping: given what a printer reports and the file it is printing, which layer is
//! printed and how far through it. The page parses the file once (web/hud-gcode.js, reused for
//! every brand) and sends the signal; this module turns it into {layer, fraction} and labels the
//! source, so the printer page, the farm wall tile, the phone and the demo printers all draw the
//! same head from the same signal.
//!
//! Signal preference, best first (WO-01 §Build 2):
//!   - byte position in the file (Klipper's virtual_sdcard.file_position)
//!   - the printer's own progress percent, mapped onto the file's layer byte spans
//!   - current layer of total (FlashForge)
//!   - elapsed print time against the file's own time estimate
//!
//! A reported Z re-anchors the drawn height (PrusaLink always reports Z) so the model is never on
//! the wrong layer. Between reports the head plays forward from the last position; on the next
//! report it snaps to the mapped position. Nothing here is ever a guess: a printer with no file
//! access and no layer number shows its last real position and says so.

/// One drawn layer: its Z height and the byte span of the file it covers.
pub struct LayerSpan {
    pub z: f64,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// The file as the follower sees it: one span per layer, the file's byte length, the layer count
/// the slicer named and its own time estimate.
pub struct ParsedFile {
    pub layers: Vec<LayerSpan>,
    pub byte_length: usize,
    pub total_layers: Option<u32>,
    pub estimated_seconds: Option<f64>,
}

/// The best signal a printer can give about where it is in the file.
pub enum Signal {
    /// The byte the printer has read (Klipper file_position; the Kobra X bridge answers as Klipper).
    BytePosition(usize),
    /// Progress fraction 0..1 the printer reports (Prusa computes it from M73 lines in the file;
    /// map it onto the file's layer spans).
    Progress(f64),
    /// Current layer (1-based) and, when the printer names it, the total layer count.
    Layer { current: u32, total: Option<u32> },
    /// Elapsed print seconds and the file's own estimate from its slicer metadata.
    Time { elapsed: f64, estimate: f64 },
}

/// Where the drawn position comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Played forward from the file.
    FollowingTheFile,
}

/// The mapped position: which layer, how far through it, the labelled source and a re-anchored Z.
pub struct Followed {
    pub layer: u32,
    pub fraction: f64,
    pub source: Source,
    pub z: Option<f64>,
}

/// Clamp a layer index into [0, total_layers).
fn in_range(layer: u32, total: Option<u32>) -> u32 {
    match total {
        Some(t) if t > 0 => layer.min(t - 1),
        _ => layer,
    }
}

/// Map a byte offset onto {layer, fraction} by binary search over the layer byte spans (the same
/// rule as web/hud-gcode.js progressToLayer: Moonraker's file_position counts bytes).
pub fn map_byte(parsed: &ParsedFile, offset: usize) -> Option<(u32, f64)> {
    if parsed.layers.is_empty() {
        return None;
    }
    let last = parsed.layers.last()?;
    let end = last.byte_end.max(parsed.byte_length);
    let at = offset.min(end);
    let mut lo = 0usize;
    let mut hi = parsed.layers.len();
    while lo < hi {
        let mid = (lo + hi) >> 1;
        if parsed.layers[mid].byte_start <= at {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let layer = lo.saturating_sub(1) as u32;
    let span = parsed.layers[layer as usize].byte_end - parsed.layers[layer as usize].byte_start;
    let fraction = if span > 0 {
        ((at - parsed.layers[layer as usize].byte_start) as f64 / span as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some((in_range(layer, parsed.total_layers), fraction))
}

/// Map a progress fraction onto {layer, fraction} through the file's byte spans.
pub fn map_progress(parsed: &ParsedFile, progress: f64) -> Option<(u32, f64)> {
    let p = progress.clamp(0.0, 1.0);
    let offset = ((parsed.byte_length as f64) * p) as usize;
    map_byte(parsed, offset)
}

/// Map a current layer (1-based) of total onto {layer, fraction 0}.
pub fn map_layer(current: u32, total: Option<u32>) -> (u32, f64) {
    (in_range(current.saturating_sub(1), total), 0.0)
}

/// Map elapsed time against the file's own estimate onto {layer, fraction}.
pub fn map_time(parsed: &ParsedFile, elapsed: f64, estimate: f64) -> Option<(u32, f64)> {
    if estimate <= 0.0 || elapsed < 0.0 {
        return None;
    }
    let fraction = (elapsed / estimate).clamp(0.0, 1.0);
    let total = parsed.total_layers.unwrap_or(parsed.layers.len() as u32);
    if total == 0 {
        return None;
    }
    let raw = fraction * (total as f64);
    let last = (total - 1) as f64;
    let layer = raw.floor().min(last) as u32;
    let frac = if raw >= last {
        fraction
    } else {
        raw - layer as f64
    };
    Some((in_range(layer, Some(total)), frac.clamp(0.0, 1.0)))
}

/// Turn the best available signal into a Followed position, re-anchoring Z when the printer
/// reports one. The previous Followed and the wall-clock time let a caller smooth between reports.
pub fn follow(parsed: &ParsedFile, signal: &Signal, z_report: Option<f64>) -> Option<Followed> {
    let (layer, fraction, source) = match signal {
        Signal::BytePosition(offset) => {
            let (l, f) = map_byte(parsed, *offset)?;
            (l, f, Source::FollowingTheFile)
        }
        Signal::Progress(p) => {
            let (l, f) = map_progress(parsed, *p)?;
            (l, f, Source::FollowingTheFile)
        }
        Signal::Layer { current, total } => {
            let (l, f) = map_layer(*current, *total);
            (l, f, Source::FollowingTheFile)
        }
        Signal::Time { elapsed, estimate } => {
            let (l, f) = map_time(parsed, *elapsed, *estimate)?;
            (l, f, Source::FollowingTheFile)
        }
    };
    Some(Followed {
        layer,
        fraction,
        source,
        z: z_report.filter(|z| z.is_finite()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed() -> ParsedFile {
        ParsedFile {
            layers: vec![
                LayerSpan {
                    z: 0.2,
                    byte_start: 0,
                    byte_end: 100,
                },
                LayerSpan {
                    z: 0.4,
                    byte_start: 100,
                    byte_end: 200,
                },
                LayerSpan {
                    z: 0.6,
                    byte_start: 200,
                    byte_end: 300,
                },
            ],
            byte_length: 300,
            total_layers: Some(3),
            estimated_seconds: Some(600.0),
        }
    }

    #[test]
    fn byte_position_maps_onto_the_layer_it_lands_in() {
        let p = parsed();
        assert_eq!(map_byte(&p, 0), Some((0, 0.0)));
        assert_eq!(map_byte(&p, 50), Some((0, 0.5)));
        assert_eq!(map_byte(&p, 100), Some((1, 0.0)));
        assert_eq!(map_byte(&p, 150), Some((1, 0.5)));
        assert_eq!(map_byte(&p, 299), Some((2, 0.99)));
        assert_eq!(map_byte(&p, 300), Some((2, 1.0)));
        assert_eq!(map_byte(&p, 1_000_000), Some((2, 1.0)));
    }

    #[test]
    fn progress_percent_maps_through_the_file_byte_spans() {
        let p = parsed();
        assert_eq!(map_progress(&p, 0.0), Some((0, 0.0)));
        assert_eq!(map_progress(&p, 0.5), Some((1, 0.5)));
        assert_eq!(map_progress(&p, 1.0), Some((2, 1.0)));
        assert_eq!(map_progress(&p, -0.3), Some((0, 0.0)));
        assert_eq!(map_progress(&p, 2.0), Some((2, 1.0)));
    }

    #[test]
    fn an_empty_parse_refuses_byte_and_progress() {
        let p = ParsedFile {
            layers: vec![],
            byte_length: 0,
            total_layers: None,
            estimated_seconds: None,
        };
        assert!(map_byte(&p, 10).is_none());
        assert!(map_progress(&p, 0.5).is_none());
    }

    #[test]
    fn layer_mapping_is_one_based_and_clamps_to_the_file() {
        assert_eq!(map_layer(1, Some(3)), (0, 0.0));
        assert_eq!(map_layer(2, Some(3)), (1, 0.0));
        assert_eq!(map_layer(3, Some(3)), (2, 0.0));
        assert_eq!(map_layer(9, Some(3)), (2, 0.0));
        assert_eq!(map_layer(1, None), (0, 0.0));
    }

    #[test]
    fn time_mapping_spreads_prints_over_the_layers() {
        let p = parsed();
        assert_eq!(map_time(&p, 0.0, 600.0), Some((0, 0.0)));
        assert_eq!(map_time(&p, 300.0, 600.0), Some((1, 0.5)));
        assert_eq!(map_time(&p, 600.0, 600.0), Some((2, 1.0)));
        assert!(map_time(&p, 100.0, 0.0).is_none());
        assert!(map_time(&p, -10.0, 600.0).is_none());
    }

    #[test]
    fn signal_preference_is_byte_then_progress_then_layer_then_time() {
        let p = parsed();
        // A byte position always wins, labelled following the file.
        let f = follow(&p, &Signal::BytePosition(150), None).unwrap();
        assert_eq!((f.layer, f.fraction), (1, 0.5));
        assert_eq!(f.source, Source::FollowingTheFile);
        // Progress without a byte position.
        let f = follow(&p, &Signal::Progress(0.5), None).unwrap();
        assert_eq!((f.layer, f.fraction), (1, 0.5));
        // Layer (1-based) wins over time.
        let f = follow(
            &p,
            &Signal::Layer {
                current: 3,
                total: Some(3),
            },
            None,
        )
        .unwrap();
        assert_eq!((f.layer, f.fraction), (2, 0.0));
    }

    #[test]
    fn a_reported_z_re_anchors_the_drawn_height() {
        let p = parsed();
        let f = follow(&p, &Signal::BytePosition(150), Some(0.4)).unwrap();
        assert_eq!(f.z, Some(0.4));
        // A non-finite report is ignored, not drawn.
        let f = follow(&p, &Signal::BytePosition(150), Some(f64::NAN)).unwrap();
        assert!(f.z.is_none());
    }

    #[test]
    fn a_printer_with_no_signal_shows_nothing_invented() {
        let p = parsed();
        assert!(follow(&p, &Signal::BytePosition(0), None).is_some());
        // An unparseable file has no layers, so nothing maps.
        let empty = ParsedFile {
            layers: vec![],
            byte_length: 0,
            total_layers: None,
            estimated_seconds: None,
        };
        assert!(follow(&empty, &Signal::BytePosition(0), None).is_none());
        assert!(follow(&empty, &Signal::Progress(0.5), None).is_none());
    }

    #[test]
    fn a_time_signal_uses_the_files_own_estimate_and_each_layer_knows_its_height() {
        let p = parsed();
        let estimate = p.estimated_seconds.unwrap();
        let f = follow(
            &p,
            &Signal::Time {
                elapsed: 300.0,
                estimate,
            },
            None,
        )
        .unwrap();
        assert_eq!((f.layer, f.fraction), (1, 0.5));
        assert_eq!(p.layers[f.layer as usize].z, 0.4);
    }
}
