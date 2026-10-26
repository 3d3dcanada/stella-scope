// The one G-code follower used by every view (finish board WO-01): printer page stage, farm wall
// tile, phone and demo printers. It parses a job's file once (web/hud-gcode.js, reused for every
// brand), caches it per job, and turns the best signal the printer reports into {layer, fraction}
// plus a plain-words source label: "live" when live X/Y is reported, "following the file"
// otherwise. A reported Z re-anchors the drawn height so the model is never on the wrong layer.
// Between reports the head plays forward from the last drawn position; on the next report it snaps
// to the mapped one. Nothing here is ever guessed: no file access and no layer number means
// unknown, shown in words.
import { parseGcode } from './hud-gcode.js';

// The best signal, in order (WO-01): byte position, the printer's own progress percent, current
// layer of total, elapsed time against the file's own estimate.
export const SIGNAL_BYTE = 'byte';
export const SIGNAL_PROGRESS = 'progress';
export const SIGNAL_LAYER = 'layer';
export const SIGNAL_TIME = 'time';

// One parsed job, held by file path + byte length so a re-sliced file is re-parsed.
const cache = new Map();

export function cachedParse(path, bytes, text) {
  const key = `${path}\0${bytes}`;
  let hit = cache.get(key);
  if (!hit) {
    hit = { parsed: parseGcode(text, { maxSegments: 2_000_000 }), bytes };
    cache.set(key, hit);
  }
  return hit.parsed;
}

export function clearCache() {
  cache.clear();
}

// ---- pure signal mappings (mirrored in Rust in src/gcode_follow.rs) --------------------------

// Byte offset in the file -> {layer, fraction} (Moonraker's file_position counts bytes).
export function mapByte(parsed, offset) {
  const offsets = parsed.byteOffsets;
  if (!Array.isArray(offsets) || offsets.length === 0) return null;
  const at = Math.min(offset, parsed.byteLength ?? offsets[offsets.length - 1] ?? 0);
  let lo = 0, hi = offsets.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid] <= at) lo = mid + 1; else hi = mid;
  }
  const layer = Math.max(0, lo - 1);
  const start = offsets[layer];
  const end = layer + 1 < offsets.length ? offsets[layer + 1] : (parsed.byteLength ?? start);
  const span = end - start;
  const fraction = span > 0 ? Math.max(0, Math.min(1, (at - start) / span)) : 0;
  return { layer, fraction };
}

// Progress fraction 0..1 -> {layer, fraction} through the file's byte spans (Prusa computes its
// progress from M73 lines in the file; the page maps it onto those spans).
export function mapProgress(parsed, progress) {
  const p = Math.max(0, Math.min(1, progress));
  return mapByte(parsed, Math.floor((parsed.byteLength ?? 0) * p));
}

// Current layer (1-based) of total -> {layer, fraction 0}.
export function mapLayer(current, total) {
  const top = total && total > 0 ? total - 1 : Infinity;
  return { layer: Math.max(0, Math.min(current - 1, top)), fraction: 0 };
}

// Elapsed seconds against the file's own estimate -> {layer, fraction}.
export function mapTime(parsed, elapsed, estimate) {
  if (!estimate || estimate <= 0 || elapsed < 0) return null;
  const total = parsed.totalLayers ?? parsed.layers.length;
  if (!total) return null;
  const raw = Math.min(1, elapsed / estimate) * total;
  const layer = Math.min(Math.floor(raw), total - 1);
  return { layer, fraction: Math.max(0, Math.min(1, raw - layer)) };
}

// Best signal first: byte, progress, layer, time. Returns {layer, fraction} or null.
export function mapSignal(parsed, signal) {
  if (!parsed) return null;
  if (signal.byte !== undefined && Number.isFinite(signal.byte)) {
    const m = mapByte(parsed, signal.byte);
    return m ? { ...m, source: 'following the file' } : null;
  }
  if (Number.isFinite(signal.progress)) {
    const m = mapProgress(parsed, signal.progress);
    return m ? { ...m, source: 'following the file' } : null;
  }
  if (Number.isFinite(signal.layer)) {
    const m = mapLayer(signal.layer, signal.layers);
    return { ...m, source: 'following the file' };
  }
  if (Number.isFinite(signal.elapsed) && Number.isFinite(signal.estimate)) {
    const m = mapTime(parsed, signal.elapsed, signal.estimate);
    return m ? { ...m, source: 'following the file' } : null;
  }
  return null;
}

// ---- the follower ----------------------------------------------------------------------------

export class GcodeFollower {
  constructor() {
    this.parsed = null;
    this.job = null;
    this.bytes = 0;
    // Last re-anchored Z, kept so a print that stops reporting Z keeps its last real height.
    this.z = null;
  }

  // Load (or reuse from cache) the parsed file for one job. Call once per job.
  loadJob(path, bytes, text) {
    const parsed = cachedParse(path, bytes, text);
    this.job = path;
    this.parsed = parsed;
    this.bytes = parsed.byteLength ?? bytes;
    return parsed;
  }

  // One tick. `signal` is the best file signal the printer gives; `live` is true when live X/Y
  // coordinates are reported; `z` is a reported height. Returns the drawn
  // state, or null when nothing is known.
  tick({ signal, live, z } = {}) {
    if (!this.parsed) return null;
    const mapped = mapSignal(this.parsed, signal);
    if (!mapped) return null;
    const source = live ? 'live' : 'following the file';
    if (Number.isFinite(z)) this.z = z;
    return { layer: mapped.layer, fraction: mapped.fraction, source, z: this.z };
  }
}
