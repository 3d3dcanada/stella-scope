#!/usr/bin/env python3
"""Offline native-OCP STEP importer for assemblies too large for the WASM path."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time

import OCP
from OCP.BRep import BRep_Tool
from OCP.BRepMesh import BRepMesh_IncrementalMesh
from OCP.TopAbs import TopAbs_FACE, TopAbs_REVERSED
from OCP.TopExp import TopExp_Explorer
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS

try:
    from cadgen._internal.step_scene_loader import load_step_scene
    from cadgen._internal.step_scene_mesh import scene_occurrence_shape
except ImportError as error:
    raise SystemExit(
        "native STEP import needs the installed FORMA cadgen package on PYTHONPATH; "
        "see tools/asset-import/README.md"
    ) from error


MAX_SOURCE_BYTES = 256 * 1024 * 1024
MAX_GEOMETRY_BYTES = 128 * 1024 * 1024
ID = re.compile(r"^[a-z0-9_-]{1,64}$")
SLOT_ID = re.compile(r"^[a-z0-9_.-]{1,128}$")
STATUSES = {"installed", "planned", "suggested", "retired", "unverified"}
LINEAR_DEFLECTION_MM = 0.2
ANGULAR_DEFLECTION_RAD = 0.35


def fail(message: str) -> None:
    raise RuntimeError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked_text(value: object, key: str, limit: int, required: bool = False) -> str | None:
    if value is None and not required:
        return None
    if not isinstance(value, str) or (required and not value) or len(value) > limit or any(c in value for c in "\0\r\n"):
        fail(f"{key} must be {'a non-empty' if required else 'a'} single-line string of at most {limit} characters")
    return value


def triplet(value: object, key: str, fallback: list[float]) -> list[float]:
    result = fallback if value is None else value
    if not isinstance(result, list) or len(result) != 3 or any(not isinstance(number, (int, float)) or not math.isfinite(number) for number in result):
        fail(f"{key} must contain exactly three finite numbers")
    return [float(number) for number in result]


def normalize_spec(raw: object) -> dict[str, object]:
    if not isinstance(raw, dict):
        fail("spec must be a JSON object")
    asset_id = checked_text(raw.get("id"), "id", 64, True)
    if not ID.fullmatch(asset_id or ""):
        fail("id must match [a-z0-9_-]{1,64}")
    source = checked_text(raw.get("source"), "source", 4096, True)
    if not Path(source or "").is_absolute():
        fail("source must be an absolute path")
    if raw.get("format") != "step" or raw.get("units", "mm") != "mm":
        fail("native import accepts STEP normalized to mm only")
    status = raw.get("status", "unverified")
    if status not in STATUSES:
        fail("status must be installed, planned, suggested, retired, or unverified")
    slot_id = raw.get("slot_id")
    if slot_id is not None and (not isinstance(slot_id, str) or not SLOT_ID.fullmatch(slot_id)):
        fail("slot_id must match [a-z0-9_.-]{1,128}")
    parent_id = raw.get("parent_id")
    if parent_id is not None and (not isinstance(parent_id, str) or not ID.fullmatch(parent_id)):
        fail("parent_id must match [a-z0-9_-]{1,64}")
    placement_raw = raw.get("placement") or {}
    if not isinstance(placement_raw, dict):
        fail("placement must be an object")
    scale = placement_raw.get("scale", 1)
    if not isinstance(scale, (int, float)) or not math.isfinite(scale) or scale <= 0 or scale > 1_000_000:
        fail("placement.scale must be a finite positive number no greater than 1000000")
    placement = {
        "position": triplet(placement_raw.get("position"), "placement.position", [0, 0, 0]),
        "rotation": triplet(placement_raw.get("rotation"), "placement.rotation", [0, 0, 0]),
        "scale": float(scale),
        "basis": checked_text(placement_raw.get("basis", "unconfirmed"), "placement.basis", 512, True),
    }
    return {
        "id": asset_id,
        "name": checked_text(raw.get("name"), "name", 512, True),
        "source": source,
        "format": "step",
        "units": "mm",
        "status": status,
        "slot_id": slot_id,
        "parent_id": parent_id,
        "manufacturer": checked_text(raw.get("manufacturer"), "manufacturer", 512),
        "model": checked_text(raw.get("model"), "model", 512),
        "revision": checked_text(raw.get("revision"), "revision", 512),
        "source_url": checked_text(raw.get("source_url"), "source_url", 4096),
        "license": checked_text(raw.get("license"), "license", 512),
        "notes": checked_text(raw.get("notes"), "notes", 4096),
        "placement": placement,
    }


def placement_transform(placement: dict[str, object]):
    px, py, pz = placement["position"]
    rx, ry, rz = placement["rotation"]
    scale = placement["scale"]
    sx, cx = math.sin(rx), math.cos(rx)
    sy, cy = math.sin(ry), math.cos(ry)
    sz, cz = math.sin(rz), math.cos(rz)

    def rotate(x: float, y: float, z: float) -> tuple[float, float, float]:
        x, y, z = x, cx * y - sx * z, sx * y + cx * z
        x, y, z = cy * x + sy * z, y, -sy * x + cy * z
        return cz * x - sz * y, sz * x + cz * y, z

    def point(x: float, y: float, z: float) -> tuple[float, float, float]:
        x, y, z = rotate(x * scale, y * scale, z * scale)
        return x + px, y + py, z + pz

    return point, rotate


def rounded(number: float) -> float:
    if not math.isfinite(number):
        fail("geometry transform produced a non-finite coordinate")
    value = round(float(number), 5)
    return 0.0 if value == 0 else value


def clean_name(name: str | None, fallback: str) -> str:
    value = " ".join(str(name or fallback).split())[:512]
    return value or fallback


def source_mapping(ancestors: list[str]) -> dict[str, str]:
    names = [re.sub(r":\d+$", "", name) for name in ancestors if name and name != "(Unsaved)"]
    if not names:
        return {}
    component = names[1] if len(names) > 1 else names[0]
    layer = re.sub(r"[^a-z0-9]+", "_", component.lower()).strip("_")
    path = " / ".join(names).lower()
    slot_id = None
    if "bed components" in path:
        slot_id = "bed"
        if "mic6 plate" in path:
            slot_id = "bed.plate"
        elif "flex sheet" in path or "magnet sheet" in path:
            slot_id = "bed.sheet"
        elif "bed heater" in path:
            slot_id = "bed.heater"
    elif "toolheads" in path:
        slot_id = "head"
        for phrase, candidate in (
            ("nozzle", "head.nozzle"),
            ("heat block", "head.block"),
            ("heatbreak", "head.heatbreak"),
            ("heatsink", "head.heatsink"),
            ("cartridge heater", "head.cartridge"),
        ):
            if phrase in path:
                slot_id = candidate
                break
    elif "x carriage" in path:
        slot_id = "head.carriage"
    elif "x axis" in path:
        slot_id = "x_rail"
    elif "a drive" in path:
        slot_id = "drive_a"
    elif "b drive" in path:
        slot_id = "drive_b"
    elif "gantry" in path:
        slot_id = "gantry"
    elif "z assembly" in path:
        slot_id = "z_drive"
    elif "bed supports" in path:
        slot_id = "bed.supports"
    elif "frame" in path:
        slot_id = "frame"
    elif "panels" in path:
        layer = "covers"
    elif "electronics" in path:
        slot_id = "electronics"
    elif "skirt" in path:
        slot_id = "enclosure.skirt"
    mapping = {"layer": layer or "source_component"}
    if slot_id:
        mapping["slot_id"] = slot_id
    return mapping


def merge_bounds(bounds: list[float] | None, point: tuple[float, float, float]) -> list[float]:
    if bounds is None:
        return [point[0], point[1], point[2], point[0], point[1], point[2]]
    bounds[0] = min(bounds[0], point[0]); bounds[1] = min(bounds[1], point[1]); bounds[2] = min(bounds[2], point[2])
    bounds[3] = max(bounds[3], point[0]); bounds[4] = max(bounds[4], point[1]); bounds[5] = max(bounds[5], point[2])
    return bounds


class GeometryWriter:
    def __init__(self, path: Path):
        self.path = path
        self.file = path.open("w", encoding="ascii", newline="\n")
        self.bytes = 0

    def write(self, text: str) -> None:
        encoded = text.encode("ascii")
        if self.bytes + len(encoded) > MAX_GEOMETRY_BYTES:
            fail(f"derived geometry exceeds the {MAX_GEOMETRY_BYTES}-byte limit")
        self.file.write(text)
        self.bytes += len(encoded)

    def value(self, value: object) -> None:
        self.write(json.dumps(value, ensure_ascii=True, allow_nan=False, separators=(",", ":")))

    def close(self) -> None:
        self.file.close()


def mesh_occurrence(scene, node, point_transform) -> tuple[dict[str, object], list[float], int]:
    shape = scene_occurrence_shape(scene, node)
    positions: list[float] = []
    indices: list[int] = []
    bounds: list[float] | None = None
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    while explorer.More():
        face = TopoDS.Face_s(explorer.Current())
        location = TopLoc_Location()
        triangulation = BRep_Tool.Triangulation_s(face, location)
        if triangulation is not None:
            transform = location.Transformation()
            reversed_face = face.Orientation() == TopAbs_REVERSED
            offset = len(positions) // 3
            for index in range(1, triangulation.NbNodes() + 1):
                cad_point = triangulation.Node(index).Transformed(transform)
                point = point_transform(cad_point.X(), cad_point.Y(), cad_point.Z())
                bounds = merge_bounds(bounds, point)
                positions.extend(rounded(value) for value in point)
            for index in range(1, triangulation.NbTriangles() + 1):
                a, b, c = triangulation.Triangle(index).Get()
                if reversed_face:
                    b, c = c, b
                indices.extend((offset + a - 1, offset + b - 1, offset + c - 1))
        explorer.Next()
    if not positions or not indices:
        fail(f"native tessellation produced empty or inconsistent geometry for {node.name or node.path}")
    color = node.color or scene.prototype_colors.get(node.prototype_key)
    mesh: dict[str, object] = {
        "name": clean_name(node.name or node.source_name, "STEP component"),
        "attributes": {"position": {"array": positions}},
        "index": {"array": indices},
    }
    if color is not None:
        mesh["color"] = [rounded(float(value)) for value in color[:3]]
    return mesh, bounds or [0.0] * 6, len(indices) // 3


def build_hierarchy(roots, asset_id: str):
    leaves: list[tuple[object, list[str], int]] = []

    def visit(node, ancestors: list[str]) -> dict[str, object]:
        name = clean_name(node.name or node.source_name, "STEP node")
        current_ancestors = [*ancestors, name]
        meshes: list[int] = []
        instance_ids: list[str] = []
        if node.prototype_key is not None:
            mesh_index = len(leaves)
            leaves.append((node, current_ancestors, mesh_index))
            meshes.append(mesh_index)
            hierarchy_path = "root/" + "/".join(str(max(0, part - 1)) for part in node.path)
            instance_ids.append(f"{asset_id}:{hierarchy_path}:{mesh_index}")
        return {
            "name": name,
            "meshes": meshes,
            "instance_ids": instance_ids,
            "children": [visit(child, current_ancestors) for child in node.children],
        }

    root_nodes = [visit(root, []) for root in roots]
    root = root_nodes[0] if len(root_nodes) == 1 else {"name": "STEP assembly", "meshes": [], "instance_ids": [], "children": root_nodes}
    return root, leaves


def create_geometry(source: Path, output: Path, asset_id: str, placement: dict[str, object]) -> dict[str, object]:
    started = time.monotonic()
    print("native-step phase=load", flush=True)
    scene = load_step_scene(source)
    print(
        f"native-step phase=loaded seconds={time.monotonic()-started:.3f} roots={len(scene.roots)} prototypes={len(scene.prototype_shapes)}",
        flush=True,
    )
    root, leaves = build_hierarchy(scene.roots, asset_id)
    point_transform, _normal_transform = placement_transform(placement)
    writer = GeometryWriter(output)
    overall_bounds: list[float] | None = None
    bed_bounds: list[float] | None = None
    surface_bounds: list[float] | None = None
    group_bounds: dict[str, list[float]] = {}
    meshed: set[int] = set()
    triangle_count = 0
    try:
        writer.write("{\"root\":"); writer.value(root); writer.write(",\"meshes\":[")
        for occurrence_index, (node, ancestors, mesh_index) in enumerate(leaves):
            if node.prototype_key not in meshed:
                BRepMesh_IncrementalMesh(
                    scene.prototype_shapes[node.prototype_key],
                    LINEAR_DEFLECTION_MM,
                    False,
                    ANGULAR_DEFLECTION_RAD,
                    False,
                ).Perform()
                meshed.add(node.prototype_key)
            mesh, bounds, triangles = mesh_occurrence(scene, node, point_transform)
            mapping = source_mapping(ancestors)
            layer = mapping.get("layer")
            mesh["mapping"] = mapping
            if mesh_index:
                writer.write(",")
            writer.value(mesh)
            triangle_count += triangles
            for point in ((bounds[0], bounds[1], bounds[2]), (bounds[3], bounds[4], bounds[5])):
                overall_bounds = merge_bounds(overall_bounds, point)
                if any("10in MIC6 Plate" in name for name in ancestors):
                    bed_bounds = merge_bounds(bed_bounds, point)
                if any("Flex Sheet" in name for name in ancestors):
                    surface_bounds = merge_bounds(surface_bounds, point)
                if layer:
                    group_bounds[layer] = merge_bounds(group_bounds.get(layer), point)
            if (occurrence_index + 1) % 100 == 0:
                print(
                    f"native-step phase=mesh occurrences={occurrence_index+1}/{len(leaves)} triangles={triangle_count} bytes={writer.bytes}",
                    flush=True,
                )
        if overall_bounds is None:
            fail("native tessellation produced no assembly bounds")
        bed_center = None
        source_bed_size = None
        if bed_bounds is not None:
            surface_top = surface_bounds[5] if surface_bounds is not None else bed_bounds[5]
            bed_center = [rounded((bed_bounds[0] + bed_bounds[3]) / 2), rounded((bed_bounds[1] + bed_bounds[4]) / 2), rounded(surface_top)]
            source_bed_size = [rounded(bed_bounds[3] - bed_bounds[0]), rounded(bed_bounds[4] - bed_bounds[1])]
        printer_frame = {
            "bed_center": bed_center,
            "bed_size": source_bed_size,
            "target_machine_bed_size": [350, 350],
            "source_pose": None,
            "basis": (
                "Z-up millimeter STEP coordinates with source placements baked into every mesh. "
                "bed_center X/Y and bed_size are derived from the named source MIC6 plate; bed_center "
                "Z is the top of the named Flex Sheet when present, otherwise the plate top. target_machine_bed_size "
                "records the printer's requested 350 mm target separately; no scaling was applied. No "
                "source-backed nozzle datum or motion classification was recovered."
            ),
        }
        writer.write("],\"printer_frame\":"); writer.value(printer_frame); writer.write("}\n")
    finally:
        writer.close()
    return {
        "mesh_count": len(leaves),
        "triangle_count": triangle_count,
        "bounds": [rounded(value) for value in overall_bounds],
        "bed_bounds": None if bed_bounds is None else [rounded(value) for value in bed_bounds],
        "surface_bounds": None if surface_bounds is None else [rounded(value) for value in surface_bounds],
        "group_bounds": {name: [rounded(value) for value in bounds] for name, bounds in group_bounds.items()},
        "hierarchy_nodes": sum(1 for _ in walk_nodes(scene.roots)),
        "prototype_count": len(scene.prototype_shapes),
        "elapsed_seconds": round(time.monotonic() - started, 3),
    }


def walk_nodes(nodes):
    stack = list(reversed(nodes))
    while stack:
        node = stack.pop(); yield node
        stack.extend(reversed(node.children))


def read_manifest(path: Path) -> dict[str, object]:
    if not path.exists():
        return {"schema": "stella-scope.assets.v1", "assets": []}
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 8 * 1024 * 1024:
        fail("managed manifest must be a bounded regular file")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "stella-scope.assets.v1" or not isinstance(manifest.get("assets"), list):
        fail("managed manifest has an unsupported schema")
    return manifest


def import_step(database: Path, raw_spec: object) -> dict[str, object]:
    if not database.is_absolute():
        fail("--database must be an absolute path")
    spec = normalize_spec(raw_spec)
    source = Path(spec["source"])
    if not source.exists() or source.is_symlink() or not source.is_file():
        fail("source must be an existing regular file, not a symlink")
    source_bytes = source.stat().st_size
    if not 0 < source_bytes <= MAX_SOURCE_BYTES:
        fail(f"source must contain 1..{MAX_SOURCE_BYTES} bytes")
    library = Path(f"{database}.assets")
    if library.exists() and (library.is_symlink() or not library.is_dir()):
        fail("managed library path must be a directory, not a symlink or file")
    library.mkdir(parents=True, exist_ok=True)
    lock_path = library / ".import.lock"
    try:
        lock_fd = os.open(lock_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        fail(f"another asset import is active for {library}; inspect the stale .import.lock")
    os.write(lock_fd, f"pid={os.getpid()}\n".encode("ascii")); os.close(lock_fd)
    stage: Path | None = None
    manifest_temp: Path | None = None
    final_directory = library / str(spec["id"])
    try:
        manifest_path = library / "manifest.json"
        manifest = read_manifest(manifest_path)
        source_hash = sha256_file(source)
        existing = next((asset for asset in manifest["assets"] if asset.get("id") == spec["id"]), None)
        if existing is not None:
            comparable = {key: existing.get(key) for key in spec}
            if comparable != spec or existing.get("source_sha256") != source_hash:
                fail(f"asset id {spec['id']} already exists with different source or metadata")
            for filename, expected in (("geometry.json", existing.get("derived_sha256")), ("original.step", source_hash)):
                managed = final_directory / filename
                if managed.is_symlink() or not managed.is_file() or sha256_file(managed) != expected:
                    fail(f"managed asset {spec['id']} has a missing or mismatched {filename}")
            return {"asset": existing, "idempotent": True}
        if final_directory.exists():
            fail(f"managed directory already exists for {spec['id']}; inspect the orphan before retrying")
        stage = Path(tempfile.mkdtemp(prefix=f".stage-{spec['id']}-", dir=library))
        geometry_path = stage / "geometry.json"
        summary = create_geometry(source, geometry_path, str(spec["id"]), spec["placement"])
        derived_bytes = geometry_path.stat().st_size
        if not 0 < derived_bytes <= MAX_GEOMETRY_BYTES:
            fail(f"derived geometry must contain 1..{MAX_GEOMETRY_BYTES} bytes")
        shutil.copyfile(source, stage / "original.step")
        if sha256_file(stage / "original.step") != source_hash:
            fail("managed source copy hash differs from the original")
        asset = {
            **spec,
            "conversion": {
                "engine": "OCP/Open CASCADE native XCAF",
                "version": OCP.__version__,
                "geometry": "triangulated-display",
                "linear_deflection_mm": LINEAR_DEFLECTION_MM,
                "angular_deflection_rad": ANGULAR_DEFLECTION_RAD,
                "parallel": False,
            },
            "source_sha256": source_hash,
            "derived_sha256": sha256_file(geometry_path),
            "source_name": source.name,
            "source_bytes": source_bytes,
            "derived_bytes": derived_bytes,
            "imported_at": datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
            "mesh_count": summary["mesh_count"],
            "triangle_count": summary["triangle_count"],
            "storage": "managed-copy",
            "source_path": str(source),
            "geometry_url": f"/api/assets/{spec['id']}/geometry",
            "original_url": f"/api/assets/{spec['id']}/original",
            "native_summary": summary,
        }
        next_manifest = {"schema": "stella-scope.assets.v1", "assets": [*manifest["assets"], asset]}
        manifest_fd, manifest_name = tempfile.mkstemp(prefix=".manifest-", suffix=".tmp", dir=library)
        manifest_temp = Path(manifest_name)
        with os.fdopen(manifest_fd, "w", encoding="utf-8") as output:
            json.dump(next_manifest, output, ensure_ascii=False, indent=2, allow_nan=False); output.write("\n")
        stage.rename(final_directory); stage = None
        try:
            os.replace(manifest_temp, manifest_path); manifest_temp = None
        except Exception:
            shutil.rmtree(final_directory, ignore_errors=True)
            raise
        return {"asset": asset, "idempotent": False}
    finally:
        if stage is not None:
            shutil.rmtree(stage, ignore_errors=True)
        if manifest_temp is not None:
            manifest_temp.unlink(missing_ok=True)
        lock_path.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--spec", type=Path, required=True)
    args = parser.parse_args()
    if not args.spec.is_absolute():
        fail("--spec must be an absolute path")
    result = import_step(args.database, json.loads(args.spec.read_text(encoding="utf-8")))
    print(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"native asset import failed: {error}", file=sys.stderr)
        raise SystemExit(1)
