//! Stella Scope NEXT (docs/next-ui/BLUEPRINT.md): served-bytes regression markers for the rebuilt
//! HUD. Kept apart from regression_tests.rs so the NEXT orders never collide with the old HUD's.

/// N01: every NEXT asset is served with its type, and `/` still serves the old HUD until the owner approves
/// the switch (BOARD N14).
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
    let classic =
        String::from_utf8(crate::api::asset("GET", "/classic-hud").unwrap().body).unwrap();
    assert!(
        classic.contains("/service.js"),
        "/classic-hud no longer serves the previous HUD"
    );
    let root = String::from_utf8(crate::api::asset("GET", "/").unwrap().body).unwrap();
    assert!(root.contains("src=\"/next.js\""), "/ does not open NEXT");
    let next = String::from_utf8(crate::api::asset("GET", "/next").unwrap().body).unwrap();
    for landmark in [
        "id=\"nx-scene\"",
        "id=\"nx-dock\"",
        "id=\"nx-estop\"",
        "src=\"/next.js\"",
    ] {
        assert!(next.contains(landmark), "/next is missing {landmark}");
    }
}

/// N03: the icon sprite carries every id BLUEPRINT section 7 names.
#[test]
fn next_icon_sprite_has_every_blueprint_icon() {
    let svg = String::from_utf8(crate::api::asset("GET", "/next-icons.svg").unwrap().body).unwrap();
    for id in [
        "home",
        "level",
        "mesh",
        "move",
        "heat",
        "bed",
        "fan",
        "files",
        "print",
        "pause",
        "play",
        "stop-square",
        "cancel",
        "exclude",
        "macro",
        "console",
        "motors-off",
        "restart",
        "fw-restart",
        "speed",
        "flow",
        "filament-load",
        "filament-unload",
        "probe",
        "estop",
        "more",
        "view",
        "light",
        "colors",
        "panels",
        "fullscreen",
        "close",
        "back",
        "live",
        "rewind",
        "step-back",
        "step-fwd",
        "link",
        "check",
        "warn",
        "arrow-up",
        "arrow-down",
        "arrow-left",
        "arrow-right",
        "minus",
        "plus",
        "z-up",
        "z-down",
        "nav-hud",
        "nav-signals",
        "nav-parts",
        "nav-electronics",
        "nav-diagnostics",
        "nav-maintenance",
        "nav-timeline",
        "nav-thermal",
        "nav-leveling",
        "nav-assembly",
        "nav-workspace",
        "nav-investigate",
    ] {
        assert!(
            svg.contains(&format!("<symbol id=\"i-{id}\" viewBox=\"0 0 24 24\">")),
            "icon i-{id} is missing"
        );
    }
}

/// NEXT review 2026-09-24 (the owner): real gauges for speed/flow, thermal cards with graphs, the G-code
/// command bar, dock sub-menus, and the hotend as photographed (white sock, brass nozzle).
#[test]
fn next_review_round_two_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (ui, js, core, scene) = (
        text("/next-ui.js"),
        text("/next.js"),
        text("/next-core.js"),
        text("/hud-scene.js"),
    );
    for (file, marker, what) in [
        (&ui, "export function gauge(", "round instrument gauge"),
        (
            &ui,
            "export function thermoCard(",
            "thermal card with graph",
        ),
        (&ui, "export function commandBar(", "G-code command bar"),
        (&ui, "export function openPopover(", "dock sub-menu"),
        (&js, "ui.cmd=commandBar(", "command bar mounted"),
        (&js, "if(a.startsWith('menu:'))", "dock menus wired"),
        (&core, "export const SUB_MENUS=", "HOME/LEVEL sub-menus"),
        (&scene, "color:0xe6e0d3", "white silicone sock"),
        (&scene, "'head.nozzle':['#c9a24b'", "brass nozzle"),
        (&scene, "BLOWER_BACK_MM=5", "4010s recessed"),
    ] {
        assert!(file.contains(marker), "{what} missing");
    }
}

/// Instrument style lab: served so the owner can try the three candidate styles on his own machine.
#[test]
fn next_lab_is_served() {
    for p in ["/next/lab", "/next-lab.js", "/next-lab.css"] {
        assert_eq!(crate::api::asset("GET", p).unwrap().code, 200, "{p}");
    }
}

/// UI board U01 (2026-10-20): the control kit from the approved LED design
/// (docs/design/knob-2026-10-20/src/led.html) is one module every page imports — the gauge at rest,
/// the knob that grows where it is and springs back, each control's own step and turn travel, the
/// click (sound and vibration) per detent, and the square key.
#[test]
fn next_ui_u01_control_kit_is_served() {
    for p in ["/next-control.js", "/next-control.css"] {
        assert_eq!(crate::api::asset("GET", p).unwrap().code, 200, "{p}");
    }
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, css) = (text("/next-control.js"), text("/next-control.css"));
    for (file, marker, what) in [
        (&js, "export const CONTROLS=", "the seven controls"),
        (
            &js,
            "min:0,max:290,step:5,turn:300",
            "the nozzle steps by 5 degrees",
        ),
        (
            &js,
            "min:0,max:120,step:1,turn:300",
            "the bed steps by 1 degree",
        ),
        (
            &js,
            "min:10,max:200,step:5,turn:300",
            "speed steps by 5 percent",
        ),
        (
            &js,
            "min:50,max:150,step:1,turn:300",
            "flow steps by 1 percent",
        ),
        (
            &js,
            "min:0,max:100,step:5,turn:270",
            "each fan steps by 5 percent",
        ),
        (&js, "SEGS=36", "36 chiclets"),
        (
            &js,
            "export function control(spec={})",
            "the control factory",
        ),
        (
            &js,
            "lift.classList.add('live')",
            "the control is drawn again, as a knob, when held (rc5's knob, kept)",
        ),
        (
            &js,
            "const HOLD_MS=220,MOVE_PX=6,HELD_PC=200,HELD_PHONE=250",
            "a hold is the knob at one size on a computer and one on a phone; a tap or a right click types the value",
        ),
        (
            &js,
            "d.acc=clamp(",
            "turning is scaled by the control's own travel",
        ),
        // U06 (2026-10-20): the snap now rounds to the step's own decimals (a 0.1 mm step left 0.30000000000000004); the line it asserted is replaced by this one, which still snaps to a whole step
        (&js, "Math.round(v/s)*s", "one click per step"),
        (
            &js,
            "navigator.vibrate(deep?14:4)",
            "a click is felt on a phone",
        ),
        (
            &js,
            "dim.id='nxctl-dim'",
            "the dim gathers under the held control",
        ),
        (&js, "export function key(spec={})", "the square key"),
        (&css, "border-radius:0", "the key is square-cornered"),
        (
            &css,
            "0 3px 0 rgba(0,0,0,.38)",
            "the key has depth under it",
        ),
        (
            &css,
            "cubic-bezier(.34,1.56,.64,1)",
            "the spring it grows and settles with",
        ),
        (&css, "repeating-conic-gradient", "the black anodised knob"),
        (
            &css,
            ".nxctl--lift .nxc-read{inset:0",
            "the word and the value are on the held knob itself (rc5)",
        ),
        (
            &css,
            "clamp(64px,calc((100vw - 70px)/4),86px)",
            "phone and PC are the same parts at two sizes",
        ),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// 2026-09-24: style A knob (default) and B bar shared one control; speed and flow clicked into detents with 100% at the top; the choice was a setting.
/// U02 (2026-10-20: a number is one piece, the LED gauge that becomes a knob when held) replaced both styles and the setting. The old dial stays exported for
/// the pages that still use it (next-ui.js); the printer page uses the kit, each gauge with its own step from the kit's CONTROLS table; nothing offers a knob/bar choice.
#[test]
fn next_controls_knob_and_bar() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (ui, js, kit) = (text("/next-ui.js"), text("/next.js"), text("/next-control.js"));
    assert!(ui.contains("export function control({style='knob'"));
    assert!(js.contains("NXC.find(c=>c.key===k)"), "every gauge takes its steps from the kit's table");
    assert!(kit.contains("key:'speed',label:'SPEED',unit:'%',min:10,max:200,step:5") && kit.contains("key:'flow',label:'FLOW',unit:'%',min:50,max:150,step:1"));
    assert!(!js.contains("localStorage.setItem('nx-inst',v)") && !js.contains("section('Instrument style'"), "no knob/bar choice any more");
}

/// NEXT review 2026-09-24, round three (the owner): part panels instead of a toast, the bed's mesh as a
/// map and a 3D surface, file details with the slicer, rail pages as NEXT sheets, the knob/bar flip
/// on the clusters, dock buttons that close what they opened, Z beside the nozzle, homing drawn in
/// the real frame, and the Y cable chain rolling instead of sliding in two halves.
#[test]
fn next_review_round_three_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages, core, scene) = (
        text("/next.js"),
        text("/next-ui.js"),
        text("/next-pages.js"),
        text("/next-core.js"),
        text("/hud-scene.js"),
    );
    for (file, marker, what) in [
        // U02 (2026-10-20): one click on the machine grows it to fill the screen, so the click no longer picks a part; a part's panel opens from the Parts page.
        (
            &js,
            "scene=createScene($('nx-scene'),id=>openPart(id),",
            "2026-10-24: a click on a part opens that part",
        ),
        (
            &js,
            "homeMap(r.point.position",
            "ticks are taken out of the homing frame",
        ),
        (
            &js,
            "btn?.hasAttribute('data-showing')",
            "a second press closes what the dock opened",
        ),
        // 2026-10-23: the GAUGES key is gone; the gauges always stand on the machine.
        (&js, "let gaugesFolded=false;", "the gauges are never folded away"),
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
        (
            &scene,
            "ts-refusedSince<0.75",
            "a refused pose cannot freeze the head",
        ),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// NEXT review 2026-09-24, round four (the owner: "do all that before I look at it"): live probing on the
/// maps and the plate, mesh compare, build sheets, sparklines, the mesh on the 3D bed, the live
/// level card, the print page with the toolpath back on the plate, kiosk mode, alerts, and the four
/// classic pages rebuilt as NEXT sheets; plus the longer drawn chain and the unhomed-Z tracking.
#[test]
fn next_review_round_four_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages, core, scene) = (
        text("/next.js"),
        text("/next-ui.js"),
        text("/next-pages.js"),
        text("/next-core.js"),
        text("/hud-scene.js"),
    );
    for (file, marker, what) in [
        (&pages, "export function diffMesh(", "mesh compare"),
        (
            &pages,
            "live(points){",
            "live probe points on the mesh views",
        ),
        (&pages, "function sheetPicker(", "build sheet picker"),
        (&pages, "export function spark(", "sparklines"),
        (
            &pages,
            "export function activityCard(",
            "live level / mesh card",
        ),
        (&pages, "export function pagePrint(", "print page"),
        (&pages, "export function pageAlerts(", "alerts page"),
        (
            &pages,
            "export function pageCauses(",
            "suspected causes as a NEXT sheet",
        ),
        (
            &pages,
            "export function pageJournal(",
            "service journal as a NEXT sheet",
        ),
        (
            &pages,
            "export function pageAssembly(",
            "assembly as a NEXT sheet",
        ),
        (
            &pages,
            "export function pageMotion(",
            "motion and motors as a NEXT sheet",
        ),
        (
            &js,
            "showToolpath(name)",
            "toolpath drawn on the plate again",
        ),
        (&js, "scene.live(r.point)", "heat glow and fan tint wired"),
        (&js, "function setKiosk(", "kiosk mode"),
        (&js, "scene.probeMarks(", "probe landings on the plate"),
        (&js, "scene.meshOverlay(", "mesh on the 3D bed"),
        (
            &ui,
            "tipFor.matches(':hover')",
            "a tooltip cannot outlive its control",
        ),
        (
            &core,
            "un[i]={anchor:",
            "an unhomed axis is tracked from where it was",
        ),
        (
            &scene,
            "'#added-'",
            "the drawn chain is long enough for full travel",
        ),
        (&scene, "function meshOverlay(", "mesh overlay in the scene"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// NEXT review 2026-09-24, round five (the owner): the rear Z chain rolls with the gantry, the umbilical
/// leaves beside it, the fans turn, the BLTouch light blinks, and anything wrong pops up.
#[test]
fn next_review_round_five_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, pages, scene) = (
        text("/next.js"),
        text("/next-pages.js"),
        text("/hud-scene.js"),
    );
    for (file, marker, what) in [
        (&scene, "motion==='zchain'", "rear Z chain rolls"),
        (
            &scene,
            "zChain.wc+zChain.R+26",
            "umbilical ends beside the Z chain",
        ),
        (&scene, "function addRotor(", "fan rotors"),
        (&scene, "function probeLed(", "BLTouch light"),
        (&js, "function paintProbeLed(", "BLTouch light state"),
        (&js, "function checkAlerts(", "problem cards"),
        (&pages, "Reset the BLTouch", "BLTouch reset from its panel"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// NEXT review 2026-09-24, round six (the owner): the compact bar style, a
/// printer that is off shows no fan speed, and the part fan is set by dragging its ring.
#[test]
fn next_review_round_six_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, css, pages) = (
        text("/next.js"),
        text("/next-ui.js"),
        text("/next.css"),
        text("/next-pages.js"),
    );
    for (file, marker, what) in [
        // Round nine brought the bars back upright (one row a side); see next_round_nine_is_served.
        (&css, "cursor:ns-resize", "bars are sliders"),
        (
            &js,
            "const fanOf=f=>stale||!f||f.speed===null||f.speed===undefined?{value:null}:",
            "no fan speed while the printer is off",
        ),
        (
            &js,
            "gauge('part',{wire:v=>send('set_fan',{value:v/100})",
            "part fan gauge sets the fan",
        ),
        (
            &ui,
            "export function fanRing({label,onOpen,onSet=null",
            "settable fan ring",
        ),
        (
            &pages,
            "function heaterFanEditor(",
            "hotend fan start temperature",
        ),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Round seven (2026-09-24): NEXT opens at "/", Z babystep, the filament wizard, print history,
/// maintenance, input shaper and belts, and the camera passed through as a real stream.
#[test]
fn next_round_seven_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, pages) = (
        text("/next.js"),
        text("/next-ui.js"),
        text("/next-pages.js"),
    );
    for (file, marker, what) in [
        (&pages, "export function babystep(", "Z babystep"),
        (&pages, "export function pageFilament(", "filament wizard"),
        (&pages, "Hot. Push the", "a load asks before it feeds filament (an unload runs by itself once hot, 2026-09-25)"),
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

/// Round nine/ten (2026-09-25): compact bars stand upright and stack exactly like the knobs,
/// dock choices open as radials, and the phone has its own page on the rail.
#[test]
fn next_round_nine_is_served() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (js, ui, css) = (text("/next.js"), text("/next-ui.js"), text("/next.css"));
    for (file, marker, what) in [
        (
            &ui,
            "drag.v+(drag.y-e.clientY)/r.height",
            "bars slide up and down",
        ),
        (
            &css,
            "the compact style keeps the round gauges' layout exactly",
            "bars stack like the knobs",
        ),
        (
            &js,
            "value:stale?null:r.nozzle.target",
            "a printer that is off shows no set values",
        ),
        (&js, "function quickRadial(", "dock presets on a radial"),
        (&js, "'sheet:phone'", "phone page on the rail"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// UI board U02 (2026-10-20): the printer page is on the LED controls (next-control.js). Nozzle, bed, speed, flow and every fan are the kit's gauges, each sending its
/// existing command once, on release, through the page's own review; the gauges fold into one bottom bar that holds the position and the square keys; the machine
/// (or the camera) is the whole background and one click grows it; over the camera the glass is light; and Back to the farm is a key at the top left.
#[test]
fn next_ui_u02_printer_page_is_on_the_kit() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    assert_eq!(crate::api::asset("GET", "/next-stage.js").unwrap().code, 200);
    let (html, js, css, stage, kit, kit_css) = (
        text("/next"),
        text("/next.js"),
        text("/next.css"),
        text("/next-stage.js"),
        text("/next-control.js"),
        text("/next-control.css"),
    );
    for (file, marker, what) in [
        (&html, "href=\"/next-control.css\"", "the kit's stylesheet is on the page"),
        (&html, "id=\"nx-gauges\"", "one wrapper for the gauges, so a phone can lay them out four across"),
        (&html, "class=\"nx-keyrow\"", "the one bar holds the keys"),
        (&js, "import {control as nxc,CONTROLS as NXC,key as nxkey,click as nxclick} from './next-control.js'", "the page imports the kit, not its own dial (and its click, for the hold keys)"),
        (&js, "gauge('nozzle',{wire:v=>send('set_heater',{target:'extruder',value:v})", "the nozzle sends the heater command on release, through send() and its review"),
        (&js, "gauge('bed',{wire:v=>send('set_heater',{target:'heater_bed',value:v})", "the bed"),
        (&js, "gauge('speed',{wire:v=>send('speed_factor',{value:v})", "speed"),
        (&js, "gauge('flow',{wire:v=>send('extrude_factor',{value:v})", "flow"),
        (&js, "gauge('part',{wire:v=>send('set_fan',{value:v/100})", "the part fan takes a fraction, the kit gives a percent"),
        (&js, "gauge('hotend',{readOnly:true", "the hotend fan is a gauge that does not turn (Klipper runs it)"),
        (&js, "gauge('board',{readOnly:true", "the controller fan too"),
        (&js, "const fanOf=f=>stale||!f||f.speed===null||f.speed===undefined?{value:null}:", "a printer that is off shows no fan speed"),
        (&js, "function renderBar(", "the keys are the kit's square keys in the one bar"),
        (&js, "function foldGauges(", "the gauges fold into the bar"),
        (&js, "the GAUGES key is gone", "no Gauges key (2026-10-23)"),
        (&js, "ui.back=nxkey({label:", "Back to the farm is a key"),
        (&js, "location.href='/farm'", "and it goes to the farm"),
        (&js, "scene=createScene($('nx-scene'),id=>openPart(id),", "2026-10-24: a click on a part opens that part; a click on empty space does nothing"),
        (&js, "initStage({scene:()=>scene", "the stage's click, zoom and reset are wired"),
        (&js, "function show3d(){", "the 3D machine on every device"),
        (&js, "id='nx-bgcam'", "Camera fills the whole background"),
        (&stage, "export function grow()", "one click grows"),
        (&stage, "export function shrink()", "and shrinks back"),
        (&stage, "const GROW=.8,", "grown is the camera at four fifths of its distance"),
        (&stage, "Math.exp(-e.deltaY*.0015)", "the wheel zooms the camera picture"),
        (&stage, "e.key==='Escape'", "Escape shrinks"),
        (&stage, "reset()", "a double click resets"),
        (&css, "body[data-grown]", "everything but the state chip and STOP falls away while grown"),
        (&css, "body[data-bg=camera]{--glass-1:rgba(255,255,255,", "over the camera the glass is light"),
        (&css, "#nx-bgcam{position:fixed;inset:0", "the camera picture is the whole background"),
        (&css, "body[data-gauges=folded]", "folded gauges leave the screen"),
        (&css, "grid-template-columns:repeat(4,1fr)", "on a phone the gauges are one bank, four across"),
        (&css, ".nxkey[data-kind=primary]", "the main key of a state is lit"),
        (&kit, "const setUnknown=", "a printer that is off reads a dash, not a value"),
        (&kit, "const setLocked=", "a refused command locks its gauge"),
        (&kit, "c.readOnly", "an automatic fan is a gauge that does not turn"),
        (&kit_css, ".nxctl[data-locked]", "a locked gauge looks locked"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
    // nothing of the old arrangement is left: no pager, no corner arrows, no knob/bar switch
    assert!(!html.contains("nx-pager"), "the phone pager is gone");
    assert!(!js.contains("nx-flip") && !js.contains("setInstStyle"), "no corner arrows, no knob/bar switch");
    // the pill for the position lives in the bar, not as a second glass capsule
    assert!(!js.contains("pos.classList.add('nx-glass')"));
    // and no little arrow in the corner of a key that opens something (2026-10-20): the pressed key says what is open
    assert!(!js.contains("el('i','nx-caret')"));
}

/// UI board U03 (2026-10-20), the bugs the owner hit on the 20th, each at its root:
/// a. the printer page waited on a chain (the 124 MB frame, then the toolhead's parts one by one, then a 0.9 s poll before it was shown) and downloaded it all again every time;
/// d. the phone's header ran off the screen (the failure-watch chip and the state chip were wider than the row);
/// f. the readings in the idle strip sat 4 px under the middle of their pill (it kept a hidden second row by padding its top).
#[test]
fn next_ui_u03_load_header_and_readings_are_fixed_at_the_root() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    let (scene, js, css) = (text("/hud-scene.js"), text("/next.js"), text("/next.css"));
    for (file, marker, what) in [
        (&scene, "const partFetch=new Map()", "the toolhead's parts are asked for beside the frame"),
        (&scene, "for(const id of TOOLHEAD_PARTS)", "all twelve of them, as soon as the library list is known"),
        (&scene, "async function assetJson(a,fail)", "geometry goes through the cache"),
        (&scene, "caches.open('stella-assets-v1')", "in the browser's Cache Storage, which holds what the HTTP cache cannot"),
        (&scene, "'/__stella-asset/'+encodeURIComponent(a.id)+'/'+sha", "keyed by the asset and its sha: a new import is a new key"),
        (&scene, "if(!tileMode&&sha&&typeof caches!=='undefined')", "the farm wall's tiles do not keep copies"),
        (&js, "document.addEventListener('stella-geometry-ready',()=>whenLoaded(),{once:true})", "the machine is shown when it is ready, not when a poll next looks"),
        (&js, "setTimeout(()=>whenLoaded(start),250)", "and the failure poll looks every 250 ms"),
        (&css, "body[data-layout=phone] #nx-eye .nx-lbl{display:none}", "the watch chip is its icon on a phone"),
        (&css, "body[data-layout=phone] .nx-state-label{min-width:0;overflow:hidden;text-overflow:ellipsis", "the state gives way with an ellipsis"),
        (&css, ".nx-idle{padding:0 8px;row-gap:0}", "the readings are centred in their pill"),
        (&css, ".nx-idle-item{height:42px}", "each item fills the pill's height"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// UI board U06 (2026-10-20): the printer's own pages are in the LED language. Their sheet bodies carry `.nx-u06` and nothing else changes (the stylesheet is scoped to it),
/// a number that is set is the kit's control (Heat and fans: nozzle, bed and the three fans; the part panels; the Maintenance interval; the hotend fan's start temperature),
/// the jog wheel and the Recording shuttle are the same relative wheel (a three-way switch of square keys for one click, one command on release, back to rest), and the
/// kit snaps a 0.1 step without float noise.
#[test]
fn next_ui_u06_the_printers_pages_are_on_the_kit() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).unwrap().body).unwrap();
    assert_eq!(crate::api::asset("GET", "/next-pages.css").unwrap().mime, "text/css; charset=utf-8");
    let (html, pages, css, js, kit) = (text("/next"), text("/next-pages.js"), text("/next-pages.css"), text("/next.js"), text("/next-control.js"));
    for (file, marker, what) in [
        (&html, "href=\"/next-pages.css\"", "the pages' stylesheet is on the page"),
        (&pages, "import {control as kctl,key as nkey,CONTROLS as KC,click as kclick} from './next-control.js'", "the pages import the kit, not their own dial"),
        (&pages, "body.classList.add('nx-u06')", "every page's sheet body carries the scope"),
        (&pages, "export function lift(c)", "a held control steps out of the scrolling sheet"),
        (&pages, "const c=ctl(which,{value:0,now:null,onSet:v=>{if(v>lim){v=lim;", "Heat and fans: the nozzle and bed targets are kit controls, sent once, clamped to the service's limit"),
        (&pages, "ctx.send('set_heater',{target:id,value:v})", "and they send the heater command every page sends"),
        (&pages, "ctl('part',{value:0,onSet:v=>ctx.send('set_fan',{value:v/100})})", "the part fan takes a fraction, the kit gives a percent"),
        (&pages, "ctl('hotend',{value:0,readOnly:true", "the hotend fan is a gauge that does not turn (Klipper runs it)"),
        (&pages, "ctl('board',{value:0,readOnly:true", "the controller fan too"),
        (&pages, "export function relWheel(", "one relative wheel for the jog wheel and the shuttle"),
        (&pages, "onSet:v=>{c.spec.value=0;c.paint();if(v)send(v,cur);}", "it springs back to rest and sends once"),
        (&pages, "step:.1,span:5,turn:300", "one click is 0.1 mm"),
        (&pages, "step:1,span:25,turn:300", "1 mm"),
        (&pages, "step:10,span:50,turn:300", "10 mm: at most the service's 50 mm either way"),
        (&pages, "ctx.send('jog',{axis,value:Math.round(v*100)/100})", "the jog wheel moves the head with the existing jog command"),
        (&pages, "const P={pos:xyz('POSITION')", "the four readings of Motion are drawn again (they sat in a comment)"),
        (&pages, "body.append(statGrid(...Object.values(P)));", "on their own line"),
        (&pages, "ctl('maint',{label:'EVERY',unit:'h',min:10,max:600,step:10", "the Maintenance interval is a kit control, 10 hours a click"),
        (&pages, "ctl('hfs',{label:'STARTS AT'", "the hotend fan's start temperature is a kit control"),
        (&pages, "nx-lv-dir", "no little arrow under a gantry corner: the direction is a word"),
        (&pages, "if(cell>=38)for(let r=0;r<rows;r++)", "the mesh map writes no values into cells too small for them (a phone showed a smear)"),
        (&js, "function sheetRecording(){", "Recording"),
        (&js, "const sh=relWheel({label:'SHUTTLE',unit:'min'", "its shuttle is the same wheel (a minute, ten minutes, an hour a click)"),
        (&js, "u06(body);const list", "Macros are in the page language"),
        (&js, "u06(body);const log=el('div','nx-console')", "and the Console"),
        (&css, ".nx-u06 .nx-btn{", "the page's buttons are square keys with a depth edge"),
        (&css, ".nx-u06 .nx-btn:active", "a press eats the key's depth (corners: look A, 2026-10-23, in next-look.css)"),
        (&css, ".nx-u06 .nx-stat-v{", "a number that is read is an LED figure"),
        (&css, ".nx-u06 .nx-seg button", "tabs and choices are separate square keys"),
        (&css, ".nx-u06 .nx-filring::before", "the filament ring is LED chiclets"),
        (&css, ".nx-u06 .nx-jw{", "the jog wheel's deck"),
        (&kit, "const snap=(c,v)=>{const s=step(c),d=(String(s).split('.')[1]||'').length;", "the kit rounds a fractional step to its own decimals"),
        (&kit, "c.format?c.format(c.value)", "a page can give its own words for a value"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
    // nothing of the old arrangement is left: no old knob or fan ring in the pages, no corner arrows, no pills on a state
    assert!(!pages.contains("control({style:'knob'") && !pages.contains("fanRing(") && !pages.contains("stepper("), "the old dials are gone from the pages");
    assert!(!pages.contains("nx-lv-arrow"), "no corner arrow on a gantry corner");
    assert!(!pages.contains("roundRect"), "no rounded labels drawn on the mesh");
    assert!(!css.contains("999px"), "no pill");
}

/// UI board U05 + U07 (2026-10-20): nothing pops up. Every sheet and dialog grows out of the key or tile that opened it and folds back into it (next-ui.js, on the kit's
/// spring), the panel's keys are the kit's square key, a number that is set in a panel is the kit's gauge, a count is an LED figure, and the failure watch takes the screen
/// the way the alarm design shows it. Every message grows from where it belongs.
#[test]
fn next_ui_u05_u07_nothing_pops_up_and_the_alarm_takes_the_screen() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).expect(p).body).unwrap();
    for p in ["/next-frame.css", "/next-alarm.js"] {
        assert_eq!(crate::api::asset("GET", p).unwrap().code, 200, "{p}");
    }
    let (ui, kit, frame, alarm, fleet, core) = (text("/next-ui.js"), text("/next-control.js"), text("/next-frame.css"), text("/next-alarm.js"), text("/next-fleet.js"), text("/next-core.js"));
    let (farm_html, next_html, js_page) = (text("/farm"), text("/next"), text("/next.js"));
    for (file, marker, what) in [
        (&farm_html, "href=\"/next-frame.css\"", "the farm page loads the frame"),
        (&next_html, "href=\"/next-frame.css\"", "the printer page loads the frame"),
        (&ui, "export function grow(node,from)", "a panel grows out of a rectangle"),
        (&ui, "export function fold(node,to,after)", "and folds back into it"),
        (&ui, "cubic-bezier(.34,1.56,.64,1)", "on the kit's spring"),
        (&ui, "document.addEventListener('pointerdown',e=>noteKey(", "what was pressed last is known when a press closes the menu that held it"),
        (&ui, "host._home=open.home", "a sheet remembers the key it folds back into"),
        (&ui, "fold(host,homeRect(host._home)", "a sheet folds into it"),
        (&ui, "fold(card,homeRect(card._home)", "and so does a dialog"),
        (&ui, "markOpener(open.home.el)", "the key stays pressed while its sheet is open"),
        (&ui, "export function emerge(node)", "a full-screen view (the cameras) grows out of its key"),
        (&ui, "export function numberField(", "a number set in a panel is the kit's gauge"),
        (&ui, "export function led(value", "a count is an LED figure"),
        (&ui, "click(false);onPress(b)", "every key clicks"),
        (&js_page, "from:{x:r.left+r.width/2", "the menu's keys grow out of the key pressed"),
        (&ui, "export function toast(text,tone='info',opts={})", "a toast knows what it is about"),
        (&ui, "fold(t,where(),()=>t.remove())", "and folds back into it"),
        (&kit, "const views=[view(el)]", "a control held inside a panel is drawn on the page itself, never cut off by the panel"),
        (&kit, "const fmt=(c,v)=>(c.prefix||'')+v.toFixed(dp(c))", "a step of .05 shows to its own decimals, a price has its sign"),
        (&frame, ".nx-sheet,body[data-layout] .nx-sheet{transform:translateY(var(--drag,0px));transition:none}", "a sheet no longer slides in from the edge"),
        (&frame, "radial-gradient(circle at var(--ox,50%) var(--oy,50%)", "the dim under a dialog gathers where it grew from"),
        (&frame, ".nx-sheet .nx-btn,.nx-modal .nx-btn,", "a panel's keys are square keys"),
        (&frame, ".nx-numf .nxctl{--nxctl-size:var(--nsz,92px)}", "a gauge in a panel is smaller than on the printer page"),
        (&frame, ".nx-fig{", "the LED figure"),
        (&frame, ".nx-choice-caret{display:none}", "no little corner arrows"),
        (&frame, ".fl-alarm::after{", "the red wash at the edges"),
        (&frame, ".fl-alarm-box{position:absolute;border:3px solid var(--alarm)", "the box where the failure was seen"),
        (&frame, ".fl-alarm-key--pause{grid-column:1/-1}", "on a phone Pause it now takes the whole row"),
        (&alarm, "label:'Pause it now'", "the first answer"),
        (&alarm, "label:'Watch'", "the second"),
        (&alarm, "label:'It is fine'", "the third"),
        (&alarm, "grow(card,from)", "the alarm grows out of the printer's tile"),
        (&alarm, "fold(card,rectOf(homeOf(e))", "and folds back into it"),
        (&alarm, "body:JSON.stringify({id:'pause',confirm:false})", "Pause it now is the one command, sent once"),
        (&fleet, "if(e.kind==='failure_watch')raiseAlarm(e,{known,onWatch:watchHook})", "the failure watch takes the screen, everything else is a pop-up"),
        (&fleet, "grow(card,rectOf(homeOf(e)))", "a pop-up grows out of its printer"),
        (&core, "export function alarmBoxes(detail)", "the boxes the alarm draws"),
        (&core, "export function alarmWords(e,printer)", "the words it says"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
    // the old slide-in is gone with its closed-state offset
    assert!(!ui.contains("setTimeout(()=>{if(!host.dataset.open)host.hidden=true;},260)"), "a sheet no longer waits 260 ms and disappears");
    // nothing else in the alarm sends a command, and nothing is stopped without being asked
    assert!(!alarm.contains("cancel") && !alarm.contains("emergency"), "the alarm never cancels or stops anything");
}

/// 2026-10-22 (Stella): CANCEL in the paused bar did nothing and said nothing; Klipper got PAUSE and never CANCEL_PRINT. A key that has to be held says so from the first touch
/// (a line and a filling ring), says "not sent" when the hand lets go early, and a completed hold is itself the confirmation: it goes with confirm:true, without a review dialog,
/// and the HUD says "Print cancelled, heaters off" only when the printer's own state reads cancelled with both heater targets at 0.
#[test]
fn next_hold_keys_say_what_holding_does_and_a_cancel_is_confirmed_by_the_printer() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).expect(p).body).unwrap();
    let (gesture, core, ui, js, css) = (text("/next-gesture.js"), text("/next-core.js"), text("/next-ui.js"), text("/next.js"), text("/next.css"));
    for (file, marker, what) in [
        (&gesture, "onStart?.();frame=requestAnimationFrame(tick)", "a hold says when it starts"),
        (&core, "export const HOLD_NOT_SENT='Not sent. Hold the key until the ring is full.'", "an early release says nothing was sent"),
        (&core, "cancel:'Hold to cancel the print'", "the line above the cancel key"),
        (&core, "motors_off:'Hold to switch the motors off'", "and above motors off"),
        (&core, "words:'Print cancelled, heaters off'", "the printer's own state confirms the cancel"),
        (&ui, "export function holdLine(near,text,tone='',ms=0)", "one line above a held key"),
        (&ui, "onStart:()=>holdLine(b,holdWords(cmd,title||label))", "every hold key made by button() says what holding does"),
        (&ui, "onCancel:()=>holdLine(b,HOLD_NOT_SENT,'warn',3500)", "and says not sent on an early release"),
        (&js, "onStart:()=>holdLine(b,holdWords(s.cmd||s.key,s.title||s.label))", "the bar's hold keys too (cancel, motors off)"),
        (&js, "execute(id,{},true,slot.title||slot.label)", "a completed hold sends with confirm:true and no second review"),
        (&js, "cancelVerdict(state.readings,", "the HUD reads the printer's state before it says cancelled"),
        (&css, ".nx-holdline{", "the line is styled"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Filament board 2026-10-22 (the owner likes the load and unload and uses it; this makes it safer and finishes it). Before anything heats, Load and Unload ask what is being loaded or
/// unloaded: the material, with the nozzle temperature it implies (an unload starts on what is recorded as in the extruder), and nothing is sent until its key is pressed. After an
/// unload the heater stays on for a wait you set (2 minutes), then goes off by itself and says so. After a load it goes off at once and a few seconds later the head is shaken (only when
/// homed, never in a print, and not if the setting is off). Every step says what it is doing and has a way out. The shake is the service's own fixed command (src/commands.rs).
#[test]
fn next_filament_asks_first_waits_then_cools_and_shakes() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).expect(p).body).unwrap();
    let (pages, core, js, css) = (text("/next-pages.js"), text("/next-core.js"), text("/next.js"), text("/next.css"));
    for (file, marker, what) in [
        (&pages, "export function filPicker(kind,{auto=false}={})", "Load and Unload ask first"),
        (&pages, "let pick=load?(mats.some(m=>m.name===fil.mat)?fil.mat:null):(mats.some(m=>m.name===inName)?inName:null);", "an unload starts on what is in the extruder; a load has to be picked"),
        (&pages, "Nothing heats until you press the key below.", "and nothing heats before its key"),
        (&pages, "go.disabled=!m||nameClash();", "its key waits for a material (and a typed name that is not taken)"),
        (&pages, "if(!mats.length)pick=OTHER;", "a printer with no materials opens on OTHER, not a dead end"),
        (&pages, "materials:[...mats,{name:m.name,nozzle:m.nozzle,bed:m.bed}]", "a named OTHER is kept with the materials"),
        (&pages, "ctx.jsend('/api/filament/guard','POST',{off_in_s:Math.min(900,n)})", "the service's heat guard is armed while a hot step is open"),
        (&pages, "filStart(kind,m.name,m.nozzle,auto)", "the job starts only from that key"),
        (&pages, "onPress:()=>filPicker('load')", "the page's Load asks first"),
        (&pages, "onPress:()=>filPicker('unload')", "and so does its Unload"),
        (&pages, "fil.until=Date.now()+fil.waitMs;fil.step='waiting'", "an unload waits for a reload"),
        (&pages, "the heater went off by itself.", "then the heater goes off by itself and says so"),
        (&pages, "await ctx.send('set_heater',{target:'extruder',value:0})", "the nozzle heater is turned off through the same command path as every heater"),
        (&pages, "else await filCool();", "after a load the heater goes off at once"),
        (&pages, "fil.shake='pending';fil.shakeAt=Date.now()+S.pause*1000", "and the shake is booked a few seconds later"),
        (&pages, "ctx.send('filament_shake')", "the shake is the service's own command"),
        (&pages, "filamentShakeVerdict({enabled:S.shake,homed:ctx.state.readings.homed,mode:ctx.state.mode,allowed:ctx.can('filament_shake')})", "only homed, never in a print, and not if it is off"),
        (&pages, "const FIL_CANCEL={", "every step has a way out"),
        (&pages, "stat('IN THE EXTRUDER')", "what is recorded as in the extruder is shown"),
        (&pages, "label:'WAIT FOR RELOAD'", "the wait is a setting"),
        (&pages, "label:'SHAKE AFTER'", "the pause before the shake is a setting"),
        (&pages, "label:S.shake?'SHAKE ON':'SHAKE OFF'", "and the shake has an off switch"),
        (&core, "export const FILAMENT_DEFAULTS={wait:120,pause:4,shake:true};", "two minutes, four seconds, on"),
        (&core, "['waiting','Wait'],['cooling','Cool'],['done','Done']", "the unload's steps"),
        (&core, "['cooling','Cool'],['shaking','Shake'],['done','Done']", "the load's steps"),
        (&js, "await filamentQuick(a.slice(9));", "the dock's Load and Unload open the picker"),
        (&css, ".nx-filpick-heat{", "the picker's temperature readout"),
    ] {
        assert!(file.contains(marker), "{what}: {marker} not served");
    }
}

/// Look A (2026-10-23): the glass cockpit is one stylesheet loaded last on the printer page and the farm wall. It keeps the 10-20 layout (the sidebar,
/// the gauges on the machine, the dock) and changes the surface: the sidebar names its pages under their icons, each gauge stands on a solid holographic disc,
/// the panels are the same glass, the GAUGES key is gone and a right click on a gauge types its value.
#[test]
fn look_a_glass_cockpit_is_loaded_everywhere_and_keeps_the_layout() {
    let text = |p: &str| String::from_utf8(crate::api::asset("GET", p).expect(p).body).unwrap();
    assert_eq!(crate::api::asset("GET", "/next-look.css").unwrap().mime, "text/css; charset=utf-8");
    let (look, farm_html, next_html, kit, js) = (text("/next-look.css"), text("/farm"), text("/next"), text("/next-control.js"), text("/next.js"));
    for (file, marker, what) in [
        (&next_html, "<link rel=\"stylesheet\" href=\"/next-frame.css\">\n<link rel=\"stylesheet\" href=\"/next-look.css\">", "the printer page loads the look last"),
        (&farm_html, "<link rel=\"stylesheet\" href=\"/farm.css\">\n<link rel=\"stylesheet\" href=\"/next-look.css\">", "the farm wall loads the look last"),
        (&look, "#nx-rail .nx-btn[data-size=icon]::after{content:attr(aria-label)", "the sidebar names each page under its icon"),
        (&look, "solid slab of smoked crystal", "each gauge stands on a solid holographic disc"),
        (&look, "--glass-rim:inset 0 1px 0", "the glass rim is a real value (it referred to itself before, so every glass shadow was dropped)"),
        (&look, ".nx-sheet,.nx-modal-card{border-radius:26px", "the panels are the same glass"),
        (&kit, "el.addEventListener('contextmenu',e=>{e.preventDefault();", "a right click on a gauge types its value"),
        (&js, "let gaugesFolded=false;", "the gauges always stand on the machine"),
    ] {
        assert!(file.contains(marker), "{what}: missing {marker}");
    }
    assert!(!js.contains("nxkey({label:'GAUGES'"), "the GAUGES key is gone");
    assert!(!next_html.contains("next-shell.css"), "the rejected rebuild's shell is not loaded");
}
