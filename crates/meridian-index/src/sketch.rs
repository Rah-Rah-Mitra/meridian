//! Near-duplicate sketches for derivation clustering (Phase 7, ADR-18).
//!
//! One 64-byte sketch per document, computed from the clean extracted text at
//! ingest and stored alongside the dedup rows (deletable in the same forget
//! transaction, ADR-19). At query time, pairwise **containment**
//! (|A∩B| / min(|A|,|B|)) over result sketches builds derivation clusters —
//! the suite-9 experiment showed raw Jaccard fails on realistic syndication
//! (truncation + boilerplate asymmetry) while containment separates cleanly
//! (`docs/plan/bench/2026-06-11-pi5-p7-experiments.md`).
//!
//! Construction: word shingles (k=4) → densified one-permutation b-bit MinHash
//! (60 bins × 1 byte). One hash pass per shingle keeps the ingest cost in the
//! microseconds — the per-perm classic MinHash the experiment swept would cost
//! ~60× more per doc. The b=8 quantization and the OPH estimator are validated
//! against the same synthetic farms by suite 9's `production sketch` gate.
//!
//! Exact-duplicate detection is NOT this module's job: blake3 content hashes
//! already dedup verbatim copies at ingest (SPEC §9.3).

/// Shingle width in words (suite-9 constant, ADR-18).
pub const SHINGLE_K: usize = 4;
/// Signature bins (b=8 each). 60 bins + the 4-byte shingle count = 64 B/doc,
/// the 02-budgets ceiling.
pub const BINS: usize = 60;
/// Derivation-edge threshold on estimated containment (suite-9 constant).
pub const CONTAINMENT_TAU: f64 = 0.3;
/// Encoded payload length: u32 LE distinct-shingle count + one byte per bin.
pub const ENCODED_LEN: usize = 4 + BINS;

/// Chance two UNRELATED bins share a byte (b=8 quantization collision).
const B_BIT_COLLISION: f64 = 1.0 / 256.0;

/// Minimum matched bins before the similarity estimate is trusted at all.
/// Noise floor: unrelated docs match ≈ BINS/256 ≈ 0.23 bins in expectation, so
/// ≥5 by chance is ~5e-7 per pair — while real derivation produces 25+.
/// Without this floor, ONE chance collision against a tiny shingle set
/// (min(|A|,|B|) ≈ 10) amplifies through the containment denominator to ≥τ —
/// a false merge observed live during the P7 exit drill (tiny repeated doc vs
/// a large unrelated article).
pub const MIN_MATCH_BINS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sketch {
    /// Distinct shingle count — the containment denominator.
    pub shingles: u32,
    pub sig: [u8; BINS],
}

impl Sketch {
    /// Sketch the clean text. Tokenization is the single canonical one for
    /// sketches: lowercase alphanumeric words; documents shorter than one
    /// shingle hash as a single whole-document shingle.
    pub fn compute(text: &str) -> Self {
        let words = word_hashes(text);
        let mut shingles = std::collections::HashSet::new();
        if words.len() < SHINGLE_K {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for w in &words {
                h = splitmix64(h ^ w);
            }
            shingles.insert(splitmix64(h));
        } else {
            for win in words.windows(SHINGLE_K) {
                let mut h = win[0];
                for w in &win[1..] {
                    h = splitmix64(h ^ w);
                }
                shingles.insert(h);
            }
        }

        // One-permutation hashing: each shingle lands in one bin; the bin keeps
        // the minimum of the remaining hash bits.
        let mut mins = [u64::MAX; BINS];
        for &s in &shingles {
            let bin = (s % BINS as u64) as usize;
            let val = s / BINS as u64;
            if val < mins[bin] {
                mins[bin] = val;
            }
        }
        // Densification (Shrivastava-style rotation): an empty bin borrows the
        // nearest filled bin's minimum, re-mixed with the offset so borrowed
        // values do not correlate across bins.
        let mut sig = [0u8; BINS];
        for i in 0..BINS {
            let mut val = mins[i];
            if val == u64::MAX {
                for t in 1..BINS {
                    let j = (i + t) % BINS;
                    if mins[j] != u64::MAX {
                        val = splitmix64(mins[j] ^ t as u64);
                        break;
                    }
                }
                if val == u64::MAX {
                    val = 0; // unreachable: compute() always inserts ≥1 shingle
                }
            }
            sig[i] = (val & 0xFF) as u8;
        }
        Self {
            shingles: shingles.len() as u32,
            sig,
        }
    }

    /// Estimated Jaccard similarity from the signatures, corrected for the b=8
    /// quantization collision rate. Below [`MIN_MATCH_BINS`] the estimate is 0
    /// — chance collisions must never clear the derivation threshold via the
    /// containment amplification (see the constant's doc).
    pub fn jaccard(&self, other: &Self) -> f64 {
        let matches = self
            .sig
            .iter()
            .zip(other.sig.iter())
            .filter(|(a, b)| a == b)
            .count();
        if matches < MIN_MATCH_BINS {
            return 0.0;
        }
        let m = matches as f64 / BINS as f64;
        ((m - B_BIT_COLLISION) / (1.0 - B_BIT_COLLISION)).clamp(0.0, 1.0)
    }

    /// Estimated containment |A∩B| / min(|A|,|B|) — robust to the
    /// truncation/boilerplate asymmetry of real syndication (ADR-18).
    pub fn containment(&self, other: &Self) -> f64 {
        let (na, nb) = (f64::from(self.shingles), f64::from(other.shingles));
        if na <= 0.0 || nb <= 0.0 {
            return 0.0;
        }
        let j = self.jaccard(other);
        let inter = j * (na + nb) / (1.0 + j);
        (inter / na.min(nb)).clamp(0.0, 1.0)
    }

    pub fn encode(&self) -> [u8; ENCODED_LEN] {
        let mut out = [0u8; ENCODED_LEN];
        out[..4].copy_from_slice(&self.shingles.to_le_bytes());
        out[4..].copy_from_slice(&self.sig);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ENCODED_LEN {
            return None;
        }
        let shingles = u32::from_le_bytes(bytes[..4].try_into().ok()?);
        let mut sig = [0u8; BINS];
        sig.copy_from_slice(&bytes[4..]);
        Some(Self { shingles, sig })
    }
}

/// FNV-1a hash per lowercase alphanumeric word, in document order.
fn word_hashes(text: &str) -> Vec<u64> {
    let mut words = Vec::new();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut in_word = false;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            in_word = true;
            for &b in ch
                .to_lowercase()
                .next()
                .unwrap_or(ch)
                .encode_utf8(&mut buf)
                .as_bytes()
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        } else if in_word {
            words.push(h);
            h = 0xcbf2_9ce4_8422_2325;
            in_word = false;
        }
    }
    if in_word {
        words.push(h);
    }
    words
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encode_decode() {
        let s = Sketch::compute("the meridian arc measurement of 1792 established the metre");
        let decoded = Sketch::decode(&s.encode()).unwrap();
        assert_eq!(s, decoded);
        assert!(Sketch::decode(&[0u8; 10]).is_none());
    }

    #[test]
    fn identity_and_disjoint() {
        let a = Sketch::compute(
            "coastal desalination plant approved after a long public consultation process \
             with environmental review and community input on water security",
        );
        assert!(a.containment(&a) > 0.95, "self-containment ≈ 1");
        let b = Sketch::compute(
            "championship football final ends in penalty shootout drama as the visiting \
             side lifts the trophy after extra time heroics from the goalkeeper",
        );
        assert!(
            a.containment(&b) < CONTAINMENT_TAU,
            "unrelated text stays below τ"
        );
    }

    /// The asymmetry that broke raw Jaccard in suite 9: a truncated copy with
    /// boilerplate must still show high containment against its origin.
    #[test]
    fn truncated_copy_has_high_containment() {
        let origin: String = (0..40)
            .map(|i| format!("origin word{i} content{i} report{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let truncated_with_boilerplate = format!(
            "{} reporting syndication outlet boilerplate footer",
            origin
                .split_whitespace()
                .take(70)
                .collect::<Vec<_>>()
                .join(" ")
        );
        let a = Sketch::compute(&origin);
        let b = Sketch::compute(&truncated_with_boilerplate);
        assert!(
            a.containment(&b) >= CONTAINMENT_TAU,
            "containment {} must clear τ={CONTAINMENT_TAU}",
            a.containment(&b)
        );
    }

    #[test]
    fn tokenization_is_case_and_punctuation_insensitive() {
        let a = Sketch::compute("The Quick-Brown Fox, jumps over the lazy dog!");
        let b = Sketch::compute("the quick brown fox jumps over the lazy dog");
        assert_eq!(a.sig, b.sig);
        assert_eq!(a.shingles, b.shingles);
    }

    #[test]
    fn tiny_document_is_a_single_shingle() {
        let s = Sketch::compute("two words");
        assert_eq!(s.shingles, 1);
    }

    /// The P7 exit-drill false merge: a tiny shingle set against a large
    /// unrelated document. One chance bin collision (≈21%/pair without the
    /// MIN_MATCH_BINS floor) amplifies through the containment denominator to
    /// ≥τ. With the floor, none of many such pairs may merge.
    #[test]
    fn size_asymmetry_noise_cannot_clear_tau() {
        let large = Sketch::compute(
            &(0..400)
                .map(|i| format!("article word{i} body{}", i * 7))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mut merges = 0;
        for k in 0..500 {
            let tiny = Sketch::compute(&format!(
                "note{k} short reminder item{} list entry final",
                k * 13
            ));
            assert!(tiny.shingles < 10, "test premise: tiny set");
            if tiny.containment(&large) >= CONTAINMENT_TAU {
                merges += 1;
            }
        }
        assert_eq!(merges, 0, "{merges}/500 tiny-vs-large chance merges");
    }
}
