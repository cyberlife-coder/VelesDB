//! Context COMPILATION — [`MemoryService::compile_context`] and its scoring,
//! source-persistence and explanation helpers — split out of
//! `memory_bridge.rs` to keep that file inside the crate's file budget, the
//! same child-module pattern as `service.rs`'s `fused_recall.rs`. The
//! working-context save/load/index half stays in `memory_bridge.rs`; this
//! half is the one whose methods carry `RecallStore`/`GraphStore` bounds
//! (#1959), so the budget seam and the facet seam coincide.

use super::{
    aggregate_events, annotate_memory_provenance, event_meta, importance_active,
    index_fragments_by_handle_hash, now_nanos, now_unix_secs, payload_confidence, positive_ttl,
    provenance, recency_norms, scope_and_k, scope_filter, source_id, source_media, stable_id,
    system_meta, BTreeMap, CompilePolicy, CompileRequest, CompiledContext, ContextCompiler,
    ContextDecision, ContextFragment, ContextSavings, ContextSource, Embedder, FactStore,
    FusionOptions, GraphStore, ImportanceWeights, Map, MemoryCandidate, MemoryError, MemoryService,
    Metadata, Ordering, PulledMemory, RecallStore, Value, CTX_EVENT_FIELD, CTX_PROJECT_FIELD,
    CTX_SOURCE_FIELD, CTX_SOURCE_MEDIA_FIELD, EVENT_ANCHOR, EVENT_ID_SALT, EVENT_SEQ,
    EXPIRES_AT_FIELD, NEUTRAL_CONFIDENCE,
};
use crate::service::embeddable_prefix;

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
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
    ) -> Result<CompiledContext, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let _generation = self.enter_generation();
        self.compile_context_inner(compiler, request)
    }

    fn compile_context_inner(
        &self,
        compiler: &ContextCompiler,
        request: &CompileRequest,
    ) -> Result<CompiledContext, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
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
    ) -> Result<CompiledContext, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let _generation = self.enter_generation();
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
        // `compile_raw`, not `compile`: annotating memory provenance below
        // can rewrite a pulled fragment's `relevance`/`reason` (and thus
        // whether it crosses the `warnings` threshold), so `decisions` must
        // stay full until that has happened and `warnings` is recomputed —
        // `slim_response` (if requested) is applied as the LAST step.
        let mut out = compiler.compile_raw(&augmented)?;
        annotate_memory_provenance(&mut out, &pulled);
        out.warnings = crate::context::warnings_for(&out.decisions);
        let policy = compiler.effective_policy(request);
        if policy.store_sources {
            self.store_context_sources(&augmented, &out, policy.source_ttl_seconds)?;
        }
        if policy.record_events {
            self.record_context_event(request, &out, policy.event_ttl_seconds)?;
        }
        Ok(crate::context::apply_slim(out, policy))
    }

    /// The memories a request's scope pulls in, as compile fragments plus
    /// their id and normalised fused relevance, importance-blended
    /// ([`Self::blend_importance`]) when the policy's weights are active.
    fn context_memories(
        &self,
        request: &CompileRequest,
        importance: &ImportanceWeights,
    ) -> Result<Vec<PulledMemory>, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
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
    ) -> Result<Vec<PulledMemory>, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let Some((scope, k)) = scope_and_k(request) else {
            return Ok(Vec::new());
        };
        let filter = scope_filter(scope);
        let opts = FusionOptions::from_knobs(scope.hops, scope.graph_boost, None);
        let ranked =
            self.recall_fused_reranked_inner(&request.query, k, filter.as_ref(), opts, reranker)?;
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
    /// `validate_media`, called from `compile`'s `validate`). TEXT is a
    /// different story, and an earlier revision of this comment got it
    /// wrong by claiming no size guard was needed on the write path: those
    /// media caps say nothing about `content`, which a `path` ingestion can
    /// fill up to 1 MiB — so [`Self::source_vector`] caps what it EMBEDS
    /// (the stored content stays whole). The lesson stands: "another layer
    /// already checked" must name which cap, over which field.
    fn store_context_sources(
        &self,
        augmented: &CompileRequest,
        out: &CompiledContext,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let by_hash = index_fragments_by_handle_hash(&augmented.fragments);
        let ttl_seconds = positive_ttl(ttl_seconds);
        for source in &out.sources {
            self.store_one_source(&source.handle, &by_hash, ttl_seconds)?;
        }
        Ok(())
    }

    /// Write the one slot behind `handle`, if this compile owns it.
    ///
    /// A handle whose fragment is no longer in the request (or that does not
    /// parse) is skipped, not an error: `out.sources` is derived from the
    /// same request, so a miss can only mean the source was externalized
    /// under a shape this write path has nothing to store.
    fn store_one_source(
        &self,
        handle: &str,
        by_hash: &BTreeMap<u64, &ContextFragment>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let Some(hash) = provenance::parse_handle(handle) else {
            return Ok(());
        };
        let Some(fragment) = by_hash.get(&hash) else {
            return Ok(());
        };
        let slot = source_id(hash);
        if !self.prepare_source_slot(slot, ttl_seconds)? {
            return Ok(());
        }
        let (embedding, media_meta) = self.source_vector(fragment, hash)?;
        let mut extra: Vec<(&str, Value)> = vec![(CTX_SOURCE_FIELD, Value::Bool(true))];
        if let Some(media) = media_meta {
            extra.push((CTX_SOURCE_MEDIA_FIELD, media));
        }
        self.store_fact(
            slot,
            fragment.content.as_str(),
            &embedding,
            Some(&system_meta(&extra)),
            ttl_seconds,
        )
    }

    /// Whether `slot` may be written for this compile, clearing a stale point
    /// first when the write upgrades it to permanent.
    ///
    /// A slot never marked as ours is never rewritten: it is a caller fact
    /// squatting the salt preimage, and clobbering it would destroy user
    /// data. A slot already marked as ours holds these exact bytes — sources
    /// are content-addressed — so content and embedding never change; only
    /// durability can, and only upward (never-downgrade TTL upgrade, see
    /// [`Self::should_store_source`]), so a handle sold as permanent never
    /// silently expires just because an earlier compile first wrote it under
    /// a TTL.
    ///
    /// Upgrading to permanent needs the old point *gone*, not merely
    /// overwritten: velesdb-core's store path preserves every `_veles_*` key
    /// from a prior version of a re-stored id unless the new write explicitly
    /// sets it (`semantic_memory.rs`'s `store_internal` carry-forward, so
    /// plain `remember` doesn't silently wipe learned state), and a permanent
    /// write has no expiry to set (`attach_expiry` is a no-op without one) —
    /// so without this delete, `_veles_expires_at` would survive the
    /// "upgrade" untouched. A TTL-to-TTL extension needs no delete: its new
    /// expiry always overwrites the old one.
    fn prepare_source_slot(
        &self,
        slot: u64,
        ttl_seconds: Option<u64>,
    ) -> Result<bool, MemoryError> {
        if !self.should_store_source(slot, ttl_seconds)? {
            return Ok(false);
        }
        if ttl_seconds.is_none() && self.store.get(slot)?.is_some() {
            self.store.delete(slot)?;
        }
        Ok(true)
    }

    /// The vector a source slot is indexed by, plus the media descriptor to
    /// stamp on it when the fragment carries one.
    ///
    /// A media fragment's vector is deterministic and derived from the
    /// DECODED bytes — never the text embedder over `content` (often blank)
    /// nor over the base64 payload itself (opaque, not language). Correct
    /// because `retrieve_context_source` resolves a media source EXCLUSIVELY
    /// by its content-addressed hash/slot, never by vector search: the vector
    /// only has to be well-formed and non-degenerate for the underlying
    /// index, never semantically meaningful. For a media fragment `hash` IS
    /// the raw-bytes hash (see `fragment_handle_hash`), so nothing is
    /// re-decoded here.
    ///
    /// A TEXT fragment is embedded over at most
    /// [`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`] of its content
    /// ([`super::embeddable_prefix`]) — a `path`-ingested file can be 1 MiB,
    /// far past what the embedding backend accepts, and handing it over
    /// whole surfaced the backend's raw failure (issue #1654's residue,
    /// found on this very path). Truncating the *embedded* text, not the
    /// stored content, is the right trade here: retrieval is hash-addressed
    /// so the source stays whole, and the vector keeps ranking on the head
    /// of the text instead of vanishing from semantic recall.
    fn source_vector(
        &self,
        fragment: &ContextFragment,
        hash: u64,
    ) -> Result<(Vec<f32>, Option<Value>), MemoryError> {
        let Some(media_ref) = &fragment.media else {
            let embeddable = embeddable_prefix(fragment.content.as_str());
            return Ok((self.embedder.embed(embeddable)?, None));
        };
        let descriptor = serde_json::to_value(media_ref).unwrap_or(Value::Null);
        Ok((self.media_placeholder_embedding(hash), Some(descriptor)))
    }

    /// Whether [`Self::store_context_sources`] should (re-)write `slot` for
    /// this compile's requested (already [`positive_ttl`]-normalized —
    /// `None` means permanent) TTL.
    ///
    /// - Not marked as ours (absent, or a caller fact squatting the salt
    ///   preimage): store only if the slot is genuinely empty.
    /// - Marked as ours: never re-embed or change content (content-addressed);
    ///   only [`Self::should_upgrade_ttl`] decides whether durability changes.
    pub(super) fn should_store_source(
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
    pub(super) fn context_source_metadata(
        &self,
        slot: u64,
    ) -> Result<Option<Metadata>, MemoryError> {
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
        let _generation = self.enter_generation();
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

    /// Explain why one fragment of `request` was preserved, abstracted,
    /// externalized, dropped, or cached — the selection primitive the MCP
    /// `explain_compilation` tool delegates to, extracted here so every
    /// adapter (MCP, Node, Python) shares one implementation instead of
    /// reimplementing it. Compilation is deterministic, so `request` is
    /// simply re-compiled — with event/source recording forced off, since an
    /// explanation must not have side effects — and the matching decision is
    /// returned.
    ///
    /// `fragment_index` (0-based position in `request.fragments`), when
    /// given, TAKES PRIORITY over `fragment_id` for locating the decision:
    /// `compile_context` records exactly one decision per input fragment, in
    /// order, so `decisions[fragment_index]` is unambiguous even when
    /// several fragments are byte-identical and therefore share the same
    /// content-addressed `fragment_id` — a plain `fragment_id` lookup always
    /// resolves to the FIRST such decision (the deduplication survivor's),
    /// never a dropped twin's.
    ///
    /// Caveat inherited from re-compiling rather than replaying stored
    /// state: with a `memory_scope` the re-compile recalls from CURRENT
    /// memory, so the decision reflects memory as it is now, not as it was
    /// at the original `compile_context` call; a caller that already
    /// resolved a `path` fragment to `content` is unaffected (this method
    /// does no I/O of its own).
    ///
    /// # Errors
    /// Returns [`MemoryError::FragmentIndexOutOfBounds`] when `fragment_index`
    /// is beyond `request.fragments`, [`MemoryError::FragmentNotFound`] when
    /// no decision matches the selector, or any error [`Self::compile_context`]
    /// itself can return (budget, caps, recall, embedding, storage).
    pub fn explain_compilation(
        &self,
        request: &CompileRequest,
        fragment_id: u64,
        fragment_index: Option<usize>,
    ) -> Result<ContextDecision, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let _generation = self.enter_generation();
        if let Some(index) = fragment_index {
            let len = request.fragments.len();
            if index >= len {
                return Err(MemoryError::FragmentIndexOutOfBounds { index, len });
            }
        }
        let mut request = request.clone();
        let mut policy = request.policy.take().unwrap_or_default();
        // Three options neutralised for one reason: an explanation must not
        // inherit the side effects, nor the presentation, of the compilation it
        // explains. The caller asked "why this fragment?", not "compile this".
        policy.record_events = false;
        policy.store_sources = false;
        // `slim_response` empties `sections` and `decisions` to save tokens
        // (see `apply_slim`). Applied here it would not trim the answer, it
        // would DELETE it: `decisions` is cleared, the lookup below finds
        // nothing, and the caller is told `FragmentNotFound` about a fragment
        // that compiled perfectly well (#1745).
        //
        // The option exists to save tokens, so a caller under a tight budget
        // turns it on by default — and lost the audit tool exactly when they
        // most needed it, with a message that sent them looking for a typo in
        // an id that was correct.
        policy.slim_response = false;
        request.policy = Some(policy);
        let compiled =
            self.compile_context_inner(&ContextCompiler::new(CompilePolicy::default()), &request)?;
        let decision = if let Some(index) = fragment_index {
            compiled.decisions.into_iter().nth(index)
        } else {
            compiled
                .decisions
                .into_iter()
                .find(|decision| decision.fragment_id == fragment_id)
        };
        decision.ok_or(MemoryError::FragmentNotFound(fragment_id))
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
    pub fn context_savings(&self, project: Option<&str>) -> Result<ContextSavings, MemoryError>
    where
        S: RecallStore,
    {
        let _generation = self.enter_generation();
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
}
