//! Node.js (napi-rs) binding for the `velesdb-memory` `MemoryService` — the
//! agent-memory wedge: `remember` / `recall` / `recallWhere` / `relate` /
//! `unrelate` / `forget` / `why` / `entity` / `feedback` / `rememberExtracted` / `compileContext` /
//! `compileTranscript` / `contextSavings` / `explainCompilation` /
//! `retrieveContextSource` / `saveWorkingContext` / `loadWorkingContext` /
//! `listWorkingContexts` / `suggestBudget`.
//!
//! It wraps the exact same hardened Rust the MCP server and the `PyO3` binding use
//! (no logic is reimplemented), mirroring `crates/velesdb-python/src/agent_memory_service.rs`
//! 1:1 — diverging only where the language forces it: `u64` ids cross the boundary
//! as decimal strings (JS 2^53), and `MemoryError` maps to stable string codes
//! since JS has no exception classes.
//!
//! ## License boundary
//! Depends on `velesdb-memory` (memory semantics only), never `velesdb-core`. The
//! addon is an in-process library, not a network service, so it stays inside the
//! `VelesDB` Core License 1.0 "no hosted/managed service" restriction.

#![deny(unsafe_code)]
// napi's panic→JS-error conversion relies on `panic = "unwind"` (the
// `release-node` profile); still forbid panicking constructs defensively so a
// dependency panic is the only way to abort the Node host.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// The error model is documented once at module level (stable string codes
// INVALID_INPUT / NOT_FOUND / INTERNAL), not re-stated per method.
#![allow(clippy::missing_errors_doc)]
// napi marshals every JS call argument into an owned Rust value at the boundary;
// the owned signatures ARE the public JS contract, so by-value args are correct.
#![allow(clippy::needless_pass_by_value)]
// Methods return an `AsyncTask` consumed by the napi-generated JS glue, never by
// Rust callers — a `#[must_use]` on each would be noise with no JS effect.
#![allow(clippy::must_use_candidate)]

mod convert;
mod dto;
mod error;
mod guards;
mod tasks;

use std::sync::Arc;

use napi::bindgen_prelude::AsyncTask;
use napi_derive::napi;
use serde_json::Value;
use velesdb_memory::context::{
    suggest_token_budget, CompilePolicy, CompileRequest, ContextCompiler, LoadedWorkingContext,
    WorkingContext,
};
use velesdb_memory::{
    DynEmbedder, HashEmbedder, MemoryService, OllamaEmbedder, OllamaExtractor, OutlineExtractor,
    DEFAULT_DIMENSION, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL,
};

use crate::dto::{
    ColumnFilterJs, CompiledContextJs, DatedRecallJs, EntityProfileJs, ExplanationJs,
    FusionOptionsJs, LinkJs, RecollectionJs, RememberedExtractionJs, UnrelateJs,
};
use crate::error::{invalid_input, to_napi_err, CODE_INTERNAL};
use crate::tasks::{Job, JsonOut};

/// Build the requested embedder. `"hash"` is deterministic and offline;
/// `"ollama"` calls a local embedding model (real semantic recall).
fn build_embedder(
    kind: &str,
    url: Option<String>,
    model: Option<String>,
) -> napi::Result<DynEmbedder> {
    match kind {
        "hash" => Ok(Box::new(HashEmbedder::new(DEFAULT_DIMENSION))),
        "ollama" => {
            let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
            let model = model.unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
            let embedder = OllamaEmbedder::new(url, model)
                .map_err(|e| napi::Error::from_reason(format!("[{CODE_INTERNAL}] {e}")))?;
            Ok(Box::new(embedder))
        }
        other => Err(invalid_input(format!(
            "unknown embedder '{other}' (expected 'hash' or 'ollama')"
        ))),
    }
}

/// Local-first agent memory with the `why()` graph wedge.
///
/// All methods are async (return a Promise) and run off the event-loop thread.
///
/// Exposed to JS as `MemoryService` (matching the `PyO3` binding and the core
/// type); the Rust struct keeps a distinct name only to avoid colliding with the
/// imported [`velesdb_memory::MemoryService`] it wraps.
#[napi(js_name = "MemoryService")]
pub struct MemoryStore {
    inner: Arc<MemoryService<DynEmbedder>>,
}

#[napi]
impl MemoryStore {
    /// Open (or create) a memory store at `path`.
    ///
    /// `embedder` is `"hash"` (default, offline) or `"ollama"` (real semantic
    /// recall); `ollamaUrl`/`ollamaModel` apply when `embedder="ollama"`.
    ///
    /// This factory is synchronous: with `embedder="ollama"` it performs a
    /// one-time blocking probe of the embedding endpoint (as the `PyO3` binding
    /// does). The default `"hash"` embedder does no I/O. Per-operation methods
    /// are all async.
    #[napi(factory)]
    pub fn open(
        path: String,
        embedder: Option<String>,
        ollama_url: Option<String>,
        ollama_model: Option<String>,
    ) -> napi::Result<Self> {
        let kind = embedder.as_deref().unwrap_or("hash");
        let emb = build_embedder(kind, ollama_url, ollama_model)?;
        let svc = MemoryService::open(&path, emb).map_err(to_napi_err)?;
        Ok(Self {
            inner: Arc::new(svc),
        })
    }

    // Every method returns an `AsyncTask` (a Promise) and does ALL validation +
    // marshalling inside the task closure, so there is exactly one error channel:
    // a rejected Promise (never a synchronous throw). The cheap DoS/size checks
    // still run as the closure's first lines, before any embedding or search, so
    // an oversized input never triggers real work.

    /// Store a fact; resolves to its decimal-string id. `links` are
    /// `{target, relation}` edges to existing memories; `metadata` is an optional
    /// object for later filtering. `ttlSeconds` makes the fact expire after that
    /// many seconds (a durable TTL that survives restarts); omit it (or `0`) for
    /// a permanent memory.
    #[napi(ts_return_type = "Promise<string>")]
    pub fn remember(
        &self,
        fact: String,
        links: Option<Vec<LinkJs>>,
        metadata: Option<Value>,
        ttl_seconds: Option<u32>,
    ) -> AsyncTask<Job<String>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            guards::check_fact(&fact)?;
            let links = convert::to_links(links)?;
            let metadata = convert::to_metadata(metadata)?;
            svc.remember_with_ttl(&fact, &links, metadata.as_ref(), ttl_seconds.map(u64::from))
                .map(convert::id_to_string)
                .map_err(to_napi_err)
        }))
    }

    /// Recall up to `k` (default 10, capped) memories similar to `query`,
    /// optionally narrowed by an exact-match metadata `filter`.
    #[napi(ts_return_type = "Promise<Array<RecollectionJs>>")]
    pub fn recall(
        &self,
        query: String,
        k: Option<u32>,
        filter: Option<Value>,
    ) -> AsyncTask<Job<Vec<RecollectionJs>>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let k = guards::clamp_limit(k.unwrap_or(10));
            let filter = convert::to_metadata(filter)?;
            let hits = svc
                .recall(&query, k, filter.as_ref())
                .map_err(to_napi_err)?;
            Ok(hits.into_iter().map(RecollectionJs::from).collect())
        }))
    }

    /// Fused vector + `ColumnStore` recall: like [`recall`](Self::recall) but the
    /// `filters` support ranges/comparisons (`gt`, `le`, …), so temporal/numeric
    /// facets become queryable. Mirrors the `PyO3` `recall_where` surface.
    #[napi(
        js_name = "recallWhere",
        ts_return_type = "Promise<Array<RecollectionJs>>"
    )]
    pub fn recall_where(
        &self,
        query: String,
        filters: Vec<ColumnFilterJs>,
        k: Option<u32>,
    ) -> AsyncTask<Job<Vec<RecollectionJs>>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let k = guards::clamp_limit(k.unwrap_or(10));
            let filters = convert::to_filters(filters)?;
            let hits = svc.recall_where(&query, k, &filters).map_err(to_napi_err)?;
            Ok(hits.into_iter().map(RecollectionJs::from).collect())
        }))
    }

    /// Fused vector + graph recall: like [`recall`](Self::recall), but also
    /// walks the graph from the top vector hit and promotes any fact it
    /// reaches into the ranking — the tri-engine ranking measured on
    /// HotpotQA/TimeQA/LoCoMo, now reachable from Node. `opts` is optional;
    /// an omitted field falls back to the proven default (`hops: 2`,
    /// `graphBoost: 0.15`, oversampled pool).
    #[napi(
        js_name = "recallFused",
        ts_return_type = "Promise<Array<RecollectionJs>>"
    )]
    pub fn recall_fused(
        &self,
        query: String,
        k: Option<u32>,
        filter: Option<Value>,
        opts: Option<FusionOptionsJs>,
    ) -> AsyncTask<Job<Vec<RecollectionJs>>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let k = guards::clamp_limit(k.unwrap_or(10));
            let filter = convert::to_metadata(filter)?;
            let opts = convert::to_fusion_options(opts);
            let hits = svc
                .recall_fused(&query, k, filter.as_ref(), opts)
                .map_err(to_napi_err)?;
            Ok(hits.into_iter().map(RecollectionJs::from).collect())
        }))
    }

    /// Fused recall plus a dated timeline: like [`recall_fused`](Self::recall_fused),
    /// but reads each fact's date from the `dateField` metadata key (a `YYYYMMDD`
    /// integer) and resolves to `{memories, datedContext, now}` — the memories, a
    /// chronological date-prefixed timeline, and a "now" anchor for temporal
    /// reasoning. A separate method (not a flag on `recallFused`) so the published
    /// `recallFused` array return type stays unchanged.
    #[napi(
        js_name = "recallFusedDated",
        ts_return_type = "Promise<DatedRecallJs>"
    )]
    pub fn recall_fused_dated(
        &self,
        query: String,
        date_field: String,
        k: Option<u32>,
        filter: Option<Value>,
        opts: Option<FusionOptionsJs>,
    ) -> AsyncTask<Job<DatedRecallJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let k = guards::clamp_limit(k.unwrap_or(10));
            let filter = convert::to_metadata(filter)?;
            let opts = convert::to_fusion_options(opts);
            let (hits, ctx) = svc
                .recall_fused_dated(&query, k, filter.as_ref(), opts, &date_field)
                .map_err(to_napi_err)?;
            Ok(DatedRecallJs {
                memories: hits.into_iter().map(RecollectionJs::from).collect(),
                dated_context: ctx.timeline,
                now: ctx.now,
            })
        }))
    }

    /// Create a typed edge `from -> to`. Resolves to the edge's decimal-string id.
    #[napi(ts_return_type = "Promise<string>")]
    pub fn relate(&self, from: String, to: String, relation: String) -> AsyncTask<Job<String>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let from = convert::parse_id(&from)?;
            let to = convert::parse_id(&to)?;
            svc.relate(from, to, &relation)
                .map(convert::id_to_string)
                .map_err(to_napi_err)
        }))
    }

    /// Remove the typed edge(s) `from -relation-> to` — [`relate`](Self::relate)'s
    /// exact undo, so a mistaken edge no longer costs the facts at its endpoints.
    /// Only the edge goes: both memories, and any entity hub, are untouched.
    ///
    /// Resolves to `{found, removed}`. Idempotent: removing an absent edge
    /// answers `found: false` instead of rejecting, so a cleanup can be
    /// replayed safely. It rejects exactly what `relate` rejects (empty
    /// relation, `from` equal to `to`), and deliberately does NOT require the
    /// endpoints to still exist — the edge of a forgotten fact is already gone.
    #[napi(ts_return_type = "Promise<UnrelateJs>")]
    pub fn unrelate(
        &self,
        from: String,
        to: String,
        relation: String,
    ) -> AsyncTask<Job<UnrelateJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let from = convert::parse_id(&from)?;
            let to = convert::parse_id(&to)?;
            svc.unrelate(from, to, &relation)
                .map(UnrelateJs::from)
                .map_err(to_napi_err)
        }))
    }

    /// Record an outcome for a recalled fact: `success = true` reinforces it,
    /// `false` weakens it. Resolves to the fact's new learned confidence in
    /// `[0, 1]` — the signal `recall` re-ranks by and the context compiler's
    /// importance blend (`policy.importance`) folds into memory selection.
    #[napi(ts_return_type = "Promise<number>")]
    pub fn feedback(&self, id: String, success: bool) -> AsyncTask<Job<f64>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let id = convert::parse_id(&id)?;
            svc.feedback(id, success)
                .map(f64::from)
                .map_err(to_napi_err)
        }))
    }

    /// Delete a memory by id. Resolves to whether a memory actually existed
    /// under that id and was deleted — `false` means nothing was stored
    /// there (a stale id or a typo), not a second successful deletion.
    #[napi(ts_return_type = "Promise<boolean>")]
    pub fn forget(&self, id: String) -> AsyncTask<Job<bool>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let id = convert::parse_id(&id)?;
            svc.forget(id).map_err(to_napi_err)
        }))
    }

    /// Explain a decision: the best-matching memory plus its connected subgraph.
    /// Resolves to `{nodes, edges}`. `maxHops` (default 2) is capped at 10.
    #[napi(ts_return_type = "Promise<ExplanationJs>")]
    pub fn why(
        &self,
        decision: String,
        max_hops: Option<u32>,
        filter: Option<Value>,
    ) -> AsyncTask<Job<ExplanationJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let max_hops = guards::clamp_hops(max_hops.unwrap_or(2));
            let filter = convert::to_metadata(filter)?;
            svc.why(&decision, max_hops, filter.as_ref())
                .map(ExplanationJs::from)
                .map_err(to_napi_err)
        }))
    }

    /// Look up everything the memory graph knows about a NAMED ENTITY (a
    /// person, a place, an organisation): the attributes merged onto its hub
    /// and the typed edges touching it, in BOTH directions. Use it for a
    /// question ABOUT a thing ("how old is X", "who is X's father") rather
    /// than about the sentences mentioning it, which is all
    /// [`recall`](Self::recall) can return — entity hubs are deliberately
    /// invisible to recall, so without this the attributes
    /// `rememberExtracted` builds are unreachable.
    ///
    /// `name` is matched case-insensitively (the id is content-addressed, so
    /// it is stable across sessions). Resolves to `{found, id, name,
    /// attributes, relations, relationsIn}`; `found: false` means nothing has
    /// ever mentioned that name, and `name` still echoes the canonicalized
    /// query.
    ///
    /// `relations` are the typed edges LEAVING the entity, `relationsIn`
    /// those pointing AT it — each naming, in `targetId`/`target`, the far
    /// end it comes FROM. Without the second list a question is only
    /// answerable from one side: the graph holds
    /// `camille --sister of--> theo`, so reading Theo's outgoing edges never
    /// finds Camille.
    #[napi(ts_return_type = "Promise<EntityProfileJs>")]
    pub fn entity(&self, name: String) -> AsyncTask<Job<EntityProfileJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let profile = svc.entity_profile(&name).map_err(to_napi_err)?;
            Ok(EntityProfileJs::from_lookup(&name, profile))
        }))
    }

    /// Compile context fragments into a token-budgeted, provenance-audited
    /// prompt context — deterministic, no LLM call; pure conversion around
    /// [`velesdb_memory`]'s context compiler (zero logic here). The request
    /// and result use the same JSON shape as the MCP `compile_context` tool
    /// (`{query, fragments, token_budget, memory_scope?, policy?, …}`), with
    /// one binding-wide difference: every id field (`fragment_id`,
    /// `content_hash`, `memory_id`, `fragment_ids`, and `fragments[].id` on
    /// input) crosses as a decimal string, like every other method here.
    #[napi(
        js_name = "compileContext",
        ts_return_type = "Promise<CompiledContextJs>"
    )]
    pub fn compile_context(&self, request: Value) -> AsyncTask<Job<CompiledContextJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let mut request = request;
            convert::parse_fragment_id_strings(&mut request)?;
            let request: CompileRequest = serde_json::from_value(request)
                .map_err(|err| invalid_input(format!("invalid compile request: {err}")))?;
            let compiled = svc
                .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
                .map_err(to_napi_err)?;
            convert::to_compiled_js(&compiled)
        }))
    }

    /// Aggregate the token (and cost) savings of past [`Self::compile_context`]
    /// calls, optionally narrowed to one `project`. Same computation and JSON
    /// shape as the MCP `context_savings` tool (figures are local estimates
    /// recorded per compilation — metadata only, never fragment content;
    /// `truncated` reports when the aggregation hit the recall cap). Pure
    /// delegation to [`velesdb_memory`]'s bridge — zero logic in the binding.
    #[napi(
        js_name = "contextSavings",
        ts_return_type = "Promise<{ events: number; tokens_in: number; tokens_out: number; tokens_saved: number; cost_saved_micros_by_currency: Record<string, number>; truncated: boolean }>"
    )]
    pub fn context_savings(&self, project: Option<String>) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let savings = svc
                .context_savings(project.as_deref())
                .map_err(to_napi_err)?;
            let value = serde_json::to_value(&savings)
                .map_err(|err| invalid_input(format!("context savings serialization: {err}")))?;
            Ok(JsonOut(value))
        }))
    }

    /// Explain why one fragment of a [`Self::compile_context`] request was
    /// preserved, abstracted, externalized, dropped, or cached. Compilation
    /// is deterministic, so `request` is re-compiled (with event/source
    /// recording forced off) and the matching decision is returned — no
    /// server-side state needed. Same JSON request/response shape as the MCP
    /// `explain_compilation` tool: `request` accepts the same shape as
    /// [`Self::compile_context`]'s (fragment ids as decimal strings on
    /// input); `fragmentId` and `fragmentIndex` mirror the MCP tool's own
    /// parameters, id fields on the returned decision cross as decimal
    /// strings. `fragmentIndex` (0-based position in `request.fragments`),
    /// when given, TAKES PRIORITY over `fragmentId` for locating the
    /// decision — see the MCP tool's own docs for the full disambiguation
    /// rationale (byte-identical fragments share a content-addressed id).
    /// Pure delegation to [`velesdb_memory`]'s bridge — zero logic in the
    /// binding.
    #[napi(
        js_name = "explainCompilation",
        ts_return_type = "Promise<{ fragment_id: string; content_hash: string; action: string; rule_id: string; relevance: number; risk: string; reason: string; memory_id?: string; handle?: string }>"
    )]
    pub fn explain_compilation(
        &self,
        request: Value,
        fragment_id: String,
        fragment_index: Option<u32>,
    ) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let mut request = request;
            convert::parse_fragment_id_strings(&mut request)?;
            let request: CompileRequest = serde_json::from_value(request)
                .map_err(|err| invalid_input(format!("invalid compile request: {err}")))?;
            let fragment_id = convert::parse_id(&fragment_id)?;
            let fragment_index = fragment_index.map(|i| i as usize);
            let decision = svc
                .explain_compilation(&request, fragment_id, fragment_index)
                .map_err(to_napi_err)?;
            let mut value = serde_json::to_value(&decision)
                .map_err(|err| invalid_input(format!("context decision serialization: {err}")))?;
            convert::stringify_id_fields(&mut value);
            Ok(JsonOut(value))
        }))
    }

    /// Fetch back the exact original content — and media, when the fragment
    /// carried one (US-009, PR3) — behind a `ctx://source/<hash>` handle
    /// from a [`Self::compile_context`] result: what was externalized or
    /// partially packed is recoverable, not lost. Same JSON shape as the MCP
    /// `retrieve_context_source` tool: `{handle, content, media?}`, `media`
    /// present only for a source whose fragment carried one. Pure
    /// delegation to [`velesdb_memory`]'s bridge — zero logic in the
    /// binding.
    #[napi(
        js_name = "retrieveContextSource",
        ts_return_type = "Promise<{ handle: string; content: string; media?: { mime: string; bytes_b64: string } }>"
    )]
    pub fn retrieve_context_source(&self, handle: String) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let source = svc.retrieve_context_source(&handle).map_err(to_napi_err)?;
            convert::to_retrieve_source_js(&handle, &source).map(JsonOut)
        }))
    }

    /// Persist the agent's distilled working state under `project` +
    /// `session` (idempotent upsert: saving again replaces the previous
    /// state), for inter-session resumption. Same JSON shape as the MCP
    /// `save_working_context` tool; resolves to the stored fact id as a
    /// decimal string, like every other id here. Pure delegation to
    /// [`velesdb_memory`]'s bridge — zero logic in the binding.
    #[napi(js_name = "saveWorkingContext", ts_return_type = "Promise<string>")]
    pub fn save_working_context(
        &self,
        project: String,
        session: String,
        working: Value,
    ) -> AsyncTask<Job<String>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let mut working = working;
            convert::parse_id_fields(&mut working)?;
            let working: WorkingContext = serde_json::from_value(working)
                .map_err(|err| invalid_input(format!("invalid working context: {err}")))?;
            svc.save_working_context(&project, &session, &working)
                .map(convert::id_to_string)
                .map_err(to_napi_err)
        }))
    }

    /// The resumption envelope for `project` + `session` — the
    /// start-of-session mirror of [`Self::save_working_context`], same shape
    /// as the MCP `load_working_context` tool: `{found, working,
    /// other_sessions}`.
    ///
    /// **BREAKING (0.12.0)**: this used to resolve the bare working context
    /// (or `null`), which collapsed two different answers into one — a
    /// project that never saved anything, and a typo in `session` that
    /// missed a session which does exist. `other_sessions` is what tells
    /// them apart, and it is filled in on a HIT too: a typo landing on
    /// another REAL session returns `found: true`, the case a caller can
    /// least detect on its own. Read `.working` for the previous return
    /// value. Pure delegation to [`velesdb_memory`]'s bridge — the envelope
    /// is composed by `resume_working_context`, zero logic in the binding.
    #[napi(
        js_name = "loadWorkingContext",
        ts_return_type = "Promise<{ found: boolean; working: object | null; other_sessions: Array<string> }>"
    )]
    pub fn load_working_context(
        &self,
        project: String,
        session: String,
    ) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            // Annotated, not inferred: `binding_parity_bdd` reads this type
            // name to prove the binding relays the SERVER's own envelope
            // rather than a shape it recomposed by hand — and the compiler
            // makes that proof real. A doc comment describing `{found,
            // working, other_sessions}` proves nothing; this does.
            let loaded: LoadedWorkingContext = svc
                .resume_working_context(&project, &session)
                .map_err(to_napi_err)?;
            let mut value = serde_json::to_value(loaded)
                .map_err(|err| invalid_input(format!("working context serialization: {err}")))?;
            // Applied at the ROOT of the envelope, not to `working` alone:
            // the walk descends by KEY NAME, so it still reaches
            // `working.decisions[].fragment_id` and
            // `working.exact_evidence[].fragment_id` one level deeper.
            convert::stringify_id_fields(&mut value);
            Ok(JsonOut(value))
        }))
    }

    /// Every session ever saved under `project`'s working-context index,
    /// most-recently-saved first — so an agent can discover what is
    /// resumable before guessing a session id at
    /// [`Self::load_working_context`], or recover from a typo. Same JSON
    /// shape as the MCP `list_working_contexts` tool: `{sessions:
    /// [{session, saved_at}]}`, empty (never an error) when the project
    /// never saved anything. Pure delegation to [`velesdb_memory`]'s bridge
    /// — zero logic in the binding.
    #[napi(
        js_name = "listWorkingContexts",
        ts_return_type = "Promise<{ sessions: Array<{ session: string; saved_at: number }> }>"
    )]
    pub fn list_working_contexts(&self, project: String) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let sessions = svc.list_working_contexts(&project).map_err(to_napi_err)?;
            let sessions_value = serde_json::to_value(&sessions).map_err(|err| {
                invalid_input(format!("working context sessions serialization: {err}"))
            })?;
            let mut map = serde_json::Map::new();
            map.insert("sessions".to_owned(), sessions_value);
            Ok(JsonOut(Value::Object(map)))
        }))
    }

    /// One-call shortcut over [`Self::compile_context`] for a raw
    /// agent-session transcript: deterministically segments it into turns
    /// (plain marker-based — `System:`/`User:`/`Human:`/`Assistant:`/`AI:`/
    /// `Tool:`/`### User`/`### Assistant` — or JSONL, one line per turn) and,
    /// within each turn, into code/log/body sub-segments (fenced code blocks
    /// stay atomic; runs of 8+ log-like lines collapse the same way
    /// `abstract.log_dedup` would), then compiles the result exactly like
    /// [`Self::compile_context`]. Same JSON request shape as the MCP
    /// `compile_transcript` tool's `transcript` (inline) input — this
    /// binding does not resolve the tool's `path` field (no
    /// `VELESDB_MEMORY_INGEST_ROOTS`-style configuration surface here; read
    /// the file yourself and pass its content as `transcript`). Resolves to
    /// `{context, segmentation}`: `context` is the same wire shape as
    /// [`Self::compile_context`]'s own output (id fields as decimal
    /// strings); `segmentation` is the detected format plus one audit entry
    /// (turn, role, kind, byte range, `fragment_id` — a decimal string) per
    /// segment, so a caller can see exactly how the transcript was cut
    /// before trusting the compiled result.
    #[napi(
        js_name = "compileTranscript",
        ts_return_type = "Promise<{ context: { content: string; sections: object; decisions: object; sources: object; retrieval_handles: object; insights: object; risk: string }; segmentation: { format_detected: string; segments: Array<{ index: number; turn: number; role?: string; kind: string; byte_start: number; byte_end: number; fragment_id: string }>; merged_segments: number } }>"
    )]
    pub fn compile_transcript(&self, request: Value) -> AsyncTask<Job<JsonOut>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            let input: convert::CompileTranscriptInput =
                serde_json::from_value(request).map_err(|err| {
                    invalid_input(format!("invalid compile_transcript request: {err}"))
                })?;
            let (request, segmentation) = convert::build_transcript_compile_request(input)?;
            let compiled = svc
                .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
                .map_err(to_napi_err)?;
            let mut context_value = serde_json::to_value(&compiled)
                .map_err(|err| invalid_input(format!("compiled context serialization: {err}")))?;
            convert::stringify_id_fields(&mut context_value);
            let mut map = serde_json::Map::new();
            map.insert("context".to_owned(), context_value);
            map.insert("segmentation".to_owned(), segmentation);
            Ok(JsonOut(Value::Object(map)))
        }))
    }

    /// Suggest a starting `tokenBudget` for [`Self::compile_context`], for a
    /// named target model — looked up in a static, committed model-name to
    /// context-window table (dated "as of", NEVER a network call).
    /// `reserveTokens` (default 0) reserves room for the response, mirroring
    /// `compileContext`'s own `policy.response_reserve_tokens`.
    ///
    /// `window`/`suggested_budget` come back `null` for a model that is not
    /// in the table — an honest "unknown", never a guess; the table is
    /// extended in a new release rather than worked around here.
    ///
    /// `reserveTokens` is a `u32` for the same reason `ttlSeconds` is: napi
    /// marshals a `u64` as a `BigInt`, and 4 billion reserved tokens is
    /// already three orders of magnitude past the largest window in the
    /// table.
    #[napi(
        js_name = "suggestBudget",
        ts_return_type = "Promise<{ window: number | null; suggested_budget: number | null; source: string }>"
    )]
    pub fn suggest_budget(
        &self,
        target_model: String,
        reserve_tokens: Option<u32>,
    ) -> AsyncTask<Job<JsonOut>> {
        AsyncTask::new(Job::new(move || {
            let budget =
                suggest_token_budget(&target_model, u64::from(reserve_tokens.unwrap_or(0)));
            let value = serde_json::to_value(&budget)
                .map_err(|err| invalid_input(format!("suggested budget serialization: {err}")))?;
            Ok(JsonOut(value))
        }))
    }

    /// Extract atomic facts from raw `text` and store them, auto-building the
    /// entity graph they state. Resolves to `{ids, skippedOverCap}`.
    ///
    /// `extractor` names the backend, defaulting to `"ollama"`:
    ///
    /// - `"ollama"` calls the local generative `model` (required for this
    ///   backend) at `url`, and reads structure out of prose.
    /// - `"outline"` is deterministic and network-free: it reads the
    ///   structure the passage STATES, one directive per line (`edge:`,
    ///   `attr:`, `fact:`), and ignores `model`/`url`. Same relationship as
    ///   the `"hash"` embedder has to `"ollama"` on
    ///   [`open`](Self::open) — an offline, reproducible choice, so the whole
    ///   contract of this method is reachable without a model running.
    ///
    /// This used to resolve to a bare `Array<string>` of ids, and that array
    /// could not say why it was short: nothing distinguished a passage that
    /// held three facts from one that held twelve of which nine were dropped
    /// for exceeding the embeddable cap. The envelope is the breaking change
    /// that ends that silence (issue #1692).
    #[napi(
        js_name = "rememberExtracted",
        ts_return_type = "Promise<RememberedExtractionJs>"
    )]
    pub fn remember_extracted(
        &self,
        text: String,
        model: Option<String>,
        url: Option<String>,
        metadata: Option<Value>,
        extractor: Option<String>,
    ) -> AsyncTask<Job<RememberedExtractionJs>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            guards::check_fact(&text)?;
            let metadata = convert::to_metadata(metadata)?;
            let outcome = match extractor.as_deref().unwrap_or("ollama") {
                "outline" => svc.remember_extracted(&text, &OutlineExtractor, metadata.as_ref()),
                "ollama" => {
                    let model = model.ok_or_else(|| {
                        invalid_input(
                            "the 'ollama' extractor needs a `model` (or pass \
                                       extractor: 'outline' for the offline backend)",
                        )
                    })?;
                    let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
                    svc.remember_extracted(
                        &text,
                        &OllamaExtractor::new(url, model),
                        metadata.as_ref(),
                    )
                }
                other => {
                    return Err(invalid_input(format!(
                        "unknown extractor '{other}' (expected 'ollama' or 'outline')"
                    )))
                }
            };
            Ok(RememberedExtractionJs::from(outcome.map_err(to_napi_err)?))
        }))
    }
}
