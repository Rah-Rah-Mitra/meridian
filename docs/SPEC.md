# MERIDIAN SEARCH PLATFORM — FULL IMPLEMENTATION PROMPT (PLAN-FIRST) — v2.3

> Paste this entire document as the opening prompt to your implementation agent (e.g., Claude Code).
> It encodes all architecture decisions, resource budgets, optimization requirements, the egress &
> anonymity subsystem (Tor via Arti, WireGuard regional vantage points), the client/data privacy
> guardrails (§13.4–§13.5), and the phased plan.
> The agent MUST complete the Planning Protocol (§1) and get sign-off before writing production code.

---

## 0. ROLE AND MISSION

You are the principal engineer implementing **Meridian**: an open-source, geo-aware,
privacy-conscious **edge search appliance** written in **Rust**, deployed on a
**Raspberry Pi 5 (8GB RAM, 8GB free disk)** via Docker Compose, scaling upward to mini PCs and
edge servers **without redesign**.

Meridian is a **metasearch + selective local index + geo analytics node**. It is NOT a whole-web
crawler/indexer. Its five jobs:

1. **Route** queries across upstream engines (via a SearXNG sidecar) and local indexes under strict
   time/budget constraints.
2. **Maintain a hot local hybrid index** (lexical BM25 + dense vectors) over the documents the
   operator cares about (ingested via API, fetch ladder, or feeds).
3. **Enrich and rank** results with place/region metadata (H3 cells) and a multi-stage ranking
   pipeline whose signals are always exposed (explainability).
4. **Compute analytics** over topic diffusion and regional attention (GDELT-derived H3 rollups,
   trends, source-graph PageRank).
5. **Offer selectable egress lanes per request** — `direct` (default), `region:<id>` (WireGuard
   vantage point), and `anon` (Tor via embedded Arti) — with fail-closed semantics and a documented
   threat model. Anonymity is a first-class, opt-in capability, never a silent default.

Cross-cutting and non-negotiable: **the operator and any end-users are not tracked or profiled by
Meridian itself** (the same posture SearXNG takes for its users, extended to the whole stack). The
client/data privacy guardrails in §13.4–§13.5 — no query logging by default, client IPs never
persisted, strict retention with an operator deletion path, and zeroized secrets — are as binding as
any functional requirement. Network anonymity (§12) protects users from *upstreams and observers*;
the privacy guardrails protect users from *Meridian's own logs, metrics, and storage*. Both must hold.

All architecture decisions below are FINAL unless your planning phase surfaces a blocking
contradiction, in which case you must raise it with evidence before deviating.

---

## 1. PLANNING PROTOCOL (MANDATORY, BEFORE ANY CODE)

Produce, in order, as committed markdown docs in `docs/plan/`:

1. **`00-adr.md`** — Architecture Decision Records: restate every locked decision in §3–§5 in your
   own words, confirm feasibility, flag contradictions. One ADR per decision with status
   (CONFIRMED / CHALLENGE: <evidence>). Mandatory ADRs include: kernel page size (4KB vs 16KB),
   musl-vs-gnu target for `ort`, extraction library bake-off plan, and the Arti SOCKS-vs-embedded
   integration choice (§12.2).
2. **`01-wbs.md`** — Work Breakdown Structure: every crate, module, and task mapped to the phases
   in §16, with dependency edges, estimated effort, and the file paths you will create.
3. **`02-budgets.md`** — Your computed disk/RAM/CPU/latency budgets per component (start from §6;
   refine, never exceed).
4. **`03-risk-register.md`** — Top 15 risks with likelihood, impact, mitigation, and a tripwire
   metric each (e.g., "RSS of meridiand > 2.8GB → shed rerank stage"; "anon lane error rate > 30%
   → surface degraded banner, never fall back to direct").
5. **`04-bench-plan.md`** — The on-device benchmark suite (§15) you will run FIRST, because all
   latency targets are estimates until measured on the Pi 5.
6. **`05-threat-model.md`** — STRIDE-lite pass over the API surface, fetcher, and the three egress
   lanes; explicitly enumerate what `anon` mode does and does not protect against (§13.3).
7. **Up to 8 clarifying questions** for the operator (corpus domain, expected QPS, internet
   exposure, which WireGuard endpoints exist, whether .onion fetching should ever be enabled).
   Do not block on answers; state defaults you'll assume.

Only after these artifacts exist may you scaffold code. Re-validate budgets at every phase exit.

---

## 2. HARD CONSTRAINTS (NON-NEGOTIABLE)

### Hardware / OS
- Raspberry Pi 5: 4× Cortex-A76 @ 2.4GHz (ARMv8.2-A, NEON, dotprod), **8GB LPDDR4X**, PCIe Gen 2 ×1
  to NVMe SSD. **Active cooling is assumed present**; implement thermal backpressure anyway (§8.6).
- OS: Raspberry Pi OS Bookworm 64-bit (aarch64). **WARNING: defaults to a 16KB-page kernel on
  Pi 5.** Either (a) document switching to the 4KB kernel (`kernel=kernel8.img` in
  `/boot/firmware/config.txt`) as the supported configuration, or (b) ensure every allocator/mmap
  dependency (mimalloc, usearch, anything pulling jemalloc) is built/configured for 16KB pages.
  Pick ONE in your ADR and test it.
- Power envelope: ~3W idle, ~9W all-core load, ~12W peak with NVMe. Design for sustained ≤10W.

### Disk
- **Total free space available: 8GB. Everything must fit: Docker images, models, indexes, caches,
  analytics, Tor directory cache, logs, and merge scratch.** Budgets in §6.1 are ceilings with
  tripwires. Target local corpus: **0.5–1M documents**.
- **NEVER compile Rust on the Pi.** All builds are cross-compiled (§14). No build caches,
  no toolchains, no source trees on the device.
- No raw HTML/document-body retention. Parse → extract → index → discard (keep content hash only).

### Software
- Language: **Rust** (stable, latest). Core is a **single process** (`meridiand`) — in-process
  function calls beat microservice IPC on 8GB RAM; Docker is for packaging/isolation, not
  decomposition.
- Permissive licensing for the core (MIT OR Apache-2.0). **SearXNG (AGPL-3.0) runs as a
  network-isolated sidecar container, accessed ONLY over HTTP/JSON.** No AGPL code linked or
  vendored. Arti is Apache-2.0/MIT dual-licensed — acceptable in-core — but **exclude any optional
  LGPL-licensed Arti feature crates**; enforce via `cargo-deny`.
- No JVM, no Python in the runtime path (Python allowed offline for model training/quantization).
- Lawful posture: respect robots.txt **on every egress lane including Tor**, per-domain rate limits
  enforced globally across lanes, honest User-Agent with contact URL on direct/region lanes.
  No stealth, no proxy rotation for evasion, no captcha bypass. WireGuard vantage points are for
  lawful region-vantaged retrieval (seeing what a query returns from another locale), not for
  circumventing legal restrictions. Cloudflare Radar API data is CC BY-NC 4.0 — feature flag,
  segregated from any commercial path.

### Out of scope (do not implement, do not scaffold)
- AI accelerator offload of any kind; vision/multimodal search; OCR.
- ColBERT/multi-vector retrieval; SPLADE/learned-sparse; GNNs; per-user personalization;
  RL ranking.
- Self-hosted Nominatim/planet OSM — use remote geocoding APIs + persistent cache.
- Elasticsearch/OpenSearch/Vespa/Weaviate/Milvus; any standalone vector DB server in v1.
- Browser automation (the "Browser" rung of the fetch ladder) — stub the interface, defer the
  engine.
- Running Meridian itself as a Tor onion service — note as a possible future flag, do not build.

---

## 3. LOCKED ARCHITECTURE (SUMMARY)

**Pattern: maximum performance-per-watt hybrid search with explainable multi-stage ranking, a
pluggable egress layer, and a clean upgrade seam to object-storage segments for the edge-server
tier later.**

| Concern | Decision | Why (one line) |
|---|---|---|
| Lexical index | **Tantivy** (library, in-process) | Rust-native, BM25 + block-WAND, mmap immutable segments, MIT |
| Dense embeddings | **model2vec** static embeddings, `potion-base-8M` (256-dim) via **`model2vec-rs`** | ~92% of MiniLM quality at >30k sentences/s on CPU, ~30MB, no attention compute |
| ANN index | **USearch** (`usearch` crate, NEON+int8 native); fallback `hnsw_rs` (pure Rust) if FFI/page-size issues | int8 scalar quantization, RAM-resident HNSW, serializable, view-from-disk option |
| Fusion | **Reciprocal Rank Fusion**, k=60 | Score-scale agnostic, no tuning, proven lift |
| LTR | LightGBM **trained offline → ONNX → `ort`** | No C++ ARM bindings at runtime; one inference runtime for everything |
| Rerank (opt-in, `mode=deep`) | `ms-marco-MiniLM-L-6-v2` cross-encoder, **INT8 ONNX** via `ort`, top-20 only, Moka-cached | Largest quality jump; too slow for the default path on A76 (~0.5–2s/20 pairs est.) |
| Intent/lang | `whichlang` (lang-ID) + tiny GBDT intent classifier (ONNX) | µs-scale, no neural cost |
| Geo | **H3 via `h3o`**; res-7 cell IDs as Tantivy u64 fast fields; remote geocoder + `redb` cache; GeoNames gazetteer fst | Hierarchical prefilter, heatmaps, trend rollups; no spatial server |
| Metasearch | **SearXNG sidecar** (Docker, internal network); optional second instance `searxng-anon` proxied through Tor | 200+ engine adapters maintained upstream; AGPL isolated; per-request proxy switching is impossible in one instance |
| **Egress lanes** | **`meridian-egress` crate**: `direct` (default) · `region:<id>` (WireGuard host interfaces + source-IP policy routing) · `anon` (embedded **Arti**, SOCKS isolation per request) | Per-request selectable privacy/vantage with fail-closed semantics (§12) |
| **Privacy guardrails** | **`meridian-privacy` crate**: no-log-by-default tracing redaction · salted/truncated client-IP hashing for rate-limit only · retention TTLs + `POST /v1/forget` deletion · `secrecy`+`zeroize` for keys/tokens · header/fingerprint minimization | Operator/users not tracked or profiled by Meridian itself (§13.4–§13.5) |
| Cache | **Moka** in-process (weighted, TTL/TTI) + `redb` persistent KV (geocode, dedup, frontier, bandit stats) | No Redis container |
| HTTP/runtime | Tokio + Axum + Reqwest + Rustls + tower middleware | Standard, lean |
| Analytics | GDELT 15-min slices → H3×topic×day counters in `redb`/zstd, 90-day TTL; `petgraph` PageRank/Louvain nightly | Bounded disk, no raw corpus retention |
| Tokenization | Tantivy default + lowercase + en stemmer; **`lindera` (CJK) behind cargo feature** — affordable at 8GB (~70MB dicts) but ships OFF by default | Operator opt-in |
| IP-region enrichment | `maxminddb` + GeoLite2 (optional feature, ~70MB, operator supplies DB — license requires their own account; never redistribute) | Analytics enrichment + region-lane verification |
| Scale-out seam | All index access behind a `SegmentStore` trait; v1 = local NVMe; v2 = object storage (Quickwit-style) on edge servers | Scale without redesign |

### Query pipeline (fast mode)
```
            ┌──────────────────────────────────────────────────────────────┐
 query ──▶ normalize ──▶ intent+lang ──▶ planner (budget, geo, mode,       │
            │                            EGRESS LANE selection)            │
            │                               │                              │
            │                ┌──────────────┼────────────────┐             │
            │                ▼              ▼                ▼             │
            │        Tantivy BM25     USearch ANN     SearXNG fan-out      │
            │        top-1000         top-200         via lane:            │
            │        (block-WAND,     (model2vec       direct → searxng    │
            │         H3 prefilter)    query vec)      anon   → searxng-   │
            │                │              │           anon (Tor SOCKS)   │
            │                └──────┬───────┘               │              │
            │                       ▼                       │              │
            │                  RRF (k=60) ◀─────────────────┘              │
            │                       ▼                                      │
            │            GBDT LTR re-score top-100 (fast-field features)   │
            │                       ▼                                      │
            │     [mode=deep only] INT8 cross-encoder top-20 (cached)      │
            │                       ▼                                      │
            │        domain-diversity (MMR-lite) ──▶ results + facets      │
            │                                        + rank_signals       │
            └──────────────────────────────────────────────────────────────┘
 Direct-lane local search is unaffected by lane choice: lanes govern NETWORK egress only.
```

### Ingest pipeline
```
 source (API / feed / search-result click-through)
   ──▶ fetch ladder: [cache] → [HTTP GET on requested lane, SSRF guard, robots, rate-limit]
       → [readability extract]
   ──▶ clean text ──▶ lang-ID ──▶ dedup (blake3 in redb) ──▶ geo-tag (gazetteer fst →
       geocode cache → H3 res-7 cell) ──▶ model2vec embed (int8) ──▶ Tantivy doc + USearch vector
   ──▶ DISCARD raw body (store: url, title, snippet ≤240 chars zstd, fast fields)
```

---

## 4. CONTAINER TOPOLOGY (DOCKER COMPOSE)

Minimal microarchitecture — two runtime containers by default, a third behind the `anon` profile:

| Service | Image | Size budget | RAM cap (cgroup) | CPU | Networks | Profile |
|---|---|---|---|---|---|---|
| `meridiand` | `FROM scratch` + musl static binary + CA certs + models baked in | **≤120MB** (binary ~25MB + models ~80MB) | `mem_limit: 3g` | 4 cores | `front`, `back` | default |
| `searxng` | `searxng/searxng` pinned by digest, trimmed `settings.yml` (≤30 engines) | ~350MB | `mem_limit: 512m` | `cpus: 1.5` | `back` + egress | default |
| `searxng-anon` | same image, second config: `outgoing.proxies: socks5h://meridiand:9150`, no direct egress route | shares layers (~0 extra) | `mem_limit: 384m` | `cpus: 1.0` | `back` ONLY (`internal`) | `anon` |

Rules:
- `front` exposes only `meridiand:8080` to the host. SearXNG UIs are NEVER published.
- `searxng-anon` lives on an `internal: true` network — its ONLY path to the internet is the Tor
  SOCKS port that `meridiand` serves on `back` (fail-closed by topology: if Arti is down, the anon
  metasearch path has no egress at all).
- Both/all containers: `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, tmpfs `/tmp`,
  named volume `/data` (meridiand) on NVMe.
- Log driver: `json-file`, `max-size: 10m`, `max-file: 3` everywhere.
- Optional `caddy` (TLS) only if the operator exposes the node; default off.
- **Region lanes deployment note:** binding to host WireGuard interfaces requires `meridiand` to
  run with `network_mode: host` (a documented alternative compose profile `regions`). The default
  bridge-network profile supports `direct` + `anon` lanes only. Document the tradeoff; never
  require host networking for users who don't use region lanes.
- Healthchecks on every service; compose `depends_on: condition: service_healthy`.

---

## 5. RUST WORKSPACE LAYOUT (SUBMODULES)

Cargo workspace, one binary, thirteen library crates. Crate boundaries == module ownership; all
cross-crate APIs are plain traits / `async fn` in traits; no dyn-heavy plugin frameworks.

```
meridian/
├── Cargo.toml                  # workspace, shared [profile.release] (§7.1)
├── crates/
│   ├── meridian-common/        # config (figment), errors (thiserror), ids, types, telemetry init
│   ├── meridian-api/           # axum router, tower layers: timeout, concurrency-limit, rate-limit
│   │                           #   (governor), compression, request-id; /metrics, /healthz
│   ├── meridian-query/         # planner + orchestrator: intent, budgets, lane selection, fan-out,
│   │                           #   deadline+hedging (DISABLED on anon lane), RRF, MMR diversity
│   ├── meridian-index/         # tantivy: schema, IndexWriter mgmt, merge policy, snippets,
│   │                           #   fast fields, H3 range pruning, SegmentStore trait (v2 seam)
│   ├── meridian-vector/        # usearch wrapper (feature "usearch", default) | hnsw_rs (feature),
│   │                           #   int8 quantizer (per-dim scale), add/search/persist, RAM acct,
│   │                           #   binary-quantization + int8 rescore path (config, default off)
│   ├── meridian-embed/         # model2vec-rs runtime, tokenizer, batch embed, int8 out, mmap'd
│   ├── meridian-rank/          # ort: intent GBDT, LTR GBDT, feature extraction from fast fields
│   ├── meridian-rerank/        # feature "rerank": INT8 MiniLM cross-encoder via ort, batch=4,
│   │                           #   Moka cache (query_hash, doc_hash), strict 1.5s stage deadline
│   ├── meridian-egress/        # THE LANE LAYER: trait Egress { client(&LaneSpec) -> reqwest set }
│   │                           #   DirectLane · RegionLane (local_address bind to wg iface IP +
│   │                           #   documented `ip rule` policy routing) · AnonLane (Arti embedded:
│   │                           #   in-process SOCKS5 on back net for searxng-anon + per-request
│   │                           #   stream isolation via random SOCKS username, RFC1929) ·
│   │                           #   kill-switch: anon NEVER falls back to direct · lane health,
│   │                           #   bootstrap mgmt, circuit budget caps
│   ├── meridian-privacy/       # CROSS-CUTTING GUARDRAILS (§13.4–§13.5): tracing redaction layer
│   │                           #   (drops query text + IPs at info; PII scrub), salted/rotating
│   │                           #   client-IP hasher (rate-limit keys only), retention/forget jobs
│   │                           #   (TTL sweeps + /v1/forget by url/domain/content-hash), secret
│   │                           #   types (secrecy::Secret + zeroize) re-exported for wg keys/tokens,
│   │                           #   per-lane header/UA policy, secure-default config asserts
│   ├── meridian-fetch/         # reqwest via meridian-egress, SSRF guard (§13.1, lane-aware DNS),
│   │                           #   robots.txt (robotstxt), per-domain token-bucket SHARED ACROSS
│   │                           #   LANES (governor), readability extraction (Phase-1 bake-off:
│   │                           #   dom_smoothie vs readability-rs), size/time caps
│   ├── meridian-searx/         # SearXNG JSON client ×2 endpoints (direct / anon instance),
│   │                           #   engine health stats, ε-greedy bandit per intent class,
│   │                           #   normalization to common Result type
│   ├── meridian-geo/           # h3o ops (cell, k-ring, parents), gazetteer fst, remote geocode
│   │                           #   client + redb cache, heatmap aggregation, maxminddb (feature)
│   └── meridian-analytics/     # GDELT slice puller (stream-parse, never store raw), H3×topic×day
│       │                       #   counters in redb (zstd), TTL compaction, petgraph
│       │                       #   PageRank/Louvain nightly, /trends queries
│   └── meridian-eval/          # bins: eval harness (nDCG@10/MRR/Recall@100, trec qrels) +
│                               #   meridian-bench (§15) + criterion microbenches
├── bins/meridiand/             # composition root: config → init crates → axum serve;
│                               #   tokio runtime + rayon pool wiring (§7.2)
├── deploy/                     # compose.yaml (+profiles anon/regions), searxng settings ×2,
│                               #   caddy (optional), wg lane templates + ip-rule docs,
│                               #   pi-setup.sh (zram, sysctl, journald, kernel page note, NVMe)
├── models/                     # fetched by build script with pinned SHA256 — NOT committed
├── train/                      # offline Python: LTR/intent training, ONNX export, INT8 quant
└── docs/plan/                  # planning artifacts (§1)
```

Dependency rules (enforce with `cargo-deny` + workspace lints):
`api → query → {index, vector, embed, rank, rerank, fetch, searx, geo}`;
`{fetch, searx} → egress`; `analytics → {geo, fetch, common}`; nothing depends on `api`;
`common` depends on nothing internal; `egress` depends only on `common`;
`privacy` depends only on `common`; `{api, egress, fetch, analytics} → privacy` (everything that
logs, holds a secret, or stores user-derived data routes through the guardrail crate).

---

## 6. RESOURCE BUDGETS (CEILINGS WITH TRIPWIRES)

### 6.1 Disk budget — total ≤ 8.0GB on NVMe
| Item | Budget | Tripwire / enforcement |
|---|---|---|
| Docker images (meridiand + searxng, shared anon layer) | 550MB | CI fails if meridiand image >120MB; searxng pinned by digest |
| Models (baked into image or /models volume) | 80MB | potion-base-8M ~30MB + CE INT8 ~23MB + 2× GBDT ONNX <10MB + intent <2MB |
| Gazetteer fst (GeoNames cities15000 + admin) | 10MB | built offline, shipped in image |
| Tantivy index (target **1M docs**, no stored bodies) | 1.4GB | merge policy caps segment ≤256MB; alert at 1.3GB |
| USearch vector store (1M × 256-dim int8, HNSW M=16) | 600MB | ≈1M×(256+8×16+overhead) ≈ 420–520MB; alert at 560MB |
| redb KV (geocode, dedup hashes, frontier, bandit/engine stats) | 600MB | LRU-evict geocode; weekly compaction |
| Analytics rollups (H3×topic×day, zstd, 90-day TTL) | 700MB | daily TTL compaction; alert at 650MB |
| **Tor (Arti) state + directory cache** | 200MB | Arti caches consensus/microdescriptors; cap + periodic prune |
| GeoLite2 DB (optional `geoip` feature, operator-supplied) | 70MB | only present if enabled |
| Logs (journald + container) | 200MB | journald `SystemMaxUse=150M`; json-file 10m×3 |
| **Merge/ingest scratch headroom** | **≥1.0GB free at all times** | ingest pauses if free <1.0GB (`statvfs`) |
| Spare | ~2.5GB | — |

Hard rules: store per doc ONLY `{url, title, snippet≤240B zstd, fast fields}`; positions only on
`title`; body indexed with freqs (no positions); fetched bytes capped at 5MB, discarded after
extraction; never persist GDELT raw files or raw fetched HTML.

### 6.2 RAM budget — 8GB total
| Item | Budget |
|---|---|
| OS + Docker daemon + journald | ~800MB |
| searxng (cgroup) | 512MB |
| searxng-anon (cgroup, only if `anon` profile up) | 384MB |
| `meridiand` RSS (cgroup 3GB): USearch resident ~520MB · Moka 384MB (query 256 + fetch/geo 128) · ort sessions ~250MB · Arti runtime ~80–150MB (only when anon enabled) · tantivy writer heap 256MB (ingest only) · tokio/rayon/misc ~350MB | ≤2.6GB steady, 3GB hard |
| Page cache for mmap'd tantivy segments + models | remainder (~3.0–3.5GB) — a feature; do not "free" it |
| zram swap (zstd, 2GB) | safety net; alert if used >256MB |

### 6.3 Latency budgets (p50 targets on-device; estimates until §15 benchmarks run)
| Path | Budget | Notes |
|---|---|---|
| `mode=fast`, local-only, `lane=direct` | **≤80ms** | normalize+intent 2ms · BM25 ≤30ms · embed <1ms · ANN ≤20ms · RRF <1ms · LTR ≤10ms · render 5ms |
| `mode=fast`, metasearch, `lane=direct` | ≤900ms | local ‖ SearXNG deadline 800ms (hedged, partial OK) |
| `mode=fast`, metasearch, `lane=anon` (Tor) | **≤8s, no hedging** | Tor adds seconds; raise deadline to 6s, expect partials; NEVER hedge onto direct |
| `mode=deep`, `lane=direct` | ≤2.5s | + cross-encoder top-20 ≤1.5s (cache hit +0ms) |
| `/geo/heatmap` | ≤150ms | fast-field scan + H3 rollup |
| Ingest throughput | ≥50 docs/s sustained, ≥200 burst | fetch+extract bound; embed >30k/s is never the limit |

---

## 7. CPU MAXIMIZATION SPEC

### 7.1 Build flags (workspace `Cargo.toml` + CI)
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```
- `RUSTFLAGS="-C target-cpu=cortex-a76"` for the aarch64 release build (NEON+dotprod
  autovectorization). Keep a generic CI build for x86 tests.
- Target `aarch64-unknown-linux-musl` for the static binary; verify `ort` links (musl+ort is the
  known friction point — fallback is the `gnu` target on a distroless base, still ≤80MB image).
  **Note:** Arti + musl is generally fine (pure-Rust TLS via rustls); confirm in the build ADR.

### 7.2 Runtime threading model (the most important CPU decision)
- **Tokio multi-threaded runtime, `worker_threads = 4`** — IO-bound work only (HTTP, SearXNG
  fan-out, fetch, Arti). **Never run >100µs of CPU work on a tokio worker.**
- **One global rayon pool, 4 threads, `nice 5`** for CPU-bound stages (BM25 collection, ANN search,
  RRF, LTR feature extraction, rerank). Bridge with `oneshot`; `spawn_blocking` only for fs ops.
- **Arti runs on the tokio runtime** (it is async/Tokio-native) — its crypto is light relative to
  network latency; do not give it a dedicated pool.
- **ort sessions:** `intra_op_threads=4, inter_op_threads=1`, one shared `Environment`; rerank
  batch=4 (verify in bench).
- **Ingest vs query isolation:** ingest at nice 10, max 2 rayon tickets; query always preempts.
  Tantivy `IndexWriter` `num_threads=2`, 256MB heap.
- Per-query concurrency limit (tower): 8 in-flight; queue 50ms → 429. Pi 5 saturates ~10–20 hybrid
  QPS; protect the tail. **Anon-lane queries get a separate, smaller concurrency budget (2)** so
  slow Tor circuits cannot exhaust the shared pool.
- Verify NEON vectorization of the int8 distance kernel with `cargo asm` (usearch ships aarch64
  SIMD — confirm the dotprod path is active).

### 7.3 OS-level (`deploy/pi-setup.sh`)
- cpufreq governor `schedutil`; rely on boost (no `performance` pin — thermal/power).
- `vm.swappiness=100`, `vm.page-cluster=0`, `vm.dirty_background_ratio=5`, `vm.dirty_ratio=15`.
- zram 2GB zstd; disable SD swapfile; `gpu_mem=16`.

---

## 8. MEMORY OPTIMIZATION SPEC

1. **Global allocator: mimalloc** (secure mode off); confirm 16KB-page compatibility per the
   kernel ADR; verify usearch mmap alignment.
2. **Vectors:** int8 scalar quantization at write (per-dim scale stored once); 256-dim. HNSW
   `M=16, ef_construction=128, ef_search=64`. At >1.5M docs switch to binary quantization + int8
   rescore of top-200 (config flag; implement in Phase 2, default off).
3. **Tantivy:** mmap dir; `madvise(MADV_RANDOM)` on postings, `MADV_WILLNEED` warm fast-field
   columns at startup. Fast fields: `h3_r7:u64`, `ts:u64`, `domain_hash:u64`, `lang:u8`,
   `quality:f32→u64`. Only stored field: the zstd snippet block.
4. **Moka:** weighted by serialized bytes. Query cache 256MB (TTI 15m, TTL 2h); fetch/extract 96MB
   (TTL 24h); geocode in redb + 32MB Moka read-through. **Anon-lane results are NEVER written to
   the shared query cache** (avoid cross-lane leakage / cache-timing inference) — use a separate
   ephemeral 32MB anon cache with TTL 5m.
5. **Zero-copy discipline:** `bytes::Bytes` for bodies; `&str` + `smallvec` in tokenization;
   `clippy::redundant_clone = deny`; cache values pre-serialized as `Bytes`.
6. **Backpressure & shedding (ordered):** RSS >2.5GB → disable rerank; >2.7GB → halve Moka;
   >2.9GB → pause ingest + `mode=fast` only. Temp >78°C → pause ingest; >82°C → local-only
   (shed all metasearch fan-out). Free disk <1.0GB → pause ingest + compact. **Arti bootstrap
   failure or anon error-rate >30% → mark anon lane DEGRADED, surface to caller, never auto-fallback
   to direct.** All shed states in `/metrics`.
7. **Leak guards:** 24h soak in Phase 5 with RSS slope assertion (<1MB/h) including an anon-lane
   load component (Arti circuit churn is a classic leak source).

---

## 9. DISK OPTIMIZATION SPEC

1. Tantivy `LogMergePolicy`, `max_merged_segment=256MB`; merge when ingest idle >60s OR nightly;
   one merge at a time (PCIe Gen2 tail). Transient merge space ≈ largest segment → 1.0GB scratch
   floor.
2. Docstore zstd; 240-char snippet generated at ingest so queries never need the body.
3. redb: `geo.redb`, `dedup.redb`, `analytics.redb`, `egress.redb` (bandit + lane stats); weekly
   compaction; dedup keys = blake3(content) 16B.
4. Analytics retention: `(h3_r5, topic_id, day)` counters; daily job drops >90 days, re-zstds cold
   blocks. Never store GDELT raw.
5. Arti state dir capped (200MB); prune stale consensus on start; place on `/data` volume.
6. Models fetched at build with pinned SHA256; device never downloads at runtime.
7. All mutable state in ONE `/data` volume → `tar`+zstd backup/restore.

---

## 10. API CONTRACT (v1, JSON over HTTP)

| Endpoint | Params / body | Returns |
|---|---|---|
| `GET /v1/search` | `q` (req), `mode=fast\|deep`, `scope=local\|web\|both`, **`lane=direct\|region:<id>\|anon`** (default `direct`), `limit≤50`, `lang`, `lat,lon,radius_km` OR `h3`, `after,before`, `domain` | `{results:[{url,title,snippet,score,rank_signals:{bm25,ann,rrf,ltr,ce?},h3?,ts?,source}], facets, timings:{stage_ms}, lane:{requested,effective,degraded?}, degraded:[flags]}` |
| `POST /v1/ingest` | `{url}` or `{text,url?,title?,ts?,lat?,lon?}`; batch ≤100; optional `lane` for the fetch | `{accepted,deduped,queued}` 202 |
| `GET /v1/fetch` | `url`, `extract=true`, `lane` | clean text + metadata (ladder cache→GET→extract) |
| `GET /v1/geo/heatmap` | `q?`, `res=3..7`, `window=7d` | `{cells:[{h3,count,score}]}` |
| `GET /v1/trends` | `topic?`, `h3?`, `window` | time series + top movers |
| `GET /v1/lanes` | — | `[{id,kind,status,bootstrap%,last_error?,exit_region?}]` — lane observability |
| `POST /v1/forget` | `{url}` \| `{domain}` \| `{content_hash}`; `purge_caches=true` | data-deletion: removes matching docs from Tantivy + USearch, drops cached entries, tombstones the content hash so re-ingest is refused until cleared; returns `{removed, caches_purged}` (operator deletion path; requires bearer) |
| `GET /healthz`, `GET /metrics` | — | liveness; Prometheus text (aggregate-only, no per-query/per-IP labels — §13.4) |

Conventions: every response carries `x-request-id`, per-stage `timings`, populated `rank_signals`
(explainability), and a `lane` block stating requested vs effective lane (so a degraded anon lane
is never silently served over direct). Errors RFC7807. Auth: static bearer for mutating endpoints;
**anon-lane search may be required to present the bearer too** (config) to prevent the node being
used as an open Tor search proxy. **Privacy-by-default (§13.4): the request/response cycle stores no
query text and no client IP; `x-request-id` is random per request and is not a session/user
identifier; no `Set-Cookie`, no tracking headers; CORS is closed by default.**

**API evolution (v2.2 / ADR-20):** the contract stays `/v1` and evolves by **additive optional
blocks only** (`evidence`, `confidence`, `divergence`, `analysis` — Phases 7–9). Each block carries
its own `schema` integer for intra-block versioning; absent block = feature off/unavailable, never
an error. Removing or re-typing an existing field requires `/v2`.

---

## 11. RANKING & RETRIEVAL DETAILS

- **BM25:** tantivy defaults (k1=1.2, b=0.75); `title^2.0, body^1.0`; top-1000 collector. H3 geo
  constraint applied as a fast-field set filter (k-ring of target cell at res 5–7 via
  `h3o::grid_disk`) BEFORE scoring.
- **ANN:** query embedded by model2vec (mean-pooled static vectors, L2-norm, int8); cosine via
  usearch; top-200, `ef_search=64`.
- **RRF:** `score(d)=Σ 1/(60+rank_i(d))` over {bm25, ann, each searx engine list}; top-100.
- **LTR features:** rrf_score, bm25_score, ann_sim, title_match_ratio, freshness (exp-decay over
  `ts`), domain_prior (PageRank, precomputed), geo_distance_km, source_count, snippet_len. Model:
  LightGBM → ONNX, ≤200 trees, depth ≤6 (<0.1ms/doc). Cold-start = hand-tuned linear weights behind
  the same ONNX interface (single Gemm).
- **Deep rerank:** top-20, truncate to 256 tokens, batch 4, INT8, cached; 1.5s stage deadline →
  on overrun return LTR order with `degraded:["rerank_timeout"]`.
- **Diversity:** cap 3 results/domain (greedy MMR-lite with domain penalty).
- **Metasearch routing:** ε-greedy bandit (ε=0.1) over engine subsets keyed by intent class;
  reward = appeared-in-final-top10; arms persisted in redb. **Hedging only on the direct lane**;
  on anon, fire a single engine subset and accept partials.

---

## 12. EGRESS & ANONYMITY SUBSYSTEM (`meridian-egress`) — EXPANDED

The single most security-sensitive subsystem. Build it test-first; no lane ships without the
fail-closed tests below passing.

### 12.1 Lane model
```
trait Egress {
    /// Returns a configured reqwest::Client (or per-request builder) bound to this lane.
    async fn client(&self, spec: &LaneSpec) -> Result<LaneClient, EgressError>;
    fn status(&self) -> LaneStatus;   // Up / Bootstrapping(pct) / Degraded(reason) / Down
}
enum Lane { Direct, Region(RegionId), Anon }
```
Lanes govern OUTBOUND NETWORK ONLY. Local index/vector/rank stages are lane-independent. The
planner resolves the requested lane to an effective lane and records both in the response; a
requested `anon` that cannot be satisfied returns `degraded` + a clear error, never a direct-lane
result.

### 12.2 `direct`
Plain Reqwest+Rustls client, shared connection pool, the only lane that may hedge. Honest
User-Agent. This is the default and the only lane enabled out of the box.

### 12.3 `region:<id>` (WireGuard vantage points — lawful region-vantaged retrieval)
- Operator brings their own WireGuard endpoints (their VPS in other regions, or a provider they're
  entitled to use); Meridian does not ship or recommend evasion endpoints.
- Implementation: each region is a host `wg` interface with its own source IP. `meridiand` (host
  network profile) binds outbound sockets to that source IP via Reqwest `local_address(IpAddr)`,
  backed by Linux `ip rule`/`ip route` policy routing tables documented in
  `deploy/wg-lane-templates/`. Provide the exact `ip rule add from <src> lookup <table>` recipes;
  do NOT auto-mutate host routing — generate scripts the operator reviews and runs.
- Verification: on lane bring-up, fetch a known IP-echo endpoint through the lane and assert the
  observed egress IP/region matches expectation (uses the optional GeoLite2 DB if present);
  mark Degraded on mismatch. This catches routing leaks.
- robots.txt and per-domain rate limits apply identically. Region lanes are for *observing*
  region-varied results, not for hammering geo-restricted endpoints.

### 12.4 `anon` (Tor via embedded Arti)
- **Arti embedded in-process** (`arti-client` + `tor-rtcompat` on Tokio). On startup (only if anon
  enabled), bootstrap a Tor client; expose an **in-process SOCKS5 listener on the `back` network**
  for `searxng-anon` to use as its sole upstream proxy (`socks5h://` so DNS resolves through Tor —
  prevents DNS leaks).
- **Per-request stream isolation:** issue each logical search/fetch on an isolated circuit. With
  Arti, use isolation tokens (`StreamPrefs`/`IsolationToken`) for in-core fetches; for the SearXNG
  path, vary the SOCKS5 username per upstream request (RFC1929 username-based isolation) so distinct
  queries don't share a circuit and become linkable.
- **Fail-closed, always:** if Arti is not bootstrapped or a circuit cannot be built, the anon lane
  returns an error/degraded status. It MUST NOT, under any code path, retry the request on the
  direct lane. Enforce with a type-level guard (an `AnonClient` that simply has no direct
  transport) plus an explicit test that injects Arti-down and asserts zero direct-lane egress.
- **.onion fetching:** OFF by default behind a separate `allow_onion` flag; when off, reject
  `.onion` URLs on every lane.
- **No hedging, no parallel duplicate circuits** for the same request (duplicates harm both
  anonymity and Tor network health). Single attempt, generous deadline (§6.3).
- **Cache isolation:** anon results use the ephemeral anon cache only (§8.4); never written to the
  shared query cache; never used to warm the LTR click-prior or bandit reward tables (which would
  leak anon behavior into shared state).
- **Rate-limit citizenship:** cap concurrent Tor circuits (config, default 6) and respect the same
  per-domain token buckets; do not become an abusive Tor client.

### 12.5 Cross-lane invariants (assert in tests)
1. A requested anon/region lane never produces direct-lane network traffic for that request.
2. DNS for anon resolves via Tor (`socks5h`), never via the host resolver.
3. SSRF guard (§13.1) runs on EVERY lane, including anon (block .onion unless `allow_onion`; block
   private ranges always).
4. Per-domain rate limits are global across lanes (a domain can't be hit 3× faster by using 3 lanes).
5. `effective_lane` in the response always equals the lane actually used.

---

## 13. SECURITY & COMPLIANCE SPEC

### 13.1 SSRF guard (meridian-fetch, mandatory, unit-tested, runs on all lanes)
- Scheme allowlist {http, https} (+`onion` only if `allow_onion`); deny non-standard ports by
  default.
- Resolve ALL A/AAAA BEFORE connect (on the direct/region lanes; on anon, the resolution happens in
  Tor — instead enforce a destination policy: reject literal private IPs and non-allowlisted ports
  in the URL, and reject `.onion` unless allowed). Reject any address in RFC1918, 127/8, ::1,
  169.254/16 (incl. cloud metadata), fe80::/10, fc00::/7, 100.64/10, 0.0.0.0/8, multicast/reserved.
  Pin the connection to the validated IP (no TOCTOU re-resolution) on direct/region lanes.
- Max 3 redirects, each re-validated; response cap 5MB streamed; total timeout per lane (10s direct,
  30s anon); connect 3s direct.
- Fetched content is DATA, never instructions: if ever forwarded to an LLM downstream, treat as
  untrusted and strip/ignore embedded prompt-like text; document this invariant.

### 13.2 Supply chain & hardening
- `cargo-deny` (allow MIT/Apache-2.0/BSD/ISC/Zlib; **deny AGPL/LGPL/GPL in the core graph** — this
  is what keeps Arti's optional LGPL crates out and SearXNG fully external); `cargo-audit` in CI;
  `Cargo.lock` committed; base images pinned by digest; SBOM (syft) per release.
- Containers: read-only rootfs, tmpfs /tmp, `cap_drop: ALL`, `no-new-privileges`, non-root UID.
- Public-endpoint rate limit (governor 5 rps/IP burst 20); body limit 1MB; strict serde
  (`deny_unknown_fields`).
- robots.txt honored (24h cache) on all lanes; per-domain budget 1 req/2s sustained; UA
  `MeridianSearch/0.x (+https://<operator-url>/bot)` on direct/region (no custom UA fingerprint on
  anon beyond a generic value).

### 13.3 Anonymity threat model (document in `05-threat-model.md`)
State plainly what `anon` does and does NOT provide. It DOES: hide the operator's source IP from
upstream engines/sites for that request; resolve DNS through Tor; isolate circuits per request.
It does NOT: defeat a global passive adversary; anonymize the *operator* to Meridian's own logs
(so anon-lane request logging must be minimized — log lane + timing + status, never the full query
string at info level for anon requests; gate query-text logging behind a debug flag with a privacy
warning); protect against the operator's own ISP seeing Tor usage; or make scraping lawful where it
otherwise isn't. The node is not an onion service and is not designed for use by untrusted third
parties unless the operator adds auth (default: bearer required even for anon search).

### 13.4 Privacy guardrails — client & data (the operator/users are not tracked or profiled)

These apply on ALL lanes, including `direct`. They are tested, not aspirational; CI includes a
"privacy smoke test" that drives traffic and greps logs/metrics for leaked query text and IPs.

**No-log by default.**
- Default log level reveals **no query text and no client IP**. The `meridian-privacy` tracing layer
  redacts `q`, full URLs of ingested/fetched content, and any `ip`/`host` field at `INFO` and above;
  a struct field carrying user input must be wrapped in a `Redacted<T>` newtype whose `Debug`/
  `Display` prints `‹redacted›`. Raw query text is available only at `TRACE` behind an explicit
  `privacy.debug_query_logging=true` config flag that logs a startup warning and is OFF in the
  shipped config and in all container images.
- Access logs (if enabled at all) record method, route, status, latency bucket, lane, and a
  coarse outcome — never the query string, never the path's query params verbatim.
- Anonymous-lane requests get the strictest treatment (per §13.3): lane + timing + status only.

**Client-IP handling.**
- Client IPs are **never persisted to disk and never logged**. They exist in memory only for the
  duration of a request and only for rate limiting.
- Rate-limit keys use `key = blake3(client_ip ‖ daily_salt)` truncated to 8 bytes; `daily_salt` is
  generated at process start and rotated every 24h (and on restart), held in a `Secret`, never
  logged, so keys cannot be reversed to IPs or correlated across days.
- `/metrics` exposes only aggregate counters/histograms with **no per-IP, per-user, or per-query
  labels** (cardinality is bounded by lane × route × status × latency-bucket). No exemplars carrying
  identifiers.
- If the node sits behind a reverse proxy, `X-Forwarded-For` is used only to derive the rate-limit
  key and is then dropped; it is never stored or echoed.

**Header & fingerprint minimization (outbound).**
- Outbound requests send a minimal, lane-consistent header set: no `Referer`, no cookies (cookie
  store disabled), no custom identifying headers. `Accept-Language` is a fixed, generic value per
  lane (configurable) rather than derived from the end-user, so requests in a lane are mutually
  indistinguishable.
- User-Agent policy (reconciles the §13.2 "honest UA" rule with anonymity): `direct`/`region` lanes
  send the honest `MeridianSearch/<ver> (+<operator-url>/bot)` UA so site owners can identify and
  contact the operator; the `anon` lane sends a **generic, common browser-class UA with no operator
  contact and no version drift**, because embedding contact info over Tor would defeat the lane's
  purpose. The same robots.txt and rate-limit rules still apply on anon (§12.4, §13.2).

**Data minimization & retention (stored content).**
- Store only what §6.1 permits: `{url, title, snippet≤240B, fast fields, content-hash}`. Never the
  raw body, never the full fetched HTML, never GDELT raw rows.
- Every stored class has an explicit TTL/limit with an enforcing job: query/anon caches (Moka TTLs),
  geocode cache (LRU + size cap), dedup tombstones (kept, tiny), analytics counters (90-day TTL),
  engine/bandit stats (rolling). Document each TTL in `meridian.toml` with its default.
- **Operator deletion path (`POST /v1/forget`, §10):** delete by `url`, `domain`, or `content_hash`
  → remove from Tantivy (delete term + commit), drop the USearch vector(s), purge cache entries, and
  tombstone the content-hash so the same content is refused on re-ingest until the tombstone is
  cleared. This is the GDPR/"right to be forgotten"-style hook for any operator who exposes the node
  to others, and the mechanism for honoring a site's removal request.
- Backups are of the `/data` volume only and inherit these contents; document that restoring a
  backup also restores anything not yet TTL-expired.

**No third-party telemetry / phone-home.** Meridian makes no analytics/telemetry/update calls to any
Anthropic or third-party endpoint. The only outbound traffic is: operator-configured search
upstreams (via SearXNG), fetches the operator/users request, GDELT/geocode if enabled, and Tor
directory traffic if anon is enabled. Any future opt-in telemetry must be explicit, off by default,
aggregate, and documented.

**Secure-by-default deployment.**
- `meridiand` binds `127.0.0.1` by default; exposing it on a LAN/Internet is an explicit operator
  choice that the docs gate behind "enable auth + TLS first."
- Bearer auth required for all mutating endpoints (`/v1/ingest`, `/v1/forget`) and configurable-but-
  recommended for `/v1/search` if exposed; anon search bearer-gated by default (§10).
- Optional `caddy` profile provides TLS (Let's Encrypt or operator cert); HTTP is localhost-only.
- CORS closed by default; if enabled, explicit origin allowlist only.

**Lawful-not-evasion (privacy ≠ circumvention).** The privacy and anonymity features exist to protect
the operator and end-users from tracking and profiling — NOT to evade access controls. Meridian
honors `robots.txt`, per-domain rate limits (globally across lanes, §12.5), and removal requests on
every lane. It does not rotate identities to defeat rate limits, does not brute-force geo-restricted
endpoints, and ships no stealth/anti-bot-evasion behavior. Region lanes are for *observing*
region-varied results from endpoints the operator is entitled to reach; the anon lane is for
*source-IP privacy*, not for amplifying request volume against a target.

### 13.5 Secrets hygiene

- All secret material — WireGuard private keys, the bearer token(s), the rotating rate-limit salt,
  any operator-supplied API keys (geocoder, MaxMind) — is held in `secrecy::Secret<T>` / `SecretBox`
  and zeroized on drop (`zeroize`). No secret implements `Debug`/`Display` in the clear.
- Secrets load from env or a `0600` file referenced by `meridian.toml`; **secrets are never written
  to logs, never in error messages, never in `/metrics`, never in `/healthz`, and never serialized
  into the `/data` volume.** A config-dump command redacts them.
- WireGuard keys live with the host `wg` config (operator-managed), not inside the index volume;
  `meridiand` only needs the source IP / interface name, not the private key, for the region lane.
- Best-effort `mlock` of the secrets region to keep keys out of swap **only after** validating it
  against the chosen kernel page size (§2) and the RAM budget; if `mlock` is unavailable, rely on the
  encrypted zram swap and document the residual risk. Do not `mlock` large buffers.
- CI secret-scanning (e.g., `gitleaks`) on the repo; `Cargo.lock` and images contain no embedded
  tokens; the build script fetches models over HTTPS with pinned SHA256 and injects no credentials.

---

## 14. BUILD, CI, AND DEPLOYMENT

- Cross-compile from x86_64: `cargo zigbuild --target aarch64-unknown-linux-musl` (or `cross` if
  zigbuild fights `ort`/`arti`). CI matrix: x86 test + aarch64 build + clippy(deny warnings) + fmt
  + deny + audit + the anon fail-closed test suite.
- Images via `docker buildx` multi-stage → `FROM scratch` + CA certs + binary + `/models`.
  Push `linux/arm64` to GHCR.
- Device install = `deploy/pi-setup.sh` (idempotent) + `docker compose up -d` (default profile);
  `--profile anon` adds Tor metasearch; `--profile regions` switches to host networking for WG
  lanes. Zero compilation on device.
- Config: single `meridian.toml` (figment: file + env). Every budget in §6 and every lane is a
  config key with documented defaults; anon and regions ship DISABLED.
- Versioned index format: refuse newer-major; `meridiand --reindex` migration.

---

## 15. BENCHMARK & EVALUATION HARNESS (BUILD FIRST — PHASE 0)

`meridian-bench` (single device binary, emits markdown + JSON):
1. model2vec embed throughput (batch 1/32/256). Gate: >2k/s.
2. USearch build 1M synthetic 256-d int8; RAM, p50/p99 @ ef=64, recall@10 vs exact. Gate: p99 <40ms.
3. Tantivy index 1M real docs (downloader for a permissive Wikipedia slice); docs/s, bytes,
   BM25 top-1000 p50/p99. Gate: p50 <30ms.
4. ort INT8 cross-encoder ms/pair at batch 1/4/8 → sets default rerank depth/batch.
5. RRF+LTR criterion microbench: <2ms for 1000+200 candidates.
6. Thermal: 10-min all-stage loop logging `vcgencmd measure_temp` + throttle flags.
7. Disk: merge amplification (ingest 1M, force merges, peak transient bytes).
8. **Anon-lane bench:** Arti bootstrap time, circuit-build p50/p99, end-to-end anon metasearch p50,
   anon RSS delta, and a leak test (assert no packets egress except via Tor when anon-only).

**Every latency number in §6.3 is provisional until this suite runs on-device; update
`docs/plan/02-budgets.md` with measured values and re-derive rerank depth, ef_search, QPS caps,
and anon deadlines.**

Quality eval (`meridian-eval`): 100-query labeled set (graded 0–3, trec qrels) from the operator's
domain in Phase 2; report nDCG@10, MRR@10, Recall@100 for BM25 vs hybrid vs hybrid+LTR vs deep.
Acceptance: hybrid ≥ BM25; no stage regresses.

Load test: `oha` at 2/5/10/20 rps fast-mode 10 min each; record p50/p99/error%/temp/RSS; separate
anon-lane load run at 1/2 rps.

Privacy smoke test (`meridian-eval`, runs in CI and on device): drive a scripted mix of searches,
ingests, fetches, and one anon-lane request with a distinctive canary token as the query and a
canary client IP; then assert the canary query string appears in **zero** log lines and zero metric
samples, the canary IP appears nowhere on disk or in `/metrics`, no `Set-Cookie` is emitted,
`POST /v1/forget` on a canary doc removes it from lexical + vector + cache, and an egress packet
capture shows no connection to any non-allowlisted (telemetry) host. Gate: all assertions pass.

---

## 16. PHASED IMPLEMENTATION PLAN (EXIT CRITERIA GATE EACH PHASE)

**Phase 0 — Plan, scaffold, bench (week 1–2)**
Planning artifacts (§1, incl. threat model) · workspace + CI cross-compile · `meridian-bench`
complete and RUN ON DEVICE (incl. anon bootstrap timing) · compose skeleton + healthchecks ·
pi-setup.sh.
EXIT: scratch image <120MB boots on Pi; bench report committed; budgets doc updated with measured
numbers.

**Phase 1 — Lexical MVP + direct lane (week 3–5)**
meridian-{common,api,index,fetch,searx,egress(Direct only),privacy} · BM25 search local+metasearch
with RRF · ingest pipeline (fetch ladder, dedup, snippets) · SSRF tests · extraction bake-off
decided · **privacy core: no-log tracing redaction, salted client-IP rate-limit hashing,
localhost-bind + bearer auth, `Redacted<T>` wrappers, privacy smoke test in CI.**
EXIT: 1M docs indexed; fast-mode local p50 <50ms; RSS <1.2GB; total disk <3GB; SearXNG isolated;
**logs/metrics provably free of query text and client IPs (privacy smoke test green)**;
0 clippy warnings.

**Phase 2 — Hybrid retrieval (week 6–8)**
meridian-{embed,vector} · int8 quantizer · RRF over {bm25, ann, searx} · Moka + shedding hooks ·
labeled eval set v1.
EXIT: hybrid nDCG@10 ≥ BM25; 1M-doc vectors ≤600MB disk / ≤520MB RAM; fast-mode p50 ≤80ms.

**Phase 3 — Ranking stack (week 9–11)**
meridian-{rank,rerank} · intent classifier · LTR (cold-start linear → GBDT) · deep mode + cache +
deadline · bandit engine routing.
EXIT: deep p50 ≤2.5s / cache-hit ≤100ms; LTR no regression; shed flags observable.

**Phase 4 — Egress lanes: anon + region (week 12–14)**
meridian-egress AnonLane (Arti embedded, SOCKS isolation, fail-closed) + RegionLane (WG bind +
verification) · searxng-anon compose profile · `/v1/lanes` · cross-lane invariant tests (§12.5) ·
anon cache isolation.
EXIT: all five §12.5 invariants tested green; anon metasearch returns real results p50 ≤8s;
injected Arti-down test proves ZERO direct-lane egress; region lane egress-IP verification passes.

**Phase 5 — Geo + analytics (week 15–16)**
meridian-{geo,analytics} · gazetteer fst · H3 fast fields + k-ring filter · heatmap + trends ·
GDELT puller + retention · nightly PageRank → domain_prior · **retention TTL sweeps + `POST /v1/forget`
deletion path (Tantivy delete + vector drop + cache purge + content-hash tombstone) + secrets
moved to `secrecy`/`zeroize`.**
EXIT: heatmap p50 ≤150ms; analytics steady disk ≤700MB over 2 simulated weeks; geo property tests
pass; **`/v1/forget` removes a doc from lexical+vector+cache and blocks its re-ingest (tested); TTL
sweeps hold every store under its §6.1 cap.**

**Phase 6 — Hardening + release (week 17–18)**
24h soak (10 rps direct + 1 rps anon + continuous ingest): no OOM, RSS slope <1MB/h, temp <80°C,
p99 stable, no circuit leak · chaos: kill searxng / kill Arti / fill disk / hot-loop CPU and verify
graceful, fail-closed shed · docs (operator manual, API ref, threat model, **privacy policy +
data-retention/deletion guide**) · v0.1.0 multi-arch images.
EXIT: soak green; backup/restore drill tested; security review of egress crate signed off;
**privacy review signed off (no-log/IP smoke test, secret-scan clean, `mlock`/swap decision
documented, no third-party phone-home verified via egress capture).**

**Phase 7 — Evidence foundations + statistical rigor (post-v0.1.0 → v0.2.0)**
SimHash+MinHash sketching at ingest (deletable: same txn as dedup/tombstone, ADR-18/19) · query-time
derivation clustering post-RRF → `evidence` block (`independent_source_count`, source clusters;
default-on with `evidence.enabled` kill-switch; web results `evidence: null` in v0.2.0 — snippet
sketches are too weak to assert independence honestly) · empirical-Bayes shrinkage + Getis-Ord Gi*
+ Benjamini-Hochberg FDR replacing the naive latest/mean movers in trends + heatmap stat fields
(ADR-21) · eval suites 9–11 (synfarm, spike, evidence-latency) + same-lane JSD noise-floor report
(suite 12 probe — Phase-8 entry evidence).
EXIT: synfarm pairwise F1 >0.8 AND false-merge <5% on BOTH generator variants; spike suite ≥3×
false-spike reduction at equal TPR vs the ratio baseline on the Poisson variant, with no regression
(≥1×) and held FDR on the overdispersed hold-out variant (detector constants fixed on the tuning
variant only — risk #21); evidence stage adds ≤2ms p50 to the fast
path @100k (Profile R); heatmap+Gi* ≤60ms R / ≤150ms F; ingest throughput regression ≤10% vs
Phase 6; sketch store ≤64 B/doc; serving RSS no regression vs the ~250MB R plateau;
**forget-correctness 100% including sketch removal (extended hermetic test)**; all 13 egress
invariants + privacy smoke green; noise-floor report committed.

**Phase 8 — Vantage divergence + confidence (v0.3.0)**
`compare=vantages` orchestrator: same query over {direct, anon}, each lane resolved independently
and fail-closed per §12.1, results compared post-hoc with ZERO shared state (no shared-cache write,
no bandit reward, anon cache stays ephemeral; randomized inter-lane jitter, default-on) ·
`divergence` block: Jensen-Shannon divergence over per-lane domain distributions + `domains_only_in`
+ bootstrap significance vs the Phase-7 noise floor (ADR-22: region lanes stay fetch-only; region
metasearch is a Phase-10 candidate behind operator sign-off) · QPP confidence block (NQC + Clarity,
ADR-23) · optional MMR diversity rerank (`diversity=mmr`).
EXIT: cross-lane JSD on the curated divergence set exceeds the same-lane noise floor at p<0.05
(bootstrap); ≥16 hermetic egress-invariant tests green (3 new fan-out invariants); fast path with
the flag absent: zero p50/p95 change vs Phase 7; compare-mode p50 ≤ slowest-lane budget + 500ms
(anon ≤8s holds); QPP confidence vs per-query nDCG@10 Spearman ρ ≥0.25 at ≤1ms added; MMR improves
alpha-nDCG@10 on the duplicate-heavy set with ≤1% nDCG@10 loss; **no per-query cross-lane record
persisted anywhere (canary-tested); compare responses never enter the shared cache (tested)**.

**Phase 9 — Adaptive frontier: decision log, OPE, contextual routing, VoI (v0.4.0)**
Privacy-vetted per-decision routing log (ADR-24: coarse buckets + arm + propensity + reward ONLY —
no query text, no IPs; 30-day TTL; k-anonymity floor; wipe path; **anon-lane decisions never
logged**, §12.4) · IPS/doubly-robust OPE harness (suite 14) · linear Thompson-sampling contextual
policy over the same 3 arms, **feature-gated default-off**, enabled only by the ADR-25 ship gate
(DR uplift 95% CI excludes zero on ≥10k decisions; inconclusive ⇒ ε-greedy retained, recorded at
exit) · VoI next-best-fetch + stopping in deep mode (Pandora's-box reservation values; novelty from
Phase-7 MinHash + embedding coverage; ADR-26).
EXIT: decision log provably holds zero query text / zero IPs (extended privacy smoke); TTL sweep,
wipe path, and k-anonymity guard tested; log ≤20MB; anon decisions provably absent (invariant count
≥17); OPE recovers synthetic ground truth with bias <5% (suite 14); contextual policy ships only
per the ADR-25 gate; VoI ≥25% fewer fetches at equal nDCG@10 (±1%) with non-degrading median
`independent_source_count` (suite 15); deep p50 ≤2.5s holds; suites 1–13 + forget-correctness +
RSS/latency re-validated.

**Phase 10 — Calibrated confidence, change-aware trends, answer mode (v0.5.0)**
Conformal confidence bands wrapping the Phase-8 block (ADR-27) — **WITHDRAWN by suite 16
(2026-06-12) before shipping**: the absolute coverage claim collapses 19pp under a held-out
query-style shift while only the relative lift survives, so the block keeps shipping raw
NQC/Clarity/score exactly as ADR-23 worded them (`schema: 1` unchanged); the suite-16 harness
ships as the standing judge for any stronger future predictor · change-point trends (ADR-28): two-state burst model per root-code day series surfacing
multi-day ramps the latest-day EB z structurally misses; z stays the single-day detector — burst
COMPLEMENTS, never replaces · answer mode (ADR-29): opt-in best-passage extraction over
deep+`fetch_budget`, selector = the dormant `pandora_walk` (single-best objective — its designed
regime per ADR-26), additive `best_passage` block, extractive only, inherits every fetch_budget
restriction · VoI embedding-coverage study (suite 15b: real embeddings in the replay;
amend-or-record against the frozen v0.4.0 selector on the hold-out) · amd64 image restoration
(gnu/distroless variant or upstream `__GLIBC__` guard — the portable-recall defect is still not an
option) · 1M ANN re-baseline behind a disk pre-flight (P7 carry).
EXIT: suite 16 hold-out band coverage within 5pp of target with strictly monotone band quality and
zero added latency budget (≤1ms QPP stage holds) — **FAILED 2026-06-12 on every condition ⇒ bands
withdrawn pre-ship (the gate worked; raw signals unchanged)**; suite 17 null FPR ≤ the EB-z baseline's on BOTH
variants with ramp TPR ≥ z+0.2 on tuning and ≥1.25×z on the hold-out (margin-on-tuning /
no-collapse-on-hold-out, the suite-10 pattern), delay past the 2× crossing ≤1d tuning / ≤2d
hold-out, z spike parity, held-out generator variant honored (risk #21) — **MET 2026-06-12,
s=2/γ=1 frozen**; suite 18 best-passage hit-rate ≥ the
<<<<<<< HEAD
snippet-head baseline with pandora-vs-additive fetch efficiency measured, answer-mode p50 ≤3.0s
device-validated (its own budget row — the deep 2.5s budget is not silently busted); suite 15b
ships only on a hold-out win, else the carry closes with a measured no — **CLOSED 2026-06-13
with the no: signal calibratable (0.72 vs 0.05 cosine), no swept combiner profitable**; amd64 green on the same CI
=======
snippet-head baseline with pandora-vs-additive fetch efficiency measured — **MET 2026-06-12
(+10.7pp; pandora 3.8× additive at fewer fetches; cost 0.1 frozen)** — with answer-mode p50 ≤3.0s
device-validated at the exit (its own budget row — the deep 2.5s budget is not silently busted); suite 15b
ships only on a hold-out win, else the carry closes with a measured no; amd64 green on the same CI
>>>>>>> origin/main
suite subset as arm64 or it stays suspended; 1M ANN re-baseline recorded (informational —
insufficient disk ⇒ honestly still-carried with the pre-flight measurement); hermetic invariant
count ≥20 holds; suites 1–8 + forget-correctness + RSS/latency re-validated.

**Phase 11 — candidates (recorded, not planned).** Region-lane metasearch sidecars per ADR-22
(operator sign-off + budget row first) · DP release for published aggregates (deferred until an
operator-facing publish/export feature exists — today's aggregates are unreleased derivatives of
public GDELT data, so there is no release boundary to protect yet) · binary quantization (trigger:
>1.5M docs, Phase-2 deferral stands) · crates.io publication (follows the operator's
repo-visibility decision).

---

## 17. RESOURCE COMPENDIUM

**Crates:** tantivy · usearch (fallback hnsw_rs) · model2vec-rs · ort · tokio · axum · tower /
tower-http · reqwest (rustls-tls) · rustls · arti-client + tor-rtcompat + arti-hyper · moka · redb ·
h3o · maxminddb (feature) · petgraph · whichlang · lindera (feature, off) · governor · robotstxt ·
scraper + dom_smoothie / readability-rs (bake-off) · figment · thiserror · serde · blake3 · fst ·
bytes · smallvec · rayon · mimalloc · **secrecy + zeroize (secrets) · hickory-dns (optional DoH on
direct/region lanes) · tracing-subscriber custom redaction layer** · criterion · metrics +
metrics-exporter-prometheus · tracing. Dev/CI: gitleaks (secret scan) · cargo-deny · cargo-audit ·
syft (SBOM) · oha (load).

**Models (pin SHA256 in build script):**
- `minishlab/potion-base-8M` (model2vec static, 256-dim, ~30MB) — embeddings.
- `cross-encoder/ms-marco-MiniLM-L-6-v2` → INT8 dynamic ONNX (~23MB) — deep rerank.
- Intent + LTR GBDTs trained in `train/`, exported ONNX (<12MB combined).
- Gazetteer: GeoNames cities15000 + admin1 (CC-BY) → fst (~10MB).
- GeoLite2 (optional, operator-supplied under their own MaxMind license; never redistributed).

**Data/services:** SearXNG (×2 sidecars: direct + Tor-proxied) · GDELT v2 15-min exports · GeoNames
· public Nominatim (≤1 rps, attribution) or operator endpoint · Tor network via Arti · operator
WireGuard endpoints · Wikipedia subset for benches.

**Key references for the implementer:** tantivy examples & merge-policy docs · usearch Rust docs
(quantization, view-from-disk) · model2vec cards (MTEB ~92% of MiniLM) · RRF (Cormack et al.,
SIGIR'09, k=60) · hnswlib ALGO_PARAMS (M/ef) · ort crate docs (threading, quantized ops) · h3o docs
(grid_disk, parents) · **Arti docs: arti-client usage, stream isolation / IsolationToken,
bootstrapping, embedding-Arti guide** · Tor SOCKS username isolation (RFC1929) · WireGuard +
Linux policy routing (`ip rule`) howtos · OWASP SSRF prevention cheat sheet · Pi 5 16KB-page kernel
notes · Quickwit architecture docs (v2 SegmentStore seam).

---

## 18. DEFINITION OF DONE (WHOLE PROJECT)

A clean Pi 5 goes from blank NVMe to serving hybrid geo-aware search with `pi-setup.sh` +
`docker compose up -d` in <15 minutes and <600MB downloaded; sustains 10 rps direct + 1 rps anon
fast-mode with continuous ingest for 24h inside the §6 budgets; every result is explainable via
`rank_signals` and `timings` and labels its `effective_lane`; the `anon` lane is provably
fail-closed (no direct-lane egress on Arti failure) and DNS-leak-free; **logs and metrics contain no
query text and no client IPs, client IPs are never persisted, `POST /v1/forget` provably erases a
document from lexical + vector + cache and blocks its re-ingest, all secrets are zeroized and absent
from logs/metrics/volume, and the node makes no third-party telemetry calls**; the core is
MIT/Apache-clean with AGPL (SearXNG) and any LGPL (Arti optional crates) fully fenced out by
`cargo-deny`; and migrating the index layer to object storage requires implementing one trait, not a
rewrite.
