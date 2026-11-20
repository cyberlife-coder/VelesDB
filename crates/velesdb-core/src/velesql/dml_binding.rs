//! The row an `INSERT`/`UPSERT` writes and the `SET` list an `UPDATE`
//! applies, bound to the caller's parameters.
//!
//! One rule set for every engine: core's `Database` and the WASM executor
//! bind a DML statement here, so the cells they accept, the point they
//! write and the words they refuse with cannot drift. It lives outside
//! `database` so that a build without `persistence` (WASM) binds exactly as
//! the server does. What each engine keeps is its own storage: the write
//! itself, and the checks its store runs on the vector it writes.
//!
//! Whether a row may, must or cannot carry a `vector` is the collection's
//! [`VectorColumn`]: a vector collection's rows all store one, a metadata
//! collection's none, and a graph's nodes may or may not carry an embedding
//! — none at all in a graph without embeddings. A `vector` a statement binds
//! is a vector, of the collection's dimension: on a graph only a row without
//! the column is a row without a vector, so an empty one is refused
//! (`VELES-004`); on a vector collection a row without the column, or with
//! an empty one, is a row without a vector, refused as the collection API
//! refuses it (`VELES-016`, [`QueryVectorTarget::check_required_write`]). A
//! vector written where none is stored is refused as the collection API
//! refuses it (`VELES-014`, [`QueryVectorTarget::check_write`]).

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::error::{Error, Result};
use crate::point::Point;
use crate::velesql::{
    bind_dml_value, bind_dml_vector, Condition, QueryVectorTarget, UpdateAssignment, Value,
};

/// What a DML row's `vector` column is to the collection it writes into.
///
/// Derived from what the collection stores, the dimension its query vectors
/// must match ([`QueryVectorTarget::vector_column`]), so "does this
/// collection store vectors" is answered in one place — and a vector a row
/// writes is checked by the rule every write surface applies
/// ([`QueryVectorTarget::check_write`]).
///
/// A vector collection's rows all store one; a graph's nodes may carry an
/// embedding or not (the node `INSERT NODE` writes); a metadata collection
/// and a graph without embeddings store none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorColumn<'a> {
    target: QueryVectorTarget<'a>,
    is_graph: bool,
}

impl<'a> VectorColumn<'a> {
    /// The column of a collection that stores `target`'s vectors and is a
    /// graph when `is_graph`.
    #[must_use]
    pub const fn new(target: QueryVectorTarget<'a>, is_graph: bool) -> Self {
        Self { target, is_graph }
    }

    /// Whether every row must carry a vector: a vector collection's.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        !self.is_graph && self.target.dimension.is_some()
    }

    /// Whether no row may carry one: the collection stores no vector.
    #[must_use]
    pub const fn is_forbidden(&self) -> bool {
        self.target.dimension.is_none()
    }

    /// Refuses a vector of `len` components a row writes: by
    /// [`QueryVectorTarget::check_required_write`] where every row stores
    /// one (an empty one is no vector, `VELES-016`), else by
    /// [`QueryVectorTarget::check_write`].
    ///
    /// # Errors
    ///
    /// As above.
    pub fn check_write(&self, len: usize) -> Result<()> {
        if self.is_required() {
            self.target.check_required_write(len)
        } else {
            self.target.check_write(len)
        }
    }
}

/// One `INSERT`/`UPSERT` row bound to the caller's parameters
/// ([`bind_insert_row`]); [`into_point`](Self::into_point) is the point it
/// writes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundInsertRow {
    id: Option<u64>,
    vector: Option<Vec<f32>>,
    payload: serde_json::Map<String, serde_json::Value>,
}

impl BoundInsertRow {
    /// The point this row writes into a collection whose `vector` column
    /// is `column`: a row without a vector is a point with an empty one,
    /// which the collection stores as a payload (a metadata row, a graph node
    /// without an embedding).
    ///
    /// # Errors
    ///
    /// [`Error::Query`] (`VELES-010`) for a row without an `id`;
    /// [`Error::VectorRequired`] (`VELES-016`) for a row without a vector, or
    /// with an empty one, on a vector collection; [`Error::VectorNotAllowed`]
    /// (`VELES-014`) for a vector on a collection that stores none and
    /// [`Error::DimensionMismatch`] (`VELES-004`) for one of another
    /// dimension than the collection's — an empty one included, on a graph
    /// ([`VectorColumn::check_write`]).
    pub fn into_point(self, column: VectorColumn<'_>) -> Result<Point> {
        let id = self
            .id
            .ok_or_else(|| Error::Query("INSERT requires integer 'id' column".to_string()))?;
        let payload = serde_json::Value::Object(self.payload);
        match self.vector {
            None if !column.is_required() => Ok(Point::metadata_only(id, payload)),
            // Where every row stores a vector, a row without the column is
            // one without a vector: the empty one the rule refuses.
            vector => {
                let vector = vector.unwrap_or_default();
                column.check_write(vector.len())?;
                Ok(Point::new(id, vector, Some(payload)))
            }
        }
    }
}

/// Binds one `VALUES` row against its `columns`: `id` is the point id,
/// `vector` its vector (bound as every vector `$name` is, through
/// [`super::bind_vector`]), every other column a payload field (a `$name`
/// as the caller's JSON value whole). Cells bind left to right; the first
/// refusal wins. A row holds one cell per column: the parser refuses any
/// other statement, and so does this binder, for a statement built another
/// way — zipping them would drop the unmatched cells, a `vector` among them.
///
/// # Errors
///
/// [`Error::Query`] (`VELES-010`) for a row whose cells do not match its
/// columns, or an `id` that is not a non-negative integer; the binders'
/// errors.
pub fn bind_insert_row<S: BuildHasher>(
    columns: &[String],
    row: &[Value],
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<BoundInsertRow> {
    if row.len() != columns.len() {
        return Err(Error::Query(format!(
            "INSERT row has {} values but {} columns were declared",
            row.len(),
            columns.len()
        )));
    }
    let mut bound = BoundInsertRow {
        id: None,
        vector: None,
        payload: serde_json::Map::new(),
    };
    for (column, value) in columns.iter().zip(row) {
        bound.bind_cell(column, value, params)?;
    }
    Ok(bound)
}

impl BoundInsertRow {
    /// Binds the cell `value` of `column` into this row: `id`, `vector` or a
    /// payload field.
    fn bind_cell<S: BuildHasher>(
        &mut self,
        column: &str,
        value: &Value,
        params: &HashMap<String, serde_json::Value, S>,
    ) -> Result<()> {
        match column {
            "vector" => self.vector = Some(bind_dml_vector(value, params)?),
            "id" => self.id = Some(point_id(&bind_dml_value(value, params)?)?),
            _ => {
                self.payload
                    .insert(column.to_string(), bind_dml_value(value, params)?);
            }
        }
        Ok(())
    }
}

/// An `UPDATE`'s `SET` list bound to the caller's parameters
/// ([`bind_update_assignments`]): the vector it assigns, if any, and its
/// payload fields in statement order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoundAssignments {
    vector: Option<Vec<f32>>,
    fields: Vec<(String, serde_json::Value)>,
}

impl BoundAssignments {
    /// `point` with these assignments applied: its payload fields set over
    /// the stored ones, and its vector replaced when one is assigned — on a
    /// graph node without an embedding that attaches one.
    ///
    /// # Errors
    ///
    /// [`VectorColumn::check_write`]'s refusal of an assigned vector:
    /// [`Error::VectorNotAllowed`] (`VELES-014`) on a collection that stores
    /// none, [`Error::VectorRequired`] (`VELES-016`) for an empty one on a
    /// vector collection, [`Error::DimensionMismatch`] (`VELES-004`) for one
    /// of another dimension than the collection's — an empty one included,
    /// on a graph.
    pub fn apply(&self, point: &Point, column: VectorColumn<'_>) -> Result<Point> {
        let mut payload = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (field, value) in &self.fields {
            payload.insert(field.clone(), value.clone());
        }
        let payload = serde_json::Value::Object(payload);
        let vector = match &self.vector {
            Some(assigned) => {
                column.check_write(assigned.len())?;
                assigned.clone()
            }
            None if column.is_forbidden() => Vec::new(),
            None => point.vector.clone(),
        };
        Ok(Point::new(point.id, vector, Some(payload)))
    }
}

/// Binds an `UPDATE`'s `SET` list before any row is read: `vector` binds as
/// an `INSERT`'s does ([`bind_insert_row`]), every other column is a payload
/// field.
///
/// # Errors
///
/// The binders' errors, and [`Error::Query`] for an assignment to `id`.
pub fn bind_update_assignments<S: BuildHasher>(
    assignments: &[UpdateAssignment],
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<BoundAssignments> {
    let mut bound = BoundAssignments::default();
    for assignment in assignments {
        match assignment.column.as_str() {
            "id" => {
                return Err(Error::Query(
                    "UPDATE cannot modify primary key column 'id'".to_string(),
                ))
            }
            "vector" => bound.vector = Some(bind_dml_vector(&assignment.value, params)?),
            column => bound.fields.push((
                column.to_string(),
                bind_dml_value(&assignment.value, params)?,
            )),
        }
    }
    Ok(bound)
}

/// Refuses a DML WHERE holding a predicate no metadata filter carries — a
/// vector or graph predicate, or a full-text `MATCH`: a DML statement
/// filters on metadata, and the filter conversion drops such a predicate,
/// which under an `AND` would widen the rows the statement touches instead
/// of narrowing them. Every engine calls it before any parameter is bound —
/// a statement is refused for its shape first, as a SELECT is validated
/// before it executes — so such a WHERE answers `VELES-010` even when one of
/// its `$name`s is unbound.
///
/// # Errors
///
/// [`Error::Query`] (`VELES-010`) naming the `statement`.
pub fn check_dml_where(statement: &str, where_clause: Option<&Condition>) -> Result<()> {
    if where_clause.is_some_and(holds_non_metadata_predicate) {
        return Err(Error::Query(format!(
            "{statement} WHERE supports metadata predicates only (no similarity/NEAR/MATCH)"
        )));
    }
    Ok(())
}

/// Whether `condition` holds a predicate no metadata filter carries (see
/// [`check_dml_where`]).
fn holds_non_metadata_predicate(condition: &Condition) -> bool {
    match condition {
        Condition::Match(_)
        | Condition::Similarity(_)
        | Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_)
        | Condition::GraphMatch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            holds_non_metadata_predicate(left) || holds_non_metadata_predicate(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => holds_non_metadata_predicate(inner),
        _ => false,
    }
}

/// A bound `id` cell as a point id: a non-negative integer.
fn point_id(value: &serde_json::Value) -> Result<u64> {
    if let Some(id) = value.as_u64() {
        return Ok(id);
    }
    value
        .as_i64()
        .ok_or_else(|| Error::Query("id must be an integer".to_string()))
        .and_then(|v| u64::try_from(v).map_err(|_| Error::Query("id must be >= 0".to_string())))
}

#[cfg(test)]
#[path = "dml_binding_tests.rs"]
mod tests;
