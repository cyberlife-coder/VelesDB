//! Binding of `$name` parameters to the caller's values.
//!
//! The one place a VelesQL `$name` becomes a value. Every engine binds
//! through it before a condition reaches a payload filter — the server's
//! SELECT, MATCH and HAVING pipelines, and the WASM executor — so a missing
//! or unsupported parameter is the same error everywhere, never a silent
//! `NULL`. It lives outside `collection` so that a build without
//! `persistence` (WASM) binds exactly as the server does.

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::error::{Error, Result};
use crate::velesql::{
    BetweenCondition, Comparison, Condition, ContainsCondition, InCondition, MatchClause, Value,
    VectorExpr,
};

/// Binds `value` when it is a `$name` parameter; returns any other value
/// unchanged.
///
/// A JSON number binds as the narrowest of `i64`, `u64` and `f64` that
/// holds it, a string, boolean or `null` as itself.
///
/// # Errors
///
/// Returns [`Error::QueryParameter`] when the parameter is missing from
/// `params`, or bound to an array or object, which no scalar operand takes.
pub fn bind_value<S: BuildHasher>(
    value: &Value,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Value> {
    let Value::Parameter(name) = value else {
        return Ok(value.clone());
    };
    let bound = params
        .get(name)
        .ok_or_else(|| Error::unbound_parameter(name))?;
    scalar_of(name, bound)
}

/// The scalar a JSON parameter value binds as (see [`bind_value`]).
fn scalar_of(name: &str, bound: &serde_json::Value) -> Result<Value> {
    match bound {
        serde_json::Value::Number(n) => Ok(number_of(n)),
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err(Error::invalid_parameter(
                name,
                format!("expected a number, string, boolean or null, got {bound}"),
            ))
        }
    }
}

/// The narrowest of `i64`, `u64` and `f64` that holds `n`.
fn number_of(n: &serde_json::Number) -> Value {
    n.as_i64()
        .map(Value::Integer)
        .or_else(|| n.as_u64().map(Value::UnsignedInteger))
        .or_else(|| n.as_f64().map(Value::Float))
        .unwrap_or(Value::Null)
}

/// Binds every scalar `$name` operand in `cond`: comparison, `IN`,
/// `BETWEEN` and `CONTAINS` values, through `AND`, `OR`, `NOT` and groups.
///
/// The other leaves are returned unchanged: they carry no scalar operand
/// (geo thresholds are `f64` literals, LIKE and MATCH patterns are strings,
/// vector parameters are bound by the vector pipeline), except `GraphMatch`,
/// whose pattern properties the MATCH engine evaluates itself.
///
/// # Errors
///
/// Returns the [`bind_value`] error of the first operand that fails.
pub fn bind_condition<S: BuildHasher>(
    cond: &Condition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Condition> {
    match cond {
        Condition::And(left, right) => {
            bind_pair(left, right, params).map(|(l, r)| Condition::And(l, r))
        }
        Condition::Or(left, right) => {
            bind_pair(left, right, params).map(|(l, r)| Condition::Or(l, r))
        }
        Condition::Not(inner) => bind_boxed(inner, params).map(Condition::Not),
        Condition::Group(inner) => bind_boxed(inner, params).map(Condition::Group),
        leaf => bind_leaf(leaf, params),
    }
}

/// [`bind_condition`] on a combinator's operand.
fn bind_boxed<S: BuildHasher>(
    cond: &Condition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Box<Condition>> {
    bind_condition(cond, params).map(Box::new)
}

/// [`bind_condition`] on a binary combinator's two operands, left first.
fn bind_pair<S: BuildHasher>(
    left: &Condition,
    right: &Condition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<(Box<Condition>, Box<Condition>)> {
    Ok((bind_boxed(left, params)?, bind_boxed(right, params)?))
}

/// [`bind_condition`] on a leaf: binds its scalar operands, if it has any.
fn bind_leaf<S: BuildHasher>(
    leaf: &Condition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Condition> {
    match leaf {
        Condition::Comparison(cmp) => bind_comparison(cmp, params).map(Condition::Comparison),
        Condition::In(in_cond) => bind_in(in_cond, params).map(Condition::In),
        Condition::Between(btw) => bind_between(btw, params).map(Condition::Between),
        Condition::Contains(contains) => bind_contains(contains, params).map(Condition::Contains),
        other => Ok(other.clone()),
    }
}

fn bind_comparison<S: BuildHasher>(
    cmp: &Comparison,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Comparison> {
    Ok(Comparison {
        column: cmp.column.clone(),
        operator: cmp.operator,
        value: bind_value(&cmp.value, params)?,
    })
}

fn bind_in<S: BuildHasher>(
    in_cond: &InCondition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<InCondition> {
    Ok(InCondition {
        column: in_cond.column.clone(),
        values: bind_values(&in_cond.values, params)?,
        negated: in_cond.negated,
    })
}

fn bind_between<S: BuildHasher>(
    btw: &BetweenCondition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<BetweenCondition> {
    Ok(BetweenCondition {
        column: btw.column.clone(),
        low: bind_value(&btw.low, params)?,
        high: bind_value(&btw.high, params)?,
    })
}

fn bind_contains<S: BuildHasher>(
    contains: &ContainsCondition,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<ContainsCondition> {
    Ok(ContainsCondition {
        column: contains.column.clone(),
        mode: contains.mode,
        values: bind_values(&contains.values, params)?,
    })
}

/// Binds the `$name` values of every inline pattern property of `clause`,
/// nodes and relationships alike, so a pattern element is tested against
/// the caller's value ([`super::inline_properties_hold`]); the WHERE is
/// bound leaf by leaf where it is evaluated.
///
/// # Errors
///
/// Returns the [`bind_value`] error of the first property that fails.
pub fn bind_match_clause<S: BuildHasher>(
    clause: &MatchClause,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<MatchClause> {
    let mut bound = clause.clone();
    for pattern in &mut bound.patterns {
        for properties in pattern
            .nodes
            .iter_mut()
            .map(|node| &mut node.properties)
            .chain(
                pattern
                    .relationships
                    .iter_mut()
                    .map(|rel| &mut rel.properties),
            )
        {
            for value in properties.values_mut() {
                *value = bind_value(value, params)?;
            }
        }
    }
    Ok(bound)
}

/// Binds a vector expression: a literal is returned as is, a `$name` binds
/// to the array of numbers the caller bound to it, each narrowed to `f32`.
///
/// The one place a vector `$name` becomes a vector — the server's `NEAR`,
/// `similarity()` and `ORDER BY similarity()` pipelines and the WASM
/// executor all bind through it.
///
/// # Errors
///
/// Returns [`Error::QueryParameter`] when the parameter is missing from
/// `params`, is not an array, or holds an element that is not a number in
/// `f32` range.
pub fn bind_vector<S: BuildHasher>(
    vector: &VectorExpr,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Vec<f32>> {
    let name = match vector {
        VectorExpr::Literal(v) => return Ok(v.clone()),
        VectorExpr::Parameter(name) => name,
    };
    let bound = params
        .get(name)
        .ok_or_else(|| Error::unbound_parameter(name))?;
    let serde_json::Value::Array(elements) = bound else {
        return Err(Error::invalid_parameter(
            name,
            "expected a vector (an array of numbers)",
        ));
    };
    elements.iter().map(|v| vector_element(v, name)).collect()
}

/// A JSON vector element as an `f32`: a number outside `f32` range or a
/// non-number is refused. A JSON number is always finite
/// (`a_json_number_is_always_finite` pins why).
fn vector_element(v: &serde_json::Value, name: &str) -> Result<f32> {
    v.as_f64().and_then(f64_to_f32).ok_or_else(|| {
        Error::invalid_parameter(
            name,
            format!("vector element {v} is not a number in f32 range"),
        )
    })
}

/// Narrows an f64 to f32, returning `None` outside f32 range.
fn f64_to_f32(f: f64) -> Option<f32> {
    #[allow(clippy::cast_possible_truncation)]
    // Reason: `f` is within f32 range, so the cast only rounds.
    (f64::from(f32::MIN)..=f64::from(f32::MAX))
        .contains(&f)
        .then_some(f as f32)
}

/// The JSON an `INSERT`/`UPDATE` payload cell stores: a literal as itself
/// ([`Value::to_json`], a temporal as its epoch seconds), a `$name` as the
/// caller's JSON value whole — an array or object included, which a payload
/// field can hold.
///
/// The one binder of a DML payload cell: [`super::bind_insert_row`] and
/// [`super::bind_update_assignments`] — core's `INSERT`, `UPSERT` and
/// `UPDATE`, and the WASM executor's — bind every cell through it.
///
/// # Errors
///
/// [`Error::QueryParameter`] (`VELES-038`) for an unbound `$name`;
/// [`Error::Query`] (`VELES-010`) for a subquery, which no DML cell runs.
pub(crate) fn bind_dml_value<S: BuildHasher>(
    value: &Value,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<serde_json::Value> {
    match value {
        Value::Parameter(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| Error::unbound_parameter(name)),
        Value::Subquery(_) => Err(dml_subquery_refused()),
        literal => Ok(literal.to_json()),
    }
}

/// The vector an `INSERT`/`UPDATE` `vector` cell assigns. A `$name` binds
/// through [`bind_vector`], the binder of every vector parameter, so one
/// bound to anything but an array of numbers in `f32` range is the same
/// parameter error (`VELES-038`) as on a `NEAR`. A literal is refused: the
/// DML grammar's `value` has no array form, so no literal is a vector.
///
/// # Errors
///
/// [`bind_vector`]'s [`Error::QueryParameter`] for the `$name`;
/// [`Error::Query`] (`VELES-010`) for a literal or a subquery.
pub(crate) fn bind_dml_vector<S: BuildHasher>(
    value: &Value,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Vec<f32>> {
    match value {
        Value::Parameter(name) => bind_vector(&VectorExpr::Parameter(name.clone()), params),
        Value::Subquery(_) => Err(dml_subquery_refused()),
        _ => Err(Error::Query(
            "'vector' must be an array of numbers".to_string(),
        )),
    }
}

/// The refusal of a subquery in a DML cell.
fn dml_subquery_refused() -> Error {
    Error::Query("Subquery values are not supported in INSERT/UPDATE".to_string())
}

/// Binds every value of a list through [`bind_value`].
fn bind_values<S: BuildHasher>(
    values: &[Value],
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Vec<Value>> {
    values.iter().map(|v| bind_value(v, params)).collect()
}

#[cfg(test)]
#[path = "param_binding_tests.rs"]
mod tests;
