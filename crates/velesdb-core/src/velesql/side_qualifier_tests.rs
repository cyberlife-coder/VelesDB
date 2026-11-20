//! Tests for `side_qualifier`: the FROM side's name qualifies the base row's
//! own columns, and nothing else.

use super::{base_column, resolve_base_qualifiers};
use crate::velesql::Parser;

fn where_of(sql: &str) -> crate::velesql::Condition {
    Parser::parse(sql)
        .expect("test: parse")
        .select
        .where_clause
        .expect("test: where")
}

#[test]
fn only_the_base_name_qualifies_a_base_column() {
    assert_eq!(base_column("d.cat", "d"), Some("cat"));
    assert_eq!(base_column("d.meta.lang", "d"), Some("meta.lang"));
    assert_eq!(base_column("docs.cat", "d"), None);
    assert_eq!(base_column("dd.cat", "d"), None);
    assert_eq!(base_column("cat", "d"), None);
    assert_eq!(base_column("d.", "d"), None);
}

#[test]
fn every_leaf_column_the_base_qualifies_is_resolved() {
    let resolved = resolve_base_qualifiers(
        &where_of(
            "SELECT * FROM docs d WHERE (d.cat = 'y' OR t.tag IN (1, 2)) \
             AND NOT d.n BETWEEN 1 AND 3 AND d.title LIKE 'a%' AND d.x IS NULL \
             AND d.content MATCH 'apple' AND meta.lang = 'en'",
        ),
        "d",
    );

    let expected = where_of(
        "SELECT * FROM docs d WHERE (cat = 'y' OR t.tag IN (1, 2)) \
         AND NOT n BETWEEN 1 AND 3 AND title LIKE 'a%' AND x IS NULL \
         AND content MATCH 'apple' AND meta.lang = 'en'",
    );
    assert_eq!(resolved, expected);
}
