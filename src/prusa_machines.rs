//! Every Prusa machine Stella Scope knows (finish-2 F09, 2026-10-19): what a PrusaLink printer is called in a sliced file,
//! how big its plate is, what nozzle it ships with, which 3D model draws it and, where none does, exactly why.
//!
//! One table for what used to be two guesses (the PrusaLink driver's hostname lists) and a page that knew nothing: the driver asks it for a
//! printer's model and bed, Add a printer offers it so a Prusa starts with its own defaults, and Printer models lists the machines that have
//! no 3D model yet with their reason. Sizes are the published print volumes (prusa3d.com and the machine profiles of PrusaSlicer, which
//! are facts about the machines); the slicer names are what PrusaSlicer writes as `printer_model` in a sliced file.
//!
//! No machine here is given a model that is not its own: `cad` names a catalogue model only where the CAD is that machine's (or, said so in
//! the model's own notice, the one a sibling shares), and `no_model` is the reason otherwise.
use serde_json::{json, Value};

pub struct Machine {
    /// Our key: the catalogue model's id where there is one.
    pub id: &'static str,
    /// What the machine is called, with the siblings it covers.
    pub name: &'static str,
    /// The short name the printer is given in a list.
    pub short: &'static str,
    /// What PrusaSlicer writes as `printer_model` for it.
    pub slicer: &'static [&'static str],
    /// Words a PrusaLink hostname ("prusa-core-one-l") holds for it; the first that matches wins, so a longer name comes first.
    pub hosts: &'static [&'static str],
    /// Print area in millimetres (width, depth) when Prusa publishes it; a round plate (the HT90) is its diameter twice.
    pub bed: Option<[f64; 2]>,
    pub height: Option<f64>,
    pub nozzle: f64,
    /// The catalogue model that draws it.
    pub cad: Option<&'static str>,
    /// Why no 3D model draws it, when none does.
    pub no_model: &'static str,
    /// What to tell the person about a camera for it.
    pub camera: &'static str,
    /// Where the numbers come from.
    pub source: &'static str,
}

const BUDDY: &str = "Prusa's Buddy3D camera fits it: switch it on at the printer, then use rtsp://<camera address>/live. Any other camera that gives an RTSP address works too.";
const NO_BUDDY: &str = "Prusa does not list its Buddy3D camera for this machine. Any camera that gives an RTSP address works; leave it empty for none.";
const PROFILE: &str = "prusa3d.com and PrusaSlicer's machine profile";

pub const MACHINES: &[Machine] = &[
    Machine {
        id: "prusa_core_one_l",
        name: "Prusa CORE One L / CORE One L+",
        short: "CORE One L",
        slicer: &["COREONEL"],
        hosts: &["core-one-l", "coreonel"],
        bed: Some([300.0, 300.0]),
        height: Some(330.0),
        nozzle: 0.4,
        cad: Some("prusa_core_one_l"),
        no_model: "",
        camera: BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_core_one",
        name: "Prusa CORE One / CORE One+ (Gen 2)",
        short: "CORE One",
        slicer: &["COREONE"],
        hosts: &["core-one", "coreone"],
        bed: Some([250.0, 220.0]),
        height: Some(270.0),
        nozzle: 0.4,
        cad: Some("prusa_core_one"),
        no_model: "",
        camera: BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_core_one_indx",
        name: "Prusa CORE One+ INDX (8 tools)",
        short: "CORE One INDX",
        slicer: &["COREONE_INDX"],
        hosts: &[],
        bed: Some([248.0, 205.0]),
        height: Some(270.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa publishes the INDX's printed parts (Printables 1758233) but no assembly of the toolchanger, and the CORE One model is the machine without it, so no model is drawn that would show it without its tools.",
        camera: BUDDY,
        source: "PrusaSlicer's machine profile",
    },
    Machine {
        id: "prusa_core_one_l_indx",
        name: "Prusa CORE One L+ INDX (8 tools)",
        short: "CORE One L INDX",
        slicer: &[],
        hosts: &[],
        bed: None,
        height: None,
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa has published no CAD of the INDX toolchanger on the CORE One L, so no model is drawn that would show the machine without its tools. Its plate size is not in a place Stella Scope reads: set it under Printers.",
        camera: BUDDY,
        source: "prusa3d.com (no print volume found for this variant)",
    },
    Machine {
        id: "prusa_mk4s",
        name: "Original Prusa MK4S / MK4 / MK3.9S / MK3.9",
        short: "MK4S",
        slicer: &["MK4S", "MK4", "MK3.9S", "MK39S", "MK3.9", "MK39"],
        hosts: &["mk39", "mk4"],
        bed: Some([250.0, 210.0]),
        height: Some(220.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa publishes the MK4's printed parts (Printables 451500, 451501) but no assembly. The one community assembly under an open licence (Printables 961911, GPL 2.0, \"MK4-COMPLETE\") has the frame, the X axis and the extruder but no heatbed and no Y carriage, so a print could not be drawn on it; the complete MK4S assemblies (Printables 1881221 and 1120525) are non-commercial, which a build cannot carry.",
        camera: BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_mk3s",
        name: "Original Prusa i3 MK3S+ / MK3S / MK3",
        short: "MK3S+",
        slicer: &["MK3S", "MK3"],
        hosts: &["mk3s", "mk3"],
        bed: Some([250.0, 210.0]),
        height: Some(210.0),
        nozzle: 0.4,
        cad: Some("prusa_mk3s"),
        no_model: "",
        camera: NO_BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_mk35",
        name: "Original Prusa MK3.5S / MK3.5",
        short: "MK3.5S",
        slicer: &["MK3.5S", "MK35S", "MK3.5", "MK35"],
        hosts: &["mk35"],
        bed: Some([250.0, 210.0]),
        height: Some(210.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa publishes the MK3.5's printed parts (Printables 706515) but no assembly, and no community assembly of it was found. The MK3S+ model is a different machine and is not drawn in its place.",
        camera: NO_BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_mini",
        name: "Original Prusa MINI / MINI+",
        short: "MINI+",
        slicer: &["MINI"],
        hosts: &["mini"],
        bed: Some([180.0, 180.0]),
        height: Some(180.0),
        nozzle: 0.4,
        cad: Some("prusa_mini"),
        no_model: "",
        camera: NO_BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_xl",
        name: "Original Prusa XL / XL+ (1 to 5 tools)",
        short: "XL",
        slicer: &["XL", "XL2", "XL3", "XL4", "XL5"],
        hosts: &["xl"],
        bed: Some([360.0, 360.0]),
        height: Some(360.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa publishes the XL's printed parts (Printables 449554, 1814173) but no assembly. No complete XL assembly was found on Printables, GrabCAD, GitHub or Thingiverse: the community XL models are a 3 MB outline and a mesh without motors, which are not real CAD of the machine.",
        camera: BUDDY,
        source: PROFILE,
    },
    Machine {
        id: "prusa_pro_ht90",
        name: "Prusa Pro HT90",
        short: "Pro HT90",
        slicer: &["HT90", "HT90_HT"],
        hosts: &["ht90"],
        bed: Some([300.0, 300.0]),
        height: Some(405.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa publishes the HT90's printed parts (Printables 903904) but no assembly, and no community assembly of it was found. Its plate is round, 300 mm across.",
        camera: NO_BUDDY,
        source: "PrusaSlicer's machine profile (a round plate of 300 mm)",
    },
    Machine {
        id: "prusa_pro_afs",
        name: "Prusa Pro AFS (iX)",
        short: "Pro AFS",
        slicer: &["iX"],
        hosts: &["afs"],
        bed: Some([260.0, 260.0]),
        height: Some(175.0),
        nozzle: 0.4,
        cad: None,
        no_model: "Prusa has published no CAD of this machine and no community assembly of it was found.",
        camera: NO_BUDDY,
        source: "PrusaSlicer's machine profile",
    },
];

pub fn by_id(id: &str) -> Option<&'static Machine> {
    MACHINES.iter().find(|m| m.id == id)
}

/// The machine a PrusaLink hostname ("prusa-core-one-l") names, if it names one. The first table row with a word the hostname holds wins, and
/// longer words are listed first, so "core-one-l" is never taken for "core-one" and "mk39" never for "mk3". A hostname that matches nothing is None, never a guess.
pub fn for_hostname(host: &str) -> Option<&'static Machine> {
    let h = host.to_ascii_lowercase();
    MACHINES
        .iter()
        .flat_map(|m| m.hosts.iter().map(move |k| (*k, m)))
        .filter(|(k, _)| h.contains(k))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, m)| m)
}

/// The machine whose short name a printer was given ("CORE One L", what `model_hint` answers).
pub fn for_short(short: &str) -> Option<&'static Machine> {
    MACHINES.iter().find(|m| m.short == short)
}

/// The table as the pages read it (`GET /api/prusa-machines`): defaults for Add a printer, and the machines that have no 3D model yet with why.
pub fn json() -> Value {
    let rows: Vec<Value> = MACHINES
        .iter()
        .map(|m| {
            json!({
                "id":m.id,"name":m.name,"short":m.short,"slicer_model":m.slicer.first(),"slicer_models":m.slicer,
                "bed_mm":m.bed,"height_mm":m.height,"nozzle_mm":m.nozzle,
                "cad_model":m.cad,"no_model":(m.cad.is_none()).then_some(m.no_model),
                "camera":m.camera,"source":m.source,
            })
        })
        .collect();
    json!({"schema":"stella-scope.prusa-machines.v1","machines":rows})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_names_the_longest_machine_it_holds() {
        let name = |h: &str| for_hostname(h).map(|m| m.id);
        assert_eq!(name("prusa-core-one-l"), Some("prusa_core_one_l"), "not the CORE One");
        assert_eq!(name("prusa-core-one"), Some("prusa_core_one"));
        assert_eq!(name("PrusaMK39"), Some("prusa_mk4s"), "the MK3.9 is not an MK3");
        assert_eq!(name("prusa-mk4"), Some("prusa_mk4s"));
        assert_eq!(name("prusa-mk3s"), Some("prusa_mk3s"));
        assert_eq!(name("prusa-mk35"), Some("prusa_mk35"), "the MK3.5 is not an MK3");
        assert_eq!(name("prusa-xl"), Some("prusa_xl"));
        assert_eq!(name("prusa-mini"), Some("prusa_mini"));
        assert_eq!(name("kitchen-printer"), None, "a name that matches nothing is not guessed");
    }

    #[test]
    fn every_machine_has_its_own_short_name_and_the_slicer_names_are_not_shared() {
        for m in MACHINES {
            assert_eq!(for_short(m.short).map(|x| x.id), Some(m.id), "{}", m.id);
            for name in m.slicer {
                let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
                let owners: Vec<&str> = MACHINES.iter().filter(|o| o.slicer.iter().any(|x| norm(x) == norm(name))).map(|o| o.id).collect();
                assert_eq!(owners, vec![m.id], "{name} is one machine's name in a sliced file");
            }
        }
        assert_eq!(for_short("CORE One L").map(|m| m.slicer), Some(&["COREONEL"][..]));
    }

    #[test]
    fn every_machine_has_a_model_or_the_exact_reason_it_has_none() {
        for m in MACHINES {
            match m.cad {
                Some(id) => {
                    assert!(m.no_model.is_empty(), "{}: a machine with a model gives no reason", m.id);
                    assert_eq!(id, m.id, "{}: the model is the machine's own", m.id);
                    assert!(crate::printer_models::machine(id).is_some(), "{}: the catalogue has {id}", m.id);
                }
                None => {
                    assert!(m.no_model.len() > 40, "{}: no model needs its reason in words", m.id);
                    assert!(crate::printer_models::machine(m.id).is_none(), "{}: a machine with no model is not in the model catalogue", m.id);
                }
            }
            assert!(m.nozzle > 0.0 && m.bed.is_none_or(|b| b[0] > 50.0 && b[1] > 50.0), "{}", m.id);
        }
        // The sizes the PrusaLink driver used to carry in its own list.
        for (id, bed) in [("prusa_core_one_l", [300.0, 300.0]), ("prusa_core_one", [250.0, 220.0]), ("prusa_xl", [360.0, 360.0]), ("prusa_mk4s", [250.0, 210.0]), ("prusa_mk3s", [250.0, 210.0]), ("prusa_mini", [180.0, 180.0])] {
            assert_eq!(by_id(id).unwrap().bed, Some(bed), "{id}");
        }
    }

    #[test]
    fn the_table_is_what_the_page_reads() {
        let v = json();
        let rows = v["machines"].as_array().unwrap();
        assert_eq!(rows.len(), MACHINES.len());
        let l = rows.iter().find(|r| r["id"] == "prusa_core_one_l").unwrap();
        assert_eq!((l["slicer_model"].as_str(), l["nozzle_mm"].as_f64(), l["bed_mm"][0].as_f64()), (Some("COREONEL"), Some(0.4), Some(300.0)));
        assert!(l["no_model"].is_null() && l["cad_model"] == "prusa_core_one_l");
        let xl = rows.iter().find(|r| r["id"] == "prusa_xl").unwrap();
        assert!(xl["cad_model"].is_null() && xl["no_model"].as_str().unwrap().contains("no assembly"), "{xl}");
    }
}
