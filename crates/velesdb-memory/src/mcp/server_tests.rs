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

    srv.forget(Parameters(ForgetParams { id: stored.id }))
        .await
        .expect("forget");

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
