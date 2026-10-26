#!/usr/bin/env python3
"""Turn a printer maker's STEP assembly into a Stella Scope printer model (offline tool, not product code).

Two steps, so motion groups are chosen from the real part list rather than guessed:

  list:  python step_to_glb.py list  SOURCE.step OUT_parts.json [split]
         every leaf part: its key (p00001...), name, assembly path and world bounds in mm. With "split", every solid
         of a leaf (a model flattened into a few compounds) is listed as a part of its own.
  glb:   python step_to_glb.py glb   SOURCE.step GROUPS.json OUT_model.glb
         a GLB in metres, one node per part, mesh extras {part_key, group}; the group comes from
         GROUPS.json {"rules":[{"group":"head","match":[...],"not":[...],"inside":[x0,y0,z0,x1,y1,z1]}]}
         (see rule_matches; first rule wins; anything unmatched is "fixed"). "split_solids": true in GROUPS.json draws every
         solid of a leaf as a part of its own (F09, 2026-10-19), for a model that arrives as a few flattened compounds.
         "orient": [[..],[..],[..]] gives the rows that turn CAD (x, y, z) into the display frame (X right, Y up, the machine's
         front toward +Z) for a STEP that is not Z-up; the default is [[1,0,0],[0,0,1],[0,-1,0]].

Run with FORMA's installed Python (OCP + cadgen): /usr/lib/FORMA/app-runtime/python/bin/python3 -I step_to_glb.py ... Deterministic for
one input: fixed tessellation, parts in assembly order, one core.
"""

from __future__ import annotations

import json
import re
import struct
import sys
from pathlib import Path

from OCP.Bnd import Bnd_Box
from OCP.BRep import BRep_Tool
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepMesh import BRepMesh_IncrementalMesh
from OCP.TopAbs import TopAbs_FACE, TopAbs_REVERSED, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS

from cadgen._internal.step_scene_loader import load_step_scene
from cadgen._internal.step_scene_mesh import scene_occurrence_shape

LINEAR_MM = 0.35
ANGULAR_RAD = 0.5


def part_name(node):
    """The name a part is listed under. A STEP that names its occurrences NAUO12 (a Fusion or Inventor export without
    instance names: the MINI assembly of Printables 307054) keeps the real part name in source_name; any other name is
    used as it always was, so a model already converted comes out the same."""
    name = (node.name or "").strip()
    if re.fullmatch(r"NAUO\d+", name) and node.source_name:
        return node.source_name.strip()
    return (name or node.source_name or "part").strip()


def leaves(scene):
    out = []

    def visit(node, path):
        name = part_name(node)
        here = [*path, name]
        if node.prototype_key is not None:
            out.append((node, here))
        for c in node.children:
            visit(c, here)

    for r in scene.roots:
        visit(r, [])
    return out


def bounds(shape):
    b = Bnd_Box()
    BRepBndLib.Add_s(shape, b)
    if b.IsVoid():
        return None
    x0, y0, z0, x1, y1, z1 = b.Get()
    return [round(v, 2) for v in (x0, y0, z0, x1, y1, z1)]


def bounds_of_faces(faces):
    b = Bnd_Box()
    for f in faces:
        BRepBndLib.Add_s(f, b)
    if b.IsVoid():
        return None
    return [round(v, 2) for v in b.Get()]


def pieces(shape, split):
    """The bodies a leaf is drawn as: the leaf itself, or (split) every solid it holds, then whatever surface is left over.
    F09 (2026-10-19): a community model flattened to two compounds (the MK4 space model of Printables 961911, written by
    CATIA) holds 49 solids that are the printer's parts; split they can be given to the head, the bed and the frame."""
    if not split:
        return [(None, shape, None)]
    out = []
    ex = TopExp_Explorer(shape, TopAbs_SOLID)
    while ex.More():
        out.append((len(out) + 1, ex.Current(), None))
        ex.Next()
    if not out:
        return [(None, shape, None)]
    rest = TopExp_Explorer(shape, TopAbs_FACE, TopAbs_SOLID)
    faces = []
    while rest.More():
        faces.append(rest.Current())
        rest.Next()
    if faces:
        out.append((len(out) + 1, None, faces))
    return out


def cmd_list(src, out, split=False):
    scene = load_step_scene(Path(src))
    rows = []
    for i, (node, path) in enumerate(leaves(scene), 1):
        shape = scene_occurrence_shape(scene, node)
        for k, solid, loose in pieces(shape, split):
            if k is None:
                rows.append({"key": f"p{i:05d}", "name": path[-1], "path": " / ".join(path), "bounds_mm": bounds(shape)})
            else:
                rows.append({"key": f"p{i:05d}s{k:02d}", "name": f"{path[-1]} solid {k}", "path": " / ".join(path) + f" / solid {k}",
                             "bounds_mm": bounds(solid) if solid is not None else bounds_of_faces(loose)})
    Path(out).write_text(json.dumps({"source": Path(src).name, "parts": rows}, indent=1))
    print(f"{len(rows)} parts -> {out}")


def rule_matches(rule, path, box):
    """A rule: `match` (any substring of the assembly path), `not` (none of these), `inside`
    ([x0,y0,z0,x1,y1,z1] mm: the part's bounds lie wholly inside). Every condition given must hold."""
    low = path.lower()
    if rule.get("match") and not any(m.lower() in low for m in rule["match"]):
        return False
    if any(m.lower() in low for m in rule.get("not", [])):
        return False
    inside = rule.get("inside")
    if inside:
        if box is None:
            return False
        if not all(box[i] >= inside[i] for i in range(3)) or not all(box[i] <= inside[i] for i in range(3, 6)):
            return False
    return True


def faces_of(shape):
    out = []
    ex = TopExp_Explorer(shape, TopAbs_FACE)
    while ex.More():
        out.append(ex.Current())
        ex.Next()
    return out


def tessellate(faces, orient=None):
    pos, idx = [], []
    for f in faces:
        face = TopoDS.Face_s(f)
        loc = TopLoc_Location()
        tri = BRep_Tool.Triangulation_s(face, loc)
        if tri is not None:
            t = loc.Transformation()
            rev = face.Orientation() == TopAbs_REVERSED
            base = len(pos) // 3
            for k in range(1, tri.NbNodes() + 1):
                p = tri.Node(k).Transformed(t)
                # glTF is Y-up: CAD (x, y, z) mm -> display (x, z, -y) m; the profile's basis
                # [[1,0,0],[0,0,1],[0,-1,0]] carries CAD motion offsets the same way. A STEP written Y-up (the MINI assembly of
                # Printables 307054) gives its own rows as "orient" in GROUPS.json (F09, 2026-10-19) and its profile the same as its basis.
                if orient is None:
                    pos.extend((p.X() / 1000.0, p.Z() / 1000.0, -p.Y() / 1000.0))
                else:
                    pos.extend(sum(r[c] * v for c, v in enumerate((p.X(), p.Y(), p.Z()))) / 1000.0 for r in orient)
            for k in range(1, tri.NbTriangles() + 1):
                a, b, c = tri.Triangle(k).Get()
                if rev:
                    b, c = c, b
                idx.extend((base + a - 1, base + b - 1, base + c - 1))
    return pos, idx


def cmd_glb(src, groups_file, out):
    spec = json.loads(Path(groups_file).read_text())
    rules, looks = spec["rules"], spec.get("looks", [])
    scene = load_step_scene(Path(src))
    meshed = set()
    bin_parts, views, accessors, meshes, nodes, materials, mat_index = [], [], [], [], [], [], {}
    offset = 0
    counts = {}
    split = bool(spec.get("split_solids"))
    orient = spec.get("orient")
    for i, (node, path) in enumerate(leaves(scene), 1):
        if node.prototype_key not in meshed:
            BRepMesh_IncrementalMesh(scene.prototype_shapes[node.prototype_key], LINEAR_MM, False, ANGULAR_RAD, False).Perform()
            meshed.add(node.prototype_key)
        shape = scene_occurrence_shape(scene, node)
        for k, solid, loose in pieces(shape, split):
            faces = faces_of(shape) if k is None else (faces_of(solid) if solid is not None else loose)
            pos, idx = tessellate(faces, orient)
            if not idx:
                continue
            joined = " / ".join(path) if k is None else " / ".join(path) + f" / solid {k}"
            box = bounds(shape) if k is None else (bounds(solid) if solid is not None else bounds_of_faces(loose))
            group = next((r["group"] for r in rules if rule_matches(r, joined, box)), "fixed")
            counts[group] = counts.get(group, 0) + 1
            color = node.color or scene.prototype_colors.get(node.prototype_key)
            look = next((r["look"] for r in looks if rule_matches(r, joined, box)), {})
            rgb = tuple(round(float(c), 3) for c in look.get("color") or (color[:3] if color is not None else (0.62, 0.64, 0.66)))
            alpha, metal, rough = float(look.get("alpha", 1.0)), float(look.get("metal", 0.15)), float(look.get("rough", 0.6))
            mkey = (rgb, alpha, metal, rough)
            if mkey not in mat_index:
                mat_index[mkey] = len(materials)
                m = {"pbrMetallicRoughness": {"baseColorFactor": [*rgb, alpha], "metallicFactor": metal, "roughnessFactor": rough}}
                if alpha < 0.999:
                    m.update({"alphaMode": "BLEND", "doubleSided": True})
                materials.append(m)
            pb = struct.pack(f"<{len(pos)}f", *pos)
            n = len(pos) // 3
            ib = struct.pack(f"<{len(idx)}I", *idx)
            xs, ys, zs = pos[0::3], pos[1::3], pos[2::3]
            for data, target in ((pb, 34962), (ib, 34963)):
                views.append({"buffer": 0, "byteOffset": offset, "byteLength": len(data), "target": target})
                bin_parts.append(data)
                offset += len(data)
                pad = (-offset) % 4
                if pad:
                    bin_parts.append(b"\0" * pad)
                    offset += pad
            accessors.append({"bufferView": len(views) - 2, "componentType": 5126, "count": n, "type": "VEC3",
                              "min": [min(xs), min(ys), min(zs)], "max": [max(xs), max(ys), max(zs)]})
            accessors.append({"bufferView": len(views) - 1, "componentType": 5125, "count": len(idx), "type": "SCALAR"})
            key = f"p{i:05d}" if k is None else f"p{i:05d}s{k:02d}"
            meshes.append({"name": path[-1] if k is None else f"{path[-1]} solid {k}", "primitives": [{"attributes": {"POSITION": len(accessors) - 2}, "indices": len(accessors) - 1,
                           "material": mat_index[mkey], "mode": 4}], "extras": {"part_key": key, "group": group, "path": joined}})
            nodes.append({"name": key, "mesh": len(meshes) - 1})
    blob = b"".join(bin_parts)
    gltf = {"asset": {"version": "2.0", "generator": "stella-scope tools/printer-cad/step_to_glb.py"},
            "scene": 0, "scenes": [{"nodes": list(range(len(nodes)))}], "nodes": nodes, "meshes": meshes,
            "materials": materials, "accessors": accessors, "bufferViews": views, "buffers": [{"byteLength": len(blob)}]}
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * ((-len(js)) % 4)
    with open(out, "wb") as f:
        f.write(struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(js) + 8 + len(blob)))
        f.write(struct.pack("<I4s", len(js), b"JSON") + js)
        f.write(struct.pack("<I4s", len(blob), b"BIN\0") + blob)
    print(f"{len(nodes)} parts, groups {counts}, {12 + 16 + len(js) + len(blob)} bytes -> {out}")


if __name__ == "__main__":
    if len(sys.argv) in (4, 5) and sys.argv[1] == "list":
        cmd_list(sys.argv[2], sys.argv[3], len(sys.argv) == 5 and sys.argv[4] == "split")
    elif len(sys.argv) == 5 and sys.argv[1] == "glb":
        cmd_glb(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
