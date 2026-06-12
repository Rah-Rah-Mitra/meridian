# 01 — Work Breakdown Structure

> SPEC §1.2. Every crate, module, and task mapped to the SPEC §16 phases, with
> dependency edges, estimated effort, and file paths. Effort is in focused
> engineer-days (d); calendar mapping follows SPEC §16's week numbers.

## 0. Crate inventory note (SPEC discrepancy)

SPEC §5 prose says "thirteen library crates" but its tree lists **fifteen**
(`common, api, query, index, vector, embed, rank, rerank, egress, privacy, fetch,
searx, geo, analytics, eval`). The list is treated as normative; all 15 are
scaffolded. A second discrepancy: the §5 dependency rules state "`egress` depends
only on `common`" and two lines later "`{api, egress, fetch, analytics} → privacy`".
Resolution: `egress → {common, privacy}` — the later, more specific rule wins, since
the lane layer must use `Redacted`/secret types to log safely.

## 1. Dependency graph (build order)

```
meridian-common  ──────────────┐ (no internal deps)
meridian-privacy ← common      │
meridian-egress  ← common, privacy
meridian-index   ← common          meridian-vector ← common
meridian-embed   ← common          meridian-rank   ← common
meridian-rerank  ← common          meridian-geo    ← common
meridian-fetch   ← common, egress, privacy
meridian-searx   ← common, egress
meridian-analytics ← common, geo, fetch, privacy
meridian-query   ← common, index, vector, embed, rank, rerank, fetch, searx, geo
meridian-api     ← common, privacy, query
meridian-eval    ← common (+ dev-deps on the crates it drives)
bins/meridiand   ← api, common (composition root; gains the rest as wiring needs them)
```
`nothing depends on api` and `common depends on nothing internal` are enforced by
review + cargo-deny graph checks in CI (a `cargo tree` assertion script lands in
Phase 1).

## 2. Phase 0 — Plan, scaffold, bench (weeks 1–2)

| # | Task | Paths | Deps | Effort |
|---|---|---|---|---|
| 0.1 | Planning artifacts (this directory) | `docs/plan/00–06` | — | 2d |
| 0.2 | Workspace scaffold, 15 crates + bin, lints, deny.toml | `Cargo.toml`, `crates/*/src/lib.rs`, `bins/meridiand/` | 0.1 | 1d ✅ |
| 0.3 | CI: fmt/clippy/test/deny/audit/gitleaks + aarch64-musl zigbuild | `.github/workflows/ci.yml` | 0.2 | 0.5d ✅ |
| 0.4 | Compose skeleton + searxng configs + healthchecks | `deploy/compose.yaml`, `deploy/searxng*/settings.yml` | — | 0.5d ✅ |
| 0.5 | pi-setup.sh (read-only skeleton → apply mode) | `deploy/pi-setup.sh` | — | 0.5d ✅(skeleton) |
| 0.6 | **meridian-bench implementation** (8 suites, md+json emitter) | `crates/meridian-eval/src/bin/meridian-bench.rs`, `crates/meridian-eval/src/bench/*` | 0.2, sign-off | 4d |
| 0.7 | Wikipedia-slice downloader for bench corpus | `crates/meridian-eval/src/bench/corpus.rs` | 0.6 | 1d |
| 0.8 | Dockerfile (scratch + CA + models) + GHCR publish job + image-size gate | `deploy/Dockerfile`, ci.yml `image` job | 0.3 | 1d |
| 0.9 | Model fetch script with pinned SHA256s | `deploy/fetch-models.sh`, `models/MANIFEST.toml` | — | 0.5d |
| 0.10 | **RUN bench on device**, update `02-budgets.md` with measured numbers | bench report → `docs/plan/bench/` | 0.6–0.9 | 1d |

**Exit gate:** scratch image <120MB boots on Pi; bench report committed; budgets
updated with measured values. *(Items 0.6–0.10 start after planning sign-off.)*

## 3. Phase 1 — Lexical MVP + direct lane + privacy core (weeks 3–5)

| # | Task | Paths | Deps | Effort |
|---|---|---|---|---|
| 1.1 | common: figment config load, telemetry init ordering | `meridian-common/src/{config,telemetry}.rs` | — | 1d |
| 1.2 | privacy: tracing redaction layer + field visitor | `meridian-privacy/src/redact.rs` | 1.1 | 2d |
| 1.3 | privacy: salted rotating client-IP hasher (blake3, 8-byte keys) | `meridian-privacy/src/iphash.rs` | — | 0.5d |
| 1.4 | privacy: secrecy/zeroize re-exports + config asserts | `meridian-privacy/src/secret.rs` | — | 0.5d |
| 1.5 | egress: DirectLane (reqwest+rustls, pool, honest UA, hedging hooks) | `meridian-egress/src/direct.rs` | 1.4 | 1.5d |
| 1.6 | fetch: SSRF guard (resolve-then-pin, deny-list, redirect re-validation) **+ unit tests** | `meridian-fetch/src/ssrf.rs` | 1.5 | 2d |
| 1.7 | fetch: robots.txt (texting_robots, 24h cache) + per-domain governor buckets (lane-global) | `meridian-fetch/src/{robots,budget}.rs` | 1.6 | 1.5d |
| 1.8 | fetch: extraction bake-off (dom_smoothie vs readability-rs) on 50-page fixture set → ADR update | `meridian-fetch/src/extract.rs`, `docs/plan/bakeoff-extraction.md` | 1.6 | 1.5d |
| 1.9 | index: tantivy schema (fast fields incl. `h3_r7`), writer mgmt, merge policy, snippets | `meridian-index/src/{schema,writer,merge}.rs` | 1.1 | 3d |
| 1.10 | index: SegmentStore local impl | `meridian-index/src/store_local.rs` | 1.9 | 1d |
| 1.11 | ingest pipeline: fetch ladder → extract → lang-ID → dedup (redb) → snippet → index | `meridian-query/src/ingest.rs` (or `meridian-index`) | 1.7–1.9 | 2.5d |
| 1.12 | searx: JSON client, result normalization, engine health | `meridian-searx/src/client.rs` | 1.5 | 1.5d |
| 1.13 | query: planner v1 (BM25 + searx fan-out, RRF, deadlines, hedging direct-only) | `meridian-query/src/{planner,rrf}.rs` | 1.9, 1.12 | 2.5d |
| 1.14 | api: axum router, tower stack, bearer auth, RFC7807, /healthz /metrics | `meridian-api/src/{router,layers,problem}.rs` | 1.2, 1.13 | 2.5d |
| 1.15 | **privacy smoke test** (canary query/IP grep over logs+metrics) in CI | `meridian-eval/src/privacy_smoke.rs`, ci job | 1.14 | 1.5d |
| 1.16 | meridiand: runtime wiring (tokio 4 workers, rayon 4 @ nice 5, oneshot bridges) | `bins/meridiand/src/main.rs` | 1.14 | 1d |
| 1.17 | searxng settings trim (≤30 engines) + digest pin | `deploy/searxng/settings.yml` | 1.12 | 0.5d |

**Exit gate:** 1M docs indexed; local p50 <50ms; RSS <1.2GB; disk <3GB; privacy
smoke green; 0 clippy warnings. *(On-device profile: 100k docs — see 02-budgets.)*

## 4. Phase 2 — Hybrid retrieval (weeks 6–8)

| # | Task | Paths | Effort |
|---|---|---|---|
| 2.1 | embed: model2vec-rs runtime, batch API, L2-norm | `meridian-embed/src/model2vec.rs` | 1.5d |
| 2.2 | vector: int8 quantizer (per-dim scale) | `meridian-vector/src/quant.rs` | 1d |
| 2.3 | vector: usearch wrapper (feature `usearch`, default) — add/search/persist/view, RAM acct | `meridian-vector/src/usearch_impl.rs` | 2d |
| 2.4 | vector: hnsw_rs fallback (feature) — u8 vectors (no i8 upstream; quantizer emits offset-u8 for this backend) | `meridian-vector/src/hnsw_impl.rs` | 2d |
| 2.5 | query: RRF over {bm25, ann, searx}; ANN stage on rayon | `meridian-query/src/{planner,rrf}.rs` | 1d |
| 2.6 | Moka caches (query 256MB / fetch 96MB / geocode read-through) + weights | `meridian-query/src/cache.rs` | 1.5d |
| 2.7 | shedding hooks (RSS/temp/disk watchers → ordered shed states, /metrics flags) | `meridian-common/src/shed.rs` | 2d |
| 2.8 | eval: labeled 100-query set v1 + nDCG/MRR/Recall harness | `meridian-eval/src/{qrels,metrics}.rs` | 2d |
| 2.9 | binary-quantization + rescore path (config, default off) | `meridian-vector/src/bq.rs` | 1.5d |

**Exit:** hybrid nDCG@10 ≥ BM25; vectors ≤600MB disk / ≤520MB RAM @1M (CI math +
bench extrapolation; 100k on-device); p50 ≤80ms.

## 5. Phase 3 — Ranking stack (weeks 9–11)

| # | Task | Paths | Effort |
|---|---|---|---|
| 3.1 | train/: LTR + intent training, ONNX export, INT8 quant scripts | `train/*.py` | 2.5d |
| 3.2 | rank: ort session mgmt (shared Environment, intra=4/inter=1), LTR features from fast fields | `meridian-rank/src/{session,features}.rs` | 2.5d |
| 3.3 | rank: cold-start linear weights behind same ONNX interface | `train/coldstart.py` | 0.5d |
| 3.4 | intent classifier integration in planner | `meridian-query/src/intent.rs` | 1d |
| 3.5 | rerank: INT8 cross-encoder, batch 4, 1.5s stage deadline, Moka cache, `degraded:["rerank_timeout"]` | `meridian-rerank/src/ce.rs` | 2.5d |
| 3.6 | searx: ε-greedy bandit per intent class, arms in redb | `meridian-searx/src/bandit.rs` | 1.5d |
| 3.7 | MMR-lite domain diversity (cap 3/domain) | `meridian-query/src/diversity.rs` | 0.5d |

**Exit:** deep p50 ≤2.5s / cache-hit ≤100ms; LTR no regression; shed flags observable.

## 6. Phase 4 — Egress lanes: anon + region (weeks 12–14)

| # | Task | Paths | Effort |
|---|---|---|---|
| 4.1 | egress: AnonLane — embedded Arti bootstrap mgmt, status surface | `meridian-egress/src/anon/arti.rs` | 3d |
| 4.2 | egress: in-process SOCKS5 listener on `back` for searxng-anon, RFC1929 username→isolation mapping | `meridian-egress/src/anon/socks.rs` | 3d |
| 4.3 | egress: per-request IsolationToken for in-core anon fetches | `meridian-egress/src/anon/client.rs` | 1.5d |
| 4.4 | egress: type-level fail-closed `AnonClient` + Arti-down injection test asserting ZERO direct egress | `meridian-egress/src/anon/mod.rs`, `tests/fail_closed.rs` | 2d |
| 4.5 | egress: RegionLane — local_address bind, lane verification via IP-echo (+GeoLite2) | `meridian-egress/src/region.rs` | 2d |
| 4.6 | wg-lane-templates: generated `ip rule` scripts + compose.regions.yaml (host network) | `deploy/wg-lane-templates/*`, `deploy/compose.regions.yaml` | 1d |
| 4.7 | anon cache isolation (ephemeral 32MB, TTL 5m; bandit/LTR firewall) | `meridian-query/src/cache.rs` | 1d |
| 4.8 | `/v1/lanes` endpoint + anon concurrency budget (2) + circuit cap (6) | `meridian-api/src/lanes.rs` | 1d |
| 4.9 | **§12.5 invariant test suite** (5 invariants, incl. socks5h DNS + lane-global rate limits) | `meridian-egress/tests/invariants.rs` | 2.5d |

**Exit:** all five §12.5 invariants green; anon metasearch p50 ≤8s; Arti-down test
proves zero direct egress; region egress-IP verification passes.

## 7. Phase 5 — Geo + analytics + retention (weeks 15–16)

| # | Task | Paths | Effort |
|---|---|---|---|
| 5.1 | geo: h3o ops + k-ring prefilter into tantivy fast-field filter | `meridian-geo/src/h3.rs`, `meridian-index/src/geofilter.rs` | 2d |
| 5.2 | geo: gazetteer fst build (offline) + lookup; geocode client + redb cache | `meridian-geo/src/{gazetteer,geocode}.rs` | 2d |
| 5.3 | analytics: GDELT puller (HTTP + manifest-MD5 verify — upstream TLS cert is broken), stream-parse, H3×topic×day counters | `meridian-analytics/src/gdelt.rs` | 2.5d |
| 5.4 | analytics: TTL compaction (90d), heatmap + trends queries | `meridian-analytics/src/{retention,trends}.rs` | 2d |
| 5.5 | analytics: nightly petgraph PageRank → domain_prior | `meridian-analytics/src/graph.rs` | 1.5d |
| 5.6 | privacy: retention TTL sweep jobs (every store under its §6.1 cap) | `meridian-privacy/src/retention.rs` | 1.5d |
| 5.7 | privacy/api: `POST /v1/forget` (tantivy delete-term + vector drop + cache purge + tombstone) **+ re-ingest refusal test** | `meridian-api/src/forget.rs` | 2d |
| 5.8 | secrets: migrate all key/token handling to secrecy/zeroize; mlock decision | `meridian-privacy/src/secret.rs` | 1d |

**Exit:** heatmap p50 ≤150ms; analytics steady ≤700MB over 2 simulated weeks;
`/v1/forget` proven; TTL sweeps hold caps.

## 8. Phase 6 — Hardening + release (weeks 17–18)

| # | Task | Effort |
|---|---|---|
| 6.1 | 24h soak (10 rps direct + 1 rps anon + ingest), RSS slope <1MB/h | 2d |
| 6.2 | Chaos drills: kill searxng / kill Arti / fill disk / hot-loop CPU → graceful fail-closed shed | 2d |
| 6.3 | Docs: operator manual, API ref, threat model, privacy policy + retention/deletion guide | 2.5d |
| 6.4 | Backup/restore drill (tar+zstd of /data) | 0.5d |
| 6.5 | Egress security review + privacy review sign-offs | 1d |
| 6.6 | v0.1.0 multi-arch images + SBOM (syft) + flip repo public | 1d |

## 9. Phase 7 — Evidence foundations + statistical rigor (post-v0.1.0 → v0.2.0)

Experiments FIRST (suites fix the ADR-18/21 constants), production code second.
Zero egress-code changes this phase; the 13 invariant tests must merely stay green.

| # | Task | Paths | Deps | Effort |
|---|---|---|---|---|
| 7.1 | Planning docs: ADR-18..26, SPEC §16 P7–P9 (v2.2), WBS/budgets/risks/bench amendments | `docs/SPEC.md`, `docs/plan/*` | — | 1.5d ✅ |
| 7.2 | Suite 9 `synfarm`: syndication-farm generator (primary + held-out variant) + shingle/MinHash/SimHash parameter sweep → pairwise F1 + false-merge; fixes ADR-18 constants | `meridian-eval/src/bench/synfarm.rs` | 7.1 | 2d |
| 7.3 | Suite 10 `spike`: planted Poisson spikes on a hex lattice; ratio baseline vs EB+Gi*+BH → FPR/TPR; fixes ADR-21 prior + k-ring | `meridian-eval/src/bench/spike.rs` | 7.1 | 1.5d |
| 7.4 | Suite 12 probe: same-lane JSD noise floor (direct/direct, anon/anon), bootstrap CI; report → `docs/plan/bench/` (Phase-8 entry evidence) | `meridian-eval/src/bench/divergence.rs` (feature `bench-divergence`) | 7.1 | 1.5d |
| 7.5 | Sketch module: word shingles → 64-bit SimHash + MinHash signatures (constants from 7.2) | `meridian-index/src/sketch.rs` | 7.2 | 2d |
| 7.6 | Ingest integration: sketch rows in `sketch_v1` (dedup.redb) written in the SAME txn as dedup/tombstone; both forget paths drop them atomically (ADR-19) | `meridian-query/src/ingest.rs` | 7.5 | 1.5d |
| 7.7 | Query-time derivation clustering + evidence assembly (post-RRF/pre-LTR slot); web results `evidence: null` | `meridian-query/src/evidence.rs`, `planner.rs` | 7.5, 7.6 | 2d |
| 7.8 | API: `evidence` block + response `analysis` summary; `evidence.enabled` kill-switch; docs | `meridian-api/src/lib.rs`, `docs/api.md` | 7.7 | 0.5d |
| 7.9 | Trends/heatmap statistics: EB shrinkage + Gi* (h3o grid_disk k-ring) + BH-FDR; raw values retained | `meridian-analytics/src/stats.rs` (new), `trends.rs` | 7.3 | 2.5d |
| 7.10 | Trends/heatmap API additive fields: `z`, `q_value`, `shrunk_rate`, `significant`, noise label | `meridian-analytics`, `meridian-api`, `docs/api.md` | 7.9 | 0.5d |
| 7.11 | Forget-correctness extension (sketch table + cluster annotations) + suite 11 `evidence-latency` micro-bench | `meridian-query` tests, `meridian-eval/src/bench/` | 7.6, 7.7 | 1d |
| 7.12 | Exit: re-run suites 1–11 on device, re-validate budgets, `phase-exits/p7.md`, release notes, tag v0.2.0 | `docs/plan/phase-exits/p7.md`, `docs/release-notes/v0.2.0.md` | all | 1d |

**Exit gate:** SPEC §16 Phase 7 (synfarm F1 >0.8 / false-merge <5% both variants;
spike ≥3× FPR reduction; evidence ≤2ms p50; sketch ≤64 B/doc; ingest regression
≤10%; RSS no regression; forget 100% incl. sketches; noise-floor report committed).
≈ 17.5d.

## 10. Phase 8 — Vantage divergence + confidence (v0.3.0)

Entry condition: Phase-7 noise-floor report committed (the divergence gate is
meaningless without a measured floor).

| # | Task | Paths | Deps | Effort |
|---|---|---|---|---|
| 8.1 | ADR-22/23 finalization + threat-model §cross-lane correlation + doc amendments | `docs/plan/*` | — | 1d |
| 8.2 | Compare orchestrator: `compare=vantages` flag; fan-out over {direct, anon}, independent fail-closed resolution; compare responses NEVER in the shared cache | `meridian-query/src/compare.rs` (new), `planner.rs` | 8.1 | 2.5d |
| 8.3 | Divergence stats: JSD over per-lane domain distributions + bootstrap vs Phase-7 floor + `domains_only_in`; `divergence` block | `meridian-query/src/compare.rs` | 8.2 | 1.5d |
| 8.4 | +3 egress invariants (≥16 total): (a) Arti-down ⇒ anon half errors, zero direct retry; (b) no shared-cache write from compare; (c) no `Bandit::reward` from the anon half | hermetic tests w/ MockDialer | 8.2 | 1.5d |
| 8.5 | Timing decorrelation: randomized inter-lane jitter (configurable, default-on) + threat-model residual-risk write-up | `compare.rs`, `docs/plan/05-threat-model.md`, `docs/privacy.md` | 8.2 | 1d |
| 8.6 | QPP confidence: NQC + Clarity post-LTR; `confidence` block | `meridian-rank/src/qpp.rs` (new), `planner.rs` | — | 2d |
| 8.7 | MMR diversity rerank (`diversity=mmr`, off by default; existing embeddings) — **built, suite-rejected (13b, both seeds), API surface withdrawn pre-release**; deviation: similarity was token-Jaccard (web results carry no vectors); carried forward: evidence-cluster diversity over ADR-18 clusters | `meridian-rank/src/mmr.rs` (library only), bench/2026-06-12-pi5-p8-gates.md | — | 1.5d |
| 8.8 | Suites 12 (cross-lane mode) + 13 (`qpp`) wired into meridian-bench; alpha-nDCG@10 in `metrics.rs` | `meridian-eval/` | 8.3, 8.6 | 1.5d |
| 8.9 | Docs: api.md (`compare`/`confidence`/`diversity`), privacy.md compare-mode disclosure (query goes out over Tor AND direct, by explicit request only) | `docs/api.md`, `docs/privacy.md` | 8.2–8.7 | 0.5d |
| 8.10 | Exit re-validation + `phase-exits/p8.md` + v0.3.0 | docs | all | 1d |

**Exit gate:** SPEC §16 Phase 8. ≈ 14d.

## 11. Phase 9 — Adaptive frontier: decision log, OPE, contextual routing, VoI (v0.4.0)

Strict internal order: log → OPE harness → policy (gated off) → DR ship decision.
The DR decision may trail the v0.4.0 tag as a config flip (decision accrual is
calendar-bound, ADR-25).

| # | Task | Paths | Deps | Effort |
|---|---|---|---|---|
| 9.1 | ADR-24 privacy review: threat-model §decision-log + privacy.md disclosure (bucketed routing metadata ≠ query logging — reconciliation recorded) | `docs/plan/05-threat-model.md`, `docs/privacy.md` | — | 1.5d |
| 9.2 | Decision log: redb table {buckets, arm, propensity, reward}; 30d TTL sweep; k-anon floor (<5/24h generalized); wipe path; ≤20MB cap; **anon decisions never logged**; ε-greedy propensities (ε/K, 1−ε+ε/K) emitted from day one | `meridian-searx/src/decision_log.rs` (new), `bandit.rs` | 9.1 | 2d |
| 9.3 | OPE harness: IPS + doubly-robust estimators + synthetic-truth recovery tests (suite 14); offline bridge to `train/` | `meridian-eval/src/ope.rs` (new), `train/` | 9.2 | 2.5d |
| 9.4 | Contextual policy: linear Thompson sampling, ~20-dim one-hot context, same 3 arms, same choose/reward interface; feature-gated default-OFF | `meridian-searx/src/contextual.rs` (new) | 9.3 | 2d |
| 9.5 | Ship decision per ADR-25 (≥10k decisions or 60 days): DR report committed; enable only if 95% CI excludes zero; else "inconclusive, ε-greedy retained" in exit note | `docs/plan/bench/` DR report | 9.2–9.4 + dwell | 1d |
| 9.6 | VoI fetch/stopping: Pandora's-box reservation values at deep-mode candidate selection + ingest frontier; novelty = MinHash + embedding coverage; `analysis.search_stopped_because` | `meridian-fetch/src/ladder.rs`, `meridian-query/src/planner.rs` | P7 sketches | 3d |
| 9.7 | Suite 15 `voi`: deep-mode fetch replay → fetches-vs-nDCG@10 frontier + evidence-diversity guard | `meridian-eval/src/bench/voi.rs` | 9.6 | 1.5d |
| 9.8 | Docs (operator manual: decision log + wipe; api.md if `fetch_budget` surfaces), exit, `phase-exits/p9.md`, v0.4.0 | docs | all | 1d |

**Exit gate:** SPEC §16 Phase 9. ≈ 14.5d (+ calendar dwell for decision accrual).

## 12. Cross-phase engineering rules

- Re-validate `02-budgets.md` at every phase exit (SPEC §1).
- Heavy deps enter the workspace only in the phase that uses them (keeps local
  `cargo check` viable on the dev Pi and CI fast).
- Every egress/privacy behavior ships WITH its test in the same PR — no
  "tests later" for the two security-critical crates.
- **Post-v0.1.0 additions:** experiments precede features (a gated feature's eval
  suite must exist and pass BEFORE the feature merges); the hermetic
  egress-invariant count is monotonically non-decreasing (13 → ≥16 → ≥17); every
  new derived structure passes the ADR-19 deletability test in the same PR.
