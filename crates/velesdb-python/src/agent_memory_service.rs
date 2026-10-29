//! Python binding for the high-level `velesdb-memory` `MemoryService` — the
//! agent-memory *wedge*: `remember` / `recall` / `relate` / `forget` / `why`
//! plus `remember_extracted` (auto text → fact↔topic graph).
//!
//! Unlike the lower-level `AgentMemory` binding (bring-your-own-vector), this
//! service **embeds text itself**, so the constructor selects an embedder. It
//! wraps the exact same hardened Rust used by the MCP server — no logic is
//! reimplemented here.
//!
//! This file holds the `MemoryService` class itself. What it converts through
//! lives beside it: `errors` (a `MemoryError` as a Python exception),
//! `filters` (the `recall_where` tuples), `backends` (choosing the embedder
//! and the extractor) and `marshal` (memory types as dicts and lists).

mod backends;
mod errors;
mod filters;
mod marshal;

use backends::{build_embedder, extract_with, warn_hash_embedder_not_semantic};
use errors::{setting_error, to_py_err};
use filters::to_column_filters;
use marshal::{
    dated_recall_to_dict, entity_profile_to_value, explanation_to_dict, fusion_options_from_dict,
    recollections_to_list, to_metadata,
};

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use std::collections::HashMap;

use velesdb_memory::context::{
    build_transcript_compile_request, suggest_token_budget, CompilePolicy, CompileRequest,
    CompiledContext, ContextCompiler, ContextDecision, ContextSavings, ContextSource,
    LoadedWorkingContext, TranscriptCompileInput, WorkingContext, WorkingContextSession,
};
use velesdb_memory::{format_dated_context, limits, DynEmbedder, Link, MemoryService};

use crate::utils::{json_to_python, python_to_json};

/// Serialize a context-compiler output value to its Python dict — the exact
/// same JSON shape the MCP tools and the Node binding serialize, going
/// through `serde_json` + the crate's manual [`json_to_python`] converter
/// rather than hand-rolled field-by-field construction. `to_value` failure is
/// an internal bug (every field here is JSON-representable), never a caller
/// input problem, hence `RuntimeError`.
macro_rules! serde_to_python {
    ($py:expr, $value:expr, $what:literal) => {{
        let json = serde_json::to_value($value).map_err(|err| {
            PyRuntimeError::new_err(format!(concat!("failed to serialize ", $what, ": {}"), err))
        })?;
        json_to_python($py, &json)
    }};
}

/// Deserialize a Python dict (the same JSON shape as the corresponding MCP
/// tool input) into a context-compiler request type.
macro_rules! python_to_serde {
    ($py:expr, $obj:expr, $what:literal) => {{
        let json = python_to_json($py, $obj)?;
        serde_json::from_value(json).map_err(|err| {
            PyValueError::new_err(format!(concat!("invalid ", $what, ": {}"), err))
        })?
    }};
}

/// Local-first agent memory with the `why()` graph wedge.
///
/// Example:
///     >>> from velesdb import MemoryService
///     >>> mem = MemoryService("./agent_mem")
///     >>> pr = mem.remember("PR #42 swaps the mutex for parking_lot")
///     >>> d  = mem.remember("we chose parking_lot to avoid lock poisoning",
///     ...                   links=[(pr, "decided_in")])
///     >>> mem.why("why did we choose parking_lot")["nodes"]
#[pyclass(name = "MemoryService")]
pub struct PyMemoryService {
    svc: MemoryService<DynEmbedder>,
    /// The embedder identity resolved at construction — what
    /// `memory_status` reports as RUNNING. The service itself only ever
    /// sees `&[f32]`, so the constructor is the one place that knows.
    embedder_model: String,
    embedder_dimension: usize,
    embedder_semantic: bool,
    /// Where the store lives, for the provenance block of `memory_status`
    /// (#1751's on-disk record).
    store_dir: std::path::PathBuf,
}

#[pymethods]
impl PyMemoryService {
    /// Open (or create) a memory store at `path`.
    ///
    /// Args:
    ///     path: store directory (created if missing; memory never leaves it).
    ///     embedder: "hash" (default, offline), "ollama" or "openai" (real
    ///         semantic recall). Omit to fall back to `VELESDB_MEMORY_EMBEDDER`
    ///         — an explicit value here always wins over the environment.
    ///     ollama_url / ollama_model: apply to embedder="ollama" only; an
    ///         explicit value wins over `VELESDB_MEMORY_EMBEDDER_URL`/`_MODEL`.
    ///         embedder="openai" reads its URL, model and credential from the
    ///         environment exclusively — see `crates/velesdb-memory/README.md`.
    ///     Opening with `hash` emits one degraded-recall notice on stderr;
    ///         `VELESDB_MEMORY_QUIET=1` suppresses it for deliberate offline use.
    ///     search_quality: recall quality ("fast", "balanced", "accurate",
    ///         "perfect", "autotune", "custom:<ef>" or
    ///         "adaptive:<min_ef>:<max_ef>"). An explicit value wins
    ///         over `VELESDB_MEMORY_SEARCH_QUALITY`; omitting both keeps the
    ///         collection's own quality. An unknown value raises `ValueError`.
    #[new]
    #[pyo3(signature = (path, embedder = None, ollama_url = None, ollama_model = None, search_quality = None))]
    fn new(
        path: String,
        embedder: Option<&str>,
        ollama_url: Option<String>,
        ollama_model: Option<String>,
        search_quality: Option<&str>,
    ) -> PyResult<Self> {
        let quality =
            velesdb_memory::resolve_search_quality(search_quality).map_err(setting_error)?;
        let (emb, embedder_model, embedder_semantic) =
            build_embedder(embedder, ollama_url, ollama_model)?;
        let embedder_dimension = emb.dimension();
        let svc = MemoryService::open(&path, emb).map_err(to_py_err)?;
        let svc = match quality {
            Some(quality) => svc.with_search_quality(quality),
            None => svc,
        };
        warn_hash_embedder_not_semantic(embedder_semantic);
        Ok(Self {
            svc,
            embedder_model,
            embedder_dimension,
            embedder_semantic,
            store_dir: std::path::PathBuf::from(path),
        })
    }

    /// Store a fact; returns its stable id. `links` is a list of `(target_id,
    /// relation)` tuples; `metadata` is an optional dict for later filtering.
    /// `ttl_seconds` makes the fact expire after that many seconds (a durable TTL
    /// that survives restarts); omit it for a permanent memory, including when
    /// re-storing a fact that already had an expiry — only what this call
    /// supplies is applied. An explicit `0` is REFUSED, because a caller
    /// writing `0` means "expire now", not "never".
    #[pyo3(signature = (fact, links = None, metadata = None, ttl_seconds = None))]
    fn remember(
        &self,
        py: Python<'_>,
        fact: &str,
        links: Option<Vec<(u64, String)>>,
        metadata: Option<HashMap<String, Py<PyAny>>>,
        ttl_seconds: Option<u64>,
    ) -> PyResult<u64> {
        if fact.len() > limits::MAX_FACT_BYTES {
            return Err(PyValueError::new_err(format!(
                "fact exceeds {} bytes ({} given)",
                limits::MAX_FACT_BYTES,
                fact.len()
            )));
        }
        let links: Vec<Link> = links
            .unwrap_or_default()
            .into_iter()
            .map(|(target, relation)| Link { target, relation })
            .collect();
        let metadata = to_metadata(py, metadata)?;
        py.detach(|| {
            self.svc
                .remember_with_ttl(fact, &links, metadata.as_ref(), ttl_seconds)
                .map_err(to_py_err)
        })
    }

    /// Recall up to `k` memories similar to `query`, optionally narrowed by an
    /// exact-match metadata `filter`. Returns a list of `{id, score, content, metadata}`
    /// (`metadata` is `None` when the fact carries none, matching the upstream
    /// `Recollection` contract).
    #[pyo3(signature = (query, k = 10, filter = None))]
    fn recall(
        &self,
        py: Python<'_>,
        query: &str,
        k: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let k = limits::clamp_recall_limit(k);
        let filter = to_metadata(py, filter)?;
        let hits = py.detach(|| {
            self.svc
                .recall(query, k, filter.as_ref())
                .map_err(to_py_err)
        })?;
        Ok(recollections_to_list(py, hits)?.into())
    }

    /// Fused vector + `ColumnStore` recall: like [`recall`](Self::recall) but the
    /// `filters` support ranges/comparisons, so numeric/temporal facets become
    /// queryable. `filters` is a list of `(field, op, value)` tuples where `op`
    /// is one of `eq`/`ne`/`lt`/`le`/`gt`/`ge`. Returns a list of
    /// `{id, score, content, metadata}` items
    /// (`metadata` is the fact's stored dict, or `None` if it carried none).
    ///
    /// Returns your own stored facts ONLY: entity hubs and the context compiler's
    /// artefacts (stored sources, compilation events, working contexts and
    /// their index) are internal scaffolding and never come back, whatever the
    /// predicate — including a `ne` one, which matches facts lacking the field
    /// entirely.
    #[pyo3(signature = (query, filters, k = 10))]
    fn recall_where(
        &self,
        py: Python<'_>,
        query: &str,
        filters: Vec<(String, String, Py<PyAny>)>,
        k: usize,
    ) -> PyResult<Py<PyAny>> {
        let k = limits::clamp_recall_limit(k);
        let filters = to_column_filters(py, filters)?;
        let hits = py.detach(|| self.svc.recall_where(query, k, &filters).map_err(to_py_err))?;
        Ok(recollections_to_list(py, hits)?.into())
    }

    /// Fused vector + graph recall: like [`recall`](Self::recall), but also
    /// walks the graph from the top vector hit and folds any connected fact
    /// into the ranking — the tri-engine ranking measured on multi-hop and
    /// temporal benchmarks. Best when an answer needs a fact the query doesn't
    /// mention directly but a stored `relate`/`remember_extracted` link
    /// connects. Advanced fusion tuning goes in `options`
    /// (`{"hops": int, "graph_boost": float, "pool": int}`, all optional — same
    /// shape as the Node/WASM binding); omit it for the proven defaults.
    ///
    /// Without `date_field`, returns a list of `{id, score, content, metadata}`
    /// (`metadata` is the fact's stored dict, or `None` if it carried none).
    /// With `date_field` set to the metadata key holding each fact's `YYYYMMDD`
    /// date, returns a dict `{"memories": [...], "dated_context": str, "now":
    /// str | None}` — the memories plus a chronological, date-prefixed timeline
    /// and a "now" anchor for temporal reasoning.
    #[pyo3(signature = (query, k = 10, filter = None, *, date_field = None, options = None))]
    fn recall_fused(
        &self,
        py: Python<'_>,
        query: &str,
        k: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
        date_field: Option<String>,
        options: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let k = limits::clamp_recall_limit(k);
        let filter = to_metadata(py, filter)?;
        let opts = fusion_options_from_dict(options.as_ref())?;
        let hits = py.detach(|| {
            self.svc
                .recall_fused(query, k, filter.as_ref(), opts)
                .map_err(to_py_err)
        })?;
        // Format the dated timeline before the hits are consumed into the list.
        let dated = date_field
            .as_ref()
            .map(|field| format_dated_context(&hits, field));
        let memories = recollections_to_list(py, hits)?;
        match dated {
            None => Ok(memories.into()),
            Some(ctx) => dated_recall_to_dict(py, &memories, ctx),
        }
    }

    /// Create a typed edge `from_id -> to_id`. Returns the edge id.
    fn relate(&self, py: Python<'_>, from_id: u64, to_id: u64, relation: &str) -> PyResult<u64> {
        py.detach(|| self.svc.relate(from_id, to_id, relation).map_err(to_py_err))
    }

    /// Remove the typed edge(s) `from_id -relation-> to_id` — `relate`'s exact
    /// undo, so a mistaken edge no longer costs the facts at its endpoints.
    /// Only the edge goes: both memories, and any entity hub, are untouched.
    ///
    /// Idempotent: removing an absent edge returns ``found=False`` instead of
    /// raising, so a cleanup can be replayed. It refuses exactly what `relate`
    /// refuses (empty relation, `from_id` equal to `to_id`), and deliberately
    /// does NOT require the endpoints to still exist — the edge of a forgotten
    /// fact is already gone.
    ///
    /// Returns:
    ///     ``{"found": bool, "removed": int}`` — `removed` counts the edges
    ///     actually deleted, parallel duplicates included.
    fn unrelate(
        &self,
        py: Python<'_>,
        from_id: u64,
        to_id: u64,
        relation: &str,
    ) -> PyResult<Py<PyAny>> {
        let outcome = py.detach(|| {
            self.svc
                .unrelate(from_id, to_id, relation)
                .map_err(to_py_err)
        })?;
        let out = PyDict::new(py);
        out.set_item(PyString::intern(py, "found"), outcome.found)?;
        out.set_item(PyString::intern(py, "removed"), outcome.removed)?;
        Ok(out.into())
    }

    /// Delete a memory by id. Returns whether a memory actually existed
    /// under that id and was deleted — `False` means nothing was stored
    /// there (a stale id or a typo), not a second successful deletion.
    fn forget(&self, py: Python<'_>, id: u64) -> PyResult<bool> {
        py.detach(|| self.svc.forget(id).map_err(to_py_err))
    }

    /// Reinforce (`success=True`) or weaken (`False`) a memory after use,
    /// closing the RL loop `recall` re-ranks against. Returns the updated
    /// confidence in `[0.0, 1.0]`. Raises `KeyError` if `id` is not a live
    /// fact — unlike `forget`, there is no confidence to report back.
    fn feedback(&self, py: Python<'_>, id: u64, success: bool) -> PyResult<f64> {
        py.detach(|| {
            self.svc
                .feedback(id, success)
                .map(f64::from)
                .map_err(to_py_err)
        })
    }

    /// Explain a decision: the best-matching memory plus its connected subgraph
    /// (multi-hop). Returns `{nodes, edges, truncated}` — the wedge a plain
    /// recall misses. `truncated` is `True` when a width budget cut the walk:
    /// a subgraph sitting exactly at a cap is otherwise indistinguishable
    /// from a complete one.
    ///
    /// `max_hops` is silently capped at 10 to prevent unbounded traversal on
    /// dense graphs (same limit as the MCP server).
    #[pyo3(signature = (decision, max_hops = 2, filter = None))]
    fn why(
        &self,
        py: Python<'_>,
        decision: &str,
        max_hops: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let max_hops = limits::clamp_hops(max_hops);
        let filter = to_metadata(py, filter)?;
        let explanation = py.detach(|| {
            self.svc
                .why(decision, max_hops, filter.as_ref())
                .map_err(to_py_err)
        })?;
        explanation_to_dict(py, &explanation)
    }

    /// Look up everything the memory graph knows about a NAMED ENTITY (a
    /// person, a place, an organisation): the attributes merged onto its hub
    /// and the typed edges touching it, in BOTH directions. Answers a
    /// question ABOUT a thing ("how old is X", "who is X's father") rather
    /// than about the sentences mentioning it, which is all `recall` can
    /// return — entity hubs are deliberately invisible to `recall`, so
    /// without this the attributes `remember_extracted` builds would be
    /// unreachable.
    ///
    /// Args:
    ///     name: matched case-insensitively (the id is content-addressed, so
    ///         it is stable across sessions).
    ///
    /// Returns:
    ///     `{found, id, name, attributes, relations, relations_in,
    ///     relations_truncated, relations_in_truncated}`, each
    ///     edge being `{predicate, target_id, target}`. `found` is `False`
    ///     when nothing has ever mentioned that name; `name` still echoes the
    ///     query in its canonical (trimmed, lowercased) form, so several
    ///     lookups can be told apart. The `*_truncated` booleans say when a
    ///     list is a PARTIAL view cut by a response budget — a list holding
    ///     exactly the cap is otherwise indistinguishable from a cut one.
    ///
    ///     `relations` are the edges LEAVING the entity, `relations_in` those
    ///     pointing AT it — there, `target_id`/`target` name the far end the
    ///     edge comes FROM. Without the second list a question is only
    ///     answerable from one side: the graph holds
    ///     `camille --sister of--> theo`, so reading Theo's outgoing edges
    ///     never finds Camille.
    fn entity(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let profile = py.detach(|| self.svc.entity_profile(name).map_err(to_py_err))?;
        Ok(json_to_python(py, &entity_profile_to_value(name, profile)))
    }

    /// Extract atomic facts from raw `text` and store them, auto-building the
    /// entity graph the passage describes.
    ///
    /// Args:
    ///     text: the passage to extract from.
    ///     model: the local generative model, required by the `"ollama"`
    ///         backend and ignored by `"outline"`.
    ///     url: the Ollama base URL (`"ollama"` only).
    ///     metadata: merged onto every stored fact.
    ///     extractor: which backend to use, `"ollama"` (default) or
    ///         `"outline"`. `"outline"` is deterministic and network-free —
    ///         it reads the structure the passage STATES, one directive per
    ///         line (`edge:`, `attr:`, `fact:`) — and stands to `"ollama"`
    ///         exactly as the `"hash"` embedder does, so the whole contract
    ///         of this method is reachable with no model running.
    ///
    /// Returns:
    ///     `{"ids": [int, ...], "skipped_over_cap": int}`.
    ///
    ///     This used to return a bare list of ids, and that list could not
    ///     say why it was short: nothing distinguished a passage that held
    ///     three facts from one that held twelve of which nine were dropped
    ///     for exceeding the embeddable cap. The dict is the breaking change
    ///     that ends that silence (issue #1692).
    #[pyo3(signature = (text, model = None, url = None, metadata = None, extractor = None))]
    fn remember_extracted(
        &self,
        py: Python<'_>,
        text: &str,
        model: Option<String>,
        url: Option<String>,
        metadata: Option<HashMap<String, Py<PyAny>>>,
        extractor: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        let metadata = to_metadata(py, metadata)?;
        let outcome = extract_with(py, &self.svc, text, model, url, extractor, metadata)?;
        Ok(json_to_python(
            py,
            &serde_json::json!({
                "ids": outcome.ids,
                "skipped_over_cap": outcome.skipped_over_cap,
            }),
        ))
    }

    /// Compile context fragments into a token-budgeted, provenance-audited
    /// prompt context — deterministic, no LLM call. Delegates directly to the
    /// same `velesdb_memory::context` bridge the MCP `compile_context` tool
    /// and the Node binding use (zero new logic here). `request` is the same
    /// JSON shape as the MCP tool's input (`{query, fragments, token_budget,
    /// project?, target_model?, memory_scope?, policy?}`); the result is the
    /// same shape as its output (`{content, sections, decisions, sources,
    /// retrieval_handles, insights, risk, warnings}`). One documented difference from
    /// the Node binding: every u64 id (`fragment_id`, `content_hash`,
    /// `memory_id`, entries of `fragment_ids`) crosses as a **native Python
    /// int** (unlimited precision), not a decimal string — both are faithful
    /// renderings of the same value, never truncated.
    fn compile_context(&self, py: Python<'_>, request: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let request: CompileRequest = python_to_serde!(py, &request, "compile request");
        let compiled: CompiledContext = py.detach(|| {
            self.svc
                .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
                .map_err(to_py_err)
        })?;
        Ok(serde_to_python!(py, &compiled, "compiled context"))
    }

    /// One-call shortcut over [`compile_context`](Self::compile_context) for
    /// a raw agent-session transcript: deterministically segments it into
    /// turns (plain marker-based — `System:`/`User:`/`Human:`/`Assistant:`/
    /// `AI:`/`Tool:`/`### User`/`### Assistant` — or JSONL, one line per
    /// turn) and, within each turn, into code/log/body sub-segments, then
    /// compiles the result exactly like `compile_context`. Pure delegation to
    /// the same `velesdb_memory` bridge the Node and WASM bindings relay, so
    /// the same transcript segments identically on all three.
    ///
    /// Args:
    ///     request: Same JSON shape as the MCP `compile_transcript` tool's
    ///         input MINUS `path` — `{query, transcript, token_budget,
    ///         project?, target_model?, policy?, segmentation?}`. Resolving a
    ///         `path` needs the MCP server's ingest-roots allowlist, which
    ///         this binding has no configuration surface for: read the file
    ///         yourself and pass its content as `transcript`.
    ///
    /// Returns:
    ///     ``{"context": ..., "segmentation": ...}`` — `context` is the same
    ///     shape as `compile_context`'s output; `segmentation` is the
    ///     detected format plus one audit entry per segment (index, turn,
    ///     role, kind, byte range, `fragment_id`) so a caller can see exactly
    ///     how the transcript was cut before trusting the compiled result.
    ///
    /// Raises:
    ///     ValueError: If the transcript is empty, the request is malformed,
    ///         or a forced segmentation format fails to parse.
    fn compile_transcript(&self, py: Python<'_>, request: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let input: TranscriptCompileInput =
            python_to_serde!(py, &request, "compile_transcript request");
        let (request, segmentation) = build_transcript_compile_request(input).map_err(to_py_err)?;
        let compiled: CompiledContext = py.detach(|| {
            self.svc
                .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
                .map_err(to_py_err)
        })?;
        let out = PyDict::new(py);
        let context = serde_to_python!(py, &compiled, "compiled context");
        let report = serde_to_python!(py, &segmentation, "segmentation report");
        out.set_item(PyString::intern(py, "context"), context)?;
        out.set_item(PyString::intern(py, "segmentation"), report)?;
        Ok(out.into())
    }

    /// Suggest a starting `token_budget` for
    /// [`compile_context`](Self::compile_context), for a named target model
    /// — looked up in a static, committed model-name to context-window table
    /// (dated "as of", NEVER a network call).
    ///
    /// Args:
    ///     target_model: Model name, matched case-insensitively.
    ///     reserve_tokens: Room to reserve for the response (default 0),
    ///         mirroring `compile_context`'s own
    ///         `policy.response_reserve_tokens`.
    ///
    /// Returns:
    ///     ``{"window": Optional[int], "suggested_budget": Optional[int],
    ///        "source": str}``. Both figures are ``None`` for a model absent
    ///     from the table — an honest "unknown", never a guess; the table is
    ///     extended in a new release rather than worked around here.
    #[pyo3(signature = (target_model, reserve_tokens = 0))]
    fn suggest_budget(
        &self,
        py: Python<'_>,
        target_model: &str,
        reserve_tokens: u64,
    ) -> PyResult<Py<PyAny>> {
        let budget = suggest_token_budget(target_model, reserve_tokens);
        Ok(serde_to_python!(py, &budget, "suggested budget"))
    }

    /// Fetch back the exact original content — and media, when the fragment
    /// carried one (US-009, PR2) — behind a `ctx://source/<hash>` handle
    /// from a [`compile_context`](Self::compile_context) result: what was
    /// externalized or partially packed is recoverable, not lost. Returns a
    /// dict shaped `{handle, content, media?}`, `media` present only for a source
    /// whose fragment carried one.
    fn retrieve_context_source(&self, py: Python<'_>, handle: &str) -> PyResult<Py<PyAny>> {
        let source: ContextSource =
            py.detach(|| self.svc.retrieve_context_source(handle).map_err(to_py_err))?;
        Ok(serde_to_python!(py, &source, "context source"))
    }

    /// Aggregate the token (and cost) savings of past
    /// [`compile_context`](Self::compile_context) calls, optionally narrowed
    /// to one `project`. Figures are local estimates recorded per
    /// compilation (metadata only, never fragment content); `truncated`
    /// reports when the sweep hit the recall cap.
    #[pyo3(signature = (project = None))]
    fn context_savings(&self, py: Python<'_>, project: Option<&str>) -> PyResult<Py<PyAny>> {
        let savings: ContextSavings =
            py.detach(|| self.svc.context_savings(project).map_err(to_py_err))?;
        Ok(serde_to_python!(py, &savings, "context savings"))
    }

    /// Explain why one fragment of a [`compile_context`](Self::compile_context)
    /// request was preserved, abstracted, externalized, dropped, or cached.
    /// Compilation is deterministic, so `request` is re-compiled (with
    /// event/source recording forced off) and the matching decision is
    /// returned — no server-side state needed. Delegates directly to the
    /// same `velesdb_memory::context` bridge the MCP `explain_compilation`
    /// tool and the Node binding use (zero new logic here).
    ///
    /// Args:
    ///     request: Same JSON shape as `compile_context`'s input.
    ///     fragment_id: The fragment whose decision to return. Looked up by
    ///         matching `decisions[].fragment_id`, UNLESS `fragment_index`
    ///         is also given — still required even then, since it is the
    ///         only disambiguator when `fragment_index` is absent.
    ///     fragment_index: Optional, 0-based position of the fragment in
    ///         `request["fragments"]`. When given, TAKES PRIORITY over
    ///         `fragment_id` for locating the decision — unambiguous even
    ///         when several fragments are byte-identical (and therefore
    ///         share the same content-addressed `fragment_id`): a plain
    ///         `fragment_id` lookup always resolves to the FIRST such
    ///         decision (the deduplication survivor's), never a dropped
    ///         twin's.
    ///
    /// Returns:
    ///     Same shape as one entry of `compile_context`'s `decisions`.
    ///
    /// Raises:
    ///     ValueError: If `fragment_index` is out of bounds, or no fragment
    ///         matches the selector, or the request is malformed.
    #[pyo3(signature = (request, fragment_id, fragment_index = None))]
    fn explain_compilation(
        &self,
        py: Python<'_>,
        request: Py<PyAny>,
        fragment_id: u64,
        fragment_index: Option<usize>,
    ) -> PyResult<Py<PyAny>> {
        let request: CompileRequest = python_to_serde!(py, &request, "compile request");
        let decision: ContextDecision = py.detach(|| {
            self.svc
                .explain_compilation(&request, fragment_id, fragment_index)
                .map_err(to_py_err)
        })?;
        Ok(serde_to_python!(py, &decision, "context decision"))
    }

    /// Persist `working` (the same JSON shape as `WorkingContext` on the
    /// wire: `{goal?, active_constraints, verified_facts, open_hypotheses,
    /// decisions, exact_evidence, pending_actions}`) under `project` +
    /// `session`. Saving again under the same pair replaces the previous
    /// state (idempotent upsert). Returns the stored system fact id.
    fn save_working_context(
        &self,
        py: Python<'_>,
        project: &str,
        session: &str,
        working: Py<PyAny>,
    ) -> PyResult<u64> {
        let working: WorkingContext = python_to_serde!(py, &working, "working context");
        py.detach(|| {
            self.svc
                .save_working_context(project, session, &working)
                .map_err(to_py_err)
        })
    }

    /// The resumption envelope for `project` + `session` (see
    /// [`save_working_context`](Self::save_working_context)) — the same
    /// wire shape as the MCP `load_working_context` tool and the Node/WASM
    /// bindings.
    ///
    /// **BREAKING (0.12.0)**: this used to return the bare working context
    /// (or `None`), which collapsed two different answers into one — a
    /// project that never saved anything, and a typo in `session` that
    /// missed a session which does exist. `other_sessions` is what tells
    /// them apart, and it is filled in on a HIT too: a typo landing on
    /// another REAL session returns `found=True`, the case a caller can
    /// least detect on its own. Read `["working"]` for the previous return
    /// value.
    ///
    /// Returns:
    ///     ``{"found": bool, "working": dict | None, "other_sessions":
    ///     [str]}``. Ids stay native Python ints here (unlimited precision),
    ///     unlike the JS bindings' decimal strings.
    /// The server's health, in the SAME envelope the MCP `memory_status`
    /// tool returns: which embedder RUNS (`embedder["semantic"] is False`
    /// means the offline `hash` default — recall matches surface form, not
    /// meaning), what the store was FILLED by per its on-disk provenance
    /// record, the extraction wiring (this binding passes its extractor per
    /// `remember_extracted` call, so nothing is pre-attached and the
    /// autograph fields report the service's actual state), and the corpus
    /// size — `memory["edges"] == 0` is the observable "why() has nothing
    /// to walk" state.
    fn memory_status(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let (recorded, facts, edges) = py.detach(|| {
            let recorded = velesdb_memory::embedding_provenance::read(&self.store_dir)
                .ok()
                .flatten();
            (recorded, self.svc.fact_count(), self.svc.edge_count())
        });
        let provenance = serde_json::json!({
            "recorded": recorded.is_some(),
            "model": recorded.as_ref().map(|record| &record.model),
            "dimension": recorded.as_ref().map(|record| record.dimension),
        });
        let status = serde_json::json!({
            "embedder": {
                "model": self.embedder_model,
                "dimension": self.embedder_dimension,
                "semantic": self.embedder_semantic,
            },
            "provenance": provenance,
            "extraction": {
                "configured": self.svc.has_autograph(),
                "autograph_active": self.svc.autograph_queue_open(),
                "autograph_dropped": self.svc.autograph_dropped(),
                "autograph_failed": self.svc.autograph_failed(),
            },
            "memory": { "facts": facts, "edges": edges },
        });
        Ok(json_to_python(py, &status))
    }

    /// Audit the store page by page — the SAME walk and envelope as the MCP
    /// `list_memories` tool: ids ascending, TTL-expired facts skipped,
    /// metadata under recall's visibility rule unless `include_internal`.
    /// `cursor` is the previous page's `next_cursor`; `None` ends the walk.
    /// A filtered page may come back sparse — keep following the cursor,
    /// the walk stays exhaustive.
    #[pyo3(signature = (cursor = None, limit = 50, filter = None, include_internal = false))]
    fn list_memories(
        &self,
        py: Python<'_>,
        cursor: Option<u64>,
        limit: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
        include_internal: bool,
    ) -> PyResult<Py<PyAny>> {
        let filter = to_metadata(py, filter)?;
        let (memories, next) = py.detach(|| {
            self.svc
                .list(cursor, limit, filter.as_ref(), include_internal)
                .map_err(to_py_err)
        })?;
        let entries: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|memory| {
                serde_json::json!({
                    "id": memory.id,
                    "id_str": memory.id.to_string(),
                    "content": memory.content,
                    "metadata": memory.metadata,
                })
            })
            .collect();
        let page = serde_json::json!({
            "memories": entries,
            "next_cursor": next.map(|id| id.to_string()),
        });
        Ok(json_to_python(py, &page))
    }

    fn load_working_context(
        &self,
        py: Python<'_>,
        project: &str,
        session: &str,
    ) -> PyResult<Py<PyAny>> {
        // Annotated, not inferred: `binding_parity_bdd` reads this type name
        // to prove the binding relays the SERVER's own envelope rather than a
        // shape it recomposed by hand — and the compiler makes that proof
        // real. The doc comment above describes the envelope; only this
        // enforces it.
        let loaded: LoadedWorkingContext = py.detach(|| {
            self.svc
                .resume_working_context(project, session)
                .map_err(to_py_err)
        })?;
        Ok(serde_to_python!(py, &loaded, "loaded working context"))
    }

    /// Every session ever saved under `project`'s working-context index,
    /// most-recently-saved first — so an agent can discover what is
    /// resumable before guessing a session id at
    /// [`load_working_context`](Self::load_working_context), or recover from
    /// a typo.
    ///
    /// Returns:
    ///     ``{"sessions": [{"session": str, "saved_at": int}, ...]}`` — the
    ///     same wire shape as the MCP `list_working_contexts` tool and the
    ///     Node/WASM bindings. Empty (never an error) when the project never
    ///     saved anything.
    fn list_working_contexts(&self, py: Python<'_>, project: &str) -> PyResult<Py<PyAny>> {
        let sessions: Vec<WorkingContextSession> =
            py.detach(|| self.svc.list_working_contexts(project).map_err(to_py_err))?;
        let listed = serde_to_python!(py, &sessions, "working context sessions");
        let out = PyDict::new(py);
        out.set_item(PyString::intern(py, "sessions"), listed)?;
        Ok(out.into())
    }
}
