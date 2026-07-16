//! The context compiler's memory bridge: memory-backed fragment selection,
//! recoverable sources, aggregatable compilation events, and persisted
//! working contexts — the `MemoryService` half of EPIC-P-070's US-002.
//!
//! Everything the bridge persists is a **system fact**: hub-marked
//! (`_veles_hub`) and carrying **only reserved `_veles_*` metadata keys**, so
//! it is invisible to unfiltered recall (hub exclusion), can never match a
//! caller's include filter (callers cannot name reserved keys), and can never
//! be forged by a caller fact (reserved keys are rejected at `remember`).
//! Stored ids are salted, and both the source writer and the handle resolver
//! verify the `_veles_ctx_source` marker, so a caller fact squatting a salt
//! preimage is neither overwritten nor ever served back as a source. Events
//! carry metadata and hashes only — never fragment content. Event recording
//! stamps wall-clock time; the compile pipeline itself stays clock-free and
//! deterministic.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Number, Value};

use super::{positive_ttl, MemoryService, Metadata, HUB_FIELD};
use crate::context::model::{
    CompileRequest, CompiledContext, ContextFragment, ContextSavings, MemoryScope, WorkingContext,
};
use crate::context::{provenance, ContextCompiler};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::id::stable_id;
use crate::model::FusionOptions;
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

/// Reserved metadata keys of the bridge's system facts. Reserved (`_veles_`)
/// on purpose: callers can neither set them (forgery) nor filter on them, so
/// system facts are invisible to every caller-facing recall path and
/// [`MemoryService::context_savings`] aggregates only genuine events (it
/// filters at the storage layer, below the caller-facing validation).
const CTX_EVENT_FIELD: &str = "_veles_ctx_event";
const CTX_PROJECT_FIELD: &str = "_veles_ctx_project";
const CTX_MODEL_FIELD: &str = "_veles_ctx_model";
const CTX_SOURCE_FIELD: &str = "_veles_ctx_source";
const CTX_WORKING_FIELD: &str = "_veles_ctx_working";
const CTX_SESSION_FIELD: &str = "_veles_ctx_session";
const CTX_TOKENS_IN_FIELD: &str = "_veles_ctx_tokens_in";
const CTX_TOKENS_OUT_FIELD: &str = "_veles_ctx_tokens_out";
const CTX_TOKENS_SAVED_FIELD: &str = "_veles_ctx_tokens_saved";
const CTX_COST_FIELD: &str = "_veles_ctx_cost_micros";
const CTX_CURRENCY_FIELD: &str = "_veles_ctx_currency";
const CTX_AT_FIELD: &str = "_veles_ctx_at";

/// Per-process sequence folded into event ids so two compilations landing on
/// the same clock tick (coarse timers, concurrent calls) never collide.
static EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

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
        // Never let pulled memories push the request over the fragment cap —
        // the cap is validated after augmentation, and a rejection there would
        // blame the caller for fragments the bridge itself added.
        let room = crate::limits::MAX_FRAGMENTS.saturating_sub(request.fragments.len());
        let k = crate::limits::clamp_recall_limit(scope.k.unwrap_or(DEFAULT_MEMORY_K)).min(room);
        if k == 0 {
            return Ok(Vec::new());
        }
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
        let ttl_seconds = positive_ttl(ttl_seconds);
        for source in &out.sources {
            let Some(hash) = provenance::parse_handle(&source.handle) else {
                continue;
            };
            let Some(content) = by_hash.get(&hash) else {
                continue;
            };
            let slot = source_id(hash);
            // Never clobber a caller fact that happens to sit at this salted
            // id (someone remembered the literal salt preimage): the slot is
            // ours only when empty or already marker-bearing.
            if self.store.get(slot)?.is_some() && !self.slot_is_context_source(slot)? {
                continue;
            }
            let embedding = self.embedder.embed(content)?;
            self.store_fact(
                slot,
                content,
                &embedding,
                Some(&system_meta(&[(CTX_SOURCE_FIELD, Value::Bool(true))])),
                ttl_seconds,
            )?;
        }
        Ok(())
    }

    /// Whether the fact at `slot` carries the stored-source marker.
    fn slot_is_context_source(&self, slot: u64) -> Result<bool, MemoryError> {
        let payloads = self.store.get_metadata_batch(&[slot])?;
        Ok(payloads.first().is_some_and(|payload| {
            payload
                .as_ref()
                .is_some_and(|meta| meta.get(CTX_SOURCE_FIELD) == Some(&Value::Bool(true)))
        }))
    }

    /// The original content behind a `ctx://source/<hash>` handle.
    ///
    /// # Errors
    /// Returns [`MemoryError::UnknownHandle`] when the handle is malformed
    /// or nothing is stored under it (never stored, expired, or forgotten).
    pub fn retrieve_context_source(&self, handle: &str) -> Result<String, MemoryError> {
        let unknown = || MemoryError::UnknownHandle(handle.to_owned());
        let hash = provenance::parse_handle(handle).ok_or_else(unknown)?;
        let slot = source_id(hash);
        // Only marker-bearing facts are sources: a caller fact squatting the
        // salted slot is never served back as compiled provenance.
        if !self.slot_is_context_source(slot)? {
            return Err(unknown());
        }
        self.store
            .get(slot)?
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
        // The per-process sequence keeps ids unique even when two compiles
        // land on the same (possibly coarse) clock tick.
        let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
        let content = format!("{EVENT_ANCHOR} {occurred_at_nanos}-{seq}");
        let id = stable_id(&format!("{EVENT_ID_SALT}{occurred_at_nanos}:{seq}"));
        let embedding = self.embedder.embed(&content)?;
        let meta = event_meta(request, out, occurred_at_nanos);
        self.store_fact(
            id,
            &content,
            &embedding,
            Some(&meta),
            positive_ttl(ttl_seconds),
        )?;
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
        // Filter at the STORAGE layer on the reserved event marker: callers
        // can neither set nor query `_veles_*` keys, so only genuine bridge
        // events can ever match — a caller fact posing as an event counts
        // for nothing.
        let mut filter = Map::new();
        filter.insert(CTX_EVENT_FIELD.to_owned(), Value::Bool(true));
        if let Some(project) = project {
            filter.insert(
                CTX_PROJECT_FIELD.to_owned(),
                Value::String(project.to_owned()),
            );
        }
        let embedding = self.embedder.embed(EVENT_ANCHOR)?;
        let hits =
            self.store
                .query_filtered(&embedding, crate::limits::MAX_RECALL_LIMIT, &filter, 0)?;
        let ids: Vec<u64> = hits.iter().map(|(id, _, _)| *id).collect();
        let payloads = self.store.get_metadata_batch(&ids)?;
        Ok(aggregate_events(&payloads))
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
            (CTX_WORKING_FIELD, Value::Bool(true)),
            (CTX_PROJECT_FIELD, Value::String(project.to_owned())),
            (CTX_SESSION_FIELD, Value::String(session.to_owned())),
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

/// The metadata of one compilation event — counts and identifiers only,
/// every key reserved.
fn event_meta(request: &CompileRequest, out: &CompiledContext, nanos: u128) -> Metadata {
    let mut extra: Vec<(&str, Value)> = vec![
        (CTX_EVENT_FIELD, Value::Bool(true)),
        (
            CTX_TOKENS_IN_FIELD,
            Value::Number(out.insights.tokens_in.into()),
        ),
        (
            CTX_TOKENS_OUT_FIELD,
            Value::Number(out.insights.tokens_out.into()),
        ),
        (
            CTX_TOKENS_SAVED_FIELD,
            Value::Number(out.insights.tokens_saved.into()),
        ),
        (
            CTX_AT_FIELD,
            Value::Number(Number::from(
                u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX),
            )),
        ),
    ];
    if let Some(project) = &request.project {
        extra.push((CTX_PROJECT_FIELD, Value::String(project.clone())));
    }
    if let Some(model) = &request.target_model {
        extra.push((CTX_MODEL_FIELD, Value::String(model.clone())));
    }
    if let (Some(micros), Some(currency)) = (
        out.insights.estimated_cost_saved_micros,
        out.insights.currency.as_ref(),
    ) {
        extra.push((CTX_COST_FIELD, Value::Number(micros.into())));
        extra.push((CTX_CURRENCY_FIELD, Value::String(currency.clone())));
    }
    system_meta(&extra)
}

/// Fold raw event payloads (reserved keys included) into one
/// [`ContextSavings`]. Every accumulation saturates — an aggregate must
/// never panic, whatever the stored numbers.
fn aggregate_events(payloads: &[Option<Metadata>]) -> ContextSavings {
    let mut savings = ContextSavings {
        events: payloads.len() as u64,
        truncated: payloads.len() >= crate::limits::MAX_RECALL_LIMIT,
        ..ContextSavings::default()
    };
    for payload in payloads {
        let Some(meta) = payload else { continue };
        savings.tokens_in = savings
            .tokens_in
            .saturating_add(meta_u64(meta, CTX_TOKENS_IN_FIELD));
        savings.tokens_out = savings
            .tokens_out
            .saturating_add(meta_u64(meta, CTX_TOKENS_OUT_FIELD));
        savings.tokens_saved = savings
            .tokens_saved
            .saturating_add(meta_u64(meta, CTX_TOKENS_SAVED_FIELD));
        if let (Some(Value::String(currency)), micros) =
            (meta.get(CTX_CURRENCY_FIELD), meta_u64(meta, CTX_COST_FIELD))
        {
            if micros > 0 {
                let entry = savings
                    .cost_saved_micros_by_currency
                    .entry(currency.clone())
                    .or_insert(0);
                *entry = entry.saturating_add(micros);
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
