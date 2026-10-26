// node scripts/model-follow-tests.mjs: the pose arithmetic the page repeats, the head's path along a file, the print
// as line segments, and the head between reports (polish WO-03, 2026-10-18).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseGcode } from '../web/hud-gcode.js';
import { poseFor, blendGroups, printPoint, pathAt, arcOf, atArc, project, byteAt, buildPrint, countAt, colorsOf, DEFAULT_COLORS, HeadFollow } from '../web/model-follow.js';

let checks = 0;
const is = (a, b, label) => { checks++; assert.deepEqual(a, b, label); };
const near = (a, b, label, eps = 1e-6) => { checks++; assert.ok(Math.abs(a - b) <= eps, `${label}: ${a} is not within ${eps} of ${b}`); };
const json = (p) => JSON.parse(readFileSync(new URL(p, import.meta.url), 'utf8'));

// --- the pose: the same numbers as src/demo.rs (tool_changes_in_the_file_move_the_active_tool_and_idle_tools_park) ---
{
 const spec = { axes: { x: [-150, 150], y: [-150, 150], z: [0, 300] }, motions: { t0: ['x', 'y', 'z'], t1: ['x', 'y', 'z'] },
  tools: [{ group: 't0', park: { x: -150 } }, { group: 't1', park: { x: 150 } }] };
 const cal = { offset_mm: [-150, -150, 0], sign: [1, 1, 1], source: [0, 1, 2] };
 let p = poseFor(spec, cal, [200, 120, 40], 1);
 is(p.groups.t1, [50, -30, 40], 'the active tool follows the head');
 is(p.groups.t0, [-150, -30, 40], 'the idle tool is at its dock');
 p = poseFor(spec, cal, [200, 120, 40], 0);
 is([p.groups.t0, p.groups.t1, p.tool], [[50, -30, 40], [150, -30, 40], 0], 'and the other way round');
 // No active tool named: every group follows the position as it is.
 p = poseFor(spec, cal, [200, 120, 40], null);
 is([p.groups.t0, p.groups.t1], [[50, -30, 40], [50, -30, 40]], 'no tool given poses every group alike');
 // The end of travel holds, and says which axis.
 p = poseFor(spec, cal, [500, 0, 0], null);
 is([p.axes.x, p.clamped, p.known], [150, ['x'], true], 'held at the end of the travel');
 p = poseFor(spec, cal, null, null);
 is(p.known, false, 'no position is not a known pose');
 // Signs, offsets and sources: Y reversed and X read from the printer's Y, as the calibration says.
 const flipped = { offset_mm: [0, 140, 0], sign: [1, -1, 1], source: [1, 1, 2] };
 p = poseFor({ axes: { x: [0, 300], y: [0, 300], z: [0, 300] }, motions: { head: ['x', 'y', 'z'] } }, flipped, [10, 40, 5]);
 is([p.axes.x, p.axes.y, p.axes.z], [40, 100, 5], 'sign, offset and source apply per axis');
 // Rest offsets add to every group; an object spec scales an axis.
 p = poseFor({ axes: { x: [0, 9], y: [0, 9], z: [0, 9] }, motions: { a: [{ z: -1 }, 'y', 0] }, rest_offsets_mm: { a: [1, 2, 3] } }, { offset_mm: [0, 0, 0], sign: [1, 1, 1], source: [0, 1, 2] }, [1, 2, 4]);
 is(p.groups.a, [-3, 4, 3], 'a factor, a plain axis, a constant and the rest offset');
 // The real V2.4 350: at the calibration's reference the model is in its CAD pose; X moves the head only.
 const v24 = json('../web/printer-motion-parts.json').profiles.voron_v24_350_printed;
 const ref = [175, 179.1, 27.0];
 p = poseFor(v24, v24.calibration, ref);
 for (const g of ['head', 'x_beam', 'z_gantry']) for (const v of p.groups[g]) near(v, 0, `${g} is in its pose at the reference`, 0.1);
 p = poseFor(v24, v24.calibration, [ref[0] + 10, ref[1], ref[2]]);
 near(p.groups.head[0], 10, 'X moves the head', 0.1); near(p.groups.x_beam[0], 0, 'and not the beam');
 p = poseFor(v24, v24.calibration, [ref[0], ref[1], ref[2] + 20]);
 is([Math.round(p.groups.z_gantry[2]), Math.round(p.groups.head[2]), Math.round(p.groups.x_beam[2])], [20, 20, 20], 'Z lifts the gantry, the beam and the head together');
 // Blending two poses.
 const b = blendGroups({ a: [0, 0, 0] }, { a: [10, 20, 30], c: [1, 1, 1] }, 0.5);
 is([b.a, b.c], [[5, 10, 15], [1, 1, 1]], 'a blend takes the middle and keeps a group only one has');
 is(blendGroups({ a: [0, 0, 0] }, { a: [4, 4, 4] }, 1).a, [4, 4, 4], 'a finished blend is the target');
}

// --- where a print file's point lands on the model --------------------------------------------------------------
{
 const frames = json('../web/printer-print-frames.json').machines;
 const f = frames.voron_v24_300_printed;
 is(printPoint(f, 0, 0, 0), f.origin_mm, 'printer 0,0,0 is the origin');
 is(printPoint(f, 10, 20, 5).map((v, i) => Math.round((v - f.origin_mm[i]) * 1000) / 1000), [10, 20, 5], 'a CAD machine: the axes are the frame');
 // The CORE One L's X runs along CAD +Y and its Y along CAD -X.
 const c = frames.prusa_core_one_l;
 is(printPoint(c, 10, 0, 0).map((v, i) => v - c.origin_mm[i]), [0, 10, 0], 'CORE One L X is CAD +Y');
 is(printPoint(c, 0, 10, 0).map((v, i) => v - c.origin_mm[i]), [-10, 0, 0], 'CORE One L Y is CAD -X');
 // The nozzle's own position agrees: the maker's reference put on the origin is the nozzle at the pose.
 const n = frames.voron_v24_300_printed.nozzle_mm;
 is(n.map((v, i) => Math.round((v - f.origin_mm[i]) * 100) / 100), [150, 154.1, 27.01], 'the nozzle is 150, 154.1, 27.0 over the plate corner in the pose');
}

// --- the head along a file ------------------------------------------------------------------------------------
// A square of four moves, then a move up and across: byte offsets are the line starts.
const SQUARE = 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\nG1 X10 Y0 E1\nG1 X10 Y10 E2\nG1 X0 Y10 E3\nG1 X0 Y0 E4\n;LAYER_CHANGE\n;Z:0.4\nG1 Z0.4\nG1 X10 Y0 E5\n';
{
 const p = parseGcode(SQUARE);
 const at = (line) => { let o = 0; const ls = SQUARE.split('\n'); for (let i = 0; i < line; i++) o += ls[i].length + 1; return o; };
 const r = (v) => Math.round(v * 1e4) / 1e4;
 const where = (byte) => { const q = pathAt(p, byte); return q && [r(q.x), r(q.y), r(q.z)]; };
 is(where(at(5)), [0, 0, 0.2], 'at the start of the first move the head is where it was');
 is(where(at(6)), [10, 0, 0.2], 'at the start of the next move the head is at its start');
 const len = SQUARE.split('\n')[5].length + 1;
 is(where(at(5) + Math.floor(len / 2)), [r(0 + 10 * Math.floor(len / 2) / len), 0, 0.2], 'half way through the lines of a move the head is part way along it');
 is(where(at(8)), [0, 10, 0.2], 'a later move');
 is(pathAt(p, at(5)).layer, 1, 'the layer it is in (layer 0 is the start code before the first marker)');
 is(pathAt(p, at(12)).layer, 2, 'and the next layer');
 is(where(1e9), [10, 0, 0.4], 'past the end the head is at the last move\'s end');
 is(pathAt({ layers: [] }, 5), null, 'a file with no moves has no path');
 is(pathAt(p, NaN), null, 'no byte, no place');
}
{
 // An arc is several segments of one line: the head walks them in order.
 const text = 'G90\nM82\nG1 X10 Y0 Z0.2 F3000\nG3 X-10 Y0 I-10 J0 E1\nG1 X-10 Y5 E2\n';
 const p = parseGcode(text);
 const start = text.indexOf('G3'), end = text.indexOf('G1 X-10 Y5');
 const a = pathAt(p, start + 1), m = pathAt(p, Math.floor((start + end) / 2)), z = pathAt(p, end - 1);
 near(Math.hypot(a.x, a.y), 10, 'the start of the arc is on the circle', 0.2);
 near(Math.hypot(m.x, m.y), 10, 'its middle is on the circle, not on the chord', 0.2);
 checks++; assert.ok(m.y > 5, 'the middle of a half circle is at the top');
 near(Math.hypot(z.x, z.y), 10, 'and its end', 0.2);
}

// --- the print as lines ------------------------------------------------------------------------------------------
{
 const BASIS = [[1, 0, 0], [0, 0, 1], [0, -1, 0]];
 const frame = { origin_mm: [100, 200, 50], axes: [[1, 0, 0], [0, 1, 0], [0, 0, 1]], group: null };
 const text = 'G90\nM82\nT0\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\nG1 X10 Y0 E1\nG1 X10 Y10 E2\nT1\nG1 X0 Y10 E3\nG1 X0 Y0 E4\nG1 X20 Y20\nG1 X30 Y20 E5\n';
 const parsed = parseGcode(text);
 const out = buildPrint(parsed, frame, BASIS, {});
 is([out.segments, out.kept, out.layersKept], [5, 5, 2], 'every extruding move is a segment when there is no budget (two layers: the start code and the first)');
 // First segment: printer (0,0,0.2) -> (10,0,0.2) is CAD (100,200,50.2) -> (110,200,50.2): display (x, z, -y) in metres.
 const p = Array.from(out.positions.slice(0, 6), (v) => Math.round(v * 1e6) / 1e6);
 is(p, [0.1, 0.0502, -0.2, 0.11, 0.0502, -0.2], 'the first segment, in the display frame, in metres');
 is(out.tools, [0, 1], 'two tools in the file');
 // Bytes: each segment carries the byte of its own line, in order.
 const lines = text.split('\n'); const off = (needle) => { let o = 0; for (const l of lines) { if (l === needle) return o; o += l.length + 1; } };
 is(Array.from(out.bytes), [off('G1 X10 Y0 E1'), off('G1 X10 Y10 E2'), off('G1 X0 Y10 E3'), off('G1 X0 Y0 E4'), off('G1 X30 Y20 E5')], 'bytes follow the file');
 is([countAt(out, 0), countAt(out, off('G1 X10 Y10 E2')), countAt(out, off('G1 X0 Y10 E3') - 1), countAt(out, 1e9), countAt(out, null)], [0, 2, 2, 5, 5], 'how many are drawn at a byte; null draws all');
 // Tool colours: segment 3 (after T1) is the second colour.
 const c = (i) => Array.from(out.colors.slice(i * 6, i * 6 + 3));
 checks++; assert.ok(c(0)[0] < c(0)[1] && c(2)[0] > c(2)[2], 'tool 0 is teal-ish (green over red), tool 1 orange-ish (red over blue)');
 // A budget: a 3000-segment file drawn within 400 segments, in order, ending where the file ends, nothing dropped but detail.
 let big = 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\n'; let e = 0;
 for (let layer = 0; layer < 30; layer++) { big += `;LAYER_CHANGE\n;Z:${(0.2 * (layer + 1)).toFixed(1)}\n`; for (let i = 0; i < 100; i++) big += `G1 X${(20 + 15 * Math.cos(i / 15.9)).toFixed(3)} Y${(20 + 15 * Math.sin(i / 15.9)).toFixed(3)} Z${(0.2 * (layer + 1)).toFixed(1)} E${(e += 0.1).toFixed(2)}\n`; }
 const bp = parseGcode(big);
 const lean = buildPrint(bp, frame, BASIS, { budget: 400 });
 checks++; assert.ok(lean.kept <= 400 && lean.kept > 100, `fits the budget: ${lean.kept}`);
 const bs = Array.from(lean.bytes); checks++; assert.ok(bs.every((v, i) => i === 0 || v >= bs[i - 1]), 'bytes never go backwards');
 const last = bp.layers.at(-1); is(lean.bytes.at(-1), last.offsets.at(-1), 'the last segment is the file\'s last move');
 const full = buildPrint(bp, frame, BASIS, {});
 is(full.kept, full.segments, 'with no budget nothing is merged or dropped');
 // The part keeps its height: the top of the thin print is the top of the full one.
 const top = (o) => { let m = -1; for (let i = 1; i < o.positions.length; i += 3) m = Math.max(m, o.positions[i]); return m; };
 near(top(lean), top(full), 'the thin print is as tall', 1e-6);
}

// --- colours -----------------------------------------------------------------------------------------------------
is(colorsOf('; extruder_colour = #04D9C4;#FF6A00\n'), [0x04d9c4, 0xff6a00], 'the file names its tool colours');
is(colorsOf('; extruder_colour = "";""\n'), DEFAULT_COLORS, 'empty names fall back to the brand colours');
is(colorsOf('G1 X1\n'), DEFAULT_COLORS, 'no line, the brand colours');

// --- the head between reports --------------------------------------------------------------------------------
{
 // Reports every 500 ms, the head going 10 mm per report along X: it glides, with no step at a report.
 const f = new HeadFollow();
 let t = 1000;
 f.report({ position: [0, 0, 0], source: 'live', state: 'printing', ageS: 0 }, t);
 const seen = [];
 for (let k = 1; k <= 6; k++) {
  t += 500; f.report({ position: [10 * k, 0, 0], source: 'live', state: 'printing', ageS: 0 }, t);
  for (let ms = 0; ms < 500; ms += 16) seen.push(f.frame(t + ms).position[0]);
 }
 let worst = 0; for (let i = 1; i < seen.length; i++) worst = Math.max(worst, Math.abs(seen[i] - seen[i - 1]));
 checks++; assert.ok(worst < 0.7, `the head moves ${worst.toFixed(2)} mm at most between frames (a step would be 10)`);
 near(f.frame(t + 500).position[0], 60, 'and arrives', 0.5);
 is(f.frame(t).label, 'live', 'a live position says live');
 // A new report comes early: the head does not jump to its new target.
 const g = new HeadFollow();
 g.report({ position: [0, 0, 0], source: 'live', state: 'printing', ageS: 0 }, 0);
 g.report({ position: [10, 0, 0], source: 'live', state: 'printing', ageS: 0 }, 500);
 const before = g.frame(700).position[0];
 g.report({ position: [20, 0, 0], source: 'live', state: 'printing', ageS: 0 }, 700);
 near(g.frame(700).position[0], before, 'an early report starts from where the head is', 1e-9);
 // The same report twice (a poll between two ticks) changes nothing.
 is(g.report({ position: [20, 0, 0], source: 'live', state: 'printing', ageS: 0.2 }, 800), false, 'a repeat of the last moment is ignored');
 // Quiet: no report for a while and the head stops asking for frames.
 is(g.frame(700 + 100).moving, true, 'moving soon after a report');
 is(g.frame(700 + 9000).moving, false, 'not asking for frames after the printer went quiet');
}
{
 // Following the file: a circle of radius 20 read at two samples a turn. The head runs on the circle between samples.
 let text = 'G90\nM82\nG1 X40 Y20 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\n'; let e = 0;
 const n = 360;
 for (let i = 1; i <= n; i++) text += `G1 X${(20 + 20 * Math.cos(2 * Math.PI * i / n)).toFixed(3)} Y${(20 + 20 * Math.sin(2 * Math.PI * i / n)).toFixed(3)} E${(e += 0.1).toFixed(2)}\n`;
 const parsed = parseGcode(text);
 const starts = parsed.layers.at(-1).offsets, bytes = (frac) => starts[Math.min(starts.length - 1, Math.floor(frac * starts.length))];
 const f = new HeadFollow({ parsed });
 let t = 0, worst = 0;
 f.report({ position: [40, 20, 0.2], source: 'file', byte: bytes(0), state: 'printing', ageS: 0 }, t);
 for (let k = 1; k <= 8; k++) { // half a turn per report
  t += 1000; f.report({ position: null, source: 'file', byte: bytes((k * 0.5) % 1 || 0.999), state: 'printing', ageS: 0 }, t);
  for (let ms = 0; ms < 1000; ms += 25) {
   const q = f.frame(t + ms).position;
   worst = Math.max(worst, Math.abs(Math.hypot(q[0] - 20, q[1] - 20) - 20));
  }
 }
 checks++; assert.ok(worst < 0.3, `the head stays on the circle between reports (${worst.toFixed(2)} mm off); the chord would be 20 mm off`);
 is(f.frame(t).label, 'following the file', 'a position from the file says so');
 is(f.frame(t).along, true, 'and is read along the file');
 // Paused: the head glides to the reported position and the file is not followed.
 f.report({ position: [31, 22, 0.2], source: 'file', byte: bytes(0.5), state: 'paused', ageS: 0 }, t + 1000);
 const p = f.frame(t + 3000);
 is([p.along, Math.round(p.position[0]), Math.round(p.position[1])], [false, 31, 22], 'paused: the head is where the printer says');
 // A new print (the byte goes back) starts there, with no glide backwards through the old one.
 f.report({ position: [40, 20, 0.2], source: 'file', byte: 0, state: 'printing', ageS: 0 }, t + 4000);
 checks++; assert.ok(f.frame(t + 4000).byte < 64, 'a restart starts over at the start of the file, not by gliding back through it');
}
{
 // A tool change blends: the tool that left goes back to its dock over the blend, the new one comes out.
 const spec = { axes: { x: [-150, 150], y: [-150, 150], z: [0, 300] }, motions: { t0: ['x', 'y', 'z'], t1: ['x', 'y', 'z'] }, tools: [{ group: 't0', park: { x: -150 } }, { group: 't1', park: { x: 150 } }] };
 const cal = { offset_mm: [-150, -150, 0], sign: [1, 1, 1], source: [0, 1, 2] };
 const f = new HeadFollow({ spec, calibration: cal, toolBlendMs: 400 });
 const at = (tool, ms) => f.report({ position: [150, 150, 40], source: 'live', state: 'printing', tool, ageS: 0 }, ms);
 at(0, 0);
 is(f.frame(10).groups.t0, [0, 0, 40], 'tool 0 is on the head');
 is(f.frame(10).groups.t1, [150, 0, 40], 'tool 1 waits');
 at(1, 1000);
 near(f.frame(1000).groups.t1[0], 150, 'at the change tool 1 is still at its dock', 1e-9);
 near(f.frame(1200).groups.t1[0], 75, 'half way through the blend it is half way out', 1e-9);
 near(f.frame(1500).groups.t1[0], 0, 'and then it is on the head', 1e-9);
 near(f.frame(1500).groups.t0[0], -150, 'while tool 0 is back at its dock', 1e-9);
 is(f.frame(1100).moving, true, 'a swap asks for frames');
 // Matched by hand: no groups, so the sliders keep the model.
 f.paused = true; is(f.frame(1500).groups, null, 'paused by the person: nothing is posed');
}

{
 // The head never waits at a target: reports every 800 ms with the file read at a steady pace; after the first two the head
 // moves in every frame, never backwards, and arrives at each report's byte when that report is due.
 let text = 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\n'; let e = 0;
 for (let i = 1; i <= 2000; i++) text += `G1 X${(100 + 40 * Math.cos(i / 60)).toFixed(3)} Y${(100 + 40 * Math.sin(i / 60)).toFixed(3)} E${(e += 0.1).toFixed(2)}\n`;
 const parsed = parseGcode(text), offs = parsed.layers.at(-1).offsets;
 const f = new HeadFollow({ parsed });
 const at = (k) => offs[k];
 let t = 0, prev = null, still = 0, back = 0, worstMiss = 0;
 for (let k = 0; k < 12; k++) {
  f.report({ position: null, source: 'file', byte: at(100 + 120 * k), state: 'printing', ageS: 0 }, t);
  for (let ms = 0; ms < 800; ms += 16) {
   const b = f.frame(t + ms).byte;
   if (k >= 3 && prev !== null) { if (b < prev) back++; if (b === prev) still++; }
   prev = b;
  }
  t += 800;
 }
 is([back, still], [0, 0], 'steady reports: the file is never read backwards and the head never stands still between reports');
 // At the moment a report is due the head is where that report said, within a report's worth of pace.
 const g = new HeadFollow({ parsed });
 for (let k = 0; k < 8; k++) g.report({ position: null, source: 'file', byte: at(100 + 120 * k), state: 'printing', ageS: 0 }, k * 800);
 const miss = Math.abs(g.frame(7 * 800 + 790).byte - at(100 + 120 * 8)) / (at(100 + 120 * 8) - at(100 + 120 * 7));
 checks++; assert.ok(miss < 0.2, `just before the next report the head is within 20 % of a report's pace of where it will be: ${(miss * 100).toFixed(0)} %`);
}

{
 // Along the path by distance. A square of short moves with one long move (100 mm) in it: while the printer is in the long
 // move the byte does not change, but the reported position does, and the head must go along the move, not wait at its start.
 let text = 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\n'; let e = 0;
 const move = (x, y) => { text += `G1 X${x} Y${y} E${(e += 0.1).toFixed(2)}\n`; };
 for (let i = 1; i <= 10; i++) move(i * 2, 0);      // 20 mm in 10 short moves
 move(120, 0);                                     // the long move: 100 mm
 for (let i = 1; i <= 10; i++) move(120, i * 2);   // 20 mm more
 const parsed = parseGcode(text), L = parsed.layers.at(-1), offs = L.offsets;
 // The helpers: distance along the path, and back.
 const longK = 10; // the eleventh move
 const a0 = arcOf(parsed, parsed.layers.length - 1, 0, 0); // the 0.2 mm lift of the start code comes first
 near(arcOf(parsed, parsed.layers.length - 1, longK, 0) - a0, 20, 'the long move starts 20 mm along the path of its layer', 0.01);
 near(arcOf(parsed, parsed.layers.length - 1, longK, 0.5) - a0, 70, 'half way along it is 70 mm', 0.01);
 const back = atArc(parsed, 70 + a0);
 is([back.segment, Math.round(back.x * 100) / 100, back.y], [longK, 70, 0], 'and 70 mm along the path is where it was');
 is(project(parsed, parsed.layers.length - 1, longK, 60, 0.4, 0.2).u, 0.4, 'a point put on the move: 40 % of the way');
 checks++; assert.ok(project(parsed, parsed.layers.length - 1, longK, 60, 9, 0.2).dist > 8, 'a point off the move is far from it');
 near(byteAt(parsed, parsed.layers.length - 1, longK, 0), offs[longK], 'the byte at the start of a move', 1e-9);
 const f = new HeadFollow({ parsed });
 const at = (k) => offs[k];
 // Reports every 800 ms, the printer 20 mm further along the long move each time. The byte is the end of the move's line (what a
 // demo printer reports), so it is the next move's start and does not change while the long move runs.
 let t = 0; const xs = [];
 for (let k = 0; k < 5; k++) {
  f.report({ position: [20 + 20 * k, 0, 0.2], source: 'file', byte: at(longK + 1), state: 'printing', ageS: 0 }, t);
  for (let ms = 0; ms < 800; ms += 16) xs.push(f.frame(t + ms).position[0]);
  t += 800;
 }
 let stalls = 0, backs = 0;
 for (let i = 60; i < xs.length; i++) { if (xs[i] <= xs[i - 1] + 1e-9) stalls++; if (xs[i] < xs[i - 1] - 1e-9) backs++; }
 is([stalls, backs], [0, 0], 'through a long move (one byte for seconds) the head keeps going along it');
 near(f.frame(t - 800 + 790).position[0], 20 + 20 * 4 + 20 * (790 / 800), 'at about the printer\'s pace', 3);
 // A position that is not on the move the byte names (a printer whose byte runs ahead of the head) is not put on it: the byte rules.
 const g = new HeadFollow({ parsed });
 g.report({ position: [100, 50, 0.2], source: 'file', byte: at(3), state: 'printing', ageS: 0 }, 0);
 is(g.arc, null, 'a position far from its move is not used');
 is(g.frame(10).along, true, 'and the file is still followed by byte');
 // 2026-10-25 (CORE One L): a printer that reports its Z but not X and Y (PrusaLink printing) keeps its own Z: at the start it is heating
 // at Z 150 with the bed down, and the file's first layer would have drawn the bed up at the nozzle.
 const h = new HeadFollow({ parsed });
 h.report({ position: null, z: 150, source: 'file', byte: at(3), state: 'printing', ageS: 0 }, 0);
 is(h.frame(10).position[2], 150, 'the live Z, not the file\'s');
 const k = new HeadFollow({ parsed });
 k.report({ position: null, source: 'file', byte: at(3), state: 'printing', ageS: 0 }, 0);
 is(k.frame(10).position[2], pathAt(parsed, at(3)).z, 'with no live Z, the file\'s');
}

console.log(`model-follow: ${checks} assertions passed.`);
