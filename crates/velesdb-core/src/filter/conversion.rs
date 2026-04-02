//! Conversion from `VelesQL` conditions to filter conditions.

use super::Condition;
use serde_json::Value;

/// Converts a single `VelesQL` value into its `serde_json` equivalent.
///
/// Temporal values are converted to epoch seconds; subqueries and parameters
/// produce `Value::Null` (runtime evaluation happens in the query executor).
fn velesql_value_to_json(v: crate::velesql::Value) -> Value {
    match v {
        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
        crate::velesql::Value::UnsignedInteger(u) => Value::Number(u.into()),
        crate::velesql::Value::Float(f) => Value::from(f),
        crate::velesql::Value::String(s) => Value::String(s),
        crate::velesql::Value::Boolean(b) => Value::Bool(b),
        crate::velesql::Value::Null
        | crate::velesql::Value::Parameter(_)
        | crate::velesql::Value::Subquery(_) => Value::Null,
        crate::velesql::Value::Temporal(t) => Value::Number(t.to_epoch_seconds().into()),
    }
}

/// Converts a `VelesQL` value to JSON, accepting only numeric types.
///
/// Non-numeric variants (String, Boolean, Null, etc.) produce `Value::Null`.
fn velesql_numeric_to_json(v: &crate::velesql::Value) -> Value {
    match *v {
        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
        crate::velesql::Value::UnsignedInteger(u) => Value::Number(u.into()),
        crate::velesql::Value::Float(f) => Value::from(f),
        _ => Value::Null,
    }
}

/// Sentinel for conditions handled externally by the query engine (vector
/// search, graph match, etc.). Uses an empty AND as the identity element.
fn engine_handled_identity() -> Condition {
    Condition::And { conditions: vec![] }
}

/// Converts a comparison condition using the shared value converter.
fn convert_comparison(
    column: String,
    operator: crate::velesql::CompareOp,
    value: crate::velesql::Value,
) -> Condition {
    let value = velesql_value_to_json(value);
    match operator {
        crate::velesql::CompareOp::Eq => Condition::eq(column, value),
        crate::velesql::CompareOp::NotEq => Condition::neq(column, value),
        crate::velesql::CompareOp::Gt => Condition::Gt {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Gte => Condition::Gte {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Lt => Condition::Lt {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Lte => Condition::Lte {
            field: column,
            value,
        },
    }
}

impl From<crate::velesql::Condition> for Condition {
    fn from(cond: crate::velesql::Condition) -> Self {
        match cond {
            crate::velesql::Condition::Comparison(cmp) => {
                convert_comparison(cmp.column, cmp.operator, cmp.value)
            }
            crate::velesql::Condition::In(inc) => {
                let in_cond = Self::In {
                    field: inc.column,
                    values: inc.values.into_iter().map(velesql_value_to_json).collect(),
                };
                if inc.negated {
                    Self::Not {
                        condition: Box::new(in_cond),
                    }
                } else {
                    in_cond
                }
            }
            crate::velesql::Condition::IsNull(isn) => {
                if isn.is_null {
                    Self::IsNull { field: isn.column }
                } else {
                    Self::IsNotNull { field: isn.column }
                }
            }
            crate::velesql::Condition::And(left, right) => Self::And {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Or(left, right) => Self::Or {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Not(inner) => Self::Not {
                condition: Box::new(Self::from(*inner)),
            },
            crate::velesql::Condition::Group(inner) => Self::from(*inner),
            crate::velesql::Condition::VectorSearch(_)
            | crate::velesql::Condition::VectorFusedSearch(_)
            | crate::velesql::Condition::SparseVectorSearch(_)
            | crate::velesql::Condition::Similarity(_)
            | crate::velesql::Condition::GraphMatch(_) => engine_handled_identity(),
            crate::velesql::Condition::Match(m) => Self::Contains {
                field: m.column,
                value: m.query,
            },
            crate::velesql::Condition::Between(btw) => Self::And {
                conditions: vec![
                    Self::Gte {
                        field: btw.column.clone(),
                        value: velesql_numeric_to_json(&btw.low),
                    },
                    Self::Lte {
                        field: btw.column,
                        value: velesql_numeric_to_json(&btw.high),
                    },
                ],
            },
            crate::velesql::Condition::Like(lk) => {
                if lk.case_insensitive {
                    Self::ILike {
                        field: lk.column,
                        pattern: lk.pattern,
                    }
                } else {
                    Self::Like {
                        field: lk.column,
                        pattern: lk.pattern,
                    }
                }
            }
        }
    }
}

// Tests moved to conversion_tests.rs per project rules
