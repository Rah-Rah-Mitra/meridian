# Meridian

**A geo-aware, privacy-conscious edge search appliance in Rust** — metasearch + selective local
hybrid index (BM25 + dense vectors) + geo analytics, sized for a Raspberry Pi 5 and scaling to
edge servers without redesign.

> Status: **v0.2.0 released — the evidence release.** Meridian now tells you
> how independent your sources are: every search response carries derivation
> clusters (`independent_source_count` vs `apparent_source_count`, built from
> deletable 64-byte MinHash sketches at +0.2ms p50), trends/heatmap carry
> defensible statistics (empirical-Bayes + overdispersion-aware z-scores +
> FDR `significant` flags instead of raw ratios), and `compare=vantages`
> previews Phase 8: the same query over direct AND Tor with a Jensen-Shannon
> divergence report. Also in v0.2.0: a latent dense-recall defect (since
> Phase 2) found and fixed — recall@10 restored 0.0 → 0.999 at scale.
> v0.2.0 images are linux/arm64 only (see release notes). Docs:
> [operator manual](docs/operator-manual.md) · [API](docs/api.md) ·
> [privacy](docs/privacy.md) · exit notes in
> [`docs/plan/phase-exits/`](docs/plan/phase-exits/).

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
- **Evidence & uncertainty (v0.2.0+)**: source-independence clusters on every response (deletable
  sketches — forget erases them in the same transaction), statistically gated trends/heatmap,
  vantage-divergence comparison across egress lanes, and raw query-performance predictors
  (`confidence` block).

The full, binding specification is [`docs/SPEC.md`](docs/SPEC.md) (v2.2).

## Repository layout

| Path | Contents |
|---|---|
| `docs/SPEC.md` | The v2.2 implementation spec (single source of truth) |
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
| 6 | Hardening + v0.1.0 release | **Done** (2026-06-11) |
| 7 | Evidence foundations + statistical rigor → v0.2.0 | **Done** (2026-06-12) — [exit note](docs/plan/phase-exits/p7.md) |
| 8 | Vantage divergence + confidence → v0.3.0 | **In progress** — compare-vantages, QPP confidence, MMR landed; pending: cross-lane gate run, confidence calibration (suite 13), alpha-nDCG eval, full soak, exit note |
| 9 | Adaptive frontier (decision log, OPE, contextual routing, VoI) → v0.4.0 | Planned (ADR-24..26) |
| 10 | Candidates: region metasearch sidecars, conformal calibration, change-point trends, amd64 image restoration | Recorded, not scheduled |

Known pending beyond the phase table: 1M ANN re-baseline + hybrid-nDCG
re-evaluation on the healed dense lane (the Phase-2 baseline understates it),
and the amd64 image (suspended at v0.2.0 — upstream numkong x86 headers
assume glibc and conflict with zig-musl).

## Installing

```sh
docker pull ghcr.io/rah-rah-mitra/meridian/meridiand:0.2.0   # linux/arm64
```

See the [operator manual](docs/operator-manual.md) for the full compose-based
install. Binaries, SBOMs, and checksums are attached to each
[GitHub release](https://github.com/Rah-Rah-Mitra/meridian/releases).

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
