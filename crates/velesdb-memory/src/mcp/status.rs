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
        description = "Report this memory server's health and configuration: which embedder is running and whether recall is SEMANTIC (`embedder.semantic: false` means the offline `hash` default — recall matches surface form, not meaning, and configuring a semantic embedder is an env-var switch, no rebuild), what embedder the store was filled by per its on-disk provenance record, whether a default extraction backend is configured (`remember_extracted` may omit `extractor` iff `extraction.configured`; explicit `outline` remains available), whether the background autograph worker is active, how many enrichments a full queue dropped (never ran) and how many ran but failed part-way through wiring (fact stored, graph structure partial — re-remember it to complete), and the corpus size — `memory.facts` and `memory.edges`. Read `memory.edges` when `why` seems to add nothing over `recall`: `0` means no fact was ever linked (by `relate`, `remember`'s `links`, or extraction), so `why` HAS no graph to walk and degrades to plain search — that is a wiring gap, not a defect. Call this at session start, or whenever recall quality or `why`'s evidence trails surprise you, and tell the user when the server runs degraded. Takes no parameters."
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
