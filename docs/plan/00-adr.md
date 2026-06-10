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

**Status: CHALLENGE → staged resolution APPROVED** (operator sign-off 2026-06-10).

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

**Status: CONFIRMED with fallback-ladder amendment.**

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

**Status: CONFIRMED.**

## ADR-10 — Geo: h3o, gazetteer fst, remote geocoder, optional GeoLite2

**Decision.** As specced. `h3o` 0.10 verified pure Rust with `grid_disk`,
`parent/children`, and lossless u64↔CellIndex conversion (the fast-field
representation). GeoNames cities15000 is 3.1MB zipped (under the 10MB fst budget).
`maxminddb` 0.28 active. Geocoder: public Nominatim @1 rps + redb cache (Q8
default). petgraph 0.8.3 has **built-in `page_rank`** (and parallel variant) for
the nightly domain-prior job — no hand-rolled implementation.

**Status: CONFIRMED.**

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
| D1 | dev-on-target | operator-approved deviation |
| D2 | dual-profile budgets | operator-approved deviation |

**Sign-off requested on:** ADR-02 (staged musl/ort), ADR-03 (dom_smoothie
presumption), ADR-07 (fallback ladder), ADR-12 (texting_robots), and the MPL-2.0
license election in ADR-04. Everything else implements the spec as written.
