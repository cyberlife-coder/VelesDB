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
    BetweenCondition, Comparison, Condition, ContainsCondition, InCondition, Value,
};

/// Binds `value` when it is a `$name` parameter; returns any other value
/// unchanged.
///
/// A JSON number binds as the narrowest of `i64`, `u64` and `f64` that
/// holds it, a string, boolean or `null` as itself.
///
/// # Errors
///
/// Returns [`Error::Query`] when the parameter is missing from `params`, or
/// bound to an array or object, which no scalar operand takes.
pub fn bind_value<S: BuildHasher>(
    value: &Value,
    params: &HashMap<String, serde_json::Value, S>,
) -> Result<Value> {
    let Value::Parameter(name) = value else {
        return Ok(value.clone());
    };
    let bound = params
        .get(name)
        .ok_or_else(|| Error::Query(format!("Missing parameter: ${name}")))?;
    scalar_of(name, bound)
}

/// The scalar a JSON parameter value binds as (see [`bind_value`]).
fn scalar_of(name: &str, bound: &serde_json::Value) -> Result<Value> {
    match bound {
        serde_json::Value::Number(n) => Ok(number_of(n)),
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Err(Error::Query(format!(
            "Unsupported parameter type for ${name}: {bound:?}",
        ))),
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
