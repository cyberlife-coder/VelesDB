//! Regression: the global `WORKING_INDEX_WRITE` lock must never be held
//! across an embedder call.
//!
//! The working-context index write needs an embedding for its slot, and an
//! embedder can be a network round-trip — hundreds of milliseconds, seconds,
//! or a hung connection. Before the fix, `update_working_index` acquired the
//! process-global index lock and THEN embedded, so one slow embedder call
//! stalled every working-index write in the process (all sessions, all
//! projects — the lock is global by design). The fix computes the embedding
//! before taking the lock, which then only covers the in-memory/local
//! read-modify-write of the index fact.
//!
//! The scenario: save A blocks inside its (gated) embedder while save B, on
//! an independent store with a fast embedder, contends only on the global
//! lock. B must complete while A is still stuck; releasing A must let it
//! finish correctly. Every wait is a bounded `recv_timeout`, so the test can
//! time out red but can never hang.

#![cfg(all(feature = "context", feature = "persistence"))]

use std::sync::{mpsc, Arc};
use std::time::Duration;

use velesdb_memory::context::WorkingContext;
use velesdb_memory::{EmbedError, Embedder, HashEmbedder, MemoryService};

/// Small dimension: recall quality is irrelevant here.
const DIM: usize = 16;

/// Generous bound for steps that must complete promptly; far below the
/// gated embedder's own 30 s watchdog, so a stalled B is unambiguous.
const STEP: Duration = Duration::from_secs(10);

/// The prefix of the text `update_working_index` embeds for the index slot.
const INDEX_EMBED_PREFIX: &str = "working context index";

/// An embedder that signals and then BLOCKS (as a hung network call would)
/// the first time it is asked to embed the working-index slot text, until
/// the test releases it. All other embeds pass straight through to the
/// deterministic [`HashEmbedder`]. The internal wait is itself bounded so
/// even a buggy test run terminates.
struct GatedIndexEmbedder {
    inner: HashEmbedder,
    entered: mpsc::Sender<()>,
    release: parking_lot::Mutex<mpsc::Receiver<()>>,
}

impl Embedder for GatedIndexEmbedder {
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        if text.starts_with(INDEX_EMBED_PREFIX) {
            // Signal the test we are inside the "network call", then hang
            // until released (bounded: a lost release cannot hang the run).
            let _ = self.entered.send(());
            let _ = self.release.lock().recv_timeout(Duration::from_secs(30));
        }
        self.inner.embed(text)
    }
}

fn working_state(goal: &str) -> WorkingContext {
    WorkingContext {
        goal: Some(goal.to_owned()),
        ..WorkingContext::default()
    }
}

#[test]
fn test_save_working_context_is_not_stalled_by_a_peer_hung_in_its_embedder() {
    // Given service A whose embedder hangs on the index-slot embed, and an
    // independent service B with a fast embedder. The two share nothing but
    // the process-global working-index write lock.
    let dir_a = tempfile::TempDir::new().expect("tempdir a");
    let dir_b = tempfile::TempDir::new().expect("tempdir b");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let svc_a = Arc::new(
        MemoryService::open(
            dir_a.path(),
            GatedIndexEmbedder {
                inner: HashEmbedder::new(DIM),
                entered: entered_tx,
                release: parking_lot::Mutex::new(release_rx),
            },
        )
        .expect("open service a"),
    );
    let svc_b = Arc::new(
        MemoryService::open(dir_b.path(), HashEmbedder::new(DIM)).expect("open service b"),
    );

    // When save A reaches its embedder and hangs there
    let (a_done_tx, a_done_rx) = mpsc::channel();
    let a = Arc::clone(&svc_a);
    let join_a = std::thread::spawn(move || {
        let result = a.save_working_context("proj-a", "s1", &working_state("goal a"));
        let _ = a_done_tx.send(result);
    });
    entered_rx
        .recv_timeout(STEP)
        .expect("save A must reach its index embed");

    // ... and save B runs concurrently
    let (b_done_tx, b_done_rx) = mpsc::channel();
    let b = Arc::clone(&svc_b);
    let join_b = std::thread::spawn(move || {
        let result = b.save_working_context("proj-b", "s1", &working_state("goal b"));
        let _ = b_done_tx.send(result);
    });

    // Then B completes while A is still blocked in its embedder. (Release A
    // before asserting, so a red run also terminates cleanly.)
    let b_outcome = b_done_rx.recv_timeout(STEP);
    let _ = release_tx.send(());
    let b_result = b_outcome.expect(
        "save B stalled behind save A: the working-index lock is being held \
         across A's embedder call",
    );
    b_result.expect("save B must succeed");

    // And once released, A finishes correctly too — the lock still fully
    // protects the index read-modify-write it exists for.
    let a_result = a_done_rx
        .recv_timeout(STEP)
        .expect("save A must finish after its embedder is released");
    a_result.expect("save A must succeed");
    join_a.join().expect("thread A must not panic");
    join_b.join().expect("thread B must not panic");
    assert_saved_and_indexed(&svc_a, "proj-a", "goal a");
    assert_saved_and_indexed(&svc_b, "proj-b", "goal b");
}

/// Both the fact and its index entry must be intact after the race.
fn assert_saved_and_indexed<E: Embedder>(svc: &MemoryService<E>, project: &str, goal: &str) {
    let loaded = svc
        .load_working_context(project, "s1")
        .expect("load must not error")
        .expect("the saved working context must be present");
    assert_eq!(loaded.goal.as_deref(), Some(goal));
    let sessions = svc
        .list_working_contexts(project)
        .expect("the index must list the saved session");
    assert_eq!(sessions.len(), 1, "exactly one session in {project}");
    assert_eq!(sessions[0].session, "s1");
}
