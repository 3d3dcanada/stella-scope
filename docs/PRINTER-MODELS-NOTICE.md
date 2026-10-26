# Printer models: what is in this build and under what terms

Stella Scope's Printer models page lists the 67 machines of **3D Print Rig** (https://github.com/Psych0h3ad/3d-print-rig, MIT), its E3NG hotend changer, and the 209 options it registers for them (toolchangers, toolheads, add-ons), and can fetch their real CAD on the person's request. This build carries only the following of that project's material.

## Carried with the build

- **The catalogue** (`web/printer-models.json`): names, sizes, SHA-256 pins, where each file is published, and each machine's and each option's licence, author and source as stated in the project's own notices. Facts, no CAD. For an option it also carries where the project registers it on a machine (a place in millimetres), so the machine and its tool bank can be drawn together once both are on the person's computer.
- **Registered motion** (`web/printer-motion.json`): which parts move with X, Y and Z for 9 machines. This is 3D Print Rig's own data from `site/viewer/motion-profiles/` (MIT), kept with this notice.
- **Pictures** (`web/printer-thumbs.json`): a small picture of each machine whose licence class allows redistribution (open licences only, listed below). A picture of a non-commercial, mixed-terms or unlicensed machine is never included.

Of 3D Print Rig's material no CAD file of any machine or option is in this build: a model is fetched, checked against its published SHA-256 and kept in the person's own library, under its author's terms, which the page shows first. The one exception is the four Prusa models below, which ship inside the build.

## Four Prusa models inside the build (finish-2 F09, 2026-10-19)

2026-10-19: "We can share it on the application ... every Prusa printer should be available." The converted CAD of four Prusa machines is embedded in the binary (`models/*.glb.gz`, 24.7 MB together, made by `tools/printer-cad/build-model.sh` from the makers' STEP files) and put in the person's library when the service starts, with each model's notice (`docs/notices/`) and licence text (`docs/licences/`) shown in the model's sheet and under About.

| machine | maker | licence | source |
|---|---|---|---|
| Prusa CORE One L | Prusa Research | Open Community License v1 (OCL) | Printables 1520379 |
| Prusa CORE One | Prusa Research | Open Community License v1 (OCL) | Printables 1520471 |
| Prusa i3 MK3S+ / MK3S / MK3 | Swesen, jzkmath, Leto; Prusa Research's design | GNU GPL (Prusa, jzkmath), CC BY (Swesen), CC0 (Leto); passed on under the GPL | Printables 563056 |
| Prusa MINI / MINI+ | Luis; Prusa Research's printed parts | stated CC0; Prusa's printed parts GPL-3.0; passed on under the GPL | Printables 307054 |

The OCL lets the CAD be converted and passed on under the same licence and forbids selling machines or remixes made from it and systematic or automated mining of it. The converted files are passed on free of charge, with Prusa's notice, and are not sold; whether a build that is itself sold with a farm licence may carry them is a question for Prusa Research that the owner has been told about (finish-2 F09 report). Every other Prusa machine is listed under Printer models with the reason it has no 3D model yet (`src/prusa_machines.rs`).

## The machines whose pictures are carried

| machine | licence | author | source |
|---|---|---|---|
| DoomCube 2 · 350 / Native CAD | GPL-3.0 | DoomCube authors | 3D Print Rig community models |
| Satsuma 180 v1.0 / Native CAD | GPL-3.0 (original machine geometry) | Squirrelbrain | https://www.printables.com/model/487266-satsuma-180-v10 |
| SOVOL SV08 · 350 / Official CAD | GPL-3.0 | Copyright SOVOL and the original contributors | https://github.com/Sovol3d/SV08/tree/a60644875f8c756d20b3828c9416518b414b5491 |
| Antithesis Aether MK1.1 · Aluminum / Conch / Galileo 2 | GPL-3.0 | antithesisengineering | 3D Print Rig community models |
| Ender-3 · Creality / Original | GPL-3.0 | Creality3DPrinting | https://github.com/Creality3DPrinting/Ender-3 |
| SIBOOR S-BOOM · January 2026 / CoreXZ | GPL-3.0 | Lzhikai | https://github.com/Lzhikai/SIBOOR-S-BOOM-2026 |
| Positron V3.2.2 · LDO / 180 | CC-BY-SA-4.0 | Positron 3D Team, LDO Motors, Kralyn | 3D Print Rig community models |
| VORON Trident 250 · standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON Trident 300 · standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON Trident 350 · standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON Trident 500 × 500 × 250 · Custom | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON Trident 1000 × 1000 × 994 · Custom | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON Trident 350 × 350 × 125 · Half Z | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 250 · R2standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 250 · LDO CNC AWD reference | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 300 · R2standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 300 · LDO CNC AWD reference | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 350 · R2standard printed assembly | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 500 × 500 × 469 · Custom | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 1000 × 1000 × 969 · Custom | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 350 × 350 × 165 · Half Z | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V2.4 350 · LDO CNC AWD reference | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V0.2r1 120 · Mini Stealthburner | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| VORON V0.2 120 · Mini Stealthburner | GPL-3.0 | VoronDesign | 3D Print Rig community models |
| Micron 120 · R1 / AntHead / WWG2 / Revo Voron | GPL-3.0 | PrintersForAnts | 3D Print Rig community models |
| Micron Plus 180 · R1 RC8 / AntHead / WWG2 / Revo Voron | GPL-3.0 | PrintersForAnts | 3D Print Rig community models |

Voron, LDO and Micron machines are GPL-3.0 (VoronDesign, LDO Motors, PrintersForAnts); embedded vendor parts keep their own terms as listed in 3D Print Rig's `docs/THIRD_PARTY_NOTICES.md`.

## Options

An option is a set of files, each with its own licence; the class of the option is the strictest of its files. A file whose own statement is missing is `unlicensed`. Open classes (GPL, MIT, CC BY, CC BY-SA, CC0) may be shared with their notice; non-commercial, mixed, maker-licence and unlicensed options are fetched from their authors on request, with the licence shown first, and never shipped. The StealthChanger bank is in the second group because its Xol head is CC BY-NC-SA 4.0. See `docs/PRINT-RIG-INVENTORY.md` for every option with its class.

## Never carried

Rat Rig (CC BY-NC-SA), Annex K1 to K3 (Annex EULA, non-commercial), VzBot (CC BY-NC), Mercury, TicTac, The 100, Rook, LH Stinger, Crossant (non-commercial), SnakeOil XY and ReMorph (mixed terms), and every machine whose author states no licence (FYSETC V2.4 Pro, the SIBOOR Trident and V2.4, 247Zero). The page shows each of these with its licence and fetches it only after the person confirms they have read it.
