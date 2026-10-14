#!/bin/sh
# Zip the Chrome extension for the store: dist/orchard-download-manager-chrome.zip.
# The store gives the extension its own key and ID, so the development key
# is left out of the manifest (add the store's ID to EXTENSION_IDS in
# src/native_host.rs, so the app lets it in).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
src="$root/extension/chrome"
out="$root/dist/orchard-download-manager-chrome.zip"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cp -R "$src/." "$tmp/"
python3 - "$tmp/manifest.json" <<'PY'
import json, sys
path = sys.argv[1]
m = json.load(open(path, encoding="utf-8"))
m.pop("key", None)
json.dump(m, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
find "$tmp" -name .DS_Store -delete
mkdir -p "$root/dist"
rm -f "$out"
(cd "$tmp" && zip -qr -X "$out" .)
echo "Built ${out#"$root"/}"
