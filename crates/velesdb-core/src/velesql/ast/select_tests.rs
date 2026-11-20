//! The base side of a SELECT — the FROM table — read among every alias in
//! scope (`from_alias` lists the FROM alias and each JOIN alias, BUG-8).

use crate::velesql::Parser;

fn select(sql: &str) -> super::SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

#[test]
fn an_unaliased_from_has_no_base_alias_though_a_join_is_aliased() {
    let stmt = select("SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id");
    assert_eq!(stmt.base_alias(), None);
    assert_eq!(stmt.base_side(), "docs");
}

#[test]
fn an_aliased_from_is_the_base_alias_beside_an_aliased_join() {
    let stmt = select("SELECT * FROM docs d JOIN tags t ON t.id = d.tag_id");
    assert_eq!(stmt.base_alias(), Some("d"));
    assert_eq!(stmt.base_side(), "d");
}

#[test]
fn an_aliased_from_is_the_base_alias_beside_an_unaliased_join() {
    let stmt = select("SELECT * FROM docs AS d JOIN tags ON tags.id = d.tag_id");
    assert_eq!(stmt.base_alias(), Some("d"));
}

#[test]
fn several_aliased_joins_never_name_the_unaliased_base() {
    let stmt = select(
        "SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id \
         JOIN users u ON u.id = docs.owner_id",
    );
    assert_eq!(stmt.base_alias(), None);
    assert_eq!(stmt.base_side(), "docs");
}

#[test]
fn an_aliased_from_stays_the_base_alias_across_several_joins() {
    let stmt = select(
        "SELECT * FROM docs AS d JOIN tags AS t ON t.id = d.tag_id \
         JOIN users AS u ON u.id = d.owner_id",
    );
    assert_eq!(stmt.base_alias(), Some("d"));
}

#[test]
fn a_single_aliased_table_is_its_own_base_alias() {
    let stmt = select("SELECT * FROM docs AS d");
    assert_eq!(stmt.base_alias(), Some("d"));
    assert_eq!(stmt.base_side(), "d");
}

#[test]
fn a_single_unaliased_table_is_read_under_its_name() {
    let stmt = select("SELECT * FROM docs");
    assert_eq!(stmt.base_alias(), None);
    assert_eq!(stmt.base_side(), "docs");
}

/// A JOIN declaring the FROM alias again is still a JOIN alias: the base
/// alias is the FROM's own, by position, not by name.
#[test]
fn a_join_reusing_the_from_alias_leaves_the_base_alias_in_place() {
    let stmt = select("SELECT * FROM docs AS x JOIN tags AS x ON x.id = x.tag_id");
    assert_eq!(stmt.base_alias(), Some("x"));
}
