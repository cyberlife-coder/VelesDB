//! Behaviour: with a worker spawned, `remember` returns as soon as the fact
//! is durably stored — the graph enrichment happens BEHIND the response
//! (#1846).
//!
//! Inline autograph held every `remember` for the extractor's full
//! generation: 46-52 s measured on the production daemon for a 12-word fact,
//! against a 0.12 s embedding — and the MCP client timed out mid-generation,
//! making a stored fact indistinguishable from a lost one (#1839). The
//! decoupling contract proven here, each clause with its own case:
//!
//! * the caller's latency is the WRITE's, not the model's;
//! * the enrichment still lands — `entity()` sees the wired edge within a
//!   bounded wait (an event poll, not a wall-clock guess: #1793's rule);
//! * a FULL queue drops enrichments COUNTED and never blocks the write;
//! * shutdown is BOUNDED: the in-flight job completes, queued jobs are
//!   skipped and counted — an exit never waits out a queue of generations.

#![cfg(feature = "persistence")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, ExtractedRelation, Extraction, Extractor, HashEmbedder,
    MemoryService, DEFAULT_DIMENSION,
};

/// An extractor that must be RELEASED by the test before answering — every
/// timing claim below is an event, never a sleep. Each release token lets
/// exactly one `extract_graph` through.
struct GatedExtractor {
    gate: Mutex<Receiver<()>>,
    calls: AtomicUsize,
}

impl GatedExtractor {
    fn new() -> (Arc<Self>, SyncSender<()>) {
        let (tx, rx) = sync_channel::<()>(64);
        (
            Arc::new(Self {
                gate: Mutex::new(rx),
                calls: AtomicUsize::new(0),
            }),
            tx,
        )
    }
}

impl Extractor for GatedExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(Vec::new())
    }

    fn extract_graph(&self, _text: &str) -> Result<Extraction, ExtractError> {
        self.gate
            .lock()
            .expect("gate lock")
            .recv()
            .map_err(|_| ExtractError::Backend("gate closed".to_owned()))?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Extraction {
            facts: Vec::new(),
            relations: vec![ExtractedRelation {
                subject: "alice martin".to_owned(),
                predicate: "travaille chez".to_owned(),
                object: "wiscale".to_owned(),
            }],
            attributes: Vec::new(),
        })
    }
}

/// A service over a temp store with the gated extractor as autograph, plus
/// the release channel. The [`TempDir`] must outlive everything.
fn gated_service() -> (
    TempDir,
    Arc<MemoryService<HashEmbedder>>,
    Arc<GatedExtractor>,
    SyncSender<()>,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (extractor, release) = GatedExtractor::new();
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open service")
        .with_autograph(extractor.clone());
    (dir, Arc::new(svc), extractor, release)
}

/// Poll until `probe` answers true — the event, not the clock (#1793).
fn wait_until(deadline: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + deadline;
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn remember_pays_for_the_write_not_for_the_model() {
    let (_dir, svc, _extractor, release) = gated_service();
    let _worker = svc
        .spawn_autograph_worker(8)
        .expect("spawn autograph worker");

    // The extractor is GATED shut: an inline autograph could not return at
    // all. The decoupled write must come back on the write's own budget.
    let started = Instant::now();
    svc.remember("Alice Martin travaille chez Wiscale.", &[], None)
        .expect("remember");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "remember must not wait on the extractor at all — it waited {elapsed:?} \
         with the extractor gate still SHUT, so it was on the response path"
    );

    // Release the enrichment and require it to LAND: the deferral must delay
    // the graph, never lose it.
    release.send(()).expect("release the gated extraction");
    let wired = wait_until(Duration::from_secs(10), || {
        svc.entity_profile("alice martin")
            .expect("entity lookup")
            .is_some_and(|p| !p.relations.is_empty())
    });
    assert!(
        wired,
        "the deferred enrichment must eventually wire alice martin's edge — \
         deferred is not dropped"
    );
}

#[test]
fn a_full_queue_drops_counted_and_never_blocks_the_write() {
    let (_dir, svc, extractor, release) = gated_service();
    let worker = svc
        .spawn_autograph_worker(2)
        .expect("spawn autograph worker");

    // Job 0 is taken by the worker and BLOCKS on the gate; jobs 1 and 2 fill
    // the queue (capacity 2); job 3 must be dropped — counted, and the
    // writes themselves still instant.
    let started = Instant::now();
    for i in 0..4 {
        svc.remember(&format!("fait en rafale numero {i}"), &[], None)
            .expect("remember");
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "four writes against a stuck extractor must not block on the queue"
    );
    let observed = wait_until(Duration::from_secs(5), || svc.autograph_dropped() == 1);
    assert!(
        observed,
        "exactly the one over-capacity enrichment is dropped and COUNTED, \
         got {} — a silent loss here is the defect class #1820 closed for \
         responses",
        svc.autograph_dropped()
    );

    // Shutdown is BOUNDED (#1846's approved design): close the queue while
    // job 0 is still in flight and jobs 1-2 sit queued. The in-flight job
    // completes; the queued ones are SKIPPED and counted with the same drop
    // counter — the daemon's exit must not wait out a queue of generations
    // (64 deep × a 46 s model would be tens of minutes).
    let joiner = std::thread::spawn(move || drop(worker));
    let closed = wait_until(Duration::from_secs(5), || !svc.autograph_queue_open());
    assert!(closed, "dropping the handle must close the queue first");
    release.send(()).expect("release the in-flight job");
    joiner.join().expect("join the dropping thread");
    assert_eq!(
        extractor.calls.load(Ordering::SeqCst),
        1,
        "only the in-flight job is wired on shutdown"
    );
    assert_eq!(
        svc.autograph_dropped(),
        3,
        "the two queued jobs skipped on shutdown join the overflow in the \
         SAME counter — a loss nobody can see is the defect class #1820 \
         closed for responses"
    );
}

#[test]
fn a_forgotten_fact_is_not_resurrected_by_its_stale_enrichment() {
    // A queued enrichment can sit for minutes-to-hours and its generation runs
    // for tens of seconds. If a `forget` lands in that window, the deletion is
    // durable and cannot be undone — the stale job must NOT wire the entity
    // hubs its extraction states, or `entity()` would serve structure derived
    // from a fact the user permanently deleted.
    let (_dir, svc, extractor, release) = gated_service();
    let worker = svc
        .spawn_autograph_worker(8)
        .expect("spawn autograph worker");

    // The write returns at once; the worker takes the job and BLOCKS in the
    // gated generation, having wired nothing yet.
    let fact_id = svc
        .remember("Alice Martin travaille chez Wiscale.", &[], None)
        .expect("remember");

    // Forget while the generation is stuck behind the gate. The fact is gone
    // and its (yet-unwired) hubs collect nothing — this is the permanent
    // deletion the stale job must respect.
    assert!(svc.forget(fact_id).expect("forget"), "the fact existed");

    // Release the generation and drain: dropping the handle closes the queue
    // and joins the worker, so the in-flight job's autograph has fully run (or
    // been skipped) by the time the join returns — a controlled order, no
    // wall-clock guess.
    release.send(()).expect("release the gated extraction");
    drop(worker);
    assert_eq!(
        extractor.calls.load(Ordering::SeqCst),
        1,
        "the released generation ran exactly once"
    );

    // The entity the retired fact would have introduced must not exist: a
    // permanently forgotten fact cannot be resurrected through the graph.
    assert!(
        svc.entity_profile("alice martin")
            .expect("entity lookup")
            .is_none(),
        "a stale enrichment resurrected the hubs of a forgotten fact — deletion \
         must be durable, and autograph derives structure only from a fact that \
         still exists"
    );
    assert!(
        svc.entity_profile("wiscale")
            .expect("entity lookup")
            .is_none(),
        "the second endpoint hub was resurrected too"
    );
}

#[test]
fn a_second_worker_is_refused_but_a_respawn_after_drop_works() {
    let (_dir, svc, extractor, release) = gated_service();
    let first = svc.spawn_autograph_worker(4).expect("first worker");
    assert!(
        svc.spawn_autograph_worker(4).is_err(),
        "two workers would race the single-writer store for no gain — the \
         second spawn must be refused, not silently doubled"
    );

    // After the first worker is gone, a respawn must serve again: the
    // closing latch is per-worker, not a one-way poison for the service.
    drop(first);
    let _second = svc.spawn_autograph_worker(4).expect("respawn after drop");
    svc.remember("Un fait après le respawn du worker.", &[], None)
        .expect("remember");
    release.send(()).expect("release the enrichment");
    let processed = wait_until(Duration::from_secs(5), || {
        extractor.calls.load(Ordering::SeqCst) == 1
    });
    assert!(
        processed,
        "a respawned worker must process new jobs — the shutdown latch has \
         to re-arm on spawn, or every later enrichment is silently skipped"
    );
}
