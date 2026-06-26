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

use velesdb_memory::{
    DynEmbedder, Explanation, HashEmbedder, Link, MemoryError, MemoryService, Metadata,
    OllamaEmbedder, OllamaExtractor, Recollection, DEFAULT_DIMENSION, DEFAULT_OLLAMA_MODEL,
    DEFAULT_OLLAMA_URL,
};

use crate::collection::query::convert_params;

/// Map a [`MemoryError`] to the most specific Python exception: caller-input
/// errors → `ValueError`, a missing memory id → `KeyError`, the rest →
/// `RuntimeError`.
fn to_py_err(e: &MemoryError) -> PyErr {
    match e {
        MemoryError::EmptyFact | MemoryError::InvalidFilter(_) | MemoryError::ReservedKey(_) => {
            PyValueError::new_err(e.to_string())
        }
        MemoryError::UnknownMemory(_) => PyKeyError::new_err(e.to_string()),
        _ => PyRuntimeError::new_err(e.to_string()),
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

/// One [`Recollection`] as a Python dict `{id, score, content}`.
fn recollection_to_dict(py: Python<'_>, r: &Recollection) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item(PyString::intern(py, "id"), r.id)?;
    dict.set_item(PyString::intern(py, "score"), r.score)?;
    dict.set_item(PyString::intern(py, "content"), &r.content)?;
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
        let svc = MemoryService::open(&path, emb).map_err(|e| to_py_err(&e))?;
        Ok(Self { svc })
    }

    /// Store a fact; returns its stable id. `links` is a list of `(target_id,
    /// relation)` tuples; `metadata` is an optional dict for later filtering.
    #[pyo3(signature = (fact, links = None, metadata = None))]
    fn remember(
        &self,
        py: Python<'_>,
        fact: &str,
        links: Option<Vec<(u64, String)>>,
        metadata: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<u64> {
        let links: Vec<Link> = links
            .unwrap_or_default()
            .into_iter()
            .map(|(target, relation)| Link { target, relation })
            .collect();
        let metadata = to_metadata(py, metadata)?;
        py.detach(|| {
            self.svc
                .remember(fact, &links, metadata.as_ref())
                .map_err(|e| to_py_err(&e))
        })
    }

    /// Recall up to `k` memories similar to `query`, optionally narrowed by an
    /// exact-match metadata `filter`. Returns a list of `{id, score, content}`.
    #[pyo3(signature = (query, k = 10, filter = None))]
    fn recall(
        &self,
        py: Python<'_>,
        query: &str,
        k: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let filter = to_metadata(py, filter)?;
        let hits = py.detach(|| {
            self.svc
                .recall(query, k, filter.as_ref())
                .map_err(|e| to_py_err(&e))
        })?;
        let list = PyList::empty(py);
        for hit in &hits {
            list.append(recollection_to_dict(py, hit)?)?;
        }
        Ok(list.into())
    }

    /// Create a typed edge `from_id -> to_id`. Returns the edge id.
    fn relate(&self, py: Python<'_>, from_id: u64, to_id: u64, relation: &str) -> PyResult<u64> {
        py.detach(|| {
            self.svc
                .relate(from_id, to_id, relation)
                .map_err(|e| to_py_err(&e))
        })
    }

    /// Delete a memory by id.
    fn forget(&self, py: Python<'_>, id: u64) -> PyResult<()> {
        py.detach(|| self.svc.forget(id).map_err(|e| to_py_err(&e)))
    }

    /// Explain a decision: the best-matching memory plus its connected subgraph
    /// (multi-hop). Returns `{nodes, edges}` — the wedge a plain recall misses.
    #[pyo3(signature = (decision, max_hops = 2, filter = None))]
    fn why(
        &self,
        py: Python<'_>,
        decision: &str,
        max_hops: usize,
        filter: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let filter = to_metadata(py, filter)?;
        let explanation = py.detach(|| {
            self.svc
                .why(decision, max_hops, filter.as_ref())
                .map_err(|e| to_py_err(&e))
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
                .map_err(|e| to_py_err(&e))
        })
    }
}
