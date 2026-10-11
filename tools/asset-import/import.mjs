#!/usr/bin/env node

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { createRequire } from "node:module";
import { Euler, Matrix3, Matrix4, Quaternion, Vector3 } from "three";
import { STLLoader } from "three/addons/loaders/STLLoader.js";

const require = createRequire(import.meta.url);
const MAX_SOURCE_BYTES = 256 * 1024 * 1024;
const MAX_GEOMETRY_BYTES = 128 * 1024 * 1024;
const ID = /^[a-z0-9_-]{1,64}$/;
const STATUSES = new Set(["installed", "planned", "suggested", "retired", "unverified"]);
const SPEC_KEYS = new Set([
  "id", "name", "source", "format", "units", "status", "slot_id", "parent_id",
  "manufacturer", "model", "revision", "source_url", "license", "notes", "placement",
]);

function fail(message) {
  throw new Error(message);
}

function text(value, key, limit, required = false) {
  if (value == null && !required) return null;
  if (typeof value !== "string" || (required && value.length === 0) || value.length > limit || /[\0\r\n]/.test(value)) {
    fail(`${key} must be ${required ? "a non-empty" : "a"} single-line string of at most ${limit} characters`);
  }
  return value;
}

function finiteTriplet(value, key, fallback) {
  const out = value ?? fallback;
  if (!Array.isArray(out) || out.length !== 3 || out.some((number) => typeof number !== "number" || !Number.isFinite(number))) {
    fail(`${key} must contain exactly three finite numbers`);
  }
  return out;
}

function normalizeSpec(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) fail("spec must be a JSON object");
  const unknown = Object.keys(input).filter((key) => !SPEC_KEYS.has(key));
  if (unknown.length) fail(`unsupported spec key: ${unknown.join(", ")}`);

  const id = text(input.id, "id", 64, true);
  if (!ID.test(id)) fail("id must match [a-z0-9_-]{1,64}");
  const format = text(input.format, "format", 8, true).toLowerCase();
  if (!new Set(["stl", "step"]).has(format)) fail("format must be stl or step; archives and 3MF are not supported in RP-01");
  let units = input.units == null ? null : text(input.units, "units", 8, true).toLowerCase();
  if (format === "stl" && !new Set(["mm", "inch"]).has(units)) fail("STL units must be explicitly mm or inch");
  if (format === "step") {
    units ??= "mm";
    if (units !== "mm") fail("STEP output is normalized to mm; units must be mm when supplied");
  }
  const status = input.status ?? "unverified";
  if (!STATUSES.has(status)) fail("status must be installed, planned, suggested, retired, or unverified");
  const source = text(input.source, "source", 4096, true);
  if (!path.isAbsolute(source)) fail("source must be an absolute path");

  const placementInput = input.placement ?? {};
  if (!placementInput || typeof placementInput !== "object" || Array.isArray(placementInput)) fail("placement must be an object");
  const placementUnknown = Object.keys(placementInput).filter((key) => !new Set(["position", "rotation", "scale", "basis"]).has(key));
  if (placementUnknown.length) fail(`unsupported placement key: ${placementUnknown.join(", ")}`);
  const scale = placementInput.scale ?? 1;
  if (typeof scale !== "number" || !Number.isFinite(scale) || scale <= 0 || scale > 1_000_000) fail("placement.scale must be a finite positive number no greater than 1000000");
  const placement = {
    position: finiteTriplet(placementInput.position, "placement.position", [0, 0, 0]),
    rotation: finiteTriplet(placementInput.rotation, "placement.rotation", [0, 0, 0]),
    scale,
    basis: text(placementInput.basis ?? "unconfirmed", "placement.basis", 512, true),
  };
  if (placement.position.some((number) => Math.abs(number) > 1_000_000_000)) fail("placement.position values must be within 1000000000 mm");

  const referenceId = (value, key) => {
    if (value == null) return null;
    const normalized = text(value, key, 64, true);
    if (!ID.test(normalized)) fail(`${key} must match [a-z0-9_-]{1,64}`);
    return normalized;
  };
  const slotId = (value) => {
    if (value == null) return null;
    const normalized = text(value, "slot_id", 128, true);
    if (!/^[a-z0-9_.-]{1,128}$/.test(normalized)) fail("slot_id must match [a-z0-9_.-]{1,128}");
    return normalized;
  };
  return {
    id,
    name: text(input.name, "name", 512, true),
    source,
    format,
    units,
    status,
    slot_id: slotId(input.slot_id),
    parent_id: referenceId(input.parent_id, "parent_id"),
    manufacturer: text(input.manufacturer, "manufacturer", 512),
    model: text(input.model, "model", 512),
    revision: text(input.revision, "revision", 512),
    source_url: text(input.source_url, "source_url", 4096),
    license: text(input.license, "license", 512),
    notes: text(input.notes, "notes", 4096),
    placement,
  };
}

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function cleanNode(node, assetId, hierarchyPath, meshCount) {
  if (!node || typeof node !== "object") fail("imported hierarchy contains an invalid node");
  const name = typeof node.name === "string" ? node.name.slice(0, 512) : "";
  if (!Array.isArray(node.meshes) || !Array.isArray(node.children)) fail("imported hierarchy node lacks meshes or children");
  const meshes = node.meshes.map((index) => {
    if (!Number.isInteger(index) || index < 0 || index >= meshCount) fail("imported hierarchy references an invalid mesh index");
    return index;
  });
  return {
    name,
    meshes,
    instance_ids: meshes.map((meshIndex) => `${assetId}:${hierarchyPath}:${meshIndex}`),
    children: node.children.map((child, index) => cleanNode(child, assetId, `${hierarchyPath}/${index}`, meshCount)),
  };
}

function placementMatrices(placement, unitScale) {
  const rotation = new Euler(...placement.rotation, "XYZ");
  const matrix = new Matrix4().compose(
    new Vector3(...placement.position),
    new Quaternion().setFromEuler(rotation),
    new Vector3(placement.scale * unitScale, placement.scale * unitScale, placement.scale * unitScale),
  );
  return { matrix, normal: new Matrix3().getNormalMatrix(matrix) };
}

function transformArray(array, matrix, normal = false) {
  if (!Array.isArray(array) && !ArrayBuffer.isView(array)) fail("geometry attribute array is invalid");
  if (array.length === 0 || array.length % 3 !== 0) fail("geometry attributes must contain non-empty number triplets");
  const out = new Array(array.length);
  const vector = new Vector3();
  for (let index = 0; index < array.length; index += 3) {
    vector.set(Number(array[index]), Number(array[index + 1]), Number(array[index + 2]));
    if (![vector.x, vector.y, vector.z].every(Number.isFinite)) fail("geometry contains a non-finite coordinate");
    if (normal) vector.applyNormalMatrix(matrix);
    else vector.applyMatrix4(matrix);
    if (![vector.x, vector.y, vector.z].every(Number.isFinite)) fail("geometry transform produced a non-finite coordinate");
    out[index] = vector.x;
    out[index + 1] = vector.y;
    out[index + 2] = vector.z;
  }
  return out;
}

function cleanMesh(mesh, matrices) {
  const positions = transformArray(mesh?.attributes?.position?.array, matrices.matrix);
  const vertexCount = positions.length / 3;
  const rawIndex = mesh?.index?.array;
  if ((!Array.isArray(rawIndex) && !ArrayBuffer.isView(rawIndex)) || rawIndex.length === 0 || rawIndex.length % 3 !== 0) {
    fail("geometry indices must contain non-empty triangles");
  }
  const index = Array.from(rawIndex, (value) => {
    if (!Number.isInteger(value) || value < 0 || value >= vertexCount) fail("geometry contains an out-of-range triangle index");
    return value;
  });
  const attributes = { position: { array: positions } };
  if (mesh.attributes.normal?.array) {
    const normals = transformArray(mesh.attributes.normal.array, matrices.normal, true);
    if (normals.length !== positions.length) fail("geometry normal count does not match its position count");
    attributes.normal = { array: normals };
  }
  const result = { name: typeof mesh.name === "string" ? mesh.name.slice(0, 512) : "", attributes, index: { array: index } };
  if (mesh.color != null) {
    if (!Array.isArray(mesh.color) || mesh.color.length !== 3 || mesh.color.some((value) => typeof value !== "number" || !Number.isFinite(value))) fail("geometry mesh color is invalid");
    result.color = mesh.color;
  }
  return result;
}

async function convert(sourceBytes, spec) {
  const unitScale = spec.format === "stl" && spec.units === "inch" ? 25.4 : 1;
  const matrices = placementMatrices(spec.placement, unitScale);
  if (spec.format === "stl") {
    let parsed;
    try {
      const bytes = sourceBytes.buffer.slice(sourceBytes.byteOffset, sourceBytes.byteOffset + sourceBytes.byteLength);
      parsed = new STLLoader().parse(bytes);
    } catch (error) {
      fail(`cannot parse STL: ${error.message}`);
    }
    const position = parsed.getAttribute("position");
    if (!position) fail("STL contains no triangles");
    if (!parsed.getAttribute("normal")) parsed.computeVertexNormals();
    const vertexCount = position.count;
    const mesh = {
      name: path.basename(spec.source),
      attributes: {
        position: { array: position.array },
        normal: { array: parsed.getAttribute("normal").array },
      },
      index: { array: Array.from({ length: vertexCount }, (_, index) => index) },
    };
    return {
      root: cleanNode({ name: spec.name, meshes: [0], children: [] }, spec.id, "root", 1),
      meshes: [cleanMesh(mesh, matrices)],
    };
  }

  const occt = await require("occt-import-js")();
  const converted = occt.ReadStepFile(sourceBytes, {
    linearUnit: "millimeter",
    linearDeflectionType: "absolute_value",
    linearDeflection: 0.2,
    angularDeflection: 0.35,
  });
  if (!converted?.success) fail("cannot parse STEP: OpenCascade import failed");
  if (!Array.isArray(converted.meshes) || converted.meshes.length === 0) fail("STEP contains no display geometry");
  return {
    root: cleanNode(converted.root, spec.id, "root", converted.meshes.length),
    meshes: converted.meshes.map((mesh) => cleanMesh(mesh, matrices)),
  };
}

async function exists(pathname) {
  try {
    return await fs.lstat(pathname);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

async function readManifest(manifestPath) {
  const stat = await exists(manifestPath);
  if (!stat) return { schema: "stella-scope.assets.v1", assets: [] };
  if (stat.isSymbolicLink() || !stat.isFile()) fail("managed manifest must be a regular file, not a symlink or directory");
  let manifest;
  try {
    manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  } catch (error) {
    fail(`cannot read managed manifest: ${error.message}`);
  }
  if (manifest?.schema !== "stella-scope.assets.v1" || !Array.isArray(manifest.assets)) fail("managed manifest has an unsupported schema");
  return manifest;
}

function stableInput(asset) {
  const { conversion, imported_at, derived_sha256, derived_bytes, mesh_count, triangle_count, geometry_url, original_url, storage, source_name, source_bytes, source_sha256, source_path, ...spec } = asset;
  return { ...spec, source_sha256 };
}

async function verifyExisting(asset, library) {
  const directory = path.join(library, asset.id);
  const original = path.join(directory, `original.${asset.format}`);
  const geometry = path.join(directory, "geometry.json");
  for (const [file, expected] of [[original, asset.source_sha256], [geometry, asset.derived_sha256]]) {
    const stat = await exists(file);
    if (!stat || stat.isSymbolicLink() || !stat.isFile()) fail(`managed asset ${asset.id} is missing ${path.basename(file)}`);
    if (sha256(await fs.readFile(file)) !== expected) fail(`managed asset ${asset.id} has a hash mismatch in ${path.basename(file)}`);
  }
}

export async function importAsset(database, rawSpec) {
  if (!path.isAbsolute(database)) fail("--database must be an absolute path");
  const spec = normalizeSpec(rawSpec);
  const sourceStat = await exists(spec.source);
  if (!sourceStat) fail(`source does not exist: ${spec.source}`);
  if (sourceStat.isSymbolicLink() || !sourceStat.isFile()) fail("source must be a regular file, not a symlink or directory");
  if (sourceStat.size > MAX_SOURCE_BYTES) fail(`source exceeds the ${MAX_SOURCE_BYTES}-byte limit`);
  if (sourceStat.size === 0) fail("source is empty");
  const extension = path.extname(spec.source).toLowerCase();
  const allowedExtensions = spec.format === "step" ? new Set([".step", ".stp"]) : new Set([".stl"]);
  if (!allowedExtensions.has(extension)) fail(`source extension does not match format ${spec.format}; archives and 3MF are not supported in RP-01`);

  const library = `${database}.assets`;
  const libraryStat = await exists(library);
  if (libraryStat?.isSymbolicLink() || (libraryStat && !libraryStat.isDirectory())) fail("managed library path must be a directory, not a symlink or file");
  await fs.mkdir(library, { recursive: true });
  const lockPath = path.join(library, ".import.lock");
  let lock;
  try {
    lock = await fs.open(lockPath, "wx");
  } catch (error) {
    if (error.code === "EEXIST") fail(`another asset import is active for ${library}; wait for it to finish or inspect the stale .import.lock`);
    throw error;
  }
  try {
    const manifestPath = path.join(library, "manifest.json");
    const manifest = await readManifest(manifestPath);
    const sourceBytes = await fs.readFile(spec.source);
    assert.equal(sourceBytes.length, sourceStat.size, "source changed size while it was being read");
    const sourceHash = sha256(sourceBytes);
    const existing = manifest.assets.find((asset) => asset.id === spec.id);
    if (existing) {
      if (JSON.stringify(stableInput(existing)) !== JSON.stringify({ ...spec, source_sha256: sourceHash })) fail(`asset id ${spec.id} already exists with different source or metadata; use a new immutable revision id`);
      await verifyExisting(existing, library);
      return { asset: existing, idempotent: true };
    }

    const geometry = await convert(sourceBytes, spec);
    if (!geometry.meshes.length) fail("converted geometry is empty");
    const geometryBytes = Buffer.from(`${JSON.stringify(geometry)}\n`);
    if (geometryBytes.length > MAX_GEOMETRY_BYTES) fail(`derived geometry exceeds the ${MAX_GEOMETRY_BYTES}-byte limit`);
    const triangleCount = geometry.meshes.reduce((sum, mesh) => sum + mesh.index.array.length / 3, 0);
    if (triangleCount === 0) fail("converted geometry contains no triangles");

    const originalName = `original.${spec.format}`;
    const stage = path.join(library, `.stage-${spec.id}-${process.pid}-${crypto.randomBytes(6).toString("hex")}`);
    const finalDirectory = path.join(library, spec.id);
    const manifestTemp = path.join(library, `.manifest-${process.pid}-${crypto.randomBytes(6).toString("hex")}.tmp`);
    if (await exists(finalDirectory)) fail(`managed directory already exists for ${spec.id}; inspect the orphan before retrying`);
    let moved = false;
    try {
      await fs.mkdir(stage);
      await fs.writeFile(path.join(stage, originalName), sourceBytes, { flag: "wx" });
      await fs.writeFile(path.join(stage, "geometry.json"), geometryBytes, { flag: "wx" });
      const copiedBytes = await fs.readFile(path.join(stage, originalName));
      if (sha256(copiedBytes) !== sourceHash) fail("managed source copy hash differs from the original");

      const asset = {
        ...spec,
        conversion: spec.format === "step" ? {
          engine: "occt-import-js",
          version: "0.0.23",
          geometry: "triangulated-display",
          linear_deflection_mm: 0.2,
          angular_deflection_rad: 0.35,
        } : {
          engine: "three-stlloader",
          version: "0.186.0",
          geometry: "triangulated-display",
        },
        source_sha256: sourceHash,
        derived_sha256: sha256(geometryBytes),
        source_name: path.basename(spec.source),
        source_bytes: sourceBytes.length,
        derived_bytes: geometryBytes.length,
        imported_at: new Date().toISOString(),
        mesh_count: geometry.meshes.length,
        triangle_count: triangleCount,
        storage: "managed-copy",
        source_path: spec.source,
        geometry_url: `/api/assets/${spec.id}/geometry`,
        original_url: `/api/assets/${spec.id}/original`,
      };
      const next = { schema: "stella-scope.assets.v1", assets: [...manifest.assets, asset] };
      await fs.writeFile(manifestTemp, `${JSON.stringify(next, null, 2)}\n`, { flag: "wx" });
      await fs.rename(stage, finalDirectory);
      moved = true;
      await fs.rename(manifestTemp, manifestPath);
      return { asset, idempotent: false };
    } catch (error) {
      await fs.rm(moved ? finalDirectory : stage, { recursive: true, force: true });
      await fs.rm(manifestTemp, { force: true });
      throw error;
    }
  } finally {
    await lock.close();
    await fs.rm(lockPath, { force: true });
  }
}

function argumentsFrom(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!new Set(["--database", "--spec"]).has(flag) || value == null) fail("usage: node import.mjs --database /absolute/file.sqlite3 --spec /absolute/spec.json");
    if (values[flag]) fail(`${flag} may be supplied only once`);
    values[flag] = value;
  }
  if (!values["--database"] || !values["--spec"]) fail("usage: node import.mjs --database /absolute/file.sqlite3 --spec /absolute/spec.json");
  if (!path.isAbsolute(values["--spec"])) fail("--spec must be an absolute path");
  return values;
}

async function main() {
  const args = argumentsFrom(process.argv.slice(2));
  let spec;
  try {
    spec = JSON.parse(await fs.readFile(args["--spec"], "utf8"));
  } catch (error) {
    fail(`cannot read spec JSON: ${error.message}`);
  }
  const result = await importAsset(args["--database"], spec);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (path.resolve(process.argv[1] ?? "") === path.resolve(new URL(import.meta.url).pathname)) {
  main().catch((error) => {
    process.stderr.write(`asset import failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
