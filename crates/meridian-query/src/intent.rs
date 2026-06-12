//! Query intent classification (SPEC §3, §11). µs-scale heuristic — "no neural
//! cost" (SPEC §3). The class keys the metasearch bandit's engine-subset arms
//! (SPEC §11) and could later steer geo prefiltering.
//!
//! ADR-02 (Phase-3): a GBDT intent model (ONNX/ort) drops in behind this same
//! enum once labeled queries exist; the heuristic is the honest cold-start.

/// Coarse intent classes. Kept small so bandit arm cardinality stays bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// "how/what/why/when …" — favors broad informational engines.
    Question,
    /// Looks like a site/brand/login target — favors general web engines.
    Navigational,
    /// Carries place/geo cues — geo prefilter + locale-aware engines.
    Geo,
    /// Short keyword lookups — the default.
    Keyword,
}

impl Intent {
    /// Stable key for redb bandit-arm storage.
    pub fn key(self) -> &'static str {
        match self {
            Intent::Question => "question",
            Intent::Navigational => "navigational",
            Intent::Geo => "geo",
            Intent::Keyword => "keyword",
        }
    }

    /// Stable numeric index for the decision log (ADR-24). Append-only: new
    /// classes take the next free value; existing values never change (logged
    /// rows outlive code by up to 30 days).
    pub fn index(self) -> u8 {
        match self {
            Intent::Question => 0,
            Intent::Navigational => 1,
            Intent::Geo => 2,
            Intent::Keyword => 3,
        }
    }
}

const QUESTION_WORDS: &[&str] = &[
    "how", "what", "why", "when", "where", "who", "which", "is", "are", "can", "does",
];
const GEO_WORDS: &[&str] = &[
    "near",
    "nearby",
    "in",
    "around",
    "city",
    "town",
    "country",
    "map",
    "directions",
    "weather",
    "restaurant",
    "hotel",
    "airport",
];
const NAV_HINTS: &[&str] = &[
    "login", "sign in", "homepage", "website", "www.", ".com", ".org",
];

/// Classify a normalized query. Order matters: a geo cue or a question word
/// outranks the keyword default.
pub fn classify(query: &str) -> Intent {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Intent::Keyword;
    }

    if NAV_HINTS.iter().any(|h| q.contains(h)) {
        return Intent::Navigational;
    }
    // A leading question word, or a trailing '?', reads as a question.
    if q.ends_with('?') || words.first().is_some_and(|w| QUESTION_WORDS.contains(w)) {
        return Intent::Question;
    }
    if words.iter().any(|w| GEO_WORDS.contains(w)) {
        return Intent::Geo;
    }
    Intent::Keyword
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_shapes() {
        assert_eq!(classify("how does tcp work"), Intent::Question);
        assert_eq!(classify("what is rust?"), Intent::Question);
        assert_eq!(classify("restaurants near central park"), Intent::Geo);
        assert_eq!(classify("weather london"), Intent::Geo);
        assert_eq!(classify("github.com login"), Intent::Navigational);
        assert_eq!(classify("borrow checker"), Intent::Keyword);
        assert_eq!(classify(""), Intent::Keyword);
    }
}
