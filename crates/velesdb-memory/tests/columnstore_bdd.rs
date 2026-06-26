//! BDD integration tests for the **`ColumnStore` facet**: structured metadata on
//! `remember`, exact-match filtering on `recall`, and the three-engine fusion
//! (vector + `ColumnStore` + graph) on `why`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::{meta, service};
use serde_json::json;
use tempfile::TempDir;
use velesdb_memory::{ColumnFilter, ColumnOp, HashEmbedder, MemoryError, MemoryService};

/// Seed one `veles`-project and one `acme`-project "auth bug" fact; returns the
/// service plus the two ids.
fn seeded_two_projects() -> (TempDir, MemoryService<HashEmbedder>, u64, u64) {
    let (dir, svc) = service();
    let veles = svc
        .remember(
            "auth bug in the login flow",
            &[],
            Some(&meta(&[("project", json!("veles"))])),
        )
        .expect("remember veles");
    let other = svc
        .remember(
            "auth bug in the login flow too",
            &[],
            Some(&meta(&[("project", json!("acme"))])),
        )
        .expect("remember acme");
    (dir, svc, veles, other)
}

// --- Nominal: vector + `ColumnStore` -----------------------------------------

#[test]
fn recall_filters_to_the_matching_metadata() {
    let (_dir, svc, veles, other) = seeded_two_projects();

    let hits = svc
        .recall(
            "auth bug login",
            10,
            Some(&meta(&[("project", json!("veles"))])),
        )
        .expect("recall filtered");

    assert!(
        hits.iter().any(|h| h.id == veles),
        "the veles fact passes the filter"
    );
    assert!(
        hits.iter().all(|h| h.id != other),
        "the acme fact is filtered out"
    );
}

#[test]
fn recall_without_filter_returns_all_projects() {
    let (_dir, svc, veles, other) = seeded_two_projects();

    let hits = svc
        .recall("auth bug login", 10, None)
        .expect("recall unfiltered");

    let ids: Vec<u64> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&veles) && ids.contains(&other),
        "no filter returns both projects"
    );
}

#[test]
fn recall_filters_on_multiple_fields() {
    let (_dir, svc) = service();
    let target = svc
        .remember(
            "deadlock under contention",
            &[],
            Some(&meta(&[
                ("project", json!("veles")),
                ("status", json!("resolved")),
            ])),
        )
        .expect("remember target");
    let _open = svc
        .remember(
            "deadlock under contention again",
            &[],
            Some(&meta(&[
                ("project", json!("veles")),
                ("status", json!("open")),
            ])),
        )
        .expect("remember open");

    let hits = svc
        .recall(
            "deadlock contention",
            10,
            Some(&meta(&[
                ("project", json!("veles")),
                ("status", json!("resolved")),
            ])),
        )
        .expect("recall");

    assert_eq!(
        hits.len(),
        1,
        "only the resolved veles fact matches both fields"
    );
    assert_eq!(hits[0].id, target);
}

// --- Headline: vector + `ColumnStore` + graph fused on `why` -----------------

#[test]
fn why_fuses_vector_columnstore_and_graph() {
    let (_dir, svc) = service();
    // Two near-identical decision chains in different projects.
    let dec_a = svc
        .remember(
            "we picked tokio for the runtime",
            &[],
            Some(&meta(&[("project", json!("A"))])),
        )
        .expect("remember dec A");
    let pr_a = svc
        .remember(
            "PR A1 wires up tokio",
            &[],
            Some(&meta(&[("project", json!("A"))])),
        )
        .expect("remember pr A");
    svc.relate(dec_a, pr_a, "decided_in").expect("relate A");

    let dec_b = svc
        .remember(
            "we picked tokio for the runtime",
            &[],
            Some(&meta(&[("project", json!("B"))])),
        )
        .expect("remember dec B");
    let pr_b = svc
        .remember(
            "PR B1 wires up tokio",
            &[],
            Some(&meta(&[("project", json!("B"))])),
        )
        .expect("remember pr B");
    svc.relate(dec_b, pr_b, "decided_in").expect("relate B");

    // Explain, scoped to project A: the `ColumnStore` filter picks the A seed,
    // the vector matches the decision text, the graph reaches PR A1.
    let explanation = svc
        .why(
            "we picked tokio",
            1,
            Some(&meta(&[("project", json!("A"))])),
        )
        .expect("why scoped");

    let ids: Vec<u64> = explanation.nodes.iter().map(|n| n.id).collect();
    assert!(ids.contains(&pr_a), "graph traversal reaches PR A1");
    assert!(
        !ids.contains(&dec_b) && !ids.contains(&pr_b),
        "project B never leaks in"
    );
}

// --- Edge / Negative -------------------------------------------------------

#[test]
fn filter_excludes_memories_stored_without_metadata() {
    let (_dir, svc) = service();
    let bare = svc
        .remember("a fact with no metadata", &[], None)
        .expect("remember bare");

    let hits = svc
        .recall("a fact", 10, Some(&meta(&[("project", json!("veles"))])))
        .expect("recall filtered");

    assert!(
        hits.iter().all(|h| h.id != bare),
        "a non-empty filter excludes metadata-less facts"
    );
}

#[test]
fn filter_with_no_match_returns_empty() {
    let (_dir, svc) = service();
    svc.remember("auth bug", &[], Some(&meta(&[("project", json!("veles"))])))
        .expect("remember");

    let hits = svc
        .recall(
            "auth bug",
            10,
            Some(&meta(&[("project", json!("nonexistent"))])),
        )
        .expect("recall");

    assert!(hits.is_empty(), "no fact matches the filter");
}

// --- Nominal: fused vector + `ColumnStore` range (recall_where) ---------------

/// Seed three "project kickoff" facts at ascending `ts` timestamps.
fn seeded_timestamps() -> (TempDir, MemoryService<HashEmbedder>, [u64; 3]) {
    let (dir, svc) = service();
    let early = svc
        .remember(
            "project kickoff meeting",
            &[],
            Some(&meta(&[("ts", json!(100))])),
        )
        .expect("remember early");
    let mid = svc
        .remember(
            "project kickoff follow-up",
            &[],
            Some(&meta(&[("ts", json!(200))])),
        )
        .expect("remember mid");
    let late = svc
        .remember(
            "project kickoff retrospective",
            &[],
            Some(&meta(&[("ts", json!(300))])),
        )
        .expect("remember late");
    (dir, svc, [early, mid, late])
}

#[test]
fn recall_where_filters_by_numeric_range() {
    let (_dir, svc, [early, mid, late]) = seeded_timestamps();

    let hits = svc
        .recall_where(
            "project kickoff",
            10,
            &[ColumnFilter {
                field: "ts".to_string(),
                op: ColumnOp::Ge,
                value: json!(200),
            }],
        )
        .expect("recall_where range");

    let ids: Vec<u64> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&mid) && ids.contains(&late),
        "ts >= 200 facts present"
    );
    assert!(!ids.contains(&early), "ts = 100 fact excluded by the range");
    assert!(
        hits.iter().all(|h| !h.content.is_empty()),
        "content is lifted from payload"
    );
}

// --- Edge: empty query / zero budget ------------------------------------------

#[test]
fn recall_where_empty_query_or_zero_k_is_empty() {
    let (_dir, svc, _) = seeded_timestamps();
    assert!(svc
        .recall_where("   ", 10, &[])
        .expect("blank query")
        .is_empty());
    assert!(svc
        .recall_where("project", 0, &[])
        .expect("zero k")
        .is_empty());
}

// --- Negative: an unsafe field name is rejected, not interpolated --------------

#[test]
fn recall_where_rejects_non_identifier_field() {
    let (_dir, svc, _) = seeded_timestamps();
    let err = svc
        .recall_where(
            "project",
            10,
            &[ColumnFilter {
                field: "ts; DROP TABLE".to_string(),
                op: ColumnOp::Ge,
                value: json!(1),
            }],
        )
        .expect_err("a non-identifier field must be rejected");
    assert!(matches!(err, MemoryError::InvalidFilter(_)));
}
