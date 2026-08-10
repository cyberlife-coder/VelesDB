//! Autograph: the background enrichment that turns a plain
//! [`MemoryService::remember`] into a self-building knowledge graph (#1846).
//! Split out of `service.rs` to keep that file under the crate's NLOC
//! budget; a child module of `service`, so it freely uses `MemoryService`'s
//! private fields and methods (`autograph`, `autograph_queue`,
//! `wire_entities`, `wire_relations`, `wire_attributes`, …) — same pattern
//! as `fused_recall` and `reinforce`.

use std::collections::{HashMap, HashSet};

use super::MemoryService;
use crate::embedder::Embedder;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::MemoryError;
use crate::storage::MemoryStore;

/// One deferred autograph: the stored fact a background worker will read for
/// entities, edges and attributes (#1846).
// The fields are read only on the worker path, which `spawn_autograph_worker`
// cfg-gates off wasm32 (no threads there) — without this the wasm check dies
// on dead_code under -D warnings.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct AutographJob {
    fact_id: u64,
    fact: String,
}

/// The decoupling state of [`MemoryService::autograph`] (#1846).
///
/// Empty by default: every construction path starts with autograph running
/// INLINE, exactly as before — the WASM binding has no threads, library
/// consumers keep the synchronous contract, and every existing test stays
/// meaningful. [`MemoryService::spawn_autograph_worker`] fills `tx`, after
/// which `remember` only ENQUEUES: the caller stops paying the generation on
/// its response path — 46 s measured for a 12-word fact on the production
/// daemon, versus a 0.12 s embedding — and the worker wires the graph behind.
///
/// `dropped` counts enrichments refused by a FULL queue or skipped by a
/// closing worker. Non-negotiably visible: a burst that outruns the
/// extractor loses graph structure, and a loss nobody can see is the exact
/// defect class #1820 closed for responses.
///
/// `closing` is the shutdown latch: the handle's drop raises it BEFORE
/// removing the sender, so the worker finishes the job in flight and SKIPS
/// what is still queued (counted, one aggregated warning) instead of
/// draining a queue of generations — 64 × a 46 s model would hold the
/// daemon's exit for tens of minutes. Re-armed by each spawn.
#[derive(Default)]
// `closing` is read only by the worker/drop path, absent on wasm32 — same
// rationale as `AutographJob` above.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct AutographQueue {
    tx: parking_lot::Mutex<Option<std::sync::mpsc::SyncSender<AutographJob>>>,
    dropped: std::sync::atomic::AtomicU64,
    closing: std::sync::atomic::AtomicBool,
}

/// Join guard for the background autograph worker.
///
/// Dropping it raises the closing latch, takes the sender out of the
/// service, and JOINS the worker: the job in flight completes, the
/// still-queued ones are SKIPPED — counted in the drop counter, one
/// aggregated warning — and only then does the drop return. Tests get
/// determinism; the daemon's shutdown waits for at most ONE generation,
/// never a queue of them.
pub struct AutographWorkerHandle {
    close_queue: Option<Box<dyn FnOnce() + Send + Sync>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for AutographWorkerHandle {
    fn drop(&mut self) {
        if let Some(close) = self.close_queue.take() {
            close();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// With a worker spawned ([`Self::spawn_autograph_worker`]), the job is
    /// ENQUEUED and this returns immediately: the enrichment leaves the
    /// caller's response path (#1846). A FULL queue drops the job, counted
    /// in [`Self::autograph_dropped`] — losing structure is recoverable by
    /// re-remembering, stalling every write behind a slow model is not. A
    /// disconnected queue (worker gone) falls back inline, so the graph
    /// keeps building even if the worker died.
    pub(super) fn autograph_if(&self, run: bool, fact_id: u64, fact: &str) {
        if !run {
            return;
        }
        let guard = self.autograph_queue.tx.lock();
        if let Some(tx) = guard.as_ref() {
            use std::sync::mpsc::TrySendError;
            match tx.try_send(AutographJob {
                fact_id,
                fact: fact.to_owned(),
            }) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    self.autograph_queue
                        .dropped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    #[cfg(feature = "mcp")]
                    tracing::warn!(
                        fact_id,
                        "autograph queue full: enrichment dropped — the fact is \
                         stored, its graph structure is not; re-remembering \
                         rebuilds it"
                    );
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    // fall through to the inline path below
                }
            }
        }
        drop(guard);
        self.autograph(fact_id, fact);
    }

    /// How many autograph enrichments a FULL queue refused since this
    /// service was built (#1846). The facts themselves were stored; only
    /// their graph wiring was skipped, and re-remembering a fact rebuilds it.
    #[must_use]
    pub fn autograph_dropped(&self) -> u64 {
        self.autograph_queue
            .dropped
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the background autograph queue is OPEN — a worker is spawned
    /// and `remember` enqueues instead of running the enrichment inline.
    /// Turns false the moment a worker handle's drop closes the queue.
    #[must_use]
    pub fn autograph_queue_open(&self) -> bool {
        self.autograph_queue.tx.lock().is_some()
    }

    /// Whether an autograph extractor is configured at all.
    #[must_use]
    pub fn has_autograph(&self) -> bool {
        self.autograph.is_some()
    }
}

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// Autograph one just-stored fact: read the entities, entity→entity edges
    /// and attributes it states, and wire them around it.
    ///
    /// **Deliberately infallible.** The caller's fact is already durably
    /// stored by the time this runs, and the caller asked to remember a fact —
    /// not to run a model. Propagating an extraction failure would turn a
    /// successful write into a reported error, and an agent that sees
    /// `remember` fail will sensibly retry it, re-running the generation and
    /// failing again. So a model that is down, slow, or talking nonsense costs
    /// the *graph enrichment* and nothing else: the memory is kept, the id is
    /// returned, and the next `remember` tries again.
    ///
    /// The trade-off is that a persistently broken extractor degrades silently
    /// to plain `remember`. That is the right way round — losing structure is
    /// recoverable by re-remembering, losing the fact is not.
    fn autograph(&self, fact_id: u64, fact: &str) {
        let Some(extractor) = self.autograph.as_ref() else {
            return;
        };
        let Ok(mut extraction) = extractor.extract_graph(fact) else {
            return;
        };
        // Privacy invariant: autograph derives structure FROM a stored fact, so
        // if the fact no longer exists, none of its derived structure may be
        // created. With a background worker a job can sit queued for
        // minutes-to-hours and its generation runs for tens of seconds, so a
        // `forget` issued in between must win — a permanent deletion cannot be
        // undone by a stale enrichment resurrecting the entity hubs. Re-check
        // once the generation has returned, before any wiring: this closes the
        // whole queue-plus-generation window, the common case, completely.
        if !self.fact_exists(fact_id) {
            return;
        }
        crate::extract::orient_kinship(fact, &mut extraction.relations);
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64, String)> = HashSet::new();
        // The caller's fact is the node the topics attach to — the extracted
        // facts are NOT stored as separate memories here, which is what
        // separates autograph from `remember_extracted`: one `remember` call
        // must still produce exactly one caller-visible memory.
        for extracted in &extraction.facts {
            let _ = self.wire_entities(fact_id, &extracted.entities, &mut entity_ids, &mut edges);
        }
        // A concurrent `forget` can still race the wiring writes above between
        // the first check and here. Re-check once more before the hub↔hub and
        // attribute writes — neither of which references the source fact, so
        // `ensure_exists` cannot catch a retired fact for them — leaving a
        // residual window of a single already-committed generation, not the
        // whole job.
        if !self.fact_exists(fact_id) {
            return;
        }
        let _ = self.wire_relations(&extraction.relations, &mut entity_ids, &mut edges);
        let _ = self.wire_attributes(&extraction.attributes, &mut entity_ids);
    }

    /// Cheap "is this fact still stored?" probe gating [`Self::autograph`]'s
    /// wiring: the same `store.get` existence check [`Self::forget`] and
    /// [`Self::ensure_exists`] use. A store read error answers `false` —
    /// autograph must never fabricate structure for a fact it cannot prove is
    /// still there, and a missing (or unprovable) fact is a clean skip, never
    /// an error on this deliberately-infallible path.
    fn fact_exists(&self, fact_id: u64) -> bool {
        matches!(self.store.get(fact_id), Ok(Some(_)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<E, S> MemoryService<E, S>
where
    E: Embedder + Send + Sync + 'static,
    S: MemoryStore + Send + Sync + 'static,
{
    /// Move autograph off the response path: spawn ONE background worker
    /// consuming a bounded queue, so `remember` returns as soon as the fact
    /// is durably stored and the graph is wired behind (#1846).
    ///
    /// Measured motivation: with the production extractor, an inline
    /// autograph held every `remember` for 46-52 s while the embedding cost
    /// 0.12 s — and the MCP client timed out mid-generation, making a stored
    /// fact indistinguishable from a lost one (#1839).
    ///
    /// The read-after-write contract changes, deliberately and visibly: an
    /// `entity()` issued right after `remember` may not see the new edges
    /// yet. The fact itself is always readable immediately — only the
    /// DERIVED structure lags by one generation.
    ///
    /// One worker on purpose: the store is single-writer, and a second
    /// in-flight generation would only add contention, not throughput.
    /// `capacity` bounds the queue ([`crate::limits::MAX_AUTOGRAPH_QUEUE`]
    /// is the daemon's choice); a full queue DROPS new enrichments, counted
    /// by [`Self::autograph_dropped`] and logged — never silent, never
    /// blocking the write path.
    ///
    /// # Errors
    /// Returns [`MemoryError::Extract`] when a worker is already spawned for
    /// this service — two workers would race the single-writer store for no
    /// gain — or when the OS refuses the thread.
    pub fn spawn_autograph_worker(
        self: &std::sync::Arc<Self>,
        capacity: usize,
    ) -> Result<AutographWorkerHandle, MemoryError> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AutographJob>(capacity);
        self.install_autograph_sender(tx)?;
        let worker_service = std::sync::Arc::clone(self);
        let join = std::thread::Builder::new()
            .name("velesdb-autograph".to_owned())
            .spawn(move || worker_service.drain_autograph_queue(rx))
            .map_err(|err| {
                MemoryError::Extract(crate::extract::ExtractError::Backend(format!(
                    "spawn autograph worker: {err}"
                )))
            })?;
        let closer_service = std::sync::Arc::clone(self);
        Ok(AutographWorkerHandle {
            close_queue: Some(Box::new(move || {
                // Latch FIRST, sender out second: the worker observes the
                // latch no later than the queue's end, so it cannot start
                // draining jobs the shutdown meant to skip.
                closer_service
                    .autograph_queue
                    .closing
                    .store(true, std::sync::atomic::Ordering::Release);
                closer_service.autograph_queue.tx.lock().take();
            })),
            join: Some(join),
        })
    }

    /// Install `tx` as the queue's sender under one lock, so a previous
    /// worker's shutdown latch can never poison a freshly spawned one — see
    /// [`Self::spawn_autograph_worker`].
    fn install_autograph_sender(
        &self,
        tx: std::sync::mpsc::SyncSender<AutographJob>,
    ) -> Result<(), MemoryError> {
        let mut guard = self.autograph_queue.tx.lock();
        if guard.is_some() {
            return Err(MemoryError::Extract(crate::extract::ExtractError::Backend(
                "autograph worker already spawned for this service".to_owned(),
            )));
        }
        *guard = Some(tx);
        // Re-arm the shutdown latch under the same lock that installs the
        // sender: a previous worker's close must not poison this one into
        // skipping every job it will ever receive.
        self.autograph_queue
            .closing
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    /// The autograph worker thread's body: consume `rx` until every sender is
    /// gone — i.e. until the handle's drop takes the sender back out of the
    /// service. Once the closing latch is up, still-queued jobs are SKIPPED:
    /// the exit pays for the job in flight, never for the queue.
    fn drain_autograph_queue(
        self: std::sync::Arc<Self>,
        rx: std::sync::mpsc::Receiver<AutographJob>,
    ) {
        let mut skipped_on_close: u64 = 0;
        for job in rx {
            if self
                .autograph_queue
                .closing
                .load(std::sync::atomic::Ordering::Acquire)
            {
                skipped_on_close += 1;
                continue;
            }
            self.autograph(job.fact_id, &job.fact);
        }
        if skipped_on_close > 0 {
            self.autograph_queue
                .dropped
                .fetch_add(skipped_on_close, std::sync::atomic::Ordering::Relaxed);
            // ONE aggregated line, not one per job (#1834's rule).
            #[cfg(feature = "mcp")]
            tracing::warn!(
                skipped = skipped_on_close,
                "autograph worker closing: queued enrichments skipped — \
                 the facts are stored, their graph structure is not; \
                 re-remembering rebuilds it"
            );
        }
    }
}
