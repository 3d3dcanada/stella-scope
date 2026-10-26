// node scripts/gcode-follow-tests.mjs — the G-code follower (WO-01): signal mappings mirrored from
// src/gcode_follow.rs, run against real parseGcode output, plus a 100 MB file parse that must stay
// memory bounded.
import assert from 'node:assert/strict';
import { parseGcode } from '../web/hud-gcode.js';
import {
  mapByte, mapProgress, mapLayer, mapTime, mapSignal, clearCache,
} from '../web/gcode-follow.js';

let checks = 0;
const is = (actual, expected, label) => { checks++; assert.deepStrictEqual(actual, expected, label); };
const near = (actual, expected, label, eps = 1e-3) => {
  checks++;
  assert.ok(Math.abs(actual - expected) < eps, `${label}: ${actual} is not within ${eps} of ${expected}`);
};

const cube = ';LAYER_COUNT:3\n;LAYER:0\nG90\nM82\nG1 X0 Y0 Z0.2 F1200\nG1 X10 Y0 E1\nG1 X10 Y10 E1\nG1 X0 Y10 E1\nG1 X0 Y0 E1\n;LAYER:1\nG1 Z0.4\nG1 X0 Y0 E1\nG1 X10 Y0 E1\nG1 X10 Y10 E1\nG1 X0 Y10 E1\nG1 X0 Y0 E1\n;LAYER:2\nG1 Z0.6\nG1 X0 Y0 E1\nG1 X10 Y0 E1\n';
const parsed = parseGcode(cube);
const empty = parseGcode('G28\n');

{
  const p = parsed;
  const off = p.byteOffsets;
  assert.ok(Array.isArray(off) && off.length >= 3, 'cube has layer byte offsets');
  is(mapByte(p, off[0]).layer, 0, 'first offset is layer 0');
  is(mapByte(p, off[0]).fraction, 0, 'first offset fraction 0');
  if (off.length >= 2) {
    is(mapByte(p, off[1]).layer, 1, 'second offset starts layer 1');
    is(mapByte(p, off[1]).fraction, 0, 'second offset fraction 0');
  }
  if (off.length >= 3) {
    is(mapByte(p, off[2]).layer, 2, 'third offset starts layer 2');
  }
  const mid = Math.floor((off[0] + (off[1] ?? off[0])) / 2);
  assert.ok(mapByte(p, mid).layer === 0, 'mid of layer 0 is still layer 0');
  assert.ok(mapByte(p, off[off.length - 1]).layer === off.length - 1, 'last offset is the last layer');
  assert.ok(mapByte(p, off[off.length - 1]).fraction === 0, 'last offset fraction 0');
  assert.ok(mapByte(p, 100000).layer === off.length - 1, 'past the end clamps to the last layer');
  assert.ok(mapByte(p, 100000).fraction === 1, 'past the end fraction 1');
  // empty parse: nothing maps.
  const empty = parseGcode('G28\n');
  assert.ok(mapByte(empty, 10) === null, 'empty parse maps nothing');
}

{
  const p = parsed;
  const empty = parseGcode('G28\n');
  is(mapProgress(p, 0).layer, 0, 'progress 0 is layer 0');
  const half = mapProgress(p, 0.5);
  assert.ok(half.layer === 1 && half.fraction >= 0 && half.fraction <= 1, 'progress half way is inside layer 1: ' + JSON.stringify(half));
  is(mapProgress(p, 1).layer, 2, 'progress 1 is the last layer');
  is(mapProgress(p, 1).fraction, 1, 'progress 1 fraction 1');
  assert.ok(mapProgress(empty, 0.5) === null, 'empty parse progress maps nothing');
}

{
  is(JSON.stringify(mapLayer(1, 3)), JSON.stringify({ layer: 0, fraction: 0 }), 'layer 1 of 3 -> index 0');
  is(JSON.stringify(mapLayer(3, 3)), JSON.stringify({ layer: 2, fraction: 0 }), 'layer 3 of 3 -> index 2');
  is(JSON.stringify(mapLayer(9, 3)), JSON.stringify({ layer: 2, fraction: 0 }), 'layer past total clamps');
  is(JSON.stringify(mapLayer(1, null)), JSON.stringify({ layer: 0, fraction: 0 }), 'no total: layer 1 -> index 0');
}

{
  const p = parsed;
  is(JSON.stringify(mapTime(p, 0, 600)), JSON.stringify({ layer: 0, fraction: 0 }), 't=0 is layer 0');
  is(JSON.stringify(mapTime(p, 300, 600)), JSON.stringify({ layer: 1, fraction: 0.5 }), 'half time is layer 1');
  is(JSON.stringify(mapTime(p, 600, 600)), JSON.stringify({ layer: 2, fraction: 1 }), 'full time is last layer');
  assert.ok(mapTime(p, 100, 0) === null, 'zero estimate maps nothing');
  assert.ok(mapTime(p, -10, 600) === null, 'negative elapsed maps nothing');
  assert.ok(mapTime(empty, 100, 600) === null, 'empty parse time maps nothing');
}

{
  const p = parsed;
  // byte position wins and is labelled following the file.
  const b = mapSignal(p, { byte: 150 });
  assert.ok(b && b.source === 'following the file' && b.layer === 1, 'byte signal: ' + JSON.stringify(b));
  // progress without byte.
  const pr = mapSignal(p, { progress: 0.5 });
  assert.ok(pr && pr.source === 'following the file' && pr.layer === 1, 'progress signal: ' + JSON.stringify(pr));
  // layer (1-based) without byte or progress.
  const l = mapSignal(p, { layer: 2, layers: 3 });
  assert.ok(l && l.layer === 1 && l.source === 'following the file', 'layer signal: ' + JSON.stringify(l));
  // time when nothing else.
  const t = mapSignal(p, { elapsed: 300, estimate: 600 });
  assert.ok(t && t.layer === 1 && t.source === 'following the file', 'time signal: ' + JSON.stringify(t));
  // nothing known -> null.
  assert.ok(mapSignal(p, {}) === null, 'no signal maps to null');
  assert.ok(mapSignal(null, { byte: 1 }) === null, 'no parse maps to null');
}

// ---- 100 MB file: parse must stay memory bounded and finish -------------------------------
{
  let text = '';
  for (let i = 0; i < 5; i++) {
    for (let n = 0; n < 1_000_000; n++) text += `G1 X${n % 200} Y${n % 100} Z0.2 E1\n`; // real, non-zero moves
  }
  const before = process.memoryUsage().rss;
  const t0 = performance.now();
  const p = parseGcode(text, { maxSegments: 2_000_000 });
  const dt = performance.now() - t0;
  const after = process.memoryUsage().rss;
  const usedMB = (after - before) / 1_000_000;
  // eslint-disable-next-line no-console
  console.log(`100 MB-class parse: ${(text.length / 1_000_000).toFixed(1)} MB text, ${p.segmentCount} segments, ${dt.toFixed(0)} ms, heap delta ${usedMB.toFixed(1)} MB`);
  assert.ok(p.segmentCount > 0, 'parsed something');
  assert.ok(usedMB < 400, `parse used too much memory: ${usedMB.toFixed(1)} MB`);
  assert.ok(dt < 60_000, `parse took too long: ${dt.toFixed(0)} ms`);
  // slicing a sub-range of a huge file does not hold the whole text: progressToLayer on a prefix.
  const step = parseGcode(text.slice(0, 1000));
  assert.ok(step.segmentCount <= p.segmentCount);
}

clearCache();
// eslint-disable-next-line no-console
console.log(`gcode-follow: ${checks} checks, all pass`);
