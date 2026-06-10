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
| /geo/heatmap | ≤150ms | ≤100ms | Phase-5 bench |
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
| 1 | disk <3GB (F) / <1GB (R); RSS <1.2GB; local p50 <50ms |
| 2 | vectors ≤600MB disk / ≤520MB RAM @1M (F, by extrapolation); p50 ≤80ms |
| 3 | deep p50 ≤2.5s; rerank cache hit ≤100ms |
| 4 | anon p50 ≤8s; Arti RSS delta ≤150MB; zero direct-lane egress under Arti-down |
| 5 | analytics ≤700MB steady; heatmap ≤150ms; every store under its cap |
| 6 | 24h soak: RSS slope <1MB/h, temp <80°C, p99 stable |
