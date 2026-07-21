//! Python binding for the high-level `velesdb-memory` `MemoryService` — the
//! agent-memory *wedge*: `remember` / `recall` / `relate` / `forget` / `why`
//! plus `remember_extracted` (auto text → fact↔topic graph).
//!
//! Unlike the lower-level `AgentMemory` binding (bring-your-own-vector), this
//! service **embeds text itself**, so the constructor selects an embedder. It
//! wraps the exact same hardened Rust used by the MCP server — no logic is
//! reimplemented here.

use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use std::collections::HashMap;

use velesdb_memory::context::{
    CompilePolicy, CompileRequest, CompiledContext, ContextCompiler, ContextDecision,
    ContextSavings, ContextSource, WorkingContext,
};
use velesdb_memory::{
    format_dated_context, limits, ColumnFilter, ColumnOp, DynEmbedder, ErrorCategory, Explanation,
    FusionOptions, HashEmbedder, Link, MemoryError, MemoryService, Metadata, OllamaEmbedder,
    OllamaExtractor, Recollection, DEFAULT_DIMENSION, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL,
};

use crate::collection::query::convert_params;
use crate::utils::{json_to_python, opt_field, python_to_json};

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

/// Map a [`MemoryError`] to the most specific Python exception, driven by its
/// transport-neutral [`ErrorCategory`] so the taxonomy stays identical to the
/// MCP server and the Node binding: caller-input → `ValueError`, a missing
/// memory id → `KeyError`, the rest → `RuntimeError`.
fn to_py_err(e: MemoryError) -> PyErr {
    let msg = e.to_string();
    match e.category() {
        ErrorCategory::InvalidInput => PyValueError::new_err(msg),
        ErrorCategory::NotFound => PyKeyError::new_err(msg),
        ErrorCategory::Internal => PyRuntimeError::new_err(msg),
    }
}

/// Parse a column-filter operator token (`eq`/`ne`/`lt`/`le`/`gt`/`ge`).
fn parse_op(op: &str) -> PyResult<ColumnOp> {
    match op.to_ascii_lowercase().as_str() {
        "eq" => Ok(ColumnOp::Eq),
        "ne" => Ok(ColumnOp::Ne),
        "lt" => Ok(ColumnOp::Lt),
        "le" => Ok(ColumnOp::Le),
        "gt" => Ok(ColumnOp::Gt),
        "ge" => Ok(ColumnOp::Ge),
        other => Err(PyValueError::new_err(format!(
            "unknown filter op '{other}' (expected eq/ne/lt/le/gt/ge)"
        ))),
    }
}

/// Build the requested embedder. `"hash"` is deterministic and offline;
/// `"ollama"` calls a local embedding model (real semantic recall).
fn build_embedder(kind: &str, url: Option<String>, model: Option<String>) -> PyResult<DynEmbedder> {
    match kind {
        "hash" => Ok(Box::new(HashEmbedder::new(DEFAULT_DIMENSION))),
        "ollama" => {
            let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
            let model = model.unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
            let embedder = OllamaEmbedder::new(url, model)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(Box::new(embedder))
        }
        other => Err(PyValueError::new_err(format!(
            "unknown embedder '{other}' (expected 'hash' or 'ollama')"
        ))),
    }
}

/// Convert a Python metadata/filter dict into the engine's [`Metadata`] map,
/// reusing the crate's `Py` → `serde_json::Value` conversion.
fn to_metadata(
    py: Python<'_>,
    map: Option<HashMap<String, Py<PyAny>>>,
) -> PyResult<Option<Metadata>> {
    match map {
        None => Ok(None),
        Some(map) => {
            let converted = convert_params(py, Some(map))?;
            Ok(Some(converted.into_iter().collect()))
        }
    }
}

/// One [`Recollection`] as a Python dict `{id, score, content, metadata}`.
///
/// `metadata` mirrors `Recollection.metadata: Option<Map<String, Value>>` —
/// populated for `recall_where` results, `None` (i.e. Python `None`) for
/// `recall`/`why` (intentional, see the Rust doc on that field). Takes `r` by
/// value so the metadata map can be moved into the `serde_json::Value` instead
/// of cloned.
fn recollection_to_dict(py: Python<'_>, r: Recollection) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item(PyString::intern(py, "id"), r.id)?;
    dict.set_item(PyString::intern(py, "score"), r.score)?;
    dict.set_item(PyString::intern(py, "content"), &r.content)?;
    let metadata = match r.metadata {
        Some(map) => json_to_python(py, &serde_json::Value::Object(map)),
        None => py.None(),
    };
    dict.set_item(PyString::intern(py, "metadata"), metadata)?;
    Ok(dict.into())
}

/// An [`Explanation`] as `{nodes: [{id, content, hop}], edges: [{from, to, relation}]}`.
fn explanation_to_dict(py: Python<'_>, e: &Explanation) -> PyResult<Py<PyAny>> {
    let nodes = PyList::empty(py);
    for n in &e.nodes {
        let d = PyDict::new(py);
        d.set_item(PyString::intern(py, "id"), n.id)?;
        d.set_item(PyString::intern(py, "content"), &n.content)?;
        d.set_item(PyString::intern(py, "hop"), n.hop)?;
        nodes.append(d)?;
    }
    let edges = PyList::empty(py);
    for edge in &e.edges {
        let d = PyDict::new(py);
        d.set_item(PyString::intern(py, "from"), edge.from)?;
        d.set_item(PyString::intern(py, "to"), edge.to)?;
        d.set_item(PyString::intern(py, "relation"), &edge.relation)?;
        edges.append(d)?;
    }
    let out = PyDict::new(py);
    out.set_item(PyString::intern(py, "nodes"), nodes)?;
    out.set_item(PyString::intern(py, "edges"), edges)?;
    Ok(out.into())
}

/// Build [`FusionOptions`] from an optional Python `{hops?, graph_boost?, pool?}`
/// dict, routed through [`FusionOptions::from_knobs`] so the defaults and clamps
/// stay identical to the MCP tool. An absent dict (or one omitting a key) uses
/// the proven defaults.
fn fusion_options_from_dict(options: Option<&Bound<'_, PyDict>>) -> PyResult<FusionOptions> {
    let Some(dict) = options else {
        return Ok(FusionOptions::from_knobs(None, None, None));
    };
    let hops = opt_field(dict, "hops")?;
    let graph_boost = opt_field(dict, "graph_boost")?;
    let pool = opt_field(dict, "pool")?;
    Ok(FusionOptions::from_knobs(hops, graph_boost, pool))
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
}

#[pymethods]
impl PyMemoryService {
    /// Open (or create) a memory store at `path`.
    ///
    /// Args:
    ///     path: store directory (created if missing; memory never leaves it).
    ///     embedder: "hash" (default, offline) or "ollama" (real semantic recall).
    ///     ollama_url / ollama_model: used when embedder="ollama".
    #[new]
    #[pyo3(signature = (path, embedder = "hash", ollama_url = None, ollama_model = None))]
    fn new(
        path: String,
        embedder: &str,
        ollama_url: Option<String>,
        ollama_model: Option<String>,
    ) -> PyResult<Self> {
        let emb = build_embedder(embedder, ollama_url, ollama_model)?;
        let svc = MemoryService::open(&path, emb).map_err(to_py_err)?;
        Ok(Self { svc })
    }

    /// Store a fact; returns its stable id. `links` is a list of `(target_id,
    /// relation)` tuples; `metadata` is an optional dict for later filtering.
    /// `ttl_seconds` makes the fact expire after that many seconds (a durable TTL
    /// that survives restarts); omit it (or `0`) for a permanent memory.
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
        let list = PyList::empty(py);
        for hit in hits {
            list.append(recollection_to_dict(py, hit)?)?;
        }
        Ok(list.into())
    }

    /// Fused vector + `ColumnStore` recall: like [`recall`](Self::recall) but the
    /// `filters` support ranges/comparisons, so numeric/temporal facets become
    /// queryable. `filters` is a list of `(field, op, value)` tuples where `op`
    /// is one of `eq`/`ne`/`lt`/`le`/`gt`/`ge`. Returns `{id, score, content, metadata}`
    /// (`metadata` is the fact's stored dict, or `None` if it carried none).
    #[pyo3(signature = (query, filters, k = 10))]
    fn recall_where(
        &self,
        py: Python<'_>,
        query: &str,
        filters: Vec<(String, String, Py<PyAny>)>,
        k: usize,
    ) -> PyResult<Py<PyAny>> {
        let k = limits::clamp_recall_limit(k);
        let filters: Vec<ColumnFilter> = filters
            .into_iter()
            .map(|(field, op, value)| {
                Ok(ColumnFilter {
                    field,
                    op: parse_op(&op)?,
                    value: python_to_json(py, &value)?,
                })
            })
            .collect::<PyResult<Vec<_>>>()?;
        let hits = py.detach(|| self.svc.recall_where(query, k, &filters).map_err(to_py_err))?;
        let list = PyList::empty(py);
        for hit in hits {
            list.append(recollection_to_dict(py, hit)?)?;
        }
        Ok(list.into())
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

        let memories = PyList::empty(py);
        for hit in hits {
            memories.append(recollection_to_dict(py, hit)?)?;
        }
        match dated {
            None => Ok(memories.into()),
            Some(ctx) => {
                let out = PyDict::new(py);
                out.set_item(PyString::intern(py, "memories"), memories)?;
                out.set_item(PyString::intern(py, "dated_context"), ctx.timeline)?;
                out.set_item(PyString::intern(py, "now"), ctx.now)?;
                Ok(out.into())
            }
        }
    }

    /// Create a typed edge `from_id -> to_id`. Returns the edge id.
    fn relate(&self, py: Python<'_>, from_id: u64, to_id: u64, relation: &str) -> PyResult<u64> {
        py.detach(|| self.svc.relate(from_id, to_id, relation).map_err(to_py_err))
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
    /// (multi-hop). Returns `{nodes, edges}` — the wedge a plain recall misses.
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

    /// Extract atomic facts from raw `text` with a local Ollama model and store
    /// them, auto-building the fact↔topic graph. Returns the stored facts' ids.
    #[pyo3(signature = (text, model, url = None, metadata = None))]
    fn remember_extracted(
        &self,
        py: Python<'_>,
        text: &str,
        model: String,
        url: Option<String>,
        metadata: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<u64>> {
        let metadata = to_metadata(py, metadata)?;
        let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
        let extractor = OllamaExtractor::new(url, model);
        py.detach(|| {
            self.svc
                .remember_extracted(text, &extractor, metadata.as_ref())
                .map_err(to_py_err)
        })
    }

    /// Compile context fragments into a token-budgeted, provenance-audited
    /// prompt context — deterministic, no LLM call. Delegates directly to the
    /// same `velesdb_memory::context` bridge the MCP `compile_context` tool
    /// and the Node binding use (zero new logic here). `request` is the same
    /// JSON shape as the MCP tool's input (`{query, fragments, token_budget,
    /// project?, target_model?, memory_scope?, policy?}`); the result is the
    /// same shape as its output (`{content, sections, decisions, sources,
    /// retrieval_handles, insights, risk}`). One documented difference from
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

    /// Fetch back the exact original content — and media, when the fragment
    /// carried one (US-009, PR2) — behind a `ctx://source/<hash>` handle
    /// from a [`compile_context`](Self::compile_context) result: what was
    /// externalized or partially packed is recoverable, not lost. Returns a
    /// dict shaped `{content, media?}`, `media` present only for a source
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

    /// The working context previously saved under `project` + `session` (see
    /// [`save_working_context`](Self::save_working_context)), or `None` when
    /// there is none.
    fn load_working_context(
        &self,
        py: Python<'_>,
        project: &str,
        session: &str,
    ) -> PyResult<Py<PyAny>> {
        let working = py.detach(|| {
            self.svc
                .load_working_context(project, session)
                .map_err(to_py_err)
        })?;
        match working {
            None => Ok(py.None()),
            Some(working) => Ok(serde_to_python!(py, &working, "working context")),
        }
    }
}
