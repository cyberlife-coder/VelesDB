//! The query vectors of a statement, checked against the collection it
//! reads — once, before any plan is chosen or any row is read.
//!
//! A query vector a collection cannot score is a property of the statement,
//! not of a row: a metadata collection (or a graph without embeddings)
//! stores no vector to compare it with, and a vector of another dimension
//! compares with none of the collection's. Whether such a statement fails
//! must not depend on which plan the executor picks, on the operator or the
//! boolean position of the `similarity()` leaf, or on whether the walk found
//! any row — so [`QueryVectorTarget::check`] runs ahead of all of them, on
//! every engine: core's SELECT, aggregate and MATCH entry points, REST
//! `/match` with a vector, and the WASM executor.
//!
//! A *row* without a usable vector is another matter, and stays a row-level
//! UNKNOWN: a payload-only graph node, a point whose named vector is absent.

use std::collections::HashMap;
use std::hash::BuildHasher;

use super::{
    bind_vector, ArithmeticExpr, Condition, MatchClause, OrderByExpr, SelectStatement,
    VectorColumn, VectorExpr, PRIMARY_VECTOR_FIELD,
};
use crate::error::{Error, Result};

/// The vectors a collection stores, as a query vector against it must match
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryVectorTarget<'a> {
    /// The collection's name, which a refusal names.
    pub collection: &'a str,
    /// The dimension of its vectors; `None` when it stores none — a metadata
    /// collection, a graph without embeddings.
    pub dimension: Option<usize>,
}

impl<'a> QueryVectorTarget<'a> {
    /// The `vector` column a DML row of this collection has, the collection
    /// being a graph when `is_graph` ([`VectorColumn::new`]).
    #[must_use]
    pub const fn vector_column(&self, is_graph: bool) -> VectorColumn<'a> {
        VectorColumn::new(*self, is_graph)
    }

    /// Refuses a vector of `len` components *written* into the collection:
    /// it stores no vector ([`Error::VectorNotAllowed`], `VELES-014`), or
    /// stores vectors of another dimension ([`Error::DimensionMismatch`],
    /// `VELES-004`). The one rule every write surface applies — DML
    /// (`VectorColumn`) and the collection API alike — as [`Self::check_len`]
    /// is the one a query vector meets.
    ///
    /// # Errors
    ///
    /// As above.
    pub fn check_write(&self, len: usize) -> Result<()> {
        let dimension = self
            .dimension
            .ok_or_else(|| Error::VectorNotAllowed(self.collection.to_string()))?;
        crate::validation::validate_dimension_match(dimension, len)
    }

    /// [`Self::check_write`] for a vector every row of the collection stores
    /// — a vector collection's: none at all (`len` 0) is a row without one,
    /// refused with [`Error::VectorRequired`] (`VELES-016`), as a payload
    /// written for an id without a vector is; any other length is checked
    /// by [`Self::check_write`] (`VELES-004` for another dimension).
    ///
    /// # Errors
    ///
    /// As above.
    pub fn check_required_write(&self, len: usize) -> Result<()> {
        if len == 0 && self.dimension.is_some() {
            return Err(Error::VectorRequired(self.collection.to_string()));
        }
        self.check_write(len)
    }

    /// Refuses a query vector of `len` components: the collection stores no
    /// vector ([`Error::SearchNotSupported`], `VELES-015`), or stores vectors
    /// of another dimension ([`Error::DimensionMismatch`], `VELES-004`).
    ///
    /// # Errors
    ///
    /// As above.
    pub fn check_len(&self, len: usize) -> Result<()> {
        let dimension = self
            .dimension
            .ok_or_else(|| Error::SearchNotSupported(self.collection.to_string()))?;
        crate::validation::validate_dimension_match(dimension, len)
    }

    /// Binds and checks every query vector in `vectors`
    /// ([`Self::check_len`]). A collection that stores no vector refuses the
    /// first one before it is bound; none at all is accepted.
    ///
    /// # Errors
    ///
    /// [`Self::check_len`]'s refusal, or [`bind_vector`]'s
    /// [`Error::QueryParameter`] (`VELES-038`) for a `$name` that is unbound
    /// or not a vector.
    pub fn check<S: BuildHasher>(
        &self,
        vectors: &[&VectorExpr],
        params: &HashMap<String, serde_json::Value, S>,
    ) -> Result<()> {
        for vector in vectors {
            if self.dimension.is_none() {
                return self.check_len(0);
            }
            self.check_len(bind_vector(vector, params)?.len())?;
        }
        Ok(())
    }

    /// Checks the query vectors of a SELECT ([`select_query_vectors`]) —
    /// the check every engine runs before it reads a row of one.
    ///
    /// # Errors
    ///
    /// [`Self::check`]'s refusal.
    pub fn check_select<S: BuildHasher>(
        &self,
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value, S>,
    ) -> Result<()> {
        self.check(&select_query_vectors(stmt), params)
    }

    /// Checks the query vectors of a MATCH ([`match_query_vectors`]),
    /// after the scoring vector a caller hands over beside the clause (REST
    /// `/match`'s), of `scoring_len` components.
    ///
    /// # Errors
    ///
    /// [`Self::check_len`]'s refusal of the scoring vector, then
    /// [`Self::check`]'s.
    pub fn check_match<S: BuildHasher>(
        &self,
        clause: &MatchClause,
        params: &HashMap<String, serde_json::Value, S>,
        scoring_len: Option<usize>,
    ) -> Result<()> {
        if let Some(len) = scoring_len {
            self.check_len(len)?;
        }
        self.check(&match_query_vectors(clause), params)
    }
}

/// The query vectors a SELECT scores against the collection's own vectors:
/// every `NEAR`, every `NEAR_FUSED` vector and every `similarity(vector, …)`
/// of its WHERE — under `AND`, `OR` and `NOT` alike — and of its `ORDER BY`,
/// inside an arithmetic key included. A named-vector `similarity(field, …)`
/// scores the array a payload holds under `field`, row by row, and is not
/// among them.
#[must_use]
pub fn select_query_vectors(stmt: &SelectStatement) -> Vec<&VectorExpr> {
    let mut vectors = Vec::new();
    if let Some(condition) = &stmt.where_clause {
        collect_condition_vectors(condition, true, &mut vectors);
    }
    for item in stmt.order_by.iter().flatten() {
        collect_order_vectors(&item.expr, true, &mut vectors);
    }
    vectors
}

/// The query vectors a MATCH scores its nodes against: every `similarity()`
/// of its WHERE and of its `ORDER BY`. Each scores a node's vector, whatever
/// the column after the alias names.
#[must_use]
pub fn match_query_vectors(clause: &MatchClause) -> Vec<&VectorExpr> {
    let mut vectors = Vec::new();
    if let Some(condition) = &clause.where_clause {
        collect_condition_vectors(condition, false, &mut vectors);
    }
    for item in clause.return_clause.order_by.iter().flatten() {
        collect_order_vectors(&item.expr, false, &mut vectors);
    }
    vectors
}

/// Pushes the query vectors of an `ORDER BY` key: a `similarity(field, $v)`,
/// alone or inside an arithmetic expression; `primary_only` as in
/// [`collect_condition_vectors`].
fn collect_order_vectors<'q>(
    expr: &'q OrderByExpr,
    primary_only: bool,
    out: &mut Vec<&'q VectorExpr>,
) {
    match expr {
        OrderByExpr::Similarity(sim) if !primary_only || sim.field == PRIMARY_VECTOR_FIELD => {
            out.push(&sim.vector);
        }
        OrderByExpr::Arithmetic(arithmetic) => {
            collect_arithmetic_vectors(arithmetic, primary_only, out);
        }
        _ => {}
    }
}

fn collect_arithmetic_vectors<'q>(
    expr: &'q ArithmeticExpr,
    primary_only: bool,
    out: &mut Vec<&'q VectorExpr>,
) {
    match expr {
        ArithmeticExpr::Similarity(inner) => collect_order_vectors(inner, primary_only, out),
        ArithmeticExpr::BinaryOp { left, right, .. } => {
            collect_arithmetic_vectors(left, primary_only, out);
            collect_arithmetic_vectors(right, primary_only, out);
        }
        _ => {}
    }
}

/// Pushes the query vectors of `condition`'s leaves; `primary_only` keeps a
/// `similarity()` only when it names the point's own vector (SELECT).
fn collect_condition_vectors<'q>(
    condition: &'q Condition,
    primary_only: bool,
    out: &mut Vec<&'q VectorExpr>,
) {
    match condition {
        Condition::And(left, right) | Condition::Or(left, right) => {
            collect_condition_vectors(left, primary_only, out);
            collect_condition_vectors(right, primary_only, out);
        }
        Condition::Not(inner) | Condition::Group(inner) => {
            collect_condition_vectors(inner, primary_only, out);
        }
        Condition::VectorSearch(search) => out.push(&search.vector),
        Condition::VectorFusedSearch(fused) => out.extend(&fused.vectors),
        Condition::Similarity(sim) if !primary_only || sim.field == PRIMARY_VECTOR_FIELD => {
            out.push(&sim.vector);
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "query_vectors_tests.rs"]
mod tests;
