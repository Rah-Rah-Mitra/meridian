# Meridian

**A geo-aware, privacy-conscious edge search appliance in Rust** — metasearch + selective local
hybrid index (BM25 + dense vectors) + geo analytics, sized for a Raspberry Pi 5 and scaling to
edge servers without redesign.

> Status: **v0.5.0 released — the honest-verdicts release.** Deep search can
> now answer: `answer=true` reads the most promising pages with the
> single-best (Pandora) selector and returns the best passage verbatim with
> its source and a raw relevance score (suite-18-gated: +10.7pp hit-rate
> over not fetching, 3.8× the page-selector's at fewer fetches; extractive
> only — never generated). Trends see a story building: `burst` flags
> sustained multi-day elevations the latest-day z statistically cannot
> (suite-17-gated at LOWER false-positive rate than the incumbent). Two
> features were killed by their own gates before any user saw them
> (conformal confidence bands; the VoI embedding-coverage term) — recorded,
> not hidden. amd64 is back (multi-arch manifest) and the 1M ANN baseline
> is now measured, not extrapolated (recall@10 0.98, p99 1.7ms, 506MB).
> 21 hermetic invariants. Previous: **v0.4.0 — the adaptive frontier
> release** (`fetch_budget` VoI page-reading + the `analysis` block; the
> ADR-25 decision-log/OPE machinery, dwelling toward its ship gate).
> Earlier:
> **v0.3.0 — the divergence release.** The flagship claim is
> now measured, per request: the same query over direct AND Tor returns
> measurably different source distributions (cross-lane JSD ~7× the same-lane
> noise floor, bootstrap p≈0 on this deployment), behind a fail-closed
> compare orchestrator pinned by 17 hermetic egress invariants. Every
> response carries a `confidence` block (QPP: ρ=0.256 vs measured nDCG@10 at
> sub-ms cost). An MMR diversity rerank was built and **withdrawn by its own
> gate** (it demoted canonical originals with their syndicated copies —
> see the p8 exit note); the Phase-9 substrate (privacy-vetted decision log,
> default off + validated IPS/DR offline-policy evaluation) shipped dark.
> Images are linux/arm64 only (see release notes). Docs:
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
| 8 | Vantage divergence + confidence → v0.3.0 | **Done** (2026-06-12) — [exit note](docs/plan/phase-exits/p8.md); divergence + QPP gates PASS; MMR suite-rejected and withdrawn |
| 9 | Adaptive frontier (decision log, OPE, contextual routing, VoI) → v0.4.0 | **Done** (2026-06-12) — [exit note](docs/plan/phase-exits/p9.md); suites 14+15 PASS; ADR-25 flip trails on decision accrual (by design) |
| 10 | Calibrated confidence, change-aware trends, answer mode → v0.5.0 | **Done** (2026-06-13) — [exit note](docs/plan/phase-exits/p10.md); suites 17+18 PASS; suites 16+15b honestly negative (features withdrawn pre-ship); amd64 + 1M ANN carries closed |
| 11 | Candidates: region metasearch sidecars (operator sign-off), DP aggregates, BQ (>1.5M docs), crates.io | Recorded, not scheduled |

Known pending beyond the phase table: hybrid-nDCG re-evaluation on the
healed dense lane (the Phase-2 baseline understates it); the ADR-25
contextual-policy verdict (calendar-bound dwell — `GET /v1/decision-log/ope`
at ≥10k decisions or the ~2026-08-11 sunset); the answer-mode latency
optimization study (win the 3.0s p50 row back from the measured 3.5s).

## Installing

```sh
docker pull ghcr.io/rah-rah-mitra/meridian/meridiand:0.5.0   # linux/arm64 + linux/amd64
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
