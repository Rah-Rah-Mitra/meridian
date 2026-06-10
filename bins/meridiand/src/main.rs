//! `meridiand` — the single Meridian process (SPEC §2: in-process function calls beat
//! microservice IPC at 8GB RAM; Docker is packaging, not decomposition).
//!
//! Phase-0 stub. Phase 1 wires: figment config → telemetry (privacy redaction layer
//! first) → tokio runtime (4 workers) + rayon pool (4 threads, nice 5) → axum serve
//! on 127.0.0.1:8080 (SPEC §7.2, §13.4).

use std::process::ExitCode;

fn main() -> ExitCode {
    let config = meridian_common::MeridianConfig::default();
    eprintln!(
        "meridiand {} — Phase 0 scaffold; no server yet (would bind {}:{})",
        env!("CARGO_PKG_VERSION"),
        config.server.bind,
        config.server.port,
    );
    // Non-zero so health checks and operators can never mistake the stub for a
    // running service.
    ExitCode::FAILURE
}
