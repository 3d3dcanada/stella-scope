# Demo printer sample prints

The demo printers (`driver: "demo"`, src/demo.rs) play these files. All four are 3D3D's own shapes, made
for the demo, and sliced for real so the head, layers, temperatures, fan and tool changes follow a genuine
file. A demo printer takes the sample that fits its machine and, when its bed's middle is not where the
slicer centred the part, moves a copy of the file to the middle of its own bed when it loads (src/demo.rs
`for_bed`: only the X and Y of the moves, in memory, kept once per distance). Nothing at load time changes
the files in this folder.

| File | Taken by | Shape | Slicer estimate |
|---|---|---|---|
| `stella-twist-vase.gcode.gz` | Klipper machines with a bed of 200 mm or more and 150 mm of Z (Voron 2.4 350, Trident 300, SV08, Ender-3, the toolchanger) | 6-lobe twisted vase, 110 mm, 120° twist | 2h 48m 40s |
| `core-one-l-twist-bowl.gcode.gz` | the Prusa CORE One L | 5-lobe twisted bowl, 55 mm, 72° twist | 1h 46m 20s |
| `small-twist-vase.gcode.gz` | smaller machines (V0.2 120, Micron+ 180) | 5-lobe twisted vase, 60 mm, 90° twist, 55 mm across | 53m 47s |
| `duo-twist.gcode.gz` | machines with two or more tools (IDEX, toolchangers) | one twisted column in two halves, 70 mm, each half its own tool (`T0`, `T1`), about 290 tool changes | 3h 46m 36s |

Shapes (`demo/tools/make-shapes.rs` writes the STLs and the duo's 3MF; `rustc -O --edition 2021
demo/tools/make-shapes.rs -o /tmp/make-shapes && /tmp/make-shapes OUTDIR`):
a ring of radius `r(t) = r0 + (r1 - r0)·t + bulge·sin(πt)` at height `h·t`, each point pushed out by
`1 + depth·cos(lobes·θ)` and turned by `twist·t`; closed top and bottom.
Vase: lobes 6, r0 34, r1 26, h 110, twist 120°, depth 0.12, bulge 12. Bowl: lobes 5, r0 30, r1 55, h 55,
twist 72°, depth 0.14, bulge 4. 80 rings of 96 points.
Small vase: lobes 5, r0 22, r1 18, h 60, twist 90°, depth 0.10, bulge 6, 60 rings of 96 points.
Duo: lobes 3, r0 28, r1 24, h 70, twist 180°, depth 0.12, bulge 4, 70 rings; the column cut along its axis,
half A (angles 0 to π) on extruder 1 and half B (π to 2π) on extruder 2, each moved 0.4 mm off the cut.

Sliced 2026-10-18 with PrusaSlicer 2.9.6 (Filament Edition AppImage, `--datadir` the flatpak's):
`Prusa CORE One L HF0.4 nozzle`, `0.25mm SPEED @COREONEL HF0.4`, `Generic PETG @COREONE HF0.4`,
`--fill-density 10% --fill-pattern grid --no-binary-gcode`. The bowl is centred at 150,150. The vase
adds `--gcode-flavor klipper --machine-limits-usage ignore --bed-shape 0x0,350x0,350x350,0x350
--start-gcode 'PRINT_START BED=[first_layer_bed_temperature] EXTRUDER=[first_layer_temperature]'
--end-gcode PRINT_END --printer-model ''` and is centred at 175,175. The small vase is the same as the
vase with `--bed-shape 0x0,120x0,120x120,0x120 --center 60,60 --dont-arrange` (STL in, `-g`). Then `gzip -9 -n`.

The duo is a PrusaSlicer 3MF (each half is an object with `extruder` 1 or 2) on a two-extruder printer
profile, because the command line takes one extruder for every file but a 3MF carries it per object:
`--printer-profile "BIBO M2 BLTouch Dual extrusion - rear" --print-profile "0.24mm DRAFT @BIBO2"
--material-profile "Generic PLA @BIBO2 - 3d3d" --material-profile "Generic PLA @BIBO2 - 3d3d"
--filament-type PETG,PETG --temperature 250,250 --first-layer-temperature 250,250 --bed-temperature 85
--first-layer-bed-temperature 85 --extruder-colour "#04D9C4;#FF6A00" --fill-density 10% --fill-pattern grid
--no-binary-gcode --gcode-flavor klipper --machine-limits-usage ignore --bed-shape 0x0,250x0,250x250,0x250
--start-gcode 'PRINT_START BED=[first_layer_bed_temperature] EXTRUDER=[first_layer_temperature]'
--end-gcode PRINT_END --toolchange-gcode '' --printer-model '' --dont-arrange -g duo-twist.3mf`. It is
centred at 125,125 and has no wipe tower. (The BIBO profile is only the slicer's way to get two extruders;
the file is Klipper G-code and the printer it runs on is whichever demo machine takes it.)

Thumbnails: PrusaSlicer's command line cannot draw the pictures it embeds, so `demo/tools/thumbnails.rs`
draws each part from its STL (a z-buffer, an iso view, a transparent background, the part's own colour:
vase teal, bowl magenta, small vase green, duo halves teal and orange) and adds 16x16, 313x173 and 440x240
PNGs as `; thumbnail begin` blocks after the file's first line, nothing else changed:
`rustc -O --edition 2021 demo/tools/thumbnails.rs -o /tmp/thumbnails && /tmp/thumbnails in.gcode out.gcode
16x16,313x173,440x240 part.stl:04D9C4`. The demo printer serves the largest picture.

The two first samples (`stella-twist-vase`, `core-one-l-twist-bowl`) were not sliced again. On 2026-10-18 they were
given their thumbnails the same way: the file as first sliced on 2026-10-18 (the vase 2h 48m 40s, the bowl 1h 46m
20s, same profiles, same moves) with one block of comment lines added after line 1 (vase +914 lines, 8,433,573 to
8,507,087 bytes unpacked; bowl +674 lines, 5,366,133 to 5,420,215). The packed files changed
(vase sha256 f14b39c6... to d635b09a..., bowl 23212503... to aecabf8c...), so their byte positions differ from
the first version; the demo reads positions from the file it holds, and its tests pass against the new files.

## The toolchanger (polish WO-03, 2026-10-18)

One printer of "Try the demo farm" is a Voron 2.4 300 with a StealthChanger bank of three docks, three JabberWocky
tools (`cad_model` `voron_v24_300_printed`, `cad_options` `stealthchanger_bank`). It replaced the DoomCube 2, whose
model has no part records and which upstream ships static, so it never moved. It plays the duo sample, so its active
tool changes at every `T0` and `T1` of the file and the third tool never leaves its dock. Every file of this selection
is open: the docks, shuttle, backplates and fasteners are GPL-3.0 and the JabberWocky head is CC BY-SA 4.0; the bank's
other tool, Xol, is CC BY-NC-SA 4.0 and is not used, so the demo farm never asks for a non-commercial licence (the
one-press fetch takes these files with the models). With the bank's files not fetched the printer works with the machine
alone (the stock head is drawn, and the tile and the printer's page say the tools are not installed). Where each tool
waits (the head's coordinates at its dock) is worked out from the bank's own placements (`printer_models::bank_tools`),
and each tool's nozzle is measured from its own file (`web/printer-print-frames.json`, `tools`), so whichever tool is
active has its nozzle on the part.

## One press fetches the farm's models; the Trident has a gantry; every motion-registered machine has its plate measured (finish 2, F03, 2026-10-19)

**One press.** "Try the demo farm" adds the eight printers and, because the person pressed the button that has the open licences in one line beside it
(`GET /api/printer-models/demo-farm`, built from the catalogue: licences, authors, size), fetches the demo's open models in the same request
(`POST /api/printers/demo-farm` with `accept_licence: true`; without it nothing is fetched and nobody is asked). The service fetches three files at a time, smallest
first, each from its author's published address over https and checked against its pinned SHA-256; `GET /api/printer-models/wanted` says per printer what its picture
still waits for (`needs`) and per file what is happening (`fetch.items`: queued, fetching with its bytes, done, failed with a plain sentence and a kind). Each wall tile
shows its own strip (waiting, getting it with a percentage, checking it, failed with Try again, not here yet with Get it, or Read notice for the CORE One L) and draws its
machine when its own files are here. Offline is said per tile in plain words ("No connection to the author's site. Check the internet, then try again.") and Try again asks
for what is still missing. One press took 31 to 40 s for 169 MB (14 files, six runs at six window sizes, mean 35.6 s) on the machine this was built on (evidence/F03/press/summary.tsv). The CORE One L is Prusa's own CAD under its
Open Community License: it is not part of the press, its tile offers Prusa's notice first, and its address (a release asset of Stella Scope's own repository) answers 404
until that release is published, which the tile says in words.

**The Trident's gantry.** The Trident base model has no gantry or toolhead; the demo Trident carries the option `trident_r2_gantry` (the R2 gantry 300 and X carriage
modules of the 3D Print Rig, VoronDesign, GPL-3.0, 5.5 MB, added to `web/printer-models-extra.json` with their pins). Their part records say what moves: the X beam with Y,
the carriage with X and Y; the Trident's registered groups `gantry` and `head` name those parts, and the bed still drops with Z. The toolhead that hangs from the carriage is
not drawn (its file has no stated licence).

**Print frames.** Every machine with registered motion has its plate and nozzle measured from its model (`tools/printer-print-frames.rs`): the Fysetc V2.4, Rook, Satsuma,
THE 100, TicTac, the Annex K1, K2 and K3 and the SnakeOil XY IDEX, besides the nine of before. The Annex files place their parts with matrices and are written with their up along
-Y or -Z, so their registered profile now carries `orient` (the rig's own turn), `basis` and `see_through` (`tools/printer-motion-parts.rs`). A machine with no registered motion is
static, and its page says so.

**IDEX.** No GPL IDEX machine is in the catalogue: the six Rat Rig V-Core IDEX files (4.0 and 4.1 at 300, 400 and 500 mm) are non-commercial (CC BY-NC-SA) and carry no record of what moves (2,640 to 3,103 parts, none with a group or a motion, checked in each file),
and the SnakeOil XY IDEX is "GPL-3.0 + Annex Engineering License" (mixed), which a build never carries and the one press never asks for. It is registered all the same
(`snakeoil_xy_idex`: two carriages, the one not printing waits at its end of the beam) so a person who fetches it, with its licence confirmed, can run the duo sample on it with
both carriages moving; it is a demo printer like any other (`cad_model` `snakeoil_xy_idex`), not one of the eight.
