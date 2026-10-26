// Runs a model that follows its G-code (polish board WO-03, 2026-10-18): the printer page's stage and the farm wall's
// tiles both use this. It reads the print file once, draws the part growing on the plate (model-print.js), and
// moves the head between reports at render rate (model-follow.js), so a poll twice a second is a target, not a step.
// One shared frame loop serves every model on the page; the wall's tiles are drawn a few a frame, within a time budget.
import { parseGcode } from './hud-gcode.js';
import { HeadFollow, colorsOf } from './model-follow.js';
import { createPrintLayer } from './model-print.js';

// ---- the shared loop ---------------------------------------------------------------------------------------
// One loop serves every model on the page. A page's own model is drawn by its view as usual; the wall's tiles are held
// and drawn here, a few a frame: each tile's drawing costs the main thread 1 to 13 ms (a model is two or three million
// triangles in a thousand parts), so what is drawn in one frame is kept inside a budget, the tile that has waited longest
// first, and a tile is never drawn more than 20 times a second. A frame is therefore never the sum of nine draws.
const live = new Set(), all = new Set();
let raf = 0;
// F04 (2026-10-19): the budget is 4 ms of a frame (it was 8: with a build running beside the page, a frame that spent 8 ms here and 8 ms on style, paint and the
// rest missed its slot, 17 hitches in 25 s at 9 printers); a tile is drawn at most every TILE_GAP_MS, and the gap grows with the number of tiles that are live
// (about 14 ms each: seven tiles are each drawn ten times a second, not twenty) and again while the page is falling behind (`load`, below).
let BUDGET_MS = 4, TILE_GAP_MS = 50, hold = 0, load = 1, calm = 0;
/** The least time between two draws of one tile: its base gap, longer by 14 ms for each live tile (at most 160), times `load` (1 when the page keeps pace, up to 3). */
export const tileGapMs = (tiles, load = 1, base = 50) => Math.min(160, Math.max(base, tiles * 14)) * load;
/** What one frame's length says about the page: a frame over 26 ms (and under a second: a hidden tab is not slowness) raises the load by half a step, 90 calm
 *  frames lower it by a quarter; it stays between 1 and 3. Returns the new {load, calm}. */
export const nextLoad = (load, calm, frameMs, hidden = false) => {
 if (frameMs > 26 && frameMs < 1000 && !hidden) return { load: Math.min(3, load + 0.5), calm: 0 };
 if (calm + 1 > 90 && load > 1) return { load: Math.max(1, load - 0.25), calm: 0 };
 return { load, calm: calm + 1 };
};
const stats = { loops: 0, last: 0, gaps: [], cost: [], drawn: [] };
if (typeof window !== 'undefined') {
 window.stellaRuns = () => [...all];
 // The frame-time evidence tries other budgets without a rebuild.
 window.stellaRunTune = (budget, gap) => { BUDGET_MS = budget; TILE_GAP_MS = gap; };
 // The wall calls this after it has rewritten its tiles (every two seconds): the style and paint work that follows is a frame's
 // worth on its own, so no tile is drawn for the next frames and the two do not land in one.
 window.stellaPauseDraws = (frames) => { hold = Math.max(hold, frames); };
}
function loop() {
 raf = 0;
 const t0 = performance.now();
 if (stats.last) {
  const g = t0 - stats.last; stats.gaps.push(g); if (stats.gaps.length > 900) stats.gaps.shift();
  // falling behind (a frame well over 16.7 ms, the tab being in front): draw the tiles less often; calm again after 90 frames at pace
  ({ load, calm } = nextLoad(load, calm, g, document.hidden));
 }
 stats.last = t0; stats.loops++;
 const GAP = tileGapMs([...all].filter((r) => r.tile && r.visible).length, load, TILE_GAP_MS);
 for (const r of [...live]) {
  if (r.tile && t0 - r.stepped < GAP - 4) continue;
  r.stepped = t0;
  if (!r.step(t0)) live.delete(r);
 }
 // Draw the held views that changed, longest wait first, until the budget is spent (the first always draws).
 let spent = 0, drawn = 0, waiting = 0;
 const dirty = [...all].filter((r) => r.tile && r.visible && r.view.ext.isDirty()).sort((a, b) => a.drawnAt - b.drawnAt);
 if (hold > 0) { hold--; waiting += dirty.length; dirty.length = 0; }
 for (const r of dirty) {
  if (drawn && spent + r.cost > BUDGET_MS) { waiting++; continue; }
  if (t0 - r.drawnAt < GAP - 4) { waiting++; continue; }
  const c = r.view.ext.drawNow();
  r.drawnAt = t0; r.cost = r.cost ? 0.7 * r.cost + 0.3 * c : c; spent += c; drawn++; // timed by the frame, so a 50 ms gap is every third frame
 }
 stats.cost.push(performance.now() - t0); if (stats.cost.length > 900) stats.cost.shift();
 stats.drawn.push(drawn); if (stats.drawn.length > 900) stats.drawn.shift();
 if (live.size || waiting) { if (!raf) raf = requestAnimationFrame(loop); } else stats.last = 0;
}
const wake = (r) => { live.add(r); if (!raf) raf = requestAnimationFrame(loop); };
/** The loop's own numbers (gap between frames, the cost of our part of each, tiles drawn per frame), for the frame-time evidence. */
export const runStats = () => ({ loops: stats.loops, running: live.size, load, gaps: stats.gaps.slice(), cost: stats.cost.slice(), drawn: stats.drawn.slice() });

// ---- the file --------------------------------------------------------------------------------------------
// Files are read one at a time with a breath between, so nine tiles do not hold the page for one long moment.
let queue = Promise.resolve();
const later = (fn) => { const p = queue.then(fn, fn); queue = p.then(() => new Promise((r) => setTimeout(r, 40)), () => new Promise((r) => setTimeout(r, 40))); return p; };

// F04 (2026-10-19): the read and the parse happen in a worker (gcode-worker.js) so the page's thread is not held for 200 to 300 ms per file. Where a worker cannot be made
// (node, an old browser, a worker that fails to load) the page does it itself, as it always did.
let gw = null, gid = 0;
const gpend = new Map();
function gcodeWorker() {
 if (gw !== null) return gw || null;
 try {
  gw = new Worker('/gcode-worker.js', { type: 'module' });
  gw.onmessage = (e) => { const p = gpend.get(e.data.id); if (!p) return; gpend.delete(e.data.id); if (e.data.error) p.rej(Error(e.data.error)); else p.res(e.data); };
  gw.onerror = () => { const lost = [...gpend.values()]; gpend.clear(); gw = false; for (const p of lost) p.rej(Object.assign(Error('worker failed'), { noWorker: true })); };
 } catch { gw = false; }
 return gw || null;
}
async function readFile(prefix, file, maxSegments) {
 const w = typeof Worker !== 'undefined' ? gcodeWorker() : null;
 if (w) {
  try { return await new Promise((res, rej) => { const id = ++gid; gpend.set(id, { res, rej }); w.postMessage({ id, url: `${prefix}/print/gcode?filename=${encodeURIComponent(file)}`, maxSegments }); }); }
  catch (e) { if (!e.noWorker) throw e; }
 }
 const r = await fetch(`${prefix}/print/gcode?filename=${encodeURIComponent(file)}`, { cache: 'no-store' });
 if (!r.ok) throw Error((await r.json().catch(() => ({}))).error || 'The print file could not be read.');
 const text = await r.text();
 // The slicer writes the extruder colours in the settings block at the end of the file.
 return { parsed: parseGcode(text, { maxSegments }), colors: colorsOf(text) };
}

/** Run `view` (a model view from model-view.js). `prefix` is the printer's API prefix (`/api/p/<id>` or `/api`).
 *  Options: `tile` (drawn by the shared loop within its budget, a lighter print), `budget` (line segments of the print), `onFrame(frame)` called
 *  for each drawn frame. Feed it each `/model` answer with `update(st)`. */
export function runModel(view, { prefix, tile = false, budget = tile ? 60000 : 600000, maxSegments = tile ? 400000 : 1500000, onFrame = null } = {}) {
 const print = createPrintLayer(view.ext);
 const follow = new HeadFollow();
 const run = {
  view, tile, stepped: 0, drawnAt: 0, cost: 0,
  visible: true, file: null, loading: false, error: null, drawn: null, last: null, live: false, pose: false, pending: null,
  follow, print,
  setVisible(v) { run.visible = v; if (v && follow.recv !== null) wake(run); },
  /** The person is matching the model by hand: the head is theirs for now. */
  pause(on) { follow.paused = !!on; if (!on) wake(run); },
  step(now) {
   if (!run.visible) return false;
   const f = follow.frame(now);
   run.last = f;
   // The printer page shows where the head is even when the printer does not say (PrusaLink while printing): the followed place, labelled as such.
   if (!tile && f.position) globalThis.__stellaHead = { position: f.position, source: f.label, at: now };
   if (!tile) globalThis.__stellaRun = run;   // (read by the page checks: what of the print is drawn)
   if (f.groups) view.setGroups(f.groups);
   // A finished print shows all of it; one being read, what has been read.
   print.show(['complete', 'finished', 'cancelled'].includes(follow.state) ? null : f.byte);
   // The part is on the plate once the first of it shows (not while it is heating, with none of the file read): a page may say so then.
   if (run.pending && print.info()?.shown > 0) { document.dispatchEvent(new CustomEvent('stella-part-drawn', { detail: run.pending })); run.pending = null; }
   if (onFrame) onFrame(f);
   return f.moving;
  },
  update(st, now = performance.now()) {
   if (!st || !st.model) return;
   const j = st.job || {}, p = st.position || {};
   follow.spec = st.pose_spec || null;
   follow.cal = st.calibration || null;
   run.pose = !!(follow.spec && follow.cal);
   run.live = st.position_source === 'live';
   const position = [p.x, p.y, p.z].every((v) => typeof v === 'number') ? [p.x, p.y, p.z] : null;
   follow.report({ position, z: typeof p.z === 'number' ? p.z : null, source: st.position_source, byte: j.file_position, progress: j.progress, state: j.state, tool: st.pose?.tool ?? null, ageS: st.tick_age_s }, now);
   // The print on the plate: read the file once per job (and again when the printer starts another).
   if (j.file && j.file !== run.file && st.print_frame && !run.loading) {
    run.loading = true; run.error = null;
    later(() => readFile(prefix, j.file, maxSegments)).then((e) => {
     run.file = j.file; run.pending = null; follow.setParsed(e.parsed);
     run.drawn = print.set(e.parsed, st.print_frame, { budget, colors: e.colors });
     if (!run.drawn) run.error = 'This print file has no extruding moves to draw.';
     // A page's own model (not a wall tile) says the part is on the plate, but only once some of it shows (see step()).
     else if (!tile) run.pending = { file: j.file, layers: e.parsed.totalLayers || e.parsed.layers.length };
    }).catch((e) => { run.file = j.file; run.drawn = null; run.error = e.message; })
     .finally(() => { run.loading = false; wake(run); });
   } else if (!j.file && run.file) { run.file = null; run.drawn = null; run.pending = null; print.clear(); follow.setParsed(null); }
   wake(run);
  },
  stop() { live.delete(run); all.delete(run); print.dispose(); if (tile) view.ext.hold(null); },
 };
 all.add(run);
 // A tile's view is drawn by the loop above, a few a frame; the page's own is drawn by the view.
 if (tile) view.ext.hold(() => wake(run));
 return run;
}
