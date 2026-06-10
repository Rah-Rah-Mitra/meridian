# train/

Offline Python tooling (allowed offline only — no Python in the runtime path, SPEC §2):

- LTR training: LightGBM on the labeled eval set → ONNX export (≤200 trees, depth ≤6).
- Intent classifier: tiny GBDT → ONNX.
- INT8 dynamic quantization of the cross-encoder (if the upstream INT8 artifact is
  ever insufficient).

Arrives in Phase 3. Outputs land in `models/` via `MANIFEST.toml` pins.
