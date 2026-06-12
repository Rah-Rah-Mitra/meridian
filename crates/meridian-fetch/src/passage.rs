//! Sentence-aligned passage splitting for answer mode (Phase 10, ADR-29).
//! Pure text manipulation — no I/O; the planner feeds fetched-and-extracted
//! full text and CE-scores the (query, passage) pairs.

/// Split extracted text into sentence-aligned passages of at most
/// `max_chars`, dropping fragments too short to carry an answer. Sentences
/// longer than `max_chars` are hard-wrapped at a char boundary rather than
/// dropped — a long sentence is still a passage, not an error.
pub fn split_passages(text: &str, max_chars: usize, max_passages: usize) -> Vec<String> {
    const MIN_CHARS: usize = 40;
    let mut passages: Vec<String> = Vec::new();
    let mut current = String::new();

    let push_current = |current: &mut String, passages: &mut Vec<String>| {
        let trimmed = current.trim();
        if trimmed.chars().count() >= MIN_CHARS {
            passages.push(trimmed.to_owned());
        }
        current.clear();
    };

    for sentence in split_sentences(text) {
        let s_len = sentence.chars().count();
        let cur_len = current.chars().count();
        if cur_len > 0 && cur_len + s_len + 1 > max_chars {
            push_current(&mut current, &mut passages);
            if passages.len() >= max_passages {
                return passages;
            }
        }
        if s_len > max_chars {
            // Hard-wrap the oversized sentence on its own.
            push_current(&mut current, &mut passages);
            let chars: Vec<char> = sentence.chars().collect();
            for chunk in chars.chunks(max_chars) {
                if passages.len() >= max_passages {
                    return passages;
                }
                let piece: String = chunk.iter().collect();
                if piece.trim().chars().count() >= MIN_CHARS {
                    passages.push(piece.trim().to_owned());
                }
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(sentence.trim());
        }
        if passages.len() >= max_passages {
            return passages;
        }
    }
    push_current(&mut current, &mut passages);
    passages.truncate(max_passages);
    passages
}

/// Sentence boundaries: '.', '!', '?', or newline runs. Deliberately simple —
/// extraction already normalized the text, and a slightly-off boundary costs
/// nothing (passages overlap sentences, not meaning).
fn split_sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_sentences_and_respects_cap() {
        let text = "The first finding was clear and well documented in the report. \
                    The second finding contradicted it across all three trials measured. \
                    A third sentence adds the necessary statistical context for both.";
        let p = split_passages(text, 120, 8);
        assert!(p.len() >= 2, "{p:?}");
        assert!(p.iter().all(|s| s.chars().count() <= 120), "{p:?}");
        assert!(p.iter().all(|s| s.chars().count() >= 40), "{p:?}");
    }

    #[test]
    fn drops_tiny_totals_and_caps_count() {
        // A sub-40-char text cannot carry an answer — dropped outright.
        assert!(split_passages("Too tiny. Really.", 100, 4).is_empty());
        // Tiny sentences ACCUMULATE into real passages — that is desired.
        let text = "Short. Tiny. No. ".repeat(50);
        assert!(!split_passages(&text, 100, 4).is_empty());
        let real =
            "This sentence is comfortably long enough to be a passage candidate. ".repeat(50);
        let p = split_passages(&real, 200, 4);
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn hard_wraps_oversized_sentences() {
        let long = "word ".repeat(300); // one giant "sentence", no terminator
        let p = split_passages(&long, 100, 8);
        assert!(!p.is_empty());
        assert!(p.iter().all(|s| s.chars().count() <= 100));
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(split_passages("", 500, 32).is_empty());
        assert!(split_passages("   \n  ", 500, 32).is_empty());
    }
}
