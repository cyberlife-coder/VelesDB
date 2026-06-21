//! BDD-style end-to-end tests pinning `VelesQL` **NOT** semantics via the
//! COMPLEMENT PROPERTY: for every negated predicate, the NOT-set must equal
//! `universe - positive-set` (and we also assert the exact id set).
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL) -> THEN
//! (verify results). Tests exercise the full pipeline: SQL string ->
//! `Parser::parse()` -> `Database::execute_query()` -> verify `SearchResult`s.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate an `items` collection (dim 4) where EVERY point has all four
/// payload fields (`category`, `price`, `tags`, `name`), so the universe is
/// well-defined and `NOT (p)` partitions it cleanly into matches / non-matches.
///
/// Vectors use the `[1.0, off, 0.0, 0.0]` family for ids 1..=5 with `off`
/// strictly increasing (0.0, 0.3, 0.7, 1.2, 3.0) so cosine similarity to
/// `[1,0,0,0]` strictly DECREASES (1 > 2 > 3 > 4 > 5); ids 6..=8 are orthogonal
/// (cosine ~0) so they always rank last under NEAR.
///
/// | id | category    | price | tags                  | name    | vector            |
/// |----|-------------|-------|-----------------------|---------|-------------------|
/// | 1  | books       | 10    | ["new","sale"]        | apple   | [1.0, 0.0, 0, 0]  |
/// | 2  | books       | 20    | ["used"]              | apricot | [1.0, 0.3, 0, 0]  |
/// | 3  | electronics | 30    | ["new"]               | banana  | [1.0, 0.7, 0, 0]  |
/// | 4  | electronics | 40    | ["sale","clearance"]  | plum    | [1.0, 1.2, 0, 0]  |
/// | 5  | clothing    | 50    | ["new","clearance"]   | pear    | [1.0, 3.0, 0, 0]  |
/// | 6  | clothing    | 60    | ["used","vintage"]    | peach   | [0.0, 1.0, 0, 0]  |
/// | 7  | toys        | 70    | ["new"]               | grape   | [0.0, 0.0, 1, 0]  |
/// | 8  | toys        | 80    | ["sale"]              | mango   | [0.0, 0.0, 0, 1]  |
fn setup_items(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION items (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE items");

    let vc = db
        .get_vector_collection("items")
        .expect("test: get items collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"category": "books", "price": 10, "tags": ["new", "sale"], "name": "apple"}))),
        Point::new(2, vec![1.0, 0.3, 0.0, 0.0], Some(json!({"category": "books", "price": 20, "tags": ["used"], "name": "apricot"}))),
        Point::new(3, vec![1.0, 0.7, 0.0, 0.0], Some(json!({"category": "electronics", "price": 30, "tags": ["new"], "name": "banana"}))),
        Point::new(4, vec![1.0, 1.2, 0.0, 0.0], Some(json!({"category": "electronics", "price": 40, "tags": ["sale", "clearance"], "name": "plum"}))),
        Point::new(5, vec![1.0, 3.0, 0.0, 0.0], Some(json!({"category": "clothing", "price": 50, "tags": ["new", "clearance"], "name": "pear"}))),
        Point::new(6, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"category": "clothing", "price": 60, "tags": ["used", "vintage"], "name": "peach"}))),
        Point::new(7, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"category": "toys", "price": 70, "tags": ["new"], "name": "grape"}))),
        Point::new(8, vec![0.0, 0.0, 0.0, 1.0], Some(json!({"category": "toys", "price": 80, "tags": ["sale"], "name": "mango"}))),
    ])
    .expect("test: upsert items");
}

/// The full id universe of the `items` collection.
fn universe() -> HashSet<u64> {
    (1u64..=8).collect()
}

/// Run two SQL queries and return their id sets `(positive, negated)`.
fn run_pair(db: &Database, positive_sql: &str, negated_sql: &str) -> (HashSet<u64>, HashSet<u64>) {
    let pos = execute_sql(db, positive_sql).expect("test: positive query");
    let neg = execute_sql(db, negated_sql).expect("test: negated query");
    (result_ids(&pos), result_ids(&neg))
}

// =========================================================================
// Form 1: WHERE n NOT IN (...)
// =========================================================================

/// GIVEN items, WHEN `category NOT IN ('books','toys')`,
/// THEN positive set = `category IN ('books','toys')` = {1,2,7,8},
/// so NOT-set = universe - {1,2,7,8} = {3,4,5,6} (electronics+clothing).
#[test]
fn test_not_in_is_complement_of_in() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let (positive, negated) = run_pair(
        &db,
        "SELECT * FROM items WHERE category IN ('books', 'toys') LIMIT 10;",
        "SELECT * FROM items WHERE category NOT IN ('books', 'toys') LIMIT 10;",
    );

    assert_eq!(
        positive,
        HashSet::from([1, 2, 7, 8]),
        "IN matches books+toys"
    );
    assert_eq!(negated, HashSet::from([3, 4, 5, 6]), "NOT IN = the other 4");
    assert_eq!(
        &universe() - &positive,
        negated,
        "complement: NOT IN == universe minus IN"
    );
}

// =========================================================================
// Form 2: WHERE NOT (category = 'x')
// =========================================================================

/// GIVEN items, WHEN `NOT (category = 'electronics')`,
/// THEN positive `category = 'electronics'` = {3,4},
/// so NOT-set = universe - {3,4} = {1,2,5,6,7,8}.
#[test]
fn test_not_equality_is_complement() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let (positive, negated) = run_pair(
        &db,
        "SELECT * FROM items WHERE category = 'electronics' LIMIT 10;",
        "SELECT * FROM items WHERE NOT (category = 'electronics') LIMIT 10;",
    );

    assert_eq!(
        positive,
        HashSet::from([3, 4]),
        "equality matches electronics"
    );
    assert_eq!(
        negated,
        HashSet::from([1, 2, 5, 6, 7, 8]),
        "NOT equality = the other 6"
    );
    assert_eq!(
        &universe() - &positive,
        negated,
        "complement: NOT (=) == universe minus ="
    );
}

// =========================================================================
// Form 3: WHERE NOT (price BETWEEN a AND b)
// =========================================================================

/// GIVEN items, WHEN `NOT (price BETWEEN 30 AND 60)` (BETWEEN is inclusive),
/// THEN positive `price BETWEEN 30 AND 60` = {3,4,5,6} (prices 30,40,50,60),
/// so NOT-set = universe - {3,4,5,6} = {1,2,7,8} (prices 10,20,70,80).
#[test]
fn test_not_between_is_complement() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let (positive, negated) = run_pair(
        &db,
        "SELECT * FROM items WHERE price BETWEEN 30 AND 60 LIMIT 10;",
        "SELECT * FROM items WHERE NOT (price BETWEEN 30 AND 60) LIMIT 10;",
    );

    assert_eq!(
        positive,
        HashSet::from([3, 4, 5, 6]),
        "BETWEEN [30,60] inclusive"
    );
    assert_eq!(
        negated,
        HashSet::from([1, 2, 7, 8]),
        "NOT BETWEEN = outside range"
    );
    assert_eq!(
        &universe() - &positive,
        negated,
        "complement: NOT BETWEEN == universe minus BETWEEN"
    );
}

// =========================================================================
// Form 4: WHERE NOT (tags CONTAINS 'y')
// =========================================================================

/// GIVEN items where every point has a `tags` array, WHEN
/// `NOT (tags CONTAINS 'new')`, THEN positive `tags CONTAINS 'new'` =
/// {1,3,5,7} (tags containing "new"), so NOT-set = universe - {1,3,5,7} =
/// {2,4,6,8}. Every point has tags, so the complement is total.
#[test]
fn test_not_contains_is_complement() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let (positive, negated) = run_pair(
        &db,
        "SELECT * FROM items WHERE tags CONTAINS 'new' LIMIT 10;",
        "SELECT * FROM items WHERE NOT (tags CONTAINS 'new') LIMIT 10;",
    );

    assert_eq!(
        positive,
        HashSet::from([1, 3, 5, 7]),
        "tags containing 'new'"
    );
    assert_eq!(
        negated,
        HashSet::from([2, 4, 6, 8]),
        "NOT CONTAINS = no 'new' tag"
    );
    assert_eq!(
        &universe() - &positive,
        negated,
        "complement: NOT CONTAINS == universe minus CONTAINS"
    );
}

// =========================================================================
// Form 5: WHERE NOT (name LIKE 'p%')
// =========================================================================

/// GIVEN items, WHEN `NOT (name LIKE 'p%')` (prefix p), THEN positive
/// `name LIKE 'p%'` = {4,5,6} (plum, pear, peach), so NOT-set = universe -
/// {4,5,6} = {1,2,3,7,8} (apple, apricot, banana, grape, mango).
#[test]
fn test_not_like_prefix_is_complement() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let (positive, negated) = run_pair(
        &db,
        "SELECT * FROM items WHERE name LIKE 'p%' LIMIT 10;",
        "SELECT * FROM items WHERE NOT (name LIKE 'p%') LIMIT 10;",
    );

    assert_eq!(
        positive,
        HashSet::from([4, 5, 6]),
        "names starting with 'p'"
    );
    assert_eq!(
        negated,
        HashSet::from([1, 2, 3, 7, 8]),
        "NOT LIKE 'p%' = the rest"
    );
    assert_eq!(
        &universe() - &positive,
        negated,
        "complement: NOT LIKE == universe minus LIKE"
    );
}

// =========================================================================
// Form 6: De Morgan — NOT (A AND B) == (NOT A) OR (NOT B)
// =========================================================================

/// GIVEN items, WHEN `NOT (category = 'books' AND price > 15)` vs the De
/// Morgan equivalent `category != 'books' OR price <= 15`, THEN both yield
/// the SAME id set. Ground truth: `books AND price>15` = {2} (apricot, 20);
/// its complement over the universe is {1,3,4,5,6,7,8}. The OR form: NOT books
/// = {3,4,5,6,7,8}; price<=15 = {1}; union = {1,3,4,5,6,7,8}.
#[test]
fn test_not_de_morgan_and_equals_or_form() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let positive = execute_sql(
        &db,
        "SELECT * FROM items WHERE category = 'books' AND price > 15 LIMIT 10;",
    )
    .expect("test: positive AND query");
    let not_form = execute_sql(
        &db,
        "SELECT * FROM items WHERE NOT (category = 'books' AND price > 15) LIMIT 10;",
    )
    .expect("test: NOT(AND) query");
    let or_form = execute_sql(
        &db,
        "SELECT * FROM items WHERE category != 'books' OR price <= 15 LIMIT 10;",
    )
    .expect("test: De Morgan OR query");

    let positive = result_ids(&positive);
    let not_ids = result_ids(&not_form);
    assert_eq!(
        positive,
        HashSet::from([2]),
        "books AND price>15 = apricot(20)"
    );
    assert_eq!(
        not_ids,
        HashSet::from([1, 3, 4, 5, 6, 7, 8]),
        "NOT(AND) excludes only id 2"
    );
    assert_eq!(
        not_ids,
        result_ids(&or_form),
        "De Morgan: NOT(A AND B) == (NOT A) OR (NOT B)"
    );
    assert_eq!(
        &universe() - &positive,
        not_ids,
        "complement: NOT(AND) == universe minus (AND)"
    );
}

// =========================================================================
// Form 6b: De Morgan — NOT (A OR B) == (NOT A) AND (NOT B)
// =========================================================================

/// GIVEN items, WHEN `NOT (category = 'toys' OR price < 30)` vs the De Morgan
/// equivalent `category != 'toys' AND price >= 30`, THEN both yield the SAME
/// id set. Ground truth: `toys OR price<30` = toys{7,8} ∪ price<30{1,2} =
/// {1,2,7,8}; complement = {3,4,5,6}. The AND form: NOT toys = {1,2,3,4,5,6};
/// price>=30 = {3,4,5,6,7,8}; intersection = {3,4,5,6}.
#[test]
fn test_not_de_morgan_or_equals_and_form() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let positive = execute_sql(
        &db,
        "SELECT * FROM items WHERE category = 'toys' OR price < 30 LIMIT 10;",
    )
    .expect("test: positive OR query");
    let not_form = execute_sql(
        &db,
        "SELECT * FROM items WHERE NOT (category = 'toys' OR price < 30) LIMIT 10;",
    )
    .expect("test: NOT(OR) query");
    let and_form = execute_sql(
        &db,
        "SELECT * FROM items WHERE category != 'toys' AND price >= 30 LIMIT 10;",
    )
    .expect("test: De Morgan AND query");

    let positive = result_ids(&positive);
    let not_ids = result_ids(&not_form);
    assert_eq!(positive, HashSet::from([1, 2, 7, 8]), "toys OR price<30");
    assert_eq!(
        not_ids,
        HashSet::from([3, 4, 5, 6]),
        "NOT(OR) = electronics+clothing"
    );
    assert_eq!(
        not_ids,
        result_ids(&and_form),
        "De Morgan: NOT(A OR B) == (NOT A) AND (NOT B)"
    );
    assert_eq!(
        &universe() - &positive,
        not_ids,
        "complement: NOT(OR) == universe minus (OR)"
    );
}

// =========================================================================
// Form 7: NEAR + WHERE NOT (...) -> exact ordered ids
// =========================================================================

/// GIVEN items, WHEN `vector NEAR [1,0,0,0] AND NOT (category = 'books')
/// LIMIT 3`, THEN books {1,2} are excluded; among the remaining cosine-family
/// points {3,4,5} (vectors [1,off,...] with off 0.7,1.2,3.0) similarity
/// strictly DECREASES, so they rank 3 > 4 > 5 and all beat the orthogonal
/// (cosine ~0) points {6,7,8}. With LIMIT 3 the exact ORDERED result is
/// [3, 4, 5].
#[test]
fn test_near_with_not_filter_exact_ordered() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM items WHERE vector NEAR $v AND NOT (category = 'books') LIMIT 3;",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: NEAR + NOT filter");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![3, 4, 5],
        "NEAR ranks non-books cosine-family by decreasing similarity: 3 > 4 > 5"
    );
}

// =========================================================================
// Form 7b: NEAR + WHERE NOT(...) monotonic similarity scores
// =========================================================================

/// GIVEN items, WHEN `vector NEAR [1,0,0,0] AND NOT (price BETWEEN 25 AND 100)
/// LIMIT 2`, THEN positive `price BETWEEN 25 AND 100` = {3,4,5,6,7,8}; the
/// non-matching set is {1,2} (prices 10,20), both in the cosine family with
/// off 0.0 and 0.3, so similarity strictly DECREASES: ordered ids [1, 2] and
/// score[0] > score[1].
#[test]
fn test_near_with_not_between_monotonic_scores() {
    let (_dir, db) = create_test_db();
    setup_items(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM items WHERE vector NEAR $v AND NOT (price BETWEEN 25 AND 100) LIMIT 2;",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: NEAR + NOT BETWEEN");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![1, 2],
        "Only ids 1,2 escape the price range; off 0.0 < 0.3"
    );
    assert!(
        results[0].score > results[1].score,
        "cosine similarity strictly decreases: id1 (off 0.0) > id2 (off 0.3), got {} vs {}",
        results[0].score,
        results[1].score
    );
}
