use std::sync::Arc;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router, ErrorData};

use super::dto::{
    ExplanationDto, ExtractionJobStatusParams, ExtractionJobStatusResult, ListMemoriesParams,
    ListMemoriesResult, ListedMemoryDto, RecallFusedParams, RecallFusedResult,
    RememberExtractedParams, RememberExtractedResult, WhyParams,
};
use super::{id_wire_input_schema, job_error, join_error, to_error, McpServer};
use crate::limits::{DEFAULT_WHY_HOPS, MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use crate::model::FusionOptions;

const DEFAULT_LIST_LIMIT: usize = 50;
const DEFAULT_RECALL_LIMIT: usize = 10;

#[tool_router(router = advanced_tool_router, vis = "pub(super)")]
impl McpServer {
    #[tool(
        name = "recall_fused",
        output_schema = crate::schema::wire_safe_output_schema::<RecallFusedResult>(),
        description = "Fused vector + graph recall: like `recall`, but also walks the graph from the top vector hit and folds any connected fact into the ranking — the tri-engine ranking (vector similarity + ColumnStore filter + graph reach) measured on multi-hop and temporal benchmarks. Reach for this when an answer needs a fact the query doesn't mention directly but a stored `relate`/extracted link connects (multi-hop reasoning, temporal chains). `hops`/`graph_boost` tune the graph reach and `pool` the depth of the vector candidate pool fusion re-ranks; omit them for the proven defaults. Optionally narrow with an exact-match `filter`. Set `date_field` (the metadata key holding a YYYYMMDD date) to also get a `dated_context` timeline and a `now` anchor for temporal questions. Most relevant first."
    )]
    pub(super) async fn recall_fused(
        &self,
        Parameters(params): Parameters<RecallFusedParams>,
    ) -> Result<Json<RecallFusedResult>, ErrorData> {
        let k = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let opts = FusionOptions::from_knobs(params.hops, params.graph_boost, params.pool);
        let service = Arc::clone(&self.service);
        let RecallFusedParams {
            query,
            filter,
            date_field,
            ..
        } = params;
        let (memories, dated_context, now) = if let Some(field) = date_field {
            let (hits, ctx) = tokio::task::spawn_blocking(move || {
                service.run(|current| {
                    current.recall_fused_dated(&query, k, filter.as_ref(), opts, &field)
                })
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, Some(ctx.timeline), ctx.now)
        } else {
            let hits = tokio::task::spawn_blocking(move || {
                service.run(|current| current.recall_fused(&query, k, filter.as_ref(), opts))
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, None, None)
        };
        Ok(Json(RecallFusedResult::new(memories, dated_context, now)))
    }

    #[tool(
        name = "why",
        output_schema = crate::schema::wire_safe_output_schema::<ExplanationDto>(),
        description = "Explain a decision: find the best-matching memory (optionally scoped by a metadata `filter`, e.g. the current project) and return the connected subgraph of related memories reachable through typed links — fusing vector, ColumnStore, and graph to surface context a plain similarity search misses."
    )]
    pub(super) async fn why(
        &self,
        Parameters(params): Parameters<WhyParams>,
    ) -> Result<Json<ExplanationDto>, ErrorData> {
        let max_hops = params
            .max_hops
            .unwrap_or(DEFAULT_WHY_HOPS)
            .min(MAX_WHY_HOPS);
        let service = Arc::clone(&self.service);
        let WhyParams {
            decision, filter, ..
        } = params;
        let explanation = tokio::task::spawn_blocking(move || {
            service.run(|current| current.why(&decision, max_hops, filter.as_ref()))
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(ExplanationDto::from(explanation)))
    }

    #[tool(
        name = "remember_extracted",
        output_schema = crate::schema::wire_safe_output_schema::<RememberExtractedResult>(),
        description = "Accept a passage for durable background extraction and return before model generation. Set `extractor` per call (`outline`, `ollama`, or `openai`), or omit it to use VELESDB_MEMORY_EXTRACTOR. Supply `idempotency_key` when retrying across a client timeout: the same key and payload reuse one job, while a changed payload is rejected. The receipt returns `request_id`, its initial `state` (`accepted`, or the persisted state of a reused request), and `reused`. Poll `extraction_status(request_id)` until `committed` or `failed`; accepted/running jobs survive process restart."
    )]
    pub(super) async fn remember_extracted(
        &self,
        Parameters(params): Parameters<RememberExtractedParams>,
    ) -> Result<Json<RememberExtractedResult>, ErrorData> {
        let RememberExtractedParams {
            text,
            metadata,
            extractor,
            idempotency_key,
        } = params;
        if text.len() > MAX_FACT_BYTES {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("text exceeds maximum size of {MAX_FACT_BYTES} bytes"),
                None,
            ));
        }
        let jobs = self.extraction_jobs.clone().ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "durable extraction jobs are not configured for this server",
                None,
            )
        })?;
        let receipt = tokio::task::spawn_blocking(move || {
            jobs.submit(
                &text,
                metadata,
                extractor.as_deref(),
                idempotency_key.as_deref(),
            )
        })
        .await
        .map_err(join_error)?
        .map_err(job_error)?;
        Ok(Json(RememberExtractedResult {
            request_id: receipt.request_id,
            state: receipt.state,
            reused: receipt.reused,
        }))
    }

    #[tool(
        name = "extraction_status",
        output_schema = crate::schema::wire_safe_output_schema::<ExtractionJobStatusResult>(),
        description = "Read one durable extraction job by the `request_id` returned from `remember_extracted`. Returns that `request_id`, its persisted `state` (`accepted`, `running`, `committed`, or `failed`), committed fact `ids` and their u64-safe decimal `ids_str` twins, `skipped_over_cap` after commit, and `error` after failure. While accepted/running, `ids` and `ids_str` are empty and both optional terminal fields are null."
    )]
    pub(super) async fn extraction_status(
        &self,
        Parameters(params): Parameters<ExtractionJobStatusParams>,
    ) -> Result<Json<ExtractionJobStatusResult>, ErrorData> {
        let jobs = self.extraction_jobs.clone().ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "durable extraction jobs are not configured for this server",
                None,
            )
        })?;
        let view = tokio::task::spawn_blocking(move || jobs.status(&params.request_id))
            .await
            .map_err(join_error)?
            .map_err(job_error)?;
        let (ids, skipped_over_cap) = view.outcome.map_or_else(
            || (Vec::new(), None),
            |outcome| (outcome.ids, Some(outcome.skipped_over_cap)),
        );
        let ids_str = ids.iter().map(u64::to_string).collect();
        Ok(Json(ExtractionJobStatusResult {
            request_id: view.request_id,
            state: view.state,
            ids,
            ids_str,
            skipped_over_cap,
            error: view.error,
        }))
    }

    #[tool(
        name = "list_memories",
        output_schema = crate::schema::wire_safe_output_schema::<ListMemoriesResult>(),
        input_schema = id_wire_input_schema::<ListMemoriesParams>(&["cursor"]),
        description = "AUDIT the store: walk every stored fact, page by page — the question `recall` structurally cannot answer, because recall ranks by resemblance to a query and what resembles nothing you thought to ask stays invisible. Use it when the user asks what the memory contains ('what do you know about me / this project?'), to review or clean up before sharing a store, or to back up its contents. Returns `memories` (ids ascending — two audits of the same store see the same order; each entry carries `id`, `id_str`, `content`, `metadata`) and `next_cursor`: pass it back as `cursor` for the next page, `null` means the walk is complete. `filter` keeps only facts whose metadata equals every given key (e.g. {\"project\": \"acme\"}); a filtered page may come back sparse — KEEP following `next_cursor`, the walk stays exhaustive. Metadata follows recall's visibility rule (business keys plus the auto-stamped `_veles_date`; internal graph scaffolding excluded) unless `include_internal` is set, which lists everything verbatim. Ids exceed 2^53 — always relay them as strings (`id_str`, and `next_cursor` is already a string)."
    )]
    pub(super) async fn list_memories(
        &self,
        Parameters(params): Parameters<ListMemoriesParams>,
    ) -> Result<Json<ListMemoriesResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let ListMemoriesParams {
            cursor,
            limit,
            filter,
            include_internal,
        } = params;
        let (memories, next) = tokio::task::spawn_blocking(move || {
            service.run(|current| {
                current.list(
                    cursor,
                    limit.unwrap_or(DEFAULT_LIST_LIMIT),
                    filter.as_ref(),
                    include_internal,
                )
            })
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(ListMemoriesResult {
            memories: memories
                .into_iter()
                .map(|memory| ListedMemoryDto {
                    id: memory.id,
                    id_str: memory.id.to_string(),
                    content: memory.content,
                    metadata: memory.metadata,
                })
                .collect(),
            next_cursor: next.map(|id| id.to_string()),
        }))
    }
}
