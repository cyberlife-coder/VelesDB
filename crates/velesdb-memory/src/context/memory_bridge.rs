//! The context compiler's memory bridge: memory-backed fragment selection,
//! recoverable sources, aggregatable compilation events, and persisted
//! working contexts — the `MemoryService` half of EPIC-P-070's US-002.
//!
//! Everything the bridge persists is a **system fact**: hub-marked
//! (`_veles_hub`, the same reserved marker entity hubs use) so it never
//! surfaces in normal recall, and stored under a **salted id space** so no
//! caller fact can collide with it. Events carry metadata and hashes only —
//! never fragment content. Event recording stamps wall-clock time; the
//! compile pipeline itself stays clock-free and deterministic.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Number, Value};

use super::{MemoryService, Metadata, HUB_FIELD};
use crate::context::model::{
    CompileRequest, CompiledContext, ContextFragment, ContextSavings, MemoryScope, WorkingContext,
};
use crate::context::{provenance, ContextCompiler};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::id::stable_id;
use crate::model::{ColumnFilter, ColumnOp, FusionOptions};
use crate::storage::MemoryStore;

/// Salt for stored source ids — disjoint from natural fact ids, so a caller
/// later remembering the same text can never overwrite a stored source (or
/// inherit its system marker).
const SOURCE_ID_SALT: &str = "veles-ctx-source:";
/// Salt for compilation-event ids.
const EVENT_ID_SALT: &str = "veles-ctx-event:";
/// Salt for working-context ids (deterministic per project+session, so a
/// save is an idempotent upsert).
const WORKING_ID_SALT: &str = "veles-ctx-working:";

/// The constant lexical anchor every event's content starts with, so one
/// vector query can sweep the event family for aggregation.
const EVENT_ANCHOR: &str = "veles context compilation event";

/// Metadata key flagging a compilation event (unreserved on purpose: it is
/// what [`MemoryService::context_savings`] filters on through `recall_where`).
const EVENT_FLAG: &str = "ctx_event";

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// [`ContextCompiler::compile`] with this service's memory folded in:
    /// when the request carries a [`MemoryScope`], relevant memories are
    /// pulled through the fused vector+graph recall and compiled alongside
    /// the caller's fragments, each with its `memory_id` and a normalised
    /// fused-ranking relevance recorded in provenance. Afterwards (policy
    /// permitting) the distinct originals are stored so every
    /// `ctx://source/<hash>` handle round-trips, and a metadata-only
    /// compilation event is recorded for [`Self::context_savings`].
    ///
    /// # Errors
    /// Returns [`MemoryError`] if compilation itself fails (budget, caps),
    /// or if recall, embedding, or storage fails.
    pub fn compile_context(
        &self,
        compiler: &ContextCompiler,
        request: &CompileRequest,
    ) -> Result<CompiledContext, MemoryError> {
        let memories = self.context_memories(request)?;
        let mut augmented = request.clone();
        let mut pulled: BTreeMap<u64, PulledMemory> = BTreeMap::new();
        for memory in memories {
            augmented.fragments.push(memory.fragment.clone());
            pulled.insert(stable_id(&memory.fragment.content), memory);
        }
        let mut out = compiler.compile(&augmented)?;
        annotate_memory_provenance(&mut out, &pulled);
        let policy = compiler.effective_policy(request);
        if policy.store_sources {
            self.store_context_sources(&augmented, &out, policy.source_ttl_seconds)?;
        }
        if policy.record_events {
            self.record_context_event(request, &out, policy.event_ttl_seconds)?;
        }
        Ok(out)
    }

    /// The memories a request's scope pulls in, as compile fragments plus
    /// their id and normalised fused relevance.
    fn context_memories(&self, request: &CompileRequest) -> Result<Vec<PulledMemory>, MemoryError> {
        let Some(scope) = &request.memory_scope else {
            return Ok(Vec::new());
        };
        let k = crate::limits::clamp_recall_limit(scope.k.unwrap_or(DEFAULT_MEMORY_K));
        let filter = scope_filter(scope);
        let scored =
            self.recall_fused_scored(&request.query, k, filter.as_ref(), FusionOptions::default())?;
        let max_fused = scored
            .iter()
            .map(|s| s.fused)
            .fold(f64::MIN, f64::max)
            .max(f64::EPSILON);
        Ok(scored
            .into_iter()
            .map(|scored| {
                let memory_id = scored.recollection.id;
                #[allow(clippy::cast_possible_truncation)] // normalised into [0, 1]
                let relevance = (scored.fused / max_fused).clamp(0.0, 1.0) as f32;
                let fragment = ContextFragment {
                    id: None,
                    content: scored.recollection.content,
                    kind: Some("memory".to_owned()),
                    priority: None,
                    metadata: None,
                };
                PulledMemory {
                    fragment,
                    memory_id,
                    relevance,
                    vector_norm: scored.vector_norm,
                    graph_weight: scored.graph_weight,
                }
            })
            .collect())
    }

    /// Store every distinct fragment's original as a hub-marked system fact
    /// keyed by its salted content hash, so its handle can be resolved later.
    fn store_context_sources(
        &self,
        augmented: &CompileRequest,
        out: &CompiledContext,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let by_hash: BTreeMap<u64, &str> = augmented
            .fragments
            .iter()
            .map(|fragment| (stable_id(&fragment.content), fragment.content.as_str()))
            .collect();
        for source in &out.sources {
            let Some(hash) = provenance::parse_handle(&source.handle) else {
                continue;
            };
            let Some(content) = by_hash.get(&hash) else {
                continue;
            };
            let embedding = self.embedder.embed(content)?;
            self.store_fact(
                source_id(hash),
                content,
                &embedding,
                Some(&system_meta(&[])),
                ttl_seconds,
            )?;
        }
        Ok(())
    }

    /// The original content behind a `ctx://source/<hash>` handle.
    ///
    /// # Errors
    /// Returns [`MemoryError::UnknownHandle`] when the handle is malformed
    /// or nothing is stored under it (never stored, expired, or forgotten).
    pub fn retrieve_context_source(&self, handle: &str) -> Result<String, MemoryError> {
        let unknown = || MemoryError::UnknownHandle(handle.to_owned());
        let hash = provenance::parse_handle(handle).ok_or_else(unknown)?;
        self.store
            .get(source_id(hash))?
            .map(|(content, _)| content)
            .ok_or_else(unknown)
    }

    /// Record one compilation's savings as a metadata-only system fact
    /// (hashes and token counts — never fragment content). Wall-clock time
    /// is stamped here, outside the deterministic compile pipeline.
    fn record_context_event(
        &self,
        request: &CompileRequest,
        out: &CompiledContext,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let occurred_at_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let content = format!("{EVENT_ANCHOR} {occurred_at_nanos}");
        let id = stable_id(&format!(
            "{EVENT_ID_SALT}{occurred_at_nanos}:{}:{}",
            out.insights.tokens_in, out.insights.tokens_out
        ));
        let embedding = self.embedder.embed(&content)?;
        let meta = event_meta(request, out, occurred_at_nanos);
        self.store_fact(id, &content, &embedding, Some(&meta), ttl_seconds)?;
        Ok(())
    }

    /// Aggregate the recorded compilation events, optionally per project.
    /// Sweeps at most [`crate::limits::MAX_RECALL_LIMIT`] events (newest
    /// need not be first — the sweep is similarity-ordered over a constant
    /// anchor, i.e. effectively the whole family until the cap);
    /// [`ContextSavings::truncated`] reports when the cap was hit.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the underlying filtered recall fails.
    pub fn context_savings(&self, project: Option<&str>) -> Result<ContextSavings, MemoryError> {
        let mut filters = vec![ColumnFilter {
            field: EVENT_FLAG.to_owned(),
            op: ColumnOp::Eq,
            value: Value::Bool(true),
        }];
        if let Some(project) = project {
            filters.push(ColumnFilter {
                field: "project".to_owned(),
                op: ColumnOp::Eq,
                value: Value::String(project.to_owned()),
            });
        }
        let hits = self.recall_where(EVENT_ANCHOR, crate::limits::MAX_RECALL_LIMIT, &filters)?;
        Ok(aggregate_events(&hits))
    }

    /// Persist `working` under `project` + `session` (idempotent upsert:
    /// saving again replaces the previous state). Returns the system fact id.
    ///
    /// # Errors
    /// Returns [`MemoryError::WorkingContextCodec`] if serialization fails,
    /// or a storage/embedding error.
    pub fn save_working_context(
        &self,
        project: &str,
        session: &str,
        working: &WorkingContext,
    ) -> Result<u64, MemoryError> {
        let content = serde_json::to_string(working)
            .map_err(|err| MemoryError::WorkingContextCodec(err.to_string()))?;
        let id = working_id(project, session);
        let embedding = self
            .embedder
            .embed(&format!("working context {project} {session}"))?;
        let meta = system_meta(&[
            ("ctx_working", Value::Bool(true)),
            ("project", Value::String(project.to_owned())),
            ("session", Value::String(session.to_owned())),
        ]);
        self.store_fact(id, &content, &embedding, Some(&meta), None)?;
        Ok(id)
    }

    /// The working context previously saved under `project` + `session`,
    /// `None` when there is none.
    ///
    /// # Errors
    /// Returns [`MemoryError::WorkingContextCodec`] if the stored payload
    /// does not parse, or a storage error.
    pub fn load_working_context(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Option<WorkingContext>, MemoryError> {
        match self.store.get(working_id(project, session))? {
            Some((content, _)) => serde_json::from_str(&content)
                .map(Some)
                .map_err(|err| MemoryError::WorkingContextCodec(err.to_string())),
            None => Ok(None),
        }
    }
}

/// How many memories a scope pulls when it does not say (`k` absent).
const DEFAULT_MEMORY_K: usize = 5;

/// The recall filter a scope narrows to (its project facet), if any.
fn scope_filter(scope: &MemoryScope) -> Option<Metadata> {
    scope.project.as_ref().map(|project| {
        let mut meta = Map::new();
        meta.insert("project".to_owned(), Value::String(project.clone()));
        meta
    })
}

/// One memory the scope pulled in, with its full ranking ventilation.
struct PulledMemory {
    fragment: ContextFragment,
    memory_id: u64,
    /// Fused score normalised over the pulled batch, in `[0, 1]`.
    relevance: f32,
    /// Normalised vector term of the fused score.
    vector_norm: f64,
    /// Graph promotion weight of the fused score.
    graph_weight: f64,
}

/// Stamp pulled memories into the compiled provenance: their decisions and
/// sources gain the backing `memory_id`, the decision's relevance becomes
/// the normalised fused-ranking score, and the reason spells out the score
/// ventilation (vector vs graph) so `why this memory` is answerable from
/// the decision alone.
fn annotate_memory_provenance(out: &mut CompiledContext, pulled: &BTreeMap<u64, PulledMemory>) {
    for decision in &mut out.decisions {
        if let Some(memory) = pulled.get(&decision.content_hash) {
            decision.memory_id = Some(memory.memory_id);
            decision.relevance = memory.relevance;
            decision.reason = format!(
                "{} — pulled from memory {} (vector {:.2}, graph {:.2})",
                decision.reason, memory.memory_id, memory.vector_norm, memory.graph_weight
            );
        }
    }
    for source in &mut out.sources {
        if let Some(hash) = provenance::parse_handle(&source.handle) {
            if let Some(memory) = pulled.get(&hash) {
                source.memory_id = Some(memory.memory_id);
            }
        }
    }
}

/// Base metadata of every bridge-stored system fact: hub-marked (invisible
/// to normal recall) plus the given extra keys.
fn system_meta(extra: &[(&str, Value)]) -> Metadata {
    let mut meta = Map::new();
    meta.insert(HUB_FIELD.to_owned(), Value::Bool(true));
    for (key, value) in extra {
        meta.insert((*key).to_owned(), value.clone());
    }
    meta
}

/// The metadata of one compilation event — counts and identifiers only.
fn event_meta(request: &CompileRequest, out: &CompiledContext, nanos: u128) -> Metadata {
    let mut extra: Vec<(&str, Value)> = vec![
        (EVENT_FLAG, Value::Bool(true)),
        ("tokens_in", Value::Number(out.insights.tokens_in.into())),
        ("tokens_out", Value::Number(out.insights.tokens_out.into())),
        (
            "tokens_saved",
            Value::Number(out.insights.tokens_saved.into()),
        ),
        (
            "occurred_at",
            Value::Number(Number::from(
                u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX),
            )),
        ),
    ];
    if let Some(project) = &request.project {
        extra.push(("project", Value::String(project.clone())));
    }
    if let Some(model) = &request.target_model {
        extra.push(("model", Value::String(model.clone())));
    }
    if let (Some(micros), Some(currency)) = (
        out.insights.estimated_cost_saved_micros,
        out.insights.currency.as_ref(),
    ) {
        extra.push(("cost_saved_micros", Value::Number(micros.into())));
        extra.push(("currency", Value::String(currency.clone())));
    }
    system_meta(&extra)
}

/// Fold recalled event metadata into one [`ContextSavings`].
fn aggregate_events(hits: &[crate::model::Recollection]) -> ContextSavings {
    let mut savings = ContextSavings {
        events: hits.len() as u64,
        truncated: hits.len() >= crate::limits::MAX_RECALL_LIMIT,
        ..ContextSavings::default()
    };
    for hit in hits {
        let Some(meta) = &hit.metadata else { continue };
        savings.tokens_in = savings
            .tokens_in
            .saturating_add(meta_u64(meta, "tokens_in"));
        savings.tokens_out = savings
            .tokens_out
            .saturating_add(meta_u64(meta, "tokens_out"));
        savings.tokens_saved = savings
            .tokens_saved
            .saturating_add(meta_u64(meta, "tokens_saved"));
        if let (Some(Value::String(currency)), micros) =
            (meta.get("currency"), meta_u64(meta, "cost_saved_micros"))
        {
            if micros > 0 {
                *savings
                    .cost_saved_micros_by_currency
                    .entry(currency.clone())
                    .or_insert(0) += micros;
            }
        }
    }
    savings
}

/// A `u64` metadata field, `0` when absent or non-numeric.
fn meta_u64(meta: &Metadata, key: &str) -> u64 {
    meta.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// The salted system-fact id of a stored source.
fn source_id(content_hash: u64) -> u64 {
    stable_id(&format!("{SOURCE_ID_SALT}{content_hash}"))
}

/// The salted, deterministic system-fact id of a working context.
fn working_id(project: &str, session: &str) -> u64 {
    stable_id(&format!("{WORKING_ID_SALT}{project}\u{1f}{session}"))
}
