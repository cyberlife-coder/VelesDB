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

use crate::embedder::Embedder;
use crate::service::{Explanation, Link, MemoryService, Metadata, Recollection};

/// Default number of memories returned by `recall`.
const DEFAULT_RECALL_LIMIT: usize = 10;
/// Default hop budget for `why` traversal.
const DEFAULT_WHY_HOPS: usize = 2;

/// Boxed embedder so the served type is concrete (the rmcp macros and the
/// async runtime work most cleanly on a non-generic handler).
pub type DynEmbedder = Box<dyn Embedder + Send + Sync>;

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

// --- The server ------------------------------------------------------------

/// MCP server wrapping a [`MemoryService`].
#[derive(Clone)]
pub struct McpServer {
    service: Arc<MemoryService<DynEmbedder>>,
    tool_router: ToolRouter<McpServer>,
}

#[tool_router]
impl McpServer {
    /// Wrap a memory service as an MCP server.
    #[must_use]
    pub fn new(service: MemoryService<DynEmbedder>) -> Self {
        Self {
            service: Arc::new(service),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "remember",
        description = "Store a fact in durable local memory. Optionally link it to existing memories (graph) and tag it with structured metadata like project/author/type/status/date (ColumnStore) for later filtering. Returns the fact's stable id."
    )]
    async fn remember(
        &self,
        Parameters(params): Parameters<RememberParams>,
    ) -> Result<Json<RememberResult>, ErrorData> {
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
        let limit = params.limit.unwrap_or(DEFAULT_RECALL_LIMIT);
        let memories = self
            .service
            .recall(&params.query, limit, params.filter.as_ref())
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
        let max_hops = params.max_hops.unwrap_or(DEFAULT_WHY_HOPS);
        let explanation = self
            .service
            .why(&params.decision, max_hops, params.filter.as_ref())
            .map_err(to_error)?;
        Ok(Json(explanation))
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
/// Client-input errors (`EmptyFact`, `UnknownLinkTarget`) become
/// `invalid_params` (-32602) so callers can distinguish bad input from a
/// server fault without parsing the message string. Everything else is
/// `internal_error` (-32603).
///
/// Takes the error by value so it can be used as `.map_err(to_error)` at every
/// call site without a per-site closure.
#[allow(clippy::needless_pass_by_value)]
fn to_error(err: crate::error::MemoryError) -> ErrorData {
    use crate::error::MemoryError;
    let code = match &err {
        MemoryError::EmptyFact | MemoryError::UnknownLinkTarget(_) => ErrorCode::INVALID_PARAMS,
        _ => ErrorCode::INTERNAL_ERROR,
    };
    ErrorData::new(code, err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::HashEmbedder;
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

    // --- Error-code mapping -------------------------------------------------

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
            "UnknownLinkTarget must map to invalid_params"
        );
    }
}
