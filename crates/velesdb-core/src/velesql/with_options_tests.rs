//! Tests for WITH clause options (EPIC-040 US-004).
//!
//! Covers:
//! - WITH(max_groups=N) for GROUP BY limit
//! - Parsing and execution of max_groups option

use crate::velesql::ast::WithValue;
use crate::velesql::Parser;

#[test]
fn test_with_max_groups_parsing() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (max_groups = 100)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH max_groups: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    // Find max_groups option
    let max_groups = with_clause
        .options
        .iter()
        .find(|opt| opt.key == "max_groups")
        .expect("max_groups option should be present");

    assert_eq!(
        max_groups.value,
        WithValue::Integer(100),
        "max_groups value must parse as Integer(100)"
    );
}

#[test]
fn test_with_multiple_options() {
    let sql = "SELECT * FROM docs WITH (max_groups = 500, timeout_ms = 1000)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH multiple options: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    assert_eq!(with_clause.options.len(), 2);
    assert_eq!(with_clause.options[0].key, "max_groups");
    assert_eq!(with_clause.options[0].value, WithValue::Integer(500));
    assert_eq!(with_clause.options[1].key, "timeout_ms");
    assert_eq!(with_clause.options[1].value, WithValue::Integer(1000));
}

#[test]
fn test_with_group_limit_option() {
    // Alternative name: group_limit instead of max_groups
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (group_limit = 50)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH group_limit: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    let group_limit = with_clause
        .options
        .iter()
        .find(|opt| opt.key == "group_limit")
        .expect("group_limit option should be present");

    assert_eq!(
        group_limit.value,
        WithValue::Integer(50),
        "group_limit value must be parsed as Integer(50)"
    );
}

// ============================================================================
// WithValue's Display: canonical VelesQL the parser reads back
// ============================================================================

/// Parses `shown` as the value of a `WITH` option, as a query would carry it.
fn parse_back(shown: &str) -> Result<WithValue, String> {
    let sql = format!("SELECT * FROM docs LIMIT 5 WITH (opt = {shown})");
    let query = Parser::parse(&sql).map_err(|e| format!("{sql}: {e:?}"))?;
    let mut with = query
        .select
        .with_clause
        .ok_or_else(|| format!("{sql}: no WITH clause"))?;
    match with.options.as_mut_slice() {
        [only] => Ok(only.value.clone()),
        options => Err(format!("{sql}: {} options", options.len())),
    }
}

/// Whether `shown` reads back as exactly `value`: a float bit for bit, so a
/// lost sign on `-0.0`, which `PartialEq` equates with `0.0`, is a failure.
fn reads_back_as(shown: &str, value: &WithValue) -> Result<(), String> {
    let back = parse_back(shown)?;
    let same = match (&back, value) {
        (WithValue::Float(back), WithValue::Float(v)) => back.to_bits() == v.to_bits(),
        _ => back == *value,
    };
    if same {
        Ok(())
    } else {
        Err(format!("{shown} read back as {back:?}, not {value:?}"))
    }
}

/// Asserts that `value` renders as a text the parser reads back as `value`.
fn assert_round_trips(value: &WithValue) {
    let shown = value.to_string();
    assert_eq!(reads_back_as(&shown, value), Ok(()));
}

/// `NaN` has no `VelesQL` form: its display must not parse at all, rather
/// than read back as another value (a bare `NaN` reads back as an identifier).
#[test]
fn test_with_value_display_of_nan_does_not_parse() {
    let shown = WithValue::Float(f64::NAN).to_string();
    let back = parse_back(&shown);
    assert!(back.is_err(), "{shown} read back as {back:?}");
}

#[test]
fn test_with_value_display_reads_back_as_the_same_value() {
    let identifier = |s: &str| WithValue::Identifier(s.to_string());
    let string = |s: &str| WithValue::String(s.to_string());
    for value in [
        WithValue::Integer(0),
        WithValue::Integer(4097),
        WithValue::Integer(-1),
        WithValue::Integer(i64::MIN),
        WithValue::Integer(i64::MAX),
        WithValue::Float(1.5),
        WithValue::Float(-1.5),
        WithValue::Float(100.0),
        WithValue::Float(-0.0),
        WithValue::Float(1e20),
        WithValue::Float(-1e300),
        WithValue::Float(f64::MAX),
        WithValue::Float(1e-20),
        WithValue::Float(f64::MIN_POSITIVE),
        WithValue::Float(5e-324),
        WithValue::Float(f64::INFINITY),
        WithValue::Float(f64::NEG_INFINITY),
        WithValue::Boolean(true),
        WithValue::Boolean(false),
        string(""),
        string("high"),
        string("it's"),
        string("''"),
        string("a \"b\" \\n"),
        identifier("high"),
        identifier("_x9"),
        identifier("TRUE"),
        identifier("false"),
        identifier("true_x"),
        identifier("my option"),
        identifier("9lives"),
        identifier("say \"hi\""),
        identifier("-1"),
        identifier("1.5"),
    ] {
        assert_round_trips(&value);
    }
}

/// The forms the review of #2276 found unreadable or mistaken for another
/// value, pinned to their canonical text.
#[test]
fn test_with_value_display_writes_the_canonical_text() {
    for (value, shown) in [
        (WithValue::Float(1e20), "100000000000000000000.0"),
        (WithValue::Float(1.50), "1.5"),
        (WithValue::Float(100.0), "100.0"),
        (WithValue::Float(1e-7), "0.0000001"),
        (WithValue::String("it's".to_string()), "'it''s'"),
        (WithValue::Identifier("high".to_string()), "high"),
        (WithValue::Identifier("TRUE".to_string()), "\"TRUE\""),
        (WithValue::Identifier("a\"b".to_string()), "\"a\"\"b\""),
        (WithValue::Boolean(true), "true"),
    ] {
        assert_eq!(value.to_string(), shown);
    }
}

proptest::proptest! {
    #[test]
    fn prop_a_finite_float_reads_back(
        v in proptest::num::f64::POSITIVE
            | proptest::num::f64::NEGATIVE
            | proptest::num::f64::NORMAL
            | proptest::num::f64::SUBNORMAL
            | proptest::num::f64::ZERO
    ) {
        let value = WithValue::Float(v);
        let shown = value.to_string();
        proptest::prop_assert!(!shown.contains(['e', 'E']), "{shown}");
        proptest::prop_assert_eq!(reads_back_as(&shown, &value), Ok(()));
    }

    #[test]
    fn prop_an_integer_reads_back(v in proptest::num::i64::ANY) {
        let value = WithValue::Integer(v);
        proptest::prop_assert_eq!(parse_back(&value.to_string()), Ok(value));
    }

    #[test]
    fn prop_a_string_reads_back(s in "\\PC*") {
        let value = WithValue::String(s);
        proptest::prop_assert_eq!(parse_back(&value.to_string()), Ok(value));
    }

    #[test]
    fn prop_an_identifier_reads_back(s in "\\PC+") {
        let value = WithValue::Identifier(s);
        proptest::prop_assert_eq!(parse_back(&value.to_string()), Ok(value));
    }
}
