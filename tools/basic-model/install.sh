#!/usr/bin/env bash
# Installs the basic Voron 2.4 R2 350 model next to a Stella Scope recording database.
#
#   tools/basic-model/install.sh /path/to/stella-scope.sqlite3
#
# The model lives in the managed asset library beside the database: <database>.assets/. If that
# library already has other assets, the new entry is merged into its manifest with python3.
# Run `stella-scope --print-config` to see which database your instance uses.
set -euo pipefail

RELEASE="${STELLA_BASIC_MODEL_URL:-https://github.com/3d3dcanada/stella-scope/releases/download/basic-model-v1/voron-24r2-350-basic.tar.gz}"
SHA256="${STELLA_BASIC_MODEL_SHA256:-b5644f1c6efd6387de28f07c4a8183abbc3e54a3974a1fd46d35df5b00539f1e}"
db="${1:?usage: install.sh /path/to/stella-scope.sqlite3}"
lib="${db}.assets"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Downloading the basic model…"
curl -fL --progress-bar -o "$work/pack.tar.gz" "$RELEASE"
if [ -n "$SHA256" ]; then
  echo "$SHA256  $work/pack.tar.gz" | sha256sum -c -
fi
tar -xzf "$work/pack.tar.gz" -C "$work"
[ -f "$work/manifest.json" ] && [ -f "$work/voron-24r2-350-basic/geometry.json" ] || { echo "The download is not a basic model pack." >&2; exit 1; }

mkdir -p "$lib"
rm -rf "$lib/voron-24r2-350-basic"
cp -r "$work/voron-24r2-350-basic" "$lib/"
if [ ! -f "$lib/manifest.json" ]; then
  cp "$work/manifest.json" "$lib/manifest.json"
else
  command -v python3 >/dev/null || { echo "The library already has a manifest; merging it needs python3." >&2; exit 1; }
  python3 - "$lib/manifest.json" "$work/manifest.json" <<'PY'
import json, sys
current, new = (json.load(open(p)) for p in sys.argv[1:3])
entry = new["assets"][0]
current["assets"] = [a for a in current.get("assets", []) if a.get("id") != entry["id"]] + [entry]
tmp = sys.argv[1] + ".tmp"
json.dump(current, open(tmp, "w"), indent=1)
import os; os.replace(tmp, sys.argv[1])
PY
fi
echo "Installed into $lib. Reload Stella Scope to see the printer."
