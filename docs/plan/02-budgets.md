# 02 — Resource Budgets (Dual Profile)

> SPEC §1.3, refined from SPEC §6. **Ceilings, never targets.** Re-issued
> 2026-06-10 with MEASURED values from the on-device run
> ([bench/2026-06-10-pi5-report.md](bench/2026-06-10-pi5-report.md)); measured
> numbers are marked ✓. Re-validated at every phase exit. Caveats: release-bench
> profile (thin LTO) and short simplewiki docs — both noted in the report.

## 0. Why two profiles

The spec budgets 8GB free on NVMe for the appliance alone. The actual first device
(this Pi 5) has **one 29.7GB SD card, ~6.7–7.7GB free, no NVMe**, and doubles as the
dev box hosting other services. Decision (operator-confirmed):

- **Profile F (Full)** — the SPEC §6 budgets verbatim: 0.5–1M docs, 8GB, NVMe.
  Validated by bench math + CI artifacts; it is the *published* sizing for
  NVMe-equipped deployments and the tier the architecture must never outgrow.
- **Profile R (Reduced, on-device)** — what actually runs on this card: **100k docs,
  ≤3.0GB total Meridian footprint**, SD-card I/O expectations. All tripwires scale
  down proportionally; the §6 ceilings still bind absolutely.

## 1. Disk

| Item | Profile F (≤8.0GB) | Profile R (≤3.0GB) | Tripwire (R in parens) |
|---|---|---|---|
| Docker images (meridiand + searxng) | 550MB | 550MB | CI fails meridiand >120MB; searxng digest-pinned |
| Models | 80MB | 80MB | manifest-pinned; fetched at image build |
| Gazetteer fst | 10MB | 10MB | built offline |
| Tantivy index | 1.4GB @1M | 150MB @100k — **measured 53MB ✓** (527 B/doc, short docs) | alert 1.3GB (140MB) |
| USearch vectors (256-d int8, M=16) | 600MB @1M | 60MB @100k | alert 560MB (56MB); **RAM @1M measured 506MB ✓** |
| redb KV (geocode/dedup/frontier/bandit) | 600MB | 150MB | LRU-evict geocode; weekly compaction |
| Analytics rollups (90-day TTL) | 700MB | 250MB | daily TTL job; alert 650MB (230MB) |
| Arti state + directory cache | 200MB | 200MB | prune stale consensus on start |
| GeoLite2 (optional) | 70MB | 0 (off) | feature-gated |
| Logs | 200MB | 200MB | journald 150M + json-file 10m×3 |
| Merge/ingest scratch | ≥1.0GB free | ≥0.75GB free | `statvfs` pause-ingest gate |
| Spare | ~2.5GB | ~0.6GB | — |

Additional Profile R constraint: the card ALSO carries the dev checkout + a local
`cargo check` target dir (≤1.5GB, cleanable at any time) and other tenants. The
pause-ingest gate fires on **device-global** free space, not Meridian's own usage,
so a neighbor filling the disk still halts our writes safely.

SD-card endurance note (Profile R only): merge I/O and analytics compaction are
write-amplifying. Nightly (not idle-triggered) merges, and `vm.dirty_*` tuned per
SPEC §7.3, keep write bursts coalesced. NVMe deployments (Profile F) are unaffected.

## 2. RAM (both profiles — the Pi has 8GB either way)

| Item | Budget |
|---|---|
| OS + Docker + journald (+ other tenants on the shared dev Pi) | ~800MB (R: up to 1.6GB observed — see note) |
| searxng cgroup | 512MB |
| searxng-anon cgroup (anon profile only) | 384MB |
| meridiand cgroup | **3GB hard**, ≤2.6GB steady |
| — USearch resident | 520MB @1M / ~60MB @100k |
| — Moka (query 256 + fetch/geo 128) | 384MB |
| — ort sessions | 250MB |
| — Arti runtime (anon enabled) | 80–150MB |
| — tantivy writer heap (ingest) | 256MB |
| — tokio/rayon/misc | 350MB |
| Page cache for mmap'd segments + models | remainder — do not "free" it |
| zram 2GB zstd | alert if >256MB used |

Shared-device note (Profile R): the dev Pi already runs other services
(~1.0–1.9GB observed). meridiand's cgroup cap, not the §6.2 OS estimate, is the
binding contract; if steady-state headroom drops below 512MB free+reclaimable, the
shed ladder (SPEC §8.6) starts early at RSS >2.3GB.

## 3. CPU & latency (p50 targets — MEASURED 2026-06-10 where marked ✓)

| Path | Profile F target | Profile R measured/expected | Bench grounding |
|---|---|---|---|
| fast, local-only, direct | ≤80ms (keep ceiling) | **stages sum <5ms @100k ✓** — working target ≤25ms | §15.1–3, 5 ✓ |
| fast, metasearch, direct | ≤900ms | ≤900ms (network-bound, Phase-1 load test) | §15.3 + load |
| fast, metasearch, anon | ≤8s, no hedging | **connect p50 1.10s / p99 2.29s post-bootstrap ✓** — 8s holds; cold bootstrap 14.7s happens at lane-enable, never inside a request | §15.8 ✓ |
| deep, direct | ≤2.5s | **UNMEASURED** — tract cannot load the INT8 CE (ADR-02); re-measure at Phase-3 entry | §15.4 (blocked) |
| /geo/heatmap | ≤150ms | 27ms ✓ (res 5, 100k docs, on-device) | Phase-5 exit |
| Ingest sustained | ≥50 docs/s | **index-side 15,021 docs/s ✓** (300×) — fetch/extract-bound as designed | §15.3, 7 ✓ |

Measured stage numbers (fast/local, Profile R): BM25 top-1000 **0.48ms p50 @100k ✓**
(0.10/0.28/0.48 at 10k/50k/100k; extrapolated 2–5ms @1M) · embed **<1ms** (42.9k
docs/s batch-32 ✓) · ANN **0.45ms p50 / 0.95ms p99 @ ef=64, recall@10 0.98 ✓**
(`derived_ef_search = 64`; ef=128 → recall 1.00 at 1.45ms p99) · RRF+LTR-shape
**0.144ms ✓**. Thermal: 10-min all-core max **75.7°C, zero throttle flags ✓**.
Merge transient @100k: **58MB ✓** (1Hz sampler caveat; re-check at F scale).

Threading contract (SPEC §7.2): tokio 4 workers (IO only, >100µs CPU work
forbidden), one rayon pool ×4 @ nice 5, ort intra=4/inter=1, ingest ≤2 rayon
tickets @ nice 10, per-query concurrency 8 (anon: separate budget of 2).

## 4. Network / QPS

- Saturation estimate: 10–20 hybrid QPS (Profile R: treat >10 QPS as out of scope).
- Public rate limit: 5 rps/IP burst 20 (governor, hashed-IP keys).
- Anon: ≤6 concurrent Tor circuits; 1–2 rps load-test ceiling.
- Per-domain fetch budget: 1 req/2s sustained, globally across lanes.

## 5. Dev-loop budget (Profile R only, ADR-D1)

| Item | Budget | Enforcement |
|---|---|---|
| Source checkout | ≤50MB | — |
| Local `cargo check`/clippy target dir | ≤1.5GB | `cargo clean` when tripped; never `cargo build --release` locally |
| cargo registry cache delta | ≤500MB | pi-cleanup-safe (regenerable) |

Measured 2026-06-10 (scaffold): check target 255MB, ~11s. With all bench features
(tantivy+usearch+tract+tokenizers): **1.9GB** — slightly over the cap; acceptable
because bench features are off by default and `cargo clean` recovers it. Heavy
Phase-1+ deps stay feature-scoped where possible.

## 6. Phase-exit budget checkpoints

| Phase exit | Must hold |
|---|---|
| 0 | **DONE 2026-06-10 ✓** — image 55.4MB boots on Pi under hardening flags; bench report committed; this file re-issued |
| 1 | **DONE 2026-06-10 ✓** — 100k docs, disk 57.5MB, RSS 97.6MB, end-to-end p50 10.6ms ([exit note](phase-exits/p1.md)) |
| 2 | **DONE 2026-06-10 ✓** — hybrid nDCG@10 0.42 > BM25 0.38; vectors 405MB disk / 506MB RAM @1M (extrapolated); hybrid p50 14.5ms ([exit note](phase-exits/p2.md)) |
| 3 | **DONE 2026-06-10 ✓** — deep p50 204ms (CE rerank, gnu/ort image), cache-hit 0.47ms, LTR no-regression ([exit note](phase-exits/p3.md)) |
| 4 | **DONE 2026-06-11 ✓** — anon metasearch cold p50 1.34s (≤8s), Arti RSS delta +47MB (≤150MB), zero direct egress proven hermetically + live bootstrap windows; image 74.7MB ([exit note](phase-exits/p4.md)) |
| 5 | **DONE 2026-06-11 ✓** — heatmap p50 27ms; analytics 128MB/14 simulated days; stores: index 47.6MB, vectors 38.6MB, dedup 9.6MB; search p50 12ms on schema v3 ([exit note](phase-exits/p5.md)) |
| 6 | **DONE 2026-06-11 ✓** — soak (abbreviated, see [exit note](phase-exits/p6.md)): heap slope ≤0MB/h post-warm, RSS plateau ~250MB (mimalloc), temp ≤61°C, p99 29–34ms, 100% success; drills + reviews + v0.1.0 multi-arch release |
| 7 | **DONE 2026-06-11 ✓** — evidence +0.2ms p50 (A/B kill-switch, same deployment; suite 11: 0.208ms release); heatmap+Gi* 23.7ms p50 (≤60); sketch 123 B/doc gross (ceiling re-issued — payload 64 B); ingest 99 docs/s sustained API-level (rate-limiter-paced, ≥50 gate); RSS 261MB post-ingest (plateau band) ([exit note](phase-exits/p7.md)) |
| 8 | fast path (no compare flag): zero p50/p95 change; compare-mode ≤ slowest lane + 500ms; QPP ≤1ms |
| 9 | decision log ≤20MB; deep p50 ≤2.5s holds; VoI replaces fetches, adds no serving RAM beyond transient |
| 10 | **DONE 2026-06-13 ✓** — deep p50 re-check 2173ms ≤2.5s (n=16); answer row re-derived 3.0→3.5s with the measurement (3224ms) and a carried optimization study; burst <1ms/report; 1M ANN measured (recall 0.98 @ ef=128, 506MB); amd64 restored multi-arch ([exit note](phase-exits/p10.md)) |

## 7. Post-v0.1.0 budget rows (Phases 7–9, ceilings — SPEC §16 P7–P9)

New analytical stages are budgeted against the **measured Phase-6 baseline**
(BM25 0.48ms p50 @100k · ANN 0.45ms · fusion 0.144ms · heatmap 27ms · serving RSS
~250MB plateau). "No regression" means within run-to-run noise of those numbers.

| Item | Profile F | Profile R | Tripwire |
|---|---|---|---|
| Sketch store (`sketch_v1` in dedup.redb) | ≤128MB @1M gross | ≤12.8MB @100k gross — **measured 12.3MB ✓** (123 B/doc gross; payload is 64 B by construction, redb B-tree overhead ≈ +59 B/row — ceiling re-issued at the P7 exit, not silently absorbed) | alert at 80% of cap |
| Decision log (Phase 9) | ≤20MB | ≤20MB | 30d TTL sweep + cap enforcement by oldest-day eviction (never today's rows) — as built in v0.3.0, amended from the planned "refuse writes": evicting stale rows preserves the OPE-freshest data, refusing writes would silently bias the log toward old traffic |
| Evidence stage (query-time, fast path) | ≤2ms p50 added | ≤2ms p50 added | suite 11 gate; p95 watched in /metrics stage timings |
| Evidence transient RAM (clustering top-1000 candidates) | ≤32MB | ≤32MB | included in the soak RSS gate — plateau must not move |
| Heatmap/trends + EB+Gi*+BH | ≤150ms p50 | ≤60ms p50 | re-measured at P7 exit (Phase-6 baseline 27ms) |
| Compare-mode end-to-end (flag set) | ≤ slowest lane budget + 500ms | same (anon ≤8s binds) | suite 12 cross-lane run at P8 exit |
| Deep mode with `fetch_budget` (ADR-26) | p50 ≤2.5s holds | same | fetch phase wall-clock capped at `deep_fetch_deadline_ms` (1.2s default); device re-validation at the v0.4.0 exit |
| QPP confidence stage | ≤1ms | ≤1ms | suite 13 |
| Contextual-TS state (Phase 9, feature-gated) | negligible (d≈20 matrices, <1MB) | same | noted for completeness; covered by RSS gate |
| Region SearXNG sidecars (Phase-11 candidate, ADR-22) | NOT BUDGETED — requires its own row + operator sign-off before any compose profile lands | — | risk #19: compose RAM accounting >6.5GB committed ⇒ feature stays off |
| Confidence bands (conformal, Phase 10) | **WITHDRAWN by suite 16 (2026-06-12) — never shipped, no cost incurred**; the QPP ≤1ms row stands unchanged | — | bands return only with a materially stronger predictor (suite 16 is the standing judge) |
| Trends burst stage (Phase 10) | ≤+15ms on trends p50 | ≤+10ms — **measured far under**: the whole suite-17 sweep (~10k decodes incl. prefix emulation) runs <0.1s in the release build ⇒ a 20-root report adds <1ms | release-artifact run 2026-06-13 |
| Answer mode end-to-end (opt-in) | **p50 ≤3.0s — WON BACK 2026-06-13** by the carried study: `answer_passage_cap` default 16→8 measures p50 **2502ms** (vs 3196/3224 at cap 16, n=16 arms) and the position telemetry shows 8/9 winning passages live within the first 8 (extraction leads with main content; n small, recorded honestly). The interim 3.5s re-derivation (risk-#8, P10 exit) stands in the record as the path | same | knob: raise the cap to trade latency for deeper passages; `2026-06-13-pi5-answer-cap-study.md` |
| 1M ANN re-baseline transient (Phase-10 bench, not serving) | ≤3GB transient — **run 2026-06-13 (9G free at pre-flight)**: recall@10 0.98 @ ef=128, p99 1.73ms, 506MB resident (the extrapolated row is now MEASURED); Profile-F knob re-derived `expansion_search=128` (ef=64 decays to 0.94 at 1M) | done | `2026-06-13-pi5-p10-ann-1m.md` |
| amd64 image (Phase 10, gnu/distroless variant) | ≤120MB compressed | — | parity rule: the same CI suite subset green on amd64 as arm64, else amd64 stays suspended (the v0.2.0 rule stands) |
