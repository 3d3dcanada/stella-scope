# Stella Scope NEXT state matrix

Run with Node 22 from the repository root:

```bash
node tools/next-matrix/run.mjs --states idle,printing --classes desktop,phone --paths /next,/ --out tools/next-matrix/review-YYYYMMDD
```

Without options, it covers all nine states and eight viewports on `/next`, writing to `docs/next-ui/matrix/<timestamp>/`. Use an output path under `tools/next-matrix/` while the Codex file-ownership rule is active. The output path must not already exist. The contact sheet is `index.html`; each shot has a PNG and JSON record of load time, console errors, visible buttons, undersized button boxes, and body dataset.

The runner reads the installed Stella Scope config and managed asset library. It creates a new database under `${TMPDIR:-/tmp}/next-matrix-<pid>/`, hard-links the asset library into that scratch directory, and removes both on exit, including interruption. It binds only loopback ports 7129 (GET-only fake Moonraker) and 7421 (one Stella Scope instance). It never contacts the printer, port 7420, or the live database. Every POST to the fake is refused. The fake and instance are stopped by their recorded PIDs after each state. The browser is launched from the cached Playwright Chromium, its PID is recorded, and it is closed after each state. No browser or instance is left between states.

Before compiling the fake and before each state, the runner checks `free -m` and takes `docs/next-ui/.heavy.lock`. It checks RAM again before starting the fake, instance, and browser. Below 2500 MB available, it waits 60 seconds and retries for up to 15 minutes. It builds `target/release/stella-scope` only if that binary is missing. A full `/` run on 2026-09-24 took **3712.4 seconds (61.9 minutes)** for 72 screenshots; available RAM before states was about 7.4–9.5 GB. Plan for at least the 2500 MB available gate plus time for software WebGL. The measured full-run output is `tools/next-matrix/full-20260924-1/index.html`.
