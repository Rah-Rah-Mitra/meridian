# 04 — Open-source model ledger (rerank · embedding · the decoder line)

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

Which concrete, openly-licensed models could serve Meridian's two neural
workloads — **cross-encoder reranking** (the `meridian-rerank` CE) and **dense
embedding** (the potion static embedder) — on the Pi 5 CPU and/or the Hailo-8L
NPU. This is a *catalog with verdicts*, not a toolchain study: the
Hailo-compilability facts (DFC v3.34 compiles BERT-class encoders, the official
`all_minilm_l6_v2` HEF, fixed seq 128, no reranker HEF, x86-only compile) are
established in `02-competitive.md` **C4** and are referenced, not re-derived.

Labels: **CONFIRMED** = read in an official primary source (HuggingFace card,
repo, leaderboard) at the cited URL; **INFERRED** = reasoned, not directly
stated. Sizes marked INFERRED are weight-only approximations (fp32 ≈ params×4 B,
int8 ≈ params×1 B); the incumbent's stated ~23 MB int8 matches 22.7 M params,
confirming the convention.

## 0. The bar each model must clear (Meridian constraints)

1. **License must be Apache-2.0 or MIT** — the repo is public and the operator
   decides publication; non-commercial (CC-BY-NC) or non-OSI (Gemma) licenses are
   **hard blockers**, flagged loudly below.
2. **Encoder-only, fixed shape** for any NPU path — Hailo-8L runs static dataflow
   graphs, no decoders/KV-cache (C4; the GenAI zoo is Hailo-10H only). This also
   aligns with the repo's extractive-only / "no generative model" stance (ADR-29,
   `00-adr.md:790-795`).
3. **CPU baselines to beat** (Profile R, measured — `01-rebaseline.md:66-70`,
   `02-budgets.md`): CE rerank **204 ms p50 @ top-20/batch-4**; embed **42.9k
   docs/s** (potion static, *not* a transformer); dense store **256-d int8**
   (~256 B/doc, 256 MB @ 1M, whole index 506 MB resident in a 3 GB cgroup).
4. **Standard ops** for NPU — learned/absolute positions + standard softmax
   attention compile cleanly; RoPE / ALiBi / GeGLU / unpadded attention
   (ModernBERT, JinaBERT, Nomic-BERT) are **compilation risks** that must be
   validated against DFC before being counted on.

---

## 1. Cross-encoder rerankers (Track K1)

Cross-encoders are encoder-only (query+passage → one relevance logit) — the same
shape class Hailo already compiles. The incumbent's backbone (MiniLM-L6) is
*exactly* the bi-encoder Hailo ships as `all_minilm_l6_v2`, plus a classification
head, so the K1 compile path is the lowest-risk transformer port available — but
C4 §6 records the honest caveats (no public cross-encoder HEF exists yet; seq-128
forces passage truncation; quantized-accuracy on a *scoring* head is unmeasured).

| Model (HF id) | Backbone / params | Max seq | License | int8 MB | Reranking quality | Source | C/I |
|---|---|---|---|---|---|---|---|
| **`ms-marco-MiniLM-L6-v2`** (incumbent) | MiniLM-L6-H384 / **22.7M** | 512 (deployed 256) | **Apache-2.0** | ~23 | MS MARCO **MRR@10 39.01**; TREC-DL19 nDCG@10 74.30 | [HF](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2) · [sbert](https://www.sbert.net/docs/cross_encoder/pretrained_models.html) | CONFIRMED |
| `ms-marco-MiniLM-L4-v2` | MiniLM-L4-H384 / **19.2M** | 512 | **Apache-2.0** | ~19 | MRR@10 **37.70**; ~2500 docs/s | [HF](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L4-v2) | CONFIRMED |
| `ms-marco-MiniLM-L2-v2` | MiniLM-L2-H384 / **15.6M** | 512 | **Apache-2.0** | ~16 | MRR@10 **34.85**; ~4100 docs/s | [HF](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L2-v2) | CONFIRMED |
| `ms-marco-TinyBERT-L2-v2` | BERT-Tiny / **4.39M** | 512 | **Apache-2.0** | ~5 | MRR@10 **32.56**; ~9000 docs/s | [HF](https://huggingface.co/cross-encoder/ms-marco-TinyBERT-L-2-v2) | CONFIRMED |
| `BAAI/bge-reranker-base` | XLM-RoBERTa-base / **278M** | 512 | **MIT** | ~280 | MMarcoReranking MAP 35.46 (no BEIR-avg on card) | [HF](https://huggingface.co/BAAI/bge-reranker-base) | CONFIRMED (score partial) |
| `BAAI/bge-reranker-v2-m3` | XLM-RoBERTa-large / **~568M** | 512 | **Apache-2.0** | ~568 | MIRACL/BEIR (card shows charts only — numeric unverified) | [HF](https://huggingface.co/BAAI/bge-reranker-v2-m3) | CONFIRMED (license/params) |
| `Alibaba-NLP/gte-reranker-modernbert-base` | ModernBERT-base (RoPE/GeGLU) / **149M** | 8192 | **Apache-2.0** | ~149 | BEIR avg nDCG@10 **56.73** | [HF](https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base) | CONFIRMED |
| `cross-encoder/ettin-reranker-{17,32,68}m-v1` (2025) | Ettin/ModernBERT-style (RoPE/GeGLU/unpadded) / **17.6 / 32.8 / 68.6M** | 8192 | **Apache-2.0** | ~18 / ~33 / ~69 | MTEB-Retrieval nDCG@10 **0.558 / 0.578 / 0.592** | [HF blog](https://huggingface.co/blog/ettin-reranker) | CONFIRMED |
| 🚩 `jinaai/jina-reranker-v2-base-multilingual` | XLM-RoBERTa-style / 278M | 1024 | **CC-BY-NC-4.0 — NON-COMMERCIAL, BLOCKER** | ~278 | BEIR nDCG@10 53.17 | [HF](https://huggingface.co/jinaai/jina-reranker-v2-base-multilingual) | CONFIRMED |
| ❌ `mixedbread-ai/mxbai-rerank-{base,large}-v2` | **Qwen2 DECODER LLM** / 0.5B / 1.5B | 32k | Apache-2.0 | ~250 / ~1500 | BEIR avg 55.57 / 57.49 | [HF](https://huggingface.co/mixedbread-ai/mxbai-rerank-base-v2) | CONFIRMED |

`jina-reranker-v1-tiny-en` (33M, JinaBERT/ALiBi, **Apache-2.0**, BEIR 48.54) and
`mxbai-rerank-xsmall-v1` (70.8M, DeBERTa-v2, Apache-2.0, BEIR 43.9) are
license-clean but carry op-set risk (ALiBi bias; DeBERTa disentangled attention —
mixedbread's own card notes it supports neither FlashAttention-2 nor SDPA, i.e.
slow) and are not recommended over the MiniLM family.

**License map.** Clean (Apache-2.0/MIT): the entire `ms-marco-*` MiniLM/TinyBERT
family, both `bge-reranker-*`, `gte-reranker-modernbert-base`, the Ettin family,
`jina-reranker-v1-tiny-en`, all mxbai. **Blocker:** `jina-reranker-v2-base-
multilingual` is **CC-BY-NC-4.0** — the *v1* sibling is Apache-2.0, so Jina
licensing is per-model; verify each individually.

**CPU/latency fit (the 204 ms / top-20 budget).** Only the *same-family*
MiniLM/TinyBERT models trade quality for latency without changing ops:
L4 (~37.7 MRR, ~1.4× faster than L6), L2 (~34.9, ~2.3×), TinyBERT-L2 (~32.6,
~5×). These are the honest *CPU* levers — and they re-frame the K4-rejected "speed
up the NPU" wish as a cheap CPU downshift when latency, not quality, is the
constraint. Everything ≥100M params (bge/gte/jina-v2/Ettin-150m+) is a multi-
context NPU job at best, not a CPU option.

**NPU-shape fit.** ✅ Clean: all `ms-marco-MiniLM-*` + `TinyBERT` (vanilla BERT
ops, learned positions, fixed 512 → pad to 256) — the incumbent is the safest NPU
candidate; `bge-reranker-*` (XLM-R encoders, standard ops, but 278–568M ⇒ heavy
multi-context). ⚠️ Risk: `gte-reranker-modernbert-base` + Ettin (RoPE/GeGLU/
unpadded — may not lower to a fixed-shape INT8 graph), `jina-v1-tiny` (ALiBi),
`mxbai-rerank-xsmall` (DeBERTa relative attention). ❌ Out: mxbai-v2 (Qwen2
decoders — no decoders on 8L, and out of CPU budget).

**Verdict for K1.** *Do not swap the model.* The recommended K1 play is to compile
the **existing `ms-marco-MiniLM-L6-v2`** to HEF (same backbone Hailo already
ships, Apache-2.0, MRR 39.01) and let the NPU buy depth/latency headroom — the §6
Bet 1 hypothesis. Keep `ms-marco-MiniLM-L4-v2` as the **CPU downshift** for a
latency-constrained build (−1.3 MRR for ~1.4× throughput, zero op-set risk). A
*quality* upgrade (bge-reranker-base on NPU, multi-context) is a Horizon-3
contingency only if suite-4/13 measure the depth ceiling as quality-binding —
unlikely given the deep path is network-dominated (§T5).

---

## 2. Dense embedding models (Track K2)

The incumbent `potion-base-8M` is a **model2vec static embedding** — table lookup
+ mean pooling, *no transformer*, 256-d, MIT, ~42.9k docs/s. The upgrade question
is whether a small transformer buys enough recall to justify a re-embed + index
rebuild (and, on NPU, a compile). The official Hailo `all_minilm_l6_v2` HEF
(seq 128, 384-d) makes MiniLM the lowest-risk NPU embedder.

| Model (HF id) | Params | Dim (Matryoshka?) | Seq | License | int8 MB | MTEB | Source | C/I |
|---|---|---|---|---|---|---|---|---|
| **`potion-base-8M`** (incumbent, STATIC) | 7.56M | **256** | n/a | **MIT** | **~8** | **51.08** (Eng avg) | [HF](https://huggingface.co/minishlab/potion-base-8M) · [results](https://github.com/MinishLab/model2vec/blob/main/results/README.md) | CONFIRMED |
| `potion-retrieval-32M` (STATIC) | 32.3M | **256** | n/a | **MIT** | ~32 | **35.06** (Retrieval); ~49.8 Eng avg | [HF](https://huggingface.co/minishlab/potion-retrieval-32M) | CONFIRMED (retrieval); avg INFERRED |
| `all-MiniLM-L6-v2` (**has official Hailo HEF**) | 22.7M | 384 | 256 | **Apache-2.0** | ~23 | **55.93** (Eng avg) | [HF](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) | CONFIRMED |
| `BAAI/bge-small-en-v1.5` | 33.4M | 384 | 512 | **MIT** | ~33 | **62.17** (Eng avg, 56 tasks) | [HF](https://huggingface.co/BAAI/bge-small-en-v1.5) | CONFIRMED |
| `thenlper/gte-small` | 33.4M | 384 | 512 | **MIT** | ~33 | **61.36** (Eng avg) | [HF](https://huggingface.co/thenlper/gte-small) | CONFIRMED |
| `intfloat/e5-small-v2` | ~33M | 384 | 512 | **MIT** | ~33 | ~58 (Eng avg, card lists per-task only) | [HF](https://huggingface.co/intfloat/e5-small-v2) | CONFIRMED (dim/license); score unverified |
| `Snowflake/snowflake-arctic-embed-s` | 33M | 384 | 512 | **Apache-2.0** | ~33 | **51.98** (BEIR nDCG@10) | [HF](https://huggingface.co/Snowflake/snowflake-arctic-embed-s) | CONFIRMED |
| `Snowflake/snowflake-arctic-embed-xs` | 22M | 384 | 512 | **Apache-2.0** | ~22 | 50.15 (BEIR nDCG@10) | [HF](https://huggingface.co/Snowflake/snowflake-arctic-embed-xs) | CONFIRMED |
| `Snowflake/snowflake-arctic-embed-m-v1.5` | 109M | **768 → 256 (Matryoshka)** | 512 | **Apache-2.0** | ~109 | **55.14** (BEIR nDCG@10 @256-d) | [HF](https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v1.5) | CONFIRMED |
| `ibm-granite/granite-embedding-30m-english` | 30M | 384 | 512 | **Apache-2.0** | ~30 | 49.1 (BEIR, 15 sets) | [HF](https://huggingface.co/ibm-granite/granite-embedding-30m-english) | CONFIRMED |
| `BAAI/bge-base-en-v1.5` / `thenlper/gte-base` | 109M | 768 | 512 | **MIT** | ~109 | 63.55 / 62.39 (Eng avg) | [HF](https://huggingface.co/BAAI/bge-base-en-v1.5) | CONFIRMED |
| `nomic-ai/nomic-embed-text-v1.5` | 137M | 768 → 256/128 (Matryoshka) | 8192 | **Apache-2.0** | ~137 | 62.28 (Eng avg) | [HF](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5) | CONFIRMED (RoPE arch) |
| `nomic-ai/modernbert-embed-base` | 149M | 768 → 256 (Matryoshka) | 8192 | **Apache-2.0** | ~149 | ">nomic-v1.5" (no headline avg on card) | [HF](https://huggingface.co/nomic-ai/modernbert-embed-base) | CONFIRMED (arch); score unverified |
| `mixedbread-ai/mxbai-embed-large-v1` | 335M | 1024 → 512 | 512 | **Apache-2.0** | ~335 | 64.68 (Eng avg) | [HF](https://huggingface.co/mixedbread-ai/mxbai-embed-large-v1) | CONFIRMED (too big for Pi) |
| 🚩 `google/embeddinggemma-300m` | 300M | 768 → 256/128 | 2048 | **`gemma` — NOT OSI-approved, BLOCKER** | ~300 | 61.15 (MTEB Multiling v2) | [HF](https://huggingface.co/google/embeddinggemma-300m) | CONFIRMED |

**License map.** Clean: MIT (bge-small/base, gte-small/base, e5-small-v2,
multilingual-e5-small, **both potion**), Apache-2.0 (all-MiniLM, arctic-embed
family, nomic-v1.5, modernbert-embed, mxbai-large, granite). **Blocker:**
`embeddinggemma-300m` ships under the **Gemma license** ([terms](https://ai.google.dev/gemma/terms))
— not OSI-approved, carries use restrictions and a prohibited-use policy. Treat as
unusable for a publishable appliance; the nearest clean substitutes are
`arctic-embed-m-v1.5` (256-d Matryoshka) or `modernbert-embed-base`.

**The bar to beat.** potion-base-8M sits at **51.08** Eng-avg / the retrieval
variant at **35.06** BEIR. A transformer buys **+4.85** (all-MiniLM) to **+11**
(bge/gte-small) Eng-avg points; on the retrieval axis the move from
potion-retrieval's 35.06 to a bge/gte/arctic transformer in the low-50s BEIR is a
**~15–20 nDCG-point step-change** — the single strongest migration argument. The
no-free-lunch: potion does *zero* matmuls (hence 43k docs/s); any transformer pays
a 1–2 order-of-magnitude CPU latency tax — which is **exactly the cost the
Hailo-8L is meant to absorb** (K2's whole premise), with NPU ingest throughput the
enabler for a full re-embed.

**Schema fit (current store = 256-d int8).**
- **Stay 256-d, no schema change, no NPU:** `potion-retrieval-32M` (static, MIT) —
  the *cheapest possible* embedding upgrade, a pure CPU swap; measure whether its
  retrieval lift over potion-base is worth the 4× vector size.
- **Stay 256-d via Matryoshka (transformer quality, same schema):**
  `arctic-embed-m-v1.5` (768→256, **55.14 BEIR@256**, Apache-2.0, BERT-base) is the
  most migration-friendly *and* NPU-aligned transformer; `modernbert-embed-base` /
  `nomic-v1.5` also truncate to 256 but use RoPE/long-context arches (NPU risk).
- **384-d (modest +50% index, ~384 MB @ 1M):** `all-MiniLM-L6-v2` (has the Hailo
  HEF — lowest NPU risk, +4.85 MTEB), `bge-small`/`gte-small` (best 384-d quality
  62/61, MIT, but you compile your own HEF).
- **768-d+ busts the storage row** (3× vectors, full rebuild): bge/gte-base,
  granite-125m, mxbai-large (335M — too big regardless).

**NPU-shape fit.** ✅ Standard BERT/RoBERTa encoders (all-MiniLM, bge-small/base,
gte-small/base, e5-small-v2, arctic-embed-xs/s/m-v1.5, granite) compile to fixed
shapes; all-MiniLM already ships as an official HEF. ⚠️ nomic-v1.5 / modernbert-
embed (RoPE, local-global attention, 8192 ctx) and EmbeddingGemma (Gemma decoder
backbone) are not stock BERT shapes — custom compile, may not map.

**Verdict for K2.** Two-step, experiment-gated (§6 Bet honorable-mention / §12 H3):
1. **Cheap CPU probe first:** swap potion-base → `potion-retrieval-32M` (static,
   256-d, MIT — *zero* schema/NPU cost) and measure recall@10 on suite-2/13 at the
   1M baseline. If the static retrieval model already closes most of the gap, the
   transformer migration is unjustified — a likely "simple-baseline-wins" outcome.
2. **If a transformer is warranted:** `arctic-embed-m-v1.5` truncated to 256-d
   (keeps the exact store schema, Apache-2.0, BERT-base/NPU-aligned, 55.14 BEIR) is
   the lead candidate; `all-MiniLM-L6-v2` at 384-d is the lower-risk-NPU / lower-
   quality fallback (its HEF already exists). Gate on suite-2 recall@10 ≥0.95 and
   hold-out hybrid nDCG@10 ≥ potion-hybrid +0.02 (the §T5 K2 acceptance), with the
   256→384-d storage-row check (the 600 MB disk row, K2 kill criterion).

---

## 3. The decoder line — out of scope, stated plainly

Generative / decoder models (LLM rerankers like `mxbai-rerank-*-v2` on Qwen2;
any answer-generation model) are rejected on **three independent grounds**, any
one sufficient:

- **Hardware:** Hailo-8L executes static dataflow graphs with no KV-cache; the
  Hailo GenAI zoo is **Hailo-10H only** (C4 §2). A decoder cannot run on this NPU.
- **Budget:** 0.5–1.5B-param rerankers are 1–2 orders over the CPU latency budget
  on 4× A76 (multi-second per query).
- **Product stance:** the repo is **extractive-only** by design — `best_passage`
  is a verbatim span, and ADR-29 records the "no generative model, no hallucinated
  answer" honesty position (`00-adr.md:790-795`). A generative reranker or answerer
  would contradict the identity §14 defends, regardless of hardware.

LLM-as-reranker quality (mxbai-v2 BEIR 55–57) is real, but it is the wrong tool
for *this* appliance. The encoder-only cross-encoder + extractive passage remains
the correct architecture; the NPU's role is to make that architecture *faster and
deeper*, not to change its class.

---

## 4. Bottom line

| Workload | Keep | CPU lever | NPU upgrade | Reject |
|---|---|---|---|---|
| **Rerank (K1)** | `ms-marco-MiniLM-L6-v2` (Apache, MRR 39.0) | downshift to `ms-marco-MiniLM-L4-v2` for latency | compile the *same* L6 to HEF (§6 Bet 1); bge-reranker-base only if depth is quality-binding | jina-v2-multilingual (CC-BY-NC); mxbai-v2 (Qwen2 decoders) |
| **Embed (K2)** | `potion-base-8M` (static, MIT, 43k docs/s) | probe `potion-retrieval-32M` (static, 256-d, free) | `arctic-embed-m-v1.5` @256-d Matryoshka, or `all-MiniLM-L6-v2` @384-d (HEF exists) | EmbeddingGemma (Gemma license); 768-d+ models (storage row); nomic/modernbert (RoPE NPU risk) |

Every row is Apache-2.0 or MIT (the blockers are named and excluded), encoder-only
(no decoders), and gated on an existing suite (4/13 for rerank, 2/13 for embed).
The recurring honest note: **the incumbents are well-chosen** — the realistic wins
are *compiling what we have to the NPU* (K1) and a *cheap static-retrieval CPU
probe before any transformer migration* (K2), not chasing a leaderboard model that
breaks the license, the storage row, or the no-decoder rule.

---

*Companion to `00-review.md` (§6 Bets 1–2, §8.5, §12 H2/H3) and `02-competitive.md`
C4 (Hailo toolchain facts). As-of: v0.6.0 / `812d3d4` / 2026-06-13.*
