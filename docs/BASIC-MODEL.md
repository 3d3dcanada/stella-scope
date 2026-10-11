# The basic printer model

Stella Scope draws the printer from a CAD assembly in the managed asset library that sits beside the
recording database (`<database>.assets/`). Without one, the cockpit still records and shows every
reading; it just has no 3D printer to put them on.

## Install it

```sh
tools/basic-model/install.sh /path/to/stella-scope.sqlite3
```

`stella-scope --print-config` shows which database your instance uses. The script downloads one
archive (about 12 MB, 57 MB unpacked) from this repository's releases and unpacks it into the
library. Reload the page afterwards.

## What it is

A stock Voron 2.4 R2 350, built from hartk1213's Voron 2.4 350 CAD assembly
([hartk1213/MISC](https://github.com/hartk1213/MISC/tree/main/Voron%20Mods/Voron%202-Trident/2.4/350_CAD_assembly),
GPL-3.0). `tools/basic-model/build.mjs` reduces it for display: fasteners and other small hardware
are removed, one hotend option (E3D V6) and the inductive probe are kept, and coordinates are rounded
to 0.01 mm. Nothing is moved or scaled. It is a reference printer, not a measured model of yours.

The model archive is distributed under GPL-3.0, as a derivative of that assembly.

## Build it yourself

1. Download the STEP assembly from the link above.
2. Convert it with the importer (see `tools/asset-import/README.md`, native OCP path for large files).
3. Reduce it:

   ```sh
   node tools/basic-model/build.mjs --geometry <library>/<id>/geometry.json --out ./basic-model
   tar -C ./basic-model -czf voron-24r2-350-basic.tar.gz manifest.json voron-24r2-350-basic
   ```

## Other printers

Today the cockpit knows one machine layout, the Voron 2.4. Its motion groups (gantry, toolhead, Z
drives, bed) are found from the part names in that assembly, in `web/hud-scene.js`
(`establishMotion`). Adding another printer means a converted assembly in the library plus a motion
map for its part names; contributions are welcome.
