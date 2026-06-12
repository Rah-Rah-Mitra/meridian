# Answer-mode passage-cap study (2026-06-13, Pi 5)

**Verdict: the carried P10 optimization study lands — `answer_passage_cap`
default 16 → 8, and the answer-mode budget row is WON BACK to p50 ≤3.0s**
(the interim 3.5s re-derivation from the P10 exit stands in the record as
the honest path here).

## Latency arms (n=16 each, live sidecar, PR-28 deep binary on host)

| Arm | p50 | vs the 3.0s row |
|---|---|---|
| cap 16 (the v0.5.0 default) | 3196ms | over (consistent with the exit's 3224ms) |
| **cap 8** | **2502ms** | **under, ~17% headroom** |

The ~700ms delta is the CE realization cost halving — each passage is one
CE pair, and the per-fetch batch is the dominant answer-mode latency term.

## Position telemetry (where winners actually live)

Debug-level trace (position only — no query text, no URL) over the cap-16
arm: winning-passage indices **[0, 0, 1, 1, 1, 1, 2, 7, 11]** (n=9 realized
passages; engine variance kept the other queries from realizing one).
**8 of 9 winners live within the first 8 passages** — the expected shape:
dom_smoothie extraction leads with main content. The one outlier (index 11)
is the measured cost: at cap 8 that document realizes a shallower passage.

## Honesty notes

- n=9 positions is small; the latency delta (the decision's other half) is
  stable across three runs (3196/3224 at 16; 2502 at 8). Recorded as is.
- Suite 18 does not model passage position (its replay reveals one
  passage-CE per doc), so the replay could not arbitrate this — the device
  measurement is the evidence, and the knob keeps the choice operator-
  reversible (`search.answer_passage_cap`).
- First telemetry run captured nothing: the log filter env is
  `MERIDIAN_LOG`, not `RUST_LOG` — recorded for the next person.
