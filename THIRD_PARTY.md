# Third-party components

Stella Scope is AGPL-3.0. These components keep their own licences.

## Bundled in this repository

| Component | Where | Licence |
|---|---|---|
| three.js r186, OrbitControls, STLLoader | `web/vendor/` | MIT, see `web/vendor/THREE-LICENSE.txt` |
| Geist, Geist Mono, Archivo, IBM Plex Mono fonts | `web/fonts/` | SIL Open Font License 1.1, see `web/fonts/OFL.txt` |
| Voron 2 STL catalogue (file names, paths and links only, no geometry) | `web/service-catalog.json` | Links to [VoronDesign/Voron-2](https://github.com/VoronDesign/Voron-2) (GPL-3.0) |
| Three layers of a Voron Design Cube, sliced with OrcaSlicer | `tests/fixtures/cube-3-layers.gcode` | Test fixture; the cube is VoronDesign's calibration model (GPL-3.0) |

## Downloaded, not stored here

| Component | Used by | Licence |
|---|---|---|
| Basic printer model, derived from hartk1213's Voron 2.4 350 CAD assembly | `tools/basic-model/` | GPL-3.0 ([source](https://github.com/hartk1213/MISC)) |
| Obico failure-detection model weights | `eye/` (`--model`) | Obico's terms ([obico-server](https://github.com/TheSpaghettiDetective/obico-server)) |

## Code that follows another project

`eye/` follows Obico's open-source failure scoring (`obico-server`, AGPL-3.0): the prediction and
failure-detection logic and its first-generation parameters.

## Rust and npm dependencies

Fetched by Cargo and npm at build time, each under its own licence (mostly MIT or Apache-2.0):
`serde`, `serde_json`, `rusqlite` (bundles SQLite, public domain), `ureq`, `getrandom` and `qrcode`
for the recorder; `iroh`, `tokio` and `anyhow` for `link/`; `tract-onnx`, `image` and `ureq` for
`eye/`; `tauri` and its plugins for `desktop/` and `mobile/`. `tools/asset-import` uses
`occt-import-js` (LGPL-2.1) and three.js. Run `cargo tree` or `npm ls` for the full list.
