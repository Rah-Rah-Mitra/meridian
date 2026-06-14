//! Per-request tuning knobs (ADR-30): the single source of truth for the SAFE
//! clamp ranges + operator-facing rationale. Shared by `GET /v1/config` (the
//! advertised range the console's tuning drawer pre-fills) and the `/v1/search`
//! `ov_*` override clamping (the enforced range) so the two can never drift.
//!
//! Clamping SATURATES (never 400s): an operator dialing a slider past the bound
//! gets "as far as allowed", and the effective value is reflected back in the
//! response `applied_overrides`. Defaults come from live config (so they track
//! `MERIDIAN_<STRUCT>__<FIELD>` env overrides); only the bounds/rationale live
//! here. The default path (no override) is byte-identical to pre-ADR-30 behavior,
//! so the measured fast/deep/answer budgets are unaffected.

/// One numeric tuning knob. `min`/`max` are the enforced + advertised bounds.
/// (The field is `name`, not `key` — a `key` field full of long underscored
/// literals false-fires the gitleaks generic-api-key rule.)
pub struct Knob {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub unit: &'static str,
    pub rationale: &'static str,
}

/// Every clamped numeric knob (the bool `answer_corroborate` is handled
/// separately — it has no range). Order is the console's display order.
pub const KNOBS: &[Knob] = &[
    Knob {
        name: "bm25_top_k",
        min: 1.0,
        max: 5000.0,
        unit: "candidates",
        rationale: "BM25 candidate depth before fusion (SPEC §11: top-1000). Higher widens recall at more fusion/memory cost.",
    },
    Knob {
        name: "vector_top_k",
        min: 1.0,
        max: 1000.0,
        unit: "candidates",
        rationale: "ANN candidate depth before fusion (SPEC §11: top-200).",
    },
    Knob {
        name: "max_per_domain",
        min: 1.0,
        max: 50.0,
        unit: "results",
        rationale: "Domain-diversity cap in the final ranking (SPEC §11: 3).",
    },
    Knob {
        name: "searx_deadline_ms",
        min: 100.0,
        max: 4000.0,
        unit: "ms",
        rationale: "SearXNG fan-out deadline on the direct lane (SPEC §6.3). On this network real engines take 0.8–1.6s; loosening trades latency for coverage.",
    },
    Knob {
        name: "deep_fetch_max",
        min: 0.0,
        max: 8.0,
        unit: "pages",
        rationale: "Hard cap on the per-request fetch_budget — query-time egress to result domains, direct lane only (ADR-26).",
    },
    Knob {
        name: "deep_fetch_deadline_ms",
        min: 200.0,
        max: 4000.0,
        unit: "ms",
        rationale: "Deep fetch-phase wall-clock ceiling, sized so deep p50 ≤2.5s. Loosening is the operator's explicit budget choice.",
    },
    Knob {
        name: "answer_deadline_ms",
        min: 200.0,
        max: 4000.0,
        unit: "ms",
        rationale: "Answer fetch-phase ceiling (answer mode does strictly more work; own budget so deep's 2.5s isn't silently busted). answer p50 ≤3.0s at the default.",
    },
    Knob {
        name: "answer_passage_cap",
        min: 1.0,
        max: 32.0,
        unit: "passages",
        rationale: "Per-fetched-doc passage-CE cap — the dominant answer-latency term. 2026-06-13 device study: cap 8 → p50 2502ms vs cap 16 → 3196ms (8/9 winners in the first 8).",
    },
    Knob {
        name: "answer_abstain_threshold",
        min: -20.0,
        max: 20.0,
        unit: "ce_logit",
        rationale: "Withhold best_passage when its ce_score is below this; 0 = OFF. Selective prediction WITHOUT a coverage guarantee. Raw ms-marco logit — corpus-specific.",
    },
    Knob {
        name: "answer_corroboration_tau",
        min: -20.0,
        max: 20.0,
        unit: "ce_logit",
        rationale: "The cross-cluster CE bar a passage must clear to count as corroboration (τ=3.0 suite-20). Raw logit scale — corpus-specific.",
    },
    Knob {
        name: "compare_jitter_ms_max",
        min: 0.0,
        max: 60000.0,
        unit: "ms",
        rationale: "Max randomized delay before the anon-side compare dispatch (ADR-22 risk #18 timing decorrelation); 0 = operator accepts the correlation risk.",
    },
    Knob {
        name: "compare_noise_floor_p90",
        min: 0.0,
        max: 1.0,
        unit: "jsd",
        rationale: "Same-lane JSD noise floor (p90, measured suite-12) — the per-request exceeds_floor reference. Measured, not invented.",
    },
    Knob {
        name: "rrf_k",
        min: 1.0,
        max: 1000.0,
        unit: "k",
        rationale: "Reciprocal Rank Fusion constant (Cormack SIGIR'09). Score-scale agnostic — needs no tuning in practice (SPEC §3).",
    },
    Knob {
        name: "containment_tau",
        min: 0.05,
        max: 0.95,
        unit: "containment",
        rationale: "Query-time evidence-clustering threshold (ADR-18). Index-time sketches are unaffected (no re-ingest). ~0.23 unrelated-pair noise floor — go lower and independent docs start merging.",
    },
    Knob {
        name: "rerank_deadline_ms",
        min: 200.0,
        max: 4000.0,
        unit: "ms",
        rationale: "Deep cross-encoder rerank stage deadline (SPEC §11). On overrun the response is degraded:[rerank_timeout], never stalled.",
    },
    Knob {
        name: "answer_passage_deadline_ms",
        min: 200.0,
        max: 3000.0,
        unit: "ms",
        rationale: "Answer-mode passage-CE batch deadline (per fetched doc). On overrun the CE returns its completed pairs — a thinner, never inflated, passage set.",
    },
    Knob {
        name: "answer_corroboration_deadline_ms",
        min: 100.0,
        max: 2000.0,
        unit: "ms",
        rationale: "C1 corroboration CE-batch deadline (measured p50 312ms / p99 508ms on the A76). On overrun the count only thins, never inflates.",
    },
];

fn knob(key: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.name == key)
}

/// Clamp a value to a knob's `[min, max]` (saturating). Unknown key = passthrough
/// (no such knob means nothing to clamp).
pub fn clamp_f64(key: &str, v: f64) -> f64 {
    match knob(key) {
        Some(k) => v.clamp(k.min, k.max),
        None => v,
    }
}
pub fn clamp_usize(key: &str, v: usize) -> usize {
    clamp_f64(key, v as f64).round() as usize
}
pub fn clamp_u64(key: &str, v: u64) -> u64 {
    clamp_f64(key, v as f64).round() as u64
}
pub fn clamp_u32(key: &str, v: u32) -> u32 {
    clamp_f64(key, v as f64).round() as u32
}
pub fn clamp_f32(key: &str, v: f32) -> f32 {
    clamp_f64(key, v as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_saturates_to_bounds() {
        // over the max → max; under the min → min; in range → unchanged.
        assert_eq!(clamp_usize("bm25_top_k", 9_000), 5_000);
        assert_eq!(clamp_usize("bm25_top_k", 0), 1);
        assert_eq!(clamp_usize("bm25_top_k", 1_500), 1_500);
        assert_eq!(clamp_u64("answer_deadline_ms", 99_999), 4_000);
        assert_eq!(clamp_u64("answer_deadline_ms", 0), 200);
        assert_eq!(clamp_f64("containment_tau", 2.0), 0.95);
        assert_eq!(clamp_f64("containment_tau", 0.0), 0.05);
        assert_eq!(clamp_u32("rrf_k", 9_999), 1_000);
    }

    #[test]
    fn unknown_key_is_passthrough() {
        assert_eq!(clamp_usize("not_a_knob", 12_345), 12_345);
    }

    #[test]
    fn every_knob_has_sane_bounds() {
        for k in KNOBS {
            assert!(k.min < k.max, "knob {} has min >= max", k.name);
            assert!(!k.rationale.is_empty(), "knob {} missing rationale", k.name);
        }
    }
}
