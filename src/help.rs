//! Help: the user guide inside the program (finish-2 F07, 2026-10-19). `docs/USER-GUIDE.md` and its pictures are built into the binary, like every
//! page, so Help works with no internet and always matches the version it came with. The page (`web/help.html`, `web/help.js`) reads the guide as
//! plain text and builds it as page elements; the two notices About points to (`THIRD_PARTY.md`, `PRINTER-MODELS-NOTICE.md`) are read the same way.
use crate::api::Response;

const GUIDE: &str = include_str!("../docs/USER-GUIDE.md");
const THIRD_PARTY: &str = include_str!("../docs/THIRD_PARTY.md");
const MODELS: &str = include_str!("../docs/PRINTER-MODELS-NOTICE.md");
const CRATES_LINK: &str = include_str!("../docs/licences/crates-link.md");
const CRATES_DESKTOP: &str = include_str!("../docs/licences/crates-desktop.md");
/// The MIT licence of 3D Print Rig, with the project's own scope paragraph, as published in its repository (the notice About shows).
pub const RIG_NOTICE: &str = include_str!("../docs/licences/3D-Print-Rig-MIT.txt");

/// The guide's pictures: real screens of this version, taken in a browser (`docs/guide/`). A picture the guide names and this table lacks is a test failure.
static IMAGES: &[(&str, &[u8])] = &[
    ("fresh-wall.jpg", include_bytes!("../docs/guide/fresh-wall.jpg")),
    ("wall-demo.jpg", include_bytes!("../docs/guide/wall-demo.jpg")),
    ("more-menu.jpg", include_bytes!("../docs/guide/more-menu.jpg")),
    ("add-printer-find.jpg", include_bytes!("../docs/guide/add-printer-find.jpg")),
    ("add-printer-klipper.jpg", include_bytes!("../docs/guide/add-printer-klipper.jpg")),
    ("add-printer-prusa.jpg", include_bytes!("../docs/guide/add-printer-prusa.jpg")),
    ("add-printer-flashforge.jpg", include_bytes!("../docs/guide/add-printer-flashforge.jpg")),
    ("printer-page.jpg", include_bytes!("../docs/guide/printer-page.jpg")),
    ("command-bar.jpg", include_bytes!("../docs/guide/command-bar.jpg")),
    ("cameras.jpg", include_bytes!("../docs/guide/cameras.jpg")),
    ("queue.jpg", include_bytes!("../docs/guide/queue.jpg")),
    ("slicer-upload.jpg", include_bytes!("../docs/guide/slicer-upload.jpg")),
    ("printer-settings.jpg", include_bytes!("../docs/guide/printer-settings.jpg")),
    ("spools.jpg", include_bytes!("../docs/guide/spools.jpg")),
    ("alert-settings.jpg", include_bytes!("../docs/guide/alert-settings.jpg")),
    ("phone.jpg", include_bytes!("../docs/guide/phone.jpg")),
    ("licence-buy.jpg", include_bytes!("../docs/guide/licence-buy.jpg")),
    ("backup.jpg", include_bytes!("../docs/guide/backup.jpg")),
    ("about.jpg", include_bytes!("../docs/guide/about.jpg")),
    ("printer-offline.jpg", include_bytes!("../docs/guide/printer-offline.jpg")),
    ("no-model.jpg", include_bytes!("../docs/guide/no-model.jpg")),
];

pub fn asset(path: &str) -> Option<Response> {
    let text = |mime: &'static str, body: &str| Some(Response::text(200, mime, body));
    match path {
        "/help" | "/help/" => text("text/html; charset=utf-8", include_str!("../web/help.html")),
        "/help.js" => text("text/javascript; charset=utf-8", include_str!("../web/help.js")),
        "/help.css" => text("text/css; charset=utf-8", include_str!("../web/help.css")),
        "/help/guide.md" => text("text/markdown; charset=utf-8", GUIDE),
        "/help/third-party.md" => text("text/markdown; charset=utf-8", THIRD_PARTY),
        "/help/printer-models.md" => text("text/markdown; charset=utf-8", MODELS),
        "/help/crates-link.md" => text("text/markdown; charset=utf-8", CRATES_LINK),
        "/help/crates-desktop.md" => text("text/markdown; charset=utf-8", CRATES_DESKTOP),
        _ => {
            let name = path.strip_prefix("/help/img/")?;
            let (_, bytes) = IMAGES.iter().find(|(n, _)| *n == name)?;
            let mime = if name.ends_with(".png") { "image/png" } else { "image/jpeg" };
            Some(Response {
                code: 200,
                mime,
                body: bytes.to_vec(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(path: &str) -> String {
        String::from_utf8(asset(path).unwrap_or_else(|| panic!("{path} is not served")).body).unwrap()
    }
    /// The picture names the guide uses: `![caption](/help/img/name.jpg)`.
    fn pictures() -> Vec<String> {
        GUIDE
            .lines()
            .filter_map(|l| l.trim().strip_prefix("!["))
            .filter_map(|l| l.split_once("](/help/img/"))
            .filter_map(|(_, rest)| rest.split_once(')').map(|(n, _)| n.to_owned()))
            .collect()
    }

    #[test]
    fn help_and_what_it_shows_are_served() {
        for p in ["/help", "/help/", "/help.js", "/help.css", "/help/guide.md", "/help/third-party.md", "/help/printer-models.md", "/help/crates-link.md", "/help/crates-desktop.md"] {
            assert_eq!(asset(p).unwrap().code, 200, "{p}");
        }
        assert!(body("/help").contains("id=\"hp-doc\""));
        assert!(asset("/help/img/nope.jpg").is_none() && asset("/help/guide").is_none());
        assert!(body("/help/guide.md").starts_with("# "), "the guide opens with its title");
    }

    /// Every picture the guide shows is in the build, and every picture in the build is shown: no 404 in the guide, no dead weight in the binary.
    #[test]
    fn every_picture_in_the_guide_is_in_the_build_and_the_other_way_round() {
        let used = pictures();
        assert!(used.len() >= 10, "the guide shows real screens: {used:?}");
        let mut seen = std::collections::HashSet::new();
        for name in &used {
            assert!(seen.insert(name.clone()), "{name} is shown twice");
            let r = asset(&format!("/help/img/{name}")).unwrap_or_else(|| panic!("{name} is in the guide but not in the build"));
            assert!(r.body.len() > 2000, "{name} is a real picture");
            let magic: &[u8] = if name.ends_with(".png") { &[0x89, b'P', b'N', b'G'] } else { &[0xff, 0xd8, 0xff] };
            assert!(r.body.starts_with(magic), "{name} is not a {} file", name.rsplit('.').next().unwrap());
        }
        for (name, _) in IMAGES {
            assert!(used.iter().any(|u| u == name), "{name} is built in but the guide never shows it");
        }
        // two different screens are never the same picture
        let mut sums = std::collections::HashSet::new();
        for (name, bytes) in IMAGES {
            assert!(sums.insert(crate::sha256::hex(bytes)), "{name} is a copy of another picture");
        }
    }

    /// 2026-10-19: no e-Transfer address is ever shown publicly. The guide says what the licence is and who to write to, and never gives an
    /// address to send money to.
    #[test]
    fn the_licence_is_in_kens_words_and_names_no_address_to_pay() {
        let g = GUIDE.to_lowercase();
        assert!(g.contains("free for up to five printers"), "the free five");
        assert!(g.contains("perpetual"), "perpetual");
        assert!(g.contains("interac e-transfer") && g.contains("payment details come with the invoice"), "how it is paid");
        assert!(g.contains("info@3d3d.ca"), "who to write to");
        for bad in ["transfer to info@", "transfer: info@", "send an interac", "send the e-transfer to", "pay info@", "pay to info@"] {
            assert!(!g.contains(bad), "the guide gives an address to send money to: {bad}");
        }
        // and the notice About shows is the project's own
        assert!(RIG_NOTICE.contains("MIT License") && RIG_NOTICE.contains("Psych0h3ad"));
    }

    /// Plain words, as the rest of the app: no em dash, and none of the developer words the work order names.
    #[test]
    fn the_guide_is_plain() {
        assert!(!GUIDE.contains('\u{2014}'), "no em dash in the guide");
        let g = GUIDE.replace("stella-scope.json", "the settings file").to_lowercase(); // the settings file's own name is not a developer word to a person who must open it
        for bad in ["endpoint", "websocket", "payload", "json", "daemon", " stdout", "a driver", "the driver"] {
            assert!(!g.contains(bad), "developer word in the guide: {bad}");
        }
        // every section the work order names
        for h in ["install", "first start", "demo farm", "add a printer", "camera", "queue", "plate check", "spool", "alerts", "phone", "licence", "backup", "troubleshooting"] {
            assert!(GUIDE.lines().any(|l| l.starts_with("## ") && l.to_lowercase().contains(h)), "the guide has no section about {h}");
        }
    }
}
