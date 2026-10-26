// The print on the plate of a model (polish board WO-03, 2026-10-18): the parsed G-code drawn as lines in the model's
// frame, growing as the printer reads the file. The geometry is made by model-follow.js (`buildPrint`, pure and
// tested); this only hands it to three.js and sets how much of it is drawn. It rides with the motion group that
// carries the plate (a bed that drops or slides), so the part moves with its bed and the nozzle stays on it.
import * as THREE from './vendor/three.module.js';
import { buildPrint, countAt } from './model-follow.js';

/** `ext` is what the model view hands out: `{root, holder, B, want}`. */
export function createPrintLayer(ext) {
 let lines = null, built = null, shown = -1;
 const clear = () => {
  if (!lines) return;
  lines.parent?.remove(lines); lines.geometry.dispose(); lines.material.dispose();
  lines = built = null; shown = -1;
 };
 return {
  /** Build the print for `frame` (the machine's print frame). Returns what was drawn, or null when nothing can be:
   *  no frame (this model's plate is not measured) or a file with no extruding move. */
  set(parsed, frame, { budget = Infinity, colors } = {}) {
   clear();
   if (!parsed || !frame) return null;
   built = buildPrint(parsed, frame, ext.B, { budget, ...(colors ? { colors } : {}) });
   if (!built.kept) { built = null; return null; }
   const g = new THREE.BufferGeometry();
   g.setAttribute('position', new THREE.BufferAttribute(built.positions, 3));
   g.setAttribute('color', new THREE.BufferAttribute(built.colors, 3));
   g.setDrawRange(0, 0);
   lines = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ vertexColors: true }));
   lines.frustumCulled = false; // its box is the whole file, which the camera frames anyway
   lines.name = 'print';
   (frame.group ? ext.holder(frame.group) : ext.root).add(lines);
   shown = 0; ext.want();
   return { segments: built.segments, kept: built.kept, layers: built.layersKept, tools: built.tools };
  },
  /** Draw what the printer has read: segments up to `byte` (null draws all). */
  show(byte) {
   if (!lines) return;
   const n = countAt(built, byte);
   if (n !== shown) { shown = n; lines.geometry.setDrawRange(0, n * 2); ext.want(); }
  },
  info: () => (built ? { kept: built.kept, shown, segments: built.segments } : null),
  clear,
  dispose: clear,
 };
}
