//! Admin executor for `VelesQL` v3.6.
//!
//! Handles FLUSH statements by delegating to existing [`Database`]
//! and collection flush APIs.

use crate::velesql::{AdminStatement, FlushStatement};
use crate::{Error, Result, SearchResult};

use super::Database;

/// Dispatches to `flush()` (fast) or `flush_full()` based on the `full` flag.
///
/// Avoids repeating `if full { flush_full() } else { flush() }` at each call
/// site while staying closure-free (no `redundant_closure_for_method_calls`).
macro_rules! flush_dispatch {
    ($coll:expr, $full:expr) => {
        if $full {
            $coll.flush_full()
        } else {
            $coll.flush()
        }
    };
}

impl Database {
    /// Dispatches an admin statement to the appropriate executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the target collection does not exist (named FLUSH)
    /// or if the flush operation itself fails.
    pub(super) fn execute_admin(&self, stmt: &AdminStatement) -> Result<Vec<SearchResult>> {
        match stmt {
            AdminStatement::Flush(flush) => self.execute_flush(flush),
        }
    }

    /// Executes a `FLUSH [FULL] [collection]` statement.
    ///
    /// - No collection: flushes all collections in every registry.
    /// - Named collection: resolves and flushes that single collection.
    /// - `full = true`: includes index serialization (`flush_full`).
    /// - `full = false`: WAL-only fast flush.
    ///
    /// # Errors
    ///
    /// Returns an error if a named collection does not exist or if any
    /// individual flush operation fails.
    fn execute_flush(&self, stmt: &FlushStatement) -> Result<Vec<SearchResult>> {
        if let Some(ref name) = stmt.collection {
            self.flush_single_collection(name, stmt.full)?;
        } else {
            self.flush_all_collections(stmt.full)?;
        }

        let payload = serde_json::json!({
            "status": "flushed",
            "full": stmt.full,
        });
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Flushes a single collection by name, choosing fast or full mode.
    ///
    /// Checks vector, graph, and metadata registries in order.
    ///
    /// # Errors
    ///
    /// Returns `CollectionNotFound` if the name does not match any registry.
    fn flush_single_collection(&self, name: &str, full: bool) -> Result<()> {
        if let Some(vc) = self.get_vector_collection(name) {
            return flush_dispatch!(vc, full);
        }
        if let Some(gc) = self.get_graph_collection(name) {
            return flush_dispatch!(gc, full);
        }
        if let Some(mc) = self.get_metadata_collection(name) {
            return flush_dispatch!(mc, full);
        }
        Err(Error::CollectionNotFound(name.to_string()))
    }

    /// Flushes all collections across all registries.
    ///
    /// Iterates vector, graph, and metadata registries. Uses fast or full
    /// mode based on the `full` flag. Fails on the first error encountered.
    ///
    /// # Errors
    ///
    /// Returns the first flush error encountered.
    fn flush_all_collections(&self, full: bool) -> Result<()> {
        for vc in self.vector_colls.read().values() {
            flush_dispatch!(vc, full)?;
        }
        for gc in self.graph_colls.read().values() {
            flush_dispatch!(gc, full)?;
        }
        for mc in self.metadata_colls.read().values() {
            flush_dispatch!(mc, full)?;
        }
        Ok(())
    }
}
