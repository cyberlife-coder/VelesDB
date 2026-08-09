//! Unit tests for the MCP server tool handlers (split out of mcp.rs to keep
//! that file under the NLOC budget; same #[cfg(test)] module, via #[path]).

use super::*;
use crate::embedder::HashEmbedder;
use crate::model::{ColumnFilter, ColumnOp, Link};
use crate::service::Metadata;
use tempfile::TempDir;

const DECISION: &str = "we chose parking_lot to avoid lock poisoning";

fn server() -> (TempDir, McpServer) {
    let dir = TempDir::new().expect("create tempdir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let service = MemoryService::open(dir.path(), embedder).expect("open memory store");
    (dir, McpServer::new(service))
}

/// Run a one-hop `why(DECISION)` through the server, returning the seed
/// subgraph's node ids and its edge count.
async fn why_one_hop(srv: &McpServer) -> (Vec<u64>, usize) {
    let Json(why) = srv
        .why(Parameters(WhyParams {
            decision: DECISION.to_owned(),
            max_hops: Some(1),
            filter: None,
        }))
        .await
        .expect("why");
    let ids: Vec<u64> = why.nodes.iter().map(|n| n.id).collect();
    (ids, why.edges.len())
}

#[tokio::test]
async fn remember_then_recall_roundtrips_through_the_server() {
    let (_dir, srv) = server();

    let Json(stored) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    let Json(recalled) = srv
        .recall(Parameters(RecallParams {
            query: "parking_lot poisoning".to_owned(),
            limit: None,
            filter: None,
        }))
        .await
        .expect("recall");

    assert!(recalled.memories.iter().any(|m| m.id == stored.id));
}

#[tokio::test]
async fn feedback_tool_reinforces_and_returns_confidence() {
    let (_dir, srv) = server();
    let Json(stored) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");

    let Json(up) = srv
        .feedback(Parameters(FeedbackParams {
            id: stored.id,
            success: true,
        }))
        .await
        .expect("feedback success");
    assert_eq!(up.id, stored.id);
    assert!(up.confidence > 0.5, "success raises confidence");

    let Json(down) = srv
        .feedback(Parameters(FeedbackParams {
            id: stored.id,
            success: false,
        }))
        .await
        .expect("feedback failure");
    assert!(down.confidence < up.confidence, "failure lowers confidence");
}

#[tokio::test]
async fn feedback_tool_errors_on_unknown_id() {
    let (_dir, srv) = server();
    let result = srv
        .feedback(Parameters(FeedbackParams {
            id: 4242,
            success: true,
        }))
        .await;
    assert!(result.is_err(), "feedback on an unknown id must error");
}

#[tokio::test]
async fn why_returns_the_connected_subgraph() {
    let (_dir, srv) = server();
    let Json(decision) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember decision");
    let Json(pr) = srv
        .remember(Parameters(RememberParams {
            fact: "PR #42 swaps the mutex".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember pr");
    srv.relate(Parameters(RelateParams {
        from: decision.id,
        to: pr.id,
        relation: "decided_in".to_owned(),
    }))
    .await
    .expect("relate");

    let (ids, edges) = why_one_hop(&srv).await;
    assert!(ids.contains(&decision.id) && ids.contains(&pr.id));
    assert_eq!(edges, 1);
}

#[tokio::test]
async fn forget_removes_a_memory_through_the_server() {
    let (_dir, srv) = server();
    let Json(stored) = srv
        .remember(Parameters(RememberParams {
            fact: "ephemeral note about France".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");

    let Json(forgotten) = srv
        .forget(Parameters(ForgetParams { id: stored.id }))
        .await
        .expect("forget");
    assert!(forgotten.found, "an existing memory must report found=true");
    assert_eq!(forgotten.id, stored.id);

    let Json(recalled) = srv
        .recall(Parameters(RecallParams {
            query: "France".to_owned(),
            limit: None,
            filter: None,
        }))
        .await
        .expect("recall");
    assert!(recalled.memories.iter().all(|m| m.id != stored.id));
}

#[tokio::test]
async fn forget_unknown_id_through_the_server_reports_not_found() {
    let (_dir, srv) = server();

    let Json(forgotten) = srv
        .forget(Parameters(ForgetParams { id: 999_999 }))
        .await
        .expect("forget on an unknown id must not error");

    assert!(
        !forgotten.found,
        "forget of an id that was never stored must report found=false"
    );
}

#[tokio::test]
async fn remember_links_are_traversable_by_why() {
    let (_dir, srv) = server();
    let Json(pr) = srv
        .remember(Parameters(RememberParams {
            fact: "PR #99 refactors locks".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember pr");
    let Json(decision) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: vec![Link {
                target: pr.id,
                relation: "decided_in".to_owned(),
            }],
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember decision with link");

    let (ids, _) = why_one_hop(&srv).await;
    assert!(ids.contains(&decision.id) && ids.contains(&pr.id));
}

#[tokio::test]
async fn metadata_and_filter_flow_through_the_server() {
    let (_dir, srv) = server();
    let mut veles_meta = Metadata::new();
    veles_meta.insert("project".to_owned(), serde_json::json!("veles"));
    let mut acme_meta = Metadata::new();
    acme_meta.insert("project".to_owned(), serde_json::json!("acme"));

    let Json(kept) = srv
        .remember(Parameters(RememberParams {
            fact: "auth bug in login".to_owned(),
            links: Vec::new(),
            metadata: Some(veles_meta.clone()),
            ttl_seconds: None,
        }))
        .await
        .expect("remember veles");
    let Json(dropped) = srv
        .remember(Parameters(RememberParams {
            fact: "auth bug in login too".to_owned(),
            links: Vec::new(),
            metadata: Some(acme_meta),
            ttl_seconds: None,
        }))
        .await
        .expect("remember acme");

    let Json(recalled) = srv
        .recall(Parameters(RecallParams {
            query: "auth bug".to_owned(),
            limit: None,
            filter: Some(veles_meta),
        }))
        .await
        .expect("recall filtered");

    assert!(recalled.memories.iter().any(|m| m.id == kept.id));
    assert!(recalled.memories.iter().all(|m| m.id != dropped.id));
}

#[tokio::test]
async fn remember_accepts_explicit_and_default_ttl() {
    let (_dir, srv) = server();
    let srv = srv.with_default_ttl(3_600);

    // Per-fact ttl_seconds flows through the tool.
    let Json(explicit) = srv
        .remember(Parameters(RememberParams {
            fact: "explicit ttl fact".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: Some(3_600),
        }))
        .await
        .expect("remember with explicit ttl");

    // No per-fact ttl → the server's default_ttl applies.
    let Json(defaulted) = srv
        .remember(Parameters(RememberParams {
            fact: "default ttl fact".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember with default ttl");

    // Both have a future expiry, so both are still recallable now.
    let Json(recalled) = srv
        .recall(Parameters(RecallParams {
            query: "ttl fact".to_owned(),
            limit: None,
            filter: None,
        }))
        .await
        .expect("recall");
    assert!(recalled.memories.iter().any(|m| m.id == explicit.id));
    assert!(recalled.memories.iter().any(|m| m.id == defaulted.id));
}

/// Build a `{"ts": <n>}` metadata map.
fn ts_meta(ts: i64) -> Metadata {
    let mut meta = Metadata::new();
    meta.insert("ts".to_owned(), serde_json::json!(ts));
    meta
}

#[tokio::test]
async fn recall_where_filters_by_range_through_the_server() {
    let (_dir, srv) = server();
    for (fact, ts) in [
        ("kickoff in january", 20_230_115),
        ("kickoff in june", 20_230_615),
    ] {
        srv.remember(Parameters(RememberParams {
            fact: fact.to_owned(),
            links: Vec::new(),
            metadata: Some(ts_meta(ts)),
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    }

    let Json(res) = srv
        .recall_where(Parameters(RecallWhereParams {
            query: "kickoff".to_owned(),
            limit: None,
            filters: vec![ColumnFilter {
                field: "ts".to_owned(),
                op: ColumnOp::Ge,
                value: serde_json::json!(20_230_601),
            }],
        }))
        .await
        .expect("recall_where");

    assert!(
        res.memories.iter().any(|m| m.content.contains("june")),
        "the june fact is within the ts range"
    );
    assert!(
        res.memories.iter().all(|m| !m.content.contains("january")),
        "the january fact is below the ts range and excluded"
    );
}

// --- Error-code mapping -------------------------------------------------

#[tokio::test]
async fn recall_where_invalid_field_returns_invalid_params() {
    let (_dir, srv) = server();
    let err = srv
        .recall_where(Parameters(RecallWhereParams {
            query: "anything".to_owned(),
            limit: None,
            filters: vec![ColumnFilter {
                field: "ts; DROP TABLE".to_owned(),
                op: ColumnOp::Ge,
                value: serde_json::json!(1),
            }],
        }))
        .await
        .map(|_| ())
        .expect_err("a non-identifier filter field must be rejected");
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn empty_fact_returns_invalid_params_not_internal_error() {
    let (_dir, srv) = server();
    let err = srv
        .remember(Parameters(RememberParams {
            fact: "   ".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .map(|_| ())
        .expect_err("whitespace fact must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "EmptyFact must map to invalid_params so clients distinguish bad input from server faults"
    );
}

#[tokio::test]
async fn unknown_link_target_returns_invalid_params_not_internal_error() {
    let (_dir, srv) = server();
    let err = srv
        .remember(Parameters(RememberParams {
            fact: "a decision".to_owned(),
            links: vec![Link {
                target: 9_999_999,
                relation: "x".to_owned(),
            }],
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .map(|_| ())
        .expect_err("unknown link target must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "UnknownMemory must map to invalid_params"
    );
}

#[tokio::test]
async fn relate_to_unknown_endpoint_returns_invalid_params_not_internal_error() {
    let (_dir, srv) = server();
    let Json(stored) = srv
        .remember(Parameters(RememberParams {
            fact: "an existing memory".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");

    // Relating an existing memory to a non-existent one is bad client input,
    // not a server fault — the agent must see invalid_params so it can fix
    // the id rather than retry a phantom internal error.
    let err = srv
        .relate(Parameters(RelateParams {
            from: stored.id,
            to: 9_999_999,
            relation: "references".to_owned(),
        }))
        .await
        .map(|_| ())
        .expect_err("relate to a missing endpoint must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "relate to an unknown endpoint must map to invalid_params"
    );
}

// --- Input size guards -----------------------------------------------------

#[tokio::test]
async fn oversized_fact_returns_invalid_params() {
    let (_dir, srv) = server();
    let huge = "x".repeat(MAX_FACT_BYTES + 1);
    let err = srv
        .remember(Parameters(RememberParams {
            fact: huge,
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .map(|_| ())
        .expect_err("oversized fact must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "oversized fact must map to invalid_params"
    );
}

#[tokio::test]
async fn entity_miss_echoes_the_canonical_queried_name() {
    // Regression (#1654-4): a miss returned `name: ""`, so a caller running
    // several lookups in parallel could not tell which question a response
    // answered. The echo is canonicalized exactly as a hit's name would be
    // (trimmed, lowercased), so a hit and a miss stay comparable.
    let (_dir, srv) = server();

    let Json(profile) = srv
        .entity(Parameters(EntityParams {
            name: "  Zzz Personne Inexistante  ".to_owned(),
        }))
        .await
        .expect("entity lookup");

    assert!(!profile.found, "nothing was ever stored under that name");
    assert_eq!(
        profile.name, "zzz personne inexistante",
        "a miss must echo the queried name, canonicalized like a hit's"
    );
    assert_eq!(profile.id, 0, "a miss still reports no id");
}

#[tokio::test]
async fn recall_fused_folds_in_a_graph_reached_fact() {
    let (_dir, srv) = server();
    // Anchor: an exact vector hit for the query.
    let Json(anchor) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember anchor");
    // Linked: unrelated text (so it is not a vector hit for the query), but
    // graph-connected to the anchor — only the graph reach can surface it.
    let Json(linked) = srv
        .remember(Parameters(RememberParams {
            fact: "the on-call rotation moved to Tuesdays".to_owned(),
            links: vec![Link {
                target: anchor.id,
                relation: "context".to_owned(),
            }],
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember linked");

    // Plain top-1 vector recall for the query finds the anchor, not the linked fact.
    let Json(plain) = srv
        .recall(Parameters(RecallParams {
            query: DECISION.to_owned(),
            limit: Some(1),
            filter: None,
        }))
        .await
        .expect("recall");
    assert!(!plain.memories.iter().any(|m| m.id == linked.id));

    // Fused recall walks the graph from the anchor seed and folds the linked fact in.
    let Json(fused) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: DECISION.to_owned(),
            limit: Some(10),
            filter: None,
            hops: None,
            graph_boost: None,
            pool: None,
            date_field: None,
        }))
        .await
        .expect("recall_fused");
    assert!(
        fused.memories.iter().any(|m| m.id == anchor.id),
        "anchor present in fused recall"
    );
    assert!(
        fused.memories.iter().any(|m| m.id == linked.id),
        "graph-reached fact folded into fused recall"
    );
}

#[tokio::test]
async fn recall_fused_with_date_field_returns_a_dated_timeline() {
    let (_dir, srv) = server();
    // Two dated facts, stored newest-first so ordering is actually exercised.
    for (fact, ts) in [
        ("the release shipped", 20_260_701_i64),
        ("the project kicked off", 20_260_103),
    ] {
        srv.remember(Parameters(RememberParams {
            fact: fact.to_owned(),
            links: Vec::new(),
            metadata: Some(ts_meta(ts)),
            ttl_seconds: None,
        }))
        .await
        .expect("remember dated fact");
    }

    let Json(res) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: "project release timeline".to_owned(),
            limit: Some(10),
            filter: None,
            hops: None,
            graph_boost: None,
            pool: None,
            date_field: Some("ts".to_owned()),
        }))
        .await
        .expect("recall_fused dated");

    let timeline = res
        .dated_context
        .expect("dated_context present when date_field set");
    // Chronological: kickoff (Jan) before release (Jul), each date-prefixed.
    assert!(timeline.contains("- [2026-01-03] the project kicked off"));
    assert!(timeline.contains("- [2026-07-01] the release shipped"));
    assert!(
        timeline.find("2026-01-03").unwrap() < timeline.find("2026-07-01").unwrap(),
        "facts must be ordered oldest-first"
    );
    assert_eq!(res.now.as_deref(), Some("2026-07-01"));
}

#[tokio::test]
async fn recall_fused_without_date_field_omits_the_timeline() {
    let (_dir, srv) = server();
    srv.remember(Parameters(RememberParams {
        fact: DECISION.to_owned(),
        links: Vec::new(),
        metadata: None,
        ttl_seconds: None,
    }))
    .await
    .expect("remember");
    let Json(res) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: DECISION.to_owned(),
            limit: Some(5),
            filter: None,
            hops: None,
            graph_boost: None,
            pool: None,
            date_field: None,
        }))
        .await
        .expect("recall_fused");
    assert!(res.dated_context.is_none());
    assert!(res.now.is_none());
}

#[tokio::test]
async fn recall_fused_survives_a_non_finite_graph_boost() {
    // A NaN graph_boost reaches the pyo3/native-float bindings unfiltered; if it
    // hit fusion it would collapse the ranking (NaN scores compare Equal) and
    // silently drop the graph-reached facts. The service sanitizes it, so the
    // linked fact is still surfaced — proving the guard holds on the real path.
    let (_dir, srv) = server();
    let Json(anchor) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember anchor");
    let Json(linked) = srv
        .remember(Parameters(RememberParams {
            fact: "the on-call rotation moved to Tuesdays".to_owned(),
            links: vec![Link {
                target: anchor.id,
                relation: "context".to_owned(),
            }],
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember linked");

    let Json(fused) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: DECISION.to_owned(),
            limit: Some(10),
            filter: None,
            hops: None,
            graph_boost: Some(f64::NAN),
            pool: None,
            date_field: None,
        }))
        .await
        .expect("recall_fused");
    assert!(
        fused.memories.iter().any(|m| m.id == linked.id),
        "graph-reached fact must still surface despite a non-finite graph_boost"
    );
}

#[tokio::test]
async fn recall_fused_limit_is_capped_at_max() {
    let (_dir, srv) = server();
    // The call must succeed (capped, not rejected) even with an absurd limit.
    // `pool` joins limit/hops here because it feeds the same oversampled
    // vector search: uncapped, it is exactly the unbounded-scan risk they are.
    let Json(result) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: "anything".to_owned(),
            limit: Some(usize::MAX),
            filter: None,
            hops: Some(usize::MAX),
            graph_boost: None,
            pool: Some(usize::MAX),
            date_field: None,
        }))
        .await
        .expect("recall_fused with huge limit/hops/pool must succeed (silently capped)");
    let _ = result;
}

#[tokio::test]
async fn recall_limit_is_capped_at_max() {
    let (_dir, srv) = server();
    // The call must succeed (capped, not rejected).
    let Json(result) = srv
        .recall(Parameters(RecallParams {
            query: "anything".to_owned(),
            limit: Some(usize::MAX),
            filter: None,
        }))
        .await
        .expect("recall with huge limit must succeed (silently capped)");
    // Empty store — just verify no error, not result length.
    let _ = result;
}

#[tokio::test]
async fn why_hop_depth_is_capped_at_max() {
    let (_dir, srv) = server();
    srv.remember(Parameters(RememberParams {
        fact: DECISION.to_owned(),
        links: Vec::new(),
        metadata: None,
        ttl_seconds: None,
    }))
    .await
    .expect("remember");
    // Must not hang or explode with an astronomical hop value.
    let Json(_) = srv
        .why(Parameters(WhyParams {
            decision: DECISION.to_owned(),
            max_hops: Some(usize::MAX),
            filter: None,
        }))
        .await
        .expect("why with huge max_hops must succeed (silently capped)");
}

// --- Auto-extraction tool ---------------------------------------------------

#[tokio::test]
async fn remember_extracted_builds_a_graph_through_the_server() {
    use crate::extract::{ExtractError, ExtractedFact, Extractor};

    struct Stub;
    impl Extractor for Stub {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
            Ok(vec![
                ExtractedFact {
                    text: "Alice ships the parser in Rust.".to_owned(),
                    entities: vec!["rust".to_owned()],
                },
                ExtractedFact {
                    text: "Bob maintains the Rust toolchain.".to_owned(),
                    entities: vec!["rust".to_owned()],
                },
            ])
        }
    }

    let (_dir, srv) = server();
    let srv = srv.with_extractor(Arc::new(Stub) as DynExtractor);

    let Json(res) = srv
        .remember_extracted(Parameters(RememberExtractedParams {
            text: "Alice and Bob work in Rust.".to_owned(),
            metadata: None,
        }))
        .await
        .expect("remember_extracted");
    assert_eq!(res.ids.len(), 2, "both facts stored");

    // why reaches the sibling fact via the shared topic, seed is a real fact.
    let Json(why) = srv
        .why(Parameters(WhyParams {
            decision: "parser in rust".to_owned(),
            max_hops: Some(2),
            filter: None,
        }))
        .await
        .expect("why");
    assert!(why.nodes.len() > 1, "graph is alive through the server");
    assert!(
        !why.nodes[0].content.starts_with("Entity:"),
        "seed is a fact, not a hub"
    );
}

#[tokio::test]
async fn reserved_metadata_key_returns_invalid_params() {
    let (_dir, srv) = server();
    let mut bad_meta = Metadata::new();
    bad_meta.insert("_veles_hub".to_owned(), serde_json::json!(true));
    let err = srv
        .remember(Parameters(RememberParams {
            fact: "a fact".to_owned(),
            links: Vec::new(),
            metadata: Some(bad_meta),
            ttl_seconds: None,
        }))
        .await
        .map(|_| ())
        .expect_err("reserved metadata key must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "ReservedKey must map to invalid_params, not internal_error"
    );
}

#[tokio::test]
async fn recall_where_non_scalar_filter_value_returns_invalid_params() {
    let (_dir, srv) = server();
    let err = srv
        .recall_where(Parameters(RecallWhereParams {
            query: "query".to_owned(),
            limit: None,
            filters: vec![ColumnFilter {
                field: "ts".to_owned(),
                op: ColumnOp::Eq,
                value: serde_json::json!([1, 2, 3]),
            }],
        }))
        .await
        .map(|_| ())
        .expect_err("array filter value must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "non-scalar filter value must map to invalid_params"
    );
}

#[tokio::test]
async fn relate_with_empty_relation_returns_invalid_params() {
    let (_dir, srv) = server();
    let Json(a) = srv
        .remember(Parameters(RememberParams {
            fact: "fact A".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember A");
    let Json(b) = srv
        .remember(Parameters(RememberParams {
            fact: "fact B".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember B");
    let err = srv
        .relate(Parameters(RelateParams {
            from: a.id,
            to: b.id,
            relation: String::new(),
        }))
        .await
        .map(|_| ())
        .expect_err("empty relation must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "InvalidRelation must map to invalid_params"
    );
}

#[tokio::test]
async fn remember_extracted_without_backend_returns_internal_error() {
    let (_dir, srv) = server(); // no extractor attached
    let err = srv
        .remember_extracted(Parameters(RememberExtractedParams {
            text: "anything".to_owned(),
            metadata: None,
        }))
        .await
        .map(|_| ())
        .expect_err("extraction with no backend must error");
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
}

// --- u64-id wire compatibility (issue #1468) --------------------------------
//
// Float-lossy JSON clients (JS `number`, Claude Code included) round a u64
// id above 2^53 on the way OUT of a response, then resubmit the rounded
// value on the way IN to relate/forget/feedback — which fails with "memory
// does not exist" against a real (large) id. The fix is additive: every id a
// memory tool returns also comes back as a decimal-string `..._str` twin, and
// every id a memory tool accepts tolerates that string form. Proven at the
// serde boundary (`serde_json::from_value`), not by constructing the Rust
// struct directly, since the latter sidesteps JSON entirely and would not
// have caught the bug.

#[test]
fn relate_params_accept_string_or_number_ids_on_the_wire() {
    let numeric: RelateParams =
        serde_json::from_value(serde_json::json!({"from": 1, "to": 2, "relation": "r"}))
            .expect("numeric ids must still deserialize (0.9.x compat)");
    assert_eq!((numeric.from, numeric.to), (1, 2));

    let stringy: RelateParams =
        serde_json::from_value(serde_json::json!({"from": "1", "to": "2", "relation": "r"}))
            .expect("decimal-string ids must deserialize");
    assert_eq!((stringy.from, stringy.to), (1, 2));
}

#[test]
fn forget_and_feedback_params_accept_string_ids_on_the_wire() {
    let forget: ForgetParams = serde_json::from_value(serde_json::json!({"id": "42"}))
        .expect("forget id must accept a decimal string");
    assert_eq!(forget.id, 42);

    let feedback: FeedbackParams =
        serde_json::from_value(serde_json::json!({"id": "42", "success": true}))
            .expect("feedback id must accept a decimal string");
    assert_eq!(feedback.id, 42);
}

#[test]
fn remember_link_target_accepts_a_string_id_on_the_wire() {
    let link: Link = serde_json::from_value(serde_json::json!({"target": "7", "relation": "r"}))
        .expect("Link::target must accept a decimal string");
    assert_eq!(link.target, 7);
}

#[tokio::test]
async fn remember_recall_relate_forget_feedback_responses_echo_an_id_str_twin() {
    let (_dir, srv) = server();
    let Json(remembered) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    assert_eq!(remembered.id_str, remembered.id.to_string());

    let Json(recalled) = srv
        .recall(Parameters(RecallParams {
            query: "parking_lot poisoning".to_owned(),
            limit: None,
            filter: None,
        }))
        .await
        .expect("recall");
    let hit = recalled
        .memories
        .iter()
        .find(|m| m.id == remembered.id)
        .expect("recalled memory present");
    assert_eq!(hit.id_str, hit.id.to_string());

    let Json(pr) = srv
        .remember(Parameters(RememberParams {
            fact: "PR #42 swaps the mutex".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember pr");
    let Json(relate_res) = srv
        .relate(Parameters(RelateParams {
            from: remembered.id,
            to: pr.id,
            relation: "decided_in".to_owned(),
        }))
        .await
        .expect("relate");
    assert_eq!(relate_res.edge_id_str, relate_res.edge_id.to_string());

    let Json(feedback_res) = srv
        .feedback(Parameters(FeedbackParams {
            id: remembered.id,
            success: true,
        }))
        .await
        .expect("feedback");
    assert_eq!(feedback_res.id_str, feedback_res.id.to_string());

    let Json(forget_res) = srv
        .forget(Parameters(ForgetParams { id: pr.id }))
        .await
        .expect("forget");
    assert_eq!(forget_res.id_str, forget_res.id.to_string());
}

#[tokio::test]
async fn why_response_echoes_id_str_and_from_to_str_on_nodes_and_edges() {
    let (_dir, srv) = server();
    let Json(decision) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember decision");
    let Json(pr) = srv
        .remember(Parameters(RememberParams {
            fact: "PR #42 swaps the mutex".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember pr");
    srv.relate(Parameters(RelateParams {
        from: decision.id,
        to: pr.id,
        relation: "decided_in".to_owned(),
    }))
    .await
    .expect("relate");

    let Json(why) = srv
        .why(Parameters(WhyParams {
            decision: DECISION.to_owned(),
            max_hops: Some(1),
            filter: None,
        }))
        .await
        .expect("why");
    assert!(!why.nodes.is_empty() && !why.edges.is_empty());
    for node in &why.nodes {
        assert_eq!(node.id_str, node.id.to_string());
    }
    for edge in &why.edges {
        assert_eq!(edge.from_str, edge.from.to_string());
        assert_eq!(edge.to_str, edge.to.to_string());
    }
}

#[tokio::test]
async fn remember_extracted_response_echoes_ids_str() {
    use crate::extract::{ExtractError, ExtractedFact, Extractor};
    struct Stub;
    impl Extractor for Stub {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
            Ok(vec![ExtractedFact {
                text: "Alice ships the parser in Rust.".to_owned(),
                entities: vec!["rust".to_owned()],
            }])
        }
    }
    let (_dir, srv) = server();
    let srv = srv.with_extractor(Arc::new(Stub) as DynExtractor);
    let Json(res) = srv
        .remember_extracted(Parameters(RememberExtractedParams {
            text: "Alice works in Rust.".to_owned(),
            metadata: None,
        }))
        .await
        .expect("remember_extracted");
    assert_eq!(res.ids_str.len(), res.ids.len());
    for (id, id_str) in res.ids.iter().zip(res.ids_str.iter()) {
        assert_eq!(*id_str, id.to_string());
    }
}

/// The round-trip that closes #1468. Simulates the reported failure two
/// ways: (1) a wrong numeric id — the client-side rounding stand-in — is
/// rejected by `relate` exactly like the maintainer's dogfooding report
/// ("memory does not exist"); (2) relaying the exact `id_str` decimal-string
/// twins instead (deserialized straight off raw JSON, not the Rust struct)
/// succeeds, and `why` finds the resulting edge — proving the fix actually
/// closes the loop end to end, not just at the DTO level.
#[tokio::test]
async fn relate_by_wrong_numeric_id_fails_but_id_str_round_trip_succeeds() {
    let (_dir, srv) = server();
    let Json(decision) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember decision");
    let Json(pr) = srv
        .remember(Parameters(RememberParams {
            fact: "PR #42 swaps the mutex".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember pr");

    // Stand-in for a float-lossy client rounding the id on the way out and
    // back in: the perturbed id was never stored, so relate must reject it.
    let wrong_from = decision.id + 1_000_003;
    let err = srv
        .relate(Parameters(RelateParams {
            from: wrong_from,
            to: pr.id,
            relation: "decided_in".to_owned(),
        }))
        .await
        .map(|_| ())
        .expect_err("a rounded/wrong id must not silently resolve to the real memory");
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);

    // The fix: relay the exact `id_str` twins as JSON strings, off the wire.
    let params: RelateParams = serde_json::from_value(serde_json::json!({
        "from": decision.id_str,
        "to": pr.id_str,
        "relation": "decided_in",
    }))
    .expect("id_str values must deserialize as RelateParams");
    srv.relate(Parameters(params))
        .await
        .expect("relate via id_str must succeed");

    let (ids, edges) = why_one_hop(&srv).await;
    assert!(ids.contains(&decision.id) && ids.contains(&pr.id));
    assert_eq!(edges, 1);
}

#[test]
fn test_get_info_instructions_cover_memory_compiler_and_working_context() {
    // The server's `get_info().instructions` is its one-shot vitrine to a
    // connecting agent — it must not hide half the product behind a
    // memory-only blurb (quick win V2a-1).
    let (_dir, srv) = server();
    let info = srv.get_info();
    let instructions = info.instructions.expect("instructions must be set");

    assert!(
        instructions.contains("recall") && instructions.contains("relate"),
        "must mention the memory family: {instructions}"
    );
    #[cfg(feature = "context")]
    {
        assert!(
            instructions.contains("compile_context"),
            "must mention the context compiler family: {instructions}"
        );
        assert!(
            instructions.contains("working"),
            "must mention working-context resumption: {instructions}"
        );
        assert!(
            instructions.contains("compile_transcript"),
            "must mention the compile_transcript shortcut (V2b-2/V2b-3): {instructions}"
        );
    }
}

#[test]
fn test_recall_where_description_documents_type_strict_comparisons() {
    // Issue #1473: `recall_where`'s comparisons are type-strict (a
    // string-stored value never matches a numeric filter value, and vice
    // versa), with no runtime coercion. The tool description must say so
    // explicitly instead of silently returning an empty set.
    let tool = McpServer::recall_where_tool_attr();
    let description = tool
        .description
        .expect("recall_where must declare a description");
    let lower = description.to_lowercase();
    assert!(
        lower.contains("type-strict") || lower.contains("type strict"),
        "recall_where's description must document type-strict comparisons: {description}"
    );
    assert!(
        description.to_lowercase().contains("numeric"),
        "recall_where's description must advise storing comparable values \
         (e.g. dates) numerically: {description}"
    );
}

/// Whether `haystack` names `field` as a WORD, not as an accidental substring.
///
/// A plain `contains` is not enough, and the measurement says so: across the
/// twenty published tools it lets `feedback`'s `id` pass on the strength of
/// "con-f-**id**-ence", and `entity`'s `relations` on "relation-ships". A
/// guard weaker than its own statement is worse than none, because it reads
/// as coverage.
///
/// Backticks, spaces and punctuation are all boundaries here; only ASCII
/// alphanumerics and `_` continue a word — so `ids` does NOT match inside
/// `ids_str`, and each field has to be named on its own.
fn description_names_field(haystack: &str, field: &str) -> bool {
    let continues_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let bytes = haystack.as_bytes();
    haystack.match_indices(field).any(|(start, _)| {
        let end = start + field.len();
        let opens = start == 0 || !continues_word(bytes[start - 1]);
        let closes = end == bytes.len() || !continues_word(bytes[end]);
        opens && closes
    })
}

/// #1747. `remember_extracted`'s published `outputSchema` carries three root
/// fields, and one of them — `skipped_over_cap` — exists PRECISELY to make a
/// silent loss visible: facts the extractor produced and this tool dropped for
/// exceeding the per-fact text cap. Its own doc-comment argues the point: "a
/// skip the caller cannot see is indistinguishable from the model extracting
/// fewer facts".
///
/// The description used to say only "Returns the stored facts' ids". So the
/// one surface a model reads BEFORE deciding to call never mentioned the field
/// whose whole purpose is to be read — the omission defeated the reason the
/// field was added (#1692).
///
/// The field names are DERIVED from the published schema rather than kept as a
/// second hard-coded list here: a root field added tomorrow is caught without
/// anyone remembering this test exists.
///
/// Scope, stated so it is not overclaimed: this pins ONE tool. Fourteen of the
/// twenty published tools would fail the same check today, and closing that
/// class is its own piece of work (#1695).
#[test]
fn test_remember_extracted_description_names_every_published_output_field() {
    let tool = McpServer::remember_extracted_tool_attr();
    let description = tool
        .description
        .clone()
        .expect("remember_extracted must declare a description");
    let schema = tool
        .output_schema
        .as_ref()
        .expect("remember_extracted declares an explicit output_schema");
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("its output schema is an object schema carrying `properties`");

    assert!(
        !properties.is_empty(),
        "the published output schema must expose root properties, else this \
         test would pass vacuously"
    );

    let missing: Vec<&str> = properties
        .keys()
        .filter(|field| !description_names_field(&description, field))
        .map(String::as_str)
        .collect();

    assert!(
        missing.is_empty(),
        "remember_extracted's description must name every root field of its \
         published outputSchema. Missing: {missing:?}. A field the description \
         omits is a field the model never learns to read, and `skipped_over_cap` \
         omitted is a silent data loss — the caller believes everything it sent \
         was stored. Description was: {description}"
    );
}

/// Issue: real MCP client harnesses (observed 2026-07-24 with Claude Code)
/// degrade a parameter whose advertised schema carries no DIRECT `type`
/// keyword — `anyOf`-wrapped optionals and `$ref`-only structs both come out
/// untyped on the client side, and the harness then serializes the argument
/// as a JSON-encoded STRING (`limit: "6"`, `filter: "{\"project\":...}"`),
/// which the server rejects. Same wire-contract class as issue #1468
/// (u64 ids vs float-lossy clients): the schema must be harness-proof, not
/// merely spec-correct.
///
/// This test locks the contract for every optional scalar/object parameter
/// of the recall family: each property's schema must expose a direct
/// `type` keyword.
#[test]
fn test_recall_fused_input_schema_types_every_parameter_directly() {
    let tool = McpServer::recall_fused_tool_attr();
    let schema = serde_json::to_value(&tool.input_schema).expect("schema serializes");
    let properties = schema["properties"]
        .as_object()
        .expect("recall_fused input schema must have properties");
    for (name, subschema) in properties {
        assert!(
            subschema.get("type").is_some(),
            "recall_fused parameter `{name}` must advertise a direct `type` \
             keyword (anyOf/$ref-only schemas get stringified by real MCP \
             harnesses); got: {subschema}"
        );
    }
}

/// Server-side tolerance half of the harness-proof contract: a client that
/// DID stringify a scalar or object argument (today's Claude Code harness
/// does exactly that for schema-degraded parameters) must still be served.
/// Mirrors the #1468 string-id acceptance.
#[test]
fn test_recall_fused_params_accept_stringified_scalars_and_objects() {
    let params: RecallFusedParams = serde_json::from_value(serde_json::json!({
        "query": "q",
        "limit": "6",
        "hops": "2",
        "graph_boost": "0.15",
        "pool": "128",
        "filter": "{\"project\": \"velesdb\"}"
    }))
    .expect("stringified scalar/object arguments must deserialize");
    assert_eq!(params.limit, Some(6));
    assert_eq!(params.hops, Some(2));
    assert_eq!(params.pool, Some(128));
    assert!((params.graph_boost.unwrap() - 0.15).abs() < f64::EPSILON);
    let filter = params.filter.expect("filter must parse from a JSON string");
    assert_eq!(
        filter.get("project").and_then(|v| v.as_str()),
        Some("velesdb")
    );
}

// ---------------------------------------------------------------------------
// unrelate (issue #1661) and entity relations_in
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unrelate_tool_removes_the_edge_and_is_idempotent() {
    let (_dir, srv) = server();
    let Json(a) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember a");
    let Json(b) = srv
        .remember(Parameters(RememberParams {
            fact: "the cause behind the decision".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember b");
    srv.relate(Parameters(RelateParams {
        from: a.id,
        to: b.id,
        relation: "caused_by".to_owned(),
    }))
    .await
    .expect("relate");

    let Json(res) = srv
        .unrelate(Parameters(UnrelateParams {
            from: a.id,
            to: b.id,
            relation: "caused_by".to_owned(),
        }))
        .await
        .expect("unrelate");
    assert!(res.found, "the edge existed");
    assert_eq!(res.removed, 1);

    let Json(res) = srv
        .unrelate(Parameters(UnrelateParams {
            from: a.id,
            to: b.id,
            relation: "caused_by".to_owned(),
        }))
        .await
        .expect("a second unrelate must not error — cleanups are replayable");
    assert!(!res.found, "already removed");
    assert_eq!(res.removed, 0);

    let (_ids, edge_count) = why_one_hop(&srv).await;
    assert_eq!(edge_count, 0, "the edge must be gone from traversal");
}

#[tokio::test]
async fn unrelate_refuses_a_self_loop_as_invalid_params() {
    let (_dir, srv) = server();
    let Json(a) = srv
        .remember(Parameters(RememberParams {
            fact: DECISION.to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    let err = srv
        .unrelate(Parameters(UnrelateParams {
            from: a.id,
            to: a.id,
            relation: "supports".to_owned(),
        }))
        .await
        .map(|_| ())
        .expect_err("a self-loop is refused exactly like relate's");
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

/// The wire contract shared with `relate` (#1468): ids arrive as JSON numbers
/// or decimal strings alike.
#[test]
fn unrelate_params_accept_string_or_number_ids_on_the_wire() {
    let params: UnrelateParams = serde_json::from_value(serde_json::json!({
        "from": "18446744073709551615",
        "to": 42,
        "relation": "caused_by"
    }))
    .expect("string and number ids must both deserialize");
    assert_eq!(params.from, u64::MAX);
    assert_eq!(params.to, 42);
}

/// #1734, and this is the test that has to cross the REAL wiring rather than a
/// seam written for it.
///
/// The defect was never in the library: `OutlineExtractor` always worked, and
/// `McpServer::with_extractor` always accepted it. What was broken is that the
/// only code able to CHOOSE `outline` sat inside the daemon's
/// `#[cfg(feature = "extract")]` block, so on a default build two of the twenty
/// published tools were dead — `remember_extracted` refused outright, and
/// `entity` answered `found: false` for every name, entity hubs being born only
/// of extraction.
///
/// So this test deliberately calls [`crate::select_extractor`] — the same
/// function `main.rs` now calls — instead of constructing `OutlineExtractor`
/// directly. Building the extractor by hand here would prove only that the
/// library works, which was never in doubt. The one step it does not cover is
/// `std::env::var` itself, which is a process-global read the daemon does once.
#[tokio::test]
async fn an_outline_configured_server_builds_a_hub_that_entity_finds() {
    let dir = TempDir::new().expect("create tempdir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let service = MemoryService::open(dir.path(), embedder).expect("open memory store");

    // The selection, through the real function. `Ready` is the whole point:
    // outline needs no configuration, no network and no optional dependency,
    // which is why it can be chosen in a build that has none of them.
    let crate::ExtractorSelection::Ready(extractor) =
        crate::select_extractor("outline").expect("`outline` must be an accepted backend name")
    else {
        panic!("`outline` must be usable as-is, with no remote configuration");
    };
    let srv = McpServer::new(service).with_extractor(extractor);

    // Tool 1 of the two that were dead: it used to answer "extraction backend
    // not configured" no matter what the operator set.
    let Json(stored) = srv
        .remember_extracted(Parameters(RememberExtractedParams {
            text: "fact: Theo has a sister called Camille | Theo, Camille\n\
                   edge: Camille | soeur de | Theo\n\
                   attr: Theo | age | 15"
                .to_owned(),
            metadata: None,
        }))
        .await
        .expect("remember_extracted must work on an outline-configured server");
    assert_eq!(
        stored.ids.len(),
        1,
        "the passage carries exactly one `fact:` directive, so exactly one \
         fact must be stored (`edge:` and `attr:` build the graph around it, \
         they are not facts of their own)"
    );

    // Tool 2: `entity` answered `found: false` for EVERY name, because hubs are
    // salted so that no caller fact can create one — they are born only of
    // extraction. If this passes, the cascade is closed.
    let Json(theo) = srv
        .entity(Parameters(EntityParams {
            name: "Theo".to_owned(),
        }))
        .await
        .expect("entity");
    assert!(
        theo.found,
        "entity must find the hub that remember_extracted just created — \
         this is the second of the two behaviours #1734 reported dead"
    );
    let outgoing: Vec<&str> = theo
        .relations
        .iter()
        .map(|r| r.predicate.as_str())
        .collect();
    let incoming: Vec<&str> = theo
        .relations_in
        .iter()
        .map(|r| r.predicate.as_str())
        .collect();
    // Asserted on EITHER side on purpose, and the reason is written here so the
    // looseness is not mistaken for carelessness. What #1734 is about is that
    // the edge reaches the graph at all: before the fix no extractor could be
    // selected, so there was no edge on either side.
    //
    // Which side it lands on is a SEPARATE question, and an open one: measured
    // here, `edge: Camille | soeur de | Theo` lands in Theo's OUTGOING edges
    // and leaves his incoming empty, while `incoming_entity_relations`'s own
    // doc uses this exact example to say the opposite. Pinning either direction
    // in this test would freeze an answer nobody has established yet — so it is
    // tracked on its own instead.
    assert!(
        incoming.contains(&"soeur de") || outgoing.contains(&"soeur de"),
        "the `edge:` directive must reach the graph — outgoing {outgoing:?}, \
         incoming {incoming:?}"
    );
}

// ---------------------------------------------------------------------------
// Autograph wiring at SERVER level (#1846/#1851)
// ---------------------------------------------------------------------------
//
// `McpServer::new` does three load-bearing things when the service carries an
// autograph extractor: it spawns the background worker (sized
// `limits::MAX_AUTOGRAPH_QUEUE`), it keeps the returned
// `AutographWorkerHandle` so the SERVER's own drop bounds shutdown, and it
// falls back inline with a warning if the spawn is refused. The service-level
// suite (tests/autograph_async_bdd.rs) proves the worker itself; nothing
// proved the server WIRED it — a regression to the pre-#1851 construction
// (no spawn, enrichment inline on the tool path) passes every service test
// and hangs every MCP client on the model's generation time.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::extract::{ExtractError, ExtractedFact, ExtractedRelation, Extraction, Extractor};

/// The gate idiom from tests/autograph_async_bdd.rs: `extract_graph` blocks
/// until the test RELEASES it, so every timing claim below is an event,
/// never a sleep. `entered` flips BEFORE the block — the observable
/// "job is in flight" event — and `completed` after it.
struct GatedServerExtractor {
    gate: Mutex<Receiver<()>>,
    entered: AtomicUsize,
    completed: AtomicUsize,
}

impl GatedServerExtractor {
    fn new() -> (Arc<Self>, SyncSender<()>) {
        let (tx, rx) = sync_channel::<()>(64);
        (
            Arc::new(Self {
                gate: Mutex::new(rx),
                entered: AtomicUsize::new(0),
                completed: AtomicUsize::new(0),
            }),
            tx,
        )
    }
}

impl Extractor for GatedServerExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(Vec::new())
    }

    fn extract_graph(&self, _text: &str) -> Result<Extraction, ExtractError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.gate
            .lock()
            .expect("gate lock")
            .recv()
            .map_err(|_| ExtractError::Backend("gate closed".to_owned()))?;
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(Extraction {
            facts: Vec::new(),
            relations: vec![ExtractedRelation {
                subject: "alice martin".to_owned(),
                predicate: "travaille chez".to_owned(),
                object: "wiscale".to_owned(),
            }],
            attributes: Vec::new(),
        })
    }
}

/// A server over a service that carries the gated extractor as autograph,
/// plus the release channel. The [`TempDir`] must outlive everything.
fn gated_server() -> (
    TempDir,
    McpServer,
    Arc<GatedServerExtractor>,
    SyncSender<()>,
) {
    let dir = TempDir::new().expect("create tempdir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let (extractor, release) = GatedServerExtractor::new();
    let service = MemoryService::open(dir.path(), embedder)
        .expect("open memory store")
        .with_autograph(extractor.clone());
    (dir, McpServer::new(service), extractor, release)
}

/// Poll until `probe` answers true — the event, not the clock (#1793).
fn wait_for(deadline: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + deadline;
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn remember_tool_answers_while_the_autograph_gate_is_still_shut() {
    let (_dir, srv, _extractor, release) = gated_server();

    // The wiring itself, before any call: the constructor must have spawned
    // the worker and kept its handle. Without the handle there is no queue,
    // and without the queue `remember` runs the enrichment inline.
    assert!(
        srv._autograph_worker.is_some(),
        "constructing the server over an autograph-carrying service must \
         spawn the background worker and STORE its handle — the handle is \
         what makes the server's drop bound shutdown"
    );
    assert!(
        srv.service.autograph_queue_open(),
        "the spawned worker's queue must be open, so remember enqueues \
         instead of running the enrichment on the response path"
    );

    // The extractor is GATED shut: an inline autograph could not return at
    // all. The tool must answer on the write's own budget.
    let started = Instant::now();
    let Json(_stored) = srv
        .remember(Parameters(RememberParams {
            fact: "Alice Martin travaille chez Wiscale.".to_owned(),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "the remember TOOL must answer with the extractor gate still SHUT — \
         it took {elapsed:?}, so the enrichment sat on the server's response \
         path, the exact pre-#1851 behaviour"
    );

    // Deferred is not dropped: release the enrichment and require the entity
    // TOOL to see the wired edge within a bounded wait (an event poll).
    release.send(()).expect("release the gated extraction");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let Json(profile) = srv
            .entity(Parameters(EntityParams {
                name: "alice martin".to_owned(),
            }))
            .await
            .expect("entity");
        if profile.found && !profile.relations.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the deferred enrichment must eventually wire alice martin's \
             edge — deferred is not dropped"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn dropping_the_server_bounds_shutdown_and_skips_queued_jobs() {
    let (_dir, srv, extractor, release) = gated_server();
    let service = Arc::clone(&srv.service);

    // Three writes: job 0 is dequeued by the worker and BLOCKS on the gate;
    // jobs 1 and 2 sit in the queue. `entered == 1` is the event proving
    // job 0 is IN FLIGHT before the drop — not a wall-clock guess.
    for i in 0..3 {
        srv.remember(Parameters(RememberParams {
            fact: format!("fait en rafale numero {i}"),
            links: Vec::new(),
            metadata: None,
            ttl_seconds: None,
        }))
        .await
        .expect("remember");
    }
    assert!(
        wait_for(Duration::from_secs(5), || {
            extractor.entered.load(Ordering::SeqCst) == 1
        }),
        "the worker must have dequeued job 0 and be blocked on the gate"
    );

    // Drop the server on a side thread: the handle it stores is the ONLY
    // thing that closes the queue and joins the worker.
    let started = Instant::now();
    let joiner = std::thread::spawn(move || drop(srv));
    assert!(
        wait_for(Duration::from_secs(5), || !service.autograph_queue_open()),
        "dropping the server must close the autograph queue via the stored \
         worker handle"
    );
    release.send(()).expect("release the in-flight job");
    joiner.join().expect("join the dropping thread");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "shutdown is BOUNDED: it waits for at most the ONE in-flight \
         generation, never the queue behind it"
    );
    assert_eq!(
        extractor.completed.load(Ordering::SeqCst),
        1,
        "only the in-flight job is wired on shutdown"
    );
    assert_eq!(
        service.autograph_dropped(),
        2,
        "the two still-queued jobs are SKIPPED and counted, not waited out"
    );
}
