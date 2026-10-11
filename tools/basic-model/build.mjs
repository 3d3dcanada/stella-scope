#!/usr/bin/env node
// Builds the basic printer model pack from a converted reference assembly.
//
//   node tools/basic-model/build.mjs --geometry <converted geometry.json> --out <directory>
//
// Input: the geometry.json that tools/asset-import writes for the Voron 2.4 R2 350 reference CAD
// (hartk1213/MISC, GPL-3.0). Output: <directory>/voron-24r2-350-basic/geometry.json plus a
// manifest.json entry, laid out like a managed asset library, ready to tar up and publish.
//
// The basic model keeps the frame, gantry, drives, bed, panels and one stock toolhead (the E3D V6
// option of the reference's hotend variants and the inductive probe). It drops fasteners and other
// small hardware, which are most of the triangles and none of the shape, and rounds coordinates to
// 0.01 mm. Nothing is moved, scaled or re-tessellated.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const arg = (name) => { const i = process.argv.indexOf(name); return i > 0 ? process.argv[i + 1] : null; };
const input = arg('--geometry'), outDir = arg('--out');
if (!input || !outDir) { console.error('usage: build.mjs --geometry <geometry.json> --out <directory>'); process.exit(2); }

const ID = 'voron-24r2-350-basic';
const HARDWARE = /screw|bolt|\bnut\b|t-nut|washer|shim|insert|magnet|\bpin\b|dowel|rivet|spring|bearing|\bf?6[0-9]{2}\b|zz\b|shcs|bhcs|fhcs|\bm[2-8]x\d/i;
const OTHER_TOOLHEADS = /^(E3D Revo Micro v4|Mosquito|Dragon|Dragonfly_BMS|Dragonfly_BMO \(1\)):1$/;
const OTHER_PROBE = /^(Panasonic GX-H15A|probe_bracket_9mm \(For Panasonic Probe\)):1$/;

const data = JSON.parse(fs.readFileSync(input, 'utf8'));
const keep = new Set();
let dropped = 0;

function walk(node, inOptional) {
  const optional = inOptional || OTHER_TOOLHEADS.test(node.name || '') || OTHER_PROBE.test(node.name || '');
  const meshes = [], ids = [];
  (node.meshes || []).forEach((index, k) => {
    const name = data.meshes[index]?.name || node.name || '';
    if (optional || HARDWARE.test(name) || HARDWARE.test(node.name || '')) { dropped++; return; }
    keep.add(index); meshes.push(index); ids.push((node.instance_ids || [])[k]);
  });
  const children = (node.children || []).map((c) => walk(c, optional)).filter(Boolean);
  if (!meshes.length && !children.length) return null;
  return { ...node, meshes, instance_ids: ids, children };
}

const root = walk(data.root, false);
const order = [...keep].sort((a, b) => a - b), remap = new Map(order.map((old, i) => [old, i]));
function renumber(node) { node.meshes = node.meshes.map((i) => remap.get(i)); node.children.forEach(renumber); }
renumber(root);

const round = (v) => Math.round(v * 100) / 100;
let triangles = 0;
const meshes = order.map((i) => {
  const m = data.meshes[i];
  triangles += m.index.array.length / 3;
  return { ...m, attributes: { position: { array: m.attributes.position.array.map(round) } } };
});

const out = { root, meshes, printer_frame: data.printer_frame };
const dir = path.join(outDir, ID);
fs.mkdirSync(dir, { recursive: true });
const text = JSON.stringify(out);
fs.writeFileSync(path.join(dir, 'geometry.json'), text);
const sha = crypto.createHash('sha256').update(text).digest('hex');

const entry = {
  id: ID,
  name: 'Voron 2.4 R2 350 · basic model',
  format: 'step',
  source_name: '2.4r2-350.step (not included; the geometry is the converted, reduced display mesh)',
  units: 'mm',
  status: 'suggested',
  slot_id: 'frame',
  parent_id: null,
  manufacturer: 'Voron community / hartk1213',
  model: 'Voron 2.4 R2 350 assembly, basic model',
  revision: 'hartk1213/MISC Voron 2-Trident/2.4/350_CAD_assembly, reduced by tools/basic-model/build.mjs',
  source_url: 'https://github.com/hartk1213/MISC/tree/main/Voron%20Mods/Voron%202-Trident/2.4/350_CAD_assembly',
  license: 'GPL-3.0 (derived from the hartk1213/MISC Voron 2.4 assembly)',
  notes: 'Stock reference printer. Fasteners and small hardware removed, one hotend option kept, coordinates rounded to 0.01 mm. Not a measured model of any particular printer.',
  placement: { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1, basis: 'Native STEP Z-up millimetre placement, unchanged' },
  derived_sha256: sha,
  derived_bytes: Buffer.byteLength(text),
  mesh_count: meshes.length,
  triangle_count: triangles,
  storage: 'managed-copy',
  geometry_url: `/api/assets/${ID}/geometry`,
};
fs.writeFileSync(path.join(outDir, 'manifest.json'), JSON.stringify({ schema: 'stella-scope.assets.v1', assets: [entry] }, null, 1) + '\n');
console.log(`${ID}: ${meshes.length} meshes (${dropped} dropped), ${triangles.toLocaleString()} triangles, ${(Buffer.byteLength(text) / 1048576).toFixed(1)} MiB`);
