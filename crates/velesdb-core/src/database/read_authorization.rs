//! The read gate's one choke point for a `VelesQL` statement.
//!
//! A statement can read more than the collection its `FROM` names: each
//! operand of a set operation, a JOIN's secondary collection, the collection a
//! `MATCH` or an aggregate reaches through the `_collection` parameter, the
//! `@collection` a `MATCH` node is enriched from, the graph `SELECT EDGES`
//! lists. [`Database::authorize_statement`] asks the observer about each of
//! them and folds what it answers into the statement. The executors that read
//! on a statement's behalf take the [`AuthorizedQuery`] it returns, and that
//! method is the type's only constructor, so a read that skips the gate does
//! not compile.
//!
//! Each decision is enforced where it can be:
//!
//! * `Deny(err)` aborts the statement with exactly `err`;
//! * a scope filter on a collection the statement filters row by row — the
//!   collection it reads first, and each set-operation operand — is
//!   AND-composed into that select's own `WHERE`, so it can only narrow;
//! * a scope filter on a read that has no row filter to compose into (a graph
//!   traversal, a JOIN's secondary collection, `@collection` enrichment,
//!   `SELECT EDGES`) refuses the statement with [`Error::Config`] rather than
//!   run it unscoped — what the server's graph endpoints and the Python
//!   binding already answer.
//!
//! Scalar subqueries are not decided here: they are resolved before the
//! statement reaches the gate, each through [`Database::execute_query`] or
//! [`Database::execute_aggregate`], so each one is decided by its own
//! collection's answer.
//!
//! However a statement reaches a collection, the observer is consulted about
//! each `(collection, operation)` **once per statement** (issue #2588): a
//! self-JOIN, a set operation over one collection, a `MATCH` naming one
//! `@collection` twice, a scalar subquery on the outer collection, or the
//! plan read an `EXPLAIN`/`EXPLAIN ANALYZE` makes ahead of the row read all
//! replay the first answer through [`GateDecisions`], so a logging or
//! metering observer never sees the same collection decided twice.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::observer::{AccessScope, QueryOperationKind};
use crate::velesql::{
    CompoundQuery, Condition, DmlStatement, IntrospectionStatement, Query, SelectStatement,
};
use crate::{Error, Result};

use super::query_engine::{derive_operation_kind, derive_select_operation_kind, RawGateOutcome};
use super::Database;

/// The parameters a statement executes with.
type Params = HashMap<String, serde_json::Value>;

/// A statement the read gate has admitted, every scope the observer returned
/// already folded into it, together with the collection it reads first.
///
/// Built only by [`Database::authorize_statement`]. The executors that read
/// collection data for a statement take this type rather than a [`Query`],
/// and they resolve the collection to read from [`target`](Self::target), the
/// name the gate decided on — never by deriving it again.
pub(super) struct AuthorizedQuery<'q> {
    query: Cow<'q, Query>,
    target: Option<&'q str>,
}

impl<'q> AuthorizedQuery<'q> {
    /// The statement to execute, narrowed by every scope the gate applied.
    pub(super) fn query(&self) -> &Query {
        &self.query
    }

    /// The collection the statement reads first (see [`read_target`]).
    pub(super) fn target(&self) -> Option<&'q str> {
        self.target
    }

    /// The statement as the gate handed it back, borrowed when nothing was
    /// narrowed — what the zero-overhead tests inspect.
    #[cfg(test)]
    pub(super) fn into_query(self) -> Cow<'q, Query> {
        self.query
    }
}

/// The collection a statement reads first: its `FROM`, else the `_collection`
/// parameter (how the REPL, the SDKs and the server's `/query` address a
/// `MATCH` or an aggregate), else none — and a statement with none is refused
/// by its executor before anything is read
/// ([`Database::resolve_read_target`]).
pub(super) fn read_target<'q>(query: &'q Query, params: &'q Params) -> Option<&'q str> {
    if !query.select.from.is_empty() {
        return Some(&query.select.from);
    }
    match params.get("_collection") {
        Some(serde_json::Value::String(name)) => Some(name),
        _ => None,
    }
}

/// AND-composes `filter` into a `WHERE` slot. The existing predicate stays the
/// left operand and is never rewritten, so the composition can only narrow.
pub(super) fn narrow_where(slot: &mut Option<Condition>, filter: Condition) {
    *slot = Some(match slot.take() {
        Some(existing) => Condition::And(Box::new(existing), Box::new(filter)),
        None => filter,
    });
}

/// The read gate's per-statement decision memo (issue #2588).
///
/// One statement can reach the same `(collection, operation)` consultation
/// more than once — see the module documentation. The observer is asked only
/// the first time; later consultations replay the recorded answer, so the
/// statement sees one policy snapshot and a logging/metering observer sees
/// each collection decided once. Created at each statement entry point
/// (`execute_query`, `execute_aggregate`, `explain_query` /
/// `explain_query_with_params`, `explain_analyze_query`) and threaded through
/// the gate; a fresh empty memo also serves call sites that consult once per
/// invocation.
///
/// Only `Allow` and `Scope` outcomes are recorded: a `Deny` or an observer
/// failure aborts the statement on first sight, so nothing downstream could
/// consult the same collection again.
#[derive(Debug, Default)]
pub(super) struct GateDecisions {
    /// Linear memo: a statement reads a handful of collections, so a scan
    /// beats a `HashMap` (and spares `QueryOperationKind` a public `Hash`).
    decided: std::cell::RefCell<Vec<(String, QueryOperationKind, StoredDecision)>>,
}

/// A replayable gate answer. `Deny` is absent by construction — see
/// [`GateDecisions`].
#[derive(Debug, Clone)]
enum StoredDecision {
    /// The read was allowed unmodified.
    Allow,
    /// The read was allowed narrowed by this scope.
    Scope(AccessScope),
}

impl StoredDecision {
    /// The recorded answer as a fresh [`RawGateOutcome`].
    fn replay(&self) -> RawGateOutcome {
        match self {
            Self::Allow => RawGateOutcome::Allow,
            Self::Scope(scope) => RawGateOutcome::Scope(scope.clone()),
        }
    }
}

impl GateDecisions {
    /// The recorded answer for `(collection, operation)`, when this statement
    /// already decided it.
    fn find(&self, collection: &str, operation: QueryOperationKind) -> Option<StoredDecision> {
        self.decided
            .borrow()
            .iter()
            .find(|(name, op, _)| name == collection && *op == operation)
            .map(|(_, _, decision)| decision.clone())
    }

    /// Records an `Allow`/`Scope` outcome; a `Deny` aborts the statement and
    /// is never replayed, so it is not stored.
    fn record(&self, collection: &str, operation: QueryOperationKind, outcome: &RawGateOutcome) {
        let stored = match outcome {
            RawGateOutcome::Allow => StoredDecision::Allow,
            RawGateOutcome::Scope(scope) => StoredDecision::Scope(scope.clone()),
            RawGateOutcome::Deny(_) => return,
        };
        self.decided
            .borrow_mut()
            .push((collection.to_string(), operation, stored));
    }
}

/// Refuses a scope whose columns are qualified with one of `select`'s JOIN
/// tables or aliases (fail closed, `VELES-009`).
///
/// A scope is a predicate on the *scoped* collection's own rows, so its
/// column references must be unqualified payload paths (or qualify the
/// collection itself). Once composed into the WHERE of a select that JOINs,
/// pushdown classifies a `join_table.col` conjunct as belonging to the join
/// side ([`analyze_for_pushdown`]) and applies it to that store — where a
/// `LEFT JOIN` keeps every base row anyway, so the base would be returned
/// unscoped. Refusing is the only answer that cannot leak.
///
/// [`analyze_for_pushdown`]: crate::collection::search::query::pushdown::analyze_for_pushdown
fn require_join_free_scope(select: &SelectStatement, filter: &Condition) -> Result<()> {
    if select.joins.is_empty() {
        return Ok(());
    }
    let join_tables =
        crate::collection::search::query::pushdown::extract_join_tables(&select.joins);
    if let Some(column) = join_qualified_column(filter, &join_tables) {
        return Err(Error::Config(format!(
            "observer scope column '{column}' names a JOIN table; a scope \
             must predicate on the scoped collection's own rows (unqualified \
             payload paths) — refusing to run unscoped"
        )));
    }
    Ok(())
}

/// The first column reference in `filter` qualified with a name in
/// `join_tables` (`table.column`), or `None` when none qualifies.
fn join_qualified_column<'c>(
    filter: &'c Condition,
    join_tables: &std::collections::HashSet<String>,
) -> Option<&'c str> {
    let column = scope_leaf_column(filter);
    if let Some(c) = column {
        return c
            .split_once('.')
            .filter(|(table, _)| join_tables.contains(*table))
            .map(|_| c);
    }
    match filter {
        Condition::And(l, r) | Condition::Or(l, r) => {
            join_qualified_column(l, join_tables).or_else(|| join_qualified_column(r, join_tables))
        }
        Condition::Not(inner) | Condition::Group(inner) => {
            join_qualified_column(inner, join_tables)
        }
        _ => None,
    }
}

/// The column a column-bearing leaf condition references, else `None`.
fn scope_leaf_column(condition: &Condition) -> Option<&str> {
    match condition {
        Condition::Comparison(c) => Some(&c.column),
        Condition::In(c) => Some(&c.column),
        Condition::Between(c) => Some(&c.column),
        Condition::Like(c) => Some(&c.column),
        Condition::IsNull(c) => Some(&c.column),
        Condition::Match(c) => Some(&c.column),
        Condition::Contains(c) => Some(&c.column),
        Condition::ContainsText(c) => Some(&c.column),
        Condition::GeoDistance(c) => Some(&c.column),
        Condition::GeoBbox(c) => Some(&c.column),
        _ => None,
    }
}

impl Database {
    /// Puts a statement through the read gate: asks the observer about every
    /// collection the statement reads and enforces each answer (see the
    /// module documentation), returning the statement to execute.
    ///
    /// With no observer registered this is a single `Option` check and the
    /// statement comes back borrowed. Statements that read nothing (DDL, the
    /// mutating DML, admin, TRAIN, introspection) carry their own gates and
    /// come back borrowed too.
    ///
    /// # Errors
    ///
    /// The observer's `Deny` error, its own failure, or [`Error::Config`] for
    /// a scope the read it narrows has no way to apply.
    pub(super) fn authorize_statement<'q>(
        &self,
        query: &'q Query,
        params: &'q Params,
        decisions: &GateDecisions,
    ) -> Result<AuthorizedQuery<'q>> {
        let target = read_target(query, params);
        let admitted = |query| AuthorizedQuery { query, target };
        if self.observer.is_none() {
            return Ok(admitted(Cow::Borrowed(query))); // zero-overhead fast path
        }
        if let Some(DmlStatement::SelectEdges(stmt)) = &query.dml {
            let graph = QueryOperationKind::GraphTraversal;
            self.require_unscoped(&stmt.collection, graph, "SELECT EDGES", decisions)?;
        }
        if let Some(intro) = &query.introspection {
            self.authorize_introspection(intro, params, decisions)?;
        }
        if !Self::is_read_path(query) {
            return Ok(admitted(Cow::Borrowed(query)));
        }
        if query.is_match_query() {
            self.authorize_match(query, target, decisions)?;
            return Ok(admitted(Cow::Borrowed(query)));
        }
        self.authorize_select(query, target, decisions)
            .map(admitted)
    }

    /// A `MATCH` traverses its target and reads every `@collection` its nodes
    /// name; none of these reads has a row filter a scope could join, so each
    /// must be allowed unscoped.
    fn authorize_match(
        &self,
        query: &Query,
        target: Option<&str>,
        decisions: &GateDecisions,
    ) -> Result<()> {
        let graph = QueryOperationKind::GraphTraversal;
        if let Some(name) = target {
            self.require_unscoped(name, graph, "MATCH queries", decisions)?;
        }
        let annotated = query
            .match_clause
            .iter()
            .flat_map(|clause| clause.patterns.iter())
            .flat_map(|pattern| pattern.nodes.iter())
            .filter_map(|node| node.collection.as_deref());
        for name in annotated {
            self.require_unscoped(name, graph, "MATCH @collection enrichment", decisions)?;
        }
        Ok(())
    }

    /// A `DESCRIBE COLLECTION` reads a collection's metadata — its
    /// `point_count` and type — and a plan-only `EXPLAIN` reads its
    /// statistics. Both are reads the gate decides like any other: a denial
    /// refuses them, and a scope, with no row to narrow, refuses them too
    /// rather than leak the unscoped count. `SHOW COLLECTIONS` names the
    /// collections only — no per-collection data — so it is not consulted.
    /// Threat model: it does leak name *existence* (a caller learns a
    /// collection exists even when its reads are denied) — the accepted leak
    /// every `list`/`SHOW` surface shares; a policy that must hide existence
    /// denies at the transport, not at this gate.
    fn authorize_introspection(
        &self,
        stmt: &IntrospectionStatement,
        params: &Params,
        decisions: &GateDecisions,
    ) -> Result<()> {
        match stmt {
            IntrospectionStatement::DescribeCollection(desc) => self.require_unscoped(
                &desc.name,
                QueryOperationKind::Select,
                "DESCRIBE COLLECTION",
                decisions,
            ),
            IntrospectionStatement::Explain(inner) => {
                self.authorize_unscoped_statement(inner, params, decisions)
            }
            IntrospectionStatement::ShowCollections => Ok(()),
        }
    }

    /// Decides every collection `query` reads with no scope channel — a
    /// plan-only `EXPLAIN`'s calibrated statistics cannot be row-narrowed, so
    /// a scope refuses the statement rather than leak them. Subqueries the
    /// statement carries are irrelevant here: the plan never executes them.
    ///
    /// `pub(super)` so the sibling `query_engine` module can route the public
    /// [`Database::explain_query`] facade through the same decision —
    /// gating here means every surface (REST, CLI, Python, the introspection
    /// executor) inherits it instead of each remembering its own.
    pub(super) fn authorize_unscoped_statement(
        &self,
        query: &Query,
        params: &Params,
        decisions: &GateDecisions,
    ) -> Result<()> {
        if let Some(DmlStatement::SelectEdges(stmt)) = &query.dml {
            let graph = QueryOperationKind::GraphTraversal;
            self.require_unscoped(&stmt.collection, graph, "SELECT EDGES", decisions)?;
        }
        if !Self::is_read_path(query) {
            return Ok(());
        }
        let target = read_target(query, params);
        if query.is_match_query() {
            return self.authorize_match(query, target, decisions);
        }
        if let Some(name) = target {
            self.require_unscoped(
                name,
                derive_operation_kind(query),
                "a plan-only EXPLAIN",
                decisions,
            )?;
        }
        self.authorize_joins(&query.select, decisions)?;
        if let Some(compound) = query.compound.as_ref() {
            for (_, operand) in &compound.operations {
                self.authorize_joins(operand, decisions)?;
                let operation = derive_select_operation_kind(operand);
                self.require_unscoped(
                    &operand.from,
                    operation,
                    "a set-operation operand",
                    decisions,
                )?;
            }
        }
        Ok(())
    }

    /// A SELECT reads its target, each set-operation operand, and the
    /// secondary collection of every JOIN. The scope of the target and of each
    /// operand joins that select's `WHERE`; a JOIN's secondary side must be
    /// allowed unscoped. The statement is cloned only when a scope narrows it.
    fn authorize_select<'q>(
        &self,
        query: &'q Query,
        target: Option<&str>,
        decisions: &GateDecisions,
    ) -> Result<Cow<'q, Query>> {
        let mut scoped: Option<Query> = None;
        if let Some(name) = target {
            if let Some(filter) =
                self.scope_filter(name, derive_operation_kind(query), decisions)?
            {
                require_join_free_scope(&query.select, &filter)?;
                let owned = scoped.get_or_insert_with(|| query.clone());
                narrow_where(&mut owned.select.where_clause, filter);
            }
        }
        self.authorize_joins(&query.select, decisions)?;
        self.authorize_operands(query, &mut scoped, decisions)?;
        Ok(scoped.map_or(Cow::Borrowed(query), Cow::Owned))
    }

    /// Decides each set-operation operand on its own collection, and folds
    /// every scope into that operand's `WHERE`.
    fn authorize_operands(
        &self,
        query: &Query,
        scoped: &mut Option<Query>,
        decisions: &GateDecisions,
    ) -> Result<()> {
        let Some(compound) = query.compound.as_ref() else {
            return Ok(());
        };
        let filters = self.operand_filters(compound, decisions)?;
        if filters.iter().all(Option::is_none) {
            return Ok(());
        }
        let owned = scoped.get_or_insert_with(|| query.clone());
        // `owned` is `query` or a clone of it whose compound part no step
        // above touched, so its operands pair one to one with `filters`.
        let operands = owned
            .compound
            .iter_mut()
            .flat_map(|c| c.operations.iter_mut());
        for ((_, operand), filter) in operands.zip(filters) {
            if let Some(filter) = filter {
                narrow_where(&mut operand.where_clause, filter);
            }
        }
        Ok(())
    }

    /// The observer's answer for each set-operation operand, in order, once
    /// each operand's JOINs are allowed.
    fn operand_filters(
        &self,
        compound: &CompoundQuery,
        decisions: &GateDecisions,
    ) -> Result<Vec<Option<Condition>>> {
        let mut filters = Vec::with_capacity(compound.operations.len());
        for (_, operand) in &compound.operations {
            self.authorize_joins(operand, decisions)?;
            let operation = derive_select_operation_kind(operand);
            let filter = self.scope_filter(&operand.from, operation, decisions)?;
            if let Some(filter) = &filter {
                require_join_free_scope(operand, filter)?;
            }
            filters.push(filter);
        }
        Ok(filters)
    }

    /// A JOIN merges its secondary collection's payloads into each row, and
    /// none of its strategies filters that side, so the secondary collection
    /// must be allowed unscoped.
    fn authorize_joins(&self, select: &SelectStatement, decisions: &GateDecisions) -> Result<()> {
        for join in &select.joins {
            let select_read = QueryOperationKind::Select;
            self.require_unscoped(
                &join.table,
                select_read,
                "a JOIN's secondary collection",
                decisions,
            )?;
        }
        Ok(())
    }

    /// [`read_gate_raw`](Self::read_gate_raw) behind the statement memo: the
    /// observer is consulted only when this statement has not already decided
    /// `(collection, operation)`; a stored answer is replayed otherwise
    /// (issue #2588). The statement gate never carries principal or tenant
    /// hints, so the two-field memo key is complete.
    fn gated_read(
        &self,
        collection: &str,
        operation: QueryOperationKind,
        decisions: &GateDecisions,
    ) -> Result<RawGateOutcome> {
        if let Some(stored) = decisions.find(collection, operation) {
            return Ok(stored.replay());
        }
        let outcome = self.read_gate_raw(collection, operation, None, None)?;
        decisions.record(collection, operation, &outcome);
        Ok(outcome)
    }

    /// The observer's answer for one read: `Ok(None)` to read unmodified,
    /// `Ok(Some(filter))` to read narrowed by `filter`, `Err` when denied.
    fn scope_filter(
        &self,
        collection: &str,
        operation: QueryOperationKind,
        decisions: &GateDecisions,
    ) -> Result<Option<Condition>> {
        match self.gated_read(collection, operation, decisions)? {
            RawGateOutcome::Allow => Ok(None),
            RawGateOutcome::Deny(err) => Err(err),
            RawGateOutcome::Scope(scope) => Ok(scope.filter),
        }
    }

    /// The observer's answer for a read that cannot apply a scope filter: a
    /// filter refuses the statement instead of letting it run unscoped.
    ///
    /// `pub(super)` so sibling `database` modules gate their public metadata
    /// reads (`analyze_collection`, `collection_diagnostics`) with the same
    /// fail-closed answer the statement gate gives.
    pub(super) fn require_unscoped(
        &self,
        collection: &str,
        operation: QueryOperationKind,
        what: &str,
        decisions: &GateDecisions,
    ) -> Result<()> {
        match self.scope_filter(collection, operation, decisions)? {
            None => Ok(()),
            Some(_) => Err(Error::Config(format!(
                "scope narrowing is not supported for {what}"
            ))),
        }
    }

    /// Resolves the collection an admitted statement reads first, refusing a
    /// statement that names none before anything is read.
    pub(super) fn resolve_read_target(
        &self,
        authorized: &AuthorizedQuery<'_>,
        missing: &str,
    ) -> Result<crate::collection::Collection> {
        let name = authorized
            .target()
            .ok_or_else(|| Error::Query(missing.to_string()))?;
        self.resolve_collection(name)
    }
}
