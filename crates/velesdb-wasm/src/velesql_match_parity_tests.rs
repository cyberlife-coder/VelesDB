//! BDD scenarios: a browser MATCH answers what core's MATCH answers.
//!
//! Each scenario pins one behaviour the WASM walk used to decide its own way
//! (audit of the WASM executor against `velesdb-core`): every pattern label
//! and relationship type, relationship properties inline and in WHERE, an
//! alias repeated in the pattern, and `@collection` properties seen by WHERE
//! and the pattern.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

const ON_G: Option<&str> = Some(r#"{"_collection": "g"}"#);

/// `g`: 1 (A, B), 2 (A), 3 (A, B); edges 1 -R-> 2 (since 2020),
/// 2 -S-> 3 (since 2021), 3 -R-> 1 (since 2022).
fn graph(db: &mut DatabaseInner) {
    for (id, labels) in [(1, r#"["A","B"]"#), (2, r#"["A"]"#), (3, r#"["A","B"]"#)] {
        execute(
            db,
            &format!(
                "INSERT NODE INTO g (id = {id}, payload = '{{\"n\": {id}, \"_labels\": {labels}}}')"
            ),
            None,
        )
        .expect("test: node");
    }
    for (source, target, label, since) in [(1, 2, "R", 2020), (2, 3, "S", 2021), (3, 1, "R", 2022)]
    {
        execute(
            db,
            &format!(
                "INSERT EDGE INTO g (source = {source}, target = {target}, label = '{label}') \
                 WITH PROPERTIES (since = {since})"
            ),
            None,
        )
        .expect("test: edge");
    }
}

/// The `alias` id of every row, sorted.
fn ids(db: &mut DatabaseInner, sql: &str, alias: &str) -> Vec<u64> {
    let result = execute(db, sql, ON_G).expect("test: MATCH");
    let mut ids: Vec<u64> = result
        .rows_ref()
        .iter()
        .map(|row| {
            let value: serde_json::Value =
                serde_json::from_str(row.data_json_ref()).expect("test: row json");
            value[alias]["id"].as_u64().expect("test: alias id")
        })
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn every_label_of_a_pattern_node_is_required() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    assert_eq!(ids(&mut db, "MATCH (x:A:B) RETURN x", "x"), vec![1, 3]);
    assert_eq!(ids(&mut db, "MATCH (x:B:A) RETURN x", "x"), vec![1, 3]);
}

#[test]
fn any_listed_relationship_type_matches() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    assert_eq!(
        ids(&mut db, "MATCH (x)-[:R|S]->(y) RETURN x, y", "x"),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(&mut db, "MATCH (x)-[:S]->(y) RETURN x, y", "x"),
        vec![2]
    );
}

#[test]
fn relationship_properties_filter_inline_and_in_where() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    assert_eq!(
        ids(
            &mut db,
            "MATCH (x)-[:R {since: 2022}]->(y) RETURN x, y",
            "x"
        ),
        vec![3]
    );
    assert_eq!(
        ids(
            &mut db,
            "MATCH (x)-[r:R]->(y) WHERE r.since < 2021 RETURN x, y",
            "x"
        ),
        vec![1]
    );
}

#[test]
fn a_repeated_alias_requires_a_cycle() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    // 1 -R-> 2 -S-> 3 -R-> 1 is the only cycle of length 3.
    let rows = ids(
        &mut db,
        "MATCH (x)-[:R]->(y)-[:S]->(z)-[:R]->(x) RETURN x, y, z",
        "x",
    );
    assert_eq!(rows, vec![1]);
    assert!(ids(&mut db, "MATCH (x)-[:R]->(x) RETURN x", "x").is_empty());
}

#[test]
fn a_parameter_in_an_inline_property_takes_the_callers_value() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    let result = execute(
        &mut db,
        "MATCH (x {n: $which}) RETURN x",
        Some(r#"{"_collection": "g", "which": 2}"#),
    )
    .expect("test: MATCH");
    assert_eq!(result.row_count(), 1);
    let missing = execute(&mut db, "MATCH (x {n: $which}) RETURN x", ON_G)
        .expect_err("test: unbound parameter");
    assert!(missing.contains("$which"), "{missing}");
}

/// `pricing` (metadata): 2 -> price 5, 3 -> price 50. The annotation adds
/// properties seen by WHERE and the pattern alike; the node's own win.
fn priced(db: &mut DatabaseInner) {
    graph(db);
    db.create_metadata_collection("pricing")
        .expect("test: pricing");
    let store = db.get_shared_store("pricing").expect("test: store");
    for (id, payload) in [
        (2, serde_json::json!({"price": 5, "n": 99})),
        (3, serde_json::json!({"price": 50})),
    ] {
        crate::store_insert::insert_with_payload(&mut store.borrow_mut(), id, &[], Some(payload));
    }
}

#[test]
fn annotated_properties_are_seen_by_where_and_the_pattern() {
    let mut db = DatabaseInner::new();
    priced(&mut db);
    assert_eq!(
        ids(
            &mut db,
            "MATCH (x)-[]->(y@pricing) WHERE y.price < 10 RETURN x, y",
            "y"
        ),
        vec![2]
    );
    assert_eq!(
        ids(
            &mut db,
            "MATCH (x)-[]->(y@pricing {price: 50}) RETURN x, y",
            "y"
        ),
        vec![3]
    );
    // The node's own `n` (2) wins over the annotated point's (99).
    assert_eq!(
        ids(&mut db, "MATCH (y@pricing {n: 2}) RETURN y", "y"),
        vec![2]
    );
}

#[test]
fn an_annotation_naming_no_collection_is_refused() {
    let mut db = DatabaseInner::new();
    graph(&mut db);
    let err = execute(&mut db, "MATCH (x@nowhere) RETURN x", ON_G).expect_err("test: refused");
    assert!(err.contains("nowhere"), "{err}");
}

/// A bare WHERE column reads the target's OWN payload, as core reads it
/// (`bound_alias`: no alias, no annotation); `y.price` reads the annotation.
#[test]
fn a_bare_column_reads_the_targets_own_payload() {
    let mut db = DatabaseInner::new();
    priced(&mut db);
    assert!(
        ids(&mut db, "MATCH (y@pricing) WHERE price < 10 RETURN y", "y").is_empty(),
        "`price` is the annotation's, not the node's"
    );
    assert_eq!(
        ids(
            &mut db,
            "MATCH (y@pricing) WHERE y.price < 10 RETURN y",
            "y"
        ),
        vec![2]
    );
}

/// An alias reads the annotation any of its occurrences carries, as core
/// registers annotations by alias: in `(b@pricing)-…->(b)` the unannotated
/// last occurrence still reads `b`'s `price` from `pricing`.
#[test]
fn an_alias_reads_its_annotation_at_every_occurrence() {
    let mut db = DatabaseInner::new();
    priced(&mut db);
    // 2 -S-> 3 -R-> 1 -R-> 2 is the only cycle through `a` = 2, the priced one.
    assert_eq!(
        ids(
            &mut db,
            "MATCH (b@pricing)-[:S]->(c)-[:R]->(d)-[:R]->(b) WHERE b.price < 10 RETURN b",
            "b"
        ),
        vec![2]
    );
}
