//! GATE 7 — the rebuild itself (#1762, PR C2b).
//!
//! Everything before this file proves the PARTS: facts read out whole and go
//! back whole, edges likewise, the journal can only advance. This file proves
//! the PASS — the loop that drives those parts over a real store, checkpoints
//! after every batch, survives being killed at the worst moment, and lands a
//! destination that re-reads identical to the source.
//!
//! The interruption tests use an injected stop rather than a real kill because
//! the property under test is WHERE the pass can stop, not whether the OS can
//! stop it. The stop fires after a batch is reinserted and BEFORE its
//! checkpoint is journalled — the widest window a crash can hit — so the
//! resume must replay that batch and tolerate its collisions.

use super::*;
use crate::embedder::HashEmbedder;
use crate::storage::NativeStore;
use crate::MemoryStore;
use std::collections::BTreeMap;
use velesdb_core::agent::AgentMemory;

/// Source facts, deliberately more than one batch at [`BATCH`].
const SEEDED: u64 = 7;
/// Small enough that the walk takes several batches, so a checkpoint exists
/// between two of them for the interruption test to land in.
const BATCH: usize = 3;
const NEW_DIM: usize = 8;

fn open_pair(
    dir: &std::path::Path,
    dimension: usize,
) -> (std::sync::Arc<velesdb_core::Database>, AgentMemory) {
    let db = std::sync::Arc::new(velesdb_core::Database::open(dir).expect("open db"));
    let memory =
        AgentMemory::with_dimension(std::sync::Arc::clone(&db), dimension).expect("agent memory");
    (db, memory)
}

/// A seeded source: SEEDED facts with metadata, one TTL'd fact, and two edges,
/// one of which carries properties.
fn seeded_source() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open source");
        for id in 1..=SEEDED {
            store
                .store_with_metadata(
                    id,
                    &format!("fact number {id}"),
                    &EMBEDDING,
                    &meta(&[("topic", serde_json::Value::from("rebuild"))]),
                )
                .expect("seed");
        }
        store.relate(1, 2, "mentions").expect("edge");
    }
    // One edge with properties, through the core API the reduced trait hides.
    let (_db, memory) = open_pair(dir.path(), DIM);
    let mut props = serde_json::Map::new();
    props.insert("weight".to_owned(), serde_json::Value::from(0.5));
    memory
        .semantic()
        .relate(2, 3, "supports", Some(&props))
        .expect("edge with properties");
    dir
}

/// An empty destination sized for the target embedder, plus a journal
/// workspace, plus the initial `Prepared` journal entry.
fn prepared_destination(
    state_template: &MigrationState,
) -> (tempfile::TempDir, tempfile::TempDir, MigrationLock) {
    let dest = tempfile::tempdir().expect("destination");
    {
        let _store = NativeStore::open(dest.path(), NEW_DIM).expect("create destination");
    }
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = MigrationLock::acquire(workspace.path(), "rebuild-test").expect("lock");
    state_template
        .write(workspace.path(), &lock)
        .expect("initial journal entry");
    (dest, workspace, lock)
}

fn fresh_state() -> MigrationState {
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: super::state_persistence::VALID_FINGERPRINT.to_owned(),
        target_model: "hash".to_owned(),
        target_dimension: NEW_DIM,
        progress: AGENT_COLLECTIONS
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    CollectionProgress::Facts { cursor: None },
                )
            })
            .collect(),
    }
}

/// Ids and payloads of every live fact, keyed for comparison.
fn contents(dir: &std::path::Path) -> BTreeMap<u64, serde_json::Value> {
    super::preservation::read_out(dir, "_semantic_memory")
        .iter()
        .map(|fact| {
            (
                fact.id,
                serde_json::from_str(&fact.payload).expect("payload json"),
            )
        })
        .collect()
}

fn edge_tuples(dir: &std::path::Path, dimension: usize) -> std::collections::BTreeSet<String> {
    let (db, memory) = open_pair(dir, dimension);
    super::super::export_edges_verified(&memory, &db, "_semantic_memory", 1024)
        .expect("edge export")
        .iter()
        .map(|edge| {
            format!(
                "{}:{}:{}:{}:{:?}",
                edge.id(),
                edge.source(),
                edge.target(),
                edge.label(),
                edge.properties().iter().collect::<BTreeMap<_, _>>()
            )
        })
        .collect()
}

/// Open both stores, run one pass, and drop every handle before returning —
/// the store's single-writer flock means any verification must reopen, so no
/// handle may outlive the run it served.
fn drive(
    source: &std::path::Path,
    dest: &std::path::Path,
    dest_dim: usize,
    journal: (&std::path::Path, &MigrationLock),
    state: &mut MigrationState,
    policy: &super::super::VectorPolicy<'_>,
    stop: Option<u64>,
) -> Result<super::super::RebuildOutcome, crate::MemoryError> {
    let (source_db, source_memory) = open_pair(source, DIM);
    let (dest_db, dest_memory) = open_pair(dest, dest_dim);
    super::super::rebuild_with_stop(
        &super::super::RebuildSource {
            db: &source_db,
            memory: &source_memory,
        },
        &super::super::RebuildDestination {
            db: &dest_db,
            memory: &dest_memory,
        },
        state,
        &super::super::RebuildJournal {
            workspace: journal.0,
            lock: journal.1,
        },
        policy,
        BATCH,
        stop,
    )
}

#[test]
fn a_full_rebuild_lands_every_fact_and_edge_and_journals_completion() {
    let source = seeded_source();
    let mut state = fresh_state();
    let (dest, workspace, lock) = prepared_destination(&state);

    let embedder = HashEmbedder::new(NEW_DIM);
    let outcome = drive(
        source.path(),
        dest.path(),
        NEW_DIM,
        (workspace.path(), &lock),
        &mut state,
        &super::super::VectorPolicy::Reembed(&embedder),
        None,
    )
    .expect("rebuild");

    assert_eq!(
        outcome.facts, SEEDED,
        "every live fact must land exactly once on a clean run"
    );
    assert_eq!(
        outcome.collisions, 0,
        "a clean run replays nothing, so a collision here means the walk \
         re-read a fact it had already written"
    );
    assert_eq!(outcome.edges, 2, "both edges must land");

    assert_eq!(
        contents(dest.path()),
        contents(source.path()),
        "the destination must hold the same facts, ids and payloads included"
    );
    assert_eq!(
        edge_tuples(dest.path(), NEW_DIM),
        edge_tuples(source.path(), DIM),
        "the destination must hold the same edge tuples, properties included"
    );

    // The journal on disk — not the in-memory copy — must say Complete for
    // every collection: a resume reads the disk.
    let journalled = MigrationState::read(workspace.path())
        .expect("read journal")
        .expect("journal exists");
    assert!(
        journalled
            .progress
            .values()
            .all(|p| *p == CollectionProgress::Complete),
        "the journal must record completion, got {:?}",
        journalled.progress
    );
}

#[test]
fn an_interrupted_rebuild_resumes_replaying_only_the_unjournalled_batch() {
    let source = seeded_source();
    let state = fresh_state();
    let (dest, workspace, lock) = prepared_destination(&state);
    let embedder = HashEmbedder::new(NEW_DIM);
    let policy = super::super::VectorPolicy::Reembed(&embedder);

    // First run: killed after the FIRST batch is written to the destination
    // but before its checkpoint reaches the journal — the widest crash window.
    let interrupted = drive(
        source.path(),
        dest.path(),
        NEW_DIM,
        (workspace.path(), &lock),
        &mut state.clone(),
        &policy,
        Some(1),
    )
    .expect_err("the injected stop must surface as an interruption");
    assert!(
        interrupted.to_string().contains("interrupted"),
        "the stop must be reported as an interruption, got: {interrupted}"
    );

    // What the journal knows is LESS than what the destination holds: that is
    // the crash window, and the positive control for the replay below.
    let journalled = MigrationState::read(workspace.path())
        .expect("read journal")
        .expect("journal exists");
    let semantic = journalled.progress.get("_semantic_memory").copied();
    assert_eq!(
        semantic,
        Some(CollectionProgress::Facts { cursor: None }),
        "the interrupted batch must NOT be journalled — if it were, the resume \
         below would replay nothing and this test would prove nothing"
    );
    assert!(
        !contents(dest.path()).is_empty(),
        "positive control: the destination must already hold the batch the \
         journal does not know about"
    );

    // Resume from the journal on disk, exactly as a fresh process would.
    let mut resumed = journalled;
    let outcome = drive(
        source.path(),
        dest.path(),
        NEW_DIM,
        (workspace.path(), &lock),
        &mut resumed,
        &policy,
        None,
    )
    .expect("resume");

    assert!(
        outcome.collisions > 0,
        "positive control: the resume must have replayed the unjournalled \
         batch and met its ids; zero collisions would mean the crash window \
         was never exercised"
    );
    assert_eq!(
        contents(dest.path()),
        contents(source.path()),
        "after the resume the destination must hold every fact exactly once"
    );
    assert_eq!(
        edge_tuples(dest.path(), NEW_DIM),
        edge_tuples(source.path(), DIM),
        "and every edge"
    );
}

#[test]
fn reuse_writes_the_source_vectors_verbatim() {
    let source = seeded_source();
    let mut state = fresh_state();
    state.target_dimension = DIM;
    let dest = tempfile::tempdir().expect("destination");
    {
        let _store = NativeStore::open(dest.path(), DIM).expect("create destination");
    }
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = MigrationLock::acquire(workspace.path(), "reuse-test").expect("lock");
    state.write(workspace.path(), &lock).expect("initial entry");

    drive(
        source.path(),
        dest.path(),
        DIM,
        (workspace.path(), &lock),
        &mut state,
        &super::super::VectorPolicy::Reuse,
        None,
    )
    .expect("rebuild under reuse");

    let source_vectors: BTreeMap<u64, Vec<f32>> =
        super::preservation::read_out(source.path(), "_semantic_memory")
            .into_iter()
            .map(|fact| (fact.id, fact.source_vector))
            .collect();
    let dest_vectors: BTreeMap<u64, Vec<f32>> =
        super::preservation::read_out(dest.path(), "_semantic_memory")
            .into_iter()
            .map(|fact| (fact.id, fact.source_vector))
            .collect();
    assert_eq!(
        dest_vectors, source_vectors,
        "under reuse the destination must carry the source's vectors verbatim; \
         anything else is a re-embedding the regime did not license"
    );
}

#[test]
fn a_fact_without_content_text_fails_reembed_by_name_and_passes_reuse() {
    let source = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(source.path(), DIM).expect("open source");
        store
            .store_with_metadata(1, "an ordinary fact", &EMBEDDING, &meta(&[]))
            .expect("seed");
    }
    {
        // A payload with no `content` key, written straight to the collection —
        // the shape an external writer can produce and `reembed` cannot serve.
        let db = velesdb_core::Database::open(source.path()).expect("open");
        let any = db
            .get_any_collection("_semantic_memory")
            .expect("collection");
        any.upsert(vec![velesdb_core::Point::new(
            2,
            EMBEDDING.to_vec(),
            Some(serde_json::json!({ "note": "no content key here" })),
        )])
        .expect("raw upsert");
    }

    let embedder = HashEmbedder::new(NEW_DIM);
    let run = |policy: &super::super::VectorPolicy<'_>, dimension: usize| {
        let mut state = fresh_state();
        state.target_dimension = dimension;
        let dest = tempfile::tempdir().expect("destination");
        {
            let _store = NativeStore::open(dest.path(), dimension).expect("create destination");
        }
        let workspace = tempfile::tempdir().expect("workspace");
        let lock = MigrationLock::acquire(workspace.path(), "content-test").expect("lock");
        state.write(workspace.path(), &lock).expect("initial entry");
        let (source_db, source_memory) = open_pair(source.path(), DIM);
        let (dest_db, dest_memory) = open_pair(dest.path(), dimension);
        super::super::rebuild(
            &super::super::RebuildSource {
                db: &source_db,
                memory: &source_memory,
            },
            &super::super::RebuildDestination {
                db: &dest_db,
                memory: &dest_memory,
            },
            &mut state,
            &super::super::RebuildJournal {
                workspace: workspace.path(),
                lock: &lock,
            },
            policy,
            BATCH,
        )
    };

    let refusal = run(&super::super::VectorPolicy::Reembed(&embedder), NEW_DIM)
        .expect_err("a fact `reembed` cannot serve must stop the pass, not be skipped");
    assert!(
        refusal.to_string().contains('2'),
        "the refusal must name the fact: {refusal}"
    );

    run(&super::super::VectorPolicy::Reuse, DIM)
        .expect("under reuse the same store rebuilds — content is never needed");
}
