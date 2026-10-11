#!/usr/bin/env python3

import hashlib
import json
from pathlib import Path
import shutil
import tempfile

from native_step_import import import_step


temporary = Path(tempfile.mkdtemp(prefix="stella-native-step-test-"))
try:
    import os, sys
    source = Path(os.environ.get("STEP_TEST_FILE", ""))
    if not source.is_file():
        print("skipped: set STEP_TEST_FILE to a small STEP assembly to run this test")
        sys.exit(0)
    database = temporary / "test.sqlite3"
    spec = {
        "id": "native-a-drive-test",
        "name": "Native A drive test",
        "source": str(source),
        "format": "step",
        "units": "mm",
        "status": "unverified",
        "slot_id": "drive_a.upper",
        "placement": {"position": [0, 0, 0], "rotation": [0, 0, 0], "scale": 1, "basis": "test"},
    }
    first = import_step(database, spec)
    assert first["idempotent"] is False
    assert first["asset"]["conversion"]["engine"] == "OCP/Open CASCADE native XCAF"
    assert first["asset"]["source_sha256"] == hashlib.sha256(source.read_bytes()).hexdigest()
    geometry_path = Path(f"{database}.assets/native-a-drive-test/geometry.json")
    geometry = json.loads(geometry_path.read_text(encoding="utf-8"))
    assert geometry["root"]["children"]
    assert len(geometry["meshes"]) == first["asset"]["mesh_count"] > 0
    assert "normal" not in geometry["meshes"][0]["attributes"]
    assert all(mesh["mapping"]["layer"] for mesh in geometry["meshes"])
    assert any(mesh["mapping"].get("slot_id") == "drive_a" for mesh in geometry["meshes"])
    assert first["asset"]["triangle_count"] > 0
    assert first["asset"]["native_summary"]["bounds"][3] > first["asset"]["native_summary"]["bounds"][0]
    assert "surface_bounds" in first["asset"]["native_summary"]
    assert import_step(database, spec)["idempotent"] is True
    print("native STEP importer test passed: hierarchy, geometry, hashes, dotted slot, bounds, idempotence")
finally:
    shutil.rmtree(temporary, ignore_errors=True)
