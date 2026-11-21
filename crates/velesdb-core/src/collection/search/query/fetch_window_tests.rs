//! `ORDER BY` ranks every row the statement admits before `LIMIT` /
//! `OFFSET` cut it, on every route: a fetch may stop at the window only when
//! it already returns rows in the ORDER BY's order, past any tie at its cut.
//!
//! A fetch that was not ranked by that order — a metadata-filtered scan, a
//! graph's node scan, a `NEAR` read least similar first, a fusion whose
//! scores depend on how deep it reads — used to stop at `LIMIT + OFFSET` rows
//! and sort only those, so `ORDER BY similarity(vector, $q) ASC LIMIT 1`
//! returned a row that was not the least similar one.
//!
//! Each fixture holds 40 rows whose similarity to `$q` is a permutation of
//! their ids, so any prefix of the storage order sorts to a wrong window.

#![cfg(all(test, feature = "persistence"))]

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::velesql::Parser;
use crate::Database;

const ROWS: u64 = 40;

/// The rank of row `id`: its vector is `[1, rank / 10]`, so a smaller rank
/// is more similar to `[1, 0]` (closer, under a distance). `17 · id mod 41`
/// permutes `1..=40`, far from the id order.
fn rank(id: u64) -> u64 {
    (17 * id) % 41
}

/// Row `id`'s vector, `rank(id)` steps away from `[1, 0]`.
fn vector(id: u64) -> Value {
    #[allow(clippy::cast_precision_loss)]
    let y = rank(id) as f32 / 10.0;
    json!([1.0, y])
}

/// Row `id`'s named vector `emb`, `[rank / 10, 1]`: under cosine, a LARGER
/// rank is more similar to `[1, 0]` — the reverse of its own vector.
fn emb(id: u64) -> Value {
    #[allow(clippy::cast_precision_loss)]
    let x = rank(id) as f32 / 10.0;
    json!([x, 1.0])
}

/// Whether row `id` stores no `emb` (a vector collection) or no embedding
/// (a graph): its similarity is null.
fn unscored(id: u64) -> bool {
    id.is_multiple_of(5)
}

/// The similarity key of row `id` to `[1, 0]` on its own vector — larger is
/// more similar.
fn vector_key(id: u64) -> i64 {
    -i64::try_from(rank(id)).expect("test: rank")
}

/// The similarity key of row `id` to `[1, 0]` on `emb`, `None` when it
/// stores none.
fn by_emb(id: u64) -> Option<i64> {
    (!unscored(id)).then(|| i64::try_from(rank(id)).expect("test: rank"))
}

/// The similarity key of row `id` on its embedding in a graph whose
/// [`unscored`] nodes have none.
fn by_embedding(id: u64) -> Option<i64> {
    (!unscored(id)).then(|| vector_key(id))
}

fn query_params() -> HashMap<String, Value> {
    HashMap::from([
        ("q".to_string(), json!([1.0, 0.0])),
        ("far".to_string(), json!([0.0, 1.0])),
        ("s".to_string(), json!({"7": 1.0})),
        ("mid".to_string(), json!([1.0, 2.0])),
    ])
}

fn run(db: &Database, sql: &str, params: &HashMap<String, Value>) -> Vec<u64> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
    db.execute_query(&query, params)
        .unwrap_or_else(|e| panic!("test: {sql}: {e}"))
        .iter()
        .map(|r| r.point.id)
        .collect()
}

/// Row `id` of the 40-row fixture: `cat` alternating `'a'` (even ids) and
/// `'b'`, `emb` on all but the rows [`unscored`] names, a `content` text
/// BM25 ranks by rank (a shorter document first) and a sparse vector whose
/// weight falls as the rank grows.
fn fixture_row(id: u64) -> crate::point::Point {
    use std::collections::BTreeMap;

    use crate::index::sparse::SparseVector;

    let cat = if id.is_multiple_of(2) { "a" } else { "b" };
    let words = usize::try_from(rank(id)).expect("test: rank");
    let mut payload = json!({"cat": cat, "content": format!("banana{}", " filler".repeat(words))});
    if !unscored(id) {
        payload["emb"] = emb(id);
    }
    #[allow(clippy::cast_precision_loss)]
    let weight = (41 - rank(id)) as f32;
    let sparse = BTreeMap::from([(String::new(), SparseVector::new(vec![(7, weight)]))]);
    let vector = serde_json::from_value(vector(id)).expect("test: vector");
    crate::point::Point::with_sparse(id, vector, Some(payload), Some(sparse))
}

/// `docs` (cosine) and `edocs` (Euclidean): 40 rows each, `cat`
/// alternating `'a'` (even ids) and `'b'`, a named vector `emb` on all but
/// the rows [`unscored`] names, and a `content` text.
fn vector_collection() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    run(
        &db,
        "CREATE COLLECTION docs (dimension = 2)",
        &HashMap::new(),
    );
    run(
        &db,
        "CREATE COLLECTION edocs (dimension = 2, metric = 'euclidean')",
        &HashMap::new(),
    );
    for name in ["docs", "edocs"] {
        let collection = db.get_vector_collection(name).expect("test: collection");
        collection
            .upsert((1..=ROWS).map(fixture_row))
            .expect("test: upsert");
    }
    (dir, db)
}

/// `kg`: a graph of 40 nodes, each storing its vector as its embedding
/// except the payload-only ones [`unscored`] names.
fn graph_collection() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    run(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 2, metric = 'cosine')",
        &HashMap::new(),
    );
    for id in 1..=ROWS {
        if unscored(id) {
            let sql = format!("INSERT NODE INTO kg (id = {id}, payload = '{{\"name\":\"n\"}}')");
            run(&db, &sql, &HashMap::new());
        } else {
            let params = HashMap::from([("v".to_string(), vector(id))]);
            let sql = format!("INSERT INTO kg (id, vector, name) VALUES ({id}, $v, 'n')");
            run(&db, &sql, &params);
        }
    }
    (dir, db)
}

/// The rows `admitted` ordered by `key` — larger is more similar, `None` a
/// null key — in `direction`, a null key last under `ASC` and first under
/// `DESC`, ties by ascending id; then `OFFSET`, then `LIMIT`.
fn expected(
    admitted: impl Iterator<Item = u64>,
    key: impl Fn(u64) -> Option<i64>,
    descending: bool,
    offset: usize,
    limit: usize,
) -> Vec<u64> {
    let mut rows: Vec<u64> = admitted.collect();
    // A null key sorts after every value, ascending.
    let sort_key = |id: u64| (key(id).is_none(), key(id));
    rows.sort_by(|&a, &b| {
        let ascending = sort_key(a).cmp(&sort_key(b));
        let directed = if descending {
            ascending.reverse()
        } else {
            ascending
        };
        directed.then(a.cmp(&b))
    });
    rows.into_iter().skip(offset).take(limit).collect()
}

/// Every `(direction, OFFSET)` pair a window test runs.
const WINDOWS: [(&str, usize); 4] = [("ASC", 0), ("ASC", 2), ("DESC", 0), ("DESC", 2)];

/// `SELECT … {tail} ORDER BY {order} {dir} LIMIT 3 OFFSET {offset}` on `db`
/// against [`expected`] over `admitted`, for every window.
fn assert_windows(
    db: &Database,
    tail: &str,
    order: &str,
    admitted: &dyn Fn() -> Vec<u64>,
    key: &dyn Fn(u64) -> Option<i64>,
) {
    for (direction, offset) in WINDOWS {
        let sql = format!("SELECT * {tail} ORDER BY {order} {direction} LIMIT 3 OFFSET {offset}");
        let want = expected(admitted().into_iter(), key, direction == "DESC", offset, 3);
        assert_eq!(run(db, &sql, &query_params()), want, "{sql}");
    }
}

/// `{sql} LIMIT 3 OFFSET {o}` is the window of `{sql} LIMIT 40` — the
/// bounded read equals the unbounded one cut — for a few offsets.
fn assert_window_of_the_whole(db: &Database, sql: &str) {
    let whole = run(db, &format!("{sql} LIMIT 40"), &query_params());
    assert!(
        whole.len() > 5,
        "{sql}: the phenomenon needs rows past the window"
    );
    for offset in [0, 1, 4] {
        let window = run(
            db,
            &format!("{sql} LIMIT 3 OFFSET {offset}"),
            &query_params(),
        );
        let want: Vec<u64> = whole.iter().copied().skip(offset).take(3).collect();
        assert_eq!(window, want, "{sql} LIMIT 3 OFFSET {offset}");
    }
}

#[test]
fn a_metadata_filtered_scan_ranks_every_admitted_row() {
    let (_dir, db) = vector_collection();
    assert_windows(
        &db,
        "FROM docs WHERE cat = 'a'",
        "similarity(vector, $q)",
        &|| (1..=ROWS).filter(|id| id.is_multiple_of(2)).collect(),
        &|id| Some(vector_key(id)),
    );
}

#[test]
fn row_numbers_use_the_whole_partition_before_limit() {
    let (_dir, db) = vector_collection();
    for tail in [
        "",
        "WHERE vector NEAR $q",
        "WHERE vector NEAR $q ORDER BY similarity() DESC",
    ] {
        let sql = format!(
            "SELECT id, ROW_NUMBER() OVER (ORDER BY id DESC) AS rn FROM docs {tail} LIMIT 2"
        );
        let query = Parser::parse(&sql).expect("test: parse");
        let rows = db
            .execute_query(&query, &query_params())
            .expect("test: query");
        assert_eq!(rows.len(), 2, "{sql}");
        for row in rows {
            let rank = row.point.payload.as_ref().and_then(|p| p["rn"].as_u64());
            assert_eq!(rank, Some(ROWS + 1 - row.point.id), "{sql}");
        }
    }
}

#[test]
fn sparse_score_ties_use_ascending_id_at_the_fetch_boundary() {
    use crate::index::sparse::SparseVector;

    let (_dir, db) = vector_collection();
    let col = db.get_vector_collection("docs").expect("test: docs");
    let points = (1..=ROWS).rev().map(|id| {
        let mut row = fixture_row(id);
        row.sparse_vectors = Some(std::collections::BTreeMap::from([(
            String::new(),
            SparseVector::new(vec![(7, 1.0)]),
        )]));
        row
    });
    col.upsert(points).expect("test: tied sparse scores");
    let sql = "SELECT * FROM docs WHERE vector SPARSE_NEAR $s \
               ORDER BY similarity() DESC LIMIT 3 OFFSET 2";
    assert_eq!(run(&db, sql, &query_params()), [3, 4, 5]);
}

#[test]
fn a_scan_without_where_ranks_every_row() {
    let (_dir, db) = vector_collection();
    assert_windows(
        &db,
        "FROM docs",
        "similarity(vector, $q)",
        &|| (1..=ROWS).collect(),
        &|id| Some(vector_key(id)),
    );
}

/// A named vector some rows do not store: those rows have a null key.
#[test]
fn a_named_vector_ranks_rows_without_one_as_null_keys() {
    let (_dir, db) = vector_collection();
    assert_windows(
        &db,
        "FROM docs WHERE id > 0",
        "similarity(emb, $q)",
        &|| (1..=ROWS).collect(),
        &by_emb,
    );
}

/// A `NEAR` ranks its rows most similar first on its own vector: read least
/// similar first, by another vector or on another field, they are ranked
/// again over every row it admits.
#[test]
fn a_near_read_in_another_order_ranks_every_admitted_row() {
    let (_dir, db) = vector_collection();
    let near = "FROM docs WHERE vector NEAR $q";
    assert_windows(&db, near, "similarity()", &|| (1..=ROWS).collect(), &|id| {
        Some(vector_key(id))
    });
    assert_windows(
        &db,
        near,
        "similarity(emb, $q)",
        &|| (1..=ROWS).collect(),
        &by_emb,
    );
    // Most similar to `[0, 1]` is least similar to `[1, 0]`.
    let sql = format!("SELECT * {near} ORDER BY similarity(vector, $far) DESC LIMIT 3");
    let want = expected(1..=ROWS, |id| Some(vector_key(id)), false, 0, 3);
    assert_eq!(run(&db, &sql, &query_params()), want, "{sql}");
}

/// An arithmetic `ORDER BY` sorts its raw value, which `1 - similarity()`
/// reverses: read `DESC`, it is least similar first.
#[test]
fn an_arithmetic_order_over_a_near_ranks_every_admitted_row() {
    let (_dir, db) = vector_collection();
    let sql = "SELECT * FROM docs WHERE vector NEAR $q ORDER BY 1 - similarity() DESC LIMIT 3";
    let want = expected(1..=ROWS, |id| Some(vector_key(id)), false, 0, 3);
    assert_eq!(run(&db, sql, &query_params()), want, "{sql}");
}

/// A `similarity()` threshold ranks its rows most similar first: read least
/// similar first, they are ranked again over every row it admits.
#[test]
fn a_threshold_read_least_similar_first_ranks_every_admitted_row() {
    let (_dir, db) = vector_collection();
    assert_windows(
        &db,
        "FROM docs WHERE similarity(vector, $q) > 0.0",
        "similarity(vector, $q)",
        &|| (1..=ROWS).collect(),
        &|id| Some(vector_key(id)),
    );
}

/// A threshold under `OR` takes the exact scan, which puts unscored rows
/// last: under `DESC` they come first, under `ASC` the least similar.
#[test]
fn the_exact_similarity_scan_ranks_every_admitted_row() {
    let (_dir, db) = vector_collection();
    assert_windows(
        &db,
        "FROM docs WHERE cat = 'a' OR similarity(emb, $q) > 0.0",
        "similarity(emb, $q)",
        &|| {
            (1..=ROWS)
                .filter(|&id| id.is_multiple_of(2) || !unscored(id))
                .collect()
        },
        &by_emb,
    );
}

#[test]
fn a_graph_scan_ranks_every_node_with_payload_only_nodes_as_null_keys() {
    let (_dir, db) = graph_collection();
    for tail in ["FROM kg", "FROM kg WHERE name = 'n'"] {
        assert_windows(
            &db,
            tail,
            "similarity(vector, $q)",
            &|| (1..=ROWS).collect(),
            &by_embedding,
        );
    }
}

#[test]
fn a_match_ranks_every_row_before_its_limit() {
    let (_dir, db) = graph_collection();
    let mut params = query_params();
    params.insert("_collection".to_string(), json!("kg"));
    for direction in ["ASC", "DESC"] {
        let descending = direction == "DESC";
        let sql =
            format!("MATCH (a) RETURN a ORDER BY similarity(a.vector, $q) {direction} LIMIT 3");
        let want = expected(1..=ROWS, by_embedding, descending, 0, 3);
        assert_eq!(run(&db, &sql, &params), want, "{sql}");

        let sql = format!(
            "MATCH (a) WHERE similarity(a.vector, $q) > 0.0 RETURN a \
             ORDER BY similarity() {direction} LIMIT 3"
        );
        let scored = (1..=ROWS).filter(|&id| !unscored(id));
        let want = expected(scored, |id| Some(vector_key(id)), descending, 0, 3);
        assert_eq!(run(&db, &sql, &params), want, "{sql}");
    }
}

/// Under a distance, `similarity() DESC` reads the smallest score first: a
/// sparse search ranks its hits highest first, so its bounded fetch is not
/// that order, and the window is the one of every hit.
#[test]
fn an_own_score_read_against_its_order_ranks_every_hit() {
    let (_dir, db) = vector_collection();
    for collection in ["docs", "edocs"] {
        assert_window_of_the_whole(
            &db,
            &format!(
                "SELECT * FROM {collection} WHERE vector SPARSE_NEAR $s \
                 ORDER BY similarity() DESC"
            ),
        );
    }
}

/// A fusion's scores depend on how deep it reads each branch: its first
/// rows at `LIMIT 3` are not the first rows of a deeper read, so it reads
/// as deep whatever the window.
#[test]
fn a_fusion_ranks_every_row_it_admits() {
    let (_dir, db) = vector_collection();
    for collection in ["docs", "edocs"] {
        assert_window_of_the_whole(
            &db,
            &format!(
                "SELECT * FROM {collection} WHERE vector NEAR $mid AND content MATCH 'banana' \
                 ORDER BY similarity() DESC"
            ),
        );
    }
}

/// A text `MATCH` beside a threshold is a membership test: the threshold
/// ranks the rows, by its own vector.
#[test]
fn a_text_match_beside_a_threshold_ranks_by_the_threshold() {
    let (_dir, db) = vector_collection();
    for direction in ["ASC", "DESC"] {
        let sql = format!(
            "SELECT * FROM docs WHERE content MATCH 'banana' AND similarity(vector, $q) > 0.0 \
             ORDER BY similarity(vector, $q) {direction} LIMIT 3"
        );
        let want = expected(
            1..=ROWS,
            |id| Some(vector_key(id)),
            direction == "DESC",
            0,
            3,
        );
        assert_eq!(run(&db, &sql, &query_params()), want, "{sql}");
    }
}

/// DISTINCT reshapes the rows before the window: it reads every row, not
/// the two nearest, which share a category.
#[test]
fn distinct_reads_past_its_window() {
    let (_dir, db) = vector_collection();
    let sql = "SELECT DISTINCT cat FROM docs WHERE vector NEAR $q LIMIT 2";
    assert_eq!(run(&db, sql, &query_params()).len(), 2, "{sql}");
}

/// `ties`: 40 rows, inserted from id 40 down, in buckets of equal
/// similarity — `rank / 4` — each row's `cat` unrelated to its id.
fn tied_collection() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    run(
        &db,
        "CREATE COLLECTION ties (dimension = 2)",
        &HashMap::new(),
    );
    for id in (1..=ROWS).rev() {
        #[allow(clippy::cast_precision_loss)]
        let y = (rank(id) / 4) as f32 / 10.0;
        let params = HashMap::from([("v".to_string(), json!([1.0, y]))]);
        let cat = format!("c{}", (id * 3) % 7);
        let sql = format!("INSERT INTO ties (id, vector, cat) VALUES ({id}, $v, '{cat}')");
        run(&db, &sql, &params);
    }
    (dir, db)
}

/// Rows tied at the window's cut take their place by ascending id, or by a
/// later ORDER BY key: the bounded read reads past the tie.
#[test]
fn rows_tied_at_the_cut_take_their_place_by_the_tie_rule() {
    let (_dir, db) = tied_collection();
    let bucket = |id: u64| -i64::try_from(rank(id) / 4).expect("test: bucket");
    let cat = |id: u64| (id * 3) % 7;
    for offset in [0, 1, 2, 5] {
        for tail in [
            "WHERE vector NEAR $q ORDER BY similarity()",
            "WHERE similarity(vector, $q) > 0.0 ORDER BY similarity(vector, $q)",
        ] {
            let sql = format!("SELECT * FROM ties {tail} DESC LIMIT 3 OFFSET {offset}");
            let want = expected(1..=ROWS, |id| Some(bucket(id)), true, offset, 3);
            assert_eq!(run(&db, &sql, &query_params()), want, "{sql}");
        }
        let sql = format!(
            "SELECT * FROM ties WHERE vector NEAR $q ORDER BY similarity() DESC, cat ASC \
             LIMIT 3 OFFSET {offset}"
        );
        let mut all: Vec<u64> = (1..=ROWS).collect();
        all.sort_by_key(|&id| (std::cmp::Reverse(bucket(id)), cat(id), id));
        let want: Vec<u64> = all.into_iter().skip(offset).take(3).collect();
        assert_eq!(run(&db, &sql, &query_params()), want, "{sql}");
    }
}

#[path = "fetch_window_plan_tests.rs"]
mod plans;

#[path = "fetch_window_warning_tests.rs"]
mod warnings;
