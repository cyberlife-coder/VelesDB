//! `SPARSE_NEAR` in the WASM executor against a real core `Database`: the
//! same points, dense and sparse, and the same statement on both engines
//! must return the same ids in the same order with the same scores.
//!
//! Host-only (`lib.rs`): the core side needs core's `persistence` feature.
//! The dense and sparse scores of the dataset are all distinct, so neither
//! engine's tie order decides a rank.

use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric as CoreMetric, Point};

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

/// One point of the dataset: `(id, vector, cat, sparse terms)`.
type Seed = (u64, [f32; 4], &'static str, &'static [(u32, f32)]);

/// Dense similarity to `$q` is 1 > 0.8 > 0.6 > 0.3 > 0; `{1: 1.0}` scores
/// 4 (2.0) > 3 (1.0) > 5 (0.5).
const POINTS: [Seed; 5] = [
    (1, [1.0, 0.0, 0.0, 0.0], "a", &[(2, 0.3)]),
    (2, [0.8, 0.6, 0.0, 0.0], "a", &[]),
    (3, [0.6, 0.0, 0.8, 0.0], "b", &[(1, 1.0)]),
    (4, [0.0, 0.6, 0.0, 0.8], "b", &[(1, 2.0), (2, 0.5)]),
    (5, [0.3, 0.0, 0.0, 0.954], "b", &[(1, 0.5)]),
];

const PARAMS: &str = r#"{"q": [1.0, 0.0, 0.0, 0.0], "s": {"1": 1.0}}"#;

fn wasm_db(metric: &str) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("c", 4, metric).expect("test: create");
    for (id, vector, cat, terms) in POINTS {
        execute(
            &mut db,
            &format!("INSERT INTO c (id, vector, cat) VALUES ({id}, $v, '{cat}')"),
            Some(&json!({ "v": vector }).to_string()),
        )
        .expect("test: insert");
        if !terms.is_empty() {
            let (indices, values): (Vec<u32>, Vec<f32>) = terms.iter().copied().unzip();
            db.get_shared_store("c")
                .expect("test: store")
                .borrow_mut()
                .sparse_insert(id, &indices, &values)
                .expect("test: sparse insert");
        }
    }
    db
}

fn core_db(metric: CoreMetric) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("c", 4, metric)
        .expect("test: create");
    let points = POINTS.iter().map(|(id, vector, cat, terms)| {
        let sparse = (!terms.is_empty())
            .then(|| BTreeMap::from([(String::new(), SparseVector::new(terms.to_vec()))]));
        Point::with_sparse(*id, vector.to_vec(), Some(json!({ "cat": cat })), sparse)
    });
    db.get_vector_collection("c")
        .expect("test: collection")
        .upsert(points)
        .expect("test: upsert");
    (dir, db)
}

fn wasm_rows(db: &mut DatabaseInner, sql: &str) -> Vec<(u64, f32)> {
    let result = execute(db, sql, Some(PARAMS)).unwrap_or_else(|e| panic!("wasm: {sql}: {e}"));
    (0..result.row_count() as usize)
        .map(|i| {
            let row = result.row(i).expect("test: row");
            (row.id(), row.score())
        })
        .collect()
}

fn core_rows(db: &Database, sql: &str) -> Vec<(u64, f32)> {
    let params: HashMap<String, serde_json::Value> =
        serde_json::from_str(PARAMS).expect("test: params");
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_query(&query, &params)
        .unwrap_or_else(|e| panic!("core: {sql}: {e}"))
        .iter()
        .map(|r| (r.point.id, r.score))
        .collect()
}

/// Runs `sql` on both engines over a collection of `metric` and asserts the
/// same rows, scores within 1e-5.
fn assert_parity(metric: (&str, CoreMetric), sql: &str) {
    let core = core_rows(&core_db(metric.1).1, sql);
    let wasm = wasm_rows(&mut wasm_db(metric.0), sql);
    assert!(!core.is_empty(), "core returned nothing for {sql}");
    let ids = |rows: &[(u64, f32)]| rows.iter().map(|r| r.0).collect::<Vec<_>>();
    assert_eq!(ids(&wasm), ids(&core), "ids for {sql}");
    for ((_, w), (id, c)) in wasm.iter().zip(&core) {
        assert!(
            (w - c).abs() < 1e-5,
            "score of {id} for {sql}: wasm {w}, core {c}"
        );
    }
}

const COSINE: (&str, CoreMetric) = ("cosine", CoreMetric::Cosine);
const EUCLIDEAN: (&str, CoreMetric) = ("euclidean", CoreMetric::Euclidean);

/// `NEAR` + `SPARSE_NEAR` is a dense + sparse fusion, RRF when no
/// `USING FUSION` names a strategy — the WASM executor used to drop the
/// sparse leaf and return the dense ranking alone.
#[test]
fn near_and_sparse_near_fuse_both_branches_as_core_does() {
    let sql = "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {1: 1.0} LIMIT 5";
    assert_parity(COSINE, sql);
    let ids: Vec<u64> = wasm_rows(&mut wasm_db("cosine"), sql)
        .iter()
        .map(|r| r.0)
        .collect();
    assert_ne!(ids, vec![1, 2, 3, 5, 4], "the dense ranking alone");
}

#[test]
fn the_metadata_residual_bounds_both_branches() {
    assert_parity(
        COSINE,
        "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR $s AND cat = 'b' LIMIT 5",
    );
}

#[test]
fn a_hybrid_window_keeps_cores_candidate_depth() {
    assert_parity(
        COSINE,
        "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {1: 1.0} LIMIT 2 OFFSET 1",
    );
}

/// Score-level strategies align the dense branch's distances with the sparse
/// branch's relevance (`fuse_with_directions`) before combining them.
#[test]
fn score_level_fusion_reads_a_distance_metric_in_its_direction() {
    for strategy in ["maximum", "average"] {
        assert_parity(
            EUCLIDEAN,
            &format!(
                "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {{1: 1.0}} \
                 LIMIT 5 USING FUSION (strategy = '{strategy}')"
            ),
        );
    }
}

#[test]
fn using_fusion_picks_the_strategy_core_maps() {
    for fusion in [
        "strategy = 'rrf', k = 10",
        "strategy = 'rsf', dense_w = 0.7, sparse_w = 0.3",
        "strategy = 'weighted_rrf', dense_w = 0.8, sparse_w = 0.2",
        "strategy = 'weighted', dense_w = 0.8, sparse_w = 0.2",
    ] {
        for metric in [COSINE, EUCLIDEAN] {
            assert_parity(
                metric,
                &format!(
                    "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {{1: 1.0}} \
                     LIMIT 5 USING FUSION ({fusion})"
                ),
            );
        }
    }
}

/// A lone `SPARSE_NEAR` ranks by the sparse score, a literal or a `$param`.
#[test]
fn a_lone_sparse_near_ranks_by_the_sparse_score() {
    assert_parity(
        COSINE,
        "SELECT * FROM c WHERE vector SPARSE_NEAR $s LIMIT 5",
    );
    assert_parity(
        COSINE,
        "SELECT * FROM c WHERE vector SPARSE_NEAR {1: 1.0, 2: 1.0} AND cat = 'b' LIMIT 1 OFFSET 1",
    );
}

/// The one refusal left is a genuinely missing index — a collection never
/// given a sparse vector, or an index name WASM does not keep — and it reads
/// as core's, error code included.
#[test]
fn a_missing_sparse_index_fails_with_cores_message() {
    let core_err = |db: &Database, sql: &str| {
        let query = Parser::parse(sql).expect("test: parse");
        db.execute_query(&query, &HashMap::new())
            .expect_err("core: missing index")
            .to_string()
    };
    let named = "SELECT * FROM c WHERE vector SPARSE_NEAR {1: 1.0} USING 'title' LIMIT 5";
    let wasm_named = execute(&mut wasm_db("cosine"), named, None).expect_err("wasm: named");
    assert_eq!(wasm_named, core_err(&core_db(CoreMetric::Cosine).1, named));

    let default = "SELECT * FROM d WHERE vector SPARSE_NEAR {1: 1.0} LIMIT 5";
    let mut wasm = DatabaseInner::new();
    wasm.create_collection("d", 4, "cosine").expect("test: create");
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let core = Database::open(dir.path()).expect("test: open");
    core.create_vector_collection("d", 4, CoreMetric::Cosine)
        .expect("test: create");
    let wasm_default = execute(&mut wasm, default, None).expect_err("wasm: default");
    assert_eq!(wasm_default, core_err(&core, default));
}
