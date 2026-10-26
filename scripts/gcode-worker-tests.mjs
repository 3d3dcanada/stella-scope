// node scripts/gcode-worker-tests.mjs: the worker that reads and parses a print file for the farm wall (finish-2 F04, 2026-10-19) gives the page exactly
// what parsing it on the page's thread gave, hands the arrays over instead of copying them, and says a refusal in the printer's own words.
import assert from 'node:assert/strict';
import { parseGcode } from '../web/hud-gcode.js';
import { colorsOf } from '../web/model-follow.js';

const sent = [];
globalThis.self = { postMessage: (msg, transfer) => sent.push({ msg, transfer }) };
const text = 'G90\nM82\n;extruder_colour = "#04D9C4";"#FF6A00"\nG1 X0 Y0 Z0.2 F3000\n;LAYER_CHANGE\n;Z:0.2\nG1 X10 Y0 E1\nG1 X10 Y10 E2\n;LAYER_CHANGE\n;Z:0.4\nG1 X0 Y10 E3\nG1 X0 Y0 E4\n';
globalThis.fetch = async (url) => String(url).includes('refused')
 ? { ok: false, json: async () => ({ error: 'The printer has no such file.' }) }
 : String(url).includes('broken') ? { ok: false, json: async () => { throw Error('not json'); } }
 : { ok: true, text: async () => text };
await import('../web/gcode-worker.js');
const ask = async (data) => { sent.length = 0; await self.onmessage({ data }); assert.equal(sent.length, 1, 'one answer per question'); return sent[0]; };
let checks = 0;
const is = (a, b, label) => { checks++; assert.deepEqual(a, b, label); };

{ // the answer is what the page would have parsed, with the same colours
 const { msg, transfer } = await ask({ id: 7, url: '/api/p/a/print/gcode?filename=part.gcode', maxSegments: 400000 });
 const direct = parseGcode(text, { maxSegments: 400000 });
 is(msg.id, 7, 'the answer carries the question id');
 is(msg.parsed, direct, 'the same parse as on the page');
 is(msg.colors, colorsOf(text), 'the same extruder colours');
 is(msg.colors, [0x04D9C4, 0xFF6A00], 'two colours read from the file');
 // every array of every layer is handed over, none copied
 const want = []; for (const l of direct.layers) for (const k of ['segments', 'extrude', 'offsets', 'tool']) want.push(l[k].byteLength);
 is(transfer.map((b) => b.byteLength), want, 'the layers\' buffers are transferred');
 is(direct.layers.length, 3, 'the start of the file, then two layers');
}
{ // a refusal keeps the printer's words; an answer with none gets the page's own
 is((await ask({ id: 8, url: '/api/p/a/print/gcode?filename=refused' })).msg, { id: 8, error: 'The printer has no such file.' }, 'the printer\'s words');
 is((await ask({ id: 9, url: '/api/p/a/print/gcode?filename=broken' })).msg, { id: 9, error: 'The print file could not be read.' }, 'the page\'s words');
}
console.log(`gcode-worker: ${checks} assertions passed.`);
