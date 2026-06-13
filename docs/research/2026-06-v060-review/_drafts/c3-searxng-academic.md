# C3 — SearXNG baseline + academic retrieval ledger

Agent: C3 (competitive review, v0.6.0). Access date: 2026-06-13.
Scope: Part 1 frames **vanilla SearXNG** (official docs only) against what Meridian's
15 Rust crates add on top of its two embedded SearXNG sidecars (ADR-11). Part 2
evaluates LEANN, ColBERTv2/PLAID, and Seismic against the measured Meridian baseline
(`01-rebaseline.md` §4): usearch HNSW int8, 1M docs, ~506MB resident, recall@10
0.98 @ ef=128, p99 1.73ms, on a Pi 5 (4× Cortex-A76, 8GB, SD-card storage).

Labels: CONFIRMED = directly supported by the cited official source.
INFERRED = derived (incl. absence-of-feature conclusions and Pi-5 extrapolations).

---

## Part 1 — Vanilla SearXNG vs Meridian's additions

### 1.1 Claim ledger

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Result fusion | SearXNG merges engine results by hashing each result, merging duplicates (keep longer content, prefer HTTPS URL), then scoring: `weight *= engines[engine].weight; weight *= len(result['positions']); score += weight / position`, summed over the positions where the result appeared; final order is descending score with category/template grouping (max 8 per group within a 20-slot window) | https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py | "results = sorted(self.main_results_map.values(), key=lambda x: x.score, reverse=True)"; merge keeps longer content: "if len(other.content …) > len(origin.content …)"; score adds `weight/position` per occurrence | CONFIRMED (official source, master @ 2026-06-13) |
| Result fusion | The fusion is purely occurrence/position/engine-weight based — no rank-fusion formula like RRF, no content/lexical/semantic relevance scoring of the result text itself | https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py | Score formula uses only engine weight, occurrence count and positions; no text-similarity term exists in `calculate_score` | INFERRED (from absence in the scoring code) |
| Ranking signals exposed | The only operator-tunable ranking signal per engine is `weight` ("Weighting of the results of this engine"); results carry `score` and `positions` fields | https://docs.searxng.org/dev/engines/enginelib.html | "Weighting of the results of this engine (weight)" | CONFIRMED |
| Ranking signals exposed | No learning-to-rank, no rerankers, no per-query adaptation: ranking knobs are static config (engine weight) only | https://docs.searxng.org/dev/engines/enginelib.html ; https://docs.searxng.org/admin/settings/settings_search.html | Neither settings nor engine docs expose any trained/adaptive ranking component | INFERRED (absence across the official docs surveyed) |
| Engine selection / health | Engine selection is static configuration (categories/engines lists per request); failure handling is reactive suspension: `ban_time_on_fail` default 5s, `max_ban_time_on_fail` default 120s, plus `suspended_times` of 3,600–1,296,000s for specific error classes (CAPTCHA, too-many-requests, …) | https://docs.searxng.org/admin/settings/settings_search.html | "Ban time in seconds after engine errors" (5); "Max ban time in seconds after engine errors" (120); `suspended_times` ranges "3,600 to 1,296,000 seconds depending on the error" | CONFIRMED |
| Engine selection / health | Per-engine `timeout` override exists; `retries` re-issues a failed request "On each retry, SearXNG uses a different proxy and source ip" — but there is no reward-driven or learned engine routing of any kind | https://docs.searxng.org/admin/settings/settings_outgoing.html ; https://docs.searxng.org/dev/engines/enginelib.html | retries: "Number of retry in case of an HTTP error. On each retry, SearXNG uses a different proxy and source ip"; timeout: "Specific timeout for search-engine" | CONFIRMED (mechanisms) / INFERRED (no learned routing — absence) |
| Proxied / Tor support | Outgoing requests can be routed through one or more proxies (round-robin per protocol) and through Tor: `using_tor_proxy` (default false, per-engine overridable); `source_ips` can spread requests over multiple interfaces/CIDRs | https://docs.searxng.org/admin/settings/settings_outgoing.html | "If there are more than one proxy for one protocol (http, https), requests to the engines are distributed in a round-robin fashion"; "Using tor proxy (true) or not (false) for all engines. The default is false and can be overwritten in the engines:" | CONFIRMED |
| JSON API | `GET/POST /search` with `q`, `categories`, `engines`, `language`, `pageno`, `time_range`, `format` (json/csv/rss), `safesearch`; **but the default `formats:` list is `- html` only** — JSON must be explicitly enabled, otherwise the instance returns 403 | https://docs.searxng.org/dev/search_api.html ; https://docs.searxng.org/admin/settings/settings_search.html | "Requesting an unset format will return a 403 Forbidden error. Be aware that many public instances have these formats disabled."; default YAML: `formats:\n  - html` | CONFIRMED |
| Local index | SearXNG maintains **no local document index of its own**. "Offline engines" are connectors to external tools (command-line, SQL/NoSQL DBs, search indexers such as Elasticsearch/Meilisearch/Solr) that the admin installs separately | https://docs.searxng.org/dev/engines/offline_concept.html | "An offline engine is an engine which does not need Internet connection to perform a search and does not use HTTP to communicate"; "If an offline engine depends on an external tool, SearXNG does not install it by default" | CONFIRMED |
| Analytics | Built-in analytics = anonymous **engine** metrics only: `enable_metrics` "Enabled by default. Record various anonymous metrics available at /stats, /stats/errors and /preferences"; optional password-gated OpenMetrics `/metrics` for Prometheus (disabled by default). No query analytics, no trends, no geo, no significance testing | https://docs.searxng.org/admin/settings/settings_general.html | "Enabled by default. Record various anonymous metrics available at `/stats`, `/stats/errors` and `/preferences`"; open_metrics "Disabled by default. Set to a secret password to expose an OpenMetrics API at `/metrics`" | CONFIRMED (metrics) / INFERRED (no richer analytics — absence) |
| Deletion / retention | There is nothing to delete: no accounts, no server-side profile, preferences live in client cookies; whether requests are *logged* is a deployment property of the operator, which is the stated reason to run a private instance. No retention/deletion controls are documented because no user-data store exists | https://docs.searxng.org/own-instance.html | Public-instance users "do not know whether their requests are logged, aggregated, and sent or sold to a third party"; "it does not matter if a public or private instance handles the request, because it is anonymized in both cases" | CONFIRMED (statelessness) / INFERRED (no retention machinery — absence) |
| Privacy defaults | By default SearXNG strips identifying data from upstream requests: no cookies forwarded, "generating a random browser profile for every request", no ads/tracking served, search query hidden from visited result pages (referrer protection); the instance's own IP remains visible upstream unless proxy/Tor is configured | https://docs.searxng.org/own-instance.html | "not sending cookies to external search engines and generating a random browser profile for every request"; "SearXNG can also be configured to use proxy or Tor"; "hiding from the results pages being visited" the referring page and search query | CONFIRMED |
| Privacy defaults | `image_proxy` (proxying image results through the instance) exists but is tied to memory cost and is part of the `public_instance` feature bundle, i.e. not forced on by default for local use | https://docs.searxng.org/admin/settings/settings_server.html | "Allow your instance of SearXNG of being able to proxy images. Uses memory space."; public_instance "allows to enable features specifically for public instances (not needed for local usage)" | CONFIRMED |

### 1.2 What Meridian's 15 crates add on top of the sidecars (repo-side, from `01-rebaseline.md`)

| Vanilla SearXNG behavior (above) | Meridian addition |
|---|---|
| No local index (connectors only) | Local hybrid corpus: usearch HNSW int8 ANN (1M docs, 506MB, 0.98 recall@10, p99 1.73ms) + BM25 (0.48ms p50 @100k), fused with web results |
| Occurrence/position score, no content signals | RRF(k=60) fusion (ADR-09), linear LTR cold-start scorer, domain PageRank prior (ADR-10), CE deep rerank (204ms p50 @ top-20), `diversity=evidence` canonical-cluster reorder (v0.6.0, dominates MMR on alpha-nDCG **and** nDCG) |
| Static engine config + failure bans | ε-greedy bandit over engine arms with top-10 contribution reward (R1/R2), linear-TS contextual policy dark behind DR-OPE gate (ADR-25); shed ladder |
| Engine error counters at /stats only | Geo heatmaps with Getis-Ord Gi* + BH-FDR, EB-shrunk quasi-NB trend z + Kleinberg burst decode (ADR-21/28), per-response NQC+Clarity confidence and JSD divergence vs measured noise floor (ADR-22/23) |
| Raw engine snippets only | Answer mode: Weitzman `pandora_walk` fetch selection + extractive `best_passage` (suite 18: 0.700 vs 0.593), VoI fetch ladder with `search_stopped_because` (ADR-26/29) |
| Stateless, so deletion is N/A by construction | Meridian *is* stateful (corpus, bandit state, analytics rows) and therefore ships ADR-19 deletability + retention as a designed property — a guarantee class vanilla SearXNG never has to (and cannot) make about local corpora |
| JSON `format` disabled by default (403) | Sidecar config must (and does, per ADR-11) enable `json` explicitly — worth re-checking on every SearXNG image bump since a default-reset silently 403s the planner |

---

## Part 2 — Academic / OSS retrieval for constrained hardware

Meridian measured baseline for comparison (Pi 5, Profile R): **1M docs, ~506MB resident,
recall@10 0.98 @ ef=128, p99 1.73ms; embed 42.9k docs/s (potion static embeddings,
no transformer at query time); RSS plateau ~250–261MB in a 3GB cgroup.**

### 2.1 LEANN — low-storage ANN via graph-based selective recomputation

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | arXiv:2506.08276, v1 2025-06-09, v2 2025-11-25; repo badges it "[MLSys 2026]"; Berkeley Sky Computing Lab | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | "LEANN: A Low-Storage Vector Index"; repo: "[MLSys2026]" + Berkeley Sky Computing Lab affiliation | CONFIRMED |
| Core idea / index size | Stores a pruned proximity graph and **recomputes embeddings at query time** instead of storing them: "up to 50x" smaller than conventional indices, "e.g., 5% of the original data"; repo: "Index 60 million text chunks in just 6GB instead of 201GB" (97% saving); paper Table 1: LEANN 4GB vs HNSW 188GB on RPJ-Wiki 60M chunks | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | Abstract: "uses only a fraction of the storage (e.g., 5% of the original data)" | CONFIRMED |
| Recall / latency | Target 90% recall@3; measured search latency **~2.48s per query on an RTX 4090** (NQ, 60M chunks); baselines: in-memory HNSW 0.03s, IVF-recompute 307.61s. Latency comes from re-running the embedding model (Contriever, 768-d; GTE-small gives 2.3× speedup) along the graph traversal with GPU dynamic batching | https://arxiv.org/abs/2506.08276 (v2 §5, Tables 1–2) | "LEANN achieves approximately 2.48 seconds retrieval on NQ"; hardware: "NVIDIA RTX 4090… 32GB RAM" and an AWS M1 Mac 128GB | CONFIRMED |
| Hardware envelope | CPU-only install exists (`cpu` extra; Linux/macOS/Windows), but acceptable latency depends on fast embedding recomputation — the paper's headline numbers are on a 4090; recomputation is the latency budget by design | https://github.com/yichuan-w/LEANN ; https://arxiv.org/abs/2506.08276 | "CPU-only (Linux): use the `cpu` extra"; dynamic batching "aggregates embedding computations across search hops" for the GPU | CONFIRMED (claims) / INFERRED (CPU latency penalty) |
| Insert / DELETE | Paper §6/App. B: optimized adds (O(M·efC)), batched insertion, and **soft deletes** ("marking nodes as inactive rather than removing them"); repo exposes build/search + `leann watch` file-change detection, but no hardened public delete API is documented in the README | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | Soft deletion = "marking nodes as inactive rather than removing them" | CONFIRMED (paper) / INFERRED (API maturity gap) |
| License / maturity | MIT; ~11.9k stars; v0.3.7 (2026-03); active, research-grade (pre-1.0, MLSys 2026 artifact) | https://github.com/yichuan-w/LEANN | MIT License; "11.9k stars"; v0.3.7 March 2026 | CONFIRMED |
| Vs Meridian | On Pi-5 8GB: LEANN trades storage for per-query transformer inference. Meridian's whole 1M-doc index is 506MB resident (well inside 8GB) and answers at p99 1.73ms with recall@10 0.98; LEANN is ~2.5s **on a 4090** at lower recall target (90%@3), and a Pi-5 CPU recompute of a 768-d transformer across hundreds of graph hops would be orders of magnitude past the latency floor — and past the *whole* deep-mode budget (2173ms p50). LEANN solves a storage problem Meridian does not have | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT.** Wrong constraint: it spends query-time compute (Meridian's scarcest
Pi-5 resource) to save storage (a resource Meridian's int8 HNSW already has under control
at 506MB/1M docs). Its soft-delete design is the one idea worth remembering if Meridian
ever needs graph-index deletes beyond ADR-19's current mechanism.

### 2.2 ColBERTv2 / PLAID — late-interaction retrieval

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | ColBERTv2: NAACL 2022 (arXiv:2112.01488, sub. 2021-12-02, rev. 2022-07-10). PLAID: arXiv:2205.09707 (sub. 2022-05-19), CIKM 2022 per the repo's publication list | https://arxiv.org/abs/2112.01488 ; https://arxiv.org/abs/2205.09707 ; https://github.com/stanford-futuredata/ColBERT | "ColBERTv2: Effective and Efficient Retrieval via Lightweight Late Interaction" (NAACL 2022); repo lists SIGIR'20…CIKM'22…EMNLP'23 | CONFIRMED |
| Index size | Residual compression cuts late-interaction index size "6–10×"; MS MARCO v1 (8.8M passages): vanilla ColBERTv2 24.6 GiB → PLAID 21.6 GiB at 2-bit/dim (1-bit used for the 138M-passage v2) — i.e. **~2.4 GiB per 1M passages**, ~5× Meridian's 506MB/1M, before the model | https://arxiv.org/abs/2112.01488 ; https://ar5iv.labs.arxiv.org/html/2205.09707 (Table 1) | "reducing the space footprint of late interaction models by 6–10×"; "Index Size (GiB): Vanilla 24.6, PLAID 21.6" | CONFIRMED |
| Recall / latency | PLAID, MS MARCO v1 dev, MRR@10 39.8 (vanilla 39.7): GPU (TITAN V) 11.5/20.2/38.4 ms at k=10/100/1000; **CPU 31.5/52.9/101.3 ms** — on a 28-core Xeon Gold 6132 (56 threads). MS MARCO v2 (138.4M): CPU 181.9ms @k=100, GPU OOM @k=1000. Speedups "up to 7× on a GPU and 45× on a CPU" over vanilla ColBERTv2 | https://ar5iv.labs.arxiv.org/html/2205.09707 (Tables 3, 6) ; https://arxiv.org/abs/2205.09707 | "latency of tens of milliseconds on a GPU and tens or just few hundreds of milliseconds on a CPU at large scale" | CONFIRMED |
| Hardware envelope | "a GPU is required for training and indexing"; CPU-only inference is supported (dedicated `conda_env_cpu.yml`); query encoding itself is a BERT-class forward pass per query | https://github.com/stanford-futuredata/ColBERT | "Note that a GPU is required for training and indexing."; "new environment file specifically for CPU-only environments" | CONFIRMED |
| Insert / DELETE | An **IndexUpdater** for adding/removing passages was merged 2023-01-29 explicitly "in beta"; no production-hardening announcement since. Default workflow remains build-once, static index | https://github.com/stanford-futuredata/ColBERT | "(1/29/23) We have merged a new index updater feature… These are in beta so please give us feedback" | CONFIRMED (beta status) / INFERRED (still not production-grade) |
| License / maturity | MIT; ~3.9k stars; strong academic pedigree but last headline feature news 2023; research codebase, not an appliance component | https://github.com/stanford-futuredata/ColBERT | MIT; "3.9k stars" | CONFIRMED |
| Vs Meridian | Per 1M docs: ~2.4 GiB index (on SD card — risk #7 endurance) vs 506MB; CPU latency 31.5ms@k=10 on a 56-thread Xeon ⇒ plausibly 150–400ms+ on 4× Cortex-A76, plus per-query BERT query encoding, vs 1.73ms p99. Quality is the draw (late interaction ≈ CE-grade), but Meridian already buys cross-attention quality where it pays: CE deep rerank 204ms p50 over top-20, gated to deep mode | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT.** ~5× the storage on SD, server-class-CPU latencies that don't
transfer to 4 Cortex-A76 cores, GPU-required indexing off-device, and beta-only
add/remove vs ADR-19 deletability. Meridian's CE-rerank-on-top-of-cheap-candidates
already occupies the same quality niche at a fraction of the standing cost.

### 2.3 Seismic — efficient learned-sparse retrieval (added per brief's substitution clause, alongside LEANN)

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | SIGIR 2024 (arXiv:2404.18812, sub. 2024-04-29; DOI 10.1145/3626772.3657769 = SIGIR '24 proceedings; repo lists SIGIR 2024 + CIKM 2024, ECIR 2025, ECIR 2026 follow-ups) | https://arxiv.org/abs/2404.18812 ; https://github.com/TusKANNy/seismic | "Efficient Inverted Indexes for Approximate Retrieval over Learned Sparse Representations" | CONFIRMED |
| Core idea | Inverted lists organized into "geometrically-cohesive blocks" with summary vectors; "one to two orders of magnitude faster than state-of-the-art inverted index-based solutions" at "sub-millisecond per-query latency" with high recall | https://arxiv.org/abs/2404.18812 | Abstract quotes as cited | CONFIRMED |
| Hardware envelope / index size | Rust with Python bindings, CPU-only design; MS MARCO v1 + SPLADE-v3: **7.9 GB RAM** (vs 24.0 GB for the compared alternative) ⇒ ~0.9 GB per 1M docs, in-RAM | https://github.com/TusKANNy/seismic | "written in Rust with Python bindings"; "Memory (GB): 7.9" vs 24.0 | CONFIRMED |
| Recall / latency | MRR@10 40.27 on MS MARCO at average query time **185µs** (~4× faster than competitors at matched quality) | https://github.com/TusKANNy/seismic | "MRR@10: 40.27"; "AQT (μs): 185" | CONFIRMED |
| Insert / DELETE | **Static index.** Build-then-query API only; no insert/delete/update mechanism documented in repo or paper | https://github.com/TusKANNy/seismic | README API covers building and querying; no update mechanisms described | INFERRED (absence — adversarial row: fails ADR-19-style deletability outright) |
| License / maturity | MIT; Rust 61.4%; v0.4.0 (2026-03-25), 3 releases, ~130 stars; active research line (4 papers) but small community | https://github.com/TusKANNy/seismic | MIT badge; "3 releases", v0.4.0 March 2026 | CONFIRMED |
| Vs Meridian | The only candidate that respects the CPU envelope (Rust, 185µs, ~0.9GB/1M in RAM). But: (a) it retrieves over **learned sparse** embeddings — every query needs a SPLADE-class transformer forward pass, which Meridian deliberately avoids (potion static embeddings, ADR-02 "no neural cost" intent path); (b) 0.9GB/1M RAM vs 506MB total resident pressures the 3GB cgroup; (c) no deletes; (d) MRR 40.27 is single-representation quality Meridian approximates via hybrid RRF + CE rerank | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT for v0.x, WATCH.** Engineering profile (Rust, CPU, µs-latency) is the
closest match to Meridian's frontier, but the SPLADE query-encoder dependency violates the
no-transformer-on-the-hot-path budget, the index is static (fails the deletability bar
ADR-19 sets), and RAM/M docs is ~1.8× Meridian's whole footprint. Re-open only if the
Hailo-8L NPU path (C4 ledger) ever makes on-device learned-sparse encoding free — and even
then it must beat hybrid BM25+ANN+CE through a suite gate, not on MS MARCO numbers.

### 2.4 Frontier verdict

Meridian's current stack (int8 HNSW + BM25 + RRF + gated CE) already sits at the right
point of the storage/latency/recall frontier *for this device*: 506MB/1M docs fits RAM
with 6× headroom, 1.73ms p99 leaves the entire latency budget for fetch/rerank/answer
phases, and 0.98 recall@10 leaves ≤2pp on the table that none of the three candidates
recovers without breaking either the latency floor (LEANN, ColBERT/PLAID), the storage
budget (ColBERT/PLAID), the no-query-transformer rule (Seismic, ColBERT, LEANN), or
deletability (Seismic, PLAID; LEANN paper-only). **No adoption recommended.** Portable
ideas worth recording: LEANN's soft-delete graph nodes; Seismic's block/summary pruning
as a possible BM25 accelerator shape if posting lists ever become the bottleneck.

---

## Sources (15)

1. https://docs.searxng.org/own-instance.html — privacy model, statelessness
2. https://docs.searxng.org/dev/search_api.html — Search API, formats, 403 behavior
3. https://docs.searxng.org/admin/settings/settings_search.html — default formats (html only), ban/suspension times
4. https://docs.searxng.org/admin/settings/settings_outgoing.html — proxies, Tor, source_ips, retries
5. https://docs.searxng.org/admin/settings/settings_general.html — enable_metrics, open_metrics
6. https://docs.searxng.org/admin/settings/settings_server.html — image_proxy, public_instance, limiter
7. https://docs.searxng.org/dev/engines/offline_concept.html — no local index; offline-engine connectors
8. https://docs.searxng.org/dev/engines/enginelib.html — engine weight, timeout, tokens
9. https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py — official source: merge + score + ordering
10. https://arxiv.org/abs/2506.08276 — LEANN paper (v2 2025-11-25)
11. https://github.com/yichuan-w/LEANN — LEANN repo (MIT, v0.3.7, MLSys 2026)
12. https://arxiv.org/abs/2112.01488 — ColBERTv2 (NAACL 2022)
13. https://arxiv.org/abs/2205.09707 (+ ar5iv HTML render for Tables 1/3/6) — PLAID (CIKM 2022)
14. https://github.com/stanford-futuredata/ColBERT — ColBERT repo (MIT, IndexUpdater beta)
15. https://arxiv.org/abs/2404.18812 + https://github.com/TusKANNy/seismic — Seismic (SIGIR 2024; MIT, Rust, v0.4.0)
