#![cfg(feature = "persistence")]
//! Cross-binding search fidelity (issue #2095 item C) — the core leg.
//!
//! Loads `conformance/search_fidelity_cases.json`, seeds its dataset through
//! `Database`/`VectorCollection`, and asserts that `search` and
//! `hybrid_search` reproduce the fixture's pinned ids AND scores exactly.
//! This is the canonical run: the REST and WASM legs
//! (`velesdb-server/tests/search_fidelity_conformance.rs`,
//! `velesdb-wasm/src/search_fidelity_conformance_tests.rs`) must match it —
//! the WASM leg reads its `surface_expect` override where
//! `expected_divergences` documents a semantic gap.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point, VectorCollection};

#[derive(Deserialize)]
struct Dataset {
    collection: String,
    dimension: usize,
    metric: String,
    points: Vec<FixturePoint>,
}

#[derive(Deserialize)]
struct FixturePoint {
    id: u64,
    vector: Vec<f32>,
    payload: serde_json::Value,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SurfaceExpect {
    expect_ids: Vec<u64>,
    expect_scores: Vec<f32>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    kind: String,
    collection: String,
    vector: String,
    #[serde(default)]
    text: Option<String>,
    top_k: usize,
    #[serde(default)]
    vector_weight: Option<f32>,
    #[serde(default)]
    rrf_k: Option<u32>,
    expect_ids: Vec<u64>,
    expect_scores: Vec<f32>,
    #[serde(default)]
    #[allow(dead_code)]
    surface_expect: HashMap<String, SurfaceExpect>,
}

#[derive(Deserialize)]
struct Divergence {
    #[serde(rename = "case")]
    case_id: String,
    surface: String,
    #[allow(dead_code)]
    issue: String,
    #[allow(dead_code)]
    reason: String,
}

#[derive(Deserialize)]
struct Fixture {
    dataset: Dataset,
    #[serde(default)]
    extra_collections: Vec<Dataset>,
    query_vectors: HashMap<String, Vec<f32>>,
    cases: Vec<Case>,
    #[serde(default)]
    expected_divergences: Vec<Divergence>,
    score_tolerance: f32,
}

fn load_fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/search_fidelity_cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read search fidelity fixture"))
        .expect("parse search fidelity fixture")
}

fn seed_collection(db: &Database, ds: &Dataset) {
    let metric = ds.metric.parse().unwrap_or(DistanceMetric::Cosine);
    db.create_collection(&ds.collection, ds.dimension, metric)
        .expect("create fidelity collection");
    let collection = db
        .get_vector_collection(&ds.collection)
        .expect("get fidelity collection");
    let points: Vec<Point> = ds
        .points
        .iter()
        .map(|p| Point::new(p.id, p.vector.clone(), Some(p.payload.clone())))
        .collect();
    collection.upsert(points).expect("upsert fidelity points");
}

fn run_case(collection: &VectorCollection, case: &Case, query: &[f32]) -> Vec<(u64, f32)> {
    let results = match case.kind.as_str() {
        "near" => collection
            .search(query, case.top_k)
            .expect("core near search"),
        "hybrid_rrf" => collection
            .hybrid_search_with_rrf(
                query,
                case.text.as_deref().expect("hybrid case text"),
                case.top_k,
                case.vector_weight,
                case.rrf_k,
            )
            .expect("core hybrid search"),
        other => panic!("unknown case kind {other:?}"),
    };
    results.iter().map(|r| (r.point.id, r.score)).collect()
}

fn assert_golden(case: &Case, actual: &[(u64, f32)], tolerance: f32) {
    let ids: Vec<u64> = actual.iter().map(|r| r.0).collect();
    assert_eq!(ids, case.expect_ids, "case {}: ids differ", case.id);
    for ((id, got), want) in actual.iter().zip(&case.expect_scores) {
        assert!(
            (got - want).abs() <= tolerance,
            "case {}: score of id {id} differs: got {got}, want {want}",
            case.id
        );
    }
}

#[test]
fn test_search_fidelity_fixture_cases() {
    let fixture = load_fixture();
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    seed_collection(&db, &fixture.dataset);
    for ds in &fixture.extra_collections {
        seed_collection(&db, ds);
    }
    for case in &fixture.cases {
        let collection = db
            .get_vector_collection(&case.collection)
            .expect("get case collection");
        let query = &fixture.query_vectors[&case.vector];
        let actual = run_case(&collection, case, query);
        assert_golden(case, &actual, fixture.score_tolerance);
    }
}

/// The canonical run must not be allowlisted for its own divergences:
/// `expected_divergences` documents OTHER surfaces' gaps, and a `core` or
/// `rest` entry would mean the golden itself is wrong.
#[test]
fn test_no_divergence_allowlisted_for_canonical_surfaces() {
    let fixture = load_fixture();
    for d in &fixture.expected_divergences {
        assert!(
            d.surface != "core" && d.surface != "rest",
            "divergence {} allowlists canonical surface {}",
            d.case_id,
            d.surface
        );
    }
}
