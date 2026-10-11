# Stella Scope managed asset importer

Imports one STL or STEP source into the managed library beside a Stella Scope database:

```bash
node tools/asset-import/import.mjs \
  --database /absolute/path/stella.sqlite3 \
  --spec /absolute/path/asset.json
```

The spec keys are `id`, `name`, `source`, `format`, `units`, `status`, `slot_id`,
`parent_id`, `manufacturer`, `model`, `revision`, `source_url`, `license`, `notes`, and
`placement`. `id`, `name`, `source`, and `format` are required. STL `units` must be `mm`
or `inch`; STEP units come from the file and `units`, when supplied, must be `mm`.
`placement` defaults to identity with `basis: "unconfirmed"`; rotations are XYZ Euler
radians and source coordinates are Z-up.
`slot_id` accepts the existing dotted inventory IDs and is bounded to 128 safe characters.

The importer copies the original unchanged, writes triangulated display geometry separately,
then atomically replaces `<database>.assets/manifest.json`. Placement is baked into display
vertices. Node `instance_ids` are stable display selections, not recovered editable joints.
STEP display geometry uses an explicit 0.2 mm absolute linear deflection and 0.35 rad angular
deflection. Each new manifest record names the conversion engine/version and tessellation values;
the managed original remains the unchanged authoritative source.
Reusing an ID is allowed only when the complete persisted asset record is identical.
Archives and 3MF are intentionally unsupported in RP-01.
One import may update a managed library at a time; a concurrent import fails without changing
the manifest.

`occt-import-js@0.0.23` and its Open CASCADE component are pinned LGPL-2.1 dependencies.
Their upstream license texts remain in `node_modules/occt-import-js/`; using the importer does
not relicense those components or their notices. Preserve the applicable upstream license and
source-offer obligations when distributing the importer or its bundled WASM.

## Native large-assembly path

A full printer assembly (the Voron 2.4 350 master is about 240 MiB) exceeds the practical runtime of
the WebAssembly OCCT converter. Large STEP assemblies can use the native OCP/XCAF converter instead.
It needs a Python with Open CASCADE bindings (`pip install cadquery-ocp`); Stella Scope itself does
not depend on Python:

```bash
python3 tools/asset-import/native_step_import.py \
  --database /absolute/path/stella.sqlite3 \
  --spec /absolute/path/asset.json
```

This offline path preserves the XCAF occurrence hierarchy, names, colors, and world placements,
then writes the same managed manifest/original/geometry layout. It uses fixed 0.2 mm / 0.35 rad
display tessellation, one core for deterministic output, staged writes, the shared import lock,
and the same 256 MiB source / 128 MiB derivative caps. It records OCP/Open CASCADE provenance.
Each mesh receives a source-derived top-level layer and, where the hierarchy is unambiguous, a
source component slot ID. These mappings describe the reference assembly and do not assert that
the same revision is fitted to your printer. Motion remains unset until a physical datum is established.
The native derivative omits optional serialized normals to stay within the display payload cap;
the existing Three.js renderer computes vertex normals from the unchanged indexed surfaces.
Source bed dimensions remain derived from the named source plate; the printer's 350 mm target is
recorded separately and never used to scale reference geometry. Bed-center X/Y comes from that
plate while Z uses the named flex-sheet top when the source assembly provides one.
It does not add Python, OCP, or FORMA to the Rust product runtime.
