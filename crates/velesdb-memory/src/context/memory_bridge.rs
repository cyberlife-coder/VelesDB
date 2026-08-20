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
    CompilePolicy, CompileRequest, CompiledContext, ContextDecision, ContextFragment,
    ContextSavings, ContextSource, ImportanceWeights, LoadedWorkingContext, MediaRef, MemoryScope,
    WorkingContext, WorkingContextIndex, WorkingContextSession,
};
use crate::context::{media, provenance, ContextCompiler};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::id::stable_id;
use crate::model::FusionOptions;
use crate::storage::{FactStore, GraphStore, RecallStore};

/// Salt for stored source ids — disjoint from natural fact ids, so a caller
/// later remembering the same text can never overwrite a stored source (or
/// inherit its system marker).
const SOURCE_ID_SALT: &str = "veles-ctx-source:";
/// Salt for compilation-event ids.
const EVENT_ID_SALT: &str = "veles-ctx-event:";
/// Salt for working-context ids (deterministic per project+session, so a
/// save is an idempotent upsert).
const WORKING_ID_SALT: &str = "veles-ctx-working:";
/// Salt for a project's working-context index id (deterministic per
/// project, so every `save_working_context` call updates the SAME system
/// fact rather than minting a new one).
const WORKING_INDEX_ID_SALT: &str = "veles-ctx-working-index:";

/// The constant lexical anchor every event's content starts with, so one
/// vector query can sweep the event family for aggregation.
const EVENT_ANCHOR: &str = "veles context compilation event";

/// Reserved metadata keys of the bridge's system facts. Reserved (`_veles_`)
/// on purpose: callers can neither set them (forgery) nor filter on them, and
/// [`MemoryService::context_savings`] aggregates only genuine events (it
/// filters at the storage layer, below the caller-facing validation).
///
/// Being unfilterable was once claimed here to make these facts "invisible to
/// every caller-facing recall path". It did not (#1737). A caller cannot
/// filter ON a reserved key, but `field != value` MATCHES a fact that has no
/// such field — and a system fact has none of the caller's columns, so every
/// `!=` predicate swept all of them in. Invisibility is now an exclusion
/// [`crate::storage::INTERNAL_MARKER_FIELDS`] states and each backend
/// applies, not a side effect of the naming rule.
///
/// The four markers below are therefore imported rather than redeclared: they
/// ARE entries of that list, and a local copy could drift from it silently.
use crate::storage::{
    CTX_EVENT_FIELD, CTX_SOURCE_FIELD, CTX_WORKING_FIELD, CTX_WORKING_INDEX_FIELD,
};

const CTX_PROJECT_FIELD: &str = "_veles_ctx_project";
const CTX_MODEL_FIELD: &str = "_veles_ctx_model";
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

/// Serializes the read-modify-write of the per-project working-context index.
///
/// The index is ONE fact per project, rewritten wholesale on every
/// `save_working_context`. Without this, two saves racing on the same project
/// both read the same pre-state and the second write erases the first
/// session's entry — a silent loss: the erased session's own fact is still on
/// disk and still loadable by exact id, but `list_working_contexts` (and
/// therefore `load_working_context`'s `other_sessions` recovery hint) no
/// longer knows it exists, and nothing anywhere returns an error.
///
/// **Scope: intra-process — which is the WHOLE problem (#1958).** An earlier
/// version of this comment claimed two processes opening the same store
/// still race past this lock. They cannot: `velesdb-core`'s
/// `Database::open_impl` takes an exclusive `flock` on `velesdb.lock` at
/// open and holds it for the `Database`'s entire lifetime — not per write —
/// so a second process fails at `open` with `DatabaseLocked` before it can
/// reach any read-modify-write, of this index or of anything else. Proven
/// with real processes by `tests/http_lock_contention.rs` and
/// `tests/working_index_two_daemons_process.rs` (the latter is #1958's
/// success criterion verbatim: sessions saved under contention and across a
/// process handoff, zero index entries lost). A trait-level compare-and-swap
/// was considered there and declined: flock is the only cross-process
/// primitive available here, and the store boundary already holds it — a
/// second one around the index would guard against a concurrency the first
/// makes unreachable. This mutex therefore covers the only concurrency that
/// exists: threads of the one process allowed to hold the store (the MCP
/// server's `spawn_blocking` handlers are exactly what made it reachable).
///
/// One global lock rather than one per project: index writes are rare (one
/// per `save_working_context`), so the contention is negligible, whereas a
/// `HashMap<String, _>` keyed by caller-supplied project names is an unbounded
/// slow leak for no measurable gain. Per-project striping is the obvious
/// upgrade if index writes ever become hot.
static WORKING_INDEX_WRITE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// The compilation half — `compile_context` and its helpers; see
/// `memory_bridge_compile.rs`'s module doc for why it is split.
#[path = "memory_bridge_compile.rs"]
mod compile;

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// Persist `working` under `project` + `session` (idempotent upsert:
    /// saving again replaces the previous state). Returns the system fact id.
    ///
    /// Serialized size is capped at [`crate::limits::MAX_FACT_BYTES`] (1
    /// MiB) — the same ceiling every other stored fact honors — checked
    /// BEFORE anything is written, so an oversized working context is never
    /// partially stored.
    ///
    /// An entirely empty `working` ([`WorkingContext::is_empty`]) is refused.
    /// Because the write is an upsert, saving one would replace — destroy —
    /// the state a previous save stored under the same project and session,
    /// and the one tool whose job is surviving a context loss must not be
    /// able to cause one on a call that carries nothing (issue #1654).
    ///
    /// # Errors
    /// Returns [`MemoryError::EmptyWorkingContext`] if `working` records
    /// nothing, [`MemoryError::WorkingContextCodec`] if serialization fails,
    /// [`MemoryError::ContextOverLimit`] if the serialized `working` exceeds
    /// [`crate::limits::MAX_FACT_BYTES`], or a storage/embedding error.
    pub fn save_working_context(
        &self,
        project: &str,
        session: &str,
        working: &WorkingContext,
    ) -> Result<u64, MemoryError> {
        let _generation = self.enter_generation();
        if working.is_empty() {
            return Err(MemoryError::EmptyWorkingContext);
        }
        let content =
            serde_json::to_string(working).map_err(|err| MemoryError::WorkingContextCodec {
                detail: "encoding the working context for storage".to_owned(),
                source: Some(Box::new(err)),
            })?;
        if content.len() > crate::limits::MAX_FACT_BYTES {
            return Err(MemoryError::ContextOverLimit(format!(
                "working context of {} bytes exceeds the cap of {} bytes",
                content.len(),
                crate::limits::MAX_FACT_BYTES
            )));
        }
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
        self.update_working_index(project, session)?;
        Ok(id)
    }

    /// The working context previously saved under `project` + `session`,
    /// `None` when there is none.
    ///
    /// Symmetric to [`Self::context_source_metadata`]'s squatter guard: the
    /// slot is only ever served back when its metadata carries the reserved
    /// [`CTX_WORKING_FIELD`] marker (set exclusively by
    /// [`Self::save_working_context`]). A slot occupied by an unmarked caller
    /// fact — one that happened to land on this salted id, or a forged
    /// probe — is indistinguishable from "nothing saved" on purpose: `None`,
    /// never the forged content, and never an error (the caller cannot tell
    /// a squatted slot from a genuinely empty one, which is the point — it
    /// must never learn that *something* occupies this id).
    ///
    /// A pure read: it never writes, never prunes, never heals. Index
    /// convergence happens on the WRITE path
    /// ([`Self::update_working_index`]) — a lookup that rewrites shared state
    /// turns every transient miss into permanent data loss and cannot safely
    /// be retried.
    ///
    /// # Errors
    /// Returns [`MemoryError::WorkingContextCodec`] if the stored payload
    /// does not parse, or if the slot is marked but its body is gone (a torn
    /// fact is corruption — reporting it as "nothing saved" would tell the
    /// caller the one thing that is certainly false), or a storage error.
    pub fn load_working_context(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Option<WorkingContext>, MemoryError> {
        let _generation = self.enter_generation();
        self.load_working_context_inner(project, session)
    }

    fn load_working_context_inner(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Option<WorkingContext>, MemoryError> {
        let slot = working_id(project, session);
        let payloads = self.store.get_metadata_batch(&[slot])?;
        let marked = payloads
            .into_iter()
            .next()
            .flatten()
            .is_some_and(|meta| meta.get(CTX_WORKING_FIELD) == Some(&Value::Bool(true)));
        if !marked {
            // The squatter/never-saved guard documented above: silent by
            // design, and the branch a `forget` lands on (deleting a fact
            // removes its metadata with it).
            return Ok(None);
        }
        let Some((content, _)) = self.store.get(slot)? else {
            return Err(MemoryError::WorkingContextCodec {
                detail: format!(
                    "working context for project '{project}', session '{session}' is corrupt: \
                     the reserved marker is present but the stored body is gone"
                ),
                source: None,
            });
        };
        serde_json::from_str(&content)
            .map(Some)
            .map_err(|err| MemoryError::WorkingContextCodec {
                detail: format!(
                    "decoding the stored working context for project '{project}', \
                     session '{session}'"
                ),
                source: Some(Box::new(err)),
            })
    }

    /// The full resumption envelope for `project` + `session`: what
    /// [`Self::load_working_context`] found, plus the OTHER sessions saved
    /// under the same project so a typo in `session` is recoverable.
    ///
    /// This is the ONE place the three policy rules live:
    ///
    /// 1. `other_sessions` is listed on a HIT too, not just on a miss — a
    ///    typo that lands on another REAL session returns `found: true`, and
    ///    the caller has no other way to notice it resumed the wrong work.
    ///    Costs one extra O(1) index read per successful load.
    /// 2. The requested `session` is never echoed back: the field is named
    ///    `other_sessions`, so returning the requested id would be a
    ///    contradiction the caller cannot act on.
    /// 3. An unreadable index is fatal on a MISS and survivable on a HIT —
    ///    see [`Self::other_sessions_for`].
    ///
    /// Every surface (the `load_working_context` MCP tool and the Node,
    /// Python and WASM bindings) calls this rather than recomposing the
    /// envelope from [`Self::load_working_context`] +
    /// [`Self::list_working_contexts`]: four recompositions are four copies
    /// of those rules, and a copy that stops matching the others fails
    /// silently — the caller still gets a well-formed envelope, just a
    /// different one.
    ///
    /// # Errors
    /// Propagates [`Self::load_working_context`]'s errors (a corrupt or
    /// unparseable stored payload), and [`Self::list_working_contexts`]'s (a
    /// corrupt index, or a storage failure) **on a miss only** — rule 3.
    pub fn resume_working_context(
        &self,
        project: &str,
        session: &str,
    ) -> Result<LoadedWorkingContext, MemoryError> {
        let _generation = self.enter_generation();
        let working = self.load_working_context_inner(project, session)?;
        let other_sessions = self.other_sessions_for(project, session, working.is_some())?;
        Ok(LoadedWorkingContext {
            found: working.is_some(),
            working,
            other_sessions,
        })
    }

    /// The project's OTHER sessions, and what to do when the index that holds
    /// them cannot be read.
    ///
    /// The two answers differ because `other_sessions` plays a different part
    /// on each path:
    ///
    /// - **On a hit** it is a HINT — "you asked for `alpha`, note that
    ///   `alpha-2` also exists, you may have resumed the wrong one". The
    ///   answer the caller actually asked for is already in hand and intact.
    ///   Failing the whole call here would turn a fault in one auxiliary fact
    ///   into a total loss of resumption for EVERY session of the project,
    ///   including the many that read back perfectly — which is why an
    ///   unreadable index degrades to an empty hint instead. Nothing is
    ///   swallowed: the corruption stays loudly reachable through
    ///   [`Self::list_working_contexts`], published on every surface.
    /// - **On a miss** it is the ONLY signal there is. `[]` then reads as the
    ///   positive assertion "nothing else was ever saved under this project",
    ///   and an agent told that starts over on top of work sitting right next
    ///   to where it looked — the exact failure this envelope exists to
    ///   prevent. An assertion we cannot support must not be manufactured, so
    ///   the error propagates.
    ///
    /// # Errors
    /// Propagates [`Self::list_working_contexts`]'s errors when `found` is
    /// false.
    fn other_sessions_for(
        &self,
        project: &str,
        session: &str,
        found: bool,
    ) -> Result<Vec<String>, MemoryError> {
        let listed = match self.list_working_contexts_inner(project) {
            Ok(listed) => listed,
            Err(_) if found => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        Ok(listed
            .into_iter()
            .map(|entry| entry.session)
            .filter(|candidate| candidate != session)
            .collect())
    }

    /// The sessions of `sessions` whose working-context fact is still there,
    /// in the same order. One batched metadata lookup for the whole set — not
    /// a store scan, but not free either (see
    /// [`Self::list_working_contexts`]'s cost note).
    ///
    /// Shared by the read path (filter, persist nothing) and the write path
    /// (filter, and persist the result), so both agree on what "alive" means.
    fn live_sessions(
        &self,
        project: &str,
        sessions: Vec<WorkingContextSession>,
    ) -> Result<Vec<WorkingContextSession>, MemoryError> {
        if sessions.is_empty() {
            return Ok(sessions);
        }
        let ids: Vec<u64> = sessions
            .iter()
            .map(|entry| working_id(project, &entry.session))
            .collect();
        let payloads = self.store.get_metadata_batch(&ids)?;
        if payloads.len() != ids.len() {
            // The trait promises one result per id. A backend that breaks
            // that promise must not be silently read as "these sessions are
            // dead" — that would delete real entries on the write path.
            return Err(MemoryError::WorkingContextCodec {
                detail: format!(
                    "storage returned {} metadata rows for {} working-context ids",
                    payloads.len(),
                    ids.len()
                ),
                source: None,
            });
        }
        Ok(sessions
            .into_iter()
            .zip(payloads)
            .filter(|(_, meta)| {
                meta.as_ref()
                    .is_some_and(|meta| meta.get(CTX_WORKING_FIELD) == Some(&Value::Bool(true)))
            })
            .map(|(entry, _)| entry)
            .collect())
    }

    /// Every session still resumable under `project`'s working-context index
    /// (V2a-1 quick win), most-recently-saved first. Empty when the project
    /// never saved anything — that, and only that, is the empty case.
    ///
    /// Cost: one O(1) index read plus ONE batched metadata lookup of the
    /// listed ids — never a store scan, but no longer a single read either.
    /// The lookup is what drops sessions whose fact was forgotten since;
    /// unlike the previous read-path prune it persists nothing, so a listing
    /// can be retried and a transient miss costs nothing durable.
    ///
    /// # Errors
    /// Returns a storage error if the index fact cannot be read, or
    /// [`MemoryError::WorkingContextCodec`] if it does not parse or is
    /// corrupt (marked, but with no body).
    pub fn list_working_contexts(
        &self,
        project: &str,
    ) -> Result<Vec<WorkingContextSession>, MemoryError> {
        let _generation = self.enter_generation();
        self.list_working_contexts_inner(project)
    }

    fn list_working_contexts_inner(
        &self,
        project: &str,
    ) -> Result<Vec<WorkingContextSession>, MemoryError> {
        let Some(index) = self.working_index(project)? else {
            // The genuine "this project never saved anything" case — the only
            // one that reaches here now that a corrupt index is an `Err`.
            return Ok(Vec::new());
        };
        let mut sessions = self.live_sessions(project, index.sessions)?;
        sessions.sort_by(|a, b| {
            b.saved_at
                .cmp(&a.saved_at)
                .then_with(|| a.session.cmp(&b.session))
        });
        Ok(sessions)
    }

    /// The raw working-context index fact for `project`, `None` when nothing
    /// was ever saved under it. Symmetric squatter guard to
    /// [`Self::load_working_context`]: a slot occupied without the reserved
    /// [`CTX_WORKING_INDEX_FIELD`] marker is treated as empty, never as a
    /// forged index.
    ///
    /// `None` means "absent". "Corrupt" is an `Err` — collapsing the two
    /// would report a store that lost the index body as a project that never
    /// saved anything, and an agent told that starts over instead of raising
    /// a problem a human could fix.
    fn working_index(&self, project: &str) -> Result<Option<WorkingContextIndex>, MemoryError> {
        let slot = working_index_id(project);
        let payloads = self.store.get_metadata_batch(&[slot])?;
        let marked = payloads
            .into_iter()
            .next()
            .flatten()
            .is_some_and(|meta| meta.get(CTX_WORKING_INDEX_FIELD) == Some(&Value::Bool(true)));
        if !marked {
            return Ok(None);
        }
        match self.store.get(slot)? {
            Some((content, _)) => serde_json::from_str(&content).map(Some).map_err(|err| {
                MemoryError::WorkingContextCodec {
                    detail: format!("decoding the working-context index for project '{project}'"),
                    source: Some(Box::new(err)),
                }
            }),
            None => Err(MemoryError::WorkingContextCodec {
                detail: format!(
                    "working-context index for project '{project}' is corrupt: the index \
                     marker is present but the stored body is gone"
                ),
                source: None,
            }),
        }
    }

    /// Append (or refresh) `session`'s entry in `project`'s working-context
    /// index — called by every [`Self::save_working_context`], so the index
    /// is always current without a separate maintenance step. A resave of
    /// the same project+session updates `saved_at` in place rather than
    /// duplicating the entry.
    ///
    /// This is also where the index CONVERGES: entries whose working-context
    /// fact was forgotten since are dropped here, on the write path, under
    /// the same lock and in the same read-modify-write that was already
    /// paid for. Reads never mutate it.
    fn update_working_index(&self, project: &str, session: &str) -> Result<(), MemoryError> {
        // The index slot's embedding derives from the PROJECT NAME alone,
        // never from the index content, so it is computed here, BEFORE the
        // lock: an embedder can be a network round-trip (or a hung one), and
        // holding the global write lock across it stalls every working-index
        // write in the process behind one slow call. Racing saves may embed
        // concurrently, but they embed the same text, so whichever vector
        // lands is equivalent — no re-check under the lock is needed. The
        // index CONTENT read-modify-write stays entirely under the lock.
        let embedding = self
            .embedder
            .embed(&format!("working context index {project}"))?;
        // Read-modify-write of a single shared fact: held for the whole
        // sequence, otherwise a concurrent save silently erases this entry.
        let _guard = WORKING_INDEX_WRITE.lock();
        // A corrupt index must not brick saving for the whole project. The
        // read path surfaces the error — that is where a human can act on it
        // — but propagating it here would make every future save of every
        // session under this project fail forever, with no way back: the
        // only writer of the index is this function. Rebuild instead.
        let mut index = match self.working_index(project) {
            Ok(index) => index.unwrap_or_default(),
            Err(MemoryError::WorkingContextCodec { .. }) => WorkingContextIndex::default(),
            Err(err) => return Err(err),
        };
        let now = now_unix_secs();
        if let Some(entry) = index.sessions.iter_mut().find(|s| s.session == session) {
            entry.saved_at = now;
        } else {
            index.sessions.push(WorkingContextSession {
                session: session.to_owned(),
                saved_at: now,
            });
        }
        // The entry just appended is alive by construction (its fact was
        // stored moments ago, before this call); this only sheds the ones a
        // `forget` orphaned.
        index.sessions = self.live_sessions(project, index.sessions)?;
        let content =
            serde_json::to_string(&index).map_err(|err| MemoryError::WorkingContextCodec {
                detail: format!("encoding the working-context index for project '{project}'"),
                source: Some(Box::new(err)),
            })?;
        self.write_working_index(project, &content, &embedding)
    }

    /// Persist a serialized index into `project`'s reserved index slot —
    /// always with the [`CTX_WORKING_INDEX_FIELD`] marker, since an index
    /// written without it would be treated as a squatter and read back as
    /// empty. Only [`Self::update_working_index`] (which holds
    /// [`WORKING_INDEX_WRITE`] and supplies the slot `embedding` it computed
    /// before taking that lock) calls this: nothing in here may call the
    /// embedder, or the lock would again be held across a network hop.
    fn write_working_index(
        &self,
        project: &str,
        content: &str,
        embedding: &[f32],
    ) -> Result<(), MemoryError> {
        let slot = working_index_id(project);
        let meta = system_meta(&[
            (CTX_WORKING_INDEX_FIELD, Value::Bool(true)),
            (CTX_PROJECT_FIELD, Value::String(project.to_owned())),
        ]);
        self.store_fact(slot, content, embedding, Some(&meta), None)?;
        Ok(())
    }
}

/// How many memories a scope pulls when it does not say (`k` absent).
const DEFAULT_MEMORY_K: usize = 5;

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
                path: None,
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

/// A `u64` metadata field, `0` when absent or non-numeric.
fn meta_u64(meta: &Metadata, key: &str) -> u64 {
    meta.get(key).and_then(Value::as_u64).unwrap_or(0)
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

/// The salted, deterministic system-fact id of a working context.
fn working_id(project: &str, session: &str) -> u64 {
    stable_id(&format!("{WORKING_ID_SALT}{project}\u{1f}{session}"))
}

/// The salted, deterministic system-fact id of a project's working-context
/// index — one per project, so every save updates the same slot.
fn working_index_id(project: &str) -> u64 {
    stable_id(&format!("{WORKING_INDEX_ID_SALT}{project}"))
}

#[cfg(all(test, feature = "persistence"))]
#[path = "memory_bridge_tests.rs"]
mod tests;
