//! Conversion from VelesQL conditions to filter conditions.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{
        BetweenCondition, CompareOp, Comparison, InCondition, IsNullCondition, LikeCondition,
        MatchCondition, Value as VelesValue,
    };

    #[test]
    fn test_comparison_eq_integer() {
        let cmp = Comparison {
            column: "age".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Integer(25),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::Eq { field, value } if field == "age" && value == Value::Number(25.into()))
        );
    }

    #[test]
    fn test_comparison_neq_string() {
        let cmp = Comparison {
            column: "status".to_string(),
            operator: CompareOp::NotEq,
            value: VelesValue::String("inactive".to_string()),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::Neq { field, value } if field == "status" && value == Value::String("inactive".to_string()))
        );
    }

    #[test]
    fn test_comparison_gt_float() {
        let cmp = Comparison {
            column: "price".to_string(),
            operator: CompareOp::Gt,
            value: VelesValue::Float(99.99),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Gt { field, .. } if field == "price"));
    }

    #[test]
    fn test_comparison_gte() {
        let cmp = Comparison {
            column: "count".to_string(),
            operator: CompareOp::Gte,
            value: VelesValue::Integer(10),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Gte { field, .. } if field == "count"));
    }

    #[test]
    fn test_comparison_lt() {
        let cmp = Comparison {
            column: "score".to_string(),
            operator: CompareOp::Lt,
            value: VelesValue::Integer(50),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Lt { field, .. } if field == "score"));
    }

    #[test]
    fn test_comparison_lte() {
        let cmp = Comparison {
            column: "level".to_string(),
            operator: CompareOp::Lte,
            value: VelesValue::Integer(5),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Lte { field, .. } if field == "level"));
    }

    #[test]
    fn test_comparison_boolean() {
        let cmp = Comparison {
            column: "active".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Boolean(true),
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::Eq { field, value } if field == "active" && value == Value::Bool(true))
        );
    }

    #[test]
    fn test_comparison_null() {
        let cmp = Comparison {
            column: "field".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Null,
        };
        let cond = crate::velesql::Condition::Comparison(cmp);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Eq { value, .. } if value == Value::Null));
    }

    #[test]
    fn test_in_condition() {
        let inc = InCondition {
            column: "category".to_string(),
            values: vec![
                VelesValue::String("a".to_string()),
                VelesValue::String("b".to_string()),
            ],
        };
        let cond = crate::velesql::Condition::In(inc);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::In { field, values } if field == "category" && values.len() == 2)
        );
    }

    #[test]
    fn test_is_null_true() {
        let isn = IsNullCondition {
            column: "optional".to_string(),
            is_null: true,
        };
        let cond = crate::velesql::Condition::IsNull(isn);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::IsNull { field } if field == "optional"));
    }

    #[test]
    fn test_is_null_false() {
        let isn = IsNullCondition {
            column: "required".to_string(),
            is_null: false,
        };
        let cond = crate::velesql::Condition::IsNull(isn);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::IsNotNull { field } if field == "required"));
    }

    #[test]
    fn test_and_condition() {
        let left = crate::velesql::Condition::Comparison(Comparison {
            column: "a".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Integer(1),
        });
        let right = crate::velesql::Condition::Comparison(Comparison {
            column: "b".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Integer(2),
        });
        let cond = crate::velesql::Condition::And(Box::new(left), Box::new(right));
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
    }

    #[test]
    fn test_or_condition() {
        let left = crate::velesql::Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Integer(1),
        });
        let right = crate::velesql::Condition::Comparison(Comparison {
            column: "y".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Integer(2),
        });
        let cond = crate::velesql::Condition::Or(Box::new(left), Box::new(right));
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Or { conditions } if conditions.len() == 2));
    }

    #[test]
    fn test_not_condition() {
        let inner = crate::velesql::Condition::Comparison(Comparison {
            column: "deleted".to_string(),
            operator: CompareOp::Eq,
            value: VelesValue::Boolean(true),
        });
        let cond = crate::velesql::Condition::Not(Box::new(inner));
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Not { .. }));
    }

    #[test]
    fn test_group_condition() {
        let inner = crate::velesql::Condition::Comparison(Comparison {
            column: "val".to_string(),
            operator: CompareOp::Gt,
            value: VelesValue::Integer(0),
        });
        let cond = crate::velesql::Condition::Group(Box::new(inner));
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::Gt { field, .. } if field == "val"));
    }

    #[test]
    fn test_match_condition() {
        let m = MatchCondition {
            column: "text".to_string(),
            query: "hello".to_string(),
        };
        let cond = crate::velesql::Condition::Match(m);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::Contains { field, value } if field == "text" && value == "hello")
        );
    }

    #[test]
    fn test_between_condition_integers() {
        let btw = BetweenCondition {
            column: "age".to_string(),
            low: VelesValue::Integer(18),
            high: VelesValue::Integer(65),
        };
        let cond = crate::velesql::Condition::Between(btw);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
    }

    #[test]
    fn test_between_condition_floats() {
        let btw = BetweenCondition {
            column: "price".to_string(),
            low: VelesValue::Float(10.0),
            high: VelesValue::Float(100.0),
        };
        let cond = crate::velesql::Condition::Between(btw);
        let result: Condition = cond.into();
        assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
    }

    #[test]
    fn test_like_case_sensitive() {
        let lk = LikeCondition {
            column: "name".to_string(),
            pattern: "%test%".to_string(),
            case_insensitive: false,
        };
        let cond = crate::velesql::Condition::Like(lk);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::Like { field, pattern } if field == "name" && pattern == "%test%")
        );
    }

    #[test]
    fn test_like_case_insensitive() {
        let lk = LikeCondition {
            column: "title".to_string(),
            pattern: "%search%".to_string(),
            case_insensitive: true,
        };
        let cond = crate::velesql::Condition::Like(lk);
        let result: Condition = cond.into();
        assert!(
            matches!(result, Condition::ILike { field, pattern } if field == "title" && pattern == "%search%")
        );
    }
}
