//! BDD integration tests for the **`ColumnStore` facet**: structured metadata on
//! `remember`, exact-match filtering on `recall`, and the three-engine fusion
//! (vector + `ColumnStore` + graph) on `why`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::{meta, service};
use serde_json::json;

// --- Nominal: vector + `ColumnStore` -----------------------------------------

#[test]
fn recall_filters_to_the_matching_metadata() {
    let (_dir, svc) = service();
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
    let (_dir, svc) = service();
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
