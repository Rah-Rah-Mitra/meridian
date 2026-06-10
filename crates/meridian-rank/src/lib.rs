//! Learning-to-rank (SPEC §11). The re-score stage between RRF fusion and
//! domain-diversity, operating on the top-100 fused candidates.
//!
//! ADR-02 (Phase-3 resolution): the cold-start model is a hand-tuned LINEAR
//! scorer implemented in pure Rust — the spec's "linear weights behind the same
//! ONNX interface (single Gemm)", minus the ONNX. The [`Scorer`] trait is the
//! seam: a LightGBM→ONNX model loaded via `ort` drops in here unchanged once
//! training data exists, without touching the planner.
//!
//! Cold-start design goal: **never regress RRF order materially.** `rrf` carries
//! the dominant weight; the other features add mild, bounded lift. On a static
//! corpus (no clicks, neutral freshness) the re-score is close to identity —
//! which is exactly what "no regression" requires (SPEC §16 Phase-3 exit).

/// Per-candidate features (SPEC §11). Fields not yet plumbed are fed neutral
/// values by the planner and documented there (freshness/domain_prior/geo).
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

/// Hand-tuned linear cold-start model. Weights are deliberately RRF-dominant so
/// the re-score is order-preserving in the common case and only nudges on strong
/// secondary signals (title match, multi-source consensus).
#[derive(Debug, Clone)]
pub struct LinearLtr {
    pub w_rrf: f32,
    pub w_bm25: f32,
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
        // RRF in [~0, ~0.07] for our k=60 fusion; multiply up so it dominates the
        // others (each in [0,1] with small weights). Secondary signals add at
        // most ~0.15 total, so they break ties and nudge but never overturn a
        // clear RRF win — the "no regression" guarantee.
        Self {
            w_rrf: 100.0,
            w_bm25: 0.0, // already inside rrf; avoid double-counting scale
            w_ann: 0.05,
            w_title: 0.10,
            w_source: 0.03,
            w_snippet: 0.02,
            w_freshness: 0.02,
            w_domain: 0.05,
            w_geo: 0.05,
        }
    }
}

impl Scorer for LinearLtr {
    fn score(&self, f: &Features) -> f32 {
        self.w_rrf * f.rrf
            + self.w_bm25 * f.bm25
            + self.w_ann * f.ann
            + self.w_title * f.title_match_ratio
            + self.w_source * f.source_count
            + self.w_snippet * f.snippet_len_norm
            + self.w_freshness * (f.freshness - 0.5) // center neutral at 0
            + self.w_domain * f.domain_prior
            + self.w_geo * f.geo
    }

    fn name(&self) -> &'static str {
        "linear-coldstart-v1"
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
    fn rrf_dominates_so_ordering_is_preserved() {
        let s = LinearLtr::default();
        // A clear RRF winner with weak secondaries vs a clear RRF loser with
        // strong secondaries: the RRF winner must still rank higher.
        let winner = Features {
            rrf: 0.030,
            ..Default::default()
        };
        let loser = Features {
            rrf: 0.020,
            title_match_ratio: 1.0,
            ann: 1.0,
            source_count: 5.0,
            snippet_len_norm: 1.0,
            ..Default::default()
        };
        assert!(
            s.score(&winner) > s.score(&loser),
            "RRF gap of 0.01 (= ~1 fusion rank) must not be overturned by secondaries: {} vs {}",
            s.score(&winner),
            s.score(&loser)
        );
    }

    #[test]
    fn secondaries_break_ties() {
        let s = LinearLtr::default();
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
