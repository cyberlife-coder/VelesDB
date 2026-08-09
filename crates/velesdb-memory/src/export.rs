//! Export a store's facts as JSONL — one JSON object per line — WITHOUT an
//! embedder.
//!
//! The design constraint is the second half of the store's ownership story:
//! `list_memories` audits a store the daemon can serve, and THIS path reads
//! a store the daemon may refuse — a provenance mismatch (#1751) blocks the
//! service from opening, and blocking the user's own data behind the very
//! misconfiguration they are trying to escape would make the refusal a trap
//! instead of a guard. Reading content requires no vectors, so the export
//! opens the engine directly and never builds an embedder at all.
//!
//! Same enumeration as the migration rebuild ([`crate::migration`], #1762):
//! the id-keyed cursor walk, complete and TTL-skipping — an export must show
//! what the store would still serve, and an expired fact is not it.

use std::io::Write;
use std::path::Path;

use crate::error::MemoryError;
use crate::storage::{is_internal_scaffolding, strip_reserved_keys};

/// Page size of the export walk. Purely an I/O batch — no ranking, no
/// scoring — so the only tradeoff is memory per page.
const EXPORT_BATCH: usize = 512;

/// Write every live fact of the store at `store_dir` to `out`, one JSON
/// object per line: `{"id", "id_str", "content", "metadata"}`. Returns how
/// many lines were written.
///
/// Visibility follows `list_memories`' policy: internal graph scaffolding is
/// skipped and reserved keys are stripped (the auto-stamped date survives),
/// unless `include_internal` — a backup wants everything verbatim.
///
/// No embedder is built and no provenance check runs: this is the one read
/// path that must work on a store whose configured embedder no longer
/// matches — your data stays yours even mid-misconfiguration.
///
/// # Errors
/// Returns [`MemoryError`] when the store cannot be opened or walked, and
/// I/O errors from `out` wrapped as [`MemoryError::InvalidFilter`]-free
/// storage errors.
pub fn export_jsonl<W: Write>(
    store_dir: &Path,
    out: &mut W,
    include_internal: bool,
) -> Result<u64, MemoryError> {
    // Refused BEFORE the engine sees the path: `Database::open` creates a
    // store that is not there, and an export that materialises an empty
    // store at a typo'd path would corrupt the very question it answers
    // ("what is in my store?" — nothing, now).
    if !store_dir.is_dir() {
        return Err(MemoryError::Storage(velesdb_core::Error::Query(format!(
            "no store directory at {} — nothing to export",
            store_dir.display()
        ))));
    }
    let db = velesdb_core::Database::open(store_dir)?;
    let mut written = 0_u64;
    let mut cursor: Option<u64> = None;
    loop {
        let (facts, next) =
            crate::migration::scroll_page(&db, "_semantic_memory", cursor, EXPORT_BATCH)?;
        written += write_page(out, &facts, include_internal)?;
        match next {
            Some(id) => cursor = Some(id),
            None => break,
        }
    }
    Ok(written)
}

/// Write one page of the walk, returning how many lines it produced. Split
/// from [`export_jsonl`] so the walk reads as a walk — page, write, advance.
fn write_page<W: Write>(
    out: &mut W,
    facts: &[crate::migration::RawFact],
    include_internal: bool,
) -> Result<u64, MemoryError> {
    let mut written = 0_u64;
    for fact in facts {
        if let Some(line) = jsonl_line(fact, include_internal) {
            writeln!(out, "{line}").map_err(|err| {
                MemoryError::Storage(velesdb_core::Error::Query(format!(
                    "export write failed: {err}"
                )))
            })?;
            written += 1;
        }
    }
    Ok(written)
}

/// One fact as its JSONL line, or `None` when the visibility policy skips
/// it (internal scaffolding under the default view). Split from the walk so
/// the loop reads as what it is — enumerate, filter, write.
fn jsonl_line(
    fact: &crate::migration::RawFact,
    include_internal: bool,
) -> Option<serde_json::Value> {
    let split = crate::storage::RawListedFact::from_raw(fact);
    if !include_internal && is_internal_scaffolding(&split.payload) {
        return None;
    }
    let metadata = if include_internal {
        (!split.payload.is_empty()).then_some(split.payload)
    } else {
        strip_reserved_keys(Some(split.payload))
    };
    Some(serde_json::json!({
        "id": split.id,
        "id_str": split.id.to_string(),
        "content": split.content,
        "metadata": metadata,
    }))
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
