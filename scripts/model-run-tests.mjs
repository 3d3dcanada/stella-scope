// node scripts/model-run-tests.mjs: a printer page's model says "the part is on the plate" once some of it shows, and
// never when nothing is drawn (polish board WO-03, Opus S4, 2026-10-18). The browser is stubbed; three.js is the real one.
import assert from 'node:assert/strict';
import * as THREE from '../web/vendor/three.module.js';

const events = [];
globalThis.document = { dispatchEvent: (e) => { events.push(e); return true; } };
globalThis.CustomEvent = class { constructor(type, init) { this.type = type; this.detail = init?.detail; } };
globalThis.requestAnimationFrame = () => 1; globalThis.cancelAnimationFrame = () => {};
const files = {
 'part.gcode': 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\nG1 X10 Y0 E1\nG1 X10 Y10 E2\nG1 X0 Y10 E3\nG1 X0 Y0 E4\n',
 'other.gcode': 'G90\nM82\nG1 X5 Y5 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\nG1 X20 Y5 E1\nG1 X20 Y20 E2\n',
 'empty.gcode': 'G90\nM82\nG1 X0 Y0 Z0.2 F3000\nG1 X10 Y0\n',
};
globalThis.fetch = async (url) => {
 const name = decodeURIComponent(String(url).split('filename=')[1]);
 return { ok: true, text: async () => files[name], json: async () => ({}) };
};
const { runModel } = await import('../web/model-run.js');

let checks = 0;
const is = (a, b, label) => { checks++; assert.deepEqual(a, b, label); };
const until = async (fn, label) => { for (let i = 0; i < 200; i++) { if (fn()) return; await new Promise((r) => setTimeout(r, 10)); } assert.fail(`timed out: ${label}`); };
const view = () => ({ ext: { root: new THREE.Group(), holder: () => new THREE.Group(), B: [[1, 0, 0], [0, 0, 1], [0, -1, 0]], want() {}, hold() {} }, setGroups() {} });
const frame = { origin_mm: [0, 0, 0], axes: [[1, 0, 0], [0, 1, 0], [0, 0, 1]], group: null };
const st = (file, byte, state) => ({ model: 'm', job: { file, file_position: byte, progress: 0.5, state }, position: {}, pose_spec: null, calibration: null, print_frame: frame, position_source: 'file', pose: {} });
const parts = () => events.filter((e) => e.type === 'stella-part-drawn');
const text = files['part.gcode'], afterFirst = text.indexOf('G1 X10 Y10');

{
 // Heating: the file is read and the part is in the scene, but none of it shows yet: no word that it is on the plate.
 const r = runModel(view(), { prefix: '/api/p/x' });
 r.update(st('part.gcode', 0, 'heating'));
 await until(() => r.drawn, 'the part is built');
 r.step(performance.now() + 3000);
 is(parts().length, 0, 'nothing shows while heating, so the page says nothing');
 // The printer reads the first lines: it shows now, and the page says so once.
 r.update(st('part.gcode', afterFirst, 'printing'));
 r.step(performance.now() + 3000);
 is(parts().length, 1, 'the first of the part shows, so the page says so');
 is(parts()[0].detail, { file: 'part.gcode', layers: 2 }, 'with the file and its layers');
 r.step(performance.now() + 4000); r.step(performance.now() + 5000);
 is(parts().length, 1, 'and only once');
 // Another file: the part is built again and announced again, in its turn.
 r.update(st('other.gcode', files['other.gcode'].length, 'printing'));
 await until(() => r.file === 'other.gcode' && r.pending, 'the next file is built');
 is(parts().length, 1, 'built, not yet shown');
 r.step(performance.now() + 9000);
 is(parts().map((e) => e.detail.file), ['part.gcode', 'other.gcode'], 'the next file is announced when it shows');
 r.stop();
}
{
 // A wall tile never says it: the wall is not a page with a toast.
 const before = parts().length, r = runModel(view(), { prefix: '/api/p/x', tile: true });
 r.update(st('part.gcode', text.length, 'printing'));
 await until(() => r.drawn, 'the tile part is built');
 r.step(performance.now() + 3000);
 is([parts().length, r.pending], [before, null], 'a tile announces nothing');
 r.stop();
}
{
 // A file with nothing to draw: an error, and no word that it is drawn.
 const before = parts().length, r = runModel(view(), { prefix: '/api/p/x' });
 r.update(st('empty.gcode', 20, 'printing'));
 await until(() => r.error, 'the empty file is refused');
 r.step(performance.now() + 3000);
 is([parts().length, r.drawn, r.pending], [before, null, null], 'nothing drawn, nothing said');
 r.stop();
}
{
 // A machine whose plate is not measured (no print frame) draws nothing and says nothing.
 const before = parts().length, r = runModel(view(), { prefix: '/api/p/x' });
 r.update({ ...st('part.gcode', text.length, 'printing'), print_frame: null });
 r.step(performance.now() + 3000);
 is([parts().length, r.drawn], [before, null], 'no frame, no part, no word');
 r.stop();
}
{
 // F04 (2026-10-19): how often a wall tile is drawn grows with the tiles that are live and with a page that is falling behind, and comes back when it keeps pace.
 const { tileGapMs, nextLoad } = await import('../web/model-run.js');
 is([tileGapMs(1), tileGapMs(3), tileGapMs(7), tileGapMs(10), tileGapMs(30)], [50, 50, 98, 140, 160], 'the gap grows by 14 ms a tile, 50 ms at the least, 160 ms at the most');
 is([tileGapMs(7, 2), tileGapMs(30, 3)], [196, 480], 'and it is longer again under load');
 is(tileGapMs(2, 1, 80), 80, 'a base gap is the least');
 is(nextLoad(1, 5, 16.7), { load: 1, calm: 6 }, 'a frame at pace is calm');
 is(nextLoad(1, 5, 40), { load: 1.5, calm: 0 }, 'a long frame raises the load by half a step');
 is(nextLoad(2.75, 0, 33), { load: 3, calm: 0 }, 'the load stops at 3');
 is(nextLoad(2, 5, 4000), { load: 2, calm: 6 }, 'a frame of seconds is a tab that was away, not a slow page');
 is(nextLoad(2, 5, 40, true), { load: 2, calm: 6 }, 'a hidden tab is not slow either');
 is(nextLoad(2, 90, 16.7), { load: 1.75, calm: 0 }, '90 calm frames lower it by a quarter');
 is(nextLoad(1, 200, 16.7), { load: 1, calm: 201 }, 'it does not go under 1');
}
console.log(`model-run: ${checks} assertions passed.`);
