//! HAVING over a group's aggregates: its verdict, with SQL's precedence, and
//! the thresholds it accepts (`V018`).

use std::cmp::Ordering;

use super::aggregator::AggregateResult;
use super::ast::{CompareOp, HavingClause, LogicalOp, Value};
use super::validation::{ValidationError, ValidationErrorKind};

impl HavingClause {
    /// Whether a group whose aggregates are `result` passes this HAVING, with
    /// SQL's precedence: `AND` binds tighter than `OR`, so `a OR b AND c` is
    /// `a OR (b AND c)`.
    pub(crate) fn accepts(&self, result: &AggregateResult) -> bool {
        let mut verdicts = self
            .conditions
            .iter()
            .map(|c| compare(&result.value_of(&c.aggregate), c.operator, &c.value));
        let Some(first) = verdicts.next() else {
            return true;
        };
        let (mut any, mut all) = (false, first);
        for (i, verdict) in verdicts.enumerate() {
            match self.operators.get(i).copied().unwrap_or(LogicalOp::And) {
                LogicalOp::And => all = all && verdict,
                LogicalOp::Or => {
                    any = any || all;
                    all = verdict;
                }
            }
        }
        any || all
    }

    /// `V018` unless every threshold is a number or `NULL`, which, SQL's
    /// UNKNOWN, matches no group. Before execution (`bound == false`) a
    /// `$parameter` or a scalar subquery passes too: both are replaced by
    /// their value before the query runs, and checked again then, by
    /// [`Grouper::new`](super::grouping::Grouper::new).
    pub(crate) fn check_thresholds(&self, bound: bool) -> Result<(), ValidationError> {
        let accepted = |value: &Value| match value {
            Value::Integer(_) | Value::UnsignedInteger(_) | Value::Float(_) | Value::Null => true,
            Value::Parameter(_) | Value::Subquery(_) => !bound,
            _ => false,
        };
        match self.conditions.iter().find(|c| !accepted(&c.value)) {
            Some(condition) => {
                let threshold = threshold_text(&condition.value);
                Err(ValidationError::new(
                    ValidationErrorKind::InvalidHaving,
                    None,
                    threshold.clone(),
                    format!(
                        "HAVING compares an aggregate with {threshold}: a threshold must be \
                         a number (or NULL, which matches no group)"
                    ),
                ))
            }
            None => Ok(()),
        }
    }
}

/// A threshold as the query states it: a literal as JSON (`"many"`,
/// `true`), anything else as VelesQL writes it.
fn threshold_text(value: &Value) -> String {
    match value {
        Value::Parameter(name) => format!("${name}"),
        Value::Temporal(_) => "a date or time".to_string(),
        Value::Subquery(_) => "a subquery".to_string(),
        literal => literal.to_json().to_string(),
    }
}

/// Compares an aggregate against a HAVING threshold, by exact value, as a
/// WHERE `=` decides (`compare_number_literal`): an integer aggregate and a
/// float threshold are told apart even past 2^53, no tolerance applies. A
/// missing aggregate or a non-numeric threshold fails every operator.
fn compare(aggregate: &serde_json::Value, op: CompareOp, threshold: &Value) -> bool {
    let serde_json::Value::Number(aggregate) = aggregate else {
        return false;
    };
    super::json_order::compare_number_literal(aggregate, threshold)
        .is_some_and(|ordering| holds(ordering, op))
}

/// Whether `ordering` (aggregate against threshold) satisfies `op`.
fn holds(ordering: Ordering, op: CompareOp) -> bool {
    match op {
        CompareOp::Eq => ordering.is_eq(),
        CompareOp::NotEq => ordering.is_ne(),
        CompareOp::Gt => ordering.is_gt(),
        CompareOp::Gte => ordering.is_ge(),
        CompareOp::Lt => ordering.is_lt(),
        CompareOp::Lte => ordering.is_le(),
    }
}
