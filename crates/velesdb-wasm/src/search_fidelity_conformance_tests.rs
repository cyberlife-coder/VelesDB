//! Cross-binding search fidelity (issue #2095 item C) — the WASM leg.
//!
//! Loads the same `conformance/search_fidelity_cases.json` as the core and
//! REST legs, seeds its dataset into a `VectorStore` (the same store the JS
//! binding wraps), and asserts ids AND scores. Where `expected_divergences`
//! lists this surface — the `hybrid_rrf` case: WASM's `hybrid_search` is a
//! raw-score linear blend over a binary substring match with no `rrf_k`,
//! versus core's BM25+RRF fusion (#2095-A7) — the case's `surface_expect`
//! override is asserted instead of the canonical golden, so the divergence
//! is pinned in both directions: it fails if the WASM algorithm drifts, and
//! it fails when a fix makes it match core (forcing the override out).
//!
//! Native `#[test]`, not `wasm_bindgen_test`: the assertions exercise the
//! `*_scored` seams because `JsValue` needs the wasm32 runtime.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::store_insert::insert_with_payload;
use crate::store_new::create_store;
use crate::{DistanceMetric, StorageMode, VectorStore};

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
    expect_ids: Vec<u64>,
    expect_scores: Vec<f32>,
    #[serde(default)]
    surface_expect: HashMap<String, SurfaceExpect>,
}

#[derive(Deserialize)]
struct Divergence {
    #[serde(rename = "case")]
    case_id: String,
    surface: String,
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
    serde_json::from_str(&std::fs::read_to_string(path).expect("read fidelity fixture"))
        .expect("parse fidelity fixture")
}

fn build_store(ds: &Dataset) -> VectorStore {
    let metric: DistanceMetric = ds
        .metric
        .parse()
        .unwrap_or_else(|_| panic!("test: fixture metric {}", ds.metric));
    let mut store = create_store(ds.dimension, metric, StorageMode::Full);
    for p in &ds.points {
        insert_with_payload(&mut store, p.id, &p.vector, Some(p.payload.clone()));
    }
    store
}

fn run_case(store: &VectorStore, case: &Case, query: &[f32]) -> Vec<(u64, f32)> {
    match case.kind.as_str() {
        "near" => store.search_scored(query, case.top_k),
        "hybrid_rrf" => store.hybrid_search_scored(
            query,
            case.text.as_deref().expect("hybrid case text"),
            case.top_k,
            case.vector_weight,
        ),
        other => panic!("unknown case kind {other:?}"),
    }
}

fn assert_scores(case_id: &str, actual: &[(u64, f32)], expect: &SurfaceExpect, tol: f32) {
    let ids: Vec<u64> = actual.iter().map(|r| r.0).collect();
    assert_eq!(ids, expect.expect_ids, "case {case_id}: ids differ");
    for ((id, got), want) in actual.iter().zip(&expect.expect_scores) {
        assert!(
            (got - want).abs() <= tol,
            "case {case_id}: score of id {id} differs: got {got}, want {want}"
        );
    }
}

/// Every `expected_divergences` entry aimed at `wasm` must carry a
/// `surface_expect.wasm` override — an allowlisted case without the pinned
/// actual output would pass untested.
#[test]
fn test_every_wasm_divergence_is_pinned() {
    let fixture = load_fixture();
    let cases: HashMap<&str, &Case> = fixture.cases.iter().map(|c| (c.id.as_str(), c)).collect();
    for d in &fixture.expected_divergences {
        if d.surface != "wasm" {
            continue;
        }
        let case = cases[d.case_id.as_str()];
        assert!(
            case.surface_expect.contains_key("wasm"),
            "divergence {} lacks a surface_expect.wasm golden",
            d.case_id
        );
    }
}

#[test]
fn test_search_fidelity_fixture_cases() {
    let fixture = load_fixture();
    let mut stores: HashMap<String, VectorStore> = HashMap::new();
    stores.insert(
        fixture.dataset.collection.clone(),
        build_store(&fixture.dataset),
    );
    for ds in &fixture.extra_collections {
        stores.insert(ds.collection.clone(), build_store(ds));
    }
    for case in &fixture.cases {
        let store = &stores[&case.collection];
        let query = &fixture.query_vectors[&case.vector];
        let actual = run_case(store, case, query);
        let expect = case.surface_expect.get("wasm").map_or_else(
            || SurfaceExpect {
                expect_ids: case.expect_ids.clone(),
                expect_scores: case.expect_scores.clone(),
            },
            |e| SurfaceExpect {
                expect_ids: e.expect_ids.clone(),
                expect_scores: e.expect_scores.clone(),
            },
        );
        assert_scores(&case.id, &actual, &expect, fixture.score_tolerance);
    }
}
