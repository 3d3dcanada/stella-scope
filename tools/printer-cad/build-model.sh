#!/bin/bash
# Rebuild one of the Prusa models the build carries from its maker's STEP file (finish-2 F09, 2026-10-19), and say what to pin.
#   tools/printer-cad/build-model.sh DIR STEP_FILE [OUT_DIR] [--install]
#     DIR        prusa-core-one-l | prusa-core-one | prusa-mk3s | prusa-mini  (the folder here that holds its groups.json)
#     STEP_FILE  the maker's STEP file, downloaded from its own page (README.md in this folder lists each address and the SHA-256 it must have)
#     OUT_DIR    where model.glb and model.glb.gz go (default /tmp/stella-model-DIR)
#     --install  copy the gzipped model to models/DIR.glb.gz in this repository (what the build embeds)
# Runs the converter (step_to_glb.py) with FORMA's installed Python (OCP + cadgen, our FORMA, /usr/lib/FORMA), one core, low priority. The same STEP and
# groups.json give the same bytes every time: the CORE One L comes out as the model the owner installed (decoded SHA-256 f068e1b7...).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); repo=$(cd "$here/../.." && pwd)
dir=$1; step=$2; out=/tmp/stella-model-$dir; install=0
shift 2; for a in "$@"; do case "$a" in --install) install=1;; *) out=$a;; esac; done
[ -f "$here/$dir/groups.json" ] || { echo "no $here/$dir/groups.json" >&2; exit 2; }
[ -f "$step" ] || { echo "no STEP file $step" >&2; exit 2; }
py=${STELLA_PYTHON:-/usr/lib/FORMA/app-runtime/python/bin/python3}
mkdir -p "$out"
echo "source STEP sha256 $(sha256sum "$step" | cut -c1-64)  $(stat -c%s "$step") bytes"
(cd /tmp && nice -n 10 "$py" -I "$here/step_to_glb.py" glb "$step" "$here/$dir/groups.json" "$out/model.glb")
gzip -n -9 -k -f "$out/model.glb"
echo "decoded  sha256 $(sha256sum "$out/model.glb" | cut -c1-64)  $(stat -c%s "$out/model.glb") bytes   (model_sha256 in the catalogue)"
echo "stored   sha256 $(sha256sum "$out/model.glb.gz" | cut -c1-64)  $(stat -c%s "$out/model.glb.gz") bytes   (stored_model_sha256 and bytes)"
if [ "$install" = 1 ]; then mkdir -p "$repo/models"; cp "$out/model.glb.gz" "$repo/models/$dir.glb.gz"; echo "installed models/$dir.glb.gz"; fi
