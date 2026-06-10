//! Readability extraction via `dom_smoothie` (ADR-03). The fetched body is
//! parsed, the clean text + title extracted, and the raw HTML DISCARDED by the
//! caller (SPEC §6.1 hard rule — content hash only).

#[derive(Debug, Clone)]
pub struct Extracted {
    pub title: Option<String>,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
#[error("extraction failed: {0}")]
pub struct ExtractError(String);

/// Extract main content from HTML. CPU-bound (~ms for typical pages) — callers
/// on the async path run this via the rayon bridge.
pub fn extract_html(html: &str, url: &str) -> Result<Extracted, ExtractError> {
    let mut readability = dom_smoothie::Readability::new(html, Some(url), None)
        .map_err(|e| ExtractError(e.to_string()))?;
    let article = readability
        .parse()
        .map_err(|e| ExtractError(e.to_string()))?;
    let title = article.title.trim().to_owned();
    Ok(Extracted {
        title: (!title.is_empty()).then_some(title),
        text: article.text_content.trim().to_owned(),
    })
}

/// Plain-text bodies skip readability entirely.
pub fn extract_plaintext(body: &str) -> Extracted {
    Extracted {
        title: None,
        text: body.trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_main_content_and_title() {
        let html = r#"<!DOCTYPE html><html><head><title>Test Article</title></head>
            <body><nav>Home | About | Login</nav>
            <article><h1>Test Article</h1>
            <p>The quick brown fox jumps over the lazy dog. This paragraph is the
            main content of the page and should survive extraction intact.</p>
            <p>A second paragraph keeps the scorer interested in the article body
            rather than the navigation chrome around it.</p></article>
            <footer>Copyright 2026 | Privacy | Terms</footer></body></html>"#;
        let out = extract_html(html, "https://example.com/a").expect("extracts");
        assert!(out.text.contains("quick brown fox"));
        assert!(
            !out.text.contains("Privacy | Terms"),
            "chrome leaked: {}",
            out.text
        );
        assert_eq!(out.title.as_deref(), Some("Test Article"));
    }

    #[test]
    fn malformed_html_does_not_panic() {
        for bad in [
            "<html><body><div><p>unclosed everything",
            "<<<>>>&&&",
            "",
            "<script>alert(1)</script>",
            "\u{0}\u{1}\u{2}binary-ish\u{fffd}",
        ] {
            // Err is acceptable; panicking is not.
            let _ = extract_html(bad, "https://example.com/x");
        }
    }
}
