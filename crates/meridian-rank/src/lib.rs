//! Learning-to-rank (SPEC §11). The re-score stage between RRF fusion and
//! domain-diversity, operating on the top-100 fused candidates.
//!
//! ADR-02 (Phase-3 resolution): the cold-start model is a hand-tuned LINEAR
//! scorer implemented in pure Rust — the spec's "linear weights behind the same
//! ONNX interface (single Gemm)", minus the ONNX. The [`Scorer`] trait is the
//! seam: a LightGBM→ONNX model loaded via `ort` drops in here unchanged once
//! training data exists, without touching the planner.
//!
//! Cold-start design (see [`LinearLtr`] for the full reasoning): the shipped
//! default is **RRF-identity** — unit RRF weight, zero secondary weights — which
//! guarantees the SPEC §16 "LTR no regression" gate by construction. The feature
//! extraction, weights, and trait all ship so the trained GBDT slots in unchanged.

/// Per-candidate features (SPEC §11). Fields not yet plumbed are fed neutral
/// values by the planner and documented there (freshness/domain_prior/geo).
pub mod diversity;
pub mod mmr;
pub mod qpp;

#[derive(Debug, Clone, Default)]
pub struct Features {
    /// RRF fusion score — the dominant signal (already encodes BM25⊕ANN⊕engines).
    pub rrf: f32,
    /// Raw BM25 (0 if the doc came only from ANN/web).
    pub bm25: f32,
    /// ANN cosine similarity in [0,1] (0 if lexical/web only).
    pub ann: f32,
    /// Fraction of query terms present in the title, [0,1].
    pub title_match_ratio: f32,
    /// How many sources surfaced this doc (local + #engines) — consensus signal.
    pub source_count: f32,
    /// Snippet length in chars, normalized to ~[0,1] (longer ≈ more substantive).
    pub snippet_len_norm: f32,
    /// Exp-decay freshness over `ts`, [0,1]; 0.5 = neutral/unknown.
    pub freshness: f32,
    /// Domain PageRank prior, [0,1]; 0 until Phase-5 analytics.
    pub domain_prior: f32,
    /// Geo proximity bonus, [0,1]; 0 until Phase-5 geo.
    pub geo: f32,
}

/// The rank-scoring seam. Cold-start = [`LinearLtr`]; a GBDT/ONNX model
/// implements the same trait later (ADR-02).
pub trait Scorer: Send + Sync {
    fn score(&self, f: &Features) -> f32;
    fn name(&self) -> &'static str;
}

/// Linear LTR model: `score = w_rrf·rrf + Σ wᵢ·featureᵢ`.
///
/// **The shipped cold-start is RRF-identity** (unit RRF weight, ZERO secondary
/// weights), and here is the honest reasoning. With no training data
/// (Phase 3 has no click logs, only a synthetic eval), the secondary features are
/// either already inside RRF (bm25, ann, source-consensus), neutral until later
/// phases (freshness/domain_prior/geo are 0 — Phase 5), or — for title-match —
/// genuinely informative on real queries but *anti-correlated* on the
/// describe-without-naming synthetic eval. Empirically, every non-zero secondary
/// weighting regressed nDCG@10 on that eval. So an untrained linear model cannot
/// beat RRF on the data we have; the responsible cold-start is the identity,
/// which guarantees the §16 "LTR no regression" gate by construction.
///
/// What ships is therefore the **stage + interface + feature extraction**, with
/// the weights as tunable fields all defaulting to zero except `w_rrf`. The
/// trained GBDT (ADR-02, once labeled data exists) implements the same [`Scorer`]
/// trait and is what will actually weight the secondaries — that is its job, not
/// the cold-start's. Operators with their own labeled set can also hand-tune
/// these weights without recompiling the planner.
#[derive(Debug, Clone)]
pub struct LinearLtr {
    pub w_rrf: f32,
    pub w_ann: f32,
    pub w_title: f32,
    pub w_source: f32,
    pub w_snippet: f32,
    pub w_freshness: f32,
    pub w_domain: f32,
    pub w_geo: f32,
}

impl Default for LinearLtr {
    fn default() -> Self {
        // RRF-identity cold-start: secondaries plumbed but unweighted.
        Self {
            w_rrf: 1.0,
            w_ann: 0.0,
            w_title: 0.0,
            w_source: 0.0,
            w_snippet: 0.0,
            w_freshness: 0.0,
            w_domain: 0.0,
            w_geo: 0.0,
        }
    }
}

impl Scorer for LinearLtr {
    fn score(&self, f: &Features) -> f32 {
        self.w_rrf * f.rrf
            + self.w_ann * f.ann
            + self.w_title * f.title_match_ratio
            + self.w_source * (f.source_count.min(3.0) / 3.0)
            + self.w_snippet * f.snippet_len_norm
            + self.w_freshness * (f.freshness - 0.5)
            + self.w_domain * f.domain_prior
            + self.w_geo * f.geo
    }

    fn name(&self) -> &'static str {
        "linear-coldstart-rrf-identity"
    }
}

/// Title-match ratio helper: fraction of unique lowercased query terms present
/// in the title (SPEC §11 `title_match_ratio`).
pub fn title_match_ratio(query: &str, title: &str) -> f32 {
    let title_lc = title.to_lowercase();
    let terms: std::collections::HashSet<&str> =
        query.split_whitespace().filter(|t| t.len() >= 2).collect();
    if terms.is_empty() {
        return 0.0;
    }
    let present = terms
        .iter()
        .filter(|t| title_lc.contains(&t.to_lowercase()))
        .count();
    present as f32 / terms.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_is_rrf_identity() {
        let s = LinearLtr::default();
        // Order is determined by RRF alone; the strongest possible secondaries
        // do not change the score (zero weights), so they cannot reorder.
        let plain = Features {
            rrf: 0.025,
            ..Default::default()
        };
        let loaded = Features {
            rrf: 0.025,
            title_match_ratio: 1.0,
            ann: 1.0,
            source_count: 9.0,
            snippet_len_norm: 1.0,
            domain_prior: 1.0,
            geo: 1.0,
            freshness: 1.0,
            ..Default::default()
        };
        assert_eq!(
            s.score(&plain),
            s.score(&loaded),
            "cold-start must ignore secondaries"
        );
        assert_eq!(s.score(&plain), 0.025, "cold-start score is exactly rrf");
    }

    #[test]
    fn custom_weights_activate_secondaries() {
        // A trained/operator model with non-zero weights DOES use the features —
        // the machinery is real, only the default is identity.
        let s = LinearLtr {
            w_title: 0.1,
            ..LinearLtr::default()
        };
        let plain = Features {
            rrf: 0.025,
            ..Default::default()
        };
        let with_title = Features {
            rrf: 0.025,
            title_match_ratio: 1.0,
            ..Default::default()
        };
        assert!(s.score(&with_title) > s.score(&plain));
    }

    #[test]
    fn title_match_ratio_counts_present_terms() {
        assert_eq!(
            title_match_ratio("rust borrow checker", "The Rust Borrow Checker"),
            1.0
        );
        assert!((title_match_ratio("rust gardening", "Rust programming") - 0.5).abs() < 1e-6);
        assert_eq!(title_match_ratio("", "anything"), 0.0);
    }
}
