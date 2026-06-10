//! Cross-cutting privacy guardrails (SPEC §13.4–§13.5).
//!
//! Everything that logs, holds a secret, or stores user-derived data routes through
//! this crate. Phase-1 brings: the tracing redaction layer, the salted/rotating
//! client-IP hasher (rate-limit keys only), per-lane header/UA policy, and re-exported
//! `secrecy`/`zeroize` secret types. Phase-5 adds retention TTL sweeps and the
//! `/v1/forget` deletion jobs.
//!
//! Status: Phase-0 scaffold — only the `Redacted<T>` wrapper is real, because every
//! other crate's structs need it from day one.

/// Wrapper for any field carrying user input (query text, client IP, fetched URL).
///
/// Its `Debug`/`Display` print `‹redacted›`, so user data cannot leak through derived
/// formatting at any log level (SPEC §13.4 "No-log by default"). Access to the inner
/// value is explicit via [`Redacted::expose`], which keeps leak sites greppable.
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Explicitly access the wrapped value. Every call site is a deliberate,
    /// reviewable decision to handle user data.
    pub fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\u{2039}redacted\u{203a}")
    }
}

impl<T> std::fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\u{2039}redacted\u{203a}")
    }
}

impl<T> From<T> for Redacted<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::Redacted;

    #[test]
    fn debug_and_display_never_reveal_inner_value() {
        let secret = Redacted::new("canary-3f9a-query".to_owned());
        assert_eq!(format!("{secret:?}"), "‹redacted›");
        assert_eq!(format!("{secret}"), "‹redacted›");
        assert_eq!(secret.expose(), "canary-3f9a-query");
    }
}
