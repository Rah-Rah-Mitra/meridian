//! Shared foundation for all Meridian crates: configuration, errors, ids, core types.
//!
//! Per SPEC §5 dependency rules, this crate depends on **nothing internal**; every other
//! crate may depend on it. Telemetry init moves here in Phase 1.
//!
//! Status: Phase-0 scaffold — types are skeletal and will grow with each phase.

pub mod config;
pub mod error;
pub mod ids;

pub use config::MeridianConfig;
pub use error::MeridianError;
