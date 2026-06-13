# K1 Hailo-8L offload — step-0 probe: BLOCKED (measured)

As-of: v0.6.0 · Pi 5 + Hailo-8L · 2026-06-13 · roadmap §6 Bet 1 / Track K1.
The bet's deliberately zero-risk step-0 (§6 Bet 1, falsifying experiment 0): probe the
NPU on-device *before* any code, to measure the real PCIe Gen2-x1 penalty and confirm a
retrieval HEF is compilable. **Outcome: K1 is blocked at step 0 on two independent
grounds, both now measured rather than assumed.**

## Device (captured live)

`hailortcli fw-control identify`:
- Architecture **HAILO8L** (13 TOPS INT8), FW **4.23.0**, `/dev/hailo0` present (world-rw).
- Board `HAILO-8L AI ACC M.2 B+M KEY MODULE EXT TMP`, serial `HLDDLBB242501026`,
  part `HM21LB1C2LAE`, at PCIe `0001:04:00.0`.

## Finding 1 — no retrieval HEF on the device; the MiniLM HEF needs the x86 DFC

`ls /usr/share/hailo-models/*.hef` → **vision CNNs only**: `resnet_v1_50_{h8l,h10}`,
`yolov5/6/11*`, `scrfd_2.5g_h8l`, `yolov5n_seg_*`. **No `all_minilm_l6`, no BERT-class,
no embedding/cross-encoder HEF.** This confirms the C4 ledger: the official MiniLM HEF
(model zoo v2.19.0) is not shipped to the device, and compiling one requires the Hailo
Dataflow Compiler, which is **x86_64-only** — it cannot run on this Pi, and no x86 DFC
host has been verified (the bet's hardest, explicitly-unverified blocker). Without a
compiled HEF there is nothing to offload.

## Finding 2 — the PCIe Gen2-x1 link is the dominant bottleneck (the K1 win-killer)

Benchmarked the HAILO8L vision model `resnet_v1_50_h8l.hef` (45 MB) as a proxy for the
on-device throughput a streamed transformer would see:

`hailortcli benchmark /usr/share/hailo-models/resnet_v1_50_h8l.hef`:

| mode | FPS |
|---|---|
| HW-only | **~23.8** |
| streaming | **~23.9** |

ResNet-50 runs at **hundreds** of FPS on a Hailo-8L over Gen3-x4 (Hailo's published
numbers); **~24 FPS here** is dominated by the **negotiated PCIe Gen2 x1 link
(~400–450 MB/s)**: a 45 MB multi-context HEF weight-streamed per inference is ~9 FPS for
the weights alone, so the link — not the 13-TOPS compute — sets the ceiling. This is the
direct, measured form of the bet's **risk #8 swing factor** ("multi-context weight
streaming … could compress the win to ~2×"): on this link it is worse than that for a
large model.

For K1's seq-256 `ms-marco-MiniLM-L6` cross-encoder the implication is the same — its
weights would stream over the same Gen2-x1 link per batch, so the **≥2× stage speedup
the accept gate requires (vs the 204 ms CPU rerank stage) is unlikely** to survive the
link penalty, even before the compile blocker.

## Verdict — RECORD-NO-GO (K1 blocked at step 0), with the data on record

K1 cannot proceed: (a) the MiniLM CE HEF is not compilable on-device (x86-only DFC, no
verified host), and (b) the measured Gen2-x1 throughput makes the ≥2× win bar unlikely.
**No code was written** (the `HailoReranker` behind the `Reranker` trait is not started);
the appliance idiom holds — Hailo stays an accelerator-never-a-dependency, and here it is
simply unavailable. This matches the roadmap's own honesty note (§14.3): *"'cannot compile
a BERT-class encoder' reduces the entire Hailo track to a documented no-go with the §T5
arithmetic on the record."* The §T5 arithmetic is now backed by an on-device PCIe
measurement, not an estimate.

**Re-entry condition** (all three, then re-run this probe): an x86_64 DFC host that
compiles the seq-256 MiniLM CE to a HAILO8L HEF · the PCIe Gen3 `config.txt` knob enabled
(~900 MB/s, ~2× the link budget) · a re-benchmark of the compiled CE showing ms/pair
≤0.5× the CPU stage AND fail-open-to-CPU proven (yank `/dev/hailo0` mid-run → degrades).
Until then K1 is a documented no-go; the salvage path (K2 query-side embedder HEF) faces
the same compile + link blockers and is not promoted.
