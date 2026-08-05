//! GATE 8 — the operator's path (#1762, PR C2b).
//!
//! [`rebuild`](super::super::rebuild) is proven on handles a test staged by
//! hand. An operator has none of that: they have a store path, a destination
//! path and an embedder. `execute` is the distance between the two — diagnose,
//! resolve the regime, stage the destination and the journal, take the lock,
//! run the pass, release the lock — and each of these tests pins one refusal
//! or one guarantee of that staging, because staging is exactly where a
//! "proven" pass gets driven with the wrong inputs.

use super::*;
use crate::embedder::HashEmbedder;
use crate::storage::NativeStore;
use crate::MemoryStore;
use std::collections::BTreeMap;

const NEW_DIM: usize = 8;
const SEEDED: u64 = 5;

/// A root holding the source store at `store/`, leaving room for a sibling
/// destination on the same filesystem — the layout the switch (C3) requires.
fn root_with_source() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("root");
    let store = root.path().join("store");
    std::fs::create_dir(&store).expect("mkdir store");
    {
        let native = NativeStore::open(&store, DIM).expect("open source");
        for id in 1..=SEEDED {
            native
                .store_with_metadata(id, &format!("fact number {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed");
        }
        native.relate(1, 2, "mentions").expect("edge");
    }
    root
}

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

fn run_with(
    root: &std::path::Path,
    embedder: &dyn crate::Embedder,
) -> Result<super::super::ExecuteOutcome, crate::MemoryError> {
    super::super::execute(
        &root.join("store"),
        root,
        &TargetContract::automatic("hash", NEW_DIM),
        &root.join("rebuilt"),
        embedder,
        1024,
    )
}

fn run(root: &std::path::Path) -> Result<super::super::ExecuteOutcome, crate::MemoryError> {
    run_with(root, &HashEmbedder::new(NEW_DIM))
}

/// The same dimension, the same claimed identity, DIFFERENT vectors — what an
/// in-place model update looks like from the outside.
struct DriftedEmbedder(HashEmbedder);

impl crate::Embedder for DriftedEmbedder {
    fn dimension(&self) -> usize {
        self.0.dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, crate::EmbedError> {
        let mut vector = self.0.embed(text)?;
        vector[0] += 1.0;
        Ok(vector)
    }
}

#[test]
fn execute_rebuilds_into_the_destination_and_releases_the_lock() {
    let root = root_with_source();
    let outcome = run(root.path()).expect("execute");

    assert_eq!(outcome.rebuild.facts, SEEDED, "every fact must land");
    assert_eq!(outcome.rebuild.edges, 1, "the edge must land");
    assert_eq!(
        contents(&outcome.destination),
        contents(&root.path().join("store")),
        "the destination must hold the same facts as the source"
    );

    let journalled = MigrationState::read(&outcome.workspace)
        .expect("read journal")
        .expect("journal exists");
    assert_eq!(
        journalled.phase,
        Phase::Prepared,
        "execute stops at Prepared"
    );
    assert!(
        journalled
            .progress
            .values()
            .all(|p| *p == CollectionProgress::Complete),
        "the journal must record completion, got {:?}",
        journalled.progress
    );

    // The lock must be RELEASED, not dropped: a dropped lock leaves canonical
    // evidence that blocks every later run until an operator removes it by
    // hand, and there is nothing to acknowledge after a clean success.
    MigrationLock::acquire(&outcome.workspace, "the-next-run")
        .expect("a clean success must leave the workspace reacquirable")
        .release()
        .expect("release probe lock");
}

#[test]
fn a_second_execute_resumes_the_journal_and_replays_nothing() {
    let root = root_with_source();
    let first = run(root.path()).expect("first execute");
    let second = run(root.path()).expect("second execute must resume, not refuse");

    assert_eq!(
        (
            second.rebuild.facts,
            second.rebuild.collisions,
            second.rebuild.edges
        ),
        (0, 0, 0),
        "a journal at Complete has nothing to replay; any nonzero count means \
         the resume re-did work the journal already recorded"
    );
    assert_eq!(
        contents(&second.destination),
        contents(&first.destination),
        "the destination must be untouched by the no-op resume"
    );
}

#[test]
fn a_resume_across_an_embedder_updated_in_place_is_refused_by_its_witness() {
    // `may_resume` compares the model's NAME, and a name is a claim: `ollama
    // pull` replaces a model's weights under the same identifier. A resume
    // across such an update would leave the replayed batches with run-one
    // vectors and write run-two vectors after them — one store, one recorded
    // model, two incompatible vector spaces. The journal therefore carries a
    // WITNESS of what the embedder produces, and this is the test that the
    // witness actually refuses.
    let root = root_with_source();
    run(root.path()).expect("first execute");

    let refusal = run_with(root.path(), &DriftedEmbedder(HashEmbedder::new(NEW_DIM)))
        .expect_err("same name, same dimension, different vectors must refuse the resume");
    let message = refusal.to_string();
    assert!(
        message.contains("witness") && message.contains("fresh migration"),
        "the refusal must name the witness and the recovery: {message}"
    );

    // Positive control: the UNCHANGED embedder still resumes — so the refusal
    // above was about the vectors, not about resuming per se.
    run(root.path()).expect("the same embedder must still resume");
}

#[test]
fn a_nonempty_destination_without_a_journal_is_refused() {
    let root = root_with_source();
    let destination = root.path().join("rebuilt");
    std::fs::create_dir(&destination).expect("mkdir destination");
    std::fs::write(destination.join("stray.txt"), b"not a rebuild").expect("stray file");

    let refusal =
        run(root.path()).expect_err("rebuilding into a directory that already holds something");
    let message = refusal.to_string();
    assert!(
        message.contains("rebuilt"),
        "the refusal must name the destination: {message}"
    );
    assert!(
        std::fs::read(destination.join("stray.txt")).is_ok(),
        "a refusal must not have touched the directory it refused"
    );
}

#[test]
fn settling_a_store_is_idempotent_which_the_resume_fingerprint_rests_on() {
    // Measured 2026-08-05: the FIRST open of a store compacts its WAL —
    // `vectors.wal` drains into materialised index files — so the on-disk tree
    // after an open is not the tree before it. `execute` therefore settles the
    // source with one open BEFORE fingerprinting, and journals the settled
    // fingerprint. That design is sound only while a SECOND open changes
    // nothing, which is exactly what this test pins. (That the first open
    // changes the tree is incidental engine behaviour and deliberately NOT
    // asserted: if the engine ever stops compacting on open, the settle
    // becomes a harmless no-op and this test must keep passing.)
    let root = root_with_source();
    let store = root.path().join("store");
    {
        let _db = velesdb_core::Database::open(&store).expect("first open settles");
    }
    let settled = super::super::fingerprint(&store).expect("settled fingerprint");
    {
        let db = velesdb_core::Database::open(&store).expect("second open");
        let _ = super::super::enumerate_by_cursor(&db, "_semantic_memory", 16).expect("walk");
    }
    assert_eq!(
        super::super::fingerprint(&store).expect("fingerprint after reopen"),
        settled,
        "a second open (with a full cursor walk) must leave the settled tree \
         byte-identical; if this ever fails, the journalled fingerprint can no \
         longer prove the source unchanged across a resume, and `execute`'s \
         settle-then-fingerprint design is void"
    );
}

#[test]
fn a_refusing_regime_stops_execute_before_anything_is_created() {
    let root = root_with_source();
    let embedder = HashEmbedder::new(NEW_DIM);
    // `reuse` against a store with no recorded provenance: the resolution is
    // Refuse, and execute must relay it rather than "helpfully" re-embedding.
    let refusal = super::super::execute(
        &root.path().join("store"),
        root.path(),
        &TargetContract {
            model: "hash".to_owned(),
            dimension: NEW_DIM,
            strategy: Strategy::Reuse,
        },
        &root.path().join("rebuilt"),
        &embedder,
        1024,
    )
    .expect_err("a refused regime must refuse the execution");
    let message = refusal.to_string();
    assert!(
        message.contains("reuse"),
        "the refusal must name the requested regime: {message}"
    );
    assert!(
        !root.path().join("rebuilt").exists(),
        "a regime refusal must not leave a half-created destination behind"
    );
}
