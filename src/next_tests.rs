//! Stella Scope NEXT: served-bytes regression markers for the rebuilt
//! HUD. Kept apart from regression_tests.rs so the NEXT orders never collide with the old HUD's.

/// N01: every NEXT asset is served with its type, and `/` still serves the old HUD until the user
/// approves the switch.
#[test]
fn next_routes_are_served_beside_the_old_hud() {
    for (path, mime) in [
        ("/next", "text/html; charset=utf-8"),
        ("/next/", "text/html; charset=utf-8"),
        ("/next.css", "text/css; charset=utf-8"),
        ("/next.js", "text/javascript; charset=utf-8"),
        ("/next-ui.js", "text/javascript; charset=utf-8"),
        ("/next-core.js", "text/javascript; charset=utf-8"),
        ("/next-gesture.js", "text/javascript; charset=utf-8"),
        ("/next-pages.js", "text/javascript; charset=utf-8"),
        ("/next-icons.svg", "image/svg+xml"),
    ] {
        let r = crate::api::asset("GET", path).unwrap_or_else(|| panic!("{path} is not served"));
        assert_eq!(r.code, 200, "{path}");
        assert_eq!(r.mime, mime, "{path}");
    }
    let classic = String::from_utf8(crate::api::asset("GET", "/classic-hud").unwrap().body).unwrap();
    assert!(classic.contains("/service.js"), "/classic-hud no longer serves the previous HUD");
    let root = String::from_utf8(crate::api::asset("GET", "/").unwrap().body).unwrap();
    assert!(root.contains("src=\"/next.js\""), "/ does not open NEXT");
    let next = String::from_utf8(crate::api::asset("GET", "/next").unwrap().body).unwrap();
    for landmark in ["id=\"nx-scene\"", "id=\"nx-dock\"", "id=\"nx-estop\"", "src=\"/next.js\""] {
        assert!(next.contains(landmark), "/next is missing {landmark}");
    }
}

/// N03: the icon sprite carries every id the design names.
#[test]
fn next_icon_sprite_has_every_blueprint_icon() {
    let svg = String::from_utf8(crate::api::asset("GET", "/next-icons.svg").unwrap().body).unwrap();
    for id in [
        "home", "level", "mesh", "move", "heat", "bed", "fan", "files", "print", "pause", "play",
        "stop-square", "cancel", "exclude", "macro", "console", "motors-off", "restart",
        "fw-restart", "speed", "flow", "filament-load", "filament-unload", "probe", "estop", "more",
        "view", "light", "colors", "panels", "fullscreen", "close", "back", "live", "rewind",
        "step-back", "step-fwd", "link", "check", "warn", "arrow-up", "arrow-down", "arrow-left",
        "arrow-right", "minus", "plus", "z-up", "z-down", "nav-hud", "nav-signals", "nav-parts", "nav-electronics",
        "nav-diagnostics", "nav-maintenance", "nav-timeline", "nav-thermal", "nav-leveling",
        "nav-assembly", "nav-workspace", "nav-investigate",
    ] {
        assert!(
            svg.contains(&format!("<symbol id=\"i-{id}\" viewBox=\"0 0 24 24\">")),
            "icon i-{id} is missing"
        );
    }
}

/// Real gauges for speed/flow, thermal cards with graphs, the G-code command bar, dock sub-menus,
/// and the hotend as photographed (white sock, brass nozzle).
#[test]
fn next_review_round_two_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (ui, js, core, scene) = (text("/next-ui.js"), text("/next.js"), text("/next-core.js"), text("/hud-scene.js"));
    for (file, marker, what) in [
        (&ui, "export function gauge(", "round instrument gauge"),
        (&ui, "export function thermoCard(", "thermal card with graph"),
        (&ui, "export function commandBar(", "G-code command bar"),
        (&ui, "export function openPopover(", "dock sub-menu"),
        (&js, "ui.cmd=commandBar(", "command bar mounted"),
        (&js, "if(a.startsWith('menu:'))", "dock menus wired"),
        (&core, "export const SUB_MENUS=", "HOME/LEVEL sub-menus"),
        (&scene, "'head.nozzle':['#c9a24b'", "brass nozzle"),
    ] {
        assert!(file.contains(marker), "{what} missing");
    }
}

/// Instrument style lab: served so the user can try the three candidate styles on their own
/// machine.
#[test]
fn next_lab_is_served() {
    for p in ["/next/lab", "/next-lab.js", "/next-lab.css"] {
        assert_eq!(crate::api::asset("GET", p).unwrap().code, 200, "{p}");
    }
}

/// Style A knob (default) and B bar share one control; speed and flow click into detents with
/// 100% at the top; the choice is a setting.
#[test]
fn next_controls_knob_and_bar() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (ui, js) = (text("/next-ui.js"), text("/next.js"));
    assert!(ui.contains("export function control({style='knob'"));
    assert!(js.contains("detents:[50,75,100,125,150]") && js.contains("detents:[90,95,100,105,110]"));
    assert!(js.contains("localStorage.setItem('nx-inst',v)"));
    assert!(js.contains("section('Instrument style'"));
}

/// Round three: part panels instead of a toast, the bed's mesh as a map and
/// a 3D surface, file details with the slicer, rail pages as NEXT sheets, the knob/bar flip on the
/// clusters, dock buttons that close what they opened, Z beside the nozzle, homing drawn in the
/// real frame, and the Y cable chain rolling instead of sliding in two halves.
#[test]
fn next_review_round_three_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages, core, scene) = (text("/next.js"), text("/next-ui.js"), text("/next-pages.js"), text("/next-core.js"), text("/hud-scene.js"));
    for (file, marker, what) in [
        (&js, "if(id)openPart(id);", "a click on a part opens its panel"),
        (&js, "homeMap(r.point.position", "ticks are taken out of the homing frame"),
        (&js, "btn?.hasAttribute('data-showing')", "a second press closes what the dock opened"),
        (&js, "ui.flips=", "knob/bar flip on the clusters"),
        (&js, "'nx-reticle'", "Z beside the nozzle"),
        (&js, "'sheet:level'", "rail opens the NEXT levelling page"),
        (&js, "openFile(path)", "a file row opens its details"),
        (&ui, "export function markOpener(", "dock opener mark"),
        (&pages, "export function meshViewer(", "mesh viewer"),
        (&pages, "function drawGrid(", "mesh grid view with values"),
        (&pages, "function draw3d(", "3D mesh surface"),
        (&pages, "export function openPart(", "part panels"),
        (&pages, "export function openFile(", "file details"),
        (&pages, "/api/print/file?filename=", "file details endpoint"),
        (&core, "export function homingFrame(", "homing frame mapper"),
        (&scene, "d.motion='chain'", "rolling cable chain"),
        (&scene, "ts-refusedSince<0.75", "a refused pose cannot freeze the head"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round four: live probing on the maps and the plate, mesh compare, build
/// sheets, sparklines, the mesh on the 3D bed, the live level card, the print page with the
/// toolpath back on the plate, kiosk mode, alerts, and the four classic pages rebuilt as NEXT
/// sheets; plus the longer drawn chain and the unhomed-Z tracking.
#[test]
fn next_review_round_four_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages, core, scene) = (text("/next.js"), text("/next-ui.js"), text("/next-pages.js"), text("/next-core.js"), text("/hud-scene.js"));
    for (file, marker, what) in [
        (&pages, "export function diffMesh(", "mesh compare"),
        (&pages, "live(points){", "live probe points on the mesh views"),
        (&pages, "function sheetPicker(", "build sheet picker"),
        (&pages, "export function spark(", "sparklines"),
        (&pages, "export function activityCard(", "live level / mesh card"),
        (&pages, "export function pagePrint(", "print page"),
        (&pages, "export function pageAlerts(", "alerts page"),
        (&pages, "export function pageCauses(", "suspected causes as a NEXT sheet"),
        (&pages, "export function pageJournal(", "service journal as a NEXT sheet"),
        (&pages, "export function pageAssembly(", "assembly as a NEXT sheet"),
        (&pages, "export function pageMotion(", "motion and motors as a NEXT sheet"),
        (&js, "showToolpath(name)", "toolpath drawn on the plate again"),
        (&js, "scene.live(r.point)", "heat glow and fan tint wired"),
        (&js, "function setKiosk(", "kiosk mode"),
        (&js, "scene.probeMarks(", "probe landings on the plate"),
        (&js, "scene.meshOverlay(", "mesh on the 3D bed"),
        (&ui, "tipFor.matches(':hover')", "a tooltip cannot outlive its control"),
        (&core, "un[i]={anchor:", "an unhomed axis is tracked from where it was"),
        (&scene, "'#added-'", "the drawn chain is long enough for full travel"),
        (&scene, "function meshOverlay(", "mesh overlay in the scene"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round five: the rear Z chain rolls with the gantry, the umbilical
/// leaves beside it, the fans turn, the BLTouch light blinks, and anything wrong pops up.
#[test]
fn next_review_round_five_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, pages, scene) = (text("/next.js"), text("/next-pages.js"), text("/hud-scene.js"));
    for (file, marker, what) in [
        (&scene, "motion==='zchain'", "rear Z chain rolls"),
        (&scene, "function addRotor(", "fan rotors"),
        (&scene, "function probeLed(", "BLTouch light"),
        (&js, "function paintProbeLed(", "BLTouch light state"),
        (&js, "function checkAlerts(", "problem cards"),
        (&pages, "Reset the BLTouch", "BLTouch reset from its panel"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round six: the compact bar style, a printer that is off shows
/// no fan speed, and the part fan is set by dragging its ring.
#[test]
fn next_review_round_six_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, css, pages) = (text("/next.js"), text("/next-ui.js"), text("/next.css"), text("/next-pages.js"));
    for (file, marker, what) in [
        // Round nine brought the bars back upright (one row a side); see next_round_nine_is_served.
        (&css, "cursor:ns-resize", "bars are sliders"),
        (&js, "const fanOf=f=>stale?{speed:null}:f", "no fan speed while the printer is off"),
        (&js, "onSet:v=>send('set_fan',{value:v})", "part fan ring sets the fan"),
        (&ui, "export function fanRing({label,onOpen,onSet=null", "settable fan ring"),
        (&pages, "function heaterFanEditor(", "hotend fan start temperature"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round seven: NEXT opens at "/", Z babystep, the filament wizard, print history, maintenance,
/// input shaper and belts, and the camera passed through as a real stream.
#[test]
fn next_round_seven_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages) = (text("/next.js"), text("/next-ui.js"), text("/next-pages.js"));
    for (file, marker, what) in [
        (&pages, "export function babystep(", "Z babystep"),
        (&pages, "export function pageFilament(", "filament wizard"),
        (&pages, "Hot. Push the", "a load asks before it feeds filament (an unload runs by itself once hot)"),
        (&pages, "export function pageHistory(", "print history"),
        (&pages, "export function pageMaintenance(", "maintenance reminders"),
        (&pages, "export function pageShaper(", "input shaper"),
        (&pages, "export function shaperResponse(", "shaper response curves"),
        (&pages, "export function pageCamera(", "camera page"),
        (&ui, "export function openModal(", "command review over the open page"),
        (&js, "openModal({title:'Review command'", "review no longer replaces the page"),
        (&js, "'sheet:shaper'", "input shaper on the rail"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round nine/ten: compact bars stand upright and stack exactly like the knobs, dock choices
/// open as radials, and the phone has its own page on the rail.
#[test]
fn next_round_nine_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, css) = (text("/next.js"), text("/next-ui.js"), text("/next.css"));
    for (file, marker, what) in [
        (&ui, "drag.v+(drag.y-e.clientY)/r.height", "bars slide up and down"),
        (&js, "value:stale?null:r.nozzle.target", "a printer that is off shows no set values"),
        (&js, "function quickRadial(", "dock presets on a radial"),
        (&js, "'sheet:phone'", "phone page on the rail"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}
