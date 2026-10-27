# Stella Scope 0.6.1

Released 2026-10-27.

- **Windows.** The first Windows build: `stella-scope_0.6.1_x64-setup.exe` for Windows 10 and 11 (64-bit). It installs the app, the recorder, the phone link and the printer models, and installs Microsoft's WebView2 if the computer does not have it.
- **Feedback.** All pages › Feedback (and More › Feedback on the farm wall) sends what you write to 3D3D. It is sent only when you press Send, with the app's version, the computer's system and, if you leave the box ticked, how many printers of each kind you run. The reply is a reference number.
- The Input shaper page's measurements no longer draw over its title; Health leaves out heat-ups that took no time.

# Stella Scope 0.6.0

Released 2026-10-26. Linux x86-64 (`stella-scope_0.6.0_amd64.deb`) and Android (`stella_0.6.0_arm64.apk`). Source: AGPL-3.0.

0.4 recorded one Klipper printer. 0.6 runs a farm of them, of three kinds, and adds calibration, material profiles, print reports with timelapses, and the machine's health over time.

## What is new since 0.4

### A farm, not one printer
- Many printers in one service, each with its own recording. The farm wall shows every printer as an instrument cluster: state, job, time left, temperatures, camera and model. The wall fits the window and scrolls on a phone.
- Three kinds of printer: Klipper through Moonraker, Prusa through PrusaLink (CORE One, CORE One L, MK4 family), FlashForge Adventurer 5M and 5M Pro through their local API.
- Printers are found on the network or added by address. Their passwords and access codes are kept in separate owner-only files.
- A demo farm of nine simulated printers (`--demo`, or Add a printer › Demo) that never connects to anything.
- The camera hub: one picture source per printer, shared by the wall, the cockpit and the timelapse.
- Alerts on the farm floor, a queue, spools, backup, and Discord or Slack notices.

### The cockpit
- A new look: clean instrument gauges, one family of keys, colour only for meaning. Accent, finish, dials and chips change on the fly from the Look page.
- Radial wheels for heat, fans, speed, flow and movement. Heat presets grouped by material family.
- The machine in 3D from open-licence printer models (26 printers) where they exist; the bed follows the printer's own Z direction; a view cube.
- Prusa control through PrusaLink: heat, move and the Prusa calibrations run as a command file the printer prints, only while it is idle.

### Calibrate
- A pre-print setup, then each calibration the printer supports, in order: PID for nozzle and bed, Z offset, gantry levelling, bed mesh, input shaper and belt comparison, rotation distance, pressure advance, flow. On a Prusa: self test, first layer, input shaper, phase stepping, nozzle cleaning.
- What each step found is kept as a record, and a result can be written into the material it belongs to.

### Materials
- Twenty materials on a fresh install, each editable: temperatures, density, price per kilogram, pressure advance, flow and fan.

### Print reports and timelapses
- One timelapse picture a layer for every print with a camera, kept for the newest 40 prints (3 GB at most).
- A report per print: how it ended, print time, filament in metres, weight and cost from the material, the plate, what the printer said while it ran, and the timelapse player.

### Health over time
- Belt match and X and Y resonance from each input-shaper run; each motor driver's current scaling and warnings; each heater's heat-up speed against its own usual.

### Phone
- The Android app pairs with a QR code and reaches the farm directly or through an encrypted relay. At most three phones, each bound to itself.

### Licence
- Free for up to five printers; demo printers never count. More than five needs a farm licence for the official builds, with a 30-day trial from the day the sixth printer is added. Terms: [FARM-LICENCE-TERMS.md](FARM-LICENCE-TERMS.md). Makers in the 3D3D network get a farm licence free.

## Known gaps
- No Windows or macOS build yet.
- Bambu Lab printers are not supported yet.
- Prusa control is new: it works only while the printer is idle, and runs each command as a small file the printer prints.
- The timelapse needs a camera the camera hub can read; a printer without one keeps a report without pictures.
- Motor health needs drivers that report their state (Klipper TMC drivers); Prusa and FlashForge printers do not.

## Upgrade from 0.4
Install the new package over the old one. The configuration and recordings are kept.
