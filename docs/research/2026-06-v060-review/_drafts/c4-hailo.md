# C4 — Hailo-8L NPU feasibility for retrieval workloads (text models)

Status: RESEARCH LEDGER — evidence with per-claim confidence. Access date: 2026-06-13.
Agent: C4. Builds on `01-rebaseline.md` §4 (device facts captured live on the Pi:
HAILO8L 13 TOPS INT8, HailoRT 4.23.0, PCIe Gen2 x1 negotiated ≈400–450 MB/s,
vision-only HEFs on disk, repo has zero Hailo awareness).

Labels: **CONFIRMED** = official Hailo source (hailo.ai, github.com/hailo-ai, Hailo
staff posts on community.hailo.ai). **INFERRED** = community/third-party evidence,
or my derivation from confirmed facts.

**Headline finding (changes the prior):** Hailo Model Zoo **v2.19.0 (tagged
2026-06-01)** added **`all_minilm_l6_v2` sentence-embedding HEFs compiled for
HAILO8L** — an official, downloadable MiniLM-L6 transformer running on exactly our
device class, with published accuracy and FPS. The 2024-era community position
("language models too large for Hailo-8L") is superseded for encoder-class models.

---

## 1. Dataflow Compiler (DFC)

| Claim | Evidence | Confidence |
|---|---|---|
| The Hailo-8/8L toolchain is the **DFC v3.x line**; the v5.x line (current v5.3.0, 2026-04-05) targets Hailo-10H/15 only | Model zoo README (master): "The Hailo-8 and Hailo-8L devices are supported on the Hailo Model Zoo v2.x branch, in combination with the Hailo Dataflow Compiler v3.x branch. The master branch is intended for Hailo-10 and Hailo-15 devices only." https://github.com/hailo-ai/hailo_model_zoo ; hailort README (master): "The `master` branch supports only the Hailo-10 and Hailo-15 device families. For Hailo-8, Hailo-8R, and Hailo-8L devices, please use the `hailo8` branch." https://github.com/hailo-ai/hailort | CONFIRMED |
| **Current DFC for Hailo-8L: v3.34.0**, paired with model zoo v2.19.0 (tag commit 2026-06-01: "Update to version v2.19.0") and badged "HailoRT (optional) 4.24.0"; Python 3.10/3.11/3.12 | README.rst at tag v2.19.0 — badges: "Hailo Dataflow Compiler-3.34.0", "HailoRT (optional)-4.24.0"; release notes: "Upgraded to Dataflow Compiler v3.34.0". https://github.com/hailo-ai/hailo_model_zoo/blob/v2.19.0/README.rst | CONFIRMED |
| **Transformer encoder architectures compile** for Hailo-8L: the zoo ships MiniLM-L6 (6-layer BERT-style encoder, self-attention + LayerNorm + GELU), CLIP/TinyCLIP/SigLIP *text* encoders (up to 85.6M params), ViT classifiers, and Whisper encoder/decoder | v2.19.0 HAILO8L docs: `HAILO8L_sentence_embedding_generation.rst`, `HAILO8L_text_image_retrieval.rst`, `HAILO8L_zero_shot_classification.rst`, `HAILO8L_automatic_speech_recognition.rst` (paths verified via GitHub API at the tag) | CONFIRMED |
| **LayerNorm**: supported (ONNX and TFLite parse paths), with specifically improved quantization | Hailo staff (Omria): "LayerNorm works great in TFLite … The Tensorflow/TFLite parser treats LayerNorm as a proper 'block' even when it gets broken down into smaller operations"; "We have specifically improved how LayerNorm gets quantized". https://community.hailo.ai/t/groupnorm-layernorm-support-in-qat-process/16572 (user in that thread still hit a parse error on a decomposed-TFLite variant — support has rough edges) | CONFIRMED (caveat INFERRED) |
| **GELU**: listed by staff as "GeLU (preview)"; Mish/Hard-Swish/SiLU/PReLU/Tanh/Exp/Sqrt supported; "No automatic operator transformation is available in Hailo DFC" (unsupported ops must be manually replaced or moved to host) | Hailo staff (Omria), supported-operators thread. https://community.hailo.ai/t/supported-operators/5046 | CONFIRMED |
| **Softmax with attention mask**: supported — the MiniLM model script calls `set_input_mask_to_softmax()`, i.e. the attention mask is a first-class HEF input wired into on-chip softmax | `hailo_model_zoo/cfg/alls/generic/all_minilm_l6_v2.alls` at v2.19.0: `set_input_mask_to_softmax()` | CONFIRMED |
| **MatMul (Q·Kᵀ, attn·V)**: supported, with a dedicated quantization correction (`matmul_correction … correction_type=zp_comp_block`) | same `.alls` file: `pre_quantization_optimization(matmul_correction, layers={matmul*}, correction_type=zp_comp_block)` | CONFIRMED |
| **Embedding lookup (vocab gather) does NOT run on-chip**: the zoo's MiniLM parser starts at the `embedding` / `masked_fill` nodes — i.e. the HEF input is the *post-lookup* hidden states (16x8x384 = seq 128 × hidden 384) plus a per-head attention-mask tensor (16x8x1536 = 12 heads × 128 × 128); token/position embedding lookup, pooling and L2-normalize are host-side | `cfg/base/all_minilm.yaml` at v2.19.0: parser `nodes: [[embedding, masked_fill], [last_hidden_state]]`, `input_shape: 16x8x384, 16x8x1536`, `output_shape: 1x128x384` | CONFIRMED |
| **Sequence length is fixed at compile time** (static shapes): zoo MiniLM = **128 tokens**, CLIP text = 77, SigLIP = 64. No dynamic sequence length; a different length means a different HEF | input shapes above + CLIP/SigLIP tables (`1x77x512`, `8x8x768`) in the HAILO8L docs at v2.19.0 | CONFIRMED (shapes) / INFERRED (no-dynamic-shapes generalization, consistent with all zoo entries) |
| **Model size limit**: no published byte threshold. Staff: "The way the compiler maps layers onto the hardware isn't a simple cumulative sum of weights + activations … there isn't a fixed byte-size threshold you can calculate manually beforehand" — use the profiler iteratively. In practice the zoo compiles 85.6M-param text encoders for 8L (multi-context, low FPS) | https://community.hailo.ai/t/hailo-8-ai-accelerator-chip-how-much-sram-does-it-have/19111 (staff: Michael); SigLIP rows in HAILO8L_text_image_retrieval.rst | CONFIRMED |
| **Host requirements: x86 only** — "The DFC is only available on x86 machines because it is a compiler used to create models, but it does not actually run the models"; "On Raspberry Pi devices, we only run the pre-compiled models … we do not use the DFC compiler itself on the RPi." Distributed as a Python `.whl` via the (registration-gated) Developer Zone | Hailo staff, https://community.hailo.ai/t/how-to-install-dataflow-compiler/1276 ; gating also evident at https://hailo.ai/products/hailo-software/hailo-ai-software-suite/ ("Sign in / Sign up is required" for downloads). RAM requirement not in any public source I could capture (the gated DFC user guide carries it); community workflows run it in Docker on commodity x86 ≥16 GB | CONFIRMED (x86-only) / INFERRED (RAM) |
| **Quantization workflow: PTQ with a small calibration set is the default**; QAT exists as an advanced path. MiniLM recipe: `calibration, batch_size=8, calibset_size=64` over an MTEB-ArguAna tfrecord, plus equalization; `optimization_level=0, compression_level=0` (no 4-bit compression used for MiniLM) | `.alls` + `cfg/base/all_minilm.yaml` (`calib_set: …mteb_arguana_val.tfrecord`) at v2.19.0; QAT thread above confirms QAT path exists | CONFIRMED |

## 2. Model zoo — text/NLP entries for HAILO8L

v2.19.0 `docs/public_models/HAILO8L/` is overwhelmingly vision, but text/NLP now exists
(all CONFIRMED from the tag):

| Entry | Models | Published numbers (host: i5-9400, **PCIe Gen3 x4**, room temp) |
|---|---|---|
| **Sentence embedding** (`HAILO8L_sentence_embedding_generation.rst`) | `all_minilm_l6_v2`, `all_minilm_l6_v2_v2a` (10.6M params, 2.9 GOPS, input seq 128 × 384) — precompiled HEF + profiler report downloadable from Hailo S3 | MTEB-style Retrieval@10: 61.7 float → **61.6 on-chip** (base); 96.4 → **94.7** (v2a tool-retrieval eval). **121 FPS @ batch 1, 492 FPS @ batch 8** |
| **Text-image retrieval** (`HAILO8L_text_image_retrieval.rst`) | CLIP RN50/RN50x4 text encoders, `clip_vit_b_16/32_text_encoder` (37.8M params), TinyCLIP text encoders (3–29M), `siglip_b_16` / `siglip2_b_32_256` text encoders (85.6M params, 11 GOPS) | e.g. clip_vit_b_32_text 90.6 → 89.3, **32.1 FPS @ b1 / 99.3 @ b8**; siglip_b_16_text 96.2 → 96.0, **13.5 FPS @ b1 / 36.3 @ b8**; tinyclip_39m_text 94.0 → 94.0, 58.1/198 FPS |
| **ASR** (`HAILO8L_automatic_speech_recognition.rst`) | `whisper_base_5s_encoder` (19.85M) + `whisper_base_5s_no_kqs_decoder` (51.87M), whisper_tiny 10s pair | encoder 41 FPS, decoder 147 FPS (8L) |
| **OCR** | text detection/recognition (PaddleOCR-class) | vision-side, not retrieval-relevant |

- **No cross-encoder / reranker** entry exists in any zoo branch (searched; nothing on
  the forum either — see §5). The zoo's MiniLM is the *bi-encoder* checkpoint.
- The GenAI zoo (LLMs, `hailo_model_zoo_genai`) is **Hailo-10H only**: "Hailo-10H
  module" required; no 8/8L support. https://github.com/hailo-ai/hailo_model_zoo_genai — CONFIRMED.
- Older staff position, for the record (now superseded for encoder models): "The
  language model is too large for Hailo-8L" (re distilBERT, 66M params; the zoo now
  ships 85.6M-param SigLIP text encoders on 8L, so the practical limit is contexts/FPS,
  not a hard wall). https://community.hailo.ai/t/nlp-with-hailo8l/3177 — CONFIRMED (the
  quote) / INFERRED (that v2.19 supersedes it).

## 3. Hailo-8L hardware

| Claim | Evidence | Confidence |
|---|---|---|
| 13 TOPS; **typical power 1.5W**; industrial -40–85°C; x86/ARM hosts | Hailo-8L product page: "13 Tera-Operations Per Second (TOPS)", "typical power consumption of 1.5W". https://hailo.ai/products/ai-accelerators/hailo-8l-ai-accelerator-for-ai-light-applications/ | CONFIRMED |
| **DRAM-free**: "Does not require external memory" — all weights/activations live in on-chip distributed memory; **on-chip memory size is not published** and staff decline to give a byte threshold (profiler-driven, see §1) | product page above + SRAM thread (§1) | CONFIRMED |
| M.2 module: B+M and A+E keys; module interface advertised **"PCIe Gen-3.0, 2-lanes"** — on our Pi 5 it negotiates Gen2 x1 (≈400–450 MB/s; Gen3 x1 ≈900 MB/s is a config.txt knob) | https://hailo.ai/products/ai-accelerators/hailo-8l-m-2-ai-acceleration-module-for-ai-light-applications/ ; Pi-side from 01-rebaseline.md §4 (`lspci -vv`, captured live) | CONFIRMED |
| **INT8 native; 4-bit weight compression available** ("quantizing some larger layers to 4 bit" to fit a single context; DFC `compression_level` knob) | staff in https://community.hailo.ai/t/multi-context-flow/8017 ; `.alls` `compression_level` field | CONFIRMED |
| **Over-budget models → multi-context**: "If a network requires more resources than available, it is split by the Hailo Dataflow Compiler into multiple contexts. During runtime each context is loaded automatically by the HailoRT runtime and the network is executed context by context." Single-context "will typically run at higher FPS"; multi-context recovers throughput "by using a larger batch size" (amortizes context swaps) | Hailo staff (user1232), https://community.hailo.ai/t/multi-context-flow/8017 | CONFIRMED |
| Context switching streams weights/state **over PCIe** each pass (no on-package DRAM to park them, unlike Hailo-10H's LPDDR4) — so multi-context cost scales with *our* Gen2 x1 link, and zoo FPS numbers (measured Gen3 x4) are optimistic for the Pi | community evidence: 3-context custom YOLOv8m-pose 56 FPS vs official 2-context 65 FPS (https://community.hailo.ai/t/how-to-force-2-context-compilation-for-custom-yolov8m-pose-3-context-gives-lower-fps-than-official-2-context-hef/19167); Hailo-10H context-switch thread contrasts 8-series PCIe reload vs 10H LPDDR (https://community.hailo.ai/t/context-switching-latency-on-hailo-10h/19342) | INFERRED (mechanism widely stated on forum; no official bandwidth model published) |
| MiniLM-L6 (10.6M, 2.9 GOPS) at 121/492 FPS vs SigLIP-text (85.6M, 11 GOPS) at 13.5/36 FPS and the b1→b8 ratios (~4x for MiniLM, ~2.7x SigLIP) are consistent with these text encoders compiling **multi-context** (batch amortization is exactly the staff-described multi-context signature) | derived from §2 tables + staff description | INFERRED |

## 4. HailoRT 4.23 integration

| Claim | Evidence | Confidence |
|---|---|---|
| HailoRT = C/C++ user-space library + `pyHailoRT` Python API + CLI + GStreamer element; Linux/Windows; x86 and ARM hosts; "up to 16 Hailo AI Accelerator devices" | hailo8 branch README, https://github.com/hailo-ai/hailort/tree/hailo8 ; suite page https://hailo.ai/products/hailo-software/hailo-ai-software-suite/ ("Multi-Host architecture support – supports both x86 & ARM", "C/C++ and Python API") | CONFIRMED |
| **Async inference and multi-model/scheduler APIs exist in the 4.x C/C++ API**: shipped examples include `async_infer_basic_example`, `async_infer_advanced_example`, `raw_async_streams_*`, `multi_network_vstream_example`, `multi_process_example`, and `switch_network_groups_example` / `switch_network_groups_manually_example` (model-scheduler-driven vs manual network-group switching on one device) | https://github.com/hailo-ai/hailort/tree/hailo8/hailort/libhailort/examples/cpp (listing captured via GitHub API at branch `hailo8`) | CONFIRMED |
| GitHub's latest 8-series release is **v4.23.0 (2025-09-30)** — exactly what the Pi runs; the hailo8 branch CMake still pins firmware 4.23.0. Model zoo v2.19.0 badges **HailoRT 4.24.0**, which is not on GitHub → 4.24 exists in the Developer Zone; v2.19 HEFs are built with DFC 3.34, so **a runtime upgrade may be required to load them on the Pi** (HEF↔HailoRT version coupling; must be verified empirically with one downloaded HEF before planning anything) | release list via GitHub API (v4.23.0 2025-09-30; v5.x thereafter); v2.19.0 README badge | CONFIRMED (versions) / INFERRED (4.24-needed risk) |
| **Rust bindings: nothing official.** Two community crates: (a) `hailort-sys` 0.1.1 — "Raw FFI bindings to the HailoRT C runtime library", crates.io, created 2026-02-27, 208 downloads, repo github.com/quinnjr/hailort-sys; (b) `kadu-v/hailort-rs` — "Safe Rust bindings for Hailo8/8-L … targeting the Hailo-8L AI accelerator on Raspberry Pi 5", 1 star, pushed 2026-02-14, sync vstream API + tokio `spawn_blocking` async wrapper. Both pre-1.0, single-maintainer, essentially unused in the wild | https://crates.io/crates/hailort-sys (API-verified); https://github.com/kadu-v/hailort-rs (README captured) | CONFIRMED (existence/contents) — maturity assessment INFERRED |
| **Python sidecar is the well-trodden fallback**: `python3-hailort` is already installed on the device (01-rebaseline §4); pyHailoRT is an official first-class API | hailo8 README; device dpkg | CONFIRMED |
| Realistic integration for meridian (Rust): own thin FFI over `libhailort.so` (stable C API, MIT) or a localhost pyHailoRT sidecar; treat the community crates as reference code, not dependencies | — | INFERRED (engineering judgment) |

## 5. Prior art

- **Official, on-target**: the model zoo v2.19.0 MiniLM-L6 numbers in §2 are the only
  published BERT-class text-encoder benchmarks for HAILO8L — 121/492 FPS (b1/b8),
  seq 128, quantized retrieval metrics within 0.1–1.7pp of float. CONFIRMED.
  Note the harness: i5-9400 host, PCIe **Gen3 x4** — not a Pi 5 at Gen2 x1.
- **Official, adjacent**: Whisper encoder/decoder HEFs for 8L (§2) prove
  attention-stack compilation is production-supported on this part. CONFIRMED.
- **Community**: pre-v2.19 threads are uniformly "not yet" for NLP — staff "The
  language model is too large for Hailo-8L" (https://community.hailo.ai/t/nlp-with-hailo8l/3177);
  community-shared Whisper HEFs (https://community.hailo.ai/t/hef-files-for-whisper-base-compatible-with-hailort-4-19-0-hailo-8-amd64/19100).
  **No published cross-encoder/reranker on any Hailo part was found** (searched
  forum + web, multiple phrasings). INFERRED (absence of evidence).
- **Academic, different NPU**: arXiv 2606.11257 "Energy-Efficient On-Device RAG on a
  Mobile NPU" (Snapdragon X Elite Hexagon) — "first end-to-end RAG pipeline that runs
  all neural stages — embedding, reranking, and LLM generation — on the … NPU";
  9.1× embedding throughput, 12.3× less energy vs CPU on indexing; 4.0× lower
  end-to-end query latency. https://arxiv.org/abs/2606.11257 — INFERRED relevance
  (validates NPU-offloaded retrieval as a pattern; Hexagon ≠ Hailo dataflow, and
  their LLM stage has no Hailo-8L analog).

## 6. Honest blockers

1. **No cross-encoder exists today** — the zoo entry is the bi-encoder
   `all-MiniLM-L6-v2`. Our reranker (MiniLM-L6-class cross-encoder) is the *same
   backbone* + classification head, so the compile path is proven, but we would be
   the first public deployment: custom ONNX surgery (host-side token+position+segment
   embedding, parser start at the post-embedding node, `set_input_mask_to_softmax`),
   PTQ calibration on (query,passage) pairs, and accuracy re-validation against the
   204ms CPU baseline. Quantized-accuracy risk on a *scoring* head is real and unmeasured.
2. **Fixed sequence length**: 128 tokens in the zoo build. A cross-encoder wants
   query+passage in one window — 128 forces aggressive passage truncation (our
   answer-mode passages are ≤500 chars, ~96–125 tokens, plus query → over budget);
   a seq-256 recompile is possible but costs ~2× compute and may push the model
   into more contexts. Untested territory.
3. **Compiler is x86_64-only and registration-gated** — a separate x86 build box (or
   CI runner) enters the toolchain permanently; HEFs are version-coupled to
   HailoRT, and the current zoo HEFs reference HailoRT 4.24 vs our installed 4.23.
4. **PCIe Gen2 x1 penalty is unquantified**: all official FPS numbers are Gen3 x4. For
   single-context models the I/O per inference is small (~245 KB → ~0.6ms at 400MB/s),
   but if the model compiles multi-context, weight streaming rides the same 400MB/s
   link every frame. Must be measured, not assumed. (Gen3 x1 config.txt knob halves this risk.)
5. **Embedding throughput is the wrong fight**: 492 seq/s (b8, Gen3 x4) vs potion's
   42,900 docs/s on CPU — the NPU transformer embedder is ~2 orders of magnitude
   slower than our static-embedding indexer. It only makes sense as a *quality* play
   (true contextual embeddings) on query-side or low-rate background re-embedding,
   never for bulk indexing.
6. **Weights are baked into the HEF at compile time** on an x86 host — any scheme
   that treats the NPU as a GEMM engine over *our corpus embeddings* (brute-force
   scoring) would need an HEF recompile per index update. Operationally absurd; and
   ANN at 0.45ms p50 leaves nothing to win.
7. ~~Hard "transformers don't compile" blocker~~ — **does not exist** as of model zoo
   v2.19.0; that is the single biggest update over the 2024 community folklore.

## 7. Feasibility verdict matrix

| Workload | Verdict | Evidence |
|---|---|---|
| **Cross-encoder offload** (MiniLM-L6 CE, top-20/batch-4, beat 204ms p50 CPU) | **POSSIBLE-WITH-WORK** | Same backbone ships officially for 8L (121 b1 / 492 b8 FPS ⇒ ~165ms/~41ms compute for 20 pairs *on a Gen3 x4 host*) — §2 CONFIRMED. But: no cross-encoder prior art anywhere (§5), 128-token window vs query+passage (§6.2), custom ONNX+PTQ pipeline on a gated x86-only compiler (§6.1/6.3), Gen2 x1 penalty unmeasured (§6.4). Expected win is "frees 4 CPU cores during rerank" more than raw latency. |
| **Sentence-embedder offload** (transformer embedder) | **SUPPORTED-TODAY** (query-side / trickle) — **BLOCKED** (bulk indexing) | Official precompiled `all_minilm_l6_v2.hef` for HAILO8L, PTQ'd, ~8ms/query b1 — download-and-run modulo the HailoRT 4.23→4.24 check (§4). For indexing it is ~100–350× slower than potion's 42.9k docs/s CPU path (§6.5), so it's a quality upgrade for query embedding only — and meridian currently has no transformer-embedder consumer, so this is an enabler without a requirement. |
| **Brute-force INT8 matmul scoring** (corpus × query on NPU) | **BLOCKED** | Weights compile into the HEF on an x86 host → recompile per index update (§6.6, CONFIRMED toolchain property); on-chip memory unpublished but bounded, large matrices go multi-context and stream over our 400MB/s link (§3); and the CPU baseline being attacked is 0.45ms ANN / 0.48ms BM25 — no deficit exists. |

**Bottom line**: the NPU is real, idle, and *can* run MiniLM-class encoders as of
2026-06-01 — but the only workload with a measured deficit (CE rerank, 204ms) is
exactly the one with zero prior art and the most toolchain work. Cheapest probe:
download `all_minilm_l6_v2.hef`, run `hailortcli run` on the Pi at Gen2 x1 and
Gen3 x1, and measure b1/b8 FPS — that single experiment converts most INFERRED
rows above into device-measured facts before any compiler work is funded.
