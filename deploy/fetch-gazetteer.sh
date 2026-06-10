#!/bin/sh
# Fetch GeoNames cities15000 and build the gazetteer fst (SPEC §3 geo-tagging;
# §6.1 budget: 10MB shipped in the image). Run from the repo root:
#   ./deploy/fetch-gazetteer.sh [out-dir]
#
# GeoNames data is CC-BY 4.0 (https://www.geonames.org/export/) — attribution
# lives in the README's data-credits section. The fst is built OFFLINE (here or
# in CI), never on the appliance.

set -eu
OUT_DIR="${1:-models}"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

URL="https://download.geonames.org/export/dump/cities15000.zip"

echo ">> fetching $URL"
curl -fsSL --retry 3 -o "$SCRATCH/cities15000.zip" "$URL"
unzip -q -o "$SCRATCH/cities15000.zip" -d "$SCRATCH"

echo ">> building fst"
cargo run -q -p meridian-eval --bin meridian-eval -- gazetteer \
    --source "$SCRATCH/cities15000.txt" \
    --out "$OUT_DIR/gazetteer.fst"

ls -la "$OUT_DIR/gazetteer.fst"
