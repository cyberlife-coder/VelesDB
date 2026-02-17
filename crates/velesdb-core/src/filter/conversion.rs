//! Conversion from `VelesQL` conditions to filter conditions.

use super::Condition;
use serde_json::Value;

impl From<crate::velesql::Condition> for Condition {
    #[allow(clippy::too_many_lines)]
    fn from(cond: crate::velesql::Condition) -> Self {
        match cond {
            crate::velesql::Condition::Comparison(cmp) => {
                let value = match cmp.value {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    crate::velesql::Value::String(s) => Value::String(s),
                    crate::velesql::Value::Boolean(b) => Value::Bool(b),
                    crate::velesql::Value::Null | crate::velesql::Value::Parameter(_) => {
                        Value::Null
                    }
                    crate::velesql::Value::Temporal(t) => {
                        // Convert temporal to epoch seconds for comparison
                        Value::Number(t.to_epoch_seconds().into())
                    }
                    crate::velesql::Value::Subquery(_) => {
                        // VP-002: Subqueries should be resolved before reaching filter conversion.
                        // If we get here, resolve_subqueries_in_condition was not called.
                        tracing::warn!(
                            "Subquery reached filter conversion without resolution — this is a bug"
                        );
                        Value::Null
                    }
                };
                match cmp.operator {
                    crate::velesql::CompareOp::Eq => Self::eq(cmp.column, value),
                    crate::velesql::CompareOp::NotEq => Self::neq(cmp.column, value),
                    crate::velesql::CompareOp::Gt => Self::Gt {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Gte => Self::Gte {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Lt => Self::Lt {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Lte => Self::Lte {
                        field: cmp.column,
                        value,
                    },
                }
            }
            crate::velesql::Condition::In(inc) => {
                let values = inc
                    .values
                    .into_iter()
                    .map(|v| match v {
                        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                        crate::velesql::Value::Float(f) => Value::from(f),
                        crate::velesql::Value::String(s) => Value::String(s),
                        crate::velesql::Value::Boolean(b) => Value::Bool(b),
                        crate::velesql::Value::Null | crate::velesql::Value::Parameter(_) => {
                            Value::Null
                        }
                        crate::velesql::Value::Temporal(t) => {
                            Value::Number(t.to_epoch_seconds().into())
                        }
                        crate::velesql::Value::Subquery(_) => {
                            // VP-002: Subqueries should be resolved before reaching filter conversion.
                            tracing::warn!("Subquery in IN clause reached filter conversion without resolution");
                            Value::Null
                        }
                    })
                    .collect();
                Self::In {
                    field: inc.column,
                    values,
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
            crate::velesql::Condition::VectorSearch(_) => {
                // Vector search is handled separately by the query engine
                Self::And { conditions: vec![] } // Identity for AND
            }
            crate::velesql::Condition::VectorFusedSearch(_) => {
                // Fused vector search is handled separately by the query engine
                Self::And { conditions: vec![] } // Identity for AND
            }
            crate::velesql::Condition::Similarity(_) => {
                // Similarity function is handled separately by the query engine
                // It combines vector search with graph traversal
                Self::And { conditions: vec![] } // Identity for AND
            }
            crate::velesql::Condition::Match(m) => Self::Contains {
                field: m.column,
                value: m.query,
            },
            crate::velesql::Condition::Between(btw) => {
                let low = match btw.low {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    _ => Value::Null,
                };
                let high = match btw.high {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    _ => Value::Null,
                };
                Self::And {
                    conditions: vec![
                        Self::Gte {
                            field: btw.column.clone(),
                            value: low,
                        },
                        Self::Lte {
                            field: btw.column,
                            value: high,
                        },
                    ],
                }
            }
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
