# Stella Scope 0.4 — machine service workbench


The RP-01 HUD review and current limits are documented in [the review folder](docs/visual-review-rp-01/README.md). The genuine 350 CAD reference, per-component inspector, local STL job preview, preparation flow and guarded calibration implementation remain pending owner visual and physical acceptance. Read [the backend control contract](docs/visual-review-rp-01/SOL-BACKEND.md) before enabling controls.

A recorder and printer HUD for the owner's Voron V2.4 R2 350. Printer controls default to disabled; replay and demo always remain read-only. The owner confirmed a BLTouch is fitted on 16 September 2026. Its exact version, mounting geometry, wiring connector and offset are not guessed.

## Run

Use the packaged executable on its matching architecture:

```sh
./stella-scope --demo
./stella-scope --config stella-scope.json
./stella-scope --replay recording.sqlite3
```

Open `http://127.0.0.1:7420/`. The new service workbench is the landing page. The 0.3 workspace remains at `/workspace`, the original dashboard at `/classic`, and 0.2 bounded investigation/export at `/investigate`. Earlier source, collector functions and SQLite tables are preserved. No installation, printer motion, homing, heating, G-code request or Klipper configuration change is performed by building or running the UI.

## What is new

**Native WebGL viewer.** Depth-buffered geometry replaces painter-sorted rendering in the new view. Seven cameras include top, bottom, back and both sides; full azimuth orbit, below-bed elevation, pan, zoom, touch pinch/pan, keyboard navigation, focus, layer visibility, fullscreen and exploded assembly separation. Click geometry to select the actual rendered component. Simplified electronics models occupy the underside compartment; a recognizable BLTouch replaces the generic probe shape. Its pin is an illustrative shape, not simulated deployment feedback.

Drag to orbit; Shift/right/middle drag to pan; Ctrl+wheel to zoom (plain scroll moves the page — the viewer no longer captures it); two fingers to pan/pinch; double-click to focus; arrow keys to orbit, Shift-arrows to pan, +/- to zoom, 0 to reset. WebGL support is required for this renderer; if unavailable, the app says so and the older workspace remains available. No false claim of 3D verification is made on an unsupported browser.

The bed and lower drives remain fixed. Gantry/head motion follows selected-time Klipper requested coordinates, not encoder feedback. Owner-corrected geometry 2026-09-16: this gantry has no front crossbar, and the Z motors lie sideways with front-pair shafts facing the front and rear-pair shafts facing the rear. Existing shape envelopes, schematic belt lanes and added control-board/PSU/relay placement are not exact CAD of the modified machine. Eight illustrative driver packages do not certify eight installed drivers or identify an Octopus PCB revision. Inventory slots are much more granular than the assembly-level rendered geometry.

**Live view: push, not poll (2026-09-17).** The recorder subscribes to Moonraker's websocket (`ws://…/websocket`, `printer.objects.subscribe`, hand-written client — no crate, no async) and stores every status change the moment Klipper reports it; a silent printer is re-sampled from the merged status at 2 Hz with no network traffic. If the stream drops, the collector falls back to HTTP `objects/query` polling at `poll_interval_ms` (default 500) and `/api/meta` shows which transport is active, updates per second, last query latency and write times. `configfile` is fetched once per connect, Klippy restart and SAVE_CONFIG — never per sample — and merged into every stored sample so the recording schema is unchanged. The page receives each stored sample over server-sent events (`GET /api/tick/stream`), served from the collector's cached tick payload, with the old `/api/tick` poll kept as fallback while the stream is down. The full workspace payload loads on seek or page change only. Head motion is replayed between the two newest recorded samples and labelled "Smoothed between recorded samples" — positions shown are interpolated recorded values, never extrapolated or invented. The activity line (printing, paused, gantry levelling with probed range and retry, bed mesh probing, homing, heating, moving, idle) is derived from recent recorded console evidence plus the newest sample and always carries its basis string; it is not a machine-reported activity state, and in rewind the line says so.

**Click-to-detail overlay.** Clicking any rendered part opens a top-layer detail dialog (available in fullscreen): live tick readings for that part, its recorded identity from the parts book, and for the bed the recorded mesh with the green-to-red deviation scale. The side inspector remains for browsing.

**Recorded bed mesh panel.** Renders whatever matrix was actually recorded — the owner's 9×9 default, or a smaller adaptive mesh when one was taken — at its true dimensions, with min/max/range in the summary. Green is nearest zero, red is the worst recorded deviation. No flat mesh is invented when none was recorded.

**Model overhaul v2 (2026-09-17).** Outlined technical-illustration rendering (edge lines), horizontal cylinders for shafts/pulleys/gears, hover tooltips naming every part, v-slot groove hints, a recognizable Rapid-Burner-style toolhead with fin stack and ducts, sideways Z drives with outward shafts and pulleys, CoreXY belt runs, PEI bed, a 2×4 driver-module controller board, host, PSU, SSR and wire runs. Part identities and slots are unchanged; still schematic, not measured CAD. 24-hour trend charts on the Leveling page show one point per distinct recorded mesh (adaptive flagged) and final probed range per levelling run, plus driver-warning counts; resonance/shaper console output is tagged as its own `resonance` event kind and captured at burst cadence while it streams.

**BLTouch probing, live (2026-09-17).** Klipper's probe lines (`// probe: at X,Y bed will contact at z=Z`, formats taken from this printer's own recording) are parsed into an additive `probe_points` table with time, position and procedure context (homing Z / gantry levelling round n / bed mesh / probe accuracy / print). While probing runs, the activity line reads "BLTouch probing · <step> · point X,Y = Z". On the machine view the newest landings glow on the bed; the Leveling page plots them over the mesh with their recorded z and lists them with context. A homing zero reference at bed centre is kept per print; when it drifts more than 0.05 mm from the recent mean a `z_health_alert` event is recorded. Z-offset console evidence (SET_GCODE_OFFSET, Z_OFFSET_APPLY_PROBE, SAVE_CONFIG, bltouch z_offset) is recorded as `z_offset_change` events and shown as one log on the Leveling page.

**Print context and the resonance band (2026-09-17).** Once per observed print start the recorder makes two read-only GETs — Moonraker's parsed `/server/files/metadata` and a one-shot ranged read of the last 64 KB of the gcode — and stores slicer metadata plus Orca's tuned settings block (flow ratio, pressure advance, temperatures, wall/acceleration values) in an additive `print_context` table. The Leveling page shows one row per print with those values, and "time near resonance": recorded seconds per hour spent at speeds whose GT2 belt-tooth frequency (v/2) sits within ±3 Hz of the measured 43.4 Hz Y resonance (~83–93 mm/s) — the hum — derived purely from recorded velocities and labelled as an estimate.

**Nine connected pages.** Machine, Signals, Electronics, Parts & revisions, Suspected causes, Service journal, Timeline, Heat & fans, Gantry & bed. Each shares one selected recording timestamp. Direct `/service/<page>` routes and timestamp fragments work. Missing data stays unknown. Future readings are not inserted into rewind. There is no command-control page.

**Reporting and signal provenance.** The collector discovers bounded additional safe status-object families: MCUs, additional named fans/heaters/temperature sensors, CAN statistics, filament sensors, configured outputs/servos and supported driver families. Explicit configured object/filter order remains intact. A capture manifest identifies available, captured and omitted objects and reasons. Macro values are not automatically collected. The Signals page differentiates sensor feedback, requested motion, commands/settings, cached query results and cumulative statistics. Numeric/boolean fields have bounded representative recorded history, not raw waveform capture.

**Electronics.** MCU chip, firmware, clock and reported statistics are shown when supplied. Configured component-to-controller/pad routes preserve inversion and pull-up/pull-down markers. They are startup/restart configuration, not electrical continuity, measured voltage or verified connector labels. Virtual pins and board aliases need manual resolution. MCU identity does not identify a commercial PCB revision. Fan duty is a command; RPM is shown only when a tachometer is reported. Host CPU/memory/throttle statistics are opt-in and have their own capture timestamp.

**BLTouch.** Control and sensor routes are separate. The display exposes recorded configuration offsets, cached `last_query`, modern `last_probe_position`, and the legacy result where present. It never labels cached `last_query` as continuous trigger feedback or infers pin extension. A changed mount/probe requires as-built identification and supervised physical commissioning before motion controls are enabled.

**154 inventory locations.** Each of four corners has 21 distinct records for housings, motor, reduction and motor pulleys, jackshaft, bearings, drive mounting, Z belt/rail/carriage, gantry joint pieces, clamp, top idler housing/pulley/axle, frame connection, foot, panel retention and fasteners. Further slots cover rear A/B and XY idlers, hotend parts, probe, extruder, fans, bed, electronics, enclosure and wiring. Add custom locations for modifications. Templates are not evidence that a particular part, variant, material or revision is installed.

Each revision can record model/maker, version, material/colour, print settings, hardware/connector labels, reason, source/STL URL, source commit, actual local-file SHA-256, licence, identification evidence, state and effective date. Revisions are appended; a historical as-built view resolves the revision effective at the recording cursor. Optional local STL hashing reads at most 20 MiB in the browser and does not upload or retain the file itself. It is not an STL geometry import/viewer.

An offline source catalogue is generated from the complete official Voron-2 STL tree pinned to `a192410e27ea345644ae5c4b29b4c9c40cbe1a73`. It lists actual file paths, Git blob IDs and source links. Git blob SHA-1 is not labelled an STL SHA-256. Source matches are candidates, not a recommendation to replace an installed modification. No official CAD/STL geometry is copied into this renderer.

**Persistence boundary.** The parts book and service journal are browser-local, with separate demo/live namespaces for this one machine. JSON export/import/merge provides a portable backup. They are NOT stored in the recorder SQLite file, server-synchronized, multi-device or automatically backed up. Clearing browser storage loses unexported entries. A version check rejects a stale-tab save; imported conflicting immutable IDs are rejected. Import is limited to 2 MB, 3000 revisions and 2000 service entries. Do not mix unrelated machines/recordings into this book. A timestamped view link does not share the data.

**Suspected causes and service journal.** Deterministic rules cite the recorded symptom or an explicitly selected operator symptom and give inspection locations, what to inspect, why it matters, safety preconditions and official reference links. A high current-scale reading alone does not diagnose binding or looseness. A nonzero cumulative communications counter alone does not prove an active fault. The tool cannot sense torque, belt tension, bolt tightness or shaft slip. No probabilities are invented. Journal entries record open/checked/replaced/deferred outcomes and evidence; follow-ups append records rather than rewrite old entries. Nothing is marked repaired automatically.

Before mechanical inspection, stop motion, cool heaters, isolate power and support the gantry appropriately. Never unplug a powered motor. Mains/PSU/SSR work requires proper isolation and a qualified person; the UI neither provides improvised mains procedures nor invents torque specifications. Follow the exact assembly or part manufacturer's procedure; not every fastener should simply be tightened harder.

## Configuration and API

All existing JSON keys remain supported with their prior defaults. Complete configuration documentation is preserved in `docs/README-0.2.md`, and 0.3 behaviour in `docs/README-0.3.md`. Use `--print-config` for the actual full configuration.

| New key | Default | Bounds / effect |
|---|---|---|
| `telemetry.discover` | true | Automatically include allowed additional reported status families. False retains configured objects only. |
| `telemetry.max_extra_objects` | 96 | 0–256 additional objects; excess is visibly omitted. |
| `telemetry.host_stats` | false | Opt-in GET `/machine/proc_stats`; failure is reported without stopping ordinary collection. |
| `telemetry.host_interval_ms` | 5000 | 1000–60000 ms independent host-cache interval. |
| `max_db_mb` | 4096 | Prunes the oldest complete keyframe groups when the database's used pages exceed this size. |

Discovery is refreshed at the existing `discovery_interval_s`. Up to 2048 valid discovered names are considered. Raw status now also carries `_scope_capture` and optional `_scope_host`. More objects and repeated capture metadata increase recording size and polling work; measure actual Pi/SD behaviour and adjust retention, object cap and polling interval. Existing recordings do not magically gain missing historical objects. Macro variables explicitly listed by the owner remain raw-recorded, but are not projected by the Signals endpoint. Do not expose the unauthenticated local interface to the internet.

`GET /api/workspace` includes `point.telemetry` only for the selected detailed point, not every history point. Projection caps are 512 scalar signals, 256 pin entries and depth/array limits, surfaced when reached. Raw arbitrary config/macro secrets are excluded from that projection. This is broad status coverage, not every Klipper internal field or every physical signal.

```text
GET /api/tick          → schema stella-scope.tick.v1, newest sample only (GET only, read-only)
```

`/api/tick` returns the latest recorded sample's compact projection, the derived activity object with its basis string, homed axes, the compact observer status and sample age. It is intended for the Live fast loop and is not a rewind API; the workspace endpoint remains the source for history.

```text
GET /api/trends?window=86400    → schema stella-scope.trends.v1 (GET only, read-only)
```

`/api/trends` (window 600–86400 s) returns distinct recorded bed meshes (rows/cols, min/max/range, adaptive flag), complete gantry levelling runs (rounds, final probed range) and driver-warning counts per hour. Repeated capture of an unchanged mesh collapses to one point; gaps are not interpolated; warnings are recorded event counts, not a health verdict.

```text
GET /api/signal-history?ts=<unix>&window=300&points=60&object=extruder&field=temperature
```

Requires a whitelisted status object, ordered dot path, 2–120 points and a 1–7200 second window. GET only; config/macros and arbitrary raw text are rejected. Indexed at-or-before samples are bounded to 2 MiB each, 64 MiB scanned and a cooperative five-second budget. History can miss peaks; use Investigation for extrema analysis. Values are numbers, booleans or null. Null is unknown. New samples use 60-second keyframes, field deltas and deduplicated context; legacy full rows remain readable.

## Build / evidence

One synchronous Rust binary, four direct dependencies, no async runtime/framework/CDN. Source is a guarded incremental continuation from `05cb83ada7267e3d68942761aa11adaf09e602c2`. The integration script verifies exact baseline Git blob hashes before editing common modules. No new runtime dependency was added. WebGL runs in the viewing browser, not on the Pi recorder.

```sh
cargo test --locked -j 2
cargo build --release --locked -j 2
node scripts/service-core-tests.mjs
node scripts/service-http-smoke.mjs
node scripts/hud-core-tests.mjs
# Against an explicitly approved loopback replay with imported CAD:
HUD_URL=http://127.0.0.1:17421 node scripts/browser-hud.mjs
```

CI checks original recorder/workspace regression suites, new telemetry/signal tests, parts validation/effective-time tests, compiled service HTTP behaviour and browser interaction checks. The legacy browser-service and offline procedural preview scripts target the retired schematic renderer; use browser-hud.mjs for the source-CAD HUD. They are not evidence of current HUD motion or acceptance. Final binaries include exact source commit, stripped size, architecture, synthetic write benchmark and RSS report under `validation/`. Published prerelease assets include checksums, source and interactive HTML/screenshot evidence. The interactive HTML contains synthetic fixtures; it does not connect to your printer. Its legacy-page links need the real application.

Physical Voron/Moonraker acceptance, exact modified toolhead/board/harness identity and geometry, a real levelling run, Pi SD latency and long-duration/concurrent-reader soak remain unverified. Hosted and synthetic results are not substituted for these tests. No merge into main or equipment installation is implied by a release.

## Source references and interpretation

The build uses the owner's confirmed BLTouch and original 350 mm/corner arrangement. External documentation informs the signal labels and safe inspection hypotheses; it does not verify installed hardware:

- Klipper status definitions: https://www.klipper3d.org/Status_Reference.html
- BLTouch control/sensor distinction: https://www.klipper3d.org/BLTouch.html
- Probe mount/calibration considerations: https://www.klipper3d.org/Probe_Calibrate.html
- Driver diagnostics and electrical limits: https://www.klipper3d.org/TMC_Drivers.html
- Voron startup/assembly reference: https://docs.vorondesign.com/build/startup/
- Pinned STL source inventory: https://github.com/VoronDesign/Voron-2/tree/a192410e27ea345644ae5c4b29b4c9c40cbe1a73/STLs

Assembly and troubleshooting statements are conditional engineering guidance, not remote identification of the actual loose component. The original brief's simplified term “load” is not reused as a measured mechanical quantity: current scale and MSCNT are not independent physical-position feedback.
