# Printer CAD recipes (finish-2 F09)

How each Prusa model the build carries is made from its maker's own STEP file. Nothing here is drawn by hand: the STEP is converted by
`step_to_glb.py` (OCP, run with FORMA's installed Python), the parts are given to the motion groups by the rules in each folder's `groups.json`, and the numbers
that follow the printer (`web/printer-motion-extra.json`, `web/printer-print-frames-prusa.json`) are measured off the model's own geometry.

```
tools/printer-cad/build-model.sh DIR STEP_FILE [OUT_DIR] [--install]
```

| Folder | Machine | STEP (download from its own page, no login) | Licence | Embedded file |
|---|---|---|---|---|
| `prusa-core-one-l` | Prusa CORE One L | Printables 1520379 `CORE_One_L_assembly.step`, 63,859,975 bytes, sha256 `2f46bccb098bd8dabc76aa13fd01b1e64c56370fa29ba8a6336f0305b8c86074` (Prusa Research) | Open Community License v1 | `models/prusa-core-one-l.glb.gz` (10,225,282 bytes) |
| `prusa-core-one` | Prusa CORE One | Printables 1520471 `CORE_One_assembly.step`, 61,178,683 bytes, sha256 `4ead8073806c86df0e2309e905f84664715b2a09b9c384839aa33a8a8591aa08` (Prusa Research) | Open Community License v1 | `models/prusa-core-one.glb.gz` (8,832,438 bytes) |
| `prusa-mk3s` | Original Prusa i3 MK3S+ / MK3S / MK3 | Printables 563056 `00Fusion360Export.step`, 41,249,381 bytes, sha256 `c2ea1cafa2f71175524431a182f9e519f5568a0760ac007375c88495430ca5b2` (Leto, from Swesen's Fusion model, from jzkmath's SolidWorks assembly of Prusa's design) | GNU GPL (Prusa, jzkmath), CC BY (Swesen), CC0 (Leto) | `models/prusa-mk3s.glb.gz` (4,361,256 bytes) |
| `prusa-mini` | Original Prusa MINI / MINI+ | Printables 307054 `Prusa Mini Assembly.STEP`, 25,510,600 bytes, sha256 `26d3bad51186ba7b7074d1a9b5405d77baf1f05d689fd3a916f0cf19bf188dd1` (Luis; printed parts are Prusa's GPL-3.0) | stated CC0; Prusa's parts GPL-3.0 | `models/prusa-mini.glb.gz` (1,319,342 bytes) |

The notice that travels with each model (maker, terms, what was done, both SHA-256 numbers, then the licence text) is `docs/notices/<id>.txt` with the licence texts in `docs/licences/`.

## What `step_to_glb.py` knows (and what F09 added to it)

`list` and `glb` as before. Added in F09, each off by default so a model already converted comes out byte for byte the same (the CORE One L is checked in the tests):
a STEP that names its occurrences `NAUO12` keeps the part name it has in `source_name` (the MINI); `"split_solids": true` in `groups.json` draws every solid of a flattened
compound as a part of its own (tried on the MK4 space model of Printables 961911, which turned out to have no heatbed: it is not carried); `"orient": [[..],[..],[..]]` turns a STEP
that is not Z-up into the display frame (the MINI's is Y-up with its display at the back).

## Where the calibration numbers come from

`evidence/scripts/` of the F09 work order holds `glbtool.mjs` (bounds per group, the lowest vertices of a part: the nozzle tip, in the CAD frame), `slices.mjs` and the logs of every
measurement. A model's sheet (the part the nozzle touches at Z0), its nozzle tip and the margins of the print area on the sheet are written in `calibration_basis` of its
profile; `tools/printer-print-frames.rs --prusa-only` re-derives the print frames from the calibration and refuses a model whose sheet is not where the calibration touches it.
