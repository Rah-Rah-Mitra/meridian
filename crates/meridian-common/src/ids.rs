//! Strongly-typed identifiers shared across crates.

use serde::{Deserialize, Serialize};

/// Stable document identity inside the local index (lexical + vector stores share it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocId(pub u64);

/// blake3(content) truncated to 16 bytes — dedup key and `/v1/forget` tombstone key
/// (SPEC §9.3, §13.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 16]);

/// Operator-assigned identifier for a WireGuard region lane, e.g. `"eu-west"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RegionId(pub String);
