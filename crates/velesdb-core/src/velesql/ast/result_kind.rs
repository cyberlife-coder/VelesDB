//! The shape of result a parsed statement produces.
//!
//! Every binding that reports "what kind of statement ran" (mobile's and
//! wasm's `QueryResult::kind`) maps from [`Query::result_kind`] instead of
//! re-deriving it from the AST, so the rule lives here once.

use super::{DmlStatement, Query};

/// What a statement's result rows mean, read from its AST alone.
///
/// Deliberately exhaustive: a binding that reports the kind matches on it, and
/// a new kind must fail to compile there rather than fall into a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryResultKind {
    /// Row-returning statement: SELECT, MATCH, SELECT EDGES, SHOW, DESCRIBE,
    /// EXPLAIN.
    Rows,
    /// Row-affecting DML: INSERT, UPSERT, UPDATE, INSERT EDGE, INSERT NODE.
    Mutation,
    /// DELETE and DELETE EDGE.
    Deletion,
    /// DDL: CREATE, DROP, ALTER, TRUNCATE, ANALYZE.
    Ddl,
    /// TRAIN QUANTIZER.
    Train,
    /// Admin command: FLUSH.
    Admin,
}

impl Query {
    /// Classifies this statement by the result it produces.
    ///
    /// The statement slots are checked in priority order TRAIN > DDL > admin >
    /// DML; a query with none of them set (SELECT, MATCH, introspection)
    /// returns rows.
    #[must_use]
    pub fn result_kind(&self) -> QueryResultKind {
        if self.is_train() {
            QueryResultKind::Train
        } else if self.is_ddl_query() {
            QueryResultKind::Ddl
        } else if self.is_admin_query() {
            QueryResultKind::Admin
        } else if let Some(dml) = &self.dml {
            dml_result_kind(dml)
        } else {
            QueryResultKind::Rows
        }
    }
}

/// Exhaustive over [`DmlStatement`], so a new DML variant must be classified.
fn dml_result_kind(dml: &DmlStatement) -> QueryResultKind {
    match dml {
        DmlStatement::Delete(_) | DmlStatement::DeleteEdge(_) => QueryResultKind::Deletion,
        DmlStatement::SelectEdges(_) => QueryResultKind::Rows,
        DmlStatement::Insert(_)
        | DmlStatement::Upsert(_)
        | DmlStatement::Update(_)
        | DmlStatement::InsertEdge(_)
        | DmlStatement::InsertNode(_) => QueryResultKind::Mutation,
    }
}

#[cfg(test)]
#[path = "result_kind_tests.rs"]
mod tests;
