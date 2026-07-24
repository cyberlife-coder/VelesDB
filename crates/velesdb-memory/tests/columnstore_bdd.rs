//! BDD integration tests for the **`ColumnStore` facet**: structured metadata on
//! `remember`, exact-match filtering on `recall`, and the three-engine fusion
//! (vector + `ColumnStore` + graph) on `why`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::{meta, service};
use serde_json::json;
use tempfile::TempDir;
use velesdb_memory::{
    ColumnFilter, ColumnOp, HashEmbedder, MemoryError, MemoryService, AUTO_DATE_FIELD,
};

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

#[test]
fn recall_where_rejects_reserved_field() {
    let (_dir, svc, _) = seeded_timestamps();
    // Reserved system columns the docs promise are off limits.
    for reserved in ["content", "_veles_expires_at", "_veles_hub"] {
        let err = svc
            .recall_where(
                "project",
                10,
                &[ColumnFilter {
                    field: reserved.to_string(),
                    op: ColumnOp::Eq,
                    value: json!(1),
                }],
            )
            .expect_err("a reserved field must be rejected");
        assert!(
            matches!(err, MemoryError::InvalidFilter(_)),
            "reserved field {reserved} must be InvalidFilter"
        );
    }
}

// --- Nominal: `recall_where` surfaces caller metadata -------------------------

#[test]
fn recall_where_surfaces_the_matching_facts_metadata() {
    let (_dir, svc, [early, _mid, _late]) = seeded_timestamps();

    let hits = svc
        .recall_where(
            "project kickoff",
            10,
            &[ColumnFilter {
                field: "ts".to_string(),
                op: ColumnOp::Eq,
                value: json!(100),
            }],
        )
        .expect("recall_where eq");

    let hit = hits
        .iter()
        .find(|h| h.id == early)
        .expect("early fact present");
    let metadata = hit.metadata.as_ref().expect("metadata is Some");
    assert_eq!(metadata.get("ts"), Some(&json!(100)), "ts round-trips");
}

#[test]
fn recall_where_metadata_holds_only_the_auto_date_when_the_fact_has_no_caller_metadata() {
    // `remember` now auto-stamps every fact with `AUTO_DATE_FIELD` (see
    // `tests/auto_date_bdd.rs`), so a caller-metadata-free fact no longer
    // round-trips as `metadata: None` here either.
    let (_dir, svc) = service();
    let bare = svc
        .remember("a bare fact with no metadata", &[], None)
        .expect("remember bare");

    let hits = svc
        .recall_where("a bare fact", 10, &[])
        .expect("recall_where unfiltered");

    let hit = hits
        .iter()
        .find(|h| h.id == bare)
        .expect("bare fact present");
    let metadata = hit.metadata.as_ref().expect("auto date stamped");
    assert_eq!(
        metadata.keys().collect::<Vec<_>>(),
        vec![AUTO_DATE_FIELD],
        "a fact stored without caller metadata must round-trip with only the auto date"
    );
}

#[test]
fn recall_where_never_leaks_reserved_keys_in_metadata() {
    let (_dir, svc) = service();
    let id = svc
        .remember_with_ttl(
            "a ttl-bearing fact",
            &[],
            Some(&meta(&[("project", json!("veles"))])),
            Some(3_600),
        )
        .expect("remember with ttl + metadata");

    let hits = svc
        .recall_where("a ttl-bearing fact", 10, &[])
        .expect("recall_where unfiltered");

    let hit = hits.iter().find(|h| h.id == id).expect("fact present");
    let metadata = hit
        .metadata
        .as_ref()
        .expect("caller metadata + auto date present");
    assert!(
        metadata
            .keys()
            .all(|k| k == AUTO_DATE_FIELD || !k.starts_with("_veles_")),
        "no `_veles_`-namespaced system key other than the documented \
         AUTO_DATE_FIELD exception ever leaks through metadata"
    );
    assert!(
        !metadata.contains_key("_veles_expires_at"),
        "the durable TTL's reserved key must never leak, unlike AUTO_DATE_FIELD"
    );
    assert_eq!(
        metadata.get("project"),
        Some(&json!("veles")),
        "caller metadata still round-trips alongside the ttl"
    );
}

#[test]
fn recall_where_rejects_non_scalar_value() {
    let (_dir, svc, _) = seeded_timestamps();
    // An array/object/null value can't be compared against a column; it must be
    // a clear client-input error (InvalidFilter), not an opaque internal error.
    for bad in [json!([1, 2, 3]), json!({"x": 1}), json!(null)] {
        let err = svc
            .recall_where(
                "project",
                10,
                &[ColumnFilter {
                    field: "ts".to_string(),
                    op: ColumnOp::Eq,
                    value: bad.clone(),
                }],
            )
            .expect_err("a non-scalar value must be rejected");
        assert!(
            matches!(err, MemoryError::InvalidFilter(_)),
            "non-scalar value {bad} must be InvalidFilter"
        );
    }
}
