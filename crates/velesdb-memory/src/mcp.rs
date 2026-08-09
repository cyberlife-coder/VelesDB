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

use crate::limits::{DEFAULT_WHY_HOPS, MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use crate::model::FusionOptions;
use crate::service::MemoryService;

/// Default number of memories returned by `recall`.
const DEFAULT_RECALL_LIMIT: usize = 10;

/// Default page size of `list_memories` — bigger than recall's because an
/// audit walks everything anyway and pages are pure I/O, no ranking.
const DEFAULT_LIST_LIMIT: usize = 50;

// The boxed embedder and the shared, runtime-attached extraction backend the
// server stores — imported for internal use only. The canonical public paths are
// `velesdb_memory::DynEmbedder` / `velesdb_memory::DynExtractor` (crate root).
use crate::embedder::DynEmbedder;
use crate::extract::DynExtractor;

// --- Tool parameter / result DTOs ------------------------------------------
//
// The request envelopes, small id-results, and the id-echoing wire wrappers
// (`RecollectionDto`, `ExplanationDto` — the `id_str` twins of issue #1468)
// live in their own module so this file stays focused on the server and tool
// wiring; the domain types in `crate::model` are unchanged.
/// The context compiler's eight tools — a second `#[tool_router]` block whose
/// router is combined with the main one below, extending the ONE server.
#[cfg(feature = "context")]
mod context_tools;

mod dto;
mod wire;
use dto::{
    EmbedderStatus, EntityParams, EntityProfileDto, ExplanationDto, ExtractionStatus,
    FeedbackParams, FeedbackResult, ForgetParams, ForgetResult, ListMemoriesParams,
    ListMemoriesResult, ListedMemoryDto, MemoryCounts, MemoryStatusResult, ProvenanceStatus,
    RecallFusedParams, RecallFusedResult, RecallParams, RecallResult, RecallWhereParams,
    RelateParams, RelateResult, RememberExtractedParams, RememberExtractedResult, RememberParams,
    RememberResult, UnrelateParams, UnrelateResult, WhyParams,
};

/// Le constructeur de schema d'ENTREE, unique pour les vingt-deux outils :
/// [`crate::schema::wire_safe_input_schema`].
///
/// `keys` nomme les proprietes que CET outil accepte en chaine decimale
/// (`relate`/`unrelate` : `from`/`to` ; `forget`/`feedback` : `id` ;
/// `remember` : le `links[].target` imbrique) — la tolerance d'un id est une
/// connaissance de l'outil, jamais une regle globale :
/// `explain_compilation.fragment_id` est un `u64` STRICT et reste annonce
/// `integer`.
///
/// Il y avait deux constructeurs, celui-ci et un `wire_safe_input_schema`
/// gate sur `context` a cle `"id"` figee ; ils appliquaient la meme suite de
/// passes a une virgule pres. Il n'y en a plus qu'un.
use crate::schema::wire_safe_input_schema as id_wire_input_schema;

// --- The server ------------------------------------------------------------

/// MCP server wrapping a [`MemoryService`].
#[derive(Clone)]
pub struct McpServer {
    service: Arc<MemoryService<DynEmbedder>>,
    /// Join guard of the background autograph worker (#1846) — present iff
    /// an autograph extractor is configured. Held only for its `Drop`: the
    /// server going down closes the queue and joins the worker — the job in
    /// flight completes, still-queued ones are skipped and counted, so exit
    /// waits for at most ONE generation.
    _autograph_worker: Option<Arc<crate::service::AutographWorkerHandle>>,
    /// Optional extraction backend powering `remember_extracted`. `None` unless
    /// a backend is attached via [`Self::with_extractor`]; the tool then reports
    /// extraction as unconfigured.
    extractor: Option<DynExtractor>,
    /// Default time-to-live (seconds) applied to `remember`d facts that don't
    /// specify their own `ttl_seconds`. `None` (the default) stores permanently.
    /// Set from `VELESDB_MEMORY_DEFAULT_TTL` by the binary.
    default_ttl: Option<u64>,
    /// The embedder identity the host declared (model name + dimension) —
    /// what `memory_status` reports as the RUNNING embedder. `None` when the
    /// host embedded this server without declaring one; the status then says
    /// "unreported" rather than guessing. Set via
    /// [`Self::with_embedder_identity`] by the binary, which is the one
    /// place that knows what `VELESDB_MEMORY_EMBEDDER` resolved to.
    embedder_identity: Option<(String, usize)>,
    /// The store directory, for reading the embedding-provenance record
    /// (#1751) in `memory_status`. `None` disables the provenance block
    /// (reported as unrecorded — a store nobody can locate has no readable
    /// record either way).
    store_dir: Option<std::path::PathBuf>,
    /// Allowlisted filesystem roots for `path`-referenced context fragments
    /// (V2b-1). `None` (the default) disables path ingestion entirely — every
    /// `path` fragment fails with an explicit error. Set from
    /// `VELESDB_MEMORY_INGEST_ROOTS` by the binary via [`Self::with_ingest_roots`].
    #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
    ingest_roots: Option<crate::context::IngestRoots>,
    tool_router: ToolRouter<McpServer>,
}

#[tool_router]
impl McpServer {
    /// Wrap a memory service as an MCP server.
    #[must_use]
    pub fn new(service: MemoryService<DynEmbedder>) -> Self {
        let service = Arc::new(service);
        // Autograph leaves the response path here (#1846): with an extractor
        // configured, ONE background worker consumes a bounded queue and
        // `remember` returns as soon as the fact is stored — measured 46-52 s
        // inline against a 0.12 s embedding. The handle rides the server so
        // shutdown finishes the job in flight and skips the rest, counted.
        let autograph_worker = if service.has_autograph() {
            match service.spawn_autograph_worker(crate::limits::MAX_AUTOGRAPH_QUEUE) {
                Ok(handle) => Some(Arc::new(handle)),
                Err(error) => {
                    tracing::warn!(%error, "autograph worker not spawned; falling back inline");
                    None
                }
            }
        } else {
            None
        };
        Self {
            service,
            _autograph_worker: autograph_worker,
            extractor: None,
            default_ttl: None,
            embedder_identity: None,
            store_dir: None,
            #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
            ingest_roots: None,
            tool_router: Self::combined_router(),
        }
    }

    /// Declare the running embedder's identity (model name + vector width)
    /// so `memory_status` can name it — and say whether recall is semantic.
    /// Undeclared, the status reports the embedder as unreported rather than
    /// guessing from the service, which only ever sees `&[f32]`.
    #[must_use]
    pub fn with_embedder_identity(mut self, model: impl Into<String>, dimension: usize) -> Self {
        self.embedder_identity = Some((model.into(), dimension));
        self
    }

    /// Point `memory_status` at the store directory so it can relay the
    /// embedding-provenance record (#1751). Without it the provenance block
    /// reports `recorded: false`.
    #[must_use]
    pub fn with_store_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.store_dir = Some(dir.into());
        self
    }

    /// The full tool router: the memory tools, plus the context compiler's
    /// tools when that feature is on. Combined here — rmcp routers add — so
    /// there is exactly ONE server whichever features are enabled.
    ///
    /// **Et le point de passage unique du durcissement d'entree.** Mesure du
    /// 2026-07-29 : 10 outils sur 20 ne declaraient AUCUN `input_schema`
    /// (`recall`, `recall_fused`, `entity`, `why`, `remember_extracted`,
    /// `context_savings`, `retrieve_context_source`, `load_working_context`,
    /// `list_working_contexts`, `suggest_budget`) — leur schema etait celui
    /// derive par rmcp, que rien ne post-traitait. Un durcissement declare
    /// outil par outil laisse donc chaque route future non protegee, et rien
    /// ne le signale : c'est une omission, pas une erreur. Ici, une route
    /// nouvelle est couverte parce qu'elle EXISTE.
    ///
    /// L'attribut `#[tool(input_schema = …)]` garde ce qui, lui, est
    /// vraiment per-outil : les cles d'id que l'outil accepte en chaine.
    fn combined_router() -> ToolRouter<McpServer> {
        #[cfg(feature = "context")]
        let mut router = Self::tool_router() + Self::context_tool_router();
        #[cfg(not(feature = "context"))]
        let mut router = Self::tool_router();

        // `reharden_tool_input` prend l'outil, pas un schema : `Tool` type
        // ses deux schemas identiquement, donc c'est la signature — et non
        // une convention de nommage — qui rend la sortie inatteignable ici.
        for route in router.map.values_mut() {
            crate::schema::reharden_tool_input(&mut route.attr);
        }
        assert_every_input_slot_is_typed(&router);
        router
    }

    /// Attach an extraction backend, enabling the `remember_extracted` tool.
    /// Without it the tool reports that extraction is not configured.
    #[must_use]
    pub fn with_extractor(mut self, extractor: DynExtractor) -> Self {
        self.extractor = Some(extractor);
        self
    }

    /// Apply a default TTL (seconds) to `remember`d facts that don't carry their
    /// own `ttl_seconds`. `0` is treated as "no default" (permanent).
    #[must_use]
    pub fn with_default_ttl(mut self, ttl_seconds: u64) -> Self {
        self.default_ttl = (ttl_seconds > 0).then_some(ttl_seconds);
        self
    }

    /// Enable path ingestion (V2b-1): `compile_context` and
    /// `explain_compilation` fragments carrying `path` are resolved against
    /// this allowlist before compilation. Without this (the default), every
    /// `path` fragment fails with an explicit "ingestion disabled" error —
    /// same pattern as [`Self::with_extractor`].
    #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
    #[must_use]
    pub fn with_ingest_roots(mut self, roots: crate::context::IngestRoots) -> Self {
        self.ingest_roots = Some(roots);
        self
    }

    #[tool(
        name = "remember",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<RememberResult>(),
        description = "Store a fact in durable local memory. Optionally link it to existing memories (graph) and tag it with structured metadata like project/author/type/status/date (ColumnStore) for later filtering — metadata is capped at 64 KiB serialized. A fact is capped at 2048 bytes: that is roughly what the embedding model's context window holds, and a longer one is REFUSED with its size, not silently mangled — split a long passage into several atomic facts, or compile it with `compile_context` and remember a summary. Set `ttl_seconds` to make the fact expire after a delay (a durable TTL that survives restarts); omit it for a permanent memory — `ttl_seconds: 0` is refused, not read as \"never\". Returns the fact's stable id. With the async autograph worker active, edges derived from a remember land asynchronously: an `entity`/`why` read immediately after may not see them yet — the fact itself is always immediately readable. Ids exceed 2^53 — always relay them as strings (`id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients.",
        input_schema = id_wire_input_schema::<RememberParams>(&["target"])
    )]
    async fn remember(
        &self,
        Parameters(params): Parameters<RememberParams>,
    ) -> Result<Json<RememberResult>, ErrorData> {
        // No size pre-check here: `MemoryService::remember_with_ttl` refuses an
        // over-long fact itself (`MAX_EMBEDDABLE_TEXT_BYTES`, far below
        // `MAX_FACT_BYTES`), with a message naming the cap AND the received
        // size — so every adapter reports the same thing, from one place.
        let service = Arc::clone(&self.service);
        let RememberParams {
            fact,
            links,
            metadata,
            ttl_seconds,
        } = params;
        let ttl = ttl_seconds.or(self.default_ttl);
        let id = tokio::task::spawn_blocking(move || {
            service.remember_with_ttl(&fact, &links, metadata.as_ref(), ttl)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(RememberResult {
            id,
            id_str: id.to_string(),
        }))
    }

    #[tool(
        name = "recall",
        // rmcp derives an output schema when none is given, and that
        // derived form keeps `$ref`s a `$defs`-blind client cannot resolve.
        output_schema = crate::schema::wire_safe_output_schema::<RecallResult>(),
        description = "Recall memories semantically similar to a query (vector). Ranking blends similarity with each fact's learned confidence (see `feedback`), so the order is not pure similarity — the returned `score` is always the raw similarity, never the blended value. Optionally narrow to exact-match metadata via `filter` (ColumnStore), e.g. {\"project\":\"veles\",\"status\":\"resolved\"}. Ids exceed 2^53 — always relay them as strings (`id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients."
    )]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let service = Arc::clone(&self.service);
        let RecallParams { query, filter, .. } = params;
        let memories =
            tokio::task::spawn_blocking(move || service.recall(&query, limit, filter.as_ref()))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(RecallResult::new(memories)))
    }

    #[tool(
        name = "recall_where",
        // rmcp derives an output schema when none is given, and that
        // derived form keeps `$ref`s a `$defs`-blind client cannot resolve.
        output_schema = crate::schema::wire_safe_output_schema::<RecallResult>(),
        description = "Fused recall: semantically similar memories (vector) constrained by structured ColumnStore predicates over metadata — ranges and comparisons, not just equality. Each filter is {field, op (eq/ne/lt/le/gt/ge), value}, ANDed. Use for time-windowed or numeric-scoped recall, e.g. facts about a topic with `ts` in a date range. Comparisons are TYPE-STRICT, with no runtime coercion: a filter value of 20230601 (a JSON number) never matches a fact stored with metadata {\"ts\": \"20230601\"} (a JSON string) — same value, different JSON type, no match, no error. Store comparable values like dates NUMERICALLY at `remember` time (e.g. 20230601, not \"20230601\") so `recall_where` filters actually match them. Most similar first. Returns your own memories ONLY: entity hubs and the context compiler's artefacts (stored sources, compilation events, working contexts and their index) are internal scaffolding and never come back, whatever the predicate — including a `ne` one, which matches facts that lack the field entirely.",
        // No id-named parameter here (hence the empty `keys`) — this goes
        // through the shared helper purely for its `$ref` inlining, so
        // `filters[]` advertises `ColumnFilter`'s own fields instead of a
        // `$ref` a `$defs`-blind harness reads as "array of anything".
        input_schema = id_wire_input_schema::<RecallWhereParams>(&[])
    )]
    async fn recall_where(
        &self,
        Parameters(params): Parameters<RecallWhereParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let service = Arc::clone(&self.service);
        let RecallWhereParams { query, filters, .. } = params;
        let memories =
            tokio::task::spawn_blocking(move || service.recall_where(&query, limit, &filters))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(RecallResult::new(memories)))
    }

    #[tool(
        name = "recall_fused",
        // rmcp derives an output schema when none is given, and that
        // derived form keeps `$ref`s a `$defs`-blind client cannot resolve.
        output_schema = crate::schema::wire_safe_output_schema::<RecallFusedResult>(),
        description = "Fused vector + graph recall: like `recall`, but also walks the graph from the top vector hit and folds any connected fact into the ranking — the tri-engine ranking (vector similarity + ColumnStore filter + graph reach) measured on multi-hop and temporal benchmarks. Reach for this when an answer needs a fact the query doesn't mention directly but a stored `relate`/extracted link connects (multi-hop reasoning, temporal chains). `hops`/`graph_boost` tune the graph reach and `pool` the depth of the vector candidate pool fusion re-ranks; omit them for the proven defaults. Optionally narrow with an exact-match `filter`. Set `date_field` (the metadata key holding a YYYYMMDD date) to also get a `dated_context` timeline and a `now` anchor for temporal questions. Most relevant first."
    )]
    async fn recall_fused(
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
        // With a date field, take the shared "recall then format" path so the
        // dated timeline stays identical to the Node/WASM bindings; without one,
        // plain fused recall (no timeline).
        let (memories, dated_context, now) = if let Some(field) = date_field {
            let (hits, ctx) = tokio::task::spawn_blocking(move || {
                service.recall_fused_dated(&query, k, filter.as_ref(), opts, &field)
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, Some(ctx.timeline), ctx.now)
        } else {
            let hits = tokio::task::spawn_blocking(move || {
                service.recall_fused(&query, k, filter.as_ref(), opts)
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, None, None)
        };
        Ok(Json(RecallFusedResult::new(memories, dated_context, now)))
    }

    #[tool(
        name = "feedback",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<FeedbackResult>(),
        description = "Reinforce a recalled memory with an outcome: `success=true` if the fact was useful, `false` if it was noise. This durably updates the fact's learned confidence, which `recall` uses to re-rank future results — over repeated feedback, useful facts drift up and noise drifts down, so the memory improves with use without retraining the model. Returns the fact's new confidence in [0,1].",
        input_schema = id_wire_input_schema::<FeedbackParams>(&["id"])
    )]
    async fn feedback(
        &self,
        Parameters(params): Parameters<FeedbackParams>,
    ) -> Result<Json<FeedbackResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let FeedbackParams { id, success } = params;
        let confidence = tokio::task::spawn_blocking(move || service.feedback(id, success))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(FeedbackResult {
            id,
            id_str: id.to_string(),
            confidence,
        }))
    }

    #[tool(
        name = "relate",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<RelateResult>(),
        description = "Create a typed, directional link between two memories (`from` → `to`) labeled by `relation`. These links are the graph edges that `why` and `recall_fused` later traverse to surface connected facts that share no words with the query — build the graph with `relate` so multi-hop reasoning works (e.g. link a decision to its cause, a fact to its source, a task to the person it concerns). Direction matters: traversal follows OUTGOING edges only, so point `from` at the memory you will later ask `why` about and `to` at its evidence (decision → cause, fact → source) — an edge pointing INTO a memory is invisible to `why(that memory)`. Idempotent per (from, relation, to); `from` and `to` must be DIFFERENT memories — a self-loop states nothing and only adds noise to `why`'s evidence trail, so it is refused. Returns the edge id as `edge_id`, plus `edge_id_str` for clients without u64-safe JSON number parsing — the one already there when this exact relation exists, since the call is idempotent. Ids exceed 2^53 — always relay them as strings (`edge_id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients.",
        input_schema = id_wire_input_schema::<RelateParams>(&["from", "to"])
    )]
    async fn relate(
        &self,
        Parameters(params): Parameters<RelateParams>,
    ) -> Result<Json<RelateResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let RelateParams { from, to, relation } = params;
        let edge_id = tokio::task::spawn_blocking(move || service.relate(from, to, &relation))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(RelateResult {
            edge_id,
            edge_id_str: edge_id.to_string(),
        }))
    }

    #[tool(
        name = "unrelate",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<UnrelateResult>(),
        description = "Remove the typed link `from` -relation-> `to` — `relate`'s exact undo, so a mistaken edge no longer costs the facts at its endpoints. Only the edge is removed: the two memories, and any entity, are untouched. Idempotent: removing an absent edge answers `found: false` instead of erroring, so a cleanup can be replayed safely; `removed` counts the edges actually deleted. It refuses exactly what `relate` refuses (empty relation, `from` == `to`). Scope: the store does not distinguish a link you created with `relate` from one auto-derived from a passage, so `unrelate` removes both alike — to correct an auto-derived link, prefer `forget` + `remember` of the source fact, otherwise remembering the same passage again can rebuild the edge removed here. Same id wire contract as `relate`: pass ids as decimal strings (`id_str`) — a JSON-number id above 2^53 loses precision on float-lossy clients.",
        input_schema = id_wire_input_schema::<UnrelateParams>(&["from", "to"])
    )]
    async fn unrelate(
        &self,
        Parameters(params): Parameters<UnrelateParams>,
    ) -> Result<Json<UnrelateResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let UnrelateParams { from, to, relation } = params;
        let outcome = tokio::task::spawn_blocking(move || service.unrelate(from, to, &relation))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(UnrelateResult {
            found: outcome.found,
            removed: outcome.removed,
        }))
    }

    #[tool(
        name = "forget",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<ForgetResult>(),
        description = "Permanently delete a memory by its `id` (as returned by `remember` or `recall`), removing the fact and its graph links. The deletion is durable and cannot be undone — use it to retract or correct stored knowledge. For automatic time-based expiry instead, set a TTL when calling `remember`. Returns the requested id plus `found`: `true` if a memory actually existed and was deleted, `false` if nothing was stored under that id (a stale id or a typo) — a no-op, not an error, but distinguishable from a real deletion.",
        input_schema = id_wire_input_schema::<ForgetParams>(&["id"])
    )]
    async fn forget(
        &self,
        Parameters(params): Parameters<ForgetParams>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let id = params.id;
        let found = tokio::task::spawn_blocking(move || service.forget(id))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(ForgetResult {
            id,
            id_str: id.to_string(),
            found,
        }))
    }

    #[tool(
        name = "entity",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<EntityProfileDto>(),
        description = "Look up everything the memory graph knows about a NAMED ENTITY (a person, a place, an organisation): the attributes it carries, the typed edges leaving it (`relations`) and the typed edges pointing AT it (`relations_in`). Both directions come back, because a question is only answerable from one side: with `camille --sister of--> theo` recorded, asking what Theo's OUTGOING edges say never finds Camille — she is in his `relations_in`. Use this for questions ABOUT a thing rather than about a sentence — \"how old is Theo\", \"who is Theo's father\", \"where does he live\" — where `recall` would only return sentences that happen to mention the name. Entities and their edges are built automatically by `remember_extracted`, which reads relationships (`X is the father of Y`) and properties (`Y is 15`) out of plain text; attributes land in ColumnStore metadata with their JSON type preserved, so a number stays a number. The name is matched case-insensitively, so `\"Theo Durand\"` and `\"theo durand\"` are the same entity — the id is content-addressed, so it is stable across sessions. Returns `found: false` when nothing has ever mentioned that name; `name` is echoed back in its canonical (trimmed, lowercased) form either way, so several lookups can be told apart. With the async autograph worker active, edges derived from a `remember` land asynchronously: an entity read immediately after that remember may not see them yet — the fact itself is always immediately readable. Ids exceed 2^53 — always relay them as strings (`id_str`)."
    )]
    async fn entity(
        &self,
        Parameters(params): Parameters<EntityParams>,
    ) -> Result<Json<EntityProfileDto>, ErrorData> {
        let service = Arc::clone(&self.service);
        let EntityParams { name } = params;
        let looked_up = name.clone();
        let profile = tokio::task::spawn_blocking(move || service.entity_profile(&looked_up))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(EntityProfileDto::from_lookup(&name, profile)))
    }

    #[tool(
        name = "why",
        // rmcp derives an output schema when none is given, and that
        // derived form keeps `$ref`s a `$defs`-blind client cannot resolve.
        output_schema = crate::schema::wire_safe_output_schema::<ExplanationDto>(),
        description = "Explain a decision: find the best-matching memory (optionally scoped by a metadata `filter`, e.g. the current project) and return the connected subgraph of related memories reachable through typed links — fusing vector, ColumnStore, and graph to surface context a plain similarity search misses."
    )]
    async fn why(
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
        let explanation =
            tokio::task::spawn_blocking(move || service.why(&decision, max_hops, filter.as_ref()))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(ExplanationDto::from(explanation)))
    }

    #[tool(
        name = "remember_extracted",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<RememberExtractedResult>(),
        description = "Store a passage of raw text by extracting its atomic facts and auto-building the fact↔topic graph, so `why` can later connect them with no manual links. Requires the server to be started with an extraction backend (set VELESDB_MEMORY_EXTRACTOR — compiled into the default build, no rebuild). Returns `ids` — the stored facts' ids, in extraction order — plus `ids_str`, their decimal-string twins: ids exceed 2^53, so always relay those on clients without u64-safe JSON number parsing. It also returns `skipped_over_cap`, and you should read it: that is how many facts the extractor DID produce out of your text and this tool did NOT store, because each was longer than the per-fact text limit. A non-zero `skipped_over_cap` means part of what you sent was extracted and then dropped — without that number, a short `ids` list is indistinguishable from a passage that simply held fewer facts."
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
                 VELESDB_MEMORY_EXTRACTOR=outline for the offline deterministic \
                 reader — it needs no model and no extra build feature — or \
                 =ollama with a local generative model",
                None,
            ));
        };
        // Extraction makes a blocking network call (up to the extractor's
        // timeout), so run it off the async worker pool to keep the stdio loop
        // responsive to other tool calls and cancellations.
        let service = Arc::clone(&self.service);
        let RememberExtractedParams { text, metadata } = params;
        let outcome = tokio::task::spawn_blocking(move || {
            service.remember_extracted(&text, &extractor, metadata.as_ref())
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        let ids_str = outcome.ids.iter().map(u64::to_string).collect();
        Ok(Json(RememberExtractedResult {
            ids: outcome.ids,
            ids_str,
            skipped_over_cap: outcome.skipped_over_cap,
        }))
    }

    #[tool(
        name = "memory_status",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<MemoryStatusResult>(),
        description = "Report this memory server's health and configuration: which embedder is running and whether recall is SEMANTIC (`embedder.semantic: false` means the offline `hash` default — recall matches surface form, not meaning, and configuring a semantic embedder is an env-var switch, no rebuild), what embedder the store was filled by per its on-disk provenance record, whether an extraction backend is configured (`remember_extracted` works iff `extraction.configured`), whether the background autograph worker is active and how many enrichments a full queue dropped, and the corpus size — `memory.facts` and `memory.edges`. Read `memory.edges` when `why` seems to add nothing over `recall`: `0` means no fact was ever linked (by `relate`, `remember`'s `links`, or extraction), so `why` HAS no graph to walk and degrades to plain search — that is a wiring gap, not a defect. Call this at session start, or whenever recall quality or `why`'s evidence trails surprise you, and tell the user when the server runs degraded. Takes no parameters."
    )]
    async fn memory_status(&self) -> Result<Json<MemoryStatusResult>, ErrorData> {
        let embedder = match &self.embedder_identity {
            Some((model, dimension)) => EmbedderStatus {
                model: Some(model.clone()),
                dimension: Some(*dimension),
                semantic: Some(model != "hash"),
            },
            None => EmbedderStatus {
                model: None,
                dimension: None,
                semantic: None,
            },
        };
        // The provenance record and the counts both touch the filesystem —
        // off the async workers, like every other tool body.
        let service = Arc::clone(&self.service);
        let store_dir = self.store_dir.clone();
        let (provenance, facts, edges) = tokio::task::spawn_blocking(move || {
            let recorded = store_dir
                .as_deref()
                .and_then(|dir| crate::embedding_provenance::read(dir).ok().flatten());
            (recorded, service.fact_count(), service.edge_count())
        })
        .await
        .map_err(join_error)?;
        let provenance = match provenance {
            Some(record) => ProvenanceStatus {
                recorded: true,
                model: Some(record.model),
                dimension: Some(record.dimension),
            },
            None => ProvenanceStatus {
                recorded: false,
                model: None,
                dimension: None,
            },
        };
        Ok(Json(MemoryStatusResult {
            embedder,
            provenance,
            extraction: ExtractionStatus {
                // Exactly the `remember_extracted` gate — the autograph
                // extractor is attached separately and reports through the
                // two autograph fields, so an autograph-only configuration
                // never claims a tool that would refuse.
                configured: self.extractor.is_some(),
                autograph_active: self.service.autograph_queue_open(),
                autograph_dropped: self.service.autograph_dropped(),
            },
            memory: MemoryCounts { facts, edges },
        }))
    }

    #[tool(
        name = "list_memories",
        // Sans declaration explicite, rmcp derive un schema de sortie qui
        // conserve des $ref qu'un client aveugle aux $defs ne resout pas —
        // or les SDK MCP valident structuredContent contre ce schema.
        output_schema = crate::schema::wire_safe_output_schema::<ListMemoriesResult>(),
        input_schema = id_wire_input_schema::<ListMemoriesParams>(&["cursor"]),
        description = "AUDIT the store: walk every stored fact, page by page — the question `recall` structurally cannot answer, because recall ranks by resemblance to a query and what resembles nothing you thought to ask stays invisible. Use it when the user asks what the memory contains ('what do you know about me / this project?'), to review or clean up before sharing a store, or to back up its contents. Returns `memories` (ids ascending — two audits of the same store see the same order; each entry carries `id`, `id_str`, `content`, `metadata`) and `next_cursor`: pass it back as `cursor` for the next page, `null` means the walk is complete. `filter` keeps only facts whose metadata equals every given key (e.g. {\"project\": \"acme\"}); a filtered page may come back sparse — KEEP following `next_cursor`, the walk stays exhaustive. Metadata follows recall's visibility rule (business keys plus the auto-stamped `_veles_date`; internal graph scaffolding excluded) unless `include_internal` is set, which lists everything verbatim. Ids exceed 2^53 — always relay them as strings (`id_str`, and `next_cursor` is already a string)."
    )]
    async fn list_memories(
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
            service.list(
                cursor,
                limit.unwrap_or(DEFAULT_LIST_LIMIT),
                filter.as_ref(),
                include_internal,
            )
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

/// `#[tool_handler]` generates `list_tools` from the router — `call_tool` is
/// written by hand below (see its doc comment) and the macro skips what
/// already exists. `get_info` is overridden so the server identifies itself
/// as `velesdb-memory` (the macro default falls back to rmcp's own
/// identity). Per-tool guidance lives in each `#[tool(description = …)]`.
/// The server's one-shot vitrine to a connecting agent (V2a-1 quick win):
/// must cover every tool family, not just memory — a `#[cfg(feature =
/// "context")]` variant since the context-compiler tools only exist in that
/// build.
#[cfg(feature = "context")]
const SERVER_INSTRUCTIONS: &str = "Local-first memory and context engineering for AI agents, three tool families: (1) durable memory — remember, remember_extracted, recall, recall_fused, recall_where, relate, unrelate, forget, feedback, entity, and why — explainable (why returns the evidence trail) and self-improving (feedback re-ranks future recall); remember_extracted reads the entities, typed edges and attributes a passage STATES and wires them into the graph, and entity(name) answers a question ABOUT a named thing rather than about the sentences mentioning it; memory_status reports the server's health — which embedder runs and whether recall is semantic, extraction wiring, and graph size; (2) the deterministic context compiler — compile_context, compile_transcript, explain_compilation, retrieve_context_source, context_savings, and suggest_budget — token-budgets and audits prompt context with no LLM call, ever; (3) cross-session working-context resumption — save_working_context, load_working_context, and list_working_contexts. compile_context/explain_compilation fragments accept a `path` instead of inline `content` to ingest a file by reference — disabled unless the server is started with VELESDB_MEMORY_INGEST_ROOTS set to an allowlist of directories (compile_transcript's own `path` field uses the same allowlist). compile_transcript is a one-call shortcut over compile_context for a raw agent-session transcript: it segments plain or JSONL text into turns before compiling, so an agent no longer needs to segment a transcript by hand. Nothing ever leaves the machine.";

#[cfg(not(feature = "context"))]
const SERVER_INSTRUCTIONS: &str = "Local-first memory for AI agents: remember facts, recall them \
     semantically, relate them, forget them, ask why a decision was made (connected subgraph), \
     and read memory_status for the server's health — embedder semantics, extraction wiring, \
     graph size.";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut instructions = SERVER_INSTRUCTIONS.to_owned();
        // The one channel a client is REQUIRED to read. The stderr warning
        // for the same fact is swallowed by every mainstream MCP harness, so
        // a degraded server that only warned there was indistinguishable
        // from a healthy one — the audit finding memory_status closes, and
        // this note is its push half (the tool is the pull half).
        if let Some(("hash", _)) = self
            .embedder_identity
            .as_ref()
            .map(|(model, dim)| (model.as_str(), dim))
        {
            instructions.push_str(
                " NOTE: this server is running the offline 'hash' embedder — recall matches \
                 surface form, NOT meaning. If recall quality matters to the user, say so: a \
                 semantic embedder is an env-var switch away (call memory_status for details).",
            );
        }
        info.instructions = Some(instructions);
        info
    }

    /// One trace event per tool call (#1780): tool name, session id, verdict,
    /// duration — never an argument or fact content
    /// (`tests/daemon_logging.rs` holds a canary against that). Written by
    /// hand so the event wraps the dispatch — `#[tool_handler]` sees the
    /// method already exists and only generates `list_tools`; the dispatch
    /// below is exactly what the macro would have generated.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, ErrorData> {
        let tool = request.name.clone();
        let session = http_session_id(&context.extensions);
        let started = std::time::Instant::now();
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let outcome = self.tool_router.call(tcc).await;
        log_tool_call(&tool, session.as_deref(), &outcome, started);
        outcome
    }
}

/// The tool-level trace event (#1780), split from `call_tool` so the verdict
/// taxonomy is readable in one place. `Err` is a protocol-level failure, but
/// a *refused* tool call comes back as `Ok` with `is_error` set INSIDE a
/// valid result — reading only the outer `Result` would log every refusal as
/// a success, the exact misreading this event exists to prevent.
fn log_tool_call(
    tool: &str,
    session: Option<&str>,
    outcome: &Result<rmcp::model::CallToolResponse, ErrorData>,
    started: std::time::Instant,
) {
    use rmcp::model::CallToolResponse;
    let verdict = match outcome {
        Err(_) => "error",
        Ok(CallToolResponse::Complete(result)) if result.is_error == Some(true) => "tool_error",
        Ok(CallToolResponse::Complete(_)) => "ok",
        // rmcp 3's InputRequired/Task responses (SEP-2663) carry no verdict:
        // the call has not completed. No tool of this server produces them —
        // if one ever appears here, "pending" keeps the event truthful
        // instead of misreporting an unfinished call as a success.
        Ok(_) => "pending",
    };
    // `%` (Display) rather than the default Debug capture: Debug renders
    // strings quoted (`tool="recall"`), and these lines exist to be grepped
    // (`grep tool=recall`) by an operator mid-incident. The target is pinned
    // explicitly for the same operators: a module refactor must not silently
    // rename the lines their tooling matches on.
    tracing::info!(
        target: "velesdb_memory::mcp",
        tool = %tool,
        session = %session.unwrap_or(crate::logging::NO_SESSION),
        verdict = %verdict,
        elapsed_ms = crate::logging::elapsed_millis(started),
        "mcp tool call"
    );
}

/// The `mcp-session-id` this call arrived under, read from the HTTP request
/// parts rmcp injects into the request's extensions (its streamable-HTTP
/// tower service does this for every incoming `POST`). HTTP transport only —
/// there is no session id on stdio, and the event then reports `-`.
#[cfg(feature = "http")]
fn http_session_id(extensions: &rmcp::model::Extensions) -> Option<String> {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| crate::http::session_from_headers(&parts.headers))
}

/// Without the HTTP transport nothing ever injects request parts, so there
/// is no session id to read — same cfg-pair shape as the binary's
/// transport-dependent helpers.
#[cfg(not(feature = "http"))]
fn http_session_id(_extensions: &rmcp::model::Extensions) -> Option<String> {
    None
}

/// La post-condition du point de passage, verifiee SUR PLACE.
///
/// Un test lointain constate ; ici on refuse. Les schemas annonces sont
/// statiques — ils ne dependent ni du store, ni de l'horloge, ni d'une
/// entree — donc une violation est deterministe : elle ne peut pas se
/// produire chez un utilisateur sans se produire aussi au premier
/// `McpServer::new` de la suite de tests. Echouer a la construction dit ou
/// est le probleme ; laisser passer un slot intypable le fait ressortir
/// plusieurs jours plus tard, dans un aller-retour de deserialisation chez
/// un agent.
///
/// # Panics
/// Si une route publie un slot d'entree qui n'annonce ni `type`, ni `enum`,
/// ni `const`.
fn assert_every_input_slot_is_typed(router: &ToolRouter<McpServer>) {
    let mut offenders: Vec<String> = Vec::new();
    for route in router.map.values() {
        for slot in crate::schema::untyped_input_slots(&route.attr.input_schema) {
            offenders.push(format!("  {}: {slot}", route.attr.name));
        }
    }
    assert!(
        offenders.is_empty(),
        "{} slot(s) d'entree n'annoncent aucun type — un harnais client les rend `{{}}`, le \
         client envoie ce qu'il devine, et le serveur le refuse :\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// Map a `spawn_blocking` join failure (a panicked or cancelled tool task) to an
/// MCP error. Every tool body runs on the blocking pool, so they all funnel
/// through this on the (rare) task-failure path.
///
/// Takes the error by value so it can be used as `.map_err(join_error)`.
#[allow(clippy::needless_pass_by_value)]
fn join_error(join: tokio::task::JoinError) -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        format!("memory task failed: {join}"),
        None,
    )
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
#[path = "mcp/server_tests.rs"]
mod tests;
