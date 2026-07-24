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
    let Json(result) = srv
        .recall_fused(Parameters(RecallFusedParams {
            query: "anything".to_owned(),
            limit: Some(usize::MAX),
            filter: None,
            hops: Some(usize::MAX),
            graph_boost: None,
            date_field: None,
        }))
        .await
        .expect("recall_fused with huge limit/hops must succeed (silently capped)");
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
        "filter": "{\"project\": \"velesdb\"}"
    }))
    .expect("stringified scalar/object arguments must deserialize");
    assert_eq!(params.limit, Some(6));
    assert_eq!(params.hops, Some(2));
    assert!((params.graph_boost.unwrap() - 0.15).abs() < f64::EPSILON);
    let filter = params.filter.expect("filter must parse from a JSON string");
    assert_eq!(
        filter.get("project").and_then(|v| v.as_str()),
        Some("velesdb")
    );
}
