// Pure-logic G-code toolpath parser for the HUD. No DOM, no three.js, no imports.
//
//   parseGcode(text, {maxSegments}) -> {layers, objects, bounds, totalLength, travelLength,
//                                       byteOffsets, byteLength, segmentCount, totalLayers, truncated}
//   progressToLayer(parsed, fileOffset) -> {layer, fraction}
//
// Each layer carries `segments` as a packed Float32Array of x0,y0,z0,x1,y1,z1 and `extrude` as a
// Uint8Array with one flag per segment (1 = extruding move, 0 = travel); `tool` (polish WO-03,
// 2026-10-18) is the tool number (T0, T1...) each segment was made with, for a part in two colours.
// `byteOffsets[i]` is the
// UTF-8 byte offset in the file where layer i starts, so Moonraker's `virtual_sdcard.file_position`
// maps straight onto a layer and a fraction inside it.
//
// `bounds` and `totalLength` describe the extruded path only, so a purge move to the bed edge does
// not blow up the view; travel is counted separately in `travelLength`. A file with nothing extruded
// reports zero bounds. `truncated` is true when `maxSegments` stopped the parse early.
//
// Deliberate limits: arcs are treated as XY (G17) only — G18/G19 planes are approximated in XY;
// `(...)` style comments are not recognised, since no FDM slicer emits them.

const ARC_CHORD_MM = 0.5; // target chord length when flattening G2/G3
const ARC_MAX_STEPS = 1000; // cap so a malformed radius cannot spin the parser
// With no layer markers anywhere, a layer is a Z step of at least this much between one extruding
// move and the next. Below the thinnest sane FDM layer, and far above the per-move climb of a
// spiral (vase mode) print, which is one continuous layer rather than thousands of slivers.
const FALLBACK_LAYER_MM = 0.02;

// UTF-8 byte length of a string. Moonraker's file_position counts bytes, not characters.
function utf8Length(s) {
 let n = 0;
 for (let i = 0; i < s.length; i++) {
  const c = s.codePointAt(i);
  if (c > 0xffff) { i++; n += 4; } else if (c > 0x7ff) n += 3; else if (c > 0x7f) n += 2; else n++;
 }
 return n;
}

// One G-code line's words, read into fixed slots so a million lines allocate nothing extra.
// A word is a letter followed by a number, with or without separating whitespace (G1X1Y2 is legal).
function lex(code, from, out) {
 out.X = out.Y = out.Z = out.E = out.I = out.J = out.R = NaN;
 let i = from;
 while (i < code.length) {
  let c = code.charCodeAt(i);
  if (c <= 32) { i++; continue; }
  if (c >= 97 && c <= 122) c -= 32;
  if (c < 65 || c > 90) { i++; continue; }
  i++;
  const start = i;
  const sign = code.charCodeAt(i);
  if (sign === 43 || sign === 45) i++;
  while (i < code.length) {
   const d = code.charCodeAt(i);
   if ((d >= 48 && d <= 57) || d === 46) i++; else break;
  }
  if (i === start) continue;
  const value = parseFloat(code.slice(start, i));
  switch (c) {
   case 88: out.X = value; break;
   case 89: out.Y = value; break;
   case 90: out.Z = value; break;
   case 69: out.E = value; break;
   case 73: out.I = value; break;
   case 74: out.J = value; break;
   case 82: out.R = value; break;
   default: break;
  }
 }
}

// Integer that follows a leading G or M, or NaN when the token is a macro such as GCODE_SOMETHING.
function commandNumber(code) {
 let i = 1;
 const start = i;
 while (i < code.length) {
  const d = code.charCodeAt(i);
  if (d >= 48 && d <= 57) i++; else break;
 }
 if (i === start) return NaN; // GCODE_SOMETHING is a macro, not a G command
 if (i < code.length && code.charCodeAt(i) === 46) return NaN; // G28.1 and friends are not handled
 return parseInt(code.slice(start, i), 10);
}

function macroValue(up, key) {
 const at = up.indexOf(key);
 if (at === -1) return '';
 let i = at + key.length;
 const start = i;
 while (i < up.length && up.charCodeAt(i) > 32) i++;
 return up.slice(start, i);
}

export function parseGcode(text, { maxSegments = 2_000_000 } = {}) {
 const source = typeof text === 'string' ? text : String(text ?? '');
 const ascii = !/[^\x00-\x7f]/.test(source);
 const lineBytes = ascii ? (s) => s.length : utf8Length;

 // Growable packed stores. Layers take a compact copy of their own slice when they close.
 let capacity = 1024;
 let seg = new Float32Array(capacity * 6);
 let ext = new Uint8Array(capacity);
 let off = new Int32Array(capacity);
 let tl = new Uint8Array(capacity); // tool of each segment (polish WO-03)
 let tool = 0; // the tool the last T command picked
 let count = 0;

 const layers = [];
 const byteOffsets = [];
 const objects = [];
 let open = -1; // index into objects of the currently open EXCLUDE_OBJECT
 let cur = null; // {z, zExplicit, start, count, offset}
 let sawMarker = false;
 let inArc = false; // a single G2/G3 is one command, so it never splits a fallback layer
 let lastExtrudeZ = null;
 let truncated = false;
 let totalLayers = null;
 let totalLength = 0;
 let travelLength = 0;
 let minX = Infinity, minY = Infinity, minZ = Infinity;
 let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
 let anyExtruded = false;

 // Machine state.
 let x = 0, y = 0, z = 0, e = 0;
 let absolute = true, absoluteE = true, eModeSet = false;
 const w = { X: NaN, Y: NaN, Z: NaN, E: NaN, I: NaN, J: NaN, R: NaN };

 const grow = () => {
  capacity *= 2;
  const s = new Float32Array(capacity * 6); s.set(seg); seg = s;
  const x8 = new Uint8Array(capacity); x8.set(ext); ext = x8;
  const o32 = new Int32Array(capacity); o32.set(off); off = o32;
  const t8 = new Uint8Array(capacity); t8.set(tl); tl = t8;
 };
 const closeLayer = () => {
  if (!cur || cur.count === 0) return;
  const from = cur.start, to = cur.start + cur.count;
  // 2026-09-21: the done ramp ran ahead of the head inside a layer. The layer was right to
  // 0.02 mm in Z, but the segment inside it was found by splitting the layer's byte span evenly
  // over its segments, which put the boundary up to 29 mm away in XY. Each segment now carries the
  // byte it starts at, so the drawn boundary is the move the printer is actually reading.
  layers.push({ z: cur.z, segments: seg.slice(from * 6, to * 6), extrude: ext.slice(from, to), offsets: off.slice(from, to), tool: tl.slice(from, to) });
  byteOffsets.push(cur.offset);
 };
 const startLayer = (layerZ, explicit, offset) => {
  closeLayer();
  cur = { z: layerZ, zExplicit: explicit, zSet: false, start: count, count: 0, offset };
 };
 // A marker on an empty layer only refines it: ;LAYER_CHANGE followed by ;Z: is one layer, and the
 // layer keeps the byte offset of the first marker of the pair.
 const markLayer = (layerZ, offset) => {
  sawMarker = true;
  if (cur && cur.count === 0) {
   if (layerZ !== null) { cur.z = layerZ; cur.zExplicit = true; }
   return;
  }
  startLayer(layerZ === null ? z : layerZ, layerZ !== null, offset);
 };

 const push = (x0, y0, z0, x1, y1, z1, extruding, offset) => {
  if (x0 === x1 && y0 === y1 && z0 === z1) return true;
  // The first layer of a file with no marker before it starts at byte 0, so progressToLayer covers
  // the start G-code too. Without any marker anywhere, an extruding move at a new Z is the layer.
  if (cur === null) cur = { z: z1, zExplicit: false, zSet: false, start: count, count: 0, offset: layers.length === 0 ? 0 : offset };
  else if (extruding && !sawMarker && !inArc && lastExtrudeZ !== null
   && Math.abs(z1 - lastExtrudeZ) >= FALLBACK_LAYER_MM) startLayer(z1, false, offset);
  if (extruding) lastExtrudeZ = z1;
  if (!cur.zExplicit) {
   if (extruding && !cur.zSet) { cur.z = z1; cur.zSet = true; } else if (cur.count === 0) cur.z = z1;
  }
  if (count >= maxSegments) { truncated = true; return false; }
  if (count === capacity) grow();
  const at = count * 6;
  seg[at] = x0; seg[at + 1] = y0; seg[at + 2] = z0;
  seg[at + 3] = x1; seg[at + 4] = y1; seg[at + 5] = z1;
  ext[count] = extruding ? 1 : 0;
  off[count] = offset | 0;
  tl[count] = tool;
  count++;
  cur.count++;
  const length = Math.hypot(x1 - x0, y1 - y0, z1 - z0);
  if (extruding) {
   totalLength += length;
   anyExtruded = true;
   if (x0 < minX) minX = x0; if (x1 < minX) minX = x1;
   if (y0 < minY) minY = y0; if (y1 < minY) minY = y1;
   if (z0 < minZ) minZ = z0; if (z1 < minZ) minZ = z1;
   if (x0 > maxX) maxX = x0; if (x1 > maxX) maxX = x1;
   if (y0 > maxY) maxY = y0; if (y1 > maxY) maxY = y1;
   if (z0 > maxZ) maxZ = z0; if (z1 > maxZ) maxZ = z1;
  } else travelLength += length;
  return true;
 };

 // G2/G3 in the XY plane, IJ form (offsets to the centre, always relative) or R form (signed
 // radius; negative selects the arc longer than 180 degrees). Follows grbl's centre derivation.
 const arc = (x1, y1, z1, clockwise, de, offset) => {
  let i = w.I, j = w.J;
  if (Number.isNaN(i) && Number.isNaN(j)) {
   const r = w.R;
   const dx = x1 - x, dy = y1 - y, d = Math.hypot(dx, dy);
   if (!Number.isFinite(r) || r === 0 || d === 0) return push(x, y, z, x1, y1, z1, de > 1e-9, offset);
   let h = 4 * r * r - dx * dx - dy * dy;
   h = -Math.sqrt(Math.max(0, h)) / d;
   if (!clockwise) h = -h;
   if (r < 0) h = -h;
   i = 0.5 * (dx - dy * h);
   j = 0.5 * (dy + dx * h);
  } else {
   if (Number.isNaN(i)) i = 0;
   if (Number.isNaN(j)) j = 0;
  }
  const cx = x + i, cy = y + j;
  const r0 = Math.hypot(i, j), r1 = Math.hypot(x1 - cx, y1 - cy);
  if (r0 === 0) return push(x, y, z, x1, y1, z1, de > 1e-9, offset);
  const a0 = Math.atan2(-j, -i);
  let a1 = Math.atan2(y1 - cy, x1 - cx);
  if (clockwise) { while (a1 >= a0) a1 -= 2 * Math.PI; } else { while (a1 <= a0) a1 += 2 * Math.PI; }
  // A start point equal to the end point in IJ form is a full circle, not a zero-length arc.
  if (Math.abs(x1 - x) < 1e-9 && Math.abs(y1 - y) < 1e-9) a1 = a0 + (clockwise ? -2 * Math.PI : 2 * Math.PI);
  const sweep = a1 - a0;
  const steps = Math.max(2, Math.min(ARC_MAX_STEPS, Math.ceil(Math.abs(sweep) * r0 / ARC_CHORD_MM)));
  const extruding = de > 1e-9;
  let px = x, py = y, pz = z;
  for (let s = 1; s <= steps; s++) {
   const t = s / steps;
   const a = a0 + sweep * t;
   const r = r0 + (r1 - r0) * t;
   const nx = s === steps ? x1 : cx + Math.cos(a) * r;
   const ny = s === steps ? y1 : cy + Math.sin(a) * r;
   const nz = z + (z1 - z) * t;
   const ok = push(px, py, pz, nx, ny, nz, extruding, offset);
   inArc = true;
   if (!ok) { inArc = false; return false; }
   px = nx; py = ny; pz = nz;
  }
  inArc = false;
  return true;
 };

 let pos = 0, byte = 0;
 while (pos <= source.length) {
  const nl = source.indexOf('\n', pos);
  const end = nl === -1 ? source.length : nl;
  let line = source.slice(pos, end);
  let eol = nl === -1 ? 0 : 1;
  if (line.endsWith('\r')) { line = line.slice(0, -1); eol += 1; }
  const lineOffset = byte;
  byte += lineBytes(line) + eol;
  if (nl !== -1) pos = nl + 1; else pos = source.length + 1;

  const trimmed = line.trim();
  if (trimmed.length === 0) continue;
  if (trimmed.charCodeAt(0) === 59) { // ';' — the only place layer markers live
   const c = trimmed.slice(1).trim();
   if (c === 'LAYER_CHANGE') markLayer(null, lineOffset);
   else if (c.startsWith('Z:')) {
    const v = parseFloat(c.slice(2));
    if (Number.isFinite(v)) markLayer(v, lineOffset);
   } else if (c.startsWith('LAYER_COUNT:')) {
    const v = parseInt(c.slice(12), 10);
    if (Number.isFinite(v)) totalLayers = v;
   } else if (c.startsWith('LAYER:')) {
    const v = parseInt(c.slice(6), 10);
    if (Number.isFinite(v)) markLayer(null, lineOffset);
   } else if (totalLayers === null && c.length < 40 && c.toLowerCase().startsWith('total layer number:')) {
    const v = parseInt(c.slice(19), 10); // Orca and PrusaSlicer write this in the header block
    if (Number.isFinite(v)) totalLayers = v;
   }
   continue;
  }
  const semi = line.indexOf(';');
  const code = (semi === -1 ? line : line.slice(0, semi)).trim();
  if (code.length === 0) continue;

  const head = code.charCodeAt(0) | 32;
  if (head === 116 && /^T\d{1,3}$/i.test(code)) { tool = Math.min(255, parseInt(code.slice(1), 10)); continue; } // T0, T1...: the tool the next moves are made with
  if (head !== 103 && head !== 109) { // not G, not M: the three macros we read
   if (head !== 101 && head !== 115) continue; // not E, not S either
   const up = code.toUpperCase();
   if (up.startsWith('EXCLUDE_OBJECT_START')) {
    if (open !== -1) objects[open].end = count;
    open = objects.length;
    objects.push({ name: macroValue(up, 'NAME='), start: count, end: count });
   } else if (up.startsWith('EXCLUDE_OBJECT_END')) {
    if (open !== -1) { objects[open].end = count; open = -1; }
   } else if (up.startsWith('SET_PRINT_STATS_INFO')) {
    const total = parseInt(macroValue(up, 'TOTAL_LAYER='), 10);
    if (Number.isFinite(total)) totalLayers = total;
    const current = macroValue(up, 'CURRENT_LAYER=');
    if (current !== '' && Number.isFinite(parseInt(current, 10))) markLayer(null, lineOffset);
   }
   continue;
  }

  const n = commandNumber(code);
  if (Number.isNaN(n)) continue;
  if (head === 109) { // M82 / M83 pin the extruder mode until the file says otherwise
   if (n === 82) { absoluteE = true; eModeSet = true; } else if (n === 83) { absoluteE = false; eModeSet = true; }
   continue;
  }
  if (n === 90 || n === 91) {
   absolute = n === 90;
   if (!eModeSet) absoluteE = absolute;
   continue;
  }
  if (n === 92) {
   lex(code, 1, w);
   const named = [w.X, w.Y, w.Z, w.E].some(Number.isFinite);
   if (!named) { x = y = z = e = 0; continue; }
   if (Number.isFinite(w.X)) x = w.X;
   if (Number.isFinite(w.Y)) y = w.Y;
   if (Number.isFinite(w.Z)) z = w.Z;
   if (Number.isFinite(w.E)) e = w.E;
   continue;
  }
  if (n !== 0 && n !== 1 && n !== 2 && n !== 3) continue;

  lex(code, 1, w);
  const x1 = Number.isFinite(w.X) ? (absolute ? w.X : x + w.X) : x;
  const y1 = Number.isFinite(w.Y) ? (absolute ? w.Y : y + w.Y) : y;
  const z1 = Number.isFinite(w.Z) ? (absolute ? w.Z : z + w.Z) : z;
  const de = Number.isFinite(w.E) ? (absoluteE ? w.E - e : w.E) : 0;
  const ok = (n === 2 || n === 3)
   ? arc(x1, y1, z1, n === 2, de, lineOffset)
   : push(x, y, z, x1, y1, z1, de > 1e-9, lineOffset);
  x = x1; y = y1; z = z1;
  if (Number.isFinite(w.E)) e = absoluteE ? w.E : e + w.E;
  if (!ok) break;
 }

 if (open !== -1) objects[open].end = count;
 closeLayer();

 const bounds = anyExtruded
  ? { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] }
  : { min: [0, 0, 0], max: [0, 0, 0] };
 return {
  layers, objects, bounds, totalLength, travelLength, byteOffsets,
  byteLength: ascii ? source.length : utf8Length(source),
  segmentCount: count, totalLayers, truncated,
 };
}

// Maps Moonraker's virtual_sdcard.file_position onto {layer, fraction within that layer}.
export function progressToLayer(parsed, fileOffset) {
 const offsets = parsed?.byteOffsets;
 if (!Array.isArray(offsets) || offsets.length === 0) return { layer: 0, fraction: 0 };
 const at = Number(fileOffset);
 if (!Number.isFinite(at) || at <= offsets[0]) return { layer: 0, fraction: 0 };
 let lo = 0, hi = offsets.length - 1;
 while (lo < hi) {
  const mid = (lo + hi + 1) >> 1;
  if (offsets[mid] <= at) lo = mid; else hi = mid - 1;
 }
 const start = offsets[lo];
 const end = lo + 1 < offsets.length ? offsets[lo + 1] : Math.max(parsed.byteLength ?? start, start);
 const span = end - start;
 return { layer: lo, fraction: span > 0 ? Math.max(0, Math.min(1, (at - start) / span)) : 1 };
}
