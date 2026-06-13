# T5 — Research Track K: Hailo-8L NPU offload

Status: DRAFT workpaper — proposes, does not decide. As-of: v0.6.0 / 2026-06-13.
Scope rule honored: read-only repo + read-only device queries (`hailortcli
fw-control identify`, `dpkg -l`, `lspci -vv`, `ls /usr/share/hailo-models`,
`getconf PAGESIZE`, `vcgencmd get_throttled`). No installs, no model loads, no
benchmarks were run.

Baseline of record: `docs/research/2026-06-v060-review/01-rebaseline.md` §4
(lines 52–71). The repo has **zero** Hailo/NPU awareness (`grep -ri hailo`
over crates/docs excluding this review tree: no hits — re-verified this
session; consistent with 01-rebaseline.md:63).

## 0. Device ground truth (re-verified live, 2026-06-13)

| Fact | Value |
|---|---|
| Device | HAILO-8L AI ACC M.2 B+M KEY MODULE EXT TMP, arch `HAILO8L`, fw 4.23.0 (`hailortcli fw-control identify`) — 13 TOPS INT8 class (01-rebaseline.md:58) |
| Runtime stack | `hailort` 4.23.0, `hailort-pcie-driver` 4.23.0, `hailo-tappas-core` 5.1.0, `python3-hailort` 4.23.0-1, metapackage `hailo-all` 5.1.1 (`dpkg -l`) |
| Device node | `/dev/hailo0` present, mode **crw-rw-rw-** (world-writable — ops note in §5) |
| PCIe | `0001:04:00.0`, kernel driver `hailo`; link **negotiated Gen2 (5GT/s) x1 ≈ 400–450 MB/s effective**; Gen3 ≈ 900 MB/s is a config.txt knob (01-rebaseline.md:59; unprivileged `lspci` hides LnkSta — the rebaseline capture is the record) |
| Kernel | 6.18.33+rpt-rpi-2712, **16K pages** (`getconf PAGESIZE` = 16384) — the Hailo driver demonstrably runs on this kernel: module bound, `/dev/hailo0` live, fw identify succeeds. ADR-01's 16K concern (00-adr.md:17) is already answered for the driver by existence proof |
| Models on disk | Vision CNNs only (yolo/resnet/scrfd HEFs in `/usr/share/hailo-models/`) — nothing retrieval-relevant is compiled |
| Thermal | `throttled=0x0` at capture; module is the extended-temperature SKU |

CPU baselines to beat (Profile R, measured): CE deep-rerank stage p50 **204ms**
@ top-20/batch-4 (phase-exits/p3.md:7); deep p50 **2173ms**, answer p50
**3224ms @ cap 16 → 2502ms @ cap 8** (bench/2026-06-13-pi5-p10-device.md:22–23;
bench/2026-06-13-pi5-answer-cap-study.md); embed **42.9k docs/s** batch-32
(meridian-embed/src/lib.rs:2–4); ANN **0.45ms p50 @100k / p99 1.73ms @1M
ef=128**; BM25 0.48ms; fusion 0.144ms; RSS plateau ~250–261MB in a 3GB cgroup
(02-budgets.md:70–90, 119, 126).

## 1. Workload characterization — what could move, what cannot

**CE rerank (the prime candidate).** `ms-marco-MiniLM-L-6-v2` INT8 ONNX, 23.2MB
(00-adr.md:285–287), 256-token pairs at fixed padding, batch 4
(meridian-rerank/src/lib.rs:129–130, 151), top-20 candidates under a 1.5s stage
deadline (planner.rs:953–980; lib.rs:98–103). Measured stage p50 204ms — and
this cost is corpus-size-independent (p3.md:51–53). Answer mode adds a second
CE workload: per-fetch sentence-aligned passages, `answer_passage_cap` now 8
(config.rs:288, 306; planner.rs:1186–1196), historically ≤32 per ADR-29
(00-adr.md:786–788). The cap study attributes **~700ms of the answer p50 to CE
realization alone** ("the per-fetch batch is the dominant answer-mode latency
term", answer-cap-study.md:15–16). This is a dense, static-shape, INT8 GEMM
pipeline — exactly the shape an NPU wants.

**Query embedding (not a candidate as-is).** potion-base-8M via model2vec-rs is
a *static* embedding — table lookups + mean pooling, no transformer
(meridian-embed/src/lib.rs:1–4; ADR-06, 00-adr.md:205–216). At 42.9k docs/s it
is "never the ingest bottleneck" by design. There is nothing to offload. A
*transformer* embedder would be a **new workload** (a quality upgrade the NPU
might make affordable — K2), not an offload of an existing cost.

**ANN (structurally not offloadable).** usearch HNSW, cosine over int8
(meridian-vector/src/lib.rs:41–49, 91–99; ADR-07). HNSW search is
data-dependent graph traversal: each hop reads a neighbor list, computes a
handful of distances, and *branches* on the results to pick the next node. It
is irregular, memory-latency-bound pointer chasing with a dynamic control path.
The Hailo programming model executes **statically compiled dataflow graphs**
(HEFs produced offline by the Dataflow Compiler); there is no representation
for data-dependent traversal, and no general-purpose cores to emulate one.
This is an architectural mismatch, not a tuning problem — stated up front so
"use the NPU to speed up search" is evaluated honestly (the steel-manned
version is K4, brute-force scoring, which dies on arithmetic).

## 2. FLOPs/latency model for CE offload

**GEMM FLOPs per pair.** MiniLM-L6: 6 layers, hidden H=384, FFN 4H=1536, seq
L=256 (lib.rs:129). Per layer, counting multiply-accumulates (MACs):

- QKV projections: 3 · L·H·H = 3 · 256 · 384 · 384 = **113.2M MACs**
- Attention output projection: L·H·H = **37.7M MACs**
- QKᵀ scores: L·L·H = 256·256·384 = **25.2M MACs**
- scores·V: **25.2M MACs**
- FFN (two GEMMs): 2 · L·H·4H = 2 · 256 · 384 · 1536 = **302.0M MACs**

Per-layer ≈ 503M MACs → ×6 layers ≈ **3.02 GMACs ≈ 6.04 GOPs per pair**
(embedding lookup + classifier head < 0.2M, negligible).

**What 13 TOPS implies.** Marketing TOPS counts MAC = 2 ops. At a *realistic*
20–40% utilization for a transformer encoder on a dataflow NPU tuned for CNNs
(attention's batched small GEMMs and softmax utilize the array poorly):
effective 2.6–5.2 TOPS → per pair 6.04 GOPs / (2.6–5.2 T) = **1.2–2.3ms
compute**; batch-20 = **23–47ms**.

**PCIe transfer per pair (negligible — shown).** In: 256 token ids (vocab
30522 fits uint16) + 256 mask + 256 type bytes ≈ **≤1.5KB**; out: 1–2 logits ≈
8B. At 400 MB/s (Gen2 x1): 1.5KB / 400MB/s ≈ **3.8µs/pair**, batch-20 ≈ 75µs —
0.04% of the 204ms stage. I/O is not the constraint *for activations*.

**The constraint that can bite: context fit.** The 23.2MB INT8 weights likely
exceed Hailo-8L single-context on-chip memory, in which case DFC splits the
graph into contexts whose weights stream from host per inference pass (the fw
identify string "extended context switch buffer" exists precisely for this).
Worst case, one full weight re-stream per batch pass: 23.2MB / 400MB/s ≈
**58ms (Gen2 x1)** or / 900MB/s ≈ **26ms (Gen3)** added per batch.

**Stage estimate (top-20).** CPU-side tokenization stays (1–4ms for 20 pairs,
HF tokenizers) + transfer 0.1ms + HailoRT scheduling ~1–5ms + compute 23–47ms
+ 0–58ms context streaming:

| Scenario | Stage estimate | Speedup vs 204ms |
|---|---|---|
| Single-context HEF | ~30–55ms | **~4–7×** |
| Multi-context, Gen2 x1 | ~85–110ms | **~2×** |
| Multi-context, Gen3 x1 | ~55–80ms | **~2.5–3.7×** |

**Headline: 2–7× on the CE stage, with 3–6× the defensible planning range.**
Equivalently, at the *same* 204ms budget: rerank depth top-40 to top-80 (×2–4).
Loud assumptions: (a) **DFC can compile this BERT-class encoder at all** —
gated on the C4 toolchain ledger (01-rebaseline.md:62 cites `02-competitive.md`
C4, which is **not yet in the tree** at draft time — treat compilability as
UNVERIFIED); (b) **INT8-on-Hailo score parity with INT8-on-ort** is an
empirical question (different quantization pipelines) — the 100-query eval +
suite-13 harness and suite 18 are the judges, not an assumption; (c)
utilization 20–40% is an engineering estimate, not a measurement — suite 4
(`rerank`, 04-bench-plan.md:23) is the instrument that turns it into one.
Note: suite 4 never recorded ms/pair at batch 1/4/8 (tract couldn't load the
model at Phase 0, bench/2026-06-10-pi5-report.md:19; P3 recorded the stage
total instead) — the derived CPU figure is 204ms/20 ≈ **10.2 ms/pair** at
batch 4, and the answer path's small per-fetch batches measure worse (~700ms /
~16 pairs ≈ **~44 ms/pair** realized, answer-cap-study.md:8–16).

**Where the win actually lands.** Deep p50 is 2173ms and network-dominated:
cutting 204→~50ms moves deep p50 by ~7%. The budget-row prize is the **answer
row**, where CE is the dominant controllable term (§3 K3), and **depth/quality
at constant latency** for K1.

## 3. Candidates

### K1 — CE rerank offload (HailoReranker)

| Field | Value |
|---|---|
| Proposal | Compile `ms-marco-MiniLM-L-6-v2` INT8 to HEF (offline, x86_64 DFC); implement `HailoReranker` behind the existing `Reranker` seam — the planner already holds `reranker: Arc<Reranker>` (planner.rs:251) and the crate already models unavailability (`Reranker::unavailable()`, `available()`, lib.rs:71–93). Fail-open to the CPU ort path (or to LTR order, the existing ladder) when `/dev/hailo0` is absent, busy, or the module is hot |
| Meridian problem | Deep rerank depth is pinned at top-20/batch-4 by A76 cost (ADR-09, 00-adr.md:281–283); CE is the largest quality jump in the stack (lib.rs:1–2) but its reach is latency-rationed |
| Math formulation | §2 model: 6.04 GOPs/pair; stage = tok + PCIe + ctx-stream + compute; speedup 2–7× or depth ×2–4 at constant budget |
| Expected gain | Rerank stage 204ms → ~30–110ms, OR top-40–80 rerank at today's budget; deep p50 −5–8%; frees ~0.8 core-seconds/query of CPU (K5) |
| Complexity | High first time: DFC pipeline + HEF artifact + FFI + scheduler integration (§4). Code-side small: one trait impl + config |
| Pi-5 cost | RAM: HailoRT lib + buffers est. 50–150MB *replacing/alongside* the 250MB ort-sessions row (02-budgets.md:58) — needs its own budget row; weights live on-device/streamed, not in RSS. Disk: HEF ≈ 25–30MB in the image. Power +1.5–2.5W active |
| Privacy impact | Query text crosses PCIe as token ids; stateless inference; see §5 invariants (lane-blindness, no residue) |
| Required data | None new — suite-4 pairs, the 100-query eval, suite-18 replay corpus all exist |
| Evaluation | Suite 4 (rerank ms/pair batch 1/4/8, CPU vs NPU); 100-query eval + suite-13 harness for score parity (nDCG@10 delta); suite 6 thermal; suite 2 untouched |
| Implementation location | `crates/meridian-rerank` (new `hailo-backend` feature, gnu image — ADR-02 precedent); `deploy/` device mapping; `train/`-adjacent DFC scripts |
| Priority | **P1** — the only candidate with a measured CPU cost worth beating |
| Acceptance | NPU ms/pair ≤ 0.5× CPU at batch 4 AND eval nDCG@10 within noise of ort CE AND fail-open proven (yank `/dev/hailo0` mid-run → degrades, never errors). Kill: DFC cannot compile the encoder (C4); or multi-context streaming puts stage p50 > 150ms at Gen2 *and* the operator declines the Gen3 knob; or parity fails the eval |

### K2 — Transformer embedding upgrade (NPU-affordable, NOT an offload)

| Field | Value |
|---|---|
| Proposal | Replace potion static embeddings with a small transformer encoder (MiniLM-class, seq 128) for *document + query* embedding, batched on the NPU at ingest |
| Meridian problem | Static embeddings cap dense-lane quality; hybrid beats BM25 by 0.04 nDCG (02-budgets.md:118) — ceiling unknown |
| Math formulation | Seq-128 encoder ≈ 1.43 GMACs/doc (half-seq variant of §2: QKV 56.6M + proj 18.9M + attn 25.2M + FFN 151M per layer ×6) → 0.55–1.1ms/doc at 2.6–5.2 effective TOPS → **~900–1800 docs/s** NPU-bound. Ingest gate is ≥50 docs/s (02-budgets.md:79): 18–36× margin. Re-embed 1M docs ≈ **9–18min** NPU time + CPU tokenization |
| Expected gain | Unknown until measured — that is the point of the gate. Plausible hybrid nDCG movement; zero gain is a live outcome (potion already beat the bar that mattered) |
| Complexity | High: **schema break** — store is 256-d int8 (02-budgets.md:30; dims probed from the embedder, meridian-embed/src/lib.rs:38, vector/src/lib.rs:42); MiniLM-class outputs 384-d → full index rebuild; disk/RAM rows re-derive (506MB measured @1M 256-d → ~760MB @384-d; the 600MB disk row busts) |
| Pi-5 cost | Index rebuild I/O (SD endurance, risk #7); +50% vector RAM/disk; NPU contention with K1 at ingest time (scheduler arbitrates) |
| Privacy impact | Document text crosses PCIe at ingest; queries at search time — same §5 invariants |
| Required data | The operator corpus + existing eval set; no new labels |
| Evaluation | Suite 1 (embed throughput, NPU arm); suite 2 (recall\@10 ≥0.95 on the new vectors — the gate that caught rung 2, 00-adr.md:253–268); 100-query eval hybrid ≥ BM25 + a positive margin over potion-hybrid; suite 13 QPP unchanged |
| Implementation location | `crates/meridian-embed` (backend enum), `meridian-vector` (dims), ingest pipeline, budgets rows |
| Priority | **P2** — real quality hypothesis, but pays a schema migration for an unproven gain; sequence after K1 proves the toolchain |
| Acceptance | Hold-out nDCG@10 ≥ potion-hybrid + 0.02 AND suite-2 recall ≥0.95 AND ingest ≥50 docs/s sustained. Kill: quality delta < 0.02; or rebuild RAM/disk busts Profile R rows; or DFC fails the encoder (shared kill with K1) |

### K3 — Answer-mode passage scorer on NPU

| Field | Value |
|---|---|
| Proposal | Route the per-fetch passage-CE batch (planner.rs:1186–1196) through the same HEF/session as K1 |
| Meridian problem | CE realization is **the** dominant answer-mode latency term — the cap study bought the 3.0s row back only by halving passage depth 16→8 (answer-cap-study.md; 02-budgets.md:148), and position telemetry showed the cost: index-11 winners realize shallower passages |
| Math formulation | ~16 pairs/query @ measured ~44ms/pair realized → ~700ms CPU; NPU ~2–5ms/pair incl. overhead → ~30–80ms. Answer p50 2502ms → est. **~1.9–2.1s** at cap 8, or **cap 16 restored under the 3.0s row** (3196 − ~650 ≈ 2.5s) |
| Expected gain | Either ~0.5s answer p50 cut or 2× passage depth at the same row — un-paying the cap study's measured quality price |
| Complexity | Low *marginal* if K1 lands: same model, same backend, second call site; HailoRT model scheduler multiplexes K1+K3 on one configured network |
| Pi-5 cost | None beyond K1 |
| Privacy impact | Fetched page text crosses PCIe — same class as query text; RAM-only discipline (ADR-29, 00-adr.md:780–782) must extend to NPU buffers (§5) |
| Required data | Suite-18 replay corpus (exists) |
| Evaluation | Suite 18 (hit-rate ≥ baseline +10pp must still hold, 04-bench-plan.md:118); the answer budget-row re-measure (n=16 interpolated medians, the device protocol); cap-16-on-NPU arm vs cap-8-on-CPU |
| Implementation location | planner.rs answer path; config (`answer_passage_cap` default re-sweep) |
| Priority | **P2, conditional on K1** — standalone it cannot justify the toolchain; with K1 it is nearly free and has the largest budget-row payoff |
| Acceptance | Answer p50 ≤2.6s at cap 16 on NPU AND suite-18 hit-rate holds. Kill: K1 killed; or scheduler contention pushes deep p50 over 2.5s |

### K4 — NPU brute-force INT8 scoring for FILTERED dense search — expected REJECT

| Field | Value |
|---|---|
| Proposal | The honest "use the NPU to speed up ANN" test: exact dot-product scan of a filtered candidate subset (10k–100k × 256-d int8) on the NPU, addressing R9 (ANN is skipped under geo/time filters, 01-rebaseline.md:24) |
| Meridian problem | Real: filtered queries lose the dense lane entirely (ADR-10; api.md) |
| Math formulation | **CPU baseline:** 100k × 256-d int8 = 25.6MB streamed; A76 sdot ≈ 76.8 GMAC/s/core theoretical, so compute (25.6 GMACs… 100k·256 = 25.6M MACs) is trivial — the scan is memory-bound: 25.6MB / 8–16 GB/s ≈ **1.6–3.2ms** (4 cores, plus top-k heap → call it 2–4ms; 10k subset ≈ 0.3ms). **NPU:** vectors are per-query data, not weights — they must cross PCIe: 25.6MB / 400MB/s ≈ **64ms transfer alone** (Gen3: 28ms), 16–30× worse than the *entire* CPU scan before any compute |
| The programming-model question, answered plainly | Pre-staging the matrix on-device means making it HEF *weights* — HEFs are compiled offline and immutable; re-"compiling" per ingest batch is absurd, and HailoRT exposes **no generic matmul-as-a-service API**: everything runs through compiled network graphs. Hailo cannot economically stream non-NN bulk data per query. **It does not support this workload.** |
| Expected gain | Negative. **Verdict: REJECT with numbers** |
| Complexity / Pi-5 cost / Privacy / Data | Moot |
| Evaluation | None warranted; the arithmetic is the evaluation |
| Implementation location | The surviving alternative is **index-side filtered ANN on CPU** (usearch filtered search predicates, or the 2–4ms exact scan above for ≤100k subsets — which *is* affordable on CPU and is a separate, NPU-free candidate for the R9 gap, referred to the main review) |
| Priority | **REJECTED** |
| Acceptance/Kill | Killed a-priori by transfer arithmetic; reopen only if a future runtime exposes resident generic GEMM with on-device data updates |

### K5 — Thermal/CPU headroom and joules-per-result

| Field | Value |
|---|---|
| Proposal | Account K1/K3's side-effect: CE moves from 4 A76 cores to a 1.5–2.5W accelerator |
| Meridian problem | Not a current failure — thermal passes with margin (75.7°C max, no throttle, 02-budgets.md:85; soak ≤61°C). This is headroom accounting, not a fix |
| Math formulation | CPU CE: ~0.2s × ~5W incremental ≈ **~1.0J**/deep query (answer: ~0.7s CE ≈ 3.5J). NPU: ~0.05s × 2W ≈ **0.1J** + host margin. Frees ~0.8 core-seconds/deep-query for BM25/fusion/tokenization and co-tenants |
| Expected gain | ~10× CE-stage energy cut (estimate); lower thermal duty under QPS load; better p99 under concurrency (rerank no longer competes with the rayon pool) |
| Complexity | None beyond K1 instrumentation |
| Pi-5 cost | +1.5–2.5W NPU active power (net likely negative total board power during CE) |
| Privacy impact | None new — but see §5 timing invariant: power/utilization is an observable |
| Required data | `vcgencmd pmic_read_adc` rails + suite-6 loop |
| Evaluation | **Suite 6 (thermal)** extended with PMIC power sampling: 10-min deep-mode loop, CPU-CE arm vs NPU-CE arm; report °C, throttle flags, J/query |
| Implementation location | `meridian-bench` suite 6 (bench-device feature) |
| Priority | **P3** — measure as part of K1's exit, never standalone |
| Acceptance | NPU arm strictly ≤ CPU arm on temp AND J/query. Kill: n/a (it is a measurement); if NPU arm is *hotter* (shared M.2 thermal envelope, EXT TMP module sits near the SD/board), record it as a K1 cost |

## 4. Toolchain & ops reality

1. **DFC is x86_64-only, offline.** The Dataflow Compiler cannot run on this Pi
   (01-rebaseline.md:62). **No x86_64 build host was verified in this
   environment** — CI builds amd64 images (02-budgets.md:150), so a GitHub
   runner *could* host the DFC docker image, but the DFC is a large licensed
   SDK download; treat "a working DFC pipeline exists" as a hard, unverified
   dependency and the first gate of K1. The C4 ledger (02-competitive.md, not
   yet in tree) owns the "can DFC compile BERT-class encoders" question.
2. **HEF ↔ HailoRT version coupling.** Device runs HailoRT 4.23.0 + driver
   4.23.0 (verified). HEFs are compiled for a runtime version family; a casual
   `apt upgrade` of `hailo-all` can orphan the HEF. The repo idiom exists:
   digest-pinning (searxng) — pin the hailort debs and record the pair
   (HEF build-id, runtime version) in the model manifest (models are already
   manifest-pinned, 02-budgets.md:27).
3. **Rust integration.** Options: (a) **libhailort C API via FFI** behind a
   `hailo-backend` cargo feature — the natural landing, exactly ADR-02's
   precedent of a gnu-image-only feature (`ort-backend`, lib.rs:4–8;
   00-adr.md:80–87): the musl/scratch image stays neural-free and degrades,
   the gnu deep image gains the feature. libhailort is a shared lib → fine on
   distroless-cc/gnu, impossible in `FROM scratch` musl — which is already
   true of deep mode wholesale. (b) **python3-hailort sidecar**: violates the
   single-binary idiom, adds an IPC hop and ~100MB+ Python RSS, a second
   failure domain, and a privacy surface (query text over a local socket).
   Score: FFI strongly preferred; sidecar only as a throwaway feasibility
   probe. Container needs `/dev/hailo0` device-mapped — compatible with
   `cap_drop ALL`/`read_only` hardening via cgroup device allow; an explicit
   compose delta on the deep profile only.
4. **16K-page kernel.** The driver demonstrably runs on 6.18.33+rpt-rpi-2712
   at 16K pages (§0 existence proof). No ADR-01-class blocker.
5. **Device contention.** The Pi is shared (rpicam hailo postprocess is
   installed; other tenants exist). HailoRT's model scheduler multiplexes
   configured networks (K1+K3 share one model anyway); but a co-tenant vision
   pipeline can hold the device. Fail-open (K1 row) is therefore not optional
   polish — it is the design.
6. **PCIe Gen3 knob.** `dtparam=pciex1_gen=3` roughly doubles link bandwidth
   (matters iff the HEF is multi-context, §2). **Operator decision** — Q-item
   for 06-operator-questions; this review documents, does not decide.

## 5. Candidate-ADR sketch — "heterogeneous compute policy"

**Decision (proposed).** The NPU is an *accelerator, never a dependency*:

1. **Identical-semantics fallback.** Every NPU path has a CPU path with the
   same output contract; planner behavior is the existing ladder
   (`rerank_shed` → `rerank_unavailable` → partial `rerank_timeout`,
   planner.rs:956–1009) extended with backend provenance. If NPU and CPU
   scores are *not* bit-identical (they will not be — different quantization),
   the served semantics are "a CE score from the pinned model family", and the
   parity gate (K1 acceptance) bounds the divergence; a `rank_signals`-level
   backend tag keeps it explainable without a new degraded marker (degraded
   markers stay reserved for *capability* loss, matching precedent).
2. **No user-data residue.** HEF inference is stateless — weights are
   immutable, activations are transient device-DDR/SRAM scratch overwritten
   per inference; no on-device persistence exists for tensors. Verify, do not
   assume: confirm HailoRT service/driver logging emits metadata only (never
   tensor contents), and that no trace/profiling mode is compiled into the
   production image. Fetched-text passages (RAM-only by ADR-29) extend their
   discipline to host-side DMA buffers: freed and not recycled across queries
   without overwrite.
3. **Lane blindness (timing side-channel) — proposed invariant inv22.** Anon
   and direct lanes must be indistinguishable by NPU usage: same backend
   selection, same batch shapes (fixed 256-token padding already guarantees
   shape uniformity, lib.rs:173–176), no lane-conditional scheduling. Threat:
   `/dev/hailo0` is **world-writable (crw-rw-rw-, verified)** — any co-tenant
   process can observe device busyness and correlate NPU activity with anon
   egress timing. Mitigation is operator-side (udev group restriction) plus
   the invariant; the privacy smoke test gains a clause: NPU utilization
   pattern for an anon deep query ≡ direct deep query.
4. **Budget honesty.** NPU runtime RAM gets its own row next to "ort sessions
   250MB" (02-budgets.md:58); the HEF gets a manifest entry; suite 6 gains the
   power arm (K5).

## 6. Rejections (a-priori, with the bar each would have to clear)

- **LLM/decoder models on Hailo-8L.** Wrong architecture class: autoregressive
  decode needs KV-cache and dynamic shapes; Hailo-8L executes static dataflow
  graphs, and its on-chip memory cannot hold even a 100M-param decoder plus
  cache — official positioning routes GenAI to Hailo-10H-class parts (C4
  ledger pointer, 01-rebaseline.md:62). Independently dead: the review's
  no-generative-model constraint and ADR-29's "extractive only" honesty stance
  (00-adr.md:790–795).
- **Neural intent classifier.** The incumbent is a 103-line lexical heuristic
  at µs scale (R3, 01-rebaseline.md:18; ADR-02's "no neural cost",
  00-adr.md:76). NPU round-trip overhead alone (~ms of scheduling + PCIe) is
  ~1000× the incumbent's budget, before any quality argument — and the 4-class
  problem has no training data (the ADR-25 refrain). P3 at absolute best;
  effectively rejected.
- **NPU HNSW traversal.** §1's architectural mismatch: irregular,
  memory-latency-bound pointer chasing with data-dependent control flow has no
  static-dataflow compilation, and the CPU already answers in 0.45ms p50 —
  there is nothing to win even if it compiled. (The filtered-search gap R9 is
  real but is K4's question, and K4 dies on transfer arithmetic; the surviving
  alternative is index-side filtered ANN on CPU.)

## 7. Sequencing & open dependencies

K1 first (it proves DFC + FFI + parity + fail-open); K3 rides K1; K5 is K1's
power/thermal exit measurement; K2 only after K1 establishes the toolchain and
only with the schema-migration cost priced; K4 rejected. Hard blocker for the
whole track: the **DFC question (C4)** — no x86_64 DFC host is verified, the
C4 ledger is not yet in the tree, and BERT-class compilability on the 8L
(single- vs multi-context fit of a 23.2MB encoder) decides whether K1's
speedup is ~4–7× or ~2×. If C4 returns "cannot compile", the entire track
reduces to a documented no-go with this arithmetic on the record.
