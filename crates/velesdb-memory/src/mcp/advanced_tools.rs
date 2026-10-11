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
        description = "Fused vector + graph recall: like `recall`, plus walks the graph from the top hit and folds connected facts into the ranking — for answers needing a fact the query doesn't name (multi-hop reasoning, temporal chains). `hops`/`graph_boost` tune graph reach, `pool` the vector candidate depth; omit for the proven defaults. Optional exact-match `filter`; set `date_field` (a YYYYMMDD metadata key) for a `dated_context` timeline and `now` anchor. Most relevant first."
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
        description = "Accept a passage for durable background extraction; returns before model generation. `extractor` per call (`outline`/`ollama`/`openai`), else VELESDB_MEMORY_EXTRACTOR. `idempotency_key` makes a retry reuse one job; same key with a changed payload is rejected. The receipt returns `request_id`, initial `state` (`accepted`, or the reused request's persisted state) and `reused`. Poll `extraction_status(request_id)` until `committed`/`failed`; accepted/running jobs survive restart."
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
        description = "AUDIT the store: page through every stored fact — what `recall` cannot answer (it only ranks by resemblance). Use for 'what do you know about me/this project?', cleanup before sharing, or backup. Returns `memories` (ids ascending; each entry carries `id`, `id_str`, `content`, `metadata`) and `next_cursor`: pass it as `cursor` for the next page, null = done. `filter` keeps facts matching every key; a sparse page is not the end — KEEP following `next_cursor`. `include_internal` also lists internal scaffolding verbatim. Ids exceed 2^53 — relay as strings."
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
