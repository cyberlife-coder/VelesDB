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
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock nanos since the Unix epoch, stamped on savings events only —
/// never in the compile pipeline. On `wasm32-unknown-unknown`
/// `SystemTime::now()` aborts (`std` has no clock there), so events carry 0:
/// the per-process sequence alone uniquifies their ids, and wasm stats are
/// per-session by design (in-memory store).
fn now_nanos() -> u128 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    }
}

/// Current Unix time in seconds — used only by
/// [`MemoryService::should_upgrade_ttl`]'s extension-only comparison (the
/// storage/expiry layer; the `compile` pipeline itself stays clock-free). On
/// `wasm32-unknown-unknown` this is 0 (no clock, mirrors [`now_nanos`]); the
/// wasm `MemoryStore` is in-memory only, so a stored durable expiry (a real
/// epoch second count) never actually exists there for 0 to be compared
/// against.
fn now_unix_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }
}

use serde_json::{Map, Number, Value};

use super::{positive_ttl, MemoryService, Metadata, HUB_FIELD};
use crate::context::model::{
    CompileRequest, CompiledContext, ContextFragment, ContextSavings, ContextSource,
    ImportanceWeights, MediaRef, MemoryScope, WorkingContext,
};
use crate::context::{media, provenance, ContextCompiler};
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
/// A stored source's media payload (US-009, PR2): `{"mime", "bytes_b64"}`,
/// the exact [`MediaRef`] shape, set only when the source fragment carried
/// one. Reserved like every other `_veles_ctx_*` key — a caller can neither
/// set nor filter on it.
const CTX_SOURCE_MEDIA_FIELD: &str = "_veles_ctx_source_media";
/// The durable-TTL payload key set by [`super::positive_ttl`]-backed writes
/// (`store_with_ttl`, via `store_fact`). Mirrors `velesdb_core::EXPIRES_AT_KEY`
/// as a literal rather than an import: that re-export is `persistence`-gated,
/// and this module (unlike `NativeStore`) must keep compiling under `context`
/// alone (e.g. `velesdb-wasm`, which never enables `persistence`).
const EXPIRES_AT_FIELD: &str = "_veles_expires_at";
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
        let importance = compiler.effective_policy(request).importance.clone();
        let memories = self.context_memories(request, &importance)?;
        self.compile_with_memories(compiler, request, memories)
    }

    /// [`Self::compile_context`] with a caller-supplied [`crate::Reranker`] driving
    /// memory selection: the reranker receives the FULL fused candidate pool
    /// (vector + graph, before the `k` cutoff) and its ordering decides
    /// which `k` memories are compiled in — the seam for a semantic
    /// cross-encoder or LLM judge a Rust embedder brings along. Not exposed
    /// on the wire (a reranker is code, not JSON), and never a default: the
    /// shipped [`crate::context::DeterministicReranker`] is *lexical*, and a
    /// lexical second stage demotes exactly the zero-vocabulary-overlap
    /// evidence the graph walk rescues (measured in the BDD suite) — bring
    /// a semantic one.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if compilation, recall, the reranker itself,
    /// or storage fails.
    pub fn compile_context_reranked<R: crate::Reranker>(
        &self,
        compiler: &ContextCompiler,
        request: &CompileRequest,
        reranker: &R,
    ) -> Result<CompiledContext, MemoryError> {
        let importance = compiler.effective_policy(request).importance.clone();
        let memories = self.context_memories_reranked(request, reranker, &importance)?;
        self.compile_with_memories(compiler, request, memories)
    }

    /// The shared back half of every compile flavour: augment the request
    /// with the pulled memories, compile, annotate provenance, persist
    /// sources/events per policy.
    fn compile_with_memories(
        &self,
        compiler: &ContextCompiler,
        request: &CompileRequest,
        memories: Vec<PulledMemory>,
    ) -> Result<CompiledContext, MemoryError> {
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
    /// their id and normalised fused relevance, importance-blended
    /// ([`Self::blend_importance`]) when the policy's weights are active.
    fn context_memories(
        &self,
        request: &CompileRequest,
        importance: &ImportanceWeights,
    ) -> Result<Vec<PulledMemory>, MemoryError> {
        let Some((scope, k)) = scope_and_k(request) else {
            return Ok(Vec::new());
        };
        let filter = scope_filter(scope);
        // The scope's fusion knobs (clamped by from_knobs); absent ones fall
        // back to the crate defaults — raising graph_boost lets a curated
        // relate-chain out-rank lexically-noisy near-misses (see MemoryScope).
        let opts = FusionOptions::from_knobs(scope.hops, scope.graph_boost, None);
        let scored = self.recall_fused_scored(&request.query, k, filter.as_ref(), opts)?;
        let max_fused = scored
            .iter()
            .map(|s| s.fused)
            .fold(f64::MIN, f64::max)
            .max(f64::EPSILON);
        let candidates = scored
            .into_iter()
            .map(|scored| {
                // Sanitise a non-finite fused score to 0 before normalising:
                // `f32::clamp` returns NaN for a NaN input (it does not clamp),
                // which would put a non-`[0, 1]` value — serialising as JSON
                // `null` — into an output sold as deterministic and auditable.
                let fused = if scored.fused.is_finite() {
                    scored.fused
                } else {
                    0.0
                };
                MemoryCandidate {
                    memory_id: scored.recollection.id,
                    base: (fused / max_fused).clamp(0.0, 1.0),
                    vector_norm: scored.vector_norm,
                    graph_weight: scored.graph_weight,
                    metadata: scored.recollection.metadata,
                    content: scored.recollection.content,
                }
            })
            .collect();
        self.blend_importance(candidates, importance)
    }

    /// Memory selection driven by a caller-supplied reranker: the fused
    /// candidate pool (at pool depth, vector + graph) is handed to the
    /// reranker whole, its ordering is truncated to `k`, and relevance is
    /// rank-based (the reranker defines the ranking; the fused ventilation
    /// no longer describes it, so vector/graph read 0 in provenance). The
    /// importance blend then composes with the seam: it re-ranks INSIDE the
    /// reranker-selected pool, exactly as it does over the fused pool.
    fn context_memories_reranked<R: crate::Reranker>(
        &self,
        request: &CompileRequest,
        reranker: &R,
        importance: &ImportanceWeights,
    ) -> Result<Vec<PulledMemory>, MemoryError> {
        let Some((scope, k)) = scope_and_k(request) else {
            return Ok(Vec::new());
        };
        let filter = scope_filter(scope);
        let opts = FusionOptions::from_knobs(scope.hops, scope.graph_boost, None);
        let ranked =
            self.recall_fused_reranked(&request.query, k, filter.as_ref(), opts, reranker)?;
        let count = ranked.len().max(1);
        let candidates = ranked
            .into_iter()
            .enumerate()
            .map(|(rank, recollection)| {
                // Computed in f32 exactly as 0.8.0 did, so inactive weights
                // reproduce the historical relevance bytes.
                #[allow(clippy::cast_precision_loss)] // rank/count are tiny
                let relevance = 1.0 - (rank as f32 / count as f32);
                MemoryCandidate {
                    memory_id: recollection.id,
                    base: f64::from(relevance),
                    vector_norm: 0.0,
                    graph_weight: 0.0,
                    metadata: recollection.metadata,
                    content: recollection.content,
                }
            })
            .collect();
        self.blend_importance(candidates, importance)
    }

    /// Fold usage-driven importance into an already-selected memory pool —
    /// the one ranking the whole engine stack shares (US-002 of EPIC-P-071):
    /// per candidate the key becomes `base + w_c·(confidence − 0.5)·2 +
    /// w_r·recency_norm`, where `base` is the fused (or rank-based)
    /// similarity in `[0, 1]`. Selection is untouched on purpose: confidence
    /// is not relevance, so a reinforced-but-off-topic fact can never buy
    /// its way into the pool here. Inactive weights take the zero-cost path
    /// and reproduce the 0.8.0 output byte for byte (golden-pinned). The
    /// stable sort keeps equal keys in selection order, and no clock is ever
    /// read — recency is min-max normalised within the batch.
    fn blend_importance(
        &self,
        candidates: Vec<MemoryCandidate>,
        weights: &ImportanceWeights,
    ) -> Result<Vec<PulledMemory>, MemoryError> {
        if !importance_active(weights) {
            return Ok(candidates
                .into_iter()
                .map(MemoryCandidate::into_pulled)
                .collect());
        }
        let ids: Vec<u64> = candidates.iter().map(|c| c.memory_id).collect();
        // Raw payloads (reserved keys included): the learned confidence
        // lives under `_veles_rl_confidence`, which caller-facing metadata
        // strips.
        let raw = self.store.get_metadata_batch(&ids)?;
        let recencies = recency_norms(&candidates, weights);
        let mut blended: Vec<(f64, PulledMemory)> = candidates
            .into_iter()
            .zip(raw)
            .zip(recencies)
            .map(|((candidate, payload), recency)| {
                let confidence = payload_confidence(payload.as_ref());
                let score = candidate.base
                    + weights.confidence * (confidence - NEUTRAL_CONFIDENCE) * 2.0
                    + weights.recency * recency;
                let mut pulled = candidate.into_pulled();
                #[allow(clippy::cast_possible_truncation)] // clamped into [0, 1]
                {
                    pulled.relevance = score.clamp(0.0, 1.0) as f32;
                }
                pulled.confidence = confidence;
                pulled.recency = recency;
                pulled.ventilated = true;
                (score, pulled)
            })
            .collect();
        // Stable: equal blended keys keep the selection order.
        blended.sort_by(|a, b| b.0.total_cmp(&a.0));
        Ok(blended.into_iter().map(|(_, pulled)| pulled).collect())
    }

    /// Store every distinct fragment's original as a hub-marked system fact
    /// keyed by its salted handle hash, so its handle can be resolved later.
    /// A fragment carrying media (US-009, PR2) has its base64 payload
    /// persisted alongside the caption under the reserved
    /// [`CTX_SOURCE_MEDIA_FIELD`] key.
    ///
    /// **Identity**: the key mirrors what the compiler mints handles from
    /// (`Analysis::handle_hash` in `context.rs`) — the caption's
    /// [`stable_id`] for text, the raw decoded bytes' hash
    /// ([`media::MediaAnalysis::raw_hash`]) for media, the same identity
    /// PR1's dedup keys on. Keying media on the caption instead was the PR2
    /// review's proven blocker: every captionless image collided onto one
    /// slot and one handle, serving arbitrary wrong bytes back. The slot
    /// stays inside the salted system-fact namespace ([`source_id`] applies
    /// `SOURCE_ID_SALT` to the hash) — same salt, no new namespace. On a
    /// same-key collision (byte-identical images with different captions)
    /// the FIRST occurrence wins, matching the dedup twin the compiler
    /// keeps — a divergent duplicate caption does not survive, exactly as
    /// its decision reason already says.
    ///
    /// Size: [`crate::limits::MAX_MEDIA_BYTES`] /
    /// [`crate::limits::MAX_TOTAL_MEDIA_BYTES`] already bounded every
    /// fragment's `bytes_b64` before `compiler.compile` ever ran (see
    /// `validate_media`, called from `compile`'s `validate`) — `augmented`
    /// here is exactly the request that passed that check, so no separate
    /// size guard is needed on the write path itself
    /// ([`crate::limits::MAX_FACT_BYTES`] governs the unrelated MCP
    /// `remember`/`extract` text ceiling, not this one).
    fn store_context_sources(
        &self,
        augmented: &CompileRequest,
        out: &CompiledContext,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let mut by_hash: BTreeMap<u64, &ContextFragment> = BTreeMap::new();
        for fragment in &augmented.fragments {
            // First occurrence wins (see the identity note above): `entry`
            // + `or_insert`, never a blind overwrite.
            by_hash
                .entry(fragment_handle_hash(fragment))
                .or_insert(fragment);
        }
        let ttl_seconds = positive_ttl(ttl_seconds);
        for source in &out.sources {
            let Some(hash) = provenance::parse_handle(&source.handle) else {
                continue;
            };
            let Some(fragment) = by_hash.get(&hash) else {
                continue;
            };
            let slot = source_id(hash);
            // A slot never marked as ours is never rewritten: it is a caller
            // fact squatting the salt preimage, and clobbering it would
            // destroy user data. A slot already marked as ours holds these
            // exact bytes — sources are content-addressed — so content and
            // embedding never change; only durability can, and only upward
            // (never-downgrade TTL upgrade, `should_store_source`) so a
            // handle sold as permanent never silently expires just because
            // an earlier compile first wrote it under a TTL.
            if !self.should_store_source(slot, ttl_seconds)? {
                continue;
            }
            // Upgrading to permanent needs the old point gone, not merely
            // overwritten: velesdb-core's store path preserves every
            // `_veles_*` key from a prior version of a re-stored id unless
            // the new write explicitly sets it (semantic_memory.rs's
            // `store_internal` carry-forward, so plain `remember` doesn't
            // silently wipe learned state), and a permanent write has no
            // expiry to explicitly set (`attach_expiry` is a no-op without
            // one) — so without this delete, `_veles_expires_at` would
            // survive the "upgrade" untouched. A TTL-to-TTL extension needs
            // no delete: its new expiry always overwrites the old one.
            if ttl_seconds.is_none() && self.store.get(slot)?.is_some() {
                self.store.delete(slot)?;
            }
            let content = fragment.content.as_str();
            let mut extra: Vec<(&str, Value)> = vec![(CTX_SOURCE_FIELD, Value::Bool(true))];
            let embedding = if let Some(media_ref) = &fragment.media {
                extra.push((
                    CTX_SOURCE_MEDIA_FIELD,
                    serde_json::to_value(media_ref).unwrap_or(Value::Null),
                ));
                // Deterministic, derived from the DECODED bytes — never the
                // text embedder over `content` (often blank) or over the
                // base64 payload itself (opaque, not language). Correct
                // because `retrieve_context_source` resolves a media source
                // EXCLUSIVELY by its content-addressed hash/slot, never by
                // vector search — this vector only needs to be well-formed
                // and non-degenerate for the underlying index, never
                // semantically meaningful. For a media fragment `hash` IS
                // the raw-bytes hash (see `fragment_handle_hash`), so no
                // re-decode is needed here.
                self.media_placeholder_embedding(hash)
            } else {
                self.embedder.embed(content)?
            };
            self.store_fact(
                slot,
                content,
                &embedding,
                Some(&system_meta(&extra)),
                ttl_seconds,
            )?;
        }
        Ok(())
    }

    /// Whether [`Self::store_context_sources`] should (re-)write `slot` for
    /// this compile's requested (already [`positive_ttl`]-normalized —
    /// `None` means permanent) TTL.
    ///
    /// - Not marked as ours (absent, or a caller fact squatting the salt
    ///   preimage): store only if the slot is genuinely empty.
    /// - Marked as ours: never re-embed or change content (content-addressed);
    ///   only [`Self::should_upgrade_ttl`] decides whether durability changes.
    fn should_store_source(
        &self,
        slot: u64,
        requested_ttl: Option<u64>,
    ) -> Result<bool, MemoryError> {
        match self.context_source_metadata(slot)? {
            Some(existing) => Ok(Self::should_upgrade_ttl(&existing, requested_ttl)),
            None => Ok(self.store.get(slot)?.is_none()),
        }
    }

    /// Never-downgrade TTL upgrade rule for an already-stored source: permanent
    /// once requested stays permanent, and a TTL only ever extends, never
    /// shortens. The clock read here is fine — this is the storage/expiry
    /// layer, not the clock-free `compile` pipeline.
    fn should_upgrade_ttl(existing: &Metadata, requested_ttl: Option<u64>) -> bool {
        let existing_expiry = existing.get(EXPIRES_AT_FIELD).and_then(Value::as_u64);
        match (requested_ttl, existing_expiry) {
            // Permanent requested, slot still carries a TTL: upgrade.
            (None, Some(_)) => true,
            // Already permanent, or a TTL requested against a permanent slot:
            // never downgrade.
            (None | Some(_), None) => false,
            // Both carry a TTL: extend only if the new one outlives what
            // remains — never shorten.
            (Some(ttl), Some(existing_exp)) => now_unix_secs().saturating_add(ttl) > existing_exp,
        }
    }

    /// A deterministic, non-degenerate embedding for a media source (US-009,
    /// PR2) — see [`Self::store_context_sources`] for why it is bytes-hash
    /// derived rather than text-embedded.
    fn media_placeholder_embedding(&self, raw_hash: u64) -> Vec<f32> {
        let dim = self.embedder.dimension();
        let mut vector = vec![0.0_f32; dim];
        let Ok(dim_u64) = u64::try_from(dim) else {
            return vector;
        };
        if dim_u64 == 0 {
            return vector;
        }
        let bucket = usize::try_from(raw_hash % dim_u64).unwrap_or(0);
        vector[bucket] = 1.0;
        velesdb_core::simd_native::normalize_inplace_native(&mut vector);
        vector
    }

    /// The fact at `slot`'s metadata, when it carries the stored-source
    /// marker (`None` otherwise — absent, or a caller fact squatting the
    /// slot).
    fn context_source_metadata(&self, slot: u64) -> Result<Option<Metadata>, MemoryError> {
        let payloads = self.store.get_metadata_batch(&[slot])?;
        Ok(payloads
            .into_iter()
            .next()
            .flatten()
            .filter(|meta| meta.get(CTX_SOURCE_FIELD) == Some(&Value::Bool(true))))
    }

    /// The original content — and media, when the fragment carried one —
    /// behind a `ctx://source/<hash>` handle.
    ///
    /// # Errors
    /// Returns [`MemoryError::UnknownHandle`] when the handle is malformed
    /// or nothing is stored under it (never stored, expired, or forgotten).
    pub fn retrieve_context_source(&self, handle: &str) -> Result<ContextSource, MemoryError> {
        let unknown = || MemoryError::UnknownHandle(handle.to_owned());
        let hash = provenance::parse_handle(handle).ok_or_else(unknown)?;
        let slot = source_id(hash);
        // Only marker-bearing facts are sources: a caller fact squatting the
        // salted slot is never served back as compiled provenance.
        let meta = self.context_source_metadata(slot)?.ok_or_else(unknown)?;
        let content = self
            .store
            .get(slot)?
            .map(|(content, _embedding)| content)
            .ok_or_else(unknown)?;
        Ok(ContextSource {
            content,
            media: source_media(&meta),
        })
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
        let occurred_at_nanos = now_nanos();
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

/// The request's memory scope plus the clamped pull count — `None` when
/// there is no scope or no room: pulled memories must never push the
/// request over the fragment cap (the cap is validated after augmentation,
/// and a rejection there would blame the caller for fragments the bridge
/// itself added).
fn scope_and_k(request: &CompileRequest) -> Option<(&MemoryScope, usize)> {
    let scope = request.memory_scope.as_ref()?;
    let room = crate::limits::MAX_FRAGMENTS.saturating_sub(request.fragments.len());
    let k = crate::limits::clamp_recall_limit(scope.k.unwrap_or(DEFAULT_MEMORY_K)).min(room);
    (k > 0).then_some((scope, k))
}

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
    /// Fused score normalised over the pulled batch, in `[0, 1]` — the
    /// importance-blended key (clamped) when the blend is active.
    relevance: f32,
    /// Normalised vector term of the fused score.
    vector_norm: f64,
    /// Graph promotion weight of the fused score.
    graph_weight: f64,
    /// Learned RL confidence the blend used (neutral `0.5` when the memory
    /// never received feedback).
    confidence: f64,
    /// Batch-relative recency contribution in `[0, 1]` (`0` when the term
    /// is inactive, the key is absent, or the batch is degenerate).
    recency: f64,
    /// Whether the importance blend ran — drives the extended four-signal
    /// reason ventilation; `false` keeps the exact 0.8.0 reason bytes.
    ventilated: bool,
}

/// A selected memory before the importance blend: its similarity base, its
/// fused ventilation, and the caller-visible metadata the recency term reads.
struct MemoryCandidate {
    memory_id: u64,
    /// Fused-normalised (or rank-based) similarity in `[0, 1]`.
    base: f64,
    vector_norm: f64,
    graph_weight: f64,
    metadata: Option<Metadata>,
    content: String,
}

impl MemoryCandidate {
    /// The unblended [`PulledMemory`] — bytes identical to the 0.8.0 pull.
    fn into_pulled(self) -> PulledMemory {
        #[allow(clippy::cast_possible_truncation)] // base is clamped into [0, 1]
        let relevance = self.base as f32;
        PulledMemory {
            fragment: ContextFragment {
                id: None,
                content: self.content,
                kind: Some("memory".to_owned()),
                priority: None,
                metadata: None,
                media: None,
            },
            memory_id: self.memory_id,
            relevance,
            vector_norm: self.vector_norm,
            graph_weight: self.graph_weight,
            confidence: NEUTRAL_CONFIDENCE,
            recency: 0.0,
            ventilated: false,
        }
    }
}

/// The neutral confidence of a memory with no feedback history — mirrors
/// `reinforce::RL_NEUTRAL_CONFIDENCE`, whose module is `persistence`-gated:
/// its contribution to the blend is exactly `0`.
const NEUTRAL_CONFIDENCE: f64 = 0.5;

/// The learned RL confidence off a raw payload, in `[0, 1]`. Without the
/// `persistence` feature the RL module (and thus `feedback`) does not exist,
/// so every memory reads neutral.
#[cfg(feature = "persistence")]
fn payload_confidence(payload: Option<&Metadata>) -> f64 {
    f64::from(payload.map_or(
        super::reinforce::RL_NEUTRAL_CONFIDENCE,
        super::reinforce::read_confidence,
    ))
}

/// See the `persistence` twin: no RL module, always neutral.
#[cfg(not(feature = "persistence"))]
fn payload_confidence(_payload: Option<&Metadata>) -> f64 {
    NEUTRAL_CONFIDENCE
}

/// Whether the policy's importance weights change anything at all: a
/// non-zero confidence weight, or a non-zero recency weight WITH a field to
/// read. Zero weights must cost nothing and change nothing (0.8.0 parity).
#[allow(
    clippy::float_cmp,
    reason = "an exact zero weight is the documented off switch; any non-zero weight, however small, is active"
)]
fn importance_active(weights: &ImportanceWeights) -> bool {
    weights.confidence != 0.0 || (weights.recency != 0.0 && weights.recency_field.is_some())
}

/// The batch-relative recency contribution of every candidate, in `[0, 1]`:
/// min-max over the candidates that carry the policy's `recency_field` as a
/// number (one monotone scale per batch — `YYYYMMDD` or an epoch, the
/// caller's choice). A candidate without the key contributes `0` (never
/// penalised), and a degenerate batch (`max == min`) contributes `0` for
/// all. No clock: recency is relative to the newest of the batch.
#[allow(
    clippy::float_cmp,
    reason = "an exact zero weight is the documented off switch for the recency term"
)]
fn recency_norms(candidates: &[MemoryCandidate], weights: &ImportanceWeights) -> Vec<f64> {
    let field = weights
        .recency_field
        .as_ref()
        .filter(|_| weights.recency != 0.0);
    let Some(field) = field else {
        return vec![0.0; candidates.len()];
    };
    let values: Vec<Option<f64>> = candidates
        .iter()
        .map(|candidate| {
            candidate
                .metadata
                .as_ref()
                .and_then(|meta| meta.get(field.as_str()))
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
        })
        .collect();
    let (min, max) = values
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        });
    if max <= min {
        return vec![0.0; candidates.len()];
    }
    values
        .into_iter()
        .map(|value| value.map_or(0.0, |v| ((v - min) / (max - min)).clamp(0.0, 1.0)))
        .collect()
}

/// Stamp pulled memories into the compiled provenance: their decisions and
/// sources gain the backing `memory_id`, the decision's relevance becomes
/// the normalised (importance-blended, when active) ranking score, and the
/// reason spells out the full score ventilation — vector and graph always,
/// plus confidence and recency when the blend ran — so `why this memory` is
/// answerable from the decision alone.
fn annotate_memory_provenance(out: &mut CompiledContext, pulled: &BTreeMap<u64, PulledMemory>) {
    for decision in &mut out.decisions {
        if let Some(memory) = pulled.get(&decision.content_hash) {
            decision.memory_id = Some(memory.memory_id);
            decision.relevance = memory.relevance;
            decision.reason = if memory.ventilated {
                format!(
                    "{} — pulled from memory {} (vector {:.2}, graph {:.2}, confidence {:.2}, recency {:.2})",
                    decision.reason,
                    memory.memory_id,
                    memory.vector_norm,
                    memory.graph_weight,
                    memory.confidence,
                    memory.recency
                )
            } else {
                format!(
                    "{} — pulled from memory {} (vector {:.2}, graph {:.2})",
                    decision.reason, memory.memory_id, memory.vector_norm, memory.graph_weight
                )
            };
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

/// The handle-identity hash of one request fragment — the bridge-side twin
/// of `Analysis::handle_hash` in `context.rs` (kept in lockstep; the two
/// must key the same identity or stored slots and minted handles drift
/// apart): raw decoded media bytes for a media fragment, caption/content
/// [`stable_id`] otherwise.
fn fragment_handle_hash(fragment: &ContextFragment) -> u64 {
    fragment.media.as_ref().map_or_else(
        || stable_id(&fragment.content),
        |media_ref| media::analyze(media_ref).raw_hash,
    )
}

/// A stored source's media payload (US-009, PR2), when its metadata carries
/// one — absent (or malformed, which should never happen for a payload this
/// bridge wrote itself) round-trips as `None` rather than an error, so a
/// media decode hiccup degrades to "text-only", never breaks the whole
/// retrieval.
fn source_media(meta: &Metadata) -> Option<MediaRef> {
    meta.get(CTX_SOURCE_MEDIA_FIELD)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

/// The salted, deterministic system-fact id of a working context.
fn working_id(project: &str, session: &str) -> u64 {
    stable_id(&format!("{WORKING_ID_SALT}{project}\u{1f}{session}"))
}

#[cfg(all(test, feature = "persistence"))]
#[path = "memory_bridge_tests.rs"]
mod tests;
