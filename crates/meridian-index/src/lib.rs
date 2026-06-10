//! Tantivy lexical index (SPEC §5): schema, IndexWriter management, merge policy,
//! snippets, fast fields, H3 range pruning — and the `SegmentStore` scale-out seam.
//!
//! Status: Phase 1 — the tantivy-backed [`lexical::LexicalIndex`] is live. The
//! `SegmentStore` seam below stays the v2 contract (SPEC §18: object storage =
//! implement one trait); in v1 tantivy owns segment lifecycle locally.

pub mod lexical;

use meridian_common::MeridianError;

/// Identifies one immutable index segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentId(pub u128);

/// Metadata the store keeps per segment (sizes feed the disk-budget tripwires).
#[derive(Debug, Clone)]
pub struct SegmentMeta {
    pub id: SegmentId,
    pub bytes: u64,
    pub num_docs: u32,
}

/// The scale-out seam (SPEC §3): all segment access goes through this trait.
///
/// v1 = local NVMe/SD directory. v2 = object storage (Quickwit-style) on the
/// edge-server tier — by implementing this trait, not by rewriting the index layer.
///
/// `async fn` in a public trait: internal workspace API, same caveat as
/// `meridian_egress::Egress`.
#[allow(async_fn_in_trait)]
pub trait SegmentStore {
    /// Enumerate live segments visible to searchers.
    async fn list_segments(&self) -> Result<Vec<SegmentMeta>, MeridianError>;

    /// Open a segment for reading (mmap locally; ranged GETs in v2).
    async fn open_segment(&self, id: SegmentId) -> Result<SegmentHandle, MeridianError>;

    /// Atomically publish a newly written/merged segment.
    async fn publish_segment(&self, meta: SegmentMeta) -> Result<(), MeridianError>;

    /// Drop a segment (post-merge cleanup, `/v1/forget` compactions).
    async fn delete_segment(&self, id: SegmentId) -> Result<(), MeridianError>;
}

/// Opaque open-segment handle; Phase 1 backs this with tantivy's directory types.
pub struct SegmentHandle {
    _private: (),
}
