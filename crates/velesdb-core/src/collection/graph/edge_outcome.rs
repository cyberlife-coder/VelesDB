//! The outcome of a single edge removal.
//!
//! `remove_edge` returns `bool` across every layer of the graph stack, and that
//! one bit conflates four genuinely different outcomes:
//!
//! 1. the edge was removed;
//! 2. the edge was not there to begin with — benign, and callers rely on it;
//! 3. the write-ahead log refused the remove, so the store was left untouched
//!    and durability is broken;
//! 4. the edge id was live in the index but missing from its source shard — an
//!    index desynchronisation that can leave a dangling incoming half-edge.
//!
//! Cases 2 and 3/4 are indistinguishable from the outside *after* the call: in
//! both the edge is absent from the index and from the source shard. Only the
//! layer performing the removal can tell them apart, which is why the internal
//! path reports this enum instead of re-deriving the answer afterwards.
//!
//! The public `remove_edge -> bool` API is unchanged and keeps folding this
//! down to `matches!(.., Removed)`; this type stays `pub(crate)` so the folding
//! happens at the crate boundary.

/// What a single `remove_edge` attempt actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EdgeRemoval {
    /// The edge existed and was removed.
    Removed,
    /// The edge was not present. Benign: removing an absent edge is a
    /// documented no-op and must never be reported as a failure.
    Absent,
    /// The removal was attempted and failed. The edge may still be present, in
    /// whole or in part. Carries a human-readable reason for the caller to
    /// surface — no error is swallowed on this path.
    Failed(String),
}

impl EdgeRemoval {
    /// Folds the outcome back to the historical `bool` contract: `true` only
    /// when the edge was actually removed.
    pub(crate) fn removed(&self) -> bool {
        matches!(self, Self::Removed)
    }
}
