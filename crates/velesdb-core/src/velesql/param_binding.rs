//! Binding of `$name` parameters to the caller's values.
//!
//! The one place a VelesQL `$name` becomes a value. Every engine binds
//! through it before a condition reaches a payload filter — the server's
//! SELECT, MATCH and HAVING pipelines, and the WASM executor — so a missing
//! or unsupported parameter is the same error everywhere, never a silent
//! `NULL`. It lives outside `collection` so that a build without
//! `persistence` (WASM) binds exactly as the server does.

use std::collections::HashMap;

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
pub fn bind_value(value: &Value, params: &HashMap<String, serde_json::Value>) -> Result<Value> {
    let Value::Parameter(name) = value else {
        return Ok(value.clone());
    };
    let bound = params
        .get(name)
        .ok_or_else(|| Error::Query(format!("Missing parameter: ${name}")))?;
    Ok(match bound {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else if let Some(u) = n.as_u64() {
                Value::UnsignedInteger(u)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            return Err(Error::Query(format!(
                "Unsupported parameter type for ${name}: {bound:?}",
            )));
        }
    })
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
pub fn bind_condition(
    cond: &Condition,
    params: &HashMap<String, serde_json::Value>,
) -> Result<Condition> {
    let boxed = |inner: &Condition| bind_condition(inner, params).map(Box::new);
    Ok(match cond {
        Condition::And(left, right) => Condition::And(boxed(left)?, boxed(right)?),
        Condition::Or(left, right) => Condition::Or(boxed(left)?, boxed(right)?),
        Condition::Not(inner) => Condition::Not(boxed(inner)?),
        Condition::Group(inner) => Condition::Group(boxed(inner)?),
        Condition::Comparison(cmp) => Condition::Comparison(Comparison {
            column: cmp.column.clone(),
            operator: cmp.operator,
            value: bind_value(&cmp.value, params)?,
        }),
        Condition::In(in_cond) => Condition::In(InCondition {
            column: in_cond.column.clone(),
            values: bind_values(&in_cond.values, params)?,
            negated: in_cond.negated,
        }),
        Condition::Between(btw) => Condition::Between(BetweenCondition {
            column: btw.column.clone(),
            low: bind_value(&btw.low, params)?,
            high: bind_value(&btw.high, params)?,
        }),
        Condition::Contains(contains) => Condition::Contains(ContainsCondition {
            column: contains.column.clone(),
            mode: contains.mode,
            values: bind_values(&contains.values, params)?,
        }),
        other => other.clone(),
    })
}

/// Binds every value of a list through [`bind_value`].
fn bind_values(
    values: &[Value],
    params: &HashMap<String, serde_json::Value>,
) -> Result<Vec<Value>> {
    values.iter().map(|v| bind_value(v, params)).collect()
}
