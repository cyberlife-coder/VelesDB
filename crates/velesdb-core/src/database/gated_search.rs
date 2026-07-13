//! Governed (gated) read primitives at the [`Database`] facade.
//!
//! Search entry points that do not build a VelesQL `Query` — REST vector /
//! text / hybrid search, and memory recall — historically bypassed the
//! control-plane read gate because [`Database::get_vector_collection`] hands
//! back a detached [`VectorCollection`](crate::VectorCollection) with no
//! observer reference. These methods restore governance for those paths: they
//! consult the observer via [`Database::read_gate_raw`] (the same resolver the
//! VelesQL gate uses), then delegate to the collection search leaf, applying any
//! observer-supplied scope narrowing.
//!
//! When no observer is registered the gate is a single `Option` check and the
//! search runs exactly as the ungated path did (zero-overhead contract).

use crate::filter::{Condition, Filter};
use crate::observer::QueryOperationKind;
use crate::point::SearchResult;
use crate::{Error, Result};

use super::query_engine::RawGateOutcome;
use super::Database;

/// A non-VelesQL read routed through the control-plane gate.
///
/// Each variant maps to a [`VectorCollection`](crate::VectorCollection) search
/// leaf and to a [`QueryOperationKind`] the observer sees. Observer-supplied
/// scope filters are AND-composed with any caller filter before execution, so
/// narrowing can only shrink the result set, never widen it.
#[derive(Debug, Clone, Copy)]
pub enum GatedRead<'a> {
    /// Dense kNN similarity search.
    ///
    /// `ef` / `quality` tune recall via the engine's dedicated entry points.
    /// They are honoured only when no filter is in effect: when a caller filter
    /// or an observer scope filter is present the search routes through the
    /// filtered leaf (`search_with_filter`), which the engine exposes
    /// separately from the tuning leaves.
    Dense {
        /// Query vector.
        query: &'a [f32],
        /// Number of neighbours to return.
        k: usize,
        /// Optional `ef_search` override.
        ef: Option<usize>,
        /// Optional named search-quality profile.
        quality: Option<crate::SearchQuality>,
        /// Optional caller metadata filter.
        filter: Option<&'a Filter>,
    },
    /// Full-text / BM25 search.
    Text {
        /// Query text.
        query: &'a str,
        /// Number of results to return.
        k: usize,
        /// Optional caller metadata filter.
        filter: Option<&'a Filter>,
    },
    /// Hybrid dense + BM25 fused search.
    Hybrid {
        /// Dense query vector.
        vector: &'a [f32],
        /// Query text.
        text: &'a str,
        /// Number of results to return.
        k: usize,
        /// Optional dense/text blend factor.
        alpha: Option<f32>,
        /// Optional caller metadata filter.
        filter: Option<&'a Filter>,
    },
}

impl GatedRead<'_> {
    /// The [`QueryOperationKind`] the observer is told this read represents, so
    /// premium RBAC/audit records the correct operation label.
    fn operation_kind(&self) -> QueryOperationKind {
        match self {
            GatedRead::Dense { .. } => QueryOperationKind::VectorSearch,
            GatedRead::Text { .. } => QueryOperationKind::TextSearch,
            GatedRead::Hybrid { .. } => QueryOperationKind::HybridSearch,
        }
    }
}

/// Lowers an [`AccessScope`](crate::observer::AccessScope) filter — expressed in
/// the VelesQL [`Condition`](crate::velesql::Condition) language — into the
/// lower-level [`filter::Filter`](crate::filter::Filter) the raw search leaves
/// accept. Infallible: reuses the existing
/// `From<velesql::Condition> for filter::Condition` conversion (the same
/// lowering the WHERE evaluator uses).
#[must_use]
pub(crate) fn scope_to_core_filter(condition: crate::velesql::Condition) -> Filter {
    Filter::new(Condition::from(condition))
}

/// AND-composes a caller filter with an observer scope filter. The result
/// matches only rows satisfying both, so composing a scope can only narrow.
fn and_filters(caller: Option<&Filter>, scope: Option<Filter>) -> Option<Filter> {
    match (caller, scope) {
        (None, None) => None,
        (Some(c), None) => Some(c.clone()),
        (None, Some(s)) => Some(s),
        (Some(c), Some(s)) => Some(Filter::new(Condition::And {
            conditions: vec![c.condition.clone(), s.condition],
        })),
    }
}

impl Database {
    /// Public read-gate check for search paths that do not map onto
    /// [`GatedRead`] — sparse, batch, multi-query, graph-embedding and MATCH —
    /// so their callers can enforce governance without a bespoke gated method
    /// per return type. Consults the observer for the given collection /
    /// operation / principal / tenant and reports what the caller must do:
    ///
    /// * `Ok(None)` — allow the read unmodified;
    /// * `Ok(Some(filter))` — allow, but AND this scope filter into the search
    ///   (callers whose search variant cannot apply a metadata filter must fail
    ///   closed rather than run unfiltered);
    /// * `Err(_)` — the read is denied (or the observer failed internally); the
    ///   caller must not touch the data plane.
    ///
    /// With no observer registered this is a single `Option` check returning
    /// `Ok(None)` (zero-overhead contract).
    ///
    /// # Errors
    ///
    /// Returns the observer's `Deny` error when access is refused, or the
    /// observer's own error on an internal failure.
    pub fn authorize_read(
        &self,
        collection: &str,
        operation: QueryOperationKind,
        principal: Option<&str>,
        tenant_hint: Option<&str>,
    ) -> Result<Option<Filter>> {
        match self.read_gate_raw(collection, operation, principal, tenant_hint)? {
            RawGateOutcome::Allow => Ok(None),
            RawGateOutcome::Deny(err) => Err(err),
            RawGateOutcome::Scope(scope) => Ok(scope.filter.map(scope_to_core_filter)),
        }
    }

    /// Executes a search through the control-plane read gate.
    ///
    /// Consults the registered observer via
    /// [`read_gate_raw`](Self::read_gate_raw) for the
    /// collection / operation / principal / tenant, then:
    /// * [`Allow`](RawGateOutcome::Allow) — runs the search unmodified;
    /// * [`Deny`](RawGateOutcome::Deny) — returns the supplied error and zero
    ///   results;
    /// * [`Scope`](RawGateOutcome::Scope) — AND-composes the scope filter with
    ///   any caller filter before running the search.
    ///
    /// With no observer registered this is a single `Option` check followed by
    /// the same leaf call the ungated path used (zero-overhead contract).
    ///
    /// # Errors
    ///
    /// Returns [`Error::CollectionNotFound`] if the collection does not exist,
    /// the observer's `Deny` error when access is refused, or any error from the
    /// underlying search leaf.
    pub fn gated_search(
        &self,
        collection: &str,
        principal: Option<&str>,
        tenant_hint: Option<&str>,
        read: GatedRead<'_>,
    ) -> Result<Vec<SearchResult>> {
        let scope_filter =
            match self.read_gate_raw(collection, read.operation_kind(), principal, tenant_hint)? {
                RawGateOutcome::Allow => None,
                RawGateOutcome::Deny(err) => return Err(err),
                RawGateOutcome::Scope(scope) => scope.filter.map(scope_to_core_filter),
            };

        let coll = self
            .get_vector_collection(collection)
            .ok_or_else(|| Error::CollectionNotFound(collection.to_string()))?;

        match read {
            GatedRead::Dense {
                query,
                k,
                ef,
                quality,
                filter,
            } => match and_filters(filter, scope_filter) {
                Some(f) => coll.search_with_filter(query, k, &f),
                None => match (ef, quality) {
                    (Some(ef), _) => coll.search_with_ef(query, k, ef),
                    (None, Some(q)) => coll.search_with_quality(query, k, q),
                    (None, None) => coll.search(query, k),
                },
            },
            GatedRead::Text { query, k, filter } => match and_filters(filter, scope_filter) {
                Some(f) => coll.text_search_with_filter(query, k, &f),
                None => coll.text_search(query, k),
            },
            GatedRead::Hybrid {
                vector,
                text,
                k,
                alpha,
                filter,
            } => match and_filters(filter, scope_filter) {
                Some(f) => coll.hybrid_search_with_filter(vector, text, k, alpha, &f),
                None => coll.hybrid_search(vector, text, k, alpha),
            },
        }
    }
}
