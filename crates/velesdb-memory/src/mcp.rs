//! MCP transport: exposes the memory service as MCP tools over stdio.
//!
//! Only **memory semantics** are exposed (`remember / recall / relate / forget
//! / why`) — never raw database capabilities. See [`crate`] docs for the
//! license boundary this enforces.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorCode, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::limits::{DEFAULT_WHY_HOPS, MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use crate::model::{ColumnFilter, Explanation, Link, Recollection};
use crate::service::{MemoryService, Metadata};

/// Default number of memories returned by `recall`.
const DEFAULT_RECALL_LIMIT: usize = 10;

// The boxed embedder and the shared, runtime-attached extraction backend the
// server stores — imported for internal use only. The canonical public paths are
// `velesdb_memory::DynEmbedder` / `velesdb_memory::DynExtractor` (crate root).
use crate::embedder::DynEmbedder;
use crate::extract::DynExtractor;

// --- Tool parameter / result DTOs ------------------------------------------
//
// Output shapes reuse the domain types from `crate::service` directly (they
// derive `Serialize` + `JsonSchema`), so there is no duplicate wire/domain
// struct. Only request envelopes and small id-results live here.

/// Parameters for the `remember` tool.
#[derive(Deserialize, JsonSchema)]
struct RememberParams {
    /// The fact to store in memory.
    fact: String,
    /// Optional typed links from this fact to existing memories.
    #[serde(default)]
    links: Vec<Link>,
    /// Optional structured metadata for later filtering (e.g.
    /// `{"project": "veles", "author": "julien", "status": "open"}`).
    metadata: Option<Metadata>,
}

/// Result of the `remember` tool.
#[derive(Serialize, JsonSchema)]
struct RememberResult {
    /// Stable id assigned to the remembered fact.
    id: u64,
}

/// Parameters for the `recall` tool.
#[derive(Deserialize, JsonSchema)]
struct RecallParams {
    /// Natural-language query to match semantically.
    query: String,
    /// Maximum number of memories to return (default 10).
    limit: Option<usize>,
    /// Optional exact-match metadata filter (e.g.
    /// `{"project": "veles", "status": "resolved"}`).
    filter: Option<Metadata>,
}

/// Result of the `recall` tool.
#[derive(Serialize, JsonSchema)]
struct RecallResult {
    /// Recalled memories, most similar first.
    memories: Vec<Recollection>,
}

/// Parameters for the `recall_where` tool.
#[derive(Deserialize, JsonSchema)]
struct RecallWhereParams {
    /// Natural-language query to match semantically.
    query: String,
    /// Maximum number of memories to return (default 10).
    limit: Option<usize>,
    /// Structured `ColumnStore` predicates (ranges/comparisons) combined with AND,
    /// e.g. a date window `[{"field":"ts","op":"ge","value":20230101},
    /// {"field":"ts","op":"le","value":20231231}]`. Each `op` is one of
    /// `eq`/`ne`/`lt`/`le`/`gt`/`ge`.
    #[serde(default)]
    filters: Vec<ColumnFilter>,
}

/// Parameters for the `relate` tool.
#[derive(Deserialize, JsonSchema)]
struct RelateParams {
    /// Source memory id.
    from: u64,
    /// Target memory id.
    to: u64,
    /// Relationship label.
    relation: String,
}

/// Result of the `relate` tool.
#[derive(Serialize, JsonSchema)]
struct RelateResult {
    /// Id of the created edge.
    edge_id: u64,
}

/// Parameters for the `forget` tool.
#[derive(Deserialize, JsonSchema)]
struct ForgetParams {
    /// Id of the memory to forget.
    id: u64,
}

/// Result of the `forget` tool.
#[derive(Serialize, JsonSchema)]
struct ForgetResult {
    /// Id of the forgotten memory.
    id: u64,
}

/// Parameters for the `why` tool.
#[derive(Deserialize, JsonSchema)]
struct WhyParams {
    /// The decision (or fact) to explain.
    decision: String,
    /// How many hops of typed links to follow (default 2).
    max_hops: Option<usize>,
    /// Optional exact-match metadata filter to scope the seed (e.g.
    /// `{"project": "veles"}`).
    filter: Option<Metadata>,
}

/// Parameters for the `remember_extracted` tool.
#[derive(Deserialize, JsonSchema)]
struct RememberExtractedParams {
    /// Raw text to extract atomic facts from and store as a connected graph.
    text: String,
    /// Optional structured metadata applied to every extracted fact.
    metadata: Option<Metadata>,
}

/// Result of the `remember_extracted` tool.
#[derive(Serialize, JsonSchema)]
struct RememberExtractedResult {
    /// Stable ids of the stored facts, in extraction order.
    ids: Vec<u64>,
}

// --- The server ------------------------------------------------------------

/// MCP server wrapping a [`MemoryService`].
#[derive(Clone)]
pub struct McpServer {
    service: Arc<MemoryService<DynEmbedder>>,
    /// Optional extraction backend powering `remember_extracted`. `None` unless
    /// a backend is attached via [`Self::with_extractor`]; the tool then reports
    /// extraction as unconfigured.
    extractor: Option<DynExtractor>,
    tool_router: ToolRouter<McpServer>,
}

#[tool_router]
impl McpServer {
    /// Wrap a memory service as an MCP server.
    #[must_use]
    pub fn new(service: MemoryService<DynEmbedder>) -> Self {
        Self {
            service: Arc::new(service),
            extractor: None,
            tool_router: Self::tool_router(),
        }
    }

    /// Attach an extraction backend, enabling the `remember_extracted` tool.
    /// Without it the tool reports that extraction is not configured.
    #[must_use]
    pub fn with_extractor(mut self, extractor: DynExtractor) -> Self {
        self.extractor = Some(extractor);
        self
    }

    #[tool(
        name = "remember",
        description = "Store a fact in durable local memory. Optionally link it to existing memories (graph) and tag it with structured metadata like project/author/type/status/date (ColumnStore) for later filtering. Returns the fact's stable id."
    )]
    async fn remember(
        &self,
        Parameters(params): Parameters<RememberParams>,
    ) -> Result<Json<RememberResult>, ErrorData> {
        if params.fact.len() > MAX_FACT_BYTES {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("fact exceeds maximum size of {MAX_FACT_BYTES} bytes"),
                None,
            ));
        }
        let id = self
            .service
            .remember(&params.fact, &params.links, params.metadata.as_ref())
            .map_err(to_error)?;
        Ok(Json(RememberResult { id }))
    }

    #[tool(
        name = "recall",
        description = "Recall memories semantically similar to a query (vector), most similar first. Optionally narrow to exact-match metadata via `filter` (ColumnStore), e.g. {\"project\":\"veles\",\"status\":\"resolved\"}."
    )]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let memories = self
            .service
            .recall(&params.query, limit, params.filter.as_ref())
            .map_err(to_error)?;
        Ok(Json(RecallResult { memories }))
    }

    #[tool(
        name = "recall_where",
        description = "Fused recall: semantically similar memories (vector) constrained by structured ColumnStore predicates over metadata — ranges and comparisons, not just equality. Each filter is {field, op (eq/ne/lt/le/gt/ge), value}, ANDed. Use for time-windowed or numeric-scoped recall, e.g. facts about a topic with `ts` in a date range. Most similar first."
    )]
    async fn recall_where(
        &self,
        Parameters(params): Parameters<RecallWhereParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let memories = self
            .service
            .recall_where(&params.query, limit, &params.filters)
            .map_err(to_error)?;
        Ok(Json(RecallResult { memories }))
    }

    #[tool(
        name = "relate",
        description = "Create a typed link from one memory to another. Returns the edge id."
    )]
    async fn relate(
        &self,
        Parameters(params): Parameters<RelateParams>,
    ) -> Result<Json<RelateResult>, ErrorData> {
        let edge_id = self
            .service
            .relate(params.from, params.to, &params.relation)
            .map_err(to_error)?;
        Ok(Json(RelateResult { edge_id }))
    }

    #[tool(name = "forget", description = "Delete a memory by id.")]
    async fn forget(
        &self,
        Parameters(params): Parameters<ForgetParams>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        self.service.forget(params.id).map_err(to_error)?;
        Ok(Json(ForgetResult { id: params.id }))
    }

    #[tool(
        name = "why",
        description = "Explain a decision: find the best-matching memory (optionally scoped by a metadata `filter`, e.g. the current project) and return the connected subgraph of related memories reachable through typed links — fusing vector, ColumnStore, and graph to surface context a plain similarity search misses."
    )]
    async fn why(
        &self,
        Parameters(params): Parameters<WhyParams>,
    ) -> Result<Json<Explanation>, ErrorData> {
        let max_hops = params
            .max_hops
            .unwrap_or(DEFAULT_WHY_HOPS)
            .min(MAX_WHY_HOPS);
        let explanation = self
            .service
            .why(&params.decision, max_hops, params.filter.as_ref())
            .map_err(to_error)?;
        Ok(Json(explanation))
    }

    #[tool(
        name = "remember_extracted",
        description = "Store a passage of raw text by extracting its atomic facts and auto-building the fact↔topic graph, so `why` can later connect them with no manual links. Requires the server to be started with an extraction backend (set VELESDB_MEMORY_EXTRACTOR; build with --features extract). Returns the stored facts' ids."
    )]
    async fn remember_extracted(
        &self,
        Parameters(params): Parameters<RememberExtractedParams>,
    ) -> Result<Json<RememberExtractedResult>, ErrorData> {
        if params.text.len() > MAX_FACT_BYTES {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("text exceeds maximum size of {MAX_FACT_BYTES} bytes"),
                None,
            ));
        }
        let Some(extractor) = self.extractor.clone() else {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "extraction backend not configured: start the server with \
                 VELESDB_MEMORY_EXTRACTOR set (built with --features extract)",
                None,
            ));
        };
        // Extraction makes a blocking network call (up to the extractor's
        // timeout), so run it off the async worker pool to keep the stdio loop
        // responsive to other tool calls and cancellations.
        let service = Arc::clone(&self.service);
        let RememberExtractedParams { text, metadata } = params;
        let ids = tokio::task::spawn_blocking(move || {
            service.remember_extracted(&text, &extractor, metadata.as_ref())
        })
        .await
        .map_err(|join| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("extraction task failed: {join}"),
                None,
            )
        })?
        .map_err(to_error)?;
        Ok(Json(RememberExtractedResult { ids }))
    }
}

/// `#[tool_handler]` generates `call_tool` / `list_tools` from the router;
/// `get_info` is overridden so the server identifies itself as `velesdb-memory`
/// (the macro default falls back to rmcp's own identity). Per-tool guidance
/// lives in each `#[tool(description = …)]`.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Local-first memory for AI agents: remember facts, recall them semantically, \
             relate them, forget them, and ask why a decision was made (connected subgraph)."
                .to_owned(),
        );
        info
    }
}

/// Map a domain error to an MCP error.
///
/// Map a [`MemoryError`](crate::error::MemoryError) onto a JSON-RPC error,
/// driven by its transport-neutral [`ErrorCategory`](crate::error::ErrorCategory)
/// so the MCP taxonomy can never drift from the bindings'. Client-input errors
/// become `invalid_params` (-32602); genuine faults `internal_error` (-32603).
/// JSON-RPC defines no "not found" code, so a missing id is reported as
/// `invalid_params` (a bad id is, from the protocol's view, a bad parameter).
///
/// Takes the error by value so it can be used as `.map_err(to_error)` at every
/// call site without a per-site closure.
#[allow(clippy::needless_pass_by_value)]
fn to_error(err: crate::error::MemoryError) -> ErrorData {
    use crate::error::ErrorCategory;
    let code = match err.category() {
        ErrorCategory::InvalidInput | ErrorCategory::NotFound => ErrorCode::INVALID_PARAMS,
        ErrorCategory::Internal => ErrorCode::INTERNAL_ERROR,
    };
    ErrorData::new(code, err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::HashEmbedder;
    use crate::model::ColumnOp;
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
            }))
            .await
            .expect("remember decision");
        let Json(pr) = srv
            .remember(Parameters(RememberParams {
                fact: "PR #42 swaps the mutex".to_owned(),
                links: Vec::new(),
                metadata: None,
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
            }))
            .await
            .expect("remember veles");
        let Json(dropped) = srv
            .remember(Parameters(RememberParams {
                fact: "auth bug in login too".to_owned(),
                links: Vec::new(),
                metadata: Some(acme_meta),
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
}
