import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const importer = path.join(here, "import.mjs");
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), "stella-asset-import-"));
const database = path.join(temporary, "recording.sqlite3");

function binaryStl() {
  const bytes = Buffer.alloc(84 + 50);
  bytes.write("Stella Scope test triangle");
  bytes.writeUInt32LE(1, 80);
  bytes.writeFloatLE(0, 84);
  bytes.writeFloatLE(0, 88);
  bytes.writeFloatLE(1, 92);
  [[0, 0, 0], [1, 0, 0], [0, 1, 0]].forEach((vertex, vertexIndex) => {
    vertex.forEach((value, axis) => bytes.writeFloatLE(value, 96 + vertexIndex * 12 + axis * 4));
  });
  return bytes;
}

let specSequence = 0;
async function run(spec, expectedStatus = 0) {
  const specPath = path.join(temporary, `spec-${specSequence++}.json`);
  await fs.writeFile(specPath, JSON.stringify(spec));
  const result = spawnSync(process.execPath, [importer, "--database", database, "--spec", specPath], { encoding: "utf8" });
  assert.equal(result.status, expectedStatus, result.stderr || result.stdout);
  return result;
}

try {
  const stl = path.join(temporary, "triangle.stl");
  const stlBytes = binaryStl();
  await fs.writeFile(stl, stlBytes);
  const baseSpec = {
    id: "test-triangle-v1",
    name: "Test triangle",
    source: stl,
    format: "stl",
    units: "mm",
    slot_id: "head.toolhead-cowl",
    placement: { position: [1, 2, 3], rotation: [0, 0, 0], scale: 2, basis: "test fixture" },
  };
  const first = JSON.parse((await run(baseSpec)).stdout);
  assert.equal(first.idempotent, false);
  assert.equal(first.asset.status, "unverified");
  assert.equal(first.asset.slot_id, "head.toolhead-cowl");
  assert.deepEqual(first.asset.conversion, {
    engine: "three-stlloader",
    version: "0.186.0",
    geometry: "triangulated-display",
  });
  const managed = `${database}.assets`;
  const manifestPath = path.join(managed, "manifest.json");
  const manifestBytes = await fs.readFile(manifestPath);
  const manifest = JSON.parse(manifestBytes);
  assert.equal(manifest.schema, "stella-scope.assets.v1");
  assert.equal(manifest.assets.length, 1);
  assert.equal(manifest.assets[0].source_sha256, crypto.createHash("sha256").update(stlBytes).digest("hex"));
  const copied = await fs.readFile(path.join(managed, baseSpec.id, "original.stl"));
  assert.deepEqual(copied, stlBytes);
  const geometry = JSON.parse(await fs.readFile(path.join(managed, baseSpec.id, "geometry.json"), "utf8"));
  assert.deepEqual(geometry.meshes[0].attributes.position.array.slice(0, 9), [1, 2, 3, 3, 2, 3, 1, 4, 3]);
  assert.deepEqual(geometry.root.instance_ids, ["test-triangle-v1:root:0"]);

  const duplicate = JSON.parse((await run(baseSpec)).stdout);
  assert.equal(duplicate.idempotent, true);
  assert.deepEqual(await fs.readFile(manifestPath), manifestBytes);

  const conflict = await run({ ...baseSpec, name: "Conflicting revision" }, 1);
  assert.match(conflict.stderr, /already exists with different source or metadata/);
  assert.deepEqual(await fs.readFile(manifestPath), manifestBytes);

  await fs.writeFile(path.join(managed, ".import.lock"), "busy");
  const busy = await run({ ...baseSpec, id: "busy" }, 1);
  assert.match(busy.stderr, /another asset import is active/);
  assert.deepEqual(await fs.readFile(manifestPath), manifestBytes);
  await fs.rm(path.join(managed, ".import.lock"));

  const badUnits = await run({ ...baseSpec, id: "bad-units", units: "unknown" }, 1);
  assert.match(badUnits.stderr, /STL units must be explicitly mm or inch/);

  const missing = await run({ ...baseSpec, id: "missing", source: path.join(temporary, "missing.stl") }, 1);
  assert.match(missing.stderr, /source does not exist/);

  const malformedPath = path.join(temporary, "malformed.stl");
  await fs.writeFile(malformedPath, "bad");
  const malformed = await run({ ...baseSpec, id: "malformed", source: malformedPath }, 1);
  assert.match(malformed.stderr, /cannot parse STL/);
  assert.deepEqual(await fs.readFile(manifestPath), manifestBytes);

  const oversizePath = path.join(temporary, "oversize.stl");
  const oversize = await fs.open(oversizePath, "w");
  await oversize.truncate(256 * 1024 * 1024 + 1);
  await oversize.close();
  const tooLarge = await run({ ...baseSpec, id: "oversize", source: oversizePath }, 1);
  assert.match(tooLarge.stderr, /source exceeds/);

  const unsupportedPath = path.join(temporary, "part.3mf");
  await fs.writeFile(unsupportedPath, "3MF");
  const unsupported = await run({ ...baseSpec, id: "unsupported", source: unsupportedPath, format: "3mf" }, 1);
  assert.match(unsupported.stderr, /archives and 3MF are not supported/);

  const step = path.join(here, "node_modules/occt-import-js/test/testfiles/simple-basic-cube/cube.stp");
  const stepResult = JSON.parse((await run({
    id: "test-step-v1",
    name: "Test STEP cube",
    source: step,
    format: "step",
    units: "mm",
  })).stdout);
  assert.ok(stepResult.asset.mesh_count > 0);
  assert.ok(stepResult.asset.triangle_count > 0);
  assert.deepEqual(stepResult.asset.conversion, {
    engine: "occt-import-js",
    version: "0.0.23",
    geometry: "triangulated-display",
    linear_deflection_mm: 0.2,
    angular_deflection_rad: 0.35,
  });
  const reread = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  assert.deepEqual(reread.assets.map(({ id }) => id), ["test-triangle-v1", "test-step-v1"]);
  process.stdout.write("asset importer tests passed: STL, STEP, durability, idempotence, conflict, malformed, oversize, missing, invalid units, unsupported format\n");
} finally {
  await fs.rm(temporary, { recursive: true, force: true });
}
