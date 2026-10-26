use crate::{
    config::Config,
    model,
    qgl::Parser,
    store::{Event, Store},
};
use serde_json::json;
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    thread,
};

#[test]
fn retention_carries_map_health_and_recording_origin_forward() {
    let mut db = Store::open(":memory:", false).unwrap();
    let mut p = Parser::default();
    let geometry = model::geometry(&Config::default());
    db.write(
        Some((10.0, &json!({"raw":true}))),
        &[
            Event::new(1.0, "scope_geometry", geometry.to_string()),
            Event::new(2.0, "scope_health", json!({"connected":false}).to_string()),
            Event::new(1.0, "scope_origin", json!("demo").to_string()),
            Event::new(8.0, "scope_health", json!({"connected":true}).to_string()),
        ],
        &mut p,
    )
    .unwrap();
    db.prune(5.0, 4096, None).unwrap();
    assert_eq!(
        db.context_at(5.0, "scope_geometry").unwrap(),
        Some(geometry)
    );
    assert_eq!(
        db.context_at(6.0, "scope_health").unwrap().unwrap()["connected"],
        false
    );
    assert_eq!(
        db.context_at(9.0, "scope_health").unwrap().unwrap()["connected"],
        true
    );
    assert_eq!(
        db.context_at(9.0, "scope_origin").unwrap(),
        Some(json!("demo"))
    );
    db.prune(7.0, 4096, None).unwrap();
    assert_eq!(
        db.context_at(7.0, "scope_origin").unwrap(),
        Some(json!("demo"))
    );
    assert!(db.events(0.0, 20.0, 100).unwrap().is_empty());
}
#[test]
fn invalid_sample_clock_is_rejected_before_insertion() {
    let mut db = Store::open(":memory:", false).unwrap();
    let mut p = Parser::default();
    assert!(db.write(Some((f64::NAN, &json!({}))), &[], &mut p).is_err());
    assert!(db.write(Some((-1.0, &json!({}))), &[], &mut p).is_err());
    assert_eq!(db.counts().unwrap().0, 0);
}
fn read_request(request: &str) -> crate::Result<(String, String)> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let text = request.to_owned();
    let worker = thread::spawn(move || {
        let mut s = TcpStream::connect(address).unwrap();
        s.write_all(text.as_bytes()).unwrap();
    });
    let (mut stream, _) = listener.accept().unwrap();
    let result = crate::server::request_line(&mut stream, &Config::default());
    worker.join().unwrap();
    result
}
#[test]
fn http_parser_accepts_only_complete_origin_form_requests() {
    assert_eq!(
        read_request("GET /api/live HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap(),
        ("GET".into(), "/api/live".into())
    );
    assert!(read_request("GET http://printer/ HTTP/1.1\r\n\r\n").is_err());
    assert!(read_request("GET //printer/ HTTP/1.1\r\n\r\n").is_err());
    assert!(read_request("GET / HTTP/1.1\r\n").is_err());
}
#[test]
fn http_parser_rejects_oversized_headers() {
    let request = format!("GET / HTTP/1.1\r\nX-Fill: {}\r\n\r\n", "x".repeat(9000));
    assert!(read_request(&request).unwrap_err().contains("exceed"));
}
/// WO-09. Every `web/*` file is `include_str!`-ed, so a HUD change is only real once it is in the
/// binary. These three carry owner-reported defects and would regress silently: the pose guard that
/// drops Klipper's one-sample `live_position [0,0,0]`, the X beam and MGN12 carriage joints, and the
/// Testing modal. Served content, not the file on disk.
#[test]
fn hud_assets_carry_the_2026_09_21_owner_fixes() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    assert!(scene.contains("poseSkipped"), "pose guard is missing");
    assert!(
        scene.contains("PRODUCT_NAME_") && scene.contains("Gantry Extrusions"),
        "X carriage / X beam joints are missing"
    );
    let hud = crate::api::asset("GET", "/hud.js").expect("hud.js is served");
    let hud = String::from_utf8(hud.body).unwrap();
    assert!(
        hud.contains("hud-testing-modal"),
        "Testing modal is missing"
    );
    let css = crate::api::asset("GET", "/hud.css").expect("hud.css is served");
    let css = String::from_utf8(css.body).unwrap();
    assert!(
        css.contains(".hud-rail.left .hud-rail-row{flex-direction:column"),
        "stacked thermal rail is missing"
    );
}
/// WO-10. The ten things the owner called out while watching his calibration prints, asserted on the
/// served bytes for the same reason as above.
#[test]
fn hud_assets_carry_the_wo10_live_review_fixes() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        ("paintToolpath", "progressive toolpath ramp"),
        ("if(!l.extrude[i])continue", "travel moves are still drawn"),
        ("outside machine travel", "homing force-position guard"),
        ("function tweenTo", "motion tween"),
        ("beltSplit", "A/B belt gantry-run split"),
        ("g.rotateZ(Math.PI)", "4010 blowers turned 180 about Z"),
        ("126.95,111.7,74.68", "3010 recessed 3 mm further"),
    ] {
        assert!(
            scene.contains(marker),
            "{what} is missing from hud-scene.js"
        );
    }
    let hud = crate::api::asset("GET", "/hud.js").expect("hud.js is served");
    let hud = String::from_utf8(hud.body).unwrap();
    assert!(
        hud.contains("hud-ctrlfan") && !hud.contains("'CONTROLLER','controller'"),
        "controller fan is not off the cooling rail and in the header"
    );
    let css = crate::api::asset("GET", "/hud.css").expect("hud.css is served");
    let css = String::from_utf8(css.body).unwrap();
    assert!(
        css.contains(".hud-reticle-text{font:600 17px"),
        "the bigger red Z readout is missing"
    );
}
/// WO-11. our second look at the WO-10 build. The two that cost him a print to find are the
/// homing excursion and the jank, and both live on the live path in service.js rather than in the
/// scene: a pose the machine cannot be in must never reach the jitter buffer, because interpolating
/// through it manufactures poses that are inside the machine, and the guard must be told the
/// display clock the interpolated pose belongs to or its reach collapses to the 2 mm floor between
/// frames. Asserted on the served bytes.
#[test]
fn hud_assets_carry_the_wo11_second_look_fixes() {
    let core = crate::api::asset("GET", "/hud-core.js").expect("hud-core.js is served");
    let core = String::from_utf8(core.body).unwrap();
    assert!(
        core.contains("export function insideTravel"),
        "the shared travel-envelope test is missing"
    );
    let service = crate::api::asset("GET", "/service.js").expect("service.js is served");
    let service = String::from_utf8(service.body).unwrap();
    assert!(
        service.contains("if(insideTravel(r.point?.position,r.point?.limits)){tickBuffer.push(r)"),
        "an impossible pose can still be interpolated through"
    );
    assert!(
        service.contains("scene.set(liveSnapshot(pos,interpAt)"),
        "the interpolated pose is not timestamped with the display clock"
    );
    assert!(
        service.contains("'homed','limits','max_velocity','max_z_velocity'"),
        "the guard is still reading homed_axes and limits off the 5 s snapshot"
    );
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        ("reachZ=dt*", "Z has no reach of its own"),
        (
            "g.rotateX(Math.PI/2);if(side==='right')",
            "4010 outlet is not turned down",
        ),
        (
            "[94.05,120.4,48.6]",
            "BLTouch is not seated under the bracket plate",
        ),
        ("[127,128,139.4]", "extruder is not standing on its mount"),
        (
            "centreOn(g,[],[127,133,109.15])",
            "Sherpa mount is not lying flat on the cowl",
        ),
        ("stella:chc-v6-sock", "silicone sock is missing"),
        ("chainSplit", "Y cable chain is not split"),
        (
            "MISSING INSTALLED PART",
            "a missing installed part is still silent",
        ),
        (
            "filePosition",
            "the done ramp is not driven by the file position",
        ),
        ("fitFasteners", "the cowl has no fasteners"),
    ] {
        assert!(scene.contains(marker), "{what}");
    }
    let css = crate::api::asset("GET", "/hud.css").expect("hud.css is served");
    let css = String::from_utf8(css.body).unwrap();
    assert!(
        css.contains(".hud-rail.right .hud-rail-row{flex-direction:column"),
        "PART is not stacked over HOTEND"
    );
    assert!(
        !css.contains(".hud-reticle-ring"),
        "the crosshair over the head is still there"
    );
}

/// WO-12 + WO-13 (2026-09-22). The visual pass is in the scene bytes; the function additions are
/// in the HUD and the served CSS. Asserted on served content for the same reason as WO-09/10/11:
/// a HUD change is only real once it is in the binary.
#[test]
fn hud_assets_carry_the_wo12_wo13_fixes() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        ("PMREMGenerator", "studio environment map"),
        ("world.environment", "environment is applied to the scene"),
        ("familyFinish", "material family pass"),
        ("function stage(state)", "state staging on the scene API"),
        ("highlightObject", "exclude-object plate highlight"),
        (
            "GridHelper(Math.max(size.x,size.y)*1.15",
            "grid sized to the machine",
        ),
        ("studioEnv", "the softbox room"),
    ] {
        assert!(
            scene.contains(marker),
            "{what} is missing from hud-scene.js"
        );
    }
    let hud = crate::api::asset("GET", "/hud.js").expect("hud.js is served");
    let hud = String::from_utf8(hud.body).unwrap();
    for (marker, what) in [
        (
            "getScene()?.stage(sceneState)",
            "the HUD never stages the scene",
        ),
        ("lastLayerText", "layer readout is missing"),
        ("/api/history/prints", "print history is missing"),
        ("hud-file-thumb", "file thumbnails are missing"),
        ("Show on plate", "exclude-object tie-in is missing"),
    ] {
        assert!(hud.contains(marker), "{what} is missing from hud.js");
    }
    let css = crate::api::asset("GET", "/hud.css").expect("hud.css is served");
    let css = String::from_utf8(css.body).unwrap();
    for (marker, what) in [
        (
            "hud-print[data-result=\"complete\"]",
            "history result accents",
        ),
        ("hud-file-thumb", "thumbnail framing"),
        ("hud-object-row", "exclude-object row"),
        ("font:600 14px var(--sans)", "job strip headline weight"),
    ] {
        assert!(css.contains(marker), "{what} is missing from hud.css");
    }
    let service = crate::api::asset("GET", "/service.js").expect("service.js is served");
    let service = String::from_utf8(service.body).unwrap();
    assert!(
        service.contains("gap|notice|alarm"),
        "probe_alarm events are not coloured in the timeline"
    );
}

/// WO-14 (2026-09-22). A BLTouch retry is recorded as `probe_retry` and must colour in the orange
/// tier next to `probe_alarm` in the served timeline, never fall through to muted or magenta.
#[test]
fn hud_assets_carry_the_wo14_probe_retry_colour() {
    let service = crate::api::asset("GET", "/service.js").expect("service.js is served");
    let service = String::from_utf8(service.body).unwrap();
    assert!(
        service.contains("gap|notice|alarm|retry/.test(k)?'ev-orange'"),
        "probe_retry events are not coloured orange in the timeline"
    );
    // The findings list must not count Klipper's own retries as a probe failure.
    let core = crate::api::asset("GET", "/service-core.js").expect("service-core.js is served");
    assert!(
        String::from_utf8(core.body)
            .unwrap()
            .contains("e.kind!=='probe_retry'&&!/BLTouch.*retrying/i.test(e.message)"),
        "diagnose still counts BLTouch retry lines as a probe failure"
    );
}

/// WO-16 (2026-09-23). A Files row keeps its 44px thumbnail column only while the image is in it;
/// a file whose thumbnail fails to load (hud.js removes the image) lays out as if it never had
/// one, instead of the filename being caught in the empty 44px column. Served bytes.
#[test]
fn hud_assets_carry_the_wo16_files_row_without_a_thumbnail() {
    let css = crate::api::asset("GET", "/hud.css").expect("hud.css is served");
    let css = String::from_utf8(css.body).unwrap();
    assert!(
        css.contains(".hud-file:has(>.hud-file-thumb){grid-template-columns:44px 1fr"),
        "the thumbnail column is not conditional on the thumbnail"
    );
    assert!(
        !css.contains(".hud-file{grid-template-columns:44px 1fr"),
        "an unconditional 44px column squeezes the name of a file without a thumbnail"
    );
}

/// WO-17 (2026-09-23). our first look at the WO-12..16 toolhead. One marker per item, asserted
/// on the served bytes because a HUD change is only real once it is in the binary. Item 1 (the
/// Rapid Burner's back half) came later and has its own test below.
#[test]
fn hud_assets_carry_the_wo17_toolhead_fixes() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        (
            "BLTOUCH_FORWARD_MM=3",
            "item 2: the BLTouch nudge is not one named constant",
        ),
        (
            "BLTOUCH_AT[1]-BLTOUCH_FORWARD_MM",
            "item 2: the BLTouch does not come forward",
        ),
        (
            "BLOWER_BACK_MM=5", // NEXT review 2026-09-24: 2 -> 5 mm, the owner photo f31de800 (fans recessed behind the cowl lip)
            "item 3: the 4010 nudge is not one named constant",
        ),
        (
            "[x,121.7+BLOWER_BACK_MM,60.1]",
            "item 3: the 4010s do not go back",
        ),
        (
            "const PLATE_COLOR='#7b4fd6'",
            "item 4: the plate colour is not the purple named constant",
        ),
        (
            "{bed:PLATE_COLOR,",
            "item 4: the plate default does not use PLATE_COLOR",
        ),
        (
            "label:[r.x,-r.y,d.x,-d.y,",
            "item 5: cube labels are not mapped onto their own faces",
        ),
        // our answer on 2026-09-23 ("the tall tower points up") overrode item 7's orientation: the
        // carrier stands tower up and the board turns over with it, so these two markers follow him.
        (
            "ops:[rx(-Math.PI/2)],at:[127.34,181.15,128.76]",
            "item 6: the EBB42 is not on the rear of the carrier",
        ),
        (
            "SOURCE_SKIP={'btt-ebb42-v12':/^M3x20\\+6$/}",
            "item 6: BTT's motor standoffs still come with the board",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIER_AT);}",
            "item 7: the carrier is not tower up on the carriage's ear inserts and the Sherpa screws",
        ),
        (
            "const CARRIER_AT=[-20.70,168.06,43.35];",
            "item 7: the carrier is not at its measured tower-up position",
        ),
        (
            "canvas.dataset.orphans",
            "item 8: hardware of parts Stella does not have is still left floating",
        ),
        (
            "COWL_SCREW_Y=117.2",
            "item 9: the cowl screws are not seated in their counterbores",
        ),
    ] {
        assert!(scene.contains(marker), "{what}");
    }
    assert!(
        !scene.contains("'3010 upper left'"),
        "item 9: screws are still made at the 3010 corners, where the cowl has no holes"
    );
    let hud = crate::api::asset("GET", "/hud.js").expect("hud.js is served");
    let hud = String::from_utf8(hud.body).unwrap();
    assert!(
        hud.contains("label.setAttribute('transform',`matrix(${f.label"),
        "item 5: the cube does not draw its labels with the face matrix"
    );
    assert!(
        !hud.contains("label.setAttribute('x',f.center[0])"),
        "item 5: cube labels are still upright screen text at the face centre"
    );
}

/// WO-17 item 1 (2026-09-23). The back half behind the cowl: chirpy's MGN12H carriage stood on the
/// X block's measured holes with the short base under it, and the Afterburner frame it replaces
/// hidden only when the carriage is really in the library. Served bytes.
#[test]
fn hud_assets_carry_the_wo17_item1_carriage() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        (
            "const CARRIAGE_AT=[127,142.1,107.56],CARRIAGE_BASE_AT=[127,147.07,70.86];",
            "the carriage and its base are not at the measured block datum",
        ),
        (
            "placeSource('chirpy-mgn12h-x-carriage-lugs',",
            "the chirpy Lugs carriage is not placed",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIAGE_AT);}",
            "the carriage is not stood up onto the block's holes",
        ),
        (
            "if(carriage)paintBlack(await placeSource('chirpy-carriage-base-short',",
            "the short base is not placed under the carriage",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIAGE_BASE_AT);}",
            "the base is not on the carriage's bottom inserts",
        ),
        (
            "if(carriage)for(const m of meshes)",
            "the Afterburner frame would vanish even with no carriage in its place",
        ),
        (
            "/\\/ X Carriage:1 \\/ X Carriage Frame:1 \\//",
            "the hidden subtree is not the Afterburner X carriage frame",
        ),
        (
            "canvas.dataset.carriageFrame=JSON.stringify(frameHidden)",
            "the hidden frame ids are not recorded on the canvas",
        ),
    ] {
        assert!(scene.contains(marker), "{what}");
    }
}

/// WO-17 item 1, second pass (2026-09-23). The Rapid Burner's own hotend mount, downloaded on our
/// go, closes the chain from the X block to the cowl: the mount's two halves on the carriage, the V6
/// seated in their bore, the cowl on the clamp's inserts with everything measured off it, and the
/// extruder mount, Sherpa, carrier and board on top. Served bytes.
#[test]
fn hud_assets_carry_the_wo17_item1_hotend_mount_chain() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        (
            "const COWL_MOVE=[0,3.40,-1.24],",
            "the cowl is not moved onto the hotend clamp's inserts",
        ),
        (
            "g.translate(...onCowl(COWL_TRANSLATION))",
            "the cowl does not ride the chain",
        ),
        (
            "const REAR_AT=[-20.715,126.60,-40.49],FRONT_AT=[-20.715,121.10,-51.805];",
            "the hotend mount halves are not at the measured carriage datum",
        ),
        (
            "placeSource('chirpy-rapid-burner-v6-mount-rear',",
            "the mount's rear half is not placed",
        ),
        (
            "placeSource('chirpy-rapid-burner-v6-mount-front',",
            "the mount's front clamp is not placed",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...REAR_AT);}",
            "the rear half is not stood on the carriage's holes",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...FRONT_AT);}",
            "the clamp is not on the rear half's split face",
        ),
        (
            "const V6_SEAT=[0,-0.29,5.83],V6_AXIS=[0,-0.29,0];",
            "the V6 is not seated in the mount's bore",
        ),
        (
            "const SHERPA_MOVE=[1.69,10.05,-16.14];",
            "the Sherpa is not on its extruder mount",
        ),
        (
            "move:shift([0,0,4.00],SHERPA_MOVE)",
            "the Sherpa's box is not corrected for the plate it leaves out",
        ),
        // Since our answer (2026-09-23) the carrier and board are placed tower up, absolutely, rather
        // than carried by SHERPA_MOVE; the carrier's upper hole pair still lands on the Sherpa screws.
        (
            "at:[127.34,181.15,128.76],note:",
            "the EBB42 is not on the carrier's bosses",
        ),
        (
            "g.rotateX(-Math.PI/2);g.translate(...CARRIER_AT);",
            "the carrier does not stay on the Sherpa's frame screws",
        ),
        (
            "const MOUNT_MOVE=[0,0.205,-8.14];",
            "the extruder mount is not on the hotend mount's top",
        ),
        (
            "turnZInPlace(g);g.translate(...MOUNT_MOVE)",
            "the extruder mount is not turned onto its inserts",
        ),
        (
            "SOURCE_SKIP['annex-sherpa-mini']=/^NAUO26$/;",
            "the Sherpa's tabbed front plate still stands in the cowl",
        ),
        (
            "BLTOUCH_AT[2]],move:COWL_MOVE",
            "the BLTouch does not follow the cowl",
        ),
        (
            "at:[126.95,111.7,74.68],move:COWL_MOVE",
            "the 3010 does not follow the cowl",
        ),
        (
            "centreOn(g,[],onCowl([x,121.7+BLOWER_BACK_MM,60.1]))",
            "the 4010s do not follow the cowl",
        ),
        (
            "centreOn(g,[],onCowl([x,COWL_SCREW_Y,z]))",
            "the cowl screws do not follow the cowl",
        ),
        (
            "part.move?shift(part.move,part.at):part.at",
            "fitted parts ignore their move",
        ),
    ] {
        assert!(scene.contains(marker), "{what}");
    }
}

/// WO-18 (2026-09-23). our part cards: one part-to-signal map read by every card, no card for a
/// part with nothing wired, the bed's newest recorded mesh, and a readout grid with no background
/// left to show through as an empty card. Served bytes; the map itself is unit-tested in
/// scripts/service-core-tests.mjs.
#[test]
fn hud_assets_carry_the_wo18_part_cards() {
    let text =
        |path: &str| String::from_utf8(crate::api::asset("GET", path).expect(path).body).unwrap();
    let (core, hud, css, service) = (
        text("/hud-core.js"),
        text("/hud.js"),
        text("/hud.css"),
        text("/service.js"),
    );
    for (file, marker, what) in [
        (
            &core,
            "export const PART_WIRING=[",
            "item 1: there is no part-to-signal map",
        ),
        (
            &core,
            "export function partSignals(wiring,point)",
            "item 1: the map has no signal filter",
        ),
        (
            &hud,
            "partSignals(card.wiring,p)",
            "item 1: the live card does not read its signals from the map",
        ),
        (
            &hud,
            "if(!wired&&!explicit){showChip(entry);return true;}",
            "item 2: a part with nothing wired still opens a card",
        ),
        (
            &hud,
            "fetch(`/api/history/mesh?to=${to}&limit=1`)",
            "item 3: the bed does not look up the newest recorded mesh",
        ),
        (
            &hud,
            "No mesh in the recording up to",
            "item 3: the bed card can still say none without asking the recording",
        ),
        (
            &hud,
            "if(wired)sets.push({id:'signals'",
            "item 5: the Raw signals tab is drawn for a part with no signals",
        ),
        (
            &hud,
            "function assemblyOf(entry)",
            "item 6: a part's whole assembly cannot be painted",
        ),
        (
            &css,
            ".hud-reads{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr))",
            "item 4: readouts are not laid out to fill their row",
        ),
        (
            &css,
            "backdrop-filter:blur(16px) saturate(135%)",
            "item 7: the card is not glass",
        ),
        (
            &service,
            "function openDetail(id,explicit=false)",
            "item 2: Details and the parts list cannot ask for the card",
        ),
    ] {
        assert!(file.contains(marker), "{what}");
    }
    assert!(
        !hud.contains("/Toolhead|hotend|Rapid Burner|Sherpa|fan|BLTouch/i.test(path)"),
        "item 1: the old catch-all that put fans and the probe on the Sherpa is back"
    );
    assert!(
        !css.contains(".hud-component-readings{"),
        "item 4: the three-column grid whose empty cell showed as a card is back"
    );
}

/// WO-19 (2026-09-23). the owner: "I just want it to feel really alive and real depth to everything."
/// Pages from the HUD to every page and back, a tooltip on every control, the toolbar a Voron
/// operator needs with an emergency stop on the row, live motion beside X, Y and Z, the bottom
/// strip, numbers printed by unit, the part preview lit, and the glass and motion of the style
/// decisions. Served bytes; the formatter and the flow arithmetic are unit-tested in
/// scripts/service-core-tests.mjs and the new point fields in api::workspace.
#[test]
fn hud_assets_carry_the_wo19_hud_alive() {
    let text =
        |path: &str| String::from_utf8(crate::api::asset("GET", path).expect(path).body).unwrap();
    let (core, hud, css, scene, service, workspace) = (
        text("/hud-core.js"),
        text("/hud.js"),
        text("/hud.css"),
        text("/hud-scene.js"),
        text("/service.js"),
        text("/workspace"),
    );
    for (file, marker, what) in [
        (&hud, "const pagesButton=button('Pages',()=>openPages(),'hud-pages-button')", "item 1: there is no Pages button"),
        (&hud, "function openPages(){", "item 1: there is no page list"),
        (&hud, "entry('Signals',PAGE_NOTES.signals,()=>route('signals'),'hud-page-lead')", "item 1: Signals does not lead the list"),
        (&hud, "link(label,note,'/workspace/'+id)", "item 1: the workspace pages cannot be reached"),
        (&hud, "link('Investigation',", "item 1: Investigation cannot be reached"),
        (&hud, "tip(button('← HUD',()=>route('machine'),'hud-drawer-close')", "item 1: the drawer has no way back to the HUD"),
        (&workspace, "<a href=\"/\" title=\"Back to the printer HUD\">← Printer HUD</a>", "item 1: the workspace has no way back to the HUD"),
        (&hud, "const tipEl=node('div','hud-tip');", "item 2: there is no tooltip"),
        (&hud, "function tip(el,what,does='',cmd=''){", "item 2: controls cannot carry a tooltip"),
        (&hud, "act('BED MESH',()=>send('mesh')", "item 3: no BED MESH on the toolbar"),
        (&hud, "act('MOTORS OFF',()=>send('motors_off')", "item 3: no MOTORS OFF on the toolbar"),
        (&hud, "act('PREHEAT',()=>openPreheat()", "item 3: no PREHEAT on the toolbar"),
        // WO-20 (2026-09-23) removed the HUD's own preheat table: the menu reads the one preset store,
        // whose seed is our profiles (operator::tests). This marker pinned the table until then.
        (&hud, "const menu=preheatMenu(PRESETS);", "item 3: the preheat menu does not read the preset store"),
        (&hud, "act('Macros',()=>openMacros()", "item 3: no Macros on the toolbar"),
        (&hud, "const mine=data.macros.filter(m=>m.macro===true)", "item 3: the macro list is not the gcode_macro sections"),
        (&hud, "act('FW RESTART',()=>send('firmware_restart')", "item 3: no FW RESTART on the toolbar"),
        (&hud, "act('E-STOP',async()=>{estop.disabled=true;await send('emergency_stop');", "item 3: no emergency stop on the toolbar"),
        (&hud, "velocityRead.set(p?.velocity,", "item 4: no live velocity beside X, Y and Z"),
        (&hud, "accelRead.set(p?.max_accel,", "item 4: no acceleration limit beside X, Y and Z"),
        (&hud, "vol=volumetricFlow(p?.extruder_velocity)", "item 4: no volumetric flow beside X, Y and Z"),
        (&hud, "const strip=node('div','hud-strip');", "item 5: there is no bottom strip"),
        (&hud, "fanRead(stripCtrl,ctrl);", "item 5: the controller fan is not on the bottom strip"),
        (&hud, "r.set(known(f?.command)?f.command*100:null,{unit:'% cmd'", "item 5: a fan without a tachometer is not labelled as a command"),
        (&hud, "function roll(el,v,fmt,instant=false){", "style: numbers do not roll between readings"),
        (&hud, "const rolling=new WeakMap(),ROLL_STEP_MS=110;", "render cost: a roll rewrites its number on every frame again"),
        (&hud, "const put=x=>{const s=fmt(x);if(el.textContent!==s)el.textContent=s;};", "render cost: a roll rewrites text that has not changed"),
        (&hud, "formatSignal(s.object,s.field,s.value)", "reviewer: the card's raw values are not printed by unit"),
        (&service, "formatSignal(s.object,s.field,s.value)", "reviewer: the Signals page values are not printed by unit"),
        (&core, "export function formatSignal(object,field,value){", "reviewer: there is no unit formatter"),
        (&core, "export function flowCeiling(loaded,measured)", "flow ceiling must come from the owner's measured limit"),
        (&scene, "new THREE.WebGLRenderer({canvas:cv,antialias:true,alpha:true})", "reviewer: the preview still clears to near black"),
        (&scene, "r.toneMappingExposure=1.5", "reviewer: the preview has no studio exposure"),
        (&css, ".hud-part-model{background:radial-gradient(", "reviewer: the preview well is not lighter"),
        (&css, ".hud-tip[data-on=\"1\"]{opacity:1;transform:none}", "item 2: the tooltip has no style"),
        (&css, ".hud .hud-estop{margin-left:auto;", "item 3: the emergency stop is not styled as one"),
        (&css, ".hud-pages[open]{display:flex}", "item 1: the Pages panel has no style"),
        (&css, "@media(min-width:761px) and (max-height:880px){.hud .hud-gauge.vertical .hud-gauge-track{gap:1px}}", "style: short screens still draw empty rail bars"),
        (&css, ".hud .hud-gauge.vertical::before{", "style: the rail gauges are not on glass pods"),
    ] {
        assert!(file.contains(marker), "{what}");
    }
    assert!(
        !hud.contains("button('Tools',()=>openDrawer('signals'))"),
        "item 1: the unlabelled Tools button is back"
    );
}

/// WO-19 item 2 (2026-09-23): "every button, gauge, readout and toolbar control" means the HUD's
/// dialogs, the part card and chip, and the pages the drawer shows too, not only the HUD surface.
/// Each control says what it is and what clicking does, and a control that loses its title keeps
/// an accessible name.
#[test]
fn hud_dialogs_and_service_pages_carry_wo19_tooltips() {
    let text =
        |path: &str| String::from_utf8(crate::api::asset("GET", path).expect(path).body).unwrap();
    let (hud, css, service, assembly, page) = (
        text("/hud.js"),
        text("/hud.css"),
        text("/service.js"),
        text("/assembly.js"),
        text("/service"),
    );
    for (file, marker, what) in [
        (&hud, "const SENDS='Sends it to the printer. The exact G-code is shown first unless it is on your no-review list.';", "printer buttons do not say what they send"),
        (&hud, "tip(button('✕',()=>closeModal(entry),'hud-modal-close'),'Close','Closes this window. Nothing is sent.')", "a dialog's close button has no tooltip"),
        (&hud, "const b=tip(button(other.title,()=>focusModal(other),'hud-chip')", "the open-window chips have no tooltip"),
        (&hud, "tip(run,'Confirm and send once',", "the review's send button has no tooltip"),
        (&hud, "const b=tip(button(def[1],()=>send(id)),def[1],def[2]+' '+SENDS,id);", "the Testing buttons do not say what they send"),
        (&hud, "if(READ_ABOUT[label])tip(el,...READ_ABOUT[label]);", "the card's dials and readouts have no tooltip"),
        (&hud, "if(!el.hasAttribute('aria-label')&&!el.textContent.trim())el.setAttribute('aria-label',what);", "a control loses its only name when its title is taken over"),
        (&css, ".hud-read:hover,.hud-dial:hover,.hud-lamp:hover{", "the card's dials and lamp have no hover state"),
        (&service, "function tipped(e,what,does=''){e.title=what;if(does)e.dataset.tipDo=does;return e;}", "the service pages cannot say what a control does"),
        (&service, "Clicking shows its reading and history.", "the Signals rows do not say what clicking does"),
        (&service, "'Moves the shared recording cursor to this moment.'", "the Timeline chips do not say what clicking does"),
        (&assembly, "const tipped=(e,what,does='')=>{e.title=what;if(does)e.dataset.tipDo=does;return e;};", "the assembly page cannot say what a control does"),
        (&assembly, "canvas.dataset.tipFocusOnly='1';", "the assembly canvas would show a tooltip while it is dragged"),
        (&page, r#"<button id="new-work" class="primary" title="Record service work" data-tip-do="#, "the service journal button has no tooltip"),
        (&page, r#"<button type="button" data-close="work-dialog" title="Close" data-tip-do="#, "the journal form's close button has no tooltip"),
    ] {
        assert!(file.contains(marker), "{what}");
    }
    assert!(
        !hud.contains(".title="),
        "a HUD control still carries a bare title instead of a tooltip"
    );
    assert!(
        !service.contains("move the shared cursor here"),
        "the Timeline chip title still carries the old em-dash text"
    );
}

/// WO-17 item 7 revision and nozzle datum (2026-09-23), our two answers. "The tall tower points
/// up on my printer; that's where the umbilical goes": the carrier stands tower up and the loom
/// starts on the tower's top face. "About 2.5 mm from the bottom of the cowl to the nozzle tip": the
/// tip is placed from that hand measurement, and stays the Z datum. Served bytes.
#[test]
fn hud_assets_carry_kens_tower_up_carrier_and_nozzle_datum() {
    let scene = crate::api::asset("GET", "/hud-scene.js").expect("hud-scene.js is served");
    let scene = String::from_utf8(scene.body).unwrap();
    for (marker, what) in [
        (
            "const CARRIER_AT=[-20.70,168.06,43.35];",
            "the carrier is not at its measured tower-up position",
        ),
        (
            "g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIER_AT);}",
            "the carrier is not turned tower up",
        ),
        (
            "ops:[rx(-Math.PI/2)],at:[127.34,181.15,128.76]",
            "the board is not turned over with the carrier",
        ),
        (
            "carrier.userData.towerTop??=towerTop(carrier.geometry)",
            "the umbilical does not start on the tower's top",
        ),
        (
            "function towerTop(g){",
            "the tower's top face is not found from the geometry",
        ),
        (
            "const NOZZLE_BELOW_COWL_MM=2.5;",
            "our 2.5 mm is not one named constant",
        ),
        (
            "(cowl.geometry.boundingBox.min.z-NOZZLE_BELOW_COWL_MM)",
            "the nozzle tip is not placed from the cowl's lowest point",
        ),
        (
            "const tip=nozzle.geometry.boundingBox;",
            "the nozzle tip is no longer the Z datum",
        ),
    ] {
        assert!(scene.contains(marker), "{what}");
    }
    for gone in [
        "centreOn(g,[],[126.87,158,130.76])",
        "at:[124.96,165.9,141.5]",
    ] {
        assert!(
            !scene.contains(gone),
            "the tower-down placement is back: {gone}"
        );
    }
}

/// WO-20 (2026-09-23), the owner: "Nothing works without the printer being on. You can look at things, but
/// you can't really alter anything," and "Anybody can change their preheat presets, and they should
/// save within the system." Served bytes: one lock for every printer control from the collector's
/// link, no preview asked for a control that cannot run, the toolpath back-off, the pose applied once
/// the model is ready, and the one preset store with its Save and Cancel.
#[test]
fn hud_assets_carry_the_wo20_printer_off_and_presets() {
    let text =
        |path: &str| String::from_utf8(crate::api::asset("GET", path).expect(path).body).unwrap();
    let (hud, core, service, scene, css) = (
        text("/hud.js"),
        text("/hud-core.js"),
        text("/service.js"),
        text("/hud-scene.js"),
        text("/hud.css"),
    );
    for (file, marker, what) in [
        (
            &core,
            "export function printerLink(observer,{replay=false,lastSeen=null,now=Date.now()/1000,",
            "there is no printer state from the collector link",
        ),
        (
            &core,
            "export function preheatMenu(presets){",
            "the preheat menu has no reading of the store",
        ),
        (
            &core,
            "export function presetDiff(saved,rows){",
            "the preset editor cannot tell what is unsaved",
        ),
        (
            &hud,
            "function link(){const s=getState();return printerLink(s.link,",
            "the HUD does not read the printer state",
        ),
        (
            &hud,
            "function lockControls(root=document){",
            "printer controls are not locked while the printer cannot run them",
        ),
        (
            &hud,
            "if(!can(id)){feedback(why(id),'refused');return null;}",
            "a control that cannot run still asks the service for a preview",
        ),
        (
            &hud,
            "const TP_BACKOFF_S=[5,15,60]",
            "the toolpath load does not back off",
        ),
        (
            &hud,
            "if(link().state==='off'||performance.now()<tpNext)return;",
            "the toolpath is fetched while the printer is off",
        ),
        (
            &hud,
            "showToolpath(name,(text,state)=>{if(state==='reported')toast(text);})",
            "the automatic toolpath load still writes over open dialogs",
        ),
        (
            &hud,
            "const menu=preheatMenu(PRESETS);",
            "the PREHEAT menu does not read the preset store",
        ),
        (
            &hud,
            "const save=tip(button('Save',",
            "the preset editor has no Save",
        ),
        (
            &hud,
            "const cancel=tip(button('Cancel',()=>modalStack.find(m=>m.id==='presets')?.refresh())",
            "the preset editor has no Cancel",
        ),
        (
            &service,
            "if(r.observer)state.link=r.observer;",
            "ticks do not carry the printer state",
        ),
        (
            &service,
            "if(r.observer_now)state.link=r.observer_now;",
            "the snapshot does not carry the printer state",
        ),
        (
            &service,
            "function paintLink(){",
            "the header has no printer-state indicator",
        ),
        (
            &scene,
            "if(lastSet)set(...lastSet);",
            "a cold replay link still leaves the head at its rest pose",
        ),
        (
            &css,
            ".hud-app #led-transport[data-state=\"off\"]",
            "the printer-off pill has no style",
        ),
        // Reviewer's decision on WO-20 (2026-09-23): the two restarts are the way out of a halted
        // Klipper while Moonraker answers, and still ask once through send().
        (
            &core,
            "export const RECOVERY=['firmware_restart','klipper_restart'];",
            "the restarts are not the way out of a halted Klipper",
        ),
        (
            &core,
            "recover:['shutdown','error'].includes(observer.klipper_state)",
            "the printer state does not say when the restarts may go",
        ),
        (
            &core,
            "export function linkAllows(link,id){",
            "no single rule for what the printer's state lets through",
        ),
        (
            &hud,
            "function can(id){const l=link();if(!linkAllows(l,id))return false;",
            "the HUD's controls do not follow that rule",
        ),
        (
            &hud,
            "if(was&&(!l.lock||l.recover))controls(true);",
            "the command list is not read again when the restarts open",
        ),
    ] {
        assert!(file.contains(marker), "{what}");
    }
    for gone in [
        "PROFILE_PREHEAT",
        "showToolpath(name).catch(",
        "let MATERIALS=[['PLA',215,60]",
    ] {
        assert!(!hud.contains(gone), "the old path is back: {gone}");
    }
}

/// WO-21 (2026-09-24): the four leftovers from WO-20, in the served bytes. The font preloads are
/// CORS preloads the @font-face loads can use; the page backs off while its own service is down and
/// says so as a state of its own; one writer, over the one state rule, sets the badge; and a replay
/// or rewound view never asks the printer for the recorded print's G-code.
#[test]
fn hud_assets_carry_the_wo21_leftovers() {
    let text =
        |path: &str| String::from_utf8(crate::api::asset("GET", path).expect(path).body).unwrap();
    let (page, service, core, hud, css) = (
        text("/"),
        text("/service.js"),
        text("/hud-core.js"),
        text("/hud.js"),
        text("/hud.css"),
    );
    for (file, marker, what) in [
        (
            &page,
            "href=\"/fonts/geist.woff2\" crossorigin>",
            "the Geist preload cannot be used by the font load",
        ),
        (
            &page,
            "href=\"/fonts/geist-mono.woff2\" crossorigin>",
            "the Geist Mono preload cannot be used by the font load",
        ),
        (
            &service,
            "const SERVICE_BACKOFF_S=[1,2,5,10]",
            "the page does not back off while its service is down",
        ),
        (
            &service,
            "if(svc.down?performance.now()<svc.next:(!state.live||sseOk))return;",
            "the tick poll ignores the back-off",
        ),
        (
            &service,
            "state.link={printer_state:'service',last_success:svc.since};",
            "the service being down is not handed to the state rule",
        ),
        (
            &service,
            "function serviceBack(){if(!svc.down)return;",
            "nothing resumes when the service answers again",
        ),
        (
            &core,
            "if(observer?.printer_state==='service')return {state:'service'",
            "the state rule has no service-down state",
        ),
        (
            &core,
            "export function controlsSummary(link,{live=true,enabled=false}={}){",
            "the badge has no rule over the printer state",
        ),
        (
            &hud,
            "function paintControls(l=link()){const c=controlsSummary(l,",
            "the badge is not written from the state rule",
        ),
        (
            &hud,
            "if(link().state==='replay'||!getState().live){if(notStored!==file)",
            "a replay or rewound view still fetches the G-code",
        ),
        (
            &css,
            "#led-transport[data-state=\"service\"]",
            "the service-down pill has no style",
        ),
    ] {
        assert!(file.contains(marker), "{what}");
    }
    for (file, gone) in [
        (&page, "href=\"/fonts/geist.woff2\">"),
        (&service, "sse.onerror=()=>{sseOk=false;};"),
        (&hud, "textContent=caps.enabled?'LIVE CONTROLS':'READ ONLY'"),
        (&hud, "'CONTROLS LOCKED'"),
    ] {
        assert!(!file.contains(gone), "the old path is back: {gone}");
    }
}
