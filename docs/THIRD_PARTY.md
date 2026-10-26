# What is inside Stella Scope, and under what terms

Stella Scope is free and open source under the **GNU Affero General Public License 3.0 (AGPL-3.0)**, made by 3D3D Canada (info@3d3d.ca). A farm
licence for more than five printers is a separate, commercial right sold by 3D3D (see `FARM-LICENCE-TERMS.md`). This page lists the code, fonts and
data made by other people that Stella Scope carries, and the terms each one is shared under. In the app it is under More, About, Licences.
Prusa Research, Voron Design, 3D Print Rig and the other makers named here are not connected with Stella Scope and have not approved it.

## The program

Stella Scope is written in Rust. It is built from 97 Rust crates (libraries), every one under a permissive licence: **none is GPL or AGPL**. The licence
of each is read from the crate's own `Cargo.toml`, at the exact version in `Cargo.lock` (tool: `docs/work-orders/finish-2-2026-10-19/evidence/scripts/f07-crate-licences.mjs`).
Where a crate offers a choice ("MIT OR Apache-2.0") Stella Scope takes the permissive one.

| crate | version | licence |
|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| base64 | 0.23.1 | MIT OR Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| cc | 1.4.6 | MIT OR Apache-2.0 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| curve25519-dalek | 4.1.3 | BSD-3-Clause |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| ed25519 | 2.2.3 | Apache-2.0 OR MIT |
| ed25519-dalek | 2.2.0 | BSD-3-Clause |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| fiat-crypto | 0.2.9 | MIT OR Apache-2.0 OR BSD-1-Clause |
| find-msvc-tools | 0.1.12 | MIT OR Apache-2.0 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 |
| foldhash | 0.1.5 | Zlib |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| hashlink | 0.10.0 | MIT OR Apache-2.0 |
| http | 1.5.0 | MIT OR Apache-2.0 |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| image | 0.25.10 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| libc | 0.2.189 | MIT OR Apache-2.0 |
| libsqlite3-sys | 0.35.0 | MIT |
| log | 0.4.34 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| qrcode | 0.14.1 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rusqlite | 0.37.0 | MIT |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 |
| rustls | 0.23.45 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.15 | ISC |
| semver | 1.0.28 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| shlex | 2.0.1 | MIT OR Apache-2.0 |
| signature | 2.2.0 | Apache-2.0 OR MIT |
| simd-adler32 | 0.3.10 | MIT |
| smallvec | 1.16.1 | MIT OR Apache-2.0 |
| subtle | 2.6.1 | BSD-3-Clause |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.5 | MIT OR Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| untrusted | 0.9.0 | ISC |
| ureq | 3.4.2 | MIT OR Apache-2.0 |
| ureq-proto | 0.6.3 | MIT OR Apache-2.0 |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 |
| vcpkg | 0.2.15 | MIT/Apache-2.0 |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| zeroize | 1.9.1 | Apache-2.0 OR MIT |
| zlib-rs | 0.6.8 | Zlib |
| zmij | 1.0.23 | MIT |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |

**SQLite** (inside `rusqlite`, the "bundled" feature) is in the public domain (https://sqlite.org/copyright.html). **ring** and **rustls** carry the licences listed
above; `webpki-roots` carries Mozilla's list of trusted certificate authorities under CDLA-Permissive-2.0.

## The phone link and the desktop window

The `stella-link` program (the encrypted link to your phone) is built from 363 crates and the `stella-desktop` window (Tauri) from 429. All are
permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, Unlicense), with a few under the Mozilla Public License 2.0, which asks only that changes to those files are shared.
The full lists are in the app: [phone link](/help?doc=crates-link), [desktop window](/help?doc=crates-desktop). The window uses the web engine your computer already has (WebKitGTK on Linux): it is not
carried in the package.

## In the pages of the app

- **three.js 0.186.0** and its `OrbitControls` and `STLLoader` (MIT, Copyright 2010-2026 three.js authors): draw the 3D printers. Licence text: `web/vendor/THREE-LICENSE.txt`.
- **Fonts**, all under the **SIL Open Font License 1.1** (licence text: `docs/licences/OFL-1.1.txt`):
  - Geist and Geist Mono: Copyright 2024 The Geist Project Authors (https://github.com/vercel/geist-font)
  - Archivo: Copyright 2020 The Archivo Project Authors (https://github.com/Omnibus-Type/Archivo)
  - IBM Plex Mono: Copyright 2017 IBM Corp. All rights reserved.
  (Each line is the one the font carries in its own file; `f07-font-notices.mjs` reads it.)

## 3D Print Rig (MIT)

The Printer models page and the catalogue of machines come from **3D Print Rig** (https://github.com/Psych0h3ad/3d-print-rig), by Psych0h3ad, under the MIT licence.
Stella Scope carries its catalogue (names, sizes, checksums, each machine's own licence) and its registered motion data for nine machines, and downloads a machine's CAD from
its author's own site only when you ask, with the licence shown first. The MIT notice, as the project publishes it, is in `docs/licences/3D-Print-Rig-MIT.txt` and in the app under About.
The project's licence covers its own viewer code and repository helpers, not the third-party CAD, which keeps its own terms (see `PRINTER-MODELS-NOTICE.md`).

## Failure watch (an add-on, not inside the package)

The optional failure watch downloads Obico's open detection model the first time you switch it on (about 200 MB, from Obico's own site). Obico's model and scoring are AGPL; they are not
part of the installer, and nothing of them is copied into Stella Scope.

## print-farm-manager (github.com/joeltelling/print-farm-manager)

MIT-licensed. Read for ideas for WO-15 (2026-10-13); the TV/wall-screen mode (`/farm?tv=1`) takes
its cue from that project. No code was copied; the feature is implemented in Stella Scope's own
style. The MIT licence requires the notice be kept when code is reused; only the idea was taken, so
this entry is the record of the read.

MIT licence text: <https://opensource.org/licenses/MIT> (Joel Telling / print-farm-manager).

## Bambuddy (github.com/maziggy/bambuddy)

AGPL. Read for feature ideas only; no code or assets taken.

## Prusa 3D models inside the build (finish-2 F09, 2026-10-19)

The converted CAD of the Prusa CORE One L and CORE One (Prusa Research, Open Community License v1), the i3 MK3S+ (Swesen, jzkmath, Leto, Prusa Research; GNU GPL / CC BY / CC0)
and the MINI (Luis, Prusa Research's printed parts under GPL-3.0; stated CC0) ships in the binary as separate data files (`models/`), never mixed into Stella Scope's code. Each has its
notice in `docs/notices/` and its licence text in `docs/licences/`; both are shown in the app (the model's sheet and About). Table, sources and checksums: `docs/PRINTER-MODELS-NOTICE.md`,
`tools/printer-cad/README.md`. Prusa Research is not connected with Stella Scope and has not approved it.
