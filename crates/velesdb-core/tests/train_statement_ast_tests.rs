#![cfg(feature = "persistence")]
//! Building a `TRAIN QUANTIZER` statement, versus printing one and parsing it back.
//!
//! A collection name and a VelesQL *bare identifier* are not the same alphabet.
//! `validation::is_valid_name_char` accepts `-` — `validate_collection_name`
//! pins `a-b` and the doc example is `docs-v2` — while `grammar.pest`'s
//! `regular_identifier` is `(ASCII_ALPHA | "_") ~ (ASCII_ALPHANUMERIC | "_")*`
//! and cannot spell one.
//!
//! A binding that renders `format!("TRAIN QUANTIZER ON {name} ...")` therefore
//! has to invent its own name rule, and that rule is a second definition which
//! drifts. The AST route emits no identifier text at all, so the question does
//! not arise; `velesdb-mobile` and `tauri-plugin-velesdb` already take it, and
//! the Python binding now does too.

#![allow(clippy::cast_precision_loss, clippy::doc_markdown)]

use std::collections::HashMap;

use tempfile::TempDir;
use velesdb_core::velesql::{Parser, Query, TrainStatement, WithValue};
use velesdb_core::{Database, DistanceMetric, Point};

const HYPHENATED: &str = "docs-v2";
const DIM: usize = 8;

fn seeded_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_collection(HYPHENATED, DIM, DistanceMetric::Cosine)
        .expect("test: core accepts a hyphenated collection name");

    let coll = db
        .get_vector_collection(HYPHENATED)
        .expect("test: collection must exist");
    let points: Vec<Point> = (0..64_u64)
        .map(|i| {
            let v: Vec<f32> = (0..DIM)
                .map(|d| ((i as f32) * 0.37 + (d as f32) * 1.13).sin())
                .collect();
            Point::without_payload(i, v)
        })
        .collect();
    coll.upsert(points).expect("test: upsert");

    (dir, db)
}

fn train_params() -> HashMap<String, WithValue> {
    let mut params = HashMap::new();
    params.insert("m".to_string(), WithValue::Integer(2));
    params.insert("k".to_string(), WithValue::Integer(4));
    params
}

/// The name is legal for a collection but cannot be a bare identifier.
///
/// This is the gap a string-building binding falls into: it must either
/// reject names core accepts, or quote and escape them — reintroducing the
/// injection surface the quoting was meant to close.
#[test]
fn a_hyphenated_collection_name_is_legal_but_unspellable_as_a_bare_identifier() {
    let rendered = format!("TRAIN QUANTIZER ON {HYPHENATED} WITH (m=2, k=4)");
    assert!(
        Parser::parse(&rendered).is_err(),
        "`{rendered}` must not parse: `regular_identifier` has no hyphen"
    );
}

/// The AST route reaches the collection the text route cannot name.
#[test]
fn training_through_the_ast_reaches_a_hyphenated_collection() {
    let (_dir, db) = seeded_db();

    let query = Query::new_train(TrainStatement {
        collection: HYPHENATED.to_string(),
        params: train_params(),
    });

    db.execute_query(&query, &HashMap::new())
        .expect("test: the AST route must train a hyphenated collection");
}

/// The AST route carries no charset rule of its own, so a name core rejects
/// fails in core with core's error rather than in a binding's private guard.
#[test]
fn a_name_core_rejects_fails_in_core_not_in_a_private_guard() {
    let (_dir, db) = seeded_db();

    let query = Query::new_train(TrainStatement {
        collection: "no-such-collection".to_string(),
        params: train_params(),
    });

    let err = db
        .execute_query(&query, &HashMap::new())
        .expect_err("training an absent collection must fail");
    let message = err.to_string();
    assert!(
        message.to_lowercase().contains("collection"),
        "the error must come from core and name the collection, got: {message}"
    );
}

/// An empty collection name is refused rather than rendering a malformed query.
///
/// The string route accepted it — `.all()` is vacuously true on an empty
/// iterator — and produced `TRAIN QUANTIZER ON  WITH (...)`.
#[test]
fn an_empty_collection_name_is_refused() {
    let (_dir, db) = seeded_db();

    let query = Query::new_train(TrainStatement {
        collection: String::new(),
        params: train_params(),
    });

    assert!(
        db.execute_query(&query, &HashMap::new()).is_err(),
        "an empty collection name must be refused"
    );
}
