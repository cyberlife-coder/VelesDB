//! Extended temporal parser tests — interval units, shorthands, and edge cases.
//!
//! Complements `parser::temporal_tests` which covers basic NOW(), INTERVAL,
//! and arithmetic. This module exercises all unit forms (singular, plural,
//! shorthand), complex temporal WHERE clauses, and negative cases.

use crate::velesql::ast::{IntervalUnit, IntervalValue, TemporalExpr, Value};
use crate::velesql::{Condition, Parser};

// =============================================================================
// Nominal: all singular unit forms
// =============================================================================

#[test]
fn test_singular_unit_second() {
    assert_interval_unit("1 second", 1, IntervalUnit::Seconds);
}

#[test]
fn test_singular_unit_minute() {
    assert_interval_unit("1 minute", 1, IntervalUnit::Minutes);
}

#[test]
fn test_singular_unit_hour() {
    assert_interval_unit("1 hour", 1, IntervalUnit::Hours);
}

#[test]
fn test_singular_unit_day() {
    assert_interval_unit("1 day", 1, IntervalUnit::Days);
}

#[test]
fn test_singular_unit_week() {
    assert_interval_unit("1 week", 1, IntervalUnit::Weeks);
}

#[test]
fn test_singular_unit_month() {
    assert_interval_unit("1 month", 1, IntervalUnit::Months);
}

// =============================================================================
// Nominal: all plural unit forms
// =============================================================================

#[test]
fn test_plural_unit_seconds() {
    assert_interval_unit("30 seconds", 30, IntervalUnit::Seconds);
}

#[test]
fn test_plural_unit_minutes() {
    assert_interval_unit("5 minutes", 5, IntervalUnit::Minutes);
}

#[test]
fn test_plural_unit_hours() {
    assert_interval_unit("24 hours", 24, IntervalUnit::Hours);
}

#[test]
fn test_plural_unit_days() {
    assert_interval_unit("7 days", 7, IntervalUnit::Days);
}

#[test]
fn test_plural_unit_weeks() {
    assert_interval_unit("4 weeks", 4, IntervalUnit::Weeks);
}

#[test]
fn test_plural_unit_months() {
    assert_interval_unit("6 months", 6, IntervalUnit::Months);
}

// =============================================================================
// Nominal: shorthand unit forms
// =============================================================================

#[test]
fn test_shorthand_s() {
    assert_interval_unit("30 s", 30, IntervalUnit::Seconds);
}

#[test]
fn test_shorthand_sec() {
    assert_interval_unit("1 sec", 1, IntervalUnit::Seconds);
}

#[test]
fn test_shorthand_min() {
    assert_interval_unit("5 min", 5, IntervalUnit::Minutes);
}

#[test]
fn test_shorthand_m() {
    assert_interval_unit("5 m", 5, IntervalUnit::Minutes);
}

#[test]
fn test_shorthand_h() {
    assert_interval_unit("2 h", 2, IntervalUnit::Hours);
}

#[test]
fn test_shorthand_d() {
    assert_interval_unit("7 d", 7, IntervalUnit::Days);
}

#[test]
fn test_shorthand_w() {
    assert_interval_unit("2 w", 2, IntervalUnit::Weeks);
}

// =============================================================================
// Nominal: temporal in comparison and complex WHERE clauses
// =============================================================================

#[test]
fn test_temporal_comparison_gt() {
    let sql = "SELECT * FROM events WHERE timestamp > NOW() - INTERVAL '7 days'";
    let query = Parser::parse(sql).expect("temporal comparison should parse");

    assert!(
        query.select.where_clause.is_some(),
        "WHERE clause must be present"
    );
    match query.select.where_clause.as_ref() {
        Some(Condition::Comparison(cmp)) => {
            assert_eq!(cmp.column, "timestamp");
            assert!(
                matches!(&cmp.value, Value::Temporal(TemporalExpr::Subtract(_, _))),
                "Expected Temporal(Subtract), got {:?}",
                cmp.value
            );
        }
        other => panic!("Expected Comparison, got {other:?}"),
    }
}

#[test]
fn test_temporal_between() {
    let sql = "SELECT * FROM events WHERE ts BETWEEN NOW() - INTERVAL '7 days' AND NOW()";
    let query = Parser::parse(sql).expect("BETWEEN with temporal should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Between(b)) => {
            assert_eq!(b.column, "ts");
            assert!(
                matches!(&b.low, Value::Temporal(TemporalExpr::Subtract(_, _))),
                "low should be Temporal(Subtract), got {:?}",
                b.low
            );
            assert!(
                matches!(&b.high, Value::Temporal(TemporalExpr::Now)),
                "high should be Temporal(Now), got {:?}",
                b.high
            );
        }
        other => panic!("Expected Between condition, got {other:?}"),
    }
}

#[test]
fn test_temporal_with_and_filter() {
    let sql = "SELECT * FROM logs WHERE ts > NOW() - INTERVAL '24 hours' AND status = 'active'";
    let query = Parser::parse(sql).expect("temporal + AND filter should parse");

    assert!(
        matches!(query.select.where_clause, Some(Condition::And(_, _))),
        "Root condition should be AND"
    );
}

#[test]
fn test_multiple_temporal_in_one_query() {
    let sql = "SELECT * FROM events WHERE start > NOW() - INTERVAL '1 hour' AND end_time < NOW() + INTERVAL '1 hour'";
    let query = Parser::parse(sql).expect("multiple temporal expressions should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::And(left, right)) => {
            // Left: start > NOW() - INTERVAL '1 hour'
            assert!(
                matches!(left.as_ref(), Condition::Comparison(_)),
                "Left should be Comparison, got {left:?}"
            );
            // Right: end_time < NOW() + INTERVAL '1 hour'
            assert!(
                matches!(right.as_ref(), Condition::Comparison(_)),
                "Right should be Comparison, got {right:?}"
            );

            // Verify the temporal directions
            if let Condition::Comparison(cmp) = left.as_ref() {
                assert!(
                    matches!(&cmp.value, Value::Temporal(TemporalExpr::Subtract(_, _))),
                    "Left value should be Subtract, got {:?}",
                    cmp.value
                );
            }
            if let Condition::Comparison(cmp) = right.as_ref() {
                assert!(
                    matches!(&cmp.value, Value::Temporal(TemporalExpr::Add(_, _))),
                    "Right value should be Add, got {:?}",
                    cmp.value
                );
            }
        }
        other => panic!("Expected AND, got {other:?}"),
    }
}

// =============================================================================
// Edge cases: large magnitude, zero, case insensitivity
// =============================================================================

#[test]
fn test_large_magnitude() {
    assert_interval_unit("365 days", 365, IntervalUnit::Days);
}

#[test]
fn test_zero_magnitude() {
    assert_interval_unit("0 seconds", 0, IntervalUnit::Seconds);
}

#[test]
fn test_case_insensitivity_interval_keyword() {
    // INTERVAL keyword is case-insensitive via pest ^"INTERVAL"
    let sql = "SELECT * FROM events WHERE ts > interval '7 days'";
    let query =
        Parser::parse(sql).unwrap_or_else(|e| panic!("Lowercase 'interval' should parse: {e:?}"));
    let where_clause = query
        .select
        .where_clause
        .as_ref()
        .expect("WHERE clause must be present");
    match where_clause {
        Condition::Comparison(cmp) => match &cmp.value {
            Value::Temporal(TemporalExpr::Interval(iv)) => {
                assert_eq!(iv.magnitude, 7);
                assert_eq!(iv.unit, IntervalUnit::Days);
            }
            other => panic!("Expected Temporal(Interval), got {other:?}"),
        },
        other => panic!("Expected Comparison, got {other:?}"),
    }
}

#[test]
fn test_case_insensitivity_unit_uppercase() {
    // The unit parsing uses `.to_lowercase()`, so uppercase units work.
    assert_interval_unit("7 DAYS", 7, IntervalUnit::Days);
}

#[test]
fn test_to_seconds_large_interval() {
    let iv = IntervalValue {
        magnitude: 365,
        unit: IntervalUnit::Days,
    };
    assert_eq!(iv.to_seconds(), 365 * 86400);
}

#[test]
fn test_to_seconds_zero() {
    let iv = IntervalValue {
        magnitude: 0,
        unit: IntervalUnit::Seconds,
    };
    assert_eq!(iv.to_seconds(), 0);
}

// =============================================================================
// Negative: invalid interval syntax
// =============================================================================

#[test]
fn test_invalid_unit_fortnights() {
    let sql = "SELECT * FROM events WHERE ts > INTERVAL '7 fortnights'";
    let result = Parser::parse(sql);
    assert!(
        result.is_err(),
        "'fortnights' is not a valid interval unit, should fail"
    );
}

#[test]
fn test_missing_magnitude() {
    let sql = "SELECT * FROM events WHERE ts > INTERVAL 'days'";
    let result = Parser::parse(sql);
    assert!(
        result.is_err(),
        "INTERVAL without magnitude should fail to parse"
    );
}

#[test]
fn test_empty_interval() {
    let sql = "SELECT * FROM events WHERE ts > INTERVAL ''";
    let result = Parser::parse(sql);
    assert!(
        result.is_err(),
        "Empty INTERVAL string should fail to parse"
    );
}

// =============================================================================
// Helper: parse INTERVAL and assert magnitude + unit
// =============================================================================

/// Parses `SELECT * FROM t WHERE c > INTERVAL '<interval_str>'` and asserts
/// the resulting `IntervalValue` has the expected magnitude and unit.
fn assert_interval_unit(interval_str: &str, expected_mag: i64, expected_unit: IntervalUnit) {
    let sql = format!("SELECT * FROM events WHERE ts > INTERVAL '{interval_str}'");
    let query = Parser::parse(&sql)
        .unwrap_or_else(|e| panic!("Failed to parse interval '{interval_str}': {e:?}"));

    let where_clause = query
        .select
        .where_clause
        .as_ref()
        .expect("WHERE clause must be present");

    match where_clause {
        Condition::Comparison(cmp) => match &cmp.value {
            Value::Temporal(TemporalExpr::Interval(iv)) => {
                assert_eq!(
                    iv.magnitude, expected_mag,
                    "Wrong magnitude for '{interval_str}': expected {expected_mag}, got {}",
                    iv.magnitude
                );
                assert_eq!(
                    iv.unit, expected_unit,
                    "Wrong unit for '{interval_str}': expected {expected_unit:?}, got {:?}",
                    iv.unit
                );
            }
            other => panic!("Expected Temporal(Interval) for '{interval_str}', got {other:?}"),
        },
        other => panic!("Expected Comparison for '{interval_str}', got {other:?}"),
    }
}
