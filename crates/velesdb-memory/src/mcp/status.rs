//! Health and configuration status for the active memory generation.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Json;
use rmcp::{tool, tool_router, ErrorData};

use super::dto::{
    EmbedderStatus, ExtractionStatus, MemoryCounts, MemoryStatusResult, ProvenanceStatus,
};
use super::{join_error, to_error, McpServer, UNREPORTED_MODEL};
use crate::embedding_provenance::EmbeddingProvenance;

type StatusSnapshot = (
    String,
    usize,
    Option<EmbeddingProvenance>,
    usize,
    Option<usize>,
    bool,
    u64,
    u64,
);

#[tool_router(router = status_tool_router, vis = "pub(super)")]
impl McpServer {
    #[tool(
        name = "memory_status",
        output_schema = crate::schema::wire_safe_output_schema::<MemoryStatusResult>(),
        description = "Report server health/config: running embedder and whether recall is SEMANTIC (`embedder.semantic: false` = offline hash — surface matching, not meaning; semantic is an env-var switch, no rebuild), embedder provenance of the store, whether a default extractor is configured (`extraction.configured` lets `remember_extracted` omit `extractor`), autograph worker status, dropped/failed enrichment counts (fact stored, graph partial — re-remember to complete), corpus size (`memory.facts`, `memory.edges`). `edges: 0` means `why` has no graph to walk and degrades to plain search — a wiring gap, not a defect. Call at session start, or when recall quality or `why`'s trails surprise — and tell the user when the server runs degraded. No parameters."
    )]
    async fn memory_status(&self) -> Result<Json<MemoryStatusResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let store_dir = self.store_dir.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            let recorded = store_dir
                .as_deref()
                .and_then(|dir| crate::embedding_provenance::read(dir).ok().flatten());
            service.inspect_active(|model, dimension, current| {
                (
                    model.to_owned(),
                    dimension,
                    recorded,
                    current.fact_count(),
                    current.edge_count(),
                    current.autograph_queue_open(),
                    current.autograph_dropped(),
                    current.autograph_failed(),
                )
            })
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(self.status_result(snapshot)))
    }

    fn status_result(&self, snapshot: StatusSnapshot) -> MemoryStatusResult {
        let (
            model,
            dimension,
            provenance,
            facts,
            edges,
            autograph_active,
            autograph_dropped,
            autograph_failed,
        ) = snapshot;
        MemoryStatusResult {
            embedder: embedder_status(model, dimension),
            provenance: provenance_status(provenance),
            extraction: ExtractionStatus {
                configured: self.extractors.read().default_is_configured(),
                autograph_active,
                autograph_dropped,
                autograph_failed,
            },
            memory: MemoryCounts { facts, edges },
        }
    }
}

fn embedder_status(model: String, dimension: usize) -> EmbedderStatus {
    if model == UNREPORTED_MODEL {
        return EmbedderStatus {
            model: None,
            dimension: None,
            semantic: None,
        };
    }
    EmbedderStatus {
        semantic: Some(model != "hash"),
        model: Some(model),
        dimension: Some(dimension),
    }
}

fn provenance_status(record: Option<EmbeddingProvenance>) -> ProvenanceStatus {
    record.map_or(
        ProvenanceStatus {
            recorded: false,
            model: None,
            dimension: None,
        },
        |record| ProvenanceStatus {
            recorded: true,
            model: Some(record.model),
            dimension: Some(record.dimension),
        },
    )
}
