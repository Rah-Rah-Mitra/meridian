#!/usr/bin/env bash
# Fetch model artifacts with pinned SHA256s (SPEC §9.6 / models/MANIFEST.toml).
# Used by the image build in CI and by operators preparing a /models volume.
# The deployed device never downloads models at runtime.
#
# Usage:
#   deploy/fetch-models.sh [target-dir]      # fetch + verify against PINS below
#   PIN=1 deploy/fetch-models.sh [target]    # fetch + print hashes (to update pins)

set -euo pipefail

DEST="${1:-models}"
mkdir -p "$DEST/potion-base-8M" "$DEST/ms-marco-minilm-l6-v2"

# url|relative-path|sha256 ("TBD" only valid in PIN mode)
ARTIFACTS=(
  "https://huggingface.co/minishlab/potion-base-8M/resolve/main/model.safetensors|potion-base-8M/model.safetensors|f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2"
  "https://huggingface.co/minishlab/potion-base-8M/resolve/main/tokenizer.json|potion-base-8M/tokenizer.json|e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50"
  "https://huggingface.co/minishlab/potion-base-8M/resolve/main/config.json|potion-base-8M/config.json|2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553"
  "https://huggingface.co/cross-encoder/ms-marco-MiniLM-L-6-v2/resolve/main/onnx/model_qint8_arm64.onnx|ms-marco-minilm-l6-v2/model_qint8_arm64.onnx|3573b6b9593cb2f75987a31815d409ca3dd8808629118fd20451bb1a5d90cec7"
  "https://huggingface.co/cross-encoder/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer.json|ms-marco-minilm-l6-v2/tokenizer.json|d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66"
)

fail=0
for entry in "${ARTIFACTS[@]}"; do
  IFS='|' read -r url rel sha <<<"$entry"
  out="$DEST/$rel"
  if [[ -f "$out" ]]; then
    echo ">> exists: $rel"
  else
    echo ">> fetching $rel"
    # HuggingFace rate-limits anonymous CI fetches with HTTP 429; 3 retries over
    # ~7s was too short and intermittently reddened CI on unrelated commits.
    # Ride out a 429 burst: more retries, fixed 10s spacing (~60s window), and
    # --retry-all-errors so curl also retries 403/transient HTTP failures under -f.
    curl -fSL --retry 6 --retry-delay 10 --retry-all-errors --connect-timeout 30 -o "$out.tmp" "$url"
    mv "$out.tmp" "$out"
  fi
  actual=$(sha256sum "$out" | cut -d' ' -f1)
  if [[ "${PIN:-0}" == "1" ]]; then
    echo "PIN  $rel  $actual"
  elif [[ "$sha" == "TBD" ]]; then
    echo "!! $rel has no pinned hash (run with PIN=1 and update this script + MANIFEST.toml)"
    fail=1
  elif [[ "$actual" != "$sha" ]]; then
    echo "!! SHA256 MISMATCH for $rel"
    echo "   expected $sha"
    echo "   actual   $actual"
    fail=1
  else
    echo "   ok: $actual"
  fi
done

exit $fail
