// Reads and parses a print file off the page's thread (finish-2 F04, 2026-10-19). A wall of nine printers reads nine files of 3 to 9 MB, and parsing each one on the
// page's own thread was a 190 to 290 ms hold-up (seven of them in the first ten seconds, measured with long-animation-frame); a frame is 16.7 ms. The page asks with
// {id, url, maxSegments}; the answer is {id, parsed, colors} with the parsed arrays handed over, not copied, or {id, error} in the words the page already used.
import { parseGcode } from './hud-gcode.js';
import { colorsOf } from './model-follow.js';
self.onmessage = async (e) => {
 const { id, url, maxSegments } = e.data || {};
 try {
  const r = await fetch(url, { cache: 'no-store' });
  if (!r.ok) { let m = ''; try { m = (await r.json()).error; } catch { /* no words in the answer */ } self.postMessage({ id, error: m || 'The print file could not be read.' }); return; }
  const text = await r.text(), parsed = parseGcode(text, { maxSegments }), colors = colorsOf(text);
  const buffers = [];
  for (const l of parsed.layers) for (const k of ['segments', 'extrude', 'offsets', 'tool']) buffers.push(l[k].buffer);
  self.postMessage({ id, parsed, colors }, buffers);
 } catch (err) { self.postMessage({ id, error: String((err && err.message) || err) }); }
};
