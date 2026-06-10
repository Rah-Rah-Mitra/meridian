# 02 — Resource Budgets (Dual Profile)

> SPEC §1.3, refined from SPEC §6. **Ceilings, never targets.** Every latency number
> is PROVISIONAL until `meridian-bench` runs on-device (SPEC §15); this file is
> re-issued with measured values as the Phase-0 exit artifact and re-validated at
> every phase exit.

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
| Tantivy index | 1.4GB @1M | 150MB @100k | alert 1.3GB (140MB) |
| USearch vectors (256-d int8, M=16) | 600MB @1M | 60MB @100k | alert 560MB (56MB) |
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

## 3. CPU & latency (p50 targets — ALL PROVISIONAL until bench)

| Path | Profile F target | Profile R expectation | Bench that grounds it |
|---|---|---|---|
| fast, local-only, direct | ≤80ms | ≤60ms (smaller index) | §15.1–3, 5 |
| fast, metasearch, direct | ≤900ms | ≤900ms (network-bound) | §15.3 + load |
| fast, metasearch, anon | ≤8s, no hedging | ≤8s | §15.8 |
| deep, direct | ≤2.5s | ≤2.0s | §15.4 |
| /geo/heatmap | ≤150ms | ≤100ms | Phase-5 bench |
| Ingest sustained | ≥50 docs/s | ≥25 docs/s (SD write path) | §15.3, 7 |

Stage sub-budgets (fast/local): normalize+intent 2ms · BM25 ≤30ms · embed <1ms ·
ANN ≤20ms · RRF <1ms · LTR ≤10ms · render 5ms.

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

Measured 2026-06-10 (scaffold, 15 stub crates): check+clippy+test target = 255MB,
wall ~11s. Re-measure when tantivy enters in Phase 1; if the target dir exceeds
1.5GB, drop to check-only (no clippy --all-targets) or strict CI-only iteration.

## 6. Phase-exit budget checkpoints

| Phase exit | Must hold |
|---|---|
| 0 | image <120MB; bench report committed; this file re-issued with measured numbers |
| 1 | disk <3GB (F) / <1GB (R); RSS <1.2GB; local p50 <50ms |
| 2 | vectors ≤600MB disk / ≤520MB RAM @1M (F, by extrapolation); p50 ≤80ms |
| 3 | deep p50 ≤2.5s; rerank cache hit ≤100ms |
| 4 | anon p50 ≤8s; Arti RSS delta ≤150MB; zero direct-lane egress under Arti-down |
| 5 | analytics ≤700MB steady; heatmap ≤150ms; every store under its cap |
| 6 | 24h soak: RSS slope <1MB/h, temp <80°C, p99 stable |
