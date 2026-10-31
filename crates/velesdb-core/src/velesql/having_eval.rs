//! HAVING over a group's aggregates: its verdict, with SQL's precedence, and
//! the thresholds it accepts (`V018`).

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
            .map(|c| compare(result.value_of(&c.aggregate).as_f64(), c.operator, &c.value));
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

/// Compares an aggregate against a HAVING threshold. A missing aggregate or a
/// non-numeric threshold fails; equality allows for float rounding in sums.
fn compare(aggregate: Option<f64>, op: CompareOp, threshold: &Value) -> bool {
    let (Some(value), Some(threshold)) = (aggregate, threshold_f64(threshold)) else {
        return false;
    };
    let epsilon = f64::EPSILON * value.abs().max(threshold.abs()).max(1.0);
    match op {
        CompareOp::Eq => (value - threshold).abs() < epsilon,
        CompareOp::NotEq => (value - threshold).abs() >= epsilon,
        CompareOp::Gt => value > threshold,
        CompareOp::Gte => value >= threshold,
        CompareOp::Lt => value < threshold,
        CompareOp::Lte => value <= threshold,
    }
}

/// A numeric threshold as the `f64` aggregates compare as; `None` otherwise
/// (`NULL`, SQL's UNKNOWN, among them).
// Reason: thresholds compare as f64, like the aggregates they test.
#[allow(clippy::cast_precision_loss)]
fn threshold_f64(threshold: &Value) -> Option<f64> {
    match threshold {
        Value::Integer(i) => Some(*i as f64),
        Value::UnsignedInteger(u) => Some(*u as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}
