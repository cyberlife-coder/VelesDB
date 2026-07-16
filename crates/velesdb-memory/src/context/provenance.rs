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

/// The recoverable address of a fragment's original content.
#[must_use]
pub(crate) fn handle_for(fragment_id: u64) -> String {
    format!("{HANDLE_PREFIX}{fragment_id}")
}

/// The source pointer recorded for one distinct fragment.
#[must_use]
pub(crate) fn source_for(fragment_id: u64) -> SourceReference {
    SourceReference {
        fragment_id,
        handle: handle_for(fragment_id),
        memory_id: None,
    }
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
