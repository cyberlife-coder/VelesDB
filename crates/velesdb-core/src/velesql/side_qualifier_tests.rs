//! Tests for `side_qualifier`: the FROM side's name qualifies the base row's
//! own columns, and nothing else.

use super::{base_alias, base_column, base_side, resolve_base_qualifiers};
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

fn select(sql: &str) -> crate::velesql::SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

#[test]
fn an_unaliased_from_has_no_base_alias_though_a_join_is_aliased() {
    let stmt = select("SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id");
    assert_eq!(base_alias(&stmt), None);
    assert_eq!(base_side(&stmt), "docs");
}

#[test]
fn an_aliased_from_is_the_base_alias_beside_an_aliased_join() {
    let stmt = select("SELECT * FROM docs d JOIN tags t ON t.id = d.tag_id");
    assert_eq!(base_alias(&stmt), Some("d"));
    assert_eq!(base_side(&stmt), "d");
}

#[test]
fn an_aliased_from_is_the_base_alias_beside_an_unaliased_join() {
    let stmt = select("SELECT * FROM docs AS d JOIN tags ON tags.id = d.tag_id");
    assert_eq!(base_alias(&stmt), Some("d"));
}

#[test]
fn several_aliased_joins_never_name_the_unaliased_base() {
    let stmt = select(
        "SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id \
         JOIN users u ON u.id = docs.owner_id",
    );
    assert_eq!(base_alias(&stmt), None);
    assert_eq!(base_side(&stmt), "docs");
}

#[test]
fn an_aliased_from_stays_the_base_alias_across_several_joins() {
    let stmt = select(
        "SELECT * FROM docs AS d JOIN tags AS t ON t.id = d.tag_id \
         JOIN users AS u ON u.id = d.owner_id",
    );
    assert_eq!(base_alias(&stmt), Some("d"));
}

#[test]
fn a_single_table_is_read_under_its_alias_or_its_name() {
    let aliased = select("SELECT * FROM docs AS d");
    assert_eq!(base_alias(&aliased), Some("d"));
    assert_eq!(base_side(&aliased), "d");

    let bare = select("SELECT * FROM docs");
    assert_eq!(base_alias(&bare), None);
    assert_eq!(base_side(&bare), "docs");
}
