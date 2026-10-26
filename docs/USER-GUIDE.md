# Stella Scope user guide

Stella Scope watches your 3D printers, one or a whole farm. It shows each printer live (temperatures, progress, camera and a 3D picture that moves with the print), keeps a queue of files for them, counts your spools, tells you when something needs you, and records everything so you can rewind. It works with Klipper printers (Voron and others), Prusa printers (PrusaLink) and FlashForge printers (Adventurer 5M). Nothing leaves your computer: there is no account and no cloud.

This guide is inside the program. Open it any time from the farm wall: More, Help.

## Install on Linux

Stella Scope comes as a `.deb` package for 64-bit Linux (Debian, Ubuntu and friends). In a terminal, in the folder that holds the file:

```
sudo apt install ./stella-scope_VERSION_amd64.deb
```

Replace `VERSION` with the number in the file name. This puts three programs in `/usr/bin` (`stella-scope`, the window `stella-desktop` and `stella-link`, the encrypted link to your phone) and a Stella Scope icon in your applications menu. It needs the web engine and window library most desktops already have (`libwebkit2gtk-4.1-0` and `libgtk-3-0`); `apt` fetches them if they are missing.

Settings are kept in `~/.config/ca.threedthreed.stellascope/` and recordings in `~/.local/share/ca.threedthreed.stellascope/`. To remove Stella Scope: `sudo apt remove stella-scope`. Your settings and recordings stay until you delete those two folders.

## First start

Open Stella Scope from the applications menu. The first time, one screen says Welcome and has one button, **Open Stella Scope**. Press it. It writes the settings and starts the program; no printer is added and nothing on your network is asked.

You land on the farm wall. With no printer yet, the wall shows one card with two buttons: **Try the demo farm** and **Add a printer**.

![The farm wall on a fresh install: one card that offers the demo farm or a first printer.](/help/img/fresh-wall.jpg)

## The demo farm

If you have no printer at hand, or want to look around first, press **Try the demo farm**. It adds nine simulated printers, each at its own place in a sample print: a Voron 2.4, a Prusa CORE One L, a Trident, a V0.2, a Micron+, an SV08, a toolchanger, an Ender-3 and a Prusa MK3S+. Every one is marked **DEMO**, nothing real is touched, and they never count toward the licence.

Their 3D models are separate downloads from their makers, under open licences that are shown first. The four Prusa models are already inside Stella Scope. When you are done, open More, Printers and press **Remove the demo farm**. Your real printers are not touched.

![The farm wall with the demo farm: every printer is marked DEMO and shows its state, progress, temperatures and time left.](/help/img/wall-demo.jpg)

Most other things are under **More**: printers, spools, analytics, the 3D models, alert settings, the slicer upload, backup, the wall screen, Help and About.

![The More menu of the farm wall, with Help and About at the end.](/help/img/more-menu.jpg)

Click a tile to open that printer. The top bar counts printers by state; press a count to see only those. The buttons at the top are the camera grid, the queue, the alerts bell, More and Add a printer. **Tiles** changes how big the tiles are, and the search box finds a printer by name, brand or state.

## Add a printer

Press **Add a printer**. It takes four steps and only reads from the printer until the last one.

1. **Find your printer.** Press **Search my network** and Stella Scope asks the devices on your network who they are, or type the printer's address yourself.
2. **Connect.** Type what that brand needs (below) and press **Test connection**. Nothing is saved yet.
3. **Name it** and give it a colour.
4. **Pick its 3D model**, or skip it. A printer with no model is shown by its picture or ring instead.

![Step 1: find your printer on the network, or type its address.](/help/img/add-printer-find.jpg)

### Klipper (Voron and others)

The address is the printer's Moonraker address, such as `http://192.168.1.50:7125`. You only need the **API key** if Moonraker asks for one. Stella Scope finds the camera by itself when the printer has one.

![Step 2 for a Klipper printer: the address and, only if Moonraker asks, an API key.](/help/img/add-printer-klipper.jpg)

### Prusa (PrusaLink)

For the CORE One, CORE One L, MK4, MINI and XL. The address is the printer's own, such as `http://192.168.1.60`. The **user name** (usually `maker`) and the **password** are in the printer's own settings, under PrusaLink. A Prusa Buddy3D camera is found on the network and offered; its address looks like `rtsp://192.168.1.61/live`. Stella Scope then asks which Prusa it is, to set the plate, the nozzle and the 3D model.

PrusaLink has no way to set a temperature, run the fan or home the printer directly. Stella Scope sends each of these as a one-line file that the printer runs by itself (it shows briefly on the printer's screen). This works only while nothing is printing; during a print PrusaLink cannot change them, and Stella Scope says so instead of sending anything.

![Step 2 for a Prusa: the printer's PrusaLink user name and password, and the camera address.](/help/img/add-printer-prusa.jpg)

### FlashForge (Adventurer 5M and 5M Pro)

Turn **LAN mode** on at the printer's screen first. The **check code** is shown there; capital and small letters matter. The serial number (it starts with SN) is optional.

![Step 2 for a FlashForge: the check code from the printer's screen.](/help/img/add-printer-flashforge.jpg)

If the printer does not answer, see Troubleshooting below.

## A printer's page

Open a printer and you see its page: the two heat dials, speed and flow, the fans, the 3D printer and a row of keys along the bottom. The rail on the left holds the other pages (Print, Camera, Print history, Alerts and more). Hover over any key to read what it does.

- **The dials.** Tap a heat dial for a wheel of your materials' temperatures, Off, or a typed number; press and hold, then turn, to set any value. A heater on its way up breathes until it reaches its target.
- **On the machine.** Small readings sit at the nozzle and the bed and move with them. Turn them off under More › Look.
- **The view cube.** The cube at the top right turns the 3D printer: press a face to look from that side, or the dot where the faces meet for the three-quarter view. The camera key in the top bar fills the screen with the live camera.
- **The keys.** Each key has a small light: it glows in your colour for the main action or the page that is open, red for stop and cancel.
- **The two pills at the top**, when nothing prints, say what filament is in and which plate is on the bed. Tap one to record what you loaded from your lists; nothing heats or moves.

### Calibrate

**Calibrate** on the rail sets a printer up, in the order that matters, and keeps what each step found and when:

1. **Nozzle heater** and **Bed heater**: a PID tune at the temperature you print at most.
2. **Z offset**: the paper test. Home first; the printer lowers to the middle of the bed, you step it down in fine steps until the paper drags, then Accept.
3. **Gantry level** and **Bed mesh**, on printers that have them.
4. **Input shaper**: opens the Input shaper page.
5. **Extruder steps**: mark the filament 120 mm above the extruder, feed 100 mm, measure what is left, and Stella works out the rotation distance.
6. **Pressure advance**: a tower print that raises it with height; type the height where the corners are sharpest.
7. **Flow**: a single-wall cube measured with calipers.

A result Klipper keeps only until it restarts is shown with **Save to printer.cfg** (SAVE_CONFIG, which restarts Klipper). **Before you print** at the top checks the plate, the filament, how old the mesh is, the Z offset, the heaters, and any result still waiting to be saved. Every step that heats or moves is shown to you first and never runs during a print.

A Prusa runs its calibrations itself, from its own screen (Selftest, First layer, Input shaper and so on): its Calibrate page lists them in order and keeps when each was done.

### Materials

**Materials** (All pages, or Edit the list on Heat) keeps each material as a card: nozzle, bed, part fan, its price a kilo and its density, and the pressure advance and flow that Calibrate measured for it. Recording a material as loaded (the Filament? pill, or a load on the Filament page) sets its pressure advance on a Klipper printer; its flow is for your slicer.

**First layer, in one go** (on Calibrate) heats for the loaded material, homes, levels the gantry, probes the mesh and sets the plate's Z offset, in order, each shown before it is sent.

### Print reports and timelapses

A print in **Print history** opens its report: how it ended, the print time, the filament in metres, grams and money (from the material's density and price), the material and the plate, what the printer reported while it ran, and its timelapse. Every print on a printer with a camera is kept as a timelapse, one picture a layer (the newest 40 prints); **Timelapses** under All pages plays any of them.

### Health

**Health** on the rail shows what changes slowly, over time: how alike the two belts answer (each Compare belts run), each axis's resonance (a belt going slack lowers it), each motor driver's current and any warning it raised, and how fast each heater heats up against its own usual.

### Look

More › **Look** (on a printer's page or the farm) changes how Stella Scope looks, at once, on this screen: the colour (Silver, Racing Yellow, Guards Red, Miami Blue, Python Green, Lava Orange, Ultraviolet or Stella Teal), the finish (Graphite, Carbon or Midnight), LED or smooth dials, and the readings on the machine.

![A printer's page: temperatures on the left, speed and flow on the right, the dock of buttons below.](/help/img/printer-page.jpg)

The box at the top left, **Find a command**, finds any control or page by its name: type `heat`, `move`, `speed`, `motors` or `failure watch` and press Enter. A control your printer does not have is named too, with "not on this printer". Press `/` to jump to the box.

![The command bar after typing motors: it offers the control by its plain name.](/help/img/command-bar.jpg)

The red **STOP** at the top right is the emergency stop. It cuts the heaters and the motors at once, with no question asked, so the printer needs a restart afterwards. To end a print, use **Cancel print** instead.

## Cameras

Give a printer a camera in its settings: open More, Printers, press **Settings** on the printer and fill in **Camera address** (it starts `rtsp://` or `http://`). Leave it empty for none. Klipper printers usually need nothing: their camera is found by itself.

The **Cameras** button shows every camera on one screen; press one for the full picture with its printer's state over it.

![Cameras: every printer's camera on one screen. These are test pictures from the test printers, not real cameras.](/help/img/cameras.jpg)

## The queue and the slicer

The queue holds sliced files waiting for a printer that fits them. Press **Queue**, then **Add files** and pick your sliced files (`.gcode`, `.bgcode` or `.gcode.3mf`). Each file is read for the printer, nozzle and material it was sliced for, and only goes to a printer that fits. You can set copies and priority, and send a file to a chosen printer.

![The queue: sliced files, the printers they fit and what happens next.](/help/img/queue.jpg)

To send files straight from your slicer, open More, **Slicer upload**. It shows an address and a key. In OrcaSlicer or PrusaSlicer add a physical printer with host type OctoPrint and paste both; **Upload** then puts the file in the queue. It never starts a print by itself.

![Slicer upload: the address and key to paste into the slicer (the key is hidden in this picture).](/help/img/slicer-upload.jpg)

## Plate check

The plate check makes sure the plate is empty before the next queued file starts. In the printer's settings, press **Set up the plate check**. Take everything off the plate, look at the picture, press **Capture the empty plate**, then drag the four corners onto the plate so only the plate is compared. Choose how closely it looks and save.

When a print is done, the printer's tile says **Plate needs clearing**. Take the print off and press **Plate cleared**: the camera checks the plate first, then the next file can start with **Start next**. If the check fails, it shows what the camera saw, and you can look again or start anyway when you can see the plate is clear. A printer with no camera has no check: the press alone releases the next file.

By default Stella Scope waits for you to press Start. In the printer's settings you can choose **Start next by itself** after a passing check.

![A printer's settings: the Set up the plate check button, and the choice between waiting for you to press Start and starting the next file by itself.](/help/img/printer-settings.jpg)

## Spools

Open More, **Spools**. Add each spool with its material, brand, colour, the grams on a full spool and a price per kilogram, and say which printer it is on. What is left on a spool drops by what each finished print on that printer used, and Stella Scope alerts you when a spool runs low. **Weigh** sets what is left from your scale.

![Spools: what is on each printer and how much is left.](/help/img/spools.jpg)

## Alerts and your phone

The bell, **Alerts**, lists everything that happened on every printer, newest first, and a number on it counts what is new. Pop-ups appear for print endings, plates to clear, offline printers and more; press one to open that printer.

To also be told away from the screen, open More, **Alert settings**. Choose what Stella Scope tells you about and where it goes: WhatsApp, ntfy (a free notification service), Discord or Slack. Each has a **Send a test**.

![Alert settings: what you are told about, and the places it goes.](/help/img/alert-settings.jpg)

To watch from your phone, open a printer, press **Pair phone** (or the Phone page). Step 1 is a code that opens the download page for the Stella app. Step 2 shows a pairing code: open the app, press **Scan the code** and point it at the screen. The phone then reaches this computer at home and away, through an encrypted link; there is no account. If the page says **Anywhere access is switched off**, open the settings file (`~/.config/ca.threedthreed.stellascope/stella-scope.json`), set `remote_link` to `true` and restart Stella Scope. The desktop window's first start does this for you.

![Pair a phone: the first code opens the download page of the Stella app, the second step shows the pairing code. This copy has anywhere access switched off, so the page says so.](/help/img/phone.jpg)

The optional **failure watch** looks at the camera every ten seconds for spaghetti and failed prints, warns you here and on WhatsApp, and never stops a print by itself. It is an add-on that is not in the package. It needs the `stella-eye` program (write to info@3d3d.ca for it) placed next to the recordings; the first time you switch it on, it downloads about 200 MB. Switch it on or off on a printer's page: View, **Failure watch**.

## Controls

Printer controls (heat, move, start, pause, cancel and the rest) are on once Stella Scope is set up from the desktop window. Each one that changes the printer is shown to you first, with the exact command, and goes out once. If a control says **Printer controls are switched off**, Stella Scope was started with controls off: open the settings file (`~/.config/ca.threedthreed.stellascope/stella-scope.json`), set `controls_enabled` to `true` and restart Stella Scope.

## Licence

Stella Scope is **free for up to five printers**. A **perpetual** licence for more is bought by Interac e-Transfer, and the payment details come with the invoice. To buy, write to **info@3d3d.ca** with your farm name and how many printers. 3D3D replies with an invoice. After payment you get one line of text that starts `SFL1.`; open More, Printers, press **Enter licence** and paste it. It is checked on your computer, with no internet, and it is not an account.

Demo printers never count. When you add a sixth real printer, a 30-day trial starts and a small note says a licence is wanted; close it and it stays quiet for a week. After the trial, the controls of the printers beyond the first five are locked until a licence covers them. Watching and recording go on, and a print that is running is never interrupted.

![Buy a farm licence: how to buy, in words, with no address to pay.](/help/img/licence-buy.jpg)

## Backup

Open More, **Backup**. **Save a backup** writes one file with your printers and their settings, the spools and prices, and which alerts are on. Passwords, keys, check codes and the recordings are left out. **Restore from a file** adds what a new computer or a fresh install does not have yet; what is already there is left alone.

The recordings are separate: to keep them, close Stella Scope and copy the data folder (`~/.local/share/ca.threedthreed.stellascope/`).

![Backup: save the farm to one file, or restore it.](/help/img/backup.jpg)

## About

More, **About** shows the version, who made Stella Scope and how to reach 3D3D (info@3d3d.ca), the licences of everything that is inside it, the licences of the 3D printer models and the 3D Print Rig credit with its MIT notice.

![About: version, licences, the 3D Print Rig credit and who to write to.](/help/img/about.jpg)

## Troubleshooting

### A printer is offline

A tile that says **Offline** or **Not answering** shows when it was **Last heard**. Check, in this order:

- The printer is on and on the same network as this computer. Open its address in a web browser on this computer to be sure.
- The address has not moved. Routers hand out new addresses; give the printer a fixed one. Open More, Printers, **Settings**: to change an address, remove the printer and add it again. Its recordings are kept.
- Prusa: the user name and password are the ones in the printer's PrusaLink settings. FlashForge: LAN mode is on and the check code is exact.
- A weak Wi-Fi link (2.4 GHz) makes a printer drop out now and then. It comes back by itself.

![A printer that is off: the tile says Offline and when it was last heard.](/help/img/printer-offline.jpg)

### The camera is black

- The camera address is right and starts `rtsp://` or `http://`. Open More, Printers, **Settings**.
- A Prusa Buddy3D camera shows nothing until it is switched on at the printer.
- If the picture is slow, the camera itself is set to a low frame rate or resolution.

### The 3D model is not drawn

A tile that says **3D model: not here yet** has its picture but not its model. Press **Get it** (or **Get the 3D models** on the wall) and Stella Scope downloads it from its maker, with the licence shown first. This is the only thing that needs the internet. A printer with no model in the list shows its picture or ring; choose a model in the printer's settings, or open More, **Printer models**. The Prusa CORE One L, CORE One, MK3S+ and MINI are already inside Stella Scope.

![A tile whose 3D model is not here yet, with the button that gets it.](/help/img/no-model.jpg)

### Something else

Write to info@3d3d.ca and say what you were doing and what you saw.
