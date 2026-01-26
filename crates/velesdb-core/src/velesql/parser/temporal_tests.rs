//! Tests for temporal expression parsing (EPIC-038).

use crate::velesql::ast::{IntervalUnit, TemporalExpr, Value};
use crate::velesql::Parser;

#[test]
fn test_parse_now_function() {
    let query = "SELECT * FROM events WHERE timestamp > NOW()";
    let result = Parser::parse(query);
    assert!(result.is_ok(), "Failed to parse NOW(): {:?}", result.err());
}

#[test]
fn test_parse_interval_days() {
    let query = "SELECT * FROM events WHERE timestamp > INTERVAL '7 days'";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse INTERVAL: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_now_minus_interval() {
    let query = "SELECT * FROM logs WHERE created_at > NOW() - INTERVAL '24 hours'";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse NOW() - INTERVAL: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_now_plus_interval() {
    let query = "SELECT * FROM tasks WHERE due_date < NOW() + INTERVAL '7 days'";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse NOW() + INTERVAL: {:?}",
        result.err()
    );
}

#[test]
fn test_interval_units() {
    let units = [
        ("1 second", IntervalUnit::Seconds),
        ("30 seconds", IntervalUnit::Seconds),
        ("5 minutes", IntervalUnit::Minutes),
        ("2 hours", IntervalUnit::Hours),
        ("7 days", IntervalUnit::Days),
        ("2 weeks", IntervalUnit::Weeks),
        ("1 month", IntervalUnit::Months),
    ];

    for (interval_str, expected_unit) in units {
        let query = format!(
            "SELECT * FROM events WHERE ts > INTERVAL '{}'",
            interval_str
        );
        let result = Parser::parse(&query);
        assert!(
            result.is_ok(),
            "Failed to parse interval '{}': {:?}",
            interval_str,
            result.err()
        );

        let parsed = result.unwrap();
        if let Some(crate::velesql::Condition::Comparison(cmp)) =
            parsed.select.where_clause.as_ref()
        {
            if let Value::Temporal(TemporalExpr::Interval(iv)) = &cmp.value {
                assert_eq!(
                    iv.unit, expected_unit,
                    "Wrong unit for '{}': expected {:?}, got {:?}",
                    interval_str, expected_unit, iv.unit
                );
            } else {
                panic!("Expected Temporal(Interval), got {:?}", cmp.value);
            }
        } else {
            panic!("Expected Comparison condition");
        }
    }
}

#[test]
fn test_temporal_expr_to_epoch_seconds() {
    use crate::velesql::ast::IntervalValue;

    // Test interval conversions
    let one_day = IntervalValue {
        magnitude: 1,
        unit: IntervalUnit::Days,
    };
    assert_eq!(one_day.to_seconds(), 86400);

    let one_week = IntervalValue {
        magnitude: 1,
        unit: IntervalUnit::Weeks,
    };
    assert_eq!(one_week.to_seconds(), 604_800);

    // Test NOW() returns a reasonable timestamp
    let now_expr = TemporalExpr::Now;
    let now_secs = now_expr.to_epoch_seconds();
    // Should be after Jan 1, 2020 (1577836800)
    assert!(now_secs > 1_577_836_800, "NOW() should return current time");

    // Test subtraction
    let week_ago = TemporalExpr::Subtract(
        Box::new(TemporalExpr::Now),
        Box::new(TemporalExpr::Interval(one_week.clone())),
    );
    let week_ago_secs = week_ago.to_epoch_seconds();
    assert!(
        now_secs - week_ago_secs >= 604_799 && now_secs - week_ago_secs <= 604_801,
        "NOW() - 1 week should be ~604800 seconds ago"
    );
}

#[test]
fn test_interval_shorthand_units() {
    let shorthands = ["1 s", "30 sec", "5 min", "2 h", "7 d", "2 w"];

    for shorthand in shorthands {
        let query = format!("SELECT * FROM events WHERE ts > INTERVAL '{}'", shorthand);
        let result = Parser::parse(&query);
        assert!(
            result.is_ok(),
            "Failed to parse shorthand interval '{}': {:?}",
            shorthand,
            result.err()
        );
    }
}
