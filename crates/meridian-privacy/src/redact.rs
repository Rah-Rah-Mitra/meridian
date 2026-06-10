//! The no-log-by-default tracing layer (SPEC §13.4).
//!
//! Defense-in-depth has three rings; this is ring 2:
//! 1. `Redacted<T>` / secret types make leaks impossible at the call site.
//! 2. THIS layer — the only writer of log output — redacts denylisted field
//!    names (`q`, `query`, `ip`, `client_ip`, `url`, `target`, `host`, …) as it
//!    formats, so even a field recorded raw by a future bug or a dependency
//!    cannot reach stdout in the clear at INFO and above.
//! 3. The privacy smoke test greps real output for canaries in CI.
//!
//! A `tracing` Layer cannot mutate events for downstream layers, so redaction
//! must live in the formatting layer itself — hence this is a self-contained
//! writer, not a filter in front of `fmt::layer()`.

use std::fmt::Write as _;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Field names whose values are user data or network identifiers — never logged
/// in the clear at INFO+ (SPEC §13.4).
const DENYLIST: &[&str] = &[
    "q",
    "query",
    "query_text",
    "ip",
    "client_ip",
    "peer",
    "peer_addr",
    "remote_addr",
    "x_forwarded_for",
    "url",
    "target_url",
    "host",
    "domain",
];

const REDACTED: &str = "\u{2039}redacted\u{203a}";

/// Formatting layer with built-in field redaction. Writes single-line events to
/// the supplied writer (stdout in production, a buffer in tests).
pub struct RedactLayer<W = fn() -> std::io::Stdout> {
    make_writer: W,
    /// When true (explicit `privacy.debug_query_logging`), TRACE-level events
    /// may carry raw query text. All other levels still redact.
    debug_query_logging: bool,
}

impl RedactLayer {
    pub fn stdout(debug_query_logging: bool) -> Self {
        Self {
            make_writer: std::io::stdout,
            debug_query_logging,
        }
    }
}

impl<W> RedactLayer<W> {
    pub fn with_writer(make_writer: W, debug_query_logging: bool) -> Self {
        Self {
            make_writer,
            debug_query_logging,
        }
    }
}

struct RedactingVisitor {
    line: String,
    message: String,
    redact: bool,
}

impl RedactingVisitor {
    fn field_value(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
            // Strip the Debug quoting on plain string messages.
            if self.message.len() >= 2 && self.message.starts_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_owned();
            }
            return;
        }
        let denied = self.redact && DENYLIST.contains(&field.name());
        if denied {
            let _ = write!(self.line, " {}={REDACTED}", field.name());
        } else {
            let _ = write!(self.line, " {}={value:?}", field.name());
        }
    }
}

impl Visit for RedactingVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.field_value(field, value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.field_value(field, &value);
    }
}

impl<S, W, O> tracing_subscriber::Layer<S> for RedactLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: Fn() -> O + 'static,
    O: std::io::Write,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        // Raw user data may pass ONLY at TRACE and only when explicitly enabled.
        let redact = !(self.debug_query_logging && *meta.level() == Level::TRACE);
        let mut visitor = RedactingVisitor {
            line: String::new(),
            message: String::new(),
            redact,
        };
        event.record(&mut visitor);

        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut out = (self.make_writer)();
        let _ = writeln!(
            out,
            "{unix_ms} {:>5} {}: {}{}",
            meta.level(),
            meta.target(),
            visitor.message,
            visitor.line
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        fn contents(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture(debug_query_logging: bool, f: impl FnOnce()) -> String {
        let sink = Sink::default();
        let writer_sink = sink.clone();
        let layer = RedactLayer::with_writer(move || writer_sink.clone(), debug_query_logging);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, f);
        sink.contents()
    }

    #[test]
    fn denylisted_fields_are_redacted_at_info() {
        let out = capture(false, || {
            tracing::info!(
                q = "CANARY-query",
                ip = "203.0.113.9",
                route = "/v1/search",
                "served"
            );
        });
        assert!(!out.contains("CANARY-query"), "query text leaked: {out}");
        assert!(!out.contains("203.0.113.9"), "client ip leaked: {out}");
        assert!(
            out.contains("route=\"/v1/search\""),
            "benign field dropped: {out}"
        );
        assert!(out.contains("‹redacted›"));
    }

    #[test]
    fn trace_stays_redacted_unless_flag_enabled() {
        let locked = capture(false, || {
            tracing::trace!(q = "CANARY-trace", "debug");
        });
        assert!(!locked.contains("CANARY-trace"));

        let open = capture(true, || {
            tracing::trace!(q = "CANARY-trace", "debug");
        });
        assert!(
            open.contains("CANARY-trace"),
            "explicit debug flag must allow TRACE: {open}"
        );

        let still_info = capture(true, || {
            tracing::info!(q = "CANARY-info", "served");
        });
        assert!(
            !still_info.contains("CANARY-info"),
            "flag must NOT unlock INFO: {still_info}"
        );
    }
}
