//! MATCH query execution for graph pattern matching (EPIC-045 US-002).
//!
//! This module implements the `execute_match()` method for executing
//! Cypher-like MATCH queries on VelesDB collections.

// Reason: Numeric casts in MATCH query execution are intentional:
// - u64->usize for result limits: limits are small (< 1M) and bounded
// - f64->f32 for embedding vectors: precision sufficient for similarity search
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

mod expand;
mod index_prefilter;
mod order_by;
mod similarity;
mod start_nodes;
mod vector_first;
pub(in crate::collection::search::query) mod where_eval;

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::guardrails::QueryContext;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage as _};
use crate::velesql::{GraphPattern, MatchClause};
use std::collections::{HashMap, HashSet};

/// Storage guards hoisted once per MATCH execution, in the decreed lock
/// order: `vector_storage` (rank 2) before `payload_storage` (rank 3) — the
/// same order writers such as `delete_vector_core_stores` use (see LOCK
/// ORDERING in `collection/types.rs`).
///
/// The top frame acquires both guards exactly once and passes this bundle
/// down the whole execution tree; no callee may re-acquire either lock.
/// A nested `read()` on a lock whose guard is already held by the same
/// thread deadlocks as soon as one writer queues (parking_lot's task-fair
/// `RwLock` is not re-entrant — see
/// `graph_api.rs::validate_edge_endpoints_exist`), and acquiring the pair
/// in the reversed order is the ABBA half of a hold-and-wait cycle with
/// the delete path.
///
/// `metric` is resolved from `config` (rank 1) BEFORE either guard is
/// acquired, so similarity evaluation never takes the rank-1 lock while
/// ranks 2/3 are held.
pub(in crate::collection::search::query) struct MatchStorageGuards<'a> {
    /// Distance metric snapshot (from `config`, rank 1) taken before the guards.
    pub(in crate::collection::search::query) metric: DistanceMetric,
    /// `vector_storage` read guard (rank 2).
    pub(in crate::collection::search::query) vector_guard: &'a MmapStorage,
    /// `payload_storage` read guard (rank 3).
    pub(in crate::collection::search::query) payload_guard: &'a LogPayloadStorage,
    /// Query-lifetime payload memo over `payload_guard` (see [`PayloadMemo`]).
    pub(in crate::collection::search::query) payload_memo: PayloadMemo<'a>,
}

/// Query-lifetime memo of payload reads during MATCH traversal.
///
/// A k-hop pattern re-reads the same node payload once per hop candidacy
/// check, once per WHERE leaf, and once per RETURN item — each a positional
/// disk read plus a full JSON deserialize. Bindings repeat across those
/// stages, so one memoized `Arc` per node id turns the repeats into pointer
/// clones.
///
/// Deliberately NOT used by the start-node full scan: that pass touches every
/// candidate id exactly once, so caching it would grow the memo to the
/// collection size for zero reuse. The memo fills from the ids traversal
/// actually binds, which bounds it by the visited-binding set.
///
/// `RefCell` is sound here: MATCH traversal is single-threaded per query (the
/// guards bundle is `!Sync` by construction and never crosses threads).
pub(in crate::collection::search::query) struct PayloadMemo<'a> {
    storage: &'a LogPayloadStorage,
    cache:
        std::cell::RefCell<rustc_hash::FxHashMap<u64, Option<std::sync::Arc<serde_json::Value>>>>,
}

impl<'a> PayloadMemo<'a> {
    pub(in crate::collection::search::query) fn new(storage: &'a LogPayloadStorage) -> Self {
        Self {
            storage,
            cache: std::cell::RefCell::new(rustc_hash::FxHashMap::default()),
        }
    }

    /// Returns the payload for `id`, reading and deserializing at most once
    /// per query. `None` (absent payload) is memoized too — repeat probes of
    /// payload-less nodes are as common as hits during traversal.
    pub(in crate::collection::search::query) fn get(
        &self,
        id: u64,
    ) -> Option<std::sync::Arc<serde_json::Value>> {
        if let Some(hit) = self.cache.borrow().get(&id) {
            return hit.clone();
        }
        let loaded = self
            .storage
            .retrieve(id)
            .ok()
            .flatten()
            .map(std::sync::Arc::new);
        self.cache.borrow_mut().insert(id, loaded.clone());
        loaded
    }
}

/// Result of a MATCH query traversal.
///
/// Relationship aliases live in exactly one of two maps: fixed-length
/// aliases in `edge_bindings` (scalar edge id), variable-length aliases in
/// `edge_paths` (ordered edge-id list, possibly empty for zero-hop matches).
/// Consumers resolving an alias must consult both (see
/// `where_eval::MatchWhereCtx::edge_targets` for the canonical helper).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MatchResult {
    /// Node ID that was matched.
    pub node_id: u64,
    /// Depth in the traversal (0 = start node).
    pub depth: u32,
    /// Path of edge IDs from start to this node.
    pub path: Vec<u64>,
    /// Bound variables from the pattern (alias -> node_id).
    pub bindings: HashMap<String, u64>,
    /// Bound relationship aliases from the pattern (alias -> edge_id).
    pub edge_bindings: HashMap<String, u64>,
    /// Variable-length relationship aliases (alias -> ordered edge-id list).
    ///
    /// openCypher list semantics: `MATCH (a)-[r*1..3]->(b)` binds `r` to the
    /// LIST of traversed relationships, not a single edge.
    pub edge_paths: HashMap<String, Vec<u64>>,
    /// Similarity score if combined with vector search.
    pub score: Option<f32>,
    /// Projected properties from RETURN clause (EPIC-058 US-007).
    /// Key format: "alias.property" (e.g., "author.name").
    pub projected: HashMap<String, serde_json::Value>,
}

impl MatchResult {
    /// Creates a new match result.
    #[must_use]
    pub fn new(node_id: u64, depth: u32, path: Vec<u64>) -> Self {
        Self {
            node_id,
            depth,
            path,
            bindings: HashMap::new(),
            edge_bindings: HashMap::new(),
            edge_paths: HashMap::new(),
            score: None,
            projected: HashMap::new(),
        }
    }

    /// Adds a variable binding.
    #[must_use]
    pub fn with_binding(mut self, alias: String, node_id: u64) -> Self {
        self.bindings.insert(alias, node_id);
        self
    }

    /// Adds projected properties (EPIC-058 US-007).
    #[must_use]
    pub fn with_projected(mut self, projected: HashMap<String, serde_json::Value>) -> Self {
        self.projected = projected;
        self
    }
}

enum AliasBinding {
    Unchanged,
    Inserted(String),
    Conflict,
}

/// A parsed RETURN clause projection item (Fix #489).
///
/// Replaces the former `parse_property_path()` that silently returned `None`
/// for wildcards, function calls, and bare aliases — leaving `projected` empty.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectionItem<'a> {
    /// `RETURN *` — project all properties from all bound aliases.
    Wildcard,
    /// `RETURN similarity()` — a function call expression.
    /// The inner `&str` is the function name (e.g., `"similarity"`).
    FunctionCall(&'a str),
    /// `RETURN n.name` — a dotted property path.
    PropertyPath {
        /// The alias portion (e.g., `"n"`).
        alias: &'a str,
        /// The property portion (e.g., `"name"` or `"metadata.category"`).
        property: &'a str,
    },
    /// `RETURN n` — a bare alias referring to a bound node.
    BareAlias(&'a str),
}

/// Parses a RETURN clause expression into a [`ProjectionItem`] (Fix #489).
///
/// Handles four patterns:
/// - `"*"` → [`ProjectionItem::Wildcard`]
/// - `"similarity()"` → `ProjectionItem::FunctionCall("similarity")`
/// - `"n.name"` → `ProjectionItem::PropertyPath { alias: "n", property: "name" }`
/// - `"n"` → `ProjectionItem::BareAlias("n")`
#[must_use]
pub fn parse_projection_item(expression: &str) -> ProjectionItem<'_> {
    if expression == "*" {
        return ProjectionItem::Wildcard;
    }

    // Function calls contain '(' — extract the name before the parenthesis.
    if let Some(paren_pos) = expression.find('(') {
        let name = &expression[..paren_pos];
        return ProjectionItem::FunctionCall(name);
    }

    // Dotted property path: split on first dot (both halves must be non-empty).
    if let Some(dot_pos) = expression.find('.') {
        let alias = &expression[..dot_pos];
        let property = &expression[dot_pos + 1..];
        if !alias.is_empty() && !property.is_empty() {
            return ProjectionItem::PropertyPath { alias, property };
        }
    }

    // Everything else is a bare alias (including edge cases like ".x" or "x.").
    ProjectionItem::BareAlias(expression)
}

/// Parses a property path expression like "alias.property" (EPIC-058 US-007).
///
/// Returns `Some((alias, property))` if valid, `None` otherwise.
/// For nested paths like "doc.metadata.category", returns `("doc", "metadata.category")`.
///
/// **Prefer [`parse_projection_item`]** for RETURN clause projection — this function
/// only handles `PropertyPath` cases and returns `None` for wildcards, function calls,
/// and bare aliases.
#[must_use]
pub fn parse_property_path(expression: &str) -> Option<(&str, &str)> {
    match parse_projection_item(expression) {
        ProjectionItem::PropertyPath { alias, property } => Some((alias, property)),
        _ => None,
    }
}

/// Context for collecting single-node pattern results (no relationships).
struct SingleNodeCtx<'a> {
    match_clause: &'a MatchClause,
    params: &'a HashMap<String, serde_json::Value>,
    guards: &'a MatchStorageGuards<'a>,
    seen_pairs: &'a mut std::collections::HashSet<(u64, u64)>,
    all_results: &'a mut Vec<MatchResult>,
    limit: usize,
    /// S4-08: Pre-computed index filter set. `None` = no index available.
    prefilter: Option<std::collections::HashSet<u64>>,
}

/// Mutable state carried through BFS traversal of a single pattern.
struct TraversalCtx<'a> {
    match_clause: &'a MatchClause,
    params: &'a HashMap<String, serde_json::Value>,
    guards: &'a MatchStorageGuards<'a>,
    guardrail: Option<&'a QueryContext>,
    all_results: &'a mut Vec<MatchResult>,
    limit: usize,
    iteration_count: &'a mut u32,
    reported_cardinality: &'a mut usize,
    seen_bindings: &'a mut HashSet<Vec<(u8, String, u64, u64)>>,
}

impl Collection {
    /// Executes a MATCH query on this collection (EPIC-045 US-002).
    ///
    /// This method performs graph pattern matching by:
    /// 1. Finding start nodes matching the first node pattern
    /// 2. Traversing relationships according to the pattern
    /// 3. Filtering results by WHERE clause conditions
    /// 4. Returning results according to RETURN clause
    ///
    /// # Arguments
    ///
    /// * `match_clause` - The parsed MATCH clause
    /// * `params` - Query parameters for resolving placeholders
    ///
    /// # Returns
    ///
    /// Vector of `MatchResult` containing matched nodes and their bindings.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed.
    /// Executes a MATCH query without guard-rail context (backward-compatible entry point).
    ///
    /// Direct entry point for the graph REST `/match` endpoint and the SDK
    /// bindings. Applies RETURN `ORDER BY` and the post-sort `LIMIT` so these
    /// surfaces match the SQL `/query` pipeline (which finalizes via
    /// `finalize_match_results`); without it the result would be raw traversal
    /// order with the ordering clause silently ignored.
    pub fn execute_match(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        let mut results = self.execute_match_with_context(match_clause, params, None)?;
        self.apply_match_order_by(&mut results, match_clause, params)?;
        if let Some(limit) = super::match_dispatch::match_return_limit(match_clause) {
            results.truncate(limit);
        }
        Ok(results)
    }

    /// Executes a MATCH query on this collection (EPIC-045 US-002, EPIC-048).
    ///
    /// Performs graph pattern matching: finds start nodes, traverses
    /// relationships, enforces guard-rail limits, filters by WHERE, and
    /// projects RETURN properties.
    ///
    /// Hoists both storage read guards once, in the decreed lock order —
    /// `vector_storage` (2) before `payload_storage` (3) — and passes them
    /// down via [`MatchStorageGuards`] so no callee re-acquires either lock.
    /// The `ConcurrentEdgeStore` manages its own internal shard locks — no
    /// outer lock is needed.
    ///
    /// Callers that already hold both guards (e.g. the aggregation runtime
    /// WHERE evaluation running a MATCH-in-WHERE predicate) must use
    /// [`Self::execute_match_with_guards`] instead of this entry point, which
    /// would re-acquire the locks under them.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed or a guard-rail is violated.
    pub fn execute_match_with_context(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: Option<&QueryContext>,
    ) -> Result<Vec<MatchResult>> {
        // Metric snapshot (config, rank 1) BEFORE the guards, then both
        // guards in decree order (2 then 3). See `MatchStorageGuards`.
        let metric = self.storage.config.read().metric;
        let vector_guard = self.storage.vector_storage.read();
        let payload_guard = self.storage.payload_storage.read();
        let guards = MatchStorageGuards {
            metric,
            vector_guard: &vector_guard,
            payload_guard: &payload_guard,
            payload_memo: PayloadMemo::new(&payload_guard),
        };
        self.execute_match_with_guards(match_clause, params, ctx, &guards)
    }

    /// Guard-accepting MATCH execution: the body of
    /// [`Self::execute_match_with_context`], for callers that already hold the
    /// storage guards in decree order and must not re-acquire them.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed or a guard-rail is violated.
    pub(in crate::collection::search::query) fn execute_match_with_guards(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: Option<&QueryContext>,
        guards: &MatchStorageGuards<'_>,
    ) -> Result<Vec<MatchResult>> {
        if match_clause.patterns.is_empty() {
            return Err(Error::Query(
                "MATCH query must have at least one pattern".to_string(),
            ));
        }

        // Documented contract (VELESQL_SPEC "Default LIMIT"): MATCH ... RETURN
        // has no implicit LIMIT 10 — results are bounded only by the
        // server-wide MAX_LIMIT ceiling shared with compound queries.
        let limit = traversal_limit(match_clause);
        let mut all_results: Vec<MatchResult> = Vec::new();
        let mut iteration_count: u32 = 0;
        let mut reported_cardinality: usize = 0;

        for pattern in &match_clause.patterns {
            if all_results.len() >= limit {
                break;
            }
            self.execute_single_pattern(
                pattern,
                match_clause,
                params,
                ctx,
                guards,
                &self.graph.edge_store,
                limit,
                &mut all_results,
                &mut iteration_count,
                &mut reported_cardinality,
            )?;
        }

        // Accumulate traversal counters into the query context for EXPLAIN
        // ANALYZE to read back. Runs on every GraphFirst MATCH (not only
        // ANALYZE), but it is one relaxed atomic-add per query — negligible, and
        // the per-edge hot loop is untouched. `nodes_visited` = start nodes
        // (added per pattern in execute_single_pattern) + the edge endpoints
        // reached here; `edges_traversed` = edges actually followed. Non-graph
        // queries never reach here.
        if let Some(qc) = ctx {
            let edges = u64::from(iteration_count);
            qc.add_traversal(edges, edges);
        }

        Ok(all_results)
    }

    /// Executes a single graph pattern: finds start nodes, then dispatches to
    /// single-node collection or BFS traversal.
    #[allow(clippy::too_many_arguments)]
    fn execute_single_pattern(
        &self,
        pattern: &GraphPattern,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: Option<&QueryContext>,
        guards: &MatchStorageGuards<'_>,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        limit: usize,
        all_results: &mut Vec<MatchResult>,
        iteration_count: &mut u32,
        reported_cardinality: &mut usize,
    ) -> Result<()> {
        let start_nodes = self.find_start_nodes(pattern, guards)?;
        if start_nodes.is_empty() {
            return Ok(());
        }
        // Count the start nodes this pattern examines toward nodes_visited.
        ctx.inspect(|qc| qc.add_traversal(start_nodes.len() as u64, 0));

        // S4-08: Compute index pre-filter once per pattern.
        let prefilter = match_clause
            .where_clause
            .as_ref()
            .and_then(|wc| index_prefilter::compute_index_prefilter(self, pattern, wc, params));

        let mut seen_pairs: std::collections::HashSet<(u64, u64)> =
            std::collections::HashSet::new();

        if pattern.relationships.is_empty() {
            let mut sn_ctx = SingleNodeCtx {
                match_clause,
                params,
                guards,
                seen_pairs: &mut seen_pairs,
                all_results,
                limit,
                prefilter,
            };
            return self.collect_single_node_results(&start_nodes, &mut sn_ctx);
        }

        let mut trav_ctx = TraversalCtx {
            match_clause,
            params,
            guards,
            guardrail: ctx,
            all_results,
            limit,
            iteration_count,
            reported_cardinality,
            seen_bindings: &mut HashSet::new(),
        };
        self.traverse_pattern(pattern, &start_nodes, edge_store, &mut trav_ctx)
    }

    /// Collects results for single-node patterns (no relationships).
    ///
    /// Uses the pre-acquired guards from the context to avoid per-node lock
    /// acquisitions.
    fn collect_single_node_results(
        &self,
        start_nodes: &[(u64, HashMap<String, u64>)],
        ctx: &mut SingleNodeCtx<'_>,
    ) -> Result<()> {
        for (node_id, bindings) in start_nodes {
            if ctx.all_results.len() >= ctx.limit {
                break;
            }
            // S4-08: Fast-reject via index pre-filter.
            if !index_prefilter::passes_prefilter(ctx.prefilter.as_ref(), *node_id) {
                continue;
            }
            if let Some(ref where_clause) = ctx.match_clause.where_clause {
                if !self.evaluate_where_condition(
                    *node_id,
                    Some(bindings),
                    where_eval::EdgeAliasBindings::NONE,
                    where_clause,
                    ctx.params,
                    ctx.guards,
                )? {
                    continue;
                }
            }
            if ctx.seen_pairs.contains(&(*node_id, *node_id)) {
                continue;
            }
            ctx.seen_pairs.insert((*node_id, *node_id));

            let mut result = MatchResult::new(*node_id, 0, Vec::new());
            result.bindings.clone_from(bindings);
            result.projected = self.project_properties(
                bindings,
                &HashMap::new(),
                &HashMap::new(),
                &ctx.match_clause.return_clause,
                &ctx.guards.payload_memo,
            );
            ctx.all_results.push(result);
        }
        Ok(())
    }

    /// Periodic guard-rail checks every 100 iterations (EPIC-048).
    #[allow(clippy::unused_self)]
    fn check_periodic_guardrails(
        &self,
        ctx: Option<&QueryContext>,
        iteration_count: u32,
        all_results: &[MatchResult],
        reported_cardinality: &mut usize,
    ) -> Result<()> {
        if !iteration_count.is_multiple_of(100) {
            return Ok(());
        }
        let Some(ctx) = ctx else { return Ok(()) };
        ctx.check_timeout()
            .map_err(|e| Error::GuardRail(e.to_string()))?;
        let new_results = all_results.len().saturating_sub(*reported_cardinality);
        if new_results > 0 {
            ctx.check_cardinality(new_results)
                .map_err(|e| Error::GuardRail(e.to_string()))?;
            *reported_cardinality = all_results.len();
        }
        Ok(())
    }
}

/// Computes the traversal-phase candidate cap for a MATCH clause.
///
/// Without a RETURN `ORDER BY`, the LIMIT applies to traversal order, so the
/// early-break at `return_clause.limit` is correct and stops traversal as soon
/// as enough rows are collected. WITH an `ORDER BY`, the post-sort LIMIT must
/// select the GLOBAL top-K, so traversal must visit the full candidate set
/// (bounded only by the shared `MAX_LIMIT` ceiling and the guard-rails) before
/// the sort — otherwise the LIMIT would be applied to the first-K-traversed
/// rows instead of the globally ordered set (backlog #1b).
fn traversal_limit(match_clause: &MatchClause) -> usize {
    if match_clause.return_clause.order_by.is_some() {
        return super::MAX_LIMIT;
    }
    match_clause
        .return_clause
        .limit
        .map_or(super::MAX_LIMIT, |l| l as usize)
}

// Tests moved to match_exec_tests.rs per project rules
