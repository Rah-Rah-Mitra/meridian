# Meridian

**A geo-aware, privacy-conscious edge search appliance in Rust** — metasearch + selective local
hybrid index (BM25 + dense vectors) + geo analytics, sized for a Raspberry Pi 5 and scaling to
edge servers without redesign.

> Status: **Phase 5 complete** — geo + analytics + retention/forget are live.
> Documents are geo-tagged at ingest by an offline GeoNames gazetteer (H3
> res-7), search takes geo/time filters, `/v1/geo/heatmap` answers in 27ms
> p50 on a 100k-doc corpus, and opt-in GDELT analytics feed `/v1/trends` plus
> a PageRank-derived ranking prior. `POST /v1/forget` provably erases a
> document from lexical+vector+caches and tombstones it against re-ingest.
> Phase 6 (hardening + v0.1.0 release) is underway. Planning artifacts:
> [`docs/plan/`](docs/plan/); docs: [operator manual](docs/operator-manual.md)
> · [API](docs/api.md) · [privacy](docs/privacy.md).

## What it is

- **Query routing** across upstream engines (SearXNG sidecar) and a local Tantivy + USearch hybrid
  index, under strict latency/resource budgets.
- **Explainable ranking**: RRF fusion → GBDT LTR → optional INT8 cross-encoder rerank; every
  response exposes `rank_signals` and per-stage `timings`.
- **Geo enrichment & analytics**: H3 cells as first-class index fields, heatmaps, GDELT-derived
  regional trend rollups.
- **Selectable egress lanes per request**: `direct` (default) · `region:<id>` (operator's WireGuard
  vantage points) · `anon` (Tor via embedded Arti, fail-closed — never silently falls back to
  direct).
- **Privacy guardrails as hard requirements**: no query logging by default, client IPs never
  persisted, strict retention TTLs, `POST /v1/forget` deletion path, zeroized secrets, no
  third-party telemetry.

The full, binding specification is [`docs/SPEC.md`](docs/SPEC.md) (v2.1).

## Repository layout

| Path | Contents |
|---|---|
| `docs/SPEC.md` | The v2.1 implementation spec (single source of truth) |
| `docs/plan/` | §1 Planning Protocol artifacts: ADRs, WBS, budgets, risk register, bench plan, threat model, operator questions |
| `crates/` | 15 library crates (see SPEC §5) |
| `bins/meridiand` | The single composition-root binary |
| `deploy/` | Docker Compose (profiles: default / `anon` / `regions`), SearXNG configs, `pi-setup.sh`, WireGuard lane templates |
| `models/` | Model manifest with pinned SHA256s — artifacts fetched at build, never committed |
| `train/` | Offline Python: LTR/intent training, ONNX export, INT8 quantization |

## Project status

| Phase | Scope | State |
|---|---|---|
| 0 | Planning artifacts, workspace + CI scaffold, bench harness | **Done** (2026-06-10) |
| 1 | Lexical MVP + direct lane + privacy core | **Done** (2026-06-10) |
| 2 | Hybrid retrieval (embeddings + ANN) | **Done** (2026-06-10) |
| 3 | Ranking stack (LTR, deep rerank, bandit routing) | **Done** (2026-06-10) |
| 4 | Egress lanes: anon (Arti) + region (WireGuard) | **Done** (2026-06-11) |
| 5 | Geo + analytics + retention/forget | **Done** (2026-06-11) |
| 6 | Hardening + v0.1.0 release | In progress |

## Building

**Never compile on the deployment Pi.** Release builds are cross-compiled in CI
(`cargo zigbuild --target aarch64-unknown-linux-musl`) and shipped as `FROM scratch` images.
For development, `cargo check` / `cargo clippy` are sufficient locally; tests and builds run in
GitHub Actions.

## Data credits

- Gazetteer place names: [GeoNames](https://www.geonames.org/) (`cities15000`),
  licensed [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) — fetched
  and compiled offline by `deploy/fetch-gazetteer.sh`, never redistributed here.
- Regional event analytics (operator opt-in): [GDELT v2](https://www.gdeltproject.org/)
  15-minute event slices — stream-parsed into bounded counters, raw slices never stored.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
The SearXNG sidecar (AGPL-3.0) is consumed only as an isolated container over HTTP/JSON — no AGPL
code is linked or vendored; `cargo-deny` enforces the license policy in the core dependency graph.
