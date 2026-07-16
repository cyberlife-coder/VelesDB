//! Recoverable source handles: `ctx://source/<fragment_id>`.
//!
//! Every fragment the compiler touches stays addressable through one handle
//! scheme so a decision is never a dead end: what was externalized (or only
//! partially packed) can be fetched back. The memoryless core only *mints*
//! handles; resolving them against stored originals is the memory bridge's
//! job (US-002).

use super::model::SourceReference;

/// URI scheme prefix of every context source handle.
pub(crate) const HANDLE_PREFIX: &str = "ctx://source/";

/// The recoverable address of a fragment's original content. Handles are
/// **content-addressed** (FNV-1a hash of the original bytes), never minted
/// from caller-supplied ids — two different fragments sharing a caller id
/// therefore keep two distinct, unambiguous addresses.
#[must_use]
pub(crate) fn handle_for(content_hash: u64) -> String {
    format!("{HANDLE_PREFIX}{content_hash}")
}

/// The source pointer recorded for one distinct fragment.
#[must_use]
pub(crate) fn source_for(fragment_id: u64, content_hash: u64) -> SourceReference {
    SourceReference {
        fragment_id,
        handle: handle_for(content_hash),
        memory_id: None,
    }
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
