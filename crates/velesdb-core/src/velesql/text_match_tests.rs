use super::*;
use crate::velesql::Value;

fn where_of(sql: &str) -> Condition {
    crate::velesql::Parser::parse(sql)
        .expect("test: parse")
        .select
        .where_clause
        .expect("test: a WHERE clause")
}

fn ranking(sql: &str) -> String {
    let condition = where_of(sql);
    match TextRanking::of(&condition) {
        TextRanking::None => "none".to_string(),
        TextRanking::Text(leaf) => format!("text {}", leaf.query),
        TextRanking::Hybrid(leaf) => format!("hybrid {}", leaf.query),
    }
}

#[test]
fn the_one_and_required_match_ranks_alone_or_fused_with_a_near() {
    for (sql, expected) in [
        ("SELECT * FROM d WHERE body MATCH 'x'", "text x"),
        (
            "SELECT * FROM d WHERE cat = 'a' AND (body MATCH 'x')",
            "text x",
        ),
        (
            "SELECT * FROM d WHERE body MATCH 'x' AND (title MATCH 'y' OR cat = 'a')",
            "text x",
        ),
        (
            "SELECT * FROM d WHERE vector NEAR $v AND body MATCH 'x'",
            "hybrid x",
        ),
        (
            "SELECT * FROM d WHERE (vector NEAR $v OR cat = 'a') AND body MATCH 'x'",
            "hybrid x",
        ),
    ] {
        assert_eq!(ranking(sql), expected, "{sql}");
    }
}

#[test]
fn no_match_ranks_beside_a_ranking_that_fuses_no_text() {
    for sql in [
        "SELECT * FROM d WHERE body MATCH 'x' OR cat = 'a'",
        "SELECT * FROM d WHERE NOT (body MATCH 'x')",
        "SELECT * FROM d WHERE body MATCH 'x' AND title MATCH 'y'",
        "SELECT * FROM d WHERE similarity(vector, $v) > 0.5 AND body MATCH 'x'",
        "SELECT * FROM d WHERE vector SPARSE_NEAR $s AND body MATCH 'x'",
        "SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] AND body MATCH 'x'",
        "SELECT * FROM d WHERE NOT (vector NEAR $v) AND body MATCH 'x'",
        "SELECT * FROM d WHERE cat = 'a'",
    ] {
        assert_eq!(ranking(sql), "none", "{sql}");
    }
}

fn hits(leaf: &MatchCondition) -> InCondition {
    let ids: Vec<u64> = if leaf.query == "x" {
        vec![3, 1]
    } else {
        vec![]
    };
    InCondition {
        column: "id".to_string(),
        values: ids.into_iter().map(Value::from).collect(),
        negated: false,
    }
}

fn bind(condition: &Condition, keep: Option<&MatchCondition>) -> Condition {
    bind_text_matches(condition, keep, &mut |leaf| {
        Ok::<_, std::convert::Infallible>(hits(leaf))
    })
    .unwrap_or_else(|never| match never {})
}

#[test]
fn binding_replaces_every_match_but_the_kept_one_by_its_hit_set() {
    let condition = where_of(
        "SELECT * FROM d WHERE body MATCH 'x' AND NOT (title MATCH 'y') AND (body MATCH 'x' OR cat = 'a')",
    );
    let TextRanking::Text(kept) = TextRanking::of(&condition) else {
        panic!("test: the first MATCH ranks");
    };

    let bound = bind(&condition, Some(kept));

    let mut left = Vec::new();
    and_required_text_matches(&bound, &mut left);
    assert_eq!(left.len(), 1, "the ranking stays: {bound:?}");
    let rendered = format!("{bound:?}");
    assert_eq!(rendered.matches("Match(").count(), 1, "{rendered}");
    assert_eq!(rendered.matches("In(").count(), 2, "{rendered}");
}

#[test]
fn binding_without_a_kept_match_leaves_none() {
    let condition = where_of("SELECT * FROM d WHERE body MATCH 'x' AND cat = 'a'");

    let bound = bind(&condition, None);

    assert!(!holds_text_match(&bound), "{bound:?}");
}

#[test]
fn a_refused_membership_fails_the_binding() {
    let condition = where_of("SELECT * FROM d WHERE NOT (body MATCH 'x')");

    let err = bind_text_matches(&condition, None, &mut |_| {
        Err::<InCondition, _>(crate::error::Error::Query("no text index".to_string()))
    })
    .expect_err("test: refused");

    assert!(err.to_string().contains("no text index"), "{err}");
}
