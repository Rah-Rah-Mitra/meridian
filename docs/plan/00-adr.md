# 00 — Architecture Decision Records

> SPEC §1.1. Every locked decision from SPEC §3–§5 restated, feasibility-checked
> against live sources on **2026-06-10** (crate versions, docs, dependency graphs —
> three parallel research passes), with status **CONFIRMED** or **CHALLENGE:
> <evidence>**. Two environment-driven ADRs (D1, D2) record deviations forced by the
> actual device. A CHALLENGE here never silently changes the spec — it states the
> evidence and the proposed resolution for sign-off.

Device facts feeding several ADRs: the first target is the operator's existing Pi 5
(8GB), running the Raspberry Pi OS **16KB-page kernel** (`getconf PAGESIZE` =
16384, kernel `6.18.33+rpt-rpi-2712`), one 29.7GB SD card (**no NVMe**), ~7GB free,
shared with other always-on services.

---

## ADR-01 — Kernel page size: support the 16K kernel (SPEC §2 option b)

**Decision.** The supported configuration is the Pi OS default **16KB-page kernel**,
i.e. SPEC option (b): every allocator/mmap dependency must work at 16K. The 4K
switch (`kernel=kernel8.img`) is documented in `pi-setup.sh` as a fallback only.

**Evidence.**
- The deployment device already runs 16K; switching kernels on a shared device that
  hosts other services is the riskier move.
- mimalloc reads the page size at **runtime** (`sysconf(_SC_PAGESIZE)` in
  `src/prim/unix/prim.c`); its internal segments are independent of OS page size.
  No 16K failure reports found in the tracker. The notorious 16K breakage belongs
  to **jemalloc** (compile-time `--with-lg-page`: ripgrep #2180, jemalloc #467) —
  so jemalloc is **banned** from the graph via cargo-deny `[bans]` (added when any
  dep tries to pull it).
- tantivy (memmap2) and usearch `view()` mmap whole files from offset 0 —
  page-size-agnostic at the syscall level; **no explicit 16K test exists upstream**
  for usearch, so the Phase-0 bench (suite 2) includes a view-mode smoke test
  on-device as the tripwire.

**Status: CONFIRMED** (option b), with the bench smoke test as a gate before
Phase 2 relies on it.

## ADR-02 — musl vs gnu for the static binary (and the `ort` friction)

**Decision (staged).**
1. **Phases 0–2 build pure `aarch64-unknown-linux-musl`** static binaries — proven
   in CI already (zigbuild job green on the scaffold). Nothing in these phases needs
   ONNX Runtime: embeddings are model2vec-rs (pure Rust), lexical is tantivy,
   ANN is usearch (C++ but zig handles C/C++ cross-linking — that is cargo-zigbuild's
   specialty).
2. **Phase 3 entry runs a decision bake-off** for the ONNX inference runtime:
   - **Plan A: `tract-onnx` on musl** (pure Rust, 0.23.0, active). Verified to
     implement the INT8 ops the CE needs (`MatMulInteger`, `QLinearMatMul`,
     `DynamicQuantizeLinear`). Keeps the single static binary. `ort` even offers a
     tract backend (`ort-tract`) so the `ort` API can be kept regardless.
   - **Plan B: `ort` with pyke prebuilts on `aarch64-unknown-linux-gnu`** + a
     distroless/Debian-13 base image (prebuilts require glibc ≥2.39 — satisfied
     *inside* the container regardless of host). Image stays ≤80MB per SPEC §7.1's
     own fallback.
   - Decision rule: if tract's CE ms/pair on the A76 is within **1.5×** of ort's,
     Plan A wins (static musl preserved); otherwise Plan B.

**CHALLENGE (evidence, resolution above):** SPEC §7.1 assumes `ort` may link on
musl. Verified otherwise: pyke ships **no musl prebuilts** (platform matrix is
glibc-only, requires glibc ≥2.39), and the maintainer confirms musl means building
ONNX Runtime from source (ort #355) — a heavy, fragile CI cost. `load-dynamic`
contradicts the `FROM scratch` single-binary goal. The staged plan defers the cost
until the phase that needs it and keeps both spec goals (static binary, one
inference runtime) recoverable. ONNX Runtime's aarch64 INT8 kernels use
NEON+dotprod (sdot/udot) at runtime — the A76 qualifies (no i8mm/SVE, so no i8mm
fast path on either plan).

**Status: CHALLENGE → RESOLVED at Phase-3 entry (2026-06-10).**

**Phase-3 resolution (the bake-off, decided):**
- **LTR + intent classifier ship as pure Rust, no inference runtime.** The spec
  itself defines cold-start LTR as "hand-tuned linear weights behind the same
  ONNX interface (single Gemm)" — we implement that Gemm in Rust behind a
  `Scorer` trait, and intent as a µs-scale heuristic ("no neural cost"). There is
  no training data yet (no click logs, only a synthetic eval set), so the GBDT
  upgrade is premature regardless of runtime. This keeps the default musl/scratch
  image neural-free.
- **Deep cross-encoder rerank uses `ort` on the gnu target, feature-gated.**
  Evidence: tract 0.23 cannot load the official `model_qint8_arm64.onnx`
  (Unsqueeze13) AND cannot build under zigbuild/musl (fp16, same family as
  numkong/risk-3); ort loads the INT8 export with full op coverage and this Pi
  runs glibc 2.41 (≥2.39 → ort prebuilts work natively on aarch64-gnu). The
  default scratch/musl image degrades `mode=deep` to LTR order with
  `degraded:["rerank_unavailable"]`; a gnu image variant carries the `rerank`
  feature. This is the §7.1-sanctioned "gnu/distroless fallback for ort."
- tract is NOT dead: if a tract-friendly CE re-export materializes it can replace
  ort behind the same `Reranker` trait. Not worth blocking Phase 3 on it.

**Original staged plan (superseded by the above):**

**Post-sign-off evidence (Phase-0 bench build):** tract-linalg 0.23's build script
compiles SVE f16 C kernels with `-march=armv8.2-a+sve+fp16` — a GCC extension
spelling that zig's clang rejects (`fullfp16` is the clang name), and its SVE
probe doesn't catch this because it probes `+sve` only. Consequence: **tract does
not build under cargo-zigbuild/musl today.** The bench binary therefore builds on
`aarch64-unknown-linux-gnu` with the GNU cross toolchain (a dev tool for the
glibc-2.41 Pi — acceptable). For Phase 3, Plan A (tract in the musl product)
requires either an upstream fix to tract-linalg's probe/flags (small,
upstreamable) or a vendored build.rs patch; otherwise Plan B (gnu + distroless)
absorbs this too. Logged as risk #2/#3 adjunct.

**On-device run evidence (2026-06-10):** tract 0.23 additionally **fails to load
the official `model_qint8_arm64.onnx` CE export** ("Failed analyse for node #379
/bert/Unsqueeze (Unsqueeze13)") at every batch size — an op/shape-inference
coverage gap on this particular INT8 graph. Plan A therefore needs a
tract-friendly (re-)export or our own quantization in `train/` before it can be
benchmarked at all; Plan B (ort + gnu/distroless) remains fully viable. The
Phase-3 bake-off starts from this position.

## ADR-03 — Extraction bake-off: dom_smoothie presumed winner

**Decision.** Phase-1 bake-off proceeds per spec, but between **`dom_smoothie`**
(0.18.0, 2026-06-07, actively maintained Readability.js port, MIT) and
`readability-rs` as a control. Fixture set: 50 real pages + malformed-HTML corpus;
criteria: extraction accuracy, p95 latency, peak RSS on A76.

**CHALLENGE (mild):** the spec's `readability-rs` is a low-adoption fork (0.5.0,
2024) of a crate unmaintained since 2023. The only 2025/2026 systematic comparison
(13 crates) found dom_smoothie "the sole readability implementation that
consistently extracted main content correctly". The bake-off stands (accuracy on
*our* fixture set still decides) but the presumption and the effort allocation
follow the evidence.

**Status: CHALLENGE → bake-off reframed** (dom_smoothie presumptive).

## ADR-04 — Arti: embedded `arti-client`, hand-rolled SOCKS front-end (SPEC §12.2/§12.4)

**Decision.** Embed **`arti-client` 0.43.x** in-process on the shared Tokio runtime
(`create_bootstrapped()` / `BootstrapBehavior::OnDemand`). For the searxng-anon
path, **implement Meridian's own thin SOCKS5 listener** over stable APIs
(`TorClient::connect` + `StreamPrefs::set_isolation`), mapping RFC1929
username/password to an `IsolationToken` per distinct credential — rather than
depending on the `arti` binary crate's `run_proxy`, which is exposed only behind
`experimental-api` and documented as unstable.

**Evidence (verified in docs + crate source).**
- Bootstrap-on-Tokio, `IsolationToken`, `StreamPrefs::set_isolation`,
  `isolated_client()` — all confirmed by name in arti-client 0.43.0 docs.
- The SOCKS listener is NOT in `arti-client`; in `arti` 2.4.0 it is
  `experimental-api`-gated, "should not be considered stable". The per-connection
  logic (`arti/src/proxy/socks.rs`, incl. the RFC1929→isolation mapping we need) is
  small and built on public arti-client APIs — reimplementing it is lower risk than
  tracking an experimental surface.
- Build features for musl: `rustls` (NOT default native-tls) + **ring** crypto
  provider (installed by the app — arti-client deliberately doesn't pick one),
  `static-sqlite` (bundled C sqlite, musl-fine), drop `compression` if zstd-sys/
  liblzma-sys cause friction (cost: larger directory downloads).
- **License scan of the full 515-package graph: no forced copyleft.**
  `priority-queue` (unconditional, via tor-proto→tor-rtmock) is
  `LGPL-3.0-or-later OR MPL-2.0` → we elect **MPL-2.0**; `r-efi` is triple-licensed
  incl. MIT. Consequence: when arti enters (Phase 4), `deny.toml` adds MPL-2.0 to
  the allow list with a comment recording this election. SPEC §13.2's
  AGPL/LGPL/GPL denial holds.
- API churn risk: arti-client 0.x ships ~monthly with documented breakage →
  pin exact version; monthly dep-bump chore owns the update.
- Footprint: no authoritative RSS benchmark exists; the §6.2 estimate (80–150MB)
  stands as provisional until bench suite 8 measures it (memquota default cap is
  1GiB — we configure it well below).

**Status: CONFIRMED** (embedded), with the SOCKS implementation choice recorded.

**Phase-4 amendments (as built, 2026-06-11).**
- **In-core anon fetches also ride the SOCKS front-end** (a fresh RFC1929
  username per logical request → its own `IsolationToken`), rather than a second
  raw `connect_with_prefs` HTTP stack. One egress path, one destination-policy
  chokepoint, and reqwest's TLS/redirect/timeout handling stays uniform across
  lanes. The spec's per-request isolation requirement is met via the
  username→token map (`anon/arti.rs::TokenMap`, unit-tested).
- **searxng-anon gets per-CONNECTION isolation, not per-query.** SearXNG's
  outgoing proxy URL is static — it cannot vary RFC1929 credentials per request —
  so no-auth connections each get a fresh token. Strictly stronger than the
  `arti` proxy default (which lumps all no-auth clients of one listener into one
  isolation group), but weaker than true per-query isolation: httpx connection
  pooling can carry several queries' engine hits over one tunnel. Recorded in
  the threat model; revisit if SearXNG grows per-request proxy auth.
- **Anon lane is compiled in unconditionally** (config-gated, not
  feature-gated): pure-Rust + bundled sqlite builds fine for musl, unlike ort
  (ADR-02). `rustls`'s provider is installed by the lane (`ring`).
- **Region verification**: implemented as exact/prefix `expected_ip` match on an
  operator-configured IP-echo endpoint, re-checked every 30 min; an unverified
  or mismatched lane REFUSES traffic (not merely "Degraded-but-serving").
  GeoLite2 region-name matching is deferred to Phase 5 (geo crate brings mmdb).

## ADR-05 — Tantivy as the lexical index

**Decision.** tantivy 0.26.x, mmap directory, BM25 + block-WAND, u64 fast fields,
zstd doc store (`zstd-compression` feature — default is lz4), snippets,
delete-by-term (the `/v1/forget` primitive). **Workspace MSRV raised to 1.86**
(tantivy's MSRV; already committed).

**Notes from verification.**
- `LogMergePolicy` caps by **doc count**, not bytes (`set_max_docs_before_merge`);
  SPEC §9.1's "max_merged_segment=256MB" is implemented as
  `max_docs = 256MB / measured_avg_doc_bytes` from bench suite 3, or a custom
  `MergePolicy` if the approximation drifts >20%.
- Index format compatibility windows are real (0.24 reads 0.21+; 0.13/0.9 were hard
  breaks) → `meridiand --reindex` migration path per SPEC §14 stands.
- `lindera-tantivy` (2.0.0) currently pins tantivy ^0.25 — lags 0.26. CJK feature
  ships OFF (operator confirmed no CJK at launch, Q6); revisit only on demand.

**Status: CONFIRMED.**

## ADR-06 — model2vec embeddings via model2vec-rs

**Decision.** `model2vec-rs` 0.2.1 + `potion-base-8M` (30.2MB safetensors, MIT).

**Evidence.** Crate verified active (MinishLab, 2026-05); pure Rust
(safetensors + HF tokenizers + ndarray — no candle/tch/ONNX); supports
potion-base-8M loading from a local path and i8 weight types; authors claim ~8k
samples/s single-threaded CPU (relative number — bench gate >2k/s on the A76
decides). Check at Phase-2 entry that the `tokenizers` backend resolves to the
pure-Rust regex engine (not `onig`) for the musl build.

**Status: CONFIRMED.**

## ADR-07 — USearch ANN with a re-ranked fallback ladder

**Decision.** `usearch` 2.25.3 (cxx FFI), `MetricKind::Cos` + `ScalarKind::I8`,
M=16/efc=128/ef=64, serialization + mmap `view()`.

**Evidence & deviations.**
- i8 + cosine + aarch64 NEON/dotprod confirmed (default `numkong` SIMD backend,
  runtime dispatch). `cargo asm`/bench verifies the dotprod path on-device (SPEC
  §7.2).
- musl C++ cross-build is **unproven upstream** (no musl CI; needs the zig C++
  toolchain; `openmp` feature stays off). CI proves it the day usearch enters
  (Phase 2) — that cross job is the tripwire.
- **Fallback ladder reordered (CHALLENGE, mild):** spec names `hnsw_rs` as the
  fallback, but verification shows it has **no SIMD on stable aarch64** (simdeez is
  x86-only; std::simd needs nightly) and **u8, not i8**, scalar support — a much
  weaker fallback than assumed. Revised ladder: (1) usearch+numkong → (2) usearch
  portable-C++ (drop the SIMD feature, keep i8) → (3) hnsw_rs with offset-u8
  quantization, accepted as degraded-perf last resort.

**Status: CONFIRMED — fallback ladder RUNG 2 ACTIVATED (2026-06-10);
RUNG 1 RESTORED at the P7 exit (2026-06-11) — see below.**

**Risk #3 tripwire fired (Phase 2):** usearch's default `numkong` SIMD backend
compiles SVE/SME C kernels (`-DNK_TARGET_SVE=1 -DNK_TARGET_SME=1 …`) that zig's
clang rejects under `aarch64-unknown-linux-musl` — the same toolchain-gap family
as tract's fp16 (ADR-02). The product image is musl, so this blocked the build.
**Resolution: `usearch = { default-features = false }`** → rung 2 (portable
auto-vectorized C++, keeps i8 + cosine + serialization + view). One code path for
both the musl product and the gnu bench. The A76 still auto-vectorizes the int8
kernels; the §15.2 gate (p99 <40ms) has ~42× headroom over the measured 0.95ms,
so the perf loss is immaterial — but the Phase-0 ANN numbers were numkong, so the
Phase-2 exit note re-validates the portable path on-device. numkong's NEON-dotprod
kernels remain available on gnu if a future need justifies a per-target feature
split; not worth the complexity now.

**P7-exit finding (2026-06-11): RUNG 2 WAS A RECALL DEFECT.** The Phase-2
re-validation covered latency and the end-to-end nDCG eval — NOT the synthetic
recall gate. The P7 exit re-ran suite 2 and found the portable build's i8
cosine path collapses with scale: recall@10 **0.52 @10k → 0.007 @100k → 0.0
@1M** (deterministic; reproduced with both the CI gnu artifact and a native
local build) vs Phase-0's numkong 0.98. The product's dense lane has been
returning near-noise neighbors at corpus scale since Phase 2 — masked because
RRF fusion is BM25-dominant and the 100-query eval moved (hybrid 0.42 ≥ BM25
0.38) on real embeddings. **Resolution: rung 1 restored** — numkong 7.7.0's
build script now probes each ISA kernel individually and hard-drops kernels
whose flags the compiler rejects (the per-kernel-probe fix the Phase-2-era
version lacked), so zig/musl builds keep NEON±dotprod and skip SVE/SME.
Re-measured with numkong: recall@10 **0.999 @100k, 0.92–0.98 @10k** across the
ef sweep. CI musl cross-build is the gate that proves the zig path; the bench
suite-2 recall gate re-runs at every exit henceforth (it was the only
instrument that could see this, and it sat unused from Phase 2 to Phase 7).

## ADR-08 — mimalloc global allocator

**Decision.** `mimalloc` crate 0.1.52 (bundles mimalloc v3), secure mode off.
Compatible with ADR-01: page size read at runtime. `mlock` of the small secrets
region is deferred to Phase 5 with an explicit 16K-granularity check (one locked
page = 16KB — fine for a few keys; never mlock large buffers, SPEC §13.5).

**Status: CONFIRMED.**

## ADR-09 — Ranking stack: RRF(k=60) → GBDT LTR → INT8 CE rerank

**Decision.** As specced. RRF k=60 (Cormack et al.); LightGBM→ONNX LTR (≤200
trees, cold-start linear behind the same interface); deep-mode CE rerank top-20,
batch 4, 1.5s stage deadline, Moka-cached.

**Verification bonus:** the official `cross-encoder/ms-marco-MiniLM-L-6-v2` repo
now ships a ready **`onnx/model_qint8_arm64.onnx` (23.2MB INT8)** — no custom
quantization needed for v1; `train/` quantization tooling becomes contingency.
Inference runtime per ADR-02's bake-off.

**Status: CONFIRMED — Phase-3 note:** LTR cold-start is the pure-Rust linear
scorer (ADR-02 resolution); GBDT→ONNX awaits training data. Deep CE via ort/gnu.

## ADR-10 — Geo: h3o, gazetteer fst, remote geocoder, optional GeoLite2

**Decision.** As specced. `h3o` 0.10 verified pure Rust with `grid_disk`,
`parent/children`, and lossless u64↔CellIndex conversion (the fast-field
representation). GeoNames cities15000 is 3.1MB zipped (under the 10MB fst budget).
`maxminddb` 0.28 active. Geocoder: public Nominatim @1 rps + redb cache (Q8
default). petgraph 0.8.3 has **built-in `page_rank`** (and parallel variant) for
the nightly domain-prior job — no hand-rolled implementation.

**Status: CONFIRMED.**

**Phase-5 amendments (as built, 2026-06-11).**
- **Geo prefilter mechanics**: `h3_r7` AND `h3_r5` are INDEXED|FAST; k-rings
  become `TermSetQuery` MUST clauses (fine res-7 sets ≤8km radius via res-6/7
  disks; >30km switches to res-5 term sets directly — no child expansion, which
  is what keeps sets ≤4096). Radius clamped to 250km.
- **ANN under a geo/ts filter is dropped** (lexical-only fusion): usearch has
  no filtered search, and post-filtering the dense list would smuggle
  out-of-area docs into RRF. Constrained queries are exact-BM25; the dense path
  returns when a filtered ANN (usearch v2 filter callbacks or rung-3 hnsw_rs)
  lands. Recorded tradeoff, not a bug.
- **PageRank substrate**: GDELT carries no hyperlink graph, so `domain_prior`
  ranks the DOMAIN CO-OCCURRENCE graph (domains reporting the same event-root
  class in the same 15-min slice, ≤30-domain cliques, weight-1 edges dropped,
  200k-edge cap). Honest available signal; bounded; recomputed nightly from
  scratch; max-normalized to [0,1]. The cold-start LTR weighs the feature 0 —
  it feeds the future GBDT.
- **Remote geocoder built but NOT wired into ingest** (gazetteer-first per
  SPEC §3 covers it without network; the Nominatim fallback + redb/Moka cache
  ships tested in `meridian-geo::geocode` awaiting an operator who wants it —
  ingest wiring is a config flag away, deferred to keep ingest zero-network).
- **GeoNames licensing**: cities15000 is CC-BY 4.0 — attribution added to the
  README data-credits line; artifact built offline via
  `deploy/fetch-gazetteer.sh`, never committed.

## ADR-11 — SearXNG sidecars (direct + Tor-proxied)

**Decision.** As specced: two instances of the official `searxng/searxng` image
(arm64 ~90MiB compressed — comfortably under the 350MB uncompressed budget),
digest-pinned in Phase 1.

**Verification.** `search.formats: [json]` enables the JSON API (default is
html-only — our config already sets it). `outgoing.proxies` with `socks5h://` is
explicitly handled in searxng source (`searx/network/client.py` sets `rdns=True` —
DNS through the proxy, the §12.5 invariant-2 mechanism). Per-instance proxy
config confirms the spec's two-instance rationale (per-request proxy switching in
one instance is impossible).

**Status: CONFIRMED.**

## ADR-12 — robots.txt: texting_robots instead of robotstxt

**CHALLENGE.** SPEC §17 names `robotstxt` (Google parser port) — dormant since
2021. `texting_robots` (0.2.2, MIT/Apache) passes Google's test suite **plus**
Moz reppy tests, is fuzzed, and was validated against 34M real Common Crawl
robots.txt files. Both are frozen, but the format itself is frozen (RFC 9309), and
texting_robots' hardening against malformed real-world input is exactly what a
fetcher needs. **Resolution: adopt `texting_robots`;** vendor/fork if an upstream
fix is ever needed.

**Status: CHALLENGE → substitution proposed** (needs sign-off).

## ADR-13 — Cache/persistence: Moka + redb (no Redis)

**Decision.** As specced. moka 0.12.15 verified (weigher-based size-aware
eviction + TTL + TTI — all three required by §8.4). redb 4.1.0 verified ("file
format is stable"); pin the major version and plan upgrades deliberately (4.x
cadence is fast). Anon-cache isolation rules per SPEC §12.4 unchanged.

**Status: CONFIRMED.**

## ADR-14 — Egress lanes over reqwest (single client per lane)

**Decision.** reqwest 0.13.4 + rustls. One `Client` per lane (per-client proxy
granularity is the reqwest model — matches the `LaneClient` design). Region lane:
`ClientBuilder::local_address(IpAddr)` confirmed; **`ClientBuilder::interface()`
(SO_BINDTODEVICE) also exists** and is the stronger primitive — region lanes use
`interface()` + `local_address` together, with ip-rule policy routing as
documented. Anon path: `socks` feature, `socks5h://` scheme confirmed parsed.
Cookie store confirmed off by default (§13.4 minimization).

**Status: CONFIRMED** (with the `interface()` improvement noted).

## ADR-15 — Analytics: GDELT over plain HTTP + manifest MD5

**Decision (forced by upstream).** `data.gdeltproject.org` serves an invalid TLS
certificate (verified live). The puller fetches over **HTTP** and verifies each
slice against the `lastupdate.txt` manifest MD5. This is integrity-only, not
authenticity — acceptable because analytics counters are non-security-critical and
never feed ranking of the trusted path directly (domain_prior is recomputed,
bounded, and observable). Documented as residual risk #4 in the threat model.
Slice sizes verified tiny (45KB–3.1MB per 15-min export) — well inside budget.

**Status: CONFIRMED with documented caveat.**

## ADR-16 — Telemetry: metrics + Prometheus exporter; redaction architecture

**Decision.** `metrics` 0.24 + `metrics-exporter-prometheus` 0.18;
`tracing-subscriber` 0.3.23 with a custom redaction `Layer`.

**Architecture note from verification:** a tracing `Layer` cannot mutate an event
for *downstream* layers — redaction must live in the layer that formats/exports.
Therefore the design is defense-in-depth: (1) `Redacted<T>`/secret types make leaks
impossible at the call site (already scaffolded + tested), (2) the fmt/export layer
filters/redacts named fields (`q`, `ip`, `url`, …) as the backstop, (3) the privacy
smoke test hunts canaries in actual output as the proof.

**Status: CONFIRMED.**

## ADR-17 — Workspace shape: 15 crates; dependency-rule fix

**Decision.** SPEC §5 prose says thirteen crates; its normative tree lists
**fifteen** — the tree wins (all 15 scaffolded). The §5 rules "`egress` depends
only on `common`" vs "`{…egress…} → privacy`" conflict; resolved as
`egress → {common, privacy}` (the lane layer needs `Redacted`/secret types to log
safely). Recorded in `01-wbs.md` §0.

**Status: CONFIRMED with two spec-text corrections.**

---

## Post-v0.1.0 ADRs (Phases 7–9, recorded 2026-06-11)

> Strategic context: v0.1.0 ships a metasearch *fuser*; Phases 7–9 turn it into an
> **evidence-and-uncertainty engine** (source-independence, vantage divergence,
> statistically defensible trends, calibrated confidence). Every ADR below honors
> the standing hard constraints: forget-correctness 100%, anon-lane firewall both
> ways (§12.4), fail-closed lanes (§12.1), no query logging, Profile R budgets.
> Operator decisions taken 2026-06-11 are marked as such.

## ADR-18 — Near-duplicate sketching & derivation clusters (Phase 7)

**Decision.** At ingest, compute per-doc **64-bit SimHash** (cheap near-dup gate)
and a **MinHash signature** over word shingles of the clean extracted text; store
both in a `sketch_v1` table inside `dedup.redb`, written in the SAME transaction
as the blake3 dedup/tombstone rows. At query time (post-RRF, pre-LTR), cluster
candidates by sketch similarity (SimHash Hamming gate → MinHash-estimated Jaccard
threshold → union-find) into derivation clusters; emit
`independent_source_count`, `apparent_source_count`, and per-result `cluster_id`
in a new additive `evidence` block. **Web results without fetched text get
`evidence: null` in v0.2.0** — snippet-only sketches are too weak to assert
independence and would mislead (honesty over coverage); deep-mode fetched docs and
local docs get full treatment.

**Parameters are NOT hand-picked:** shingle size, permutation count, and the
cluster threshold are fixed by the suite-9 (`synfarm`) parameter sweep on
synthetic syndication farms with a held-out generator variant (anti-overfit,
risk #21). Gate: pairwise F1 >0.8 AND false-merge <5% on both variants.

**Tradeoff.** ≤64 B/doc disk + ≤2ms p50 query-time budget vs the false-merge rate;
a false merge actively misleads (collapses genuinely independent sources), so the
threshold errs conservative and cluster membership is exposed in `rank_signals`
for auditability (risk #16).

**Status: CONFIRMED — constants fixed by the suite-9 run of 2026-06-11**
(`docs/plan/bench/2026-06-11-pi5-p7-experiments.md`): similarity =
**containment** (|A∩B|/min(|A|,|B|), estimated from MinHash Jaccard + exact set
sizes) — raw Jaccard fails outright on realistic truncation+boilerplate
syndication (F1 ≈ 0.01 at τ≥0.5); **shingle k=4, 128 perms, τ=0.3** → pairwise
F1 1.0 (primary) / 0.89 (hold-out), false-merge 0.0 on both; domain-dedup
baseline F1 0.054. Caveat from the run: the SimHash 99%-coverage Hamming radius
is 33/64 — too wide to be a useful general prefilter; SimHash serves only as a
verbatim-dup fast path (Hamming ≲ 6), with MinHash-containment doing the real
work. Evidence block default-ON with `evidence.enabled` kill-switch (operator,
2026-06-11).

**P7-exit amendment (`MIN_MATCH_BINS = 5`):** the live exit drill caught a
false merge the synthetic farms missed — ONE chance b=8 bin collision
(~21%/pair) against a tiny shingle set amplifies through the containment
denominator past τ. The estimator now returns 0 below 5 matched bins (noise
P(≥5) ≈ 5e-7; real derivation produces 25+); suite 9's generator gained tiny
stub docs so the size-asymmetry case is gated permanently. Hold-out F1
improved to 0.918 (the floor removes noise edges).

## ADR-19 — Derived-structure deletability policy (standing rule)

**Decision.** Any derived structure computed over **user-touchable data** (docs,
queries, fetch results) must either (a) join the atomic forget transaction
(delete-by-key alongside index/vector/dedup, like `sketch_v1`), or (b) be provably
rebuildable from surviving docs on a bounded schedule. Approximate structures that
cannot delete (plain Bloom, HyperLogLog, standard Count-Min) are permitted ONLY
over **GDELT-derived aggregates** (no per-user data; forget-orthogonal by
construction). Any analytic that depends on an approximate structure surfaces its
error bound in the response.

**Tradeoff.** Widening the forget transaction grows its latency/failure-atomicity
surface vs background-rebuild complexity; (a) is preferred while the per-forget
row count stays small (sketches: one row per doc).

**Status: CONFIRMED (extends the §13.4/Phase-5 forget guarantees to all future
analytics; tripwire = extended hermetic forget test, risk #17).**

## ADR-20 — API schema evolution for analysis blocks

**Decision.** The API stays `/v1`; Phases 7–9 add **additive optional blocks
only** (`evidence`, `confidence`, `divergence`, response-level `analysis`), each
carrying its own `schema` integer for intra-block versioning. Absent block =
feature off/unavailable — never an error, never a placeholder object. Removing or
re-typing an existing field requires `/v2`. (SPEC §10 amended, v2.2.)

**Tradeoff.** Field accretion over time vs endpoint forking — on a
single-operator appliance, client simplicity wins; the `schema` field gives an
escape hatch per block without a global version bump.

**Status: CONFIRMED.**

## ADR-21 — Statistical trends/heatmap methodology (Phase 7)

**Decision.** Replace the latest/window-mean ratio movers with: (1)
**Gamma-Poisson empirical-Bayes shrinkage** of per-cell/per-topic rates toward
the window mean (prior fit by method of moments across cells — stabilizes
low-count cells); (2) **Getis-Ord Gi*** z-scores over H3 res-5 neighborhoods
(k-ring 1–2 via `h3o` grid_disk inside `meridian-analytics`); (3)
**Benjamini-Hochberg FDR** at q=0.05 across all scanned cells (the FDR family is
the full scanned set, not the viewport — fixed and documented). Raw counts and
ratios remain in the output for explainability; new additive fields: `z`,
`q_value`, `shrunk_rate`, `significant`, plus a plain-language
"likely low-sample noise" label when the shrunk CI overlaps baseline.

**Prior + k-ring radius are fixed by the suite-10 (`spike`) injection study**
(planted Poisson spikes; gate ≥3× false-spike reduction at equal TPR vs the
thresholded-ratio baseline on the tuning variant, no regression with held FDR on
an overdispersed hold-out). GDELT caveat stands (ADR-15): these are signals
about *media coverage*, labeled as such — not ground truth about the world.

**Tradeoff.** O(cells × neighbors) per trends/heatmap call (res-5 grid → ms-scale,
budget ≤60ms R) vs the current detector firing on single-digit-count noise.

**Status: CONFIRMED — constants fixed by the suite-10 run of 2026-06-11**
(`docs/plan/bench/2026-06-11-pi5-p7-experiments.md`), with two findings the
first run forced: (1) the z-scale must be **quasi-NB** (pooled inverse-dispersion
1/r̂ from window moments — self-reduces to Poisson on equidispersed data); the
pure Poisson scale FAILED the overdispersed hold-out (0.83× — worse than the
ratio baseline). (2) **k-ring 0** (per-cell EB z + BH) for movers — Gi* k-ring-1
smoothing dilutes isolated spikes and lost on both variants; Gi* k=1 remains the
tool for *spatially clustered* heatmap hot-spots, applied there only. Adopted:
EB(Gamma MoM prior) + quasi-NB z + BH q=0.05; measured 3.76× FPR reduction at
matched TPR (Poisson), 1.81× with FDR held at 0.024 on the NB hold-out.

## ADR-22 — Multi-lane fan-out & region-lane metasearch (Phase 8 / Phase 10)

**Decision (operator, 2026-06-11).** `compare=vantages` (Phase 8) fans the same
query over **{direct, anon} only**, via a post-hoc orchestrator: each lane
resolved independently and fail-closed per §12.1; per-lane results compared AFTER
both complete; **zero shared state** (no shared-cache write of compare responses,
no bandit reward from the anon half, anon cache stays ephemeral); randomized
inter-lane jitter (default-on) decorrelates timing. Region lanes **stay
fetch-only** in v0.3.0 (planner refuses them a metasearch backend today — that
stands). The future architecture for regional vantage search is **option (b):
per-region SearXNG sidecars bound to WireGuard interfaces** — one cgroup
(384–512MB) each — implemented only as a Phase-10 candidate after an explicit
budget row and operator sign-off. **Option (a) — routing the direct sidecar's
egress through a WG netns — is REJECTED**: it turns a routing mistake into a
§12.5-invariant-1-class leak (wrong-lane egress), the exact failure family the
lane design exists to prevent.

**Tradeoff.** Two-lane divergence now (cheap, zero egress-architecture change) vs
waiting for ≥3 vantages; each region sidecar costs real RAM on the 8GB Pi
(risk #19), so scope stays closed until a region lane exists with a budget.

**Status: CONFIRMED (operator).**

**P7-exit amendment (suite-12 findings, as built):** (1) both compare halves
also **pin instance-default engines** (`pin_engines`) — the probe measured the
ε-greedy bandit's arm churn at p90 JSD 0.667 WITHIN the direct lane alone,
which would drown any vantage signal; pinning makes the halves differ by
vantage only, and removes bandit reads/rewards from compares entirely.
(2) The deployment's measured fixed-engine noise floor is **p90 0.30**
(`search.compare_noise_floor_p90`; anon lane, 336 pairs, mean 0.096
CI [0.082, 0.110]) — the per-request `exceeds_floor` reference. (3) Both
halves bypass the query caches (`bypass_cache`): a cached half would compare
different points in time. Full data: `bench/2026-06-11-pi5-p7-exit.md`.

## ADR-23 — QPP confidence methodology (Phase 8)

**Decision.** Post-retrieval query-performance prediction: **NQC** (normalized
top-k score deviation) + **Clarity** (KL divergence of the top-k language model
vs the collection model), fused into one calibrated `confidence` block with the
contributing signals exposed. Conformal risk control (distribution-free coverage)
is explicitly **deferred to Phase 10** — it needs a stable calibration set that
the Phase-8 eval extension only begins to accumulate.

**Tradeoff.** Cheap (O(k), ≤1ms), well-studied predictors with modest correlation
(gate: Spearman ρ ≥0.25 vs per-query nDCG@10) vs waiting for stronger but heavier
calibration machinery. Modest-but-honest beats absent.

**Status: CONFIRMED.**

## ADR-24 — Per-decision routing log for offline policy evaluation (Phase 9)

**Decision (operator, 2026-06-11 — privacy sign-off).** To enable IPS/DR offline
evaluation of routing policies, log one redb row per **direct-lane** routing
decision: `{intent class, query-length bucket, language, time-of-day bucket,
geo-filter-present flag, arm, propensity, reward}`. **No query text. No IPs. No
timestamps finer than the time-of-day bucket.** 30-day TTL sweep; k-anonymity
floor (bucket combinations seen <5 times/24h are generalized before write); an
operator wipe path; ≤20MB disk cap; **anon-lane decisions are never logged**
(the §12.4 firewall extends to the log). The privacy policy is amended in Phase 9
to disclose exactly this (bucketed routing metadata ≠ query logging — the
"no query logging" promise refers to query text/IPs and is preserved); the
privacy smoke canary extends to the log file.

**Why now:** today's ε-greedy has **known propensities** (ε/K for explore arms,
1−ε+ε/K for the greedy arm) — logging them makes the incumbent policy the
logging policy for free, so candidate policies can be evaluated offline
(doubly-robust) before any live traffic shifts.

**Tradeoff.** Estimator power (coarse features, single-operator traffic — risk
#23: OPE may stay inconclusive forever) vs the privacy surface (risk #20). The
sunset rule lives in ADR-25.

**Status: CONFIRMED (operator). Implemented 2026-06-12 (v0.3.0): substrate
shipped default-OFF (`searx.decision_log`) — 13-byte rows in egress.redb,
k-floor/TTL/cap/wipe in `meridian-searx/src/decision_log.rs`, logging at the
bandit-reward site (anon unreachable by construction), propensities captured
at choice time. privacy.md + operator-manual amended in the same train. The
flag-flip decision is the v0.4.0 exit (ADR-25).**

## ADR-25 — Contextual routing policy & ship gate (Phase 9)

**Decision.** Linear **Thompson sampling** over a ~20-dim coarse context vector
(the ADR-24 buckets, one-hot), same 3 arms as today, behind the existing
choose/reward interface; **feature-gated, default OFF**. It is enabled only if
doubly-robust OPE on ≥10k logged decisions shows a reward uplift whose 95% CI
excludes zero. **Explicit sunset rule:** if at the 10k-decision review (or 60
days, whichever first) the CI straddles zero or the minimum detectable uplift
exceeds the CI width, the experiment is declared inconclusive in the phase-exit
note, ε-greedy is retained, and the log TTLs out. No silent limbo.

**Rejected alternatives** (Pi 5 cost/assumption tests): deep/neural routers (no
training data, no GPU, unexplainable), MCTS planning, full bandits-with-knapsacks
(NP-hard in general; the shed ladder already gates the action set as hard
constraints).

**Tradeoff.** Exploration variance + cold-start regret vs ε-greedy's known-good
simplicity; the honest possible outcome is "never ships," and the plan budgets
for that.

**Status: CONFIRMED (gate), implementation Phase 9. Estimator precondition
met 2026-06-12: suite 14 PASS — IPS rel. bias 2.0%, DR 0.74% (<5% gate)
against synthetic truth under the shipped ε-greedy logging policy, DR tighter
than IPS (bench/2026-06-12-pi5-p9-ope.json). Policy + gate machinery
implemented DARK 2026-06-12: linear TS (`meridian-searx/src/contextual.rs`,
26-dim one-hot, hand-rolled Cholesky, MC propensities — a documented
imprecision vs ε-greedy's exact values), `searx.contextual_policy` default
OFF and refusing to start without the decision log, and the gate report at
`GET /v1/decision-log/ope` (temporal 80/20 split, DR uplift bootstrap CI,
verdicts pass/inconclusive/negative/insufficient_data). The flip decision
remains the v0.4.0 exit, on ≥10k live decisions or the 60-day sunset.**

## ADR-26 — Value-of-information fetch & stopping (Phase 9)

**Decision.** Deep-mode fetch-candidate selection (and the ingest frontier)
adopts **Pandora's-box reservation values** (Weitzman): each candidate gets a
reservation index from (estimated marginal value, fetch cost); candidates are
opened in decreasing index order and the walk stops when the best observed value
exceeds the next index. Marginal value is estimated **without any generative
model**: novelty = 1 − max MinHash similarity vs already-fetched set (Phase-7
sketches) + embedding-coverage gain; cost = lane-aware expected latency. Emits
`search_stopped_because` + `estimated_marginal_gain_remaining` in the `analysis`
block. Guard: suite-15 replay must show median `independent_source_count` does
not drop (a fetch policy that optimizes relevance by starving dissenting sources
is a regression — risk #22).

**Tradeoff.** O(n log n) candidate ranking + cheap novelty estimates vs ≥25%
fewer fetches at equal nDCG@10 (the gate); rejected MCTS as unjustified when the
inspection-cost structure has a provably optimal index policy.

**Status: CONFIRMED (design) — with one measured amendment. Suite-15
(2026-06-12, hermetic replay, tuning/hold-out seeds): the PANDORA stopping
rule is the wrong objective for page-level ranking — Weitzman optimizes the
single best find, while nDCG over the page is additive, and the replay
measured the walk starving 2 of 3 subtopic clusters after its first find.
The shipped selector is therefore the additive-objective greedy
(`meridian-fetch::voi::additive_walk`: open while expected net marginal
value `p·(novelty-discounted gain) − cost` > 0), which is near-optimal for
the submodular page objective; `pandora_walk` is retained for the
single-best regime (answer mode, Phase-10 candidate). Suite-15 GATE PASS on
the hold-out with frozen constants (β0=0.2, β2=0.3 from the tuning sweep):
nDCG@10 0.7411 vs fetch-all 0.7434 at 30.5% fewer fetches, median fetched
clusters 3 vs rank-greedy 2 (bench/2026-06-12-pi5-p9-voi.json). Deviations
recorded: the ingest-frontier hook is DEFERRED (no in-process frontier queue
exists to reorder); embedding-coverage joins the value model at planner
wiring time (the hermetic replay has no embedding space). Planner wiring SHIPPED
2026-06-12 in the same train as this note: `fetch_budget` (deep + direct
only, capped, never with compare), the `analysis` block, the standard-ladder
fetch path with in-RAM-only usage, inv19 (fails safe without a CE) and the
privacy.md/api.md/operator-manual disclosures. Device re-validation of the
deep ≤2.5s gate is the v0.4.0 exit row.**

---

## Environment-driven ADRs (not in the spec)

## ADR-D1 — Dev-on-target deviation: local check, CI builds

The dev machine IS the deployment Pi, so SPEC §2's "no toolchains/source trees on
the device" cannot hold verbatim during development. **Operator-approved
deviation:** local `cargo check`/`clippy`/`fmt` only (target dir ≤1.5GB,
regenerable, pi-cleanup-safe); every build/test/cross-compile/release runs in
GitHub Actions; `cargo build --release` is never run locally. The *deployed
appliance* artifacts (images) remain 100% CI-built — the spec's intent (no
compilation in the serving path, no build debris in the budget) is preserved.
Measured baseline: scaffold check+clippy = 255MB target, ~11s.

## ADR-D2 — Dual-profile resource budgets

The first device has no NVMe and ~7GB free on a shared 29.7GB SD card. **Profile
F** (SPEC §6 verbatim: 1M docs/8GB/NVMe) remains the published sizing and absolute
ceiling set; **Profile R** (100k docs/≤3GB, SD-aware) is what runs on this card.
Full numbers in `02-budgets.md`. The architecture is identical in both; only
capacity knobs and tripwire thresholds differ.

---

## Summary table

| ADR | Topic | Status |
|---|---|---|
| 01 | 16K-page kernel supported | CONFIRMED (bench smoke gate) |
| 02 | musl vs gnu / ort | **CHALLENGE → staged: musl now, tract-vs-ort bake-off at Phase 3** |
| 03 | Extraction bake-off | **CHALLENGE → dom_smoothie presumptive** |
| 04 | Arti embedded + own SOCKS front-end | CONFIRMED (impl. choice recorded; MPL-2.0 election) |
| 05 | Tantivy | CONFIRMED (MSRV 1.86; merge-by-docs note) |
| 06 | model2vec-rs | CONFIRMED |
| 07 | USearch | CONFIRMED (**fallback ladder amended**) |
| 08 | mimalloc | CONFIRMED |
| 09 | RRF/LTR/CE | CONFIRMED (upstream INT8 arm64 artifact exists) |
| 10 | h3o/geo | CONFIRMED (petgraph page_rank built-in) |
| 11 | SearXNG ×2 | CONFIRMED |
| 12 | robots.txt crate | **CHALLENGE → texting_robots** |
| 13 | Moka/redb | CONFIRMED |
| 14 | reqwest lanes | CONFIRMED (+`interface()` upgrade) |
| 15 | GDELT transport | CONFIRMED (HTTP+MD5 caveat) |
| 16 | telemetry/redaction | CONFIRMED (layered redaction architecture) |
| 17 | 15 crates, dep-rule fix | CONFIRMED (spec-text corrections) |
| 18 | sketching + derivation clusters | CONFIRMED (suite-9 run 2026-06-11: containment k=4/128/τ=0.3; evidence default-ON) |
| 19 | derived-structure deletability | CONFIRMED (standing rule) |
| 20 | additive API blocks + per-block `schema` | CONFIRMED |
| 21 | EB + quasi-NB z + BH trends; Gi* for clustered heatmap only | CONFIRMED (suite-10 run 2026-06-11: k-ring 0 for movers) |
| 22 | two-lane compare now; region sidecars = Phase-10 option (b); option (a) rejected | CONFIRMED (operator 2026-06-11) |
| 23 | QPP confidence (NQC+Clarity; conformal deferred) | CONFIRMED |
| 24 | per-decision routing log (coarse buckets, TTL, k-anon, anon never logged) | CONFIRMED (operator 2026-06-11) |
| 25 | linear-TS routing behind DR ship gate + sunset rule | CONFIRMED (gate) |
| 26 | Pandora's-box VoI fetch/stopping + diversity guard | CONFIRMED (design) |
| D1 | dev-on-target | operator-approved deviation |
| D2 | dual-profile budgets | operator-approved deviation |

**Sign-off requested on:** ADR-02 (staged musl/ort), ADR-03 (dom_smoothie
presumption), ADR-07 (fallback ladder), ADR-12 (texting_robots), and the MPL-2.0
license election in ADR-04. Everything else implements the spec as written.

**Post-v0.1.0 sign-offs already taken (2026-06-11):** ADR-18 (evidence block
default-on), ADR-22 (two-lane compare; region metasearch deferred to Phase 10,
option b), ADR-24 (decision log acceptable with the listed safeguards). ADR-18/21
constants were fixed the same day by the suite-9/-10 runs
(`bench/2026-06-11-pi5-p7-experiments.md`).
