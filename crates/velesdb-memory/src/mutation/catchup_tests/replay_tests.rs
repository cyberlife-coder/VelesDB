use std::collections::HashSet;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};

use super::{journal, FixedEmbedder, TestRig};
use crate::mutation::catchup::{CatchUpConfig, OnlineCatchUp};
use crate::storage::NativeStore;
use crate::{EmbedError, Embedder, MemoryService, Metadata};

#[test]
fn repeated_overwrites_coalesce_and_apply_the_latest_source_state() {
    let rig = TestRig::new();
    let id = rig.source.remember("alpha", &[], None).expect("alpha");
    let copy = rig.start();
    copy.copy_base().expect("base copy");
    for version in 1..=3 {
        let mut metadata = Metadata::new();
        metadata.insert("version".to_owned(), json!(version));
        rig.source
            .remember("alpha", &[], Some(&metadata))
            .expect("overwrite");
    }

    let progress = copy.catch_up_batch().expect("catch up");
    assert_eq!(progress.records, 3);
    assert_eq!(progress.dirty_keys, 1);
    assert_eq!(progress.backlog, 0);
    assert_eq!(
        rig.destination
            .migration_payload(id)
            .expect("destination payload"),
        rig.source
            .migration_store()
            .migration_payload(id)
            .expect("source payload")
    );
    copy.finish().expect("finish");
}

#[test]
fn deletion_and_edge_replacement_converge_idempotently() {
    let rig = TestRig::new();
    let from = rig.source.remember("from", &[], None).expect("from");
    let old = rig.source.remember("old", &[], None).expect("old");
    let new = rig.source.remember("new", &[], None).expect("new");
    let deleted = rig.source.remember("deleted", &[], None).expect("deleted");
    rig.source.relate(from, old, "uses").expect("old edge");
    let copy = rig.start();
    copy.copy_base().expect("base copy");

    rig.source.forget(deleted).expect("delete");
    rig.source.unrelate(from, old, "uses").expect("unrelate");
    rig.source.relate(from, new, "uses").expect("new edge");
    let progress = copy.catch_up_batch().expect("catch up");

    assert_eq!(progress.records, 3);
    assert_eq!(progress.dirty_keys, 2);
    assert!(!rig
        .destination
        .migration_contains(deleted)
        .expect("deleted absent"));
    assert_eq!(
        rig.destination
            .migration_live_edges(from, 8)
            .expect("destination edges"),
        rig.source
            .migration_store()
            .migration_live_edges(from, 8)
            .expect("source edges")
    );
    copy.finish().expect("finish");
}

#[test]
fn fuzzy_cursor_plus_bounded_replay_converges_to_current_state() {
    let mut rig = TestRig::new();
    rig.config.fact_batch = 2;
    rig.config.replay_batch = 2;
    for fact in ["seed-a", "seed-b", "seed-c", "seed-d"] {
        rig.source.remember(fact, &[], None).expect("seed");
    }
    let (first, cursor) = rig
        .source
        .migration_store()
        .migration_list(None, 2)
        .expect("first page");
    let cursor = cursor.expect("cursor");
    let existing: HashSet<u64> = live_ids(&rig.source).into_iter().collect();
    let lower = find_content(|id| id < cursor && !existing.contains(&id));
    let higher = find_content(|id| id > cursor && !existing.contains(&id));
    let deleted = first[0].id;
    let overwritten = first[1].id;
    let overwritten_content = content_of(&rig, overwritten);
    let mut lower_id = 0;
    let copy = rig.start();

    copy.copy_base_with_page_hook(|page| {
        if page != 1 {
            return Ok(());
        }
        lower_id = rig.source.remember(&lower, &[], None)?;
        rig.source.forget(deleted)?;
        let mut metadata = Metadata::new();
        metadata.insert("version".to_owned(), json!(2));
        rig.source
            .remember(&overwritten_content, &[], Some(&metadata))?;
        let higher_id = rig.source.remember(&higher, &[], None)?;
        rig.source.relate(overwritten, higher_id, "references")?;
        Ok(())
    })
    .expect("fuzzy base copy");
    assert!(!rig
        .destination
        .migration_contains(lower_id)
        .expect("lower not base-copied"));
    assert!(rig
        .destination
        .migration_contains(deleted)
        .expect("deleted still present before replay"));

    while copy.catch_up_batch().expect("catch-up batch").backlog != 0 {}
    assert_converged(&rig);
    copy.finish().expect("finish");
}

#[test]
fn replay_waits_for_an_in_flight_mutation_before_acknowledging_it() {
    let root = tempfile::tempdir().expect("root");
    let source_path = root.path().join("source");
    let destination_path = root.path().join("destination");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(Mutex::new(None));
    let source = Arc::new(
        MemoryService::open(
            &source_path,
            BlockingEmbedder {
                gate: Arc::clone(&gate),
                entered: Arc::clone(&entered),
            },
        )
        .expect("source"),
    );
    let id = source.remember("alpha", &[], None).expect("seed");
    let destination = NativeStore::open(&destination_path, 3).expect("destination");
    let journal = journal(
        &root.path().join("journal"),
        &source_path,
        &destination_path,
    );
    let target = FixedEmbedder {
        vector: vec![7.0, 8.0, 9.0],
    };
    let copy = OnlineCatchUp::start(
        source.as_ref(),
        &destination,
        &target,
        journal,
        CatchUpConfig {
            fact_batch: 8,
            replay_batch: 8,
            edge_cap: 8,
        },
    )
    .expect("start");
    copy.copy_base().expect("base copy");
    let (entered_tx, entered_rx) = mpsc::channel();
    *entered.lock() = Some(entered_tx);
    *gate.0.lock() = true;
    let mut metadata = Metadata::new();
    metadata.insert("version".to_owned(), json!(2));

    let writer_source = Arc::clone(&source);
    let writer = std::thread::spawn(move || writer_source.remember("alpha", &[], Some(&metadata)));
    entered_rx.recv().expect("mutation entered embedder");
    let release_gate = Arc::clone(&gate);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        *release_gate.0.lock() = false;
        release_gate.1.notify_all();
    });

    let progress = copy.catch_up_batch().expect("replay");
    writer.join().expect("writer thread").expect("overwrite");
    releaser.join().expect("releaser thread");

    assert_eq!(progress.records, 1);
    assert_eq!(progress.backlog, 0);
    assert_eq!(
        destination.migration_payload(id).expect("destination"),
        source
            .migration_store()
            .migration_payload(id)
            .expect("source")
    );
    copy.finish().expect("finish");
}

fn live_ids(source: &crate::MemoryService<super::FixedEmbedder>) -> Vec<u64> {
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let (page, next) = source
            .migration_store()
            .migration_list(cursor, 32)
            .expect("list");
        ids.extend(page.into_iter().map(|fact| fact.id));
        let Some(next) = next else { break };
        cursor = Some(next);
    }
    ids
}

fn assert_converged(rig: &TestRig) {
    let source_ids = live_ids(&rig.source);
    let destination_ids = list_store_ids(&rig.destination);
    assert_eq!(destination_ids, source_ids);
    for id in source_ids {
        assert_eq!(
            rig.destination.migration_payload(id).expect("destination"),
            rig.source
                .migration_store()
                .migration_payload(id)
                .expect("source")
        );
        assert_eq!(
            rig.destination.migration_live_edges(id, 8).expect("edges"),
            rig.source
                .migration_store()
                .migration_live_edges(id, 8)
                .expect("source edges")
        );
    }
}

fn list_store_ids(store: &crate::storage::NativeStore) -> Vec<u64> {
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let (page, next) = store.migration_list(cursor, 32).expect("list store");
        ids.extend(page.into_iter().map(|fact| fact.id));
        let Some(next) = next else { break };
        cursor = Some(next);
    }
    ids
}

fn find_content(predicate: impl Fn(u64) -> bool) -> String {
    (0..100_000)
        .map(|index| format!("candidate-{index}"))
        .find(|content| predicate(crate::id::stable_id(content)))
        .expect("candidate")
}

fn content_of(rig: &TestRig, id: u64) -> String {
    rig.source
        .migration_store()
        .migration_payload(id)
        .expect("payload")
        .and_then(|payload| payload.get("content").cloned())
        .and_then(|value| match value {
            Value::String(content) => Some(content),
            _ => None,
        })
        .expect("content")
}

struct BlockingEmbedder {
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl Embedder for BlockingEmbedder {
    fn dimension(&self) -> usize {
        2
    }

    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut blocked = self.gate.0.lock();
        if *blocked {
            if let Some(entered) = self.entered.lock().take() {
                let _ = entered.send(());
            }
            while *blocked {
                self.gate.1.wait(&mut blocked);
            }
        }
        Ok(vec![1.0, 2.0])
    }
}
