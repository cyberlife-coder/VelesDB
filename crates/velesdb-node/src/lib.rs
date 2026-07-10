//! Node.js (napi-rs) binding for the `velesdb-memory` `MemoryService` — the
//! agent-memory wedge: `remember` / `recall` / `recallWhere` / `relate` /
//! `forget` / `why` / `rememberExtracted`.
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
use velesdb_memory::{
    DynEmbedder, HashEmbedder, MemoryService, OllamaEmbedder, OllamaExtractor, DEFAULT_DIMENSION,
    DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL,
};

use crate::dto::{
    ColumnFilterJs, DatedRecallJs, ExplanationJs, FusionOptionsJs, LinkJs, RecollectionJs,
};
use crate::error::{invalid_input, to_napi_err, CODE_INTERNAL};
use crate::tasks::Job;

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

    /// Delete a memory by id.
    #[napi(ts_return_type = "Promise<void>")]
    pub fn forget(&self, id: String) -> AsyncTask<Job<()>> {
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

    /// Extract atomic facts from raw `text` with a local Ollama `model` and store
    /// them, auto-building the fact↔topic graph. Resolves to the stored ids.
    #[napi(
        js_name = "rememberExtracted",
        ts_return_type = "Promise<Array<string>>"
    )]
    pub fn remember_extracted(
        &self,
        text: String,
        model: String,
        url: Option<String>,
        metadata: Option<Value>,
    ) -> AsyncTask<Job<Vec<String>>> {
        let svc = Arc::clone(&self.inner);
        AsyncTask::new(Job::new(move || {
            guards::check_fact(&text)?;
            let metadata = convert::to_metadata(metadata)?;
            let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
            let extractor = OllamaExtractor::new(url, model);
            let ids = svc
                .remember_extracted(&text, &extractor, metadata.as_ref())
                .map_err(to_napi_err)?;
            Ok(ids.into_iter().map(convert::id_to_string).collect())
        }))
    }
}
