// A model that follows its G-code (polish board WO-03, 2026-10-18). Pure logic, no DOM and no three.js, so it runs
// under node as well as in the page. Three jobs:
//   1. poseFor: the service's pose arithmetic (src/printer_models.rs pose_for_tool), repeated here so the head can be
//      posed at render rate for any position, not only the one the last poll carried.
//   2. pathAt / buildPrint: where the head is in a parsed print file at a byte, and the print itself as line segments
//      in the model's display frame, thinned to a budget and ordered so that "up to this byte" is a draw range.
//   3. HeadFollow: between reports the head glides, never steps. Each report is a target; the head arrives at it as the
//      next report is expected (continuous position). A printer that does not report its position live (a demo printer,
//      PrusaLink) is followed along the file's own path by the byte it has read, so the head runs along the part and not
//      along the chord between two samples, and keeps running at the pace the file has been read at until the next report
//      says where it is (it never waits at a target). The reading says which of the two it is.
// Nothing here makes a number up: no file, no byte and no position means no movement is drawn, said in words.
import { offsetOf } from './model-glb.js';

const NAMES = ['x', 'y', 'z'];
const num = (v) => typeof v === 'number' && Number.isFinite(v);
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

// ---- 1. the pose ---------------------------------------------------------------------------------------------

const component = (spec, axes) => {
 if (typeof spec === 'number') return spec;
 if (typeof spec === 'string') { const i = NAMES.indexOf(spec); return i < 0 ? 0 : axes[i]; }
 if (spec && typeof spec === 'object') {
  const [k, f] = Object.entries(spec)[0] || [];
  const i = NAMES.indexOf(k);
  return (i < 0 ? 0 : axes[i]) * (num(f) ? f : 1);
 }
 return 0;
};

/** The model's pose for a position: `{axes, clamped, known, tool, groups}`, as the service computes it.
 *  `spec` is the pose spec (`axes`, `motions`, `rest_offsets_mm`, `tools`), `cal` the calibration
 *  (`offset_mm`, `sign`, `source`), `position` three numbers or null, `tool` the active tool or null. */
export function poseFor(spec, cal, position, tool = null) {
 const axes = [0, 0, 0], clamped = [];
 for (let i = 0; i < 3; i++) {
  const lo = spec.axes?.[NAMES[i]]?.[0] ?? 0, hi = spec.axes?.[NAMES[i]]?.[1] ?? 0;
  const reported = position?.[cal.source[i]];
  const raw = cal.sign[i] * (num(reported) ? reported : 0) + cal.offset_mm[i];
  const held = clamp(raw, Math.min(lo, hi), Math.max(lo, hi));
  if (Math.abs(held - raw) > 1e-9) clamped.push(NAMES[i]);
  axes[i] = held;
 }
 const groups = {};
 for (const [name, motion] of Object.entries(spec.motions || {})) {
  const rest = spec.rest_offsets_mm?.[name];
  groups[name] = [0, 1, 2].map((c) => component(motion[c], axes) + (rest?.[c] || 0));
 }
 if (num(tool) && Array.isArray(spec.tools)) {
  spec.tools.forEach((t, i) => {
   const motion = spec.motions?.[t.group];
   if (i === tool || !motion || !t.park) return;
   const at = axes.slice();
   for (let k = 0; k < 3; k++) {
    const v = t.park[NAMES[k]];
    if (num(v)) { const lo = spec.axes?.[NAMES[k]]?.[0] ?? 0, hi = spec.axes?.[NAMES[k]]?.[1] ?? 0; at[k] = clamp(v, Math.min(lo, hi), Math.max(lo, hi)); }
   }
   const rest = spec.rest_offsets_mm?.[t.group];
   groups[t.group] = [0, 1, 2].map((c) => component(motion[c], at) + (rest?.[c] || 0));
  });
 }
 return { axes: { x: axes[0], y: axes[1], z: axes[2] }, clamped, known: Array.isArray(position) && position.every(num), tool, groups };
}

/** Two sets of group offsets blended: `u` 0 is `a`, 1 is `b`. A group only one of them has is taken as it is. */
export function blendGroups(a, b, u) {
 if (u >= 1) return b;
 const out = {};
 for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) {
  const x = a[k] || b[k], y = b[k] || a[k];
  out[k] = [0, 1, 2].map((c) => x[c] + (y[c] - x[c]) * u);
 }
 return out;
}

// ---- 2. the file ---------------------------------------------------------------------------------------------

/** A point of a print file in the model's frame (millimetres): `origin + x*axes[0] + y*axes[1] + z*axes[2]`. */
export function printPoint(frame, x, y, z) {
 const o = frame.origin_mm, a = frame.axes;
 return [0, 1, 2].map((c) => o[c] + x * a[0][c] + y * a[1][c] + z * a[2][c]);
}

const lastAtOrBefore = (arr, v, lo = 0, hi = arr.length) => {
 // The last index in [lo, hi) with arr[i] <= v, or lo - 1 when there is none.
 let a = lo, b = hi;
 while (a < b) { const m = (a + b) >> 1; if (arr[m] <= v) a = m + 1; else b = m; }
 return a - 1;
};

/** Where the head is when the printer has read `byte` bytes of the file: `{x, y, z, layer, segment}` in the file's own
 *  coordinates, along the moves (travel included), or null when the file has no moves. Inside one move the position
 *  runs with the bytes of the lines between this move and the next. */
export function pathAt(parsed, byte) {
 const layers = parsed?.layers;
 if (!layers?.length || !num(byte)) return null;
 const starts = parsed.byteOffsets;
 let l = Math.max(0, lastAtOrBefore(starts, byte));
 // A layer with no moves of its own cannot hold the head (the parser drops those), but stay inside the file.
 l = Math.min(l, layers.length - 1);
 const layer = layers[l], offs = layer.offsets, n = offs.length;
 if (!n) return null;
 // The first segment of this layer's group of equal offsets (an arc is several segments of one line).
 let k = Math.max(0, lastAtOrBefore(offs, byte));
 const here = offs[k];
 const first = lastAtOrBefore(offs, here - 1) + 1;
 let last = k; while (last + 1 < n && offs[last + 1] === here) last++;
 const nextOff = last + 1 < n ? offs[last + 1] : (l + 1 < layers.length ? starts[l + 1] : (parsed.byteLength ?? here + 1));
 const span = Math.max(1, nextOff - here), f = clamp((byte - here) / span, 0, 1);
 // Within the group, f walks the sub-segments in order.
 const count = last - first + 1, at = Math.min(count - 1, Math.floor(f * count)), sub = f * count - at;
 k = first + at;
 const s = layer.segments, i = k * 6;
 const u = count === 1 ? f : clamp(sub, 0, 1);
 return { x: s[i] + (s[i + 3] - s[i]) * u, y: s[i + 1] + (s[i + 4] - s[i + 1]) * u, z: s[i + 2] + (s[i + 5] - s[i + 2]) * u, layer: l, segment: k, u };
}

// ---- along the file by distance -----------------------------------------------------------------------------
// A byte names the move the printer is in, not where in it: a long move is one byte for as long as it takes. When the
// printer also says where the head is (a demo printer does), the head's place on the file's path is the distance along
// the path, found by putting the reported position on the move the byte names; the head then runs on along the path at
// the pace it has had, through the long moves as through the short ones.

/** Cumulative lengths of the moves (travel included), per layer and across layers, built once per parsed file. */
function pathIndex(parsed) {
 if (parsed._pi) return parsed._pi;
 const L = parsed.layers, base = new Float64Array(L.length + 1), cum = [];
 for (let l = 0; l < L.length; l++) {
  const sg = L[l].segments, n = L[l].offsets.length, c = new Float64Array(n + 1);
  for (let k = 0; k < n; k++) { const i = k * 6; c[k + 1] = c[k] + Math.hypot(sg[i + 3] - sg[i], sg[i + 4] - sg[i + 1], sg[i + 5] - sg[i + 2]); }
  cum.push(c); base[l + 1] = base[l] + c[n];
 }
 return (parsed._pi = { base, cum });
}

/** Distance along the whole path at fraction `u` of move `k` of layer `l`. */
export function arcOf(parsed, l, k, u) {
 const { base, cum } = pathIndex(parsed), c = cum[l];
 return base[l] + c[k] + u * (c[k + 1] - c[k]);
}

/** The place at distance `s` along the path: `{x, y, z, layer, segment, u}`. */
export function atArc(parsed, s) {
 const { base, cum } = pathIndex(parsed), L = parsed.layers;
 const l = clamp(lastAtOrBefore(base, s, 0, L.length), 0, L.length - 1), c = cum[l], n = c.length - 1;
 const k = clamp(lastAtOrBefore(c, s - base[l], 0, n), 0, n - 1);
 const len = c[k + 1] - c[k], u = len > 0 ? clamp((s - base[l] - c[k]) / len, 0, 1) : 0, sg = L[l].segments, i = k * 6;
 return { x: sg[i] + (sg[i + 3] - sg[i]) * u, y: sg[i + 1] + (sg[i + 4] - sg[i + 1]) * u, z: sg[i + 2] + (sg[i + 5] - sg[i + 2]) * u, layer: l, segment: k, u };
}

/** Where a point is on move `k` of layer `l`: the fraction along it that is nearest, and how far the point is from the move. */
export function project(parsed, l, k, x, y, z) {
 const sg = parsed.layers[l].segments, i = k * 6, dx = sg[i + 3] - sg[i], dy = sg[i + 4] - sg[i + 1], dz = sg[i + 5] - sg[i + 2];
 const len2 = dx * dx + dy * dy + dz * dz;
 const u = len2 > 0 ? clamp(((x - sg[i]) * dx + (y - sg[i + 1]) * dy + (z - sg[i + 2]) * dz) / len2, 0, 1) : 0;
 return { u, dist: Math.hypot(sg[i] + dx * u - x, sg[i + 1] + dy * u - y, sg[i + 2] + dz * u - z) };
}

/** The byte at fraction `u` of move `k` of layer `l` (the lines from this move to the next). */
export function byteAt(parsed, l, k, u) {
 const offs = parsed.layers[l].offsets, here = offs[k];
 let last = k; while (last + 1 < offs.length && offs[last + 1] === here) last++;
 const next = last + 1 < offs.length ? offs[last + 1] : (l + 1 < parsed.layers.length ? parsed.byteOffsets[l + 1] : (parsed.byteLength ?? here + 1));
 return here + u * (next - here);
}

/** The print as line segments in the model's display frame (metres), for a three.js LineSegments.
 *  `frame` is the machine's print frame, `basis` the machine's basis. `budget` caps the segments: layers are
 *  dropped evenly (never the last) and chains of short moves are merged until the count fits. Segments come out
 *  in file order, each with the byte of the move it ends in, so `countAt(byte)` is a draw range.
 *  Returns {positions, colors, bytes, segments, kept, layersKept, tools} with `bytes[i]` the byte of segment i. */
export function buildPrint(parsed, frame, basis, { budget = Infinity, colors = DEFAULT_COLORS, light = [-0.55, -0.8] } = {}) {
 const layers = parsed.layers;
 let total = 0;
 for (const l of layers) for (let i = 0; i < l.extrude.length; i++) total += l.extrude[i];
 const keepLayers = budget === Infinity || total <= budget ? 1 : Math.min(8, Math.ceil(total / budget / 2));
 const kept = layers.filter((_, i) => i % keepLayers === 0 || i === layers.length - 1);
 let keptTotal = 0;
 for (const l of kept) for (let i = 0; i < l.extrude.length; i++) keptTotal += l.extrude[i];
 // After dropping layers, merge chains of moves shorter than minLen so the rest fits.
 const length = parsed.totalLength * (kept.length / Math.max(1, layers.length));
 const minLen = budget === Infinity || keptTotal <= budget ? 0 : Math.min(8, length / budget);
 const cap = keptTotal + 16; // merging only ever makes fewer segments, so this is room enough
 const pos = new Float32Array(cap * 6), col = new Float32Array(cap * 6), bytes = new Int32Array(cap);
 const tools = new Set();
 const b = parsed.bounds, cx = (b.min[0] + b.max[0]) / 2, cy = (b.min[1] + b.max[1]) / 2, zTop = Math.max(1e-6, b.max[2]);
 const rgb = colors.map((h) => [(h >> 16 & 255) / 255, (h >> 8 & 255) / 255, (h & 255) / 255]);
 const ll = Math.hypot(light[0], light[1]) || 1, lx = light[0] / ll, ly = light[1] / ll;
 let n = 0;
 const emit = (x0, y0, z0, x1, y1, z1, tool, byte) => {
  if (n >= cap) return;
  const p0 = offsetOf(basis, printPoint(frame, x0, y0, z0)), p1 = offsetOf(basis, printPoint(frame, x1, y1, z1));
  pos.set(p0, n * 6); pos.set(p1, n * 6 + 3);
  // A baked light: the side of the part that faces the light is bright, the far side dark, a little darker low down.
  const base = rgb[tool % rgb.length];
  const mx = (x0 + x1) / 2 - cx, my = (y0 + y1) / 2 - cy, r = Math.hypot(mx, my) || 1;
  const lit = 0.5 + 0.5 * ((mx / r) * lx + (my / r) * ly);
  const shade = (0.55 + 0.45 * lit) * (0.82 + 0.18 * Math.min(1, z1 / zTop));
  for (let v = 0; v < 2; v++) { col[n * 6 + v * 3] = base[0] * shade; col[n * 6 + v * 3 + 1] = base[1] * shade; col[n * 6 + v * 3 + 2] = base[2] * shade; }
  bytes[n] = byte;
  tools.add(tool);
  n++;
 };
 for (const layer of kept) {
  const s = layer.segments, e = layer.extrude, o = layer.offsets, t = layer.tool;
  let cur = null; // the chain being merged: start, end, tool, byte of its last move
  const flush = () => { if (cur) { emit(cur[0], cur[1], cur[2], cur[3], cur[4], cur[5], cur[6], cur[7]); cur = null; } };
  for (let i = 0; i < e.length; i++) {
   if (!e[i]) { flush(); continue; }
   const k = i * 6, tool = t ? t[i] : 0;
   if (cur && cur[6] === tool && Math.abs(s[k] - cur[3]) < 1e-3 && Math.abs(s[k + 1] - cur[4]) < 1e-3 && Math.abs(s[k + 2] - cur[5]) < 1e-3) {
    cur[3] = s[k + 3]; cur[4] = s[k + 4]; cur[5] = s[k + 5]; cur[7] = o[i];
   } else { flush(); cur = [s[k], s[k + 1], s[k + 2], s[k + 3], s[k + 4], s[k + 5], tool, o[i]]; }
   if (cur && Math.hypot(cur[3] - cur[0], cur[4] - cur[1], cur[5] - cur[2]) >= minLen) flush();
  }
  flush();
 }
 return { positions: pos.slice(0, n * 6), colors: col.slice(0, n * 6), bytes: bytes.slice(0, n), segments: total, kept: n, layersKept: kept.length, tools: [...tools].sort() };
}

/** How many segments of a built print are drawn when the printer has read `byte` bytes. */
export function countAt(print, byte) {
 return num(byte) ? lastAtOrBefore(print.bytes, byte) + 1 : print.kept;
}

/** The colours of a print's tools: the file's own `extruder_colour` when it names them, else the brand teal and orange. */
export const DEFAULT_COLORS = [0x04d9c4, 0xff6a00, 0x18e36b, 0xff2d92, 0x3d8bff, 0xb58cff];
export function colorsOf(text) {
 const m = /^;\s*extruder_colour\s*=\s*(.+)$/m.exec(text || '');
 if (!m) return DEFAULT_COLORS;
 const list = m[1].split(';').map((s) => /^"?#([0-9a-f]{6})"?$/i.exec(s.trim())).map((x) => (x ? parseInt(x[1], 16) : null));
 return list.length && list.every((c) => c !== null) ? list : DEFAULT_COLORS;
}

// ---- 3. the head between reports ------------------------------------------------------------------------------

const lerp = (a, b, u) => a + (b - a) * u;
const lerp3 = (a, b, u) => [lerp(a[0], b[0], u), lerp(a[1], b[1], u), lerp(a[2], b[2], u)];

/** The head of one model. Feed it every report (`report`), ask it for any moment (`frame`). Times are milliseconds
 *  on one clock (performance.now()). A report: {position: [x,y,z]|null, source: 'live'|'file', byte, progress, state,
 *  tool, ageS}. */
export class HeadFollow {
 constructor({ spec = null, calibration = null, parsed = null, toolBlendMs = 450 } = {}) {
  this.spec = spec; this.cal = calibration; this.parsed = parsed; this.toolBlendMs = toolBlendMs;
  this.dur = 700;            // the gap between reports, as measured
  this.rate = 0; this.last = null; // how fast the file is being read, and the report before
  this.arc = null; this.arcRate = 0; this.lastArc = null; // the same along the path by distance, when the position can be put on it
  this.recv = null; this.stamp = null;
  this.pos = null;           // {from, to, t0}
  this.bytes = null;         // {from, to, t0}
  this.state = ''; this.source = 'file'; this.tool = null; this.prevTool = null; this.toolAt = -1e9;
  this.paused = false;       // the person is matching the model by hand: no movement is drawn
  this.liveZ = null;         // the printer's own Z when it reports one
 }

 setParsed(parsed) { this.parsed = parsed; }

 /** A report from the service. A repeat of the last one (same moment) changes nothing. */
 report(rep, now) {
  const stamp = now - (num(rep.ageS) ? rep.ageS * 1000 : 0);
  const fresh = this.stamp === null || stamp > this.stamp + 1;
  if (!fresh) return false;
  const here = this.sample(now); // where the head is now, before the gap between reports is re-measured
  if (this.recv !== null) this.dur = clamp(0.7 * this.dur + 0.3 * (now - this.recv), 120, 2500);
  this.recv = now; this.stamp = stamp;
  if (num(rep.tool) && rep.tool !== this.tool) { this.prevTool = this.tool; this.tool = rep.tool; this.toolAt = this.prevTool === null ? -1e9 : now; }
  this.state = rep.state || ''; this.source = rep.source === 'live' ? 'live' : 'file';
  // 2026-10-25 (CORE One L): PrusaLink says where Z is even while it hides X and Y. The bed follows that live Z, never the file's:
  // heating and wiping at Z 150, the file's first layer would have drawn the bed up at the nozzle while the real one went down.
  this.liveZ = num(rep.z) ? rep.z : null;
  if (Array.isArray(rep.position) && rep.position.every(num)) {
   this.pos = { from: here.position || rep.position, to: rep.position, t0: now };
  }
  const total = this.parsed?.byteLength;
  const byte = num(rep.byte) ? rep.byte : (num(rep.progress) && total ? Math.floor(total * clamp(rep.progress, 0, 1)) : null);
  if (num(byte)) {
   // A new print, or the same one again, starts over with no glide backwards; a head a little ahead of the report (it ran on at pace)
   // simply waits for the file to catch up, so the slack is a report's worth of pace.
   // A print always reads byte 0 first (it is heating), so a 0 after a file has been read is a start over as well.
   const back = here.byte !== null && (byte < here.byte - (this.rate * this.dur * 1.5 + 64) || (byte === 0 && here.byte > 0));
   // The pace the file is being read at, from this report and the one before (bytes a millisecond of the printer's own time).
   const gap = this.last && stamp > this.last.stamp ? stamp - this.last.stamp : 0;
   if (back || !gap || byte < this.last.byte) this.rate = 0;
   else this.rate = this.rate ? 0.5 * this.rate + 0.5 * ((byte - this.last.byte) / gap) : (byte - this.last.byte) / gap;
   this.last = { byte, stamp };
   // Glide to where the file will have been read by the next report, so the head runs on at pace rather than waiting
   // at this one; never backwards.
   const from = back || here.byte === null ? byte : here.byte;
   this.bytes = { from, to: Math.max(from, byte + this.rate * this.dur), t0: now };
   this.trackArc(rep, byte, stamp, here, now);
  }
  return true;
 }

 /** Put the reported position on the move the byte names; when it lies on it, the head is followed by distance along the path. */
 trackArc(rep, byte, stamp, here, now) {
  let arc = null;
  if (this.parsed && Array.isArray(rep.position) && rep.position.every(num)) {
   // The byte a printer reports is the end of the line it is in (a demo printer) or a little ahead of the head (Klipper reads
   // ahead), so the head is on one of the moves just before it: the nearest of those, within a millimetre, is where it is.
   const at = pathAt(this.parsed, byte);
   if (at) {
    const n = this.parsed.layers[at.layer].offsets.length;
    let best = null;
    for (let k = Math.max(0, at.segment - 6); k <= Math.min(n - 1, at.segment + 2); k++) {
     const pr = project(this.parsed, at.layer, k, rep.position[0], rep.position[1], rep.position[2]);
     if (!best || pr.dist < best.dist) best = { ...pr, k };
    }
    if (best && best.dist < 1) arc = arcOf(this.parsed, at.layer, best.k, best.u);
   }
  }
  if (arc === null) { this.arc = null; this.arcRate = 0; this.lastArc = null; return; }
  const gap = this.lastArc && stamp > this.lastArc.stamp ? stamp - this.lastArc.stamp : 0;
  const back = here.arc != null && arc < here.arc - (this.arcRate * this.dur * 1.5 + 4);
  if (back || !gap || arc < this.lastArc.arc) this.arcRate = 0;
  else this.arcRate = this.arcRate ? 0.5 * this.arcRate + 0.5 * ((arc - this.lastArc.arc) / gap) : (arc - this.lastArc.arc) / gap;
  this.lastArc = { arc, stamp };
  const from = back || here.arc == null ? arc : here.arc;
  this.arc = { from, to: Math.max(from, arc + this.arcRate * this.dur), t0: now };
 }

 sample(now) {
  const u = (g) => clamp((now - g.t0) / this.dur, 0, 1);
  // A file being read keeps running at pace past the glide's end (a report that is late), for as long again as the glide took.
  const byte = !this.bytes ? null : lerp(this.bytes.from, this.bytes.to, u(this.bytes))
   + (this.rate > 0 ? this.rate * clamp(now - this.bytes.t0 - this.dur, 0, this.dur) : 0);
  const arc = !this.arc ? null : lerp(this.arc.from, this.arc.to, u(this.arc))
   + (this.arcRate > 0 ? this.arcRate * clamp(now - this.arc.t0 - this.dur, 0, this.dur) : 0);
  return {
   position: this.pos ? lerp3(this.pos.from, this.pos.to, u(this.pos)) : null,
   byte, arc,
   settled: (!this.pos || u(this.pos) >= 1) && (!this.bytes || u(this.bytes) >= 1),
  };
 }

 /** The whole picture at `now`: where the head is, which tool, the groups to pose, what to call it. */
 frame(now) {
  const s = this.sample(now);
  // A print being read is followed along the file; any other state (paused, done, idle) glides to the reported spot.
  const along = this.source === 'file' && this.state === 'printing' && this.parsed && s.byte !== null;
  let position = s.position, layer = null, byte = s.byte;
  if (along) {
   // By distance along the path when the position could be put on it; by the byte when it could not.
   const at = s.arc !== null ? atArc(this.parsed, s.arc) : pathAt(this.parsed, s.byte);
   if (at) {
    position = [at.x, at.y, num(this.liveZ) ? this.liveZ : at.z]; layer = at.layer;
    if (s.arc !== null) byte = byteAt(this.parsed, at.layer, at.segment, at.u);
   }
  }
  const blend = clamp((now - this.toolAt) / this.toolBlendMs, 0, 1);
  let groups = null;
  if (this.spec && this.cal && position && !this.paused) {
   groups = poseFor(this.spec, this.cal, position, this.tool).groups;
   if (blend < 1 && num(this.prevTool)) groups = blendGroups(poseFor(this.spec, this.cal, position, this.prevTool).groups, groups, blend);
  }
  // Keep asking for frames while the head is moving along the file, gliding to a target, or swapping tools; stop a
  // few seconds after the last report (a printer that went quiet is not moving).
  const quiet = this.recv === null || now - this.recv > 4000;
  const moving = !quiet && (along || !s.settled || blend < 1);
  return { position, byte, layer, tool: this.tool, groups, along, moving, label: this.source === 'live' ? 'live' : 'following the file' };
 }
}
