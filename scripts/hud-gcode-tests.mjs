// Node scripts/hud-gcode-tests.mjs — synthetic cases for every parser rule, plus a real slice of a
// Voron Design Cube sliced by OrcaSlicer and read read-only off the test printer's Moonraker.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseGcode, progressToLayer } from '../web/hud-gcode.js';

let checks = 0;
const is = (actual, expected, label) => { checks++; assert.deepEqual(actual, expected, label); };
const near = (actual, expected, label, eps = 1e-3) => {
 checks++;
 assert.ok(Math.abs(actual - expected) < eps, `${label}: ${actual} is not within ${eps} of ${expected}`);
};
// Segments are stored as Float32Array, so compare them at the precision a 350 mm bed needs.
const seg = (layer, i) => Array.from(layer.segments.slice(i * 6, i * 6 + 6), (v) => Math.round(v * 1e4) / 1e4);

// --- absolute G0/G1, travel versus extrusion -------------------------------------------------
{
 const p = parseGcode('G90\nM82\nG1 X0 Y0 Z0.2\nG1 X10 Y0 E1\nG1 X10 Y10\nG1 X0 Y10 E2\n');
 is(p.layers.length, 1, 'one layer without markers');
 is(p.layers[0].z, 0.2, 'layer z from the first extruding move');
 is(Array.from(p.layers[0].extrude), [0, 1, 0, 1], 'travel and extrusion flagged per segment');
 is(seg(p.layers[0], 1), [0, 0, 0.2, 10, 0, 0.2], 'packed x0,y0,z0,x1,y1,z1');
 near(p.totalLength, 20, 'extruded length');
 near(p.travelLength, 10.2, 'travel length, the 0.2 lift included');
 is(p.bounds, { min: [0, 0, 0.2], max: [10, 10, 0.2] }, 'bounds cover extruded moves only');
 is(p.segmentCount, 4, 'segment count');
 is(p.truncated, false, 'not truncated');
}

// --- relative motion (G91) and relative extrusion (M83) --------------------------------------
{
 const p = parseGcode('G90\nM83\nG1 X5 Y5 Z0.2\nG91\nG1 X10 E1\nG1 Y10 E1\nG90\nG1 X5 Y5 E1\n');
 const l = p.layers[0];
 is(seg(l, 1), [5, 5, 0.2, 15, 5, 0.2], 'G91 adds to the current position');
 is(seg(l, 2), [15, 5, 0.2, 15, 15, 0.2], 'G91 keeps adding');
 is(seg(l, 3), [15, 15, 0.2, 5, 5, 0.2], 'G90 returns to absolute');
 is(Array.from(l.extrude), [0, 1, 1, 1], 'M83 relative E always extrudes on positive E');
}

// --- absolute extrusion, retractions are not extrusion, G92 resets ---------------------------
{
 const p = parseGcode('G90\nM82\nG1 X0 Y0 Z0.2\nG1 X10 E5\nG1 E4\nG1 E5\nG1 X20\nG1 X30 E4\nG92 E0\nG1 X40 E1\n');
 is(p.segmentCount, 5, 'a retraction that does not move the head makes no segment');
 is(Array.from(p.layers[0].extrude), [0, 1, 0, 0, 1], 'absolute E going down is travel; G92 E0 restarts E');
 near(p.totalLength, 20, 'only the two extruding moves count');
}
{
 const p = parseGcode('G90\nG1 X0 Y0 Z0.2\nG1 X10 E1\nG92 X0\nG1 X5 E1\n');
 is(seg(p.layers[0], 2), [0, 0, 0.2, 5, 0, 0.2], 'G92 X0 renames the current X without moving');
}
{
 const p = parseGcode('G90\nM83\nG1 X4 Y4 Z0.2 E1\nG92\nG1 X1 Y1 Z0.2 E1\n');
 is(seg(p.layers[0], 1), [0, 0, 0, 1, 1, 0.2], 'bare G92 zeroes every axis');
}

// --- G90/G91 also move E unless M82/M83 pinned it --------------------------------------------
{
 const p = parseGcode('G90\nG1 X0 Y0 Z0.2\nG1 X10 E10\nG91\nG1 X10 E1\nG1 X10 E1\n');
 is(Array.from(p.layers[0].extrude), [0, 1, 1, 1], 'G91 makes E relative when no M82/M83 was seen');
 const q = parseGcode('G90\nM82\nG1 X0 Y0 Z0.2\nG1 X10 E10\nG91\nG1 X10 E1\n');
 is(Array.from(q.layers[0].extrude), [0, 1, 0], 'M82 survives G91, so E1 after E10 is a retraction');
}

// --- G2/G3 arcs, IJ form -----------------------------------------------------------------------
{
 // Quarter circle, radius 10, centre (0,0), from (10,0) to (0,10) counter-clockwise.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG3 X0 Y10 I-10 J0 E5\n');
 const l = p.layers[0];
 assert.ok(l.extrude.length > 10, 'the arc is flattened into many segments');
 checks++;
 for (let i = 1; i < l.extrude.length; i++) {
  const [, , , x1, y1] = seg(l, i);
  near(Math.hypot(x1, y1), 10, `arc point ${i} stays on the radius`, 1e-2);
 }
 near(p.totalLength, 2 * Math.PI * 10 / 4, 'arc length is a quarter circumference', 0.05);
 const last = seg(l, l.extrude.length - 1);
 near(last[3], 0, 'arc lands exactly on the target X');
 near(last[4], 10, 'arc lands exactly on the target Y');
 is(Array.from(l.extrude).slice(1).every((v) => v === 1), true, 'the whole arc is extruding');
}
{
 // Same geometry clockwise is the long way round: three quarters of the circumference.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG2 X0 Y10 I-10 J0 E5\n');
 near(p.totalLength, 3 * 2 * Math.PI * 10 / 4, 'G2 sweeps the other way', 0.2);
}
{
 // IJ with an identical start and end is a full circle, not a no-op.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG2 X10 Y0 I-10 J0 E5\n');
 near(p.totalLength, 2 * Math.PI * 10, 'IJ full circle', 0.2);
}

// --- G2/G3 arcs, R form ------------------------------------------------------------------------
{
 // Positive R takes the short way: a quarter circle of radius 10.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG3 X0 Y10 R10 E5\n');
 near(p.totalLength, 2 * Math.PI * 10 / 4, 'R10 is the minor arc', 0.05);
 const l = p.layers[0];
 const last = seg(l, l.extrude.length - 1);
 near(last[3], 0, 'R-form arc lands on the target X');
 near(last[4], 10, 'R-form arc lands on the target Y');
}
{
 // Negative R takes the long way round the same two points.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG3 X0 Y10 R-10 E5\n');
 near(p.totalLength, 3 * 2 * Math.PI * 10 / 4, 'R-10 is the major arc', 0.2);
}
{
 // A radius too small for the chord must not produce NaN geometry.
 const p = parseGcode('G90\nM83\nG1 X0 Y0 Z0.2\nG2 X20 Y0 R1 E5\n');
 const l = p.layers[0];
 checks++;
 assert.ok(Array.from(l.segments).every(Number.isFinite), 'an impossible radius still yields finite points');
}
{
 // Helical arc: Z interpolates across the sweep.
 const p = parseGcode('G90\nM83\nG1 X10 Y0 Z0.2\nG3 X0 Y10 Z1.2 I-10 J0 E5\n');
 const l = p.layers[0];
 const last = seg(l, l.extrude.length - 1);
 near(last[5], 1.2, 'the arc climbs to the commanded Z');
 is(p.layers.length, 1, 'one G2/G3 command is one layer, however far it climbs');
}
{
 // Vase mode: Z creeps up inside one continuous spiral, which is one layer, not thousands.
 const lines = ['G90', 'M83', 'G1 X10 Y0 Z0.2'];
 for (let i = 1; i <= 400; i++) {
  const a = i * Math.PI / 100;
  lines.push(`G1 X${(10 * Math.cos(a)).toFixed(4)} Y${(10 * Math.sin(a)).toFixed(4)} Z${(0.2 + i * 0.0005).toFixed(4)} E0.16`);
 }
 const p = parseGcode(lines.join('\n'));
 is(p.layers.length, 1, 'a spiral climb below the fallback step stays one layer');
}

// --- layer markers -----------------------------------------------------------------------------
{
 // PrusaSlicer / Orca: ;LAYER_CHANGE then ;Z: is one layer that starts at the first of the pair.
 const text = 'G90\nM83\n;LAYER_CHANGE\n;Z:0.2\nG1 X0 Y0 Z0.2\nG1 X10 E1\n;LAYER_CHANGE\n;Z:0.4\nG1 Z0.4\nG1 X0 E1\n';
 const p = parseGcode(text);
 is(p.layers.length, 2, 'LAYER_CHANGE plus Z is one layer each');
 is(p.layers.map((l) => l.z), [0.2, 0.4], 'z comes from the ;Z: marker');
 is(p.byteOffsets, [text.indexOf(';LAYER_CHANGE'), text.lastIndexOf(';LAYER_CHANGE')], 'offsets point at the first marker of each pair');
}
{
 // Cura: ;LAYER: with no z, and ;LAYER_COUNT: must not be mistaken for a layer change.
 const p = parseGcode(';LAYER_COUNT:2\nG90\nM83\n;LAYER:0\nG1 X0 Y0 Z0.3\nG1 X10 E1\n;LAYER:1\nG1 Z0.6\nG1 X0 E1\n');
 is(p.layers.length, 2, 'Cura markers split layers');
 is(p.totalLayers, 2, ';LAYER_COUNT: is metadata, not a layer');
 is(p.layers.map((l) => l.z), [0.3, 0.6], 'z falls back to the first extruding move');
}
{
 // Klipper's own layer reporting.
 const p = parseGcode('G90\nM83\nSET_PRINT_STATS_INFO TOTAL_LAYER=2\nSET_PRINT_STATS_INFO CURRENT_LAYER=1\nG1 X0 Y0 Z0.2\nG1 X10 E1\nSET_PRINT_STATS_INFO CURRENT_LAYER=2\nG1 Z0.4\nG1 X0 E1\n');
 is(p.totalLayers, 2, 'TOTAL_LAYER is read');
 is(p.layers.length, 2, 'CURRENT_LAYER changes the layer');
}
{
 // No markers at all: an extruding move at a new Z opens the next layer.
 const p = parseGcode('G90\nM83\nG1 X0 Y0 Z0.2\nG1 X10 E1\nG1 Z0.4\nG1 X0 E1\nG1 Z0.6\nG1 X10 E1\n');
 is(p.layers.length, 3, 'fallback splits on an extruding Z change');
 is(p.layers.map((l) => l.z), [0.2, 0.4, 0.6], 'fallback layer heights');
}
{
 // A Z hop between extrusions at the same height is not a new layer.
 const p = parseGcode('G90\nM83\nG1 X0 Y0 Z0.2\nG1 X10 E1\nG1 Z0.6\nG1 X20\nG1 Z0.2\nG1 X30 E1\n');
 is(p.layers.length, 1, 'a travel Z hop does not start a layer');
}
{
 // Moves before the first marker belong to a leading layer that starts at byte 0.
 const text = 'G90\nM83\nG1 X0 Y0 Z0.2\nG1 X10 E1\n;LAYER_CHANGE\n;Z:0.4\nG1 Z0.4\nG1 X0 E1\n';
 const p = parseGcode(text);
 is(p.layers.length, 2, 'the start G-code becomes its own layer');
 is(p.byteOffsets[0], 0, 'the leading layer starts at byte 0');
 is(p.byteOffsets[1], text.indexOf(';LAYER_CHANGE'), 'the marked layer starts at its marker');
}

// --- comments, blank lines, whitespace, inline comments ----------------------------------------
{
 const p = parseGcode('\n; a comment\n\n   \nG90 ; set absolute\nM83\nG1 X0 Y0 Z0.2 ; move\n\nG1 X10 E1 ; draw\n; trailing\n');
 is(p.segmentCount, 2, 'comments and blank lines are ignored');
 is(Array.from(p.layers[0].extrude), [0, 1], 'inline comments do not break a move');
}
{
 const p = parseGcode('g90\nm83\ng1 x0 y0 z0.2\ng1 x10 e1\n');
 is(p.segmentCount, 2, 'lower case G-code parses');
}
{
 const p = parseGcode('G90\nM83\nG1X0Y0Z0.2\nG1X10E1\n');
 is(p.segmentCount, 2, 'words without separating spaces parse');
}

// --- EXCLUDE_OBJECT ranges ----------------------------------------------------------------------
{
 const p = parseGcode([
  'G90', 'M83', 'G1 X0 Y0 Z0.2',
  'EXCLUDE_OBJECT_START NAME=cube_a',
  'G1 X10 E1', 'G1 Y10 E1',
  'EXCLUDE_OBJECT_END NAME=cube_a',
  'G1 X50 Y50',
  'EXCLUDE_OBJECT_START NAME=cube_b',
  'G1 X60 E1',
  '',
 ].join('\n'));
 is(p.objects, [{ name: 'CUBE_A', start: 1, end: 3 }, { name: 'CUBE_B', start: 4, end: 5 }],
  'object names map onto segment index ranges, the open one closed at end of file');
 is(p.segmentCount, 5, 'segment indices are global across layers');
}

// --- byte offsets count UTF-8 bytes and CRLF ------------------------------------------------------
{
 const text = 'G90\r\nM83\r\n;LAYER_CHANGE\r\n;Z:0.2\r\nG1 X0 Y0 Z0.2\r\nG1 X10 E1\r\n;LAYER_CHANGE\r\n;Z:0.4\r\nG1 Z0.4\r\nG1 X0 E1\r\n';
 const p = parseGcode(text);
 const bytes = Buffer.from(text, 'utf8');
 is(p.byteOffsets, [bytes.indexOf(';LAYER_CHANGE'), bytes.lastIndexOf(';LAYER_CHANGE')], 'CRLF counts as two bytes');
 is(p.byteLength, bytes.length, 'byteLength is the file length in bytes');
}
{
 // A non-ASCII comment is more bytes than characters; offsets must follow the bytes.
 const text = '; brûlée — café ☕\nG90\nM83\n;LAYER_CHANGE\n;Z:0.2\nG1 X0 Y0 Z0.2\nG1 X10 E1\n';
 const p = parseGcode(text);
 const bytes = Buffer.from(text, 'utf8');
 checks++;
 assert.ok(bytes.length > text.length, 'the fixture really is multi-byte');
 is(p.byteOffsets, [bytes.indexOf(';LAYER_CHANGE')], 'UTF-8 bytes, not characters');
 is(p.byteLength, bytes.length, 'byteLength in bytes for a non-ASCII file');
}

// --- maxSegments cap ------------------------------------------------------------------------------
{
 const lines = ['G90', 'M83', 'G1 X0 Y0 Z0.2'];
 for (let i = 1; i <= 100; i++) lines.push(`G1 X${i} E0.1`);
 const p = parseGcode(lines.join('\n'), { maxSegments: 10 });
 is(p.truncated, true, 'the cap reports truncation');
 is(p.segmentCount, 10, 'the cap holds');
 is(p.layers.reduce((n, l) => n + l.extrude.length, 0), 10, 'no segment escapes the cap');
 const q = parseGcode(lines.join('\n'));
 is(q.truncated, false, 'under the default cap nothing is truncated');
 is(q.segmentCount, 101, 'every move is kept below the cap');
}

// --- a few hundred thousand lines, no recursion ----------------------------------------------------
{
 const lines = ['G90', 'M83'];
 for (let i = 0; i < 200000; i++) lines.push(`G1 X${(i % 100) + 100} Y${(i % 97) + 100} Z0.2 E0.02`);
 const started = Date.now();
 const p = parseGcode(lines.join('\n'));
 checks++;
 assert.equal(p.segmentCount, 200000, '200k moves parse without a stack overflow');
 checks++;
 assert.ok(Date.now() - started < 20000, 'the parse finishes in a sane time');
}

// --- progressToLayer ---------------------------------------------------------------------------------
{
 const text = 'G90\nM83\n;LAYER_CHANGE\n;Z:0.2\nG1 X0 Y0 Z0.2\nG1 X10 E1\n;LAYER_CHANGE\n;Z:0.4\nG1 Z0.4\nG1 X0 E1\n';
 const p = parseGcode(text);
 const [a, b] = p.byteOffsets;
 is(progressToLayer(p, 0), { layer: 0, fraction: 0 }, 'before the first layer');
 is(progressToLayer(p, a), { layer: 0, fraction: 0 }, 'at the first layer boundary');
 is(progressToLayer(p, a + (b - a) / 2), { layer: 0, fraction: 0.5 }, 'halfway through a layer');
 is(progressToLayer(p, b), { layer: 1, fraction: 0 }, 'at the second layer boundary');
 is(progressToLayer(p, p.byteLength), { layer: 1, fraction: 1 }, 'at the end of the file');
 is(progressToLayer(p, p.byteLength * 4), { layer: 1, fraction: 1 }, 'past the end of the file');
 is(progressToLayer(p, NaN), { layer: 0, fraction: 0 }, 'a bad offset is layer zero');
 is(progressToLayer({ byteOffsets: [] }, 10), { layer: 0, fraction: 0 }, 'an empty parse is layer zero');
}

// --- a real file: three layers of a Voron Design Cube, OrcaSlicer 2.4.2, read off the test printer
// ----------
{
 const file = fileURLToPath(new URL('../tests/fixtures/cube-3-layers.gcode', import.meta.url));
 const bytes = readFileSync(file);
 const p = parseGcode(bytes.toString('utf8'));
 is(p.layers.length, 3, 'three layers in the trimmed cube');
 is(p.layers.map((l) => l.z), [0.2, 0.4, 0.6], 'layer heights from the ;Z: markers');
 is(p.totalLayers, 150, 'the whole cube is 150 layers, per the slicer header block');

 // byteOffsets must land on the real byte positions of the layer-change lines in the file.
 const real = [];
 for (let at = bytes.indexOf(';LAYER_CHANGE'); at !== -1; at = bytes.indexOf(';LAYER_CHANGE', at + 1)) {
  if (at === 0 || bytes[at - 1] === 0x0a) real.push(at);
 }
 is(real.length, 3, 'the fixture holds exactly three layer-change lines');
 is(p.byteOffsets, real, 'byteOffsets are the real byte positions of the layer-change lines');
 is(p.byteLength, bytes.length, 'byteLength matches the file size on disk');

 for (const axis of [0, 1, 2]) {
  checks++;
  assert.ok(p.bounds.min[axis] >= 0 && p.bounds.max[axis] <= 350,
   `axis ${axis} bounds ${p.bounds.min[axis]}..${p.bounds.max[axis]} are inside the 350 mm bed`);
 }
 checks++;
 assert.ok(p.bounds.max[0] - p.bounds.min[0] > 20, 'the cube is about 30 mm across in X');
 checks++;
 assert.ok(p.bounds.max[1] - p.bounds.min[1] > 20, 'the cube is about 30 mm across in Y');
 checks++;
 assert.ok(p.totalLength > 1000, `extruded path is ${p.totalLength.toFixed(0)} mm over three layers`);
 checks++;
 assert.ok(p.objects.length >= 3 && p.objects.every((o) => o.name.startsWith('VORON_DESIGN_CUBE')),
  'every EXCLUDE_OBJECT range names the cube');
 checks++;
 assert.ok(p.objects.every((o) => o.end > o.start), 'every object range holds segments');
 is(p.truncated, false, 'the real slice fits under the default cap');

 // The middle of layer two must resolve back to layer two.
 const mid = Math.round((p.byteOffsets[1] + p.byteOffsets[2]) / 2);
 is(progressToLayer(p, mid).layer, 1, 'a file_position inside layer two resolves to layer two');
 near(progressToLayer(p, mid).fraction, 0.5, 'and to roughly half way through it', 0.02);

 console.log(`Real slice: ${p.layers.length} layers, ${p.segmentCount} segments, ` +
  `${p.totalLength.toFixed(0)} mm extruded, bounds X ${p.bounds.min[0]}..${p.bounds.max[0]} ` +
  `Y ${p.bounds.min[1]}..${p.bounds.max[1]} Z ${p.bounds.min[2]}..${p.bounds.max[2]}.`);
}

console.log(`HUD G-code parser: ${checks} assertions passed.`);
