// node scripts/model-status-tests.mjs: what a wall tile says about its 3D model (F03, 2026-10-19), from the service's answer to
// /api/printer-models/wanted. The words and the kind of each state decide which button a tile offers, so they are pinned here.
import assert from 'node:assert/strict';
import { modelStatus } from '../web/next-getmodels.js';

const row = (id, o = {}) => ({ id, label: id, open: true, bytes: 1000, notice: false, why: null, ...o });
const item = (id, state, o = {}) => ({ id, label: id, kind: 'model', bytes: 1000, got: 0, state, ...o });
const answer = ({ needs, models = [], items = [], state = 'running' }) => ({ needs, models, fetch: { state, items } });
let n = 0;
const test = (name, fn) => { fn(); n++; };

test('nothing to say when the printer waits for nothing', () => {
 assert.equal(modelStatus(answer({ needs: {} }), 'a'), null);
 assert.equal(modelStatus(answer({ needs: { b: ['m'] } }), 'a'), null);
 assert.equal(modelStatus(null, 'a'), null);
 assert.equal(modelStatus({}, 'a'), null);
});

test('a file queued behind others is waiting for its turn, one arriving has its own percentage', () => {
 const w = answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'queued')] });
 assert.deepEqual(modelStatus(w, 'a'), { state: 'waiting', pct: 0, got: 0, total: 1000, checking: false });
 const g = answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'fetching', { got: 250 })] });
 assert.deepEqual(modelStatus(g, 'a'), { state: 'getting', pct: 0.25, got: 250, total: 1000, checking: false });
});

test('every byte read but not yet kept: the tile says it is checking the file, not that it is still downloading', () => {
 const done = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'fetching', { got: 1000 })] }), 'a');
 assert.deepEqual([done.state, done.pct, done.checking], ['getting', 1, true]);
 const part = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'fetching', { got: 999 })] }), 'a');
 assert.equal(part.checking, false);
 const unknown = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'fetching', { got: 0, bytes: 0 })] }), 'a');
 assert.equal(unknown.checking, false, 'a file whose size is not known is never called checked');
});

test('a toolchanger counts its model and its tool files as one: the bar is all their bytes', () => {
 const items = [item('base', 'done', { got: 1000, bytes: 1000 }), item('t1', 'fetching', { got: 100, bytes: 100, kind: 'part' }), item('t2', 'queued', { bytes: 100, kind: 'part' })];
 const s = modelStatus(answer({ needs: { a: ['base', 't1', 't2'] }, models: [row('base'), row('t1'), row('t2')], items }), 'a');
 assert.equal(s.state, 'getting');
 assert.equal(s.total, 1200);
 assert.equal(s.got, 1100);
 assert.ok(Math.abs(s.pct - 1100 / 1200) < 1e-9);
});

test('a file that has landed but is still on the list (the answer was read a moment early) is arriving at 100 percent', () => {
 const s = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'done', { got: 1000 })] }), 'a');
 assert.deepEqual([s.state, s.pct], ['getting', 1]);
});

test('a failure is said in the service words, with its kind, and wins over files still arriving', () => {
 const f = { kind: 'offline', plain: 'No connection to the author\'s site, so the 3D model was not fetched. Check the internet connection, then try again.' };
 const s = modelStatus(answer({ needs: { a: ['m', 'p'] }, models: [row('m'), row('p')], items: [item('m', 'failed', { failure: f, error: 'raw' }), item('p', 'fetching')], state: 'done' }), 'a');
 assert.deepEqual([s.state, s.kind, s.plain], ['failed', 'offline', f.plain]);
 const bare = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m')], items: [item('m', 'failed', { error: 'raw words' })] }), 'a');
 assert.deepEqual([bare.state, bare.kind, bare.plain], ['failed', 'other', 'raw words']);
});

test('nothing fetching it: a press gets it; a model whose maker\'s notice comes first, or that is added by hand, says so', () => {
 const m = modelStatus(answer({ needs: { a: ['m'] }, models: [row('m', { bytes: 5000 })], state: 'idle' }), 'a');
 assert.deepEqual(m, { state: 'missing', bytes: 5000 });
 // Finish-2 F09 (2026-10-19): there is no "notice first" state any more. The models Prusa shares under its own licence ship inside the build, so a farm never waits
 // for a download behind a notice; the CORE One L's row that said {notice:true} now never reaches this answer, and a model that is neither open nor fetchable says why ("hand").
 const core = row('core', { open: false, bytes: 10_225_282, why: 'The CORE One L model ships inside Stella Scope and is put in your library when the service starts.' });
 const nt = modelStatus(answer({ needs: { a: ['core'] }, models: [core], state: 'idle' }), 'a');
 assert.deepEqual([nt.state, nt.plain], ['hand', core.why]);
 const hand = modelStatus(answer({ needs: { a: ['x'] }, models: [row('x', { open: false, why: 'The X model comes from its maker\'s own files and is added by hand under Printer models.' })], state: 'idle' }), 'a');
 assert.deepEqual([hand.state, hand.plain], ['hand', 'The X model comes from its maker\'s own files and is added by hand under Printer models.']);
});

test('the same answer gives each printer its own state', () => {
 const w = answer({ needs: { a: ['m1'], b: ['m2'], c: ['m3'] }, models: [row('m1'), row('m2'), row('m3')], items: [item('m1', 'fetching', { got: 500 }), item('m2', 'queued'), item('m3', 'failed', { failure: { kind: 'gone', plain: 'gone' } })] });
 assert.deepEqual(['a', 'b', 'c'].map((p) => modelStatus(w, p).state), ['getting', 'waiting', 'failed']);
});

console.log(`model-status-tests: ${n} cases passed`);
