use super::*;

// ---------------------------------------------------------------------------
// GATE 4 — preservation
//
// Reading every fact out is half the question. The other half is whether it
// goes back the SAME: same id, same content, same ordinary and RESERVED
// metadata, the same absolute instant of expiry, the same edges between the
// same endpoints. Every comparison below is against the SOURCE's own values —
// never against a constant this file made up, which would only prove the file
// agrees with itself.
// ---------------------------------------------------------------------------

/// The width the new embedder produces — deliberately NOT [`DIM`], because the
/// whole migration exists to move between two widths and a destination sized
/// like the source would hide every place the old vector leaked through.
const NEW_DIM: usize = 8;
pub(super) const NEW_EMBEDDING: [f32; NEW_DIM] = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

/// An empty destination store, sized for the NEW embedder.
pub(super) fn destination() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(dir.path(), NEW_DIM).expect("open destination");
    }
    dir
}

/// Walk `collection` out of the store at `dir`, by cursor.
pub(super) fn read_out(dir: &std::path::Path, collection: &str) -> Vec<RawFact> {
    let db = velesdb_core::Database::open(dir).expect("open source");
    super::enumerate_by_cursor(&db, collection, 1024).expect("cursor walk")
}

#[test]
fn a_fact_round_trips_with_id_metadata_and_ttl() {
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    assert!(!out.is_empty(), "positive control: the source must be read");

    let dest = destination();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        for fact in &out {
            assert_eq!(
                reinsert(&db, "_semantic_memory", fact, &NEW_EMBEDDING).expect("reinsert"),
                Reinsertion::Inserted,
                "an empty destination must accept every fact; a collision here \
                 would mean the ids are not what the walk reported"
            );
        }
    }

    let back = read_out(dest.path(), "_semantic_memory");
    let by_id = |facts: &[RawFact]| -> std::collections::BTreeMap<u64, Value> {
        facts
            .iter()
            .map(|f| (f.id, serde_json::from_str(&f.payload).expect("json")))
            .collect()
    };
    let (source_facts, dest_facts) = (by_id(&out), by_id(&back));

    assert_eq!(
        source_facts.keys().collect::<Vec<_>>(),
        dest_facts.keys().collect::<Vec<_>>(),
        "every id must survive verbatim — a renumbered fact severs its edges, \
         its hub and the working-context index that address it BY id"
    );
    assert_eq!(
        source_facts, dest_facts,
        "content, ordinary metadata and RESERVED metadata must come back \
         byte-identical; a stripped `_veles_*` key is a fact the rebuild quietly \
         demoted"
    );

    // ...and the expiry specifically, because it is the one field a plausible
    // implementation would RECOMPUTE from a duration and silently extend.
    let ttl_source = source_facts
        .get(&100)
        .and_then(|p| p.get("_veles_expires_at"))
        .expect("the source ttl fact carries an absolute expiry");
    assert_eq!(
        dest_facts
            .get(&100)
            .and_then(|p| p.get("_veles_expires_at")),
        Some(ttl_source),
        "the expiry must be the SAME absolute instant, not the same duration \
         measured from migration time"
    );
}

#[test]
fn a_collision_has_an_explicit_result() {
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    let first = out.iter().find(|f| f.id == 1).expect("fact 1");

    let dest = destination();
    let db = velesdb_core::Database::open(dest.path()).expect("open destination");

    // The positive control comes first: the same call on a free id must succeed,
    // or "collision" below would just be this function failing at everything.
    assert_eq!(
        reinsert(&db, "_semantic_memory", first, &NEW_EMBEDDING).expect("first insert"),
        Reinsertion::Inserted,
        "a free id must accept the fact"
    );

    // Now the same id, carrying DIFFERENT content — the case where a silent
    // overwrite would destroy a fact and report success.
    let intruder = RawFact {
        id: 1,
        payload: serde_json::json!({ "content": "an intruder that must not land" }).to_string(),
    };
    let outcome = reinsert(&db, "_semantic_memory", &intruder, &NEW_EMBEDDING).expect("second");
    match &outcome {
        Reinsertion::Collision { existing } => {
            let stored: Value = serde_json::from_str(existing).expect("json");
            assert_eq!(
                stored.get("content").and_then(Value::as_str),
                Some("fact number 1"),
                "a collision must report what is ALREADY there, so the caller can \
                 tell a re-run from a genuine clash"
            );
        }
        Reinsertion::Inserted => panic!(
            "the second write reported success — meaning `upsert` overwrote fact 1 \
             without a word, which is exactly how a rebuild destroys what it is \
             preserving"
        ),
    }

    // And nothing was written: the fact under that id is untouched.
    drop(db);
    let back = read_out(dest.path(), "_semantic_memory");
    let stored: Value = serde_json::from_str(&back[0].payload).expect("json");
    assert_eq!(
        stored.get("content").and_then(Value::as_str),
        Some("fact number 1"),
        "a refused collision must leave the destination exactly as it was"
    );
}

/// Write a point straight into the collection, expiry included.
///
/// NO published API produces an already-expired fact: `store_with_metadata`
/// STRIPS `_veles_expires_at` out of caller metadata (`build_payload`), and
/// `store_with_ttl(_, 0)` DELETES the fact rather than expiring it. An expired
/// fact is only ever reached by time passing — which a test cannot wait for and
/// must not fake with a sleep. So the fixture writes the on-disk state such a
/// fact actually has: the engine never rewrites a payload when its expiry
/// passes, it filters at read time.
fn seed_raw(dir: &std::path::Path, id: u64, content: &str, expires_at: Option<u64>) {
    let db = velesdb_core::Database::open(dir).expect("open");
    let any = db
        .get_any_collection("_semantic_memory")
        .expect("collection exists");
    let mut payload = serde_json::Map::new();
    payload.insert("content".to_owned(), Value::from(content));
    if let Some(exp) = expires_at {
        payload.insert("_veles_expires_at".to_owned(), Value::from(exp));
    }
    any.upsert(vec![velesdb_core::Point::new(
        id,
        EMBEDDING.to_vec(),
        Some(Value::Object(payload)),
    )])
    .expect("upsert");
}

#[test]
fn expired_points_are_not_resurrected() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        store
            .store_with_metadata(1, "a live fact", &EMBEDDING, &meta(&[]))
            .expect("seed live");
    }
    seed_raw(
        dir.path(),
        2,
        "a fact whose time has passed",
        Some(1_000_000),
    );

    let out = read_out(dir.path(), "_semantic_memory");
    let ids: BTreeSet<u64> = out.iter().map(|f| f.id).collect();
    assert!(
        ids.contains(&1),
        "positive control: the LIVE fact must come back, or this test proves only \
         that the walk returns nothing"
    );
    assert!(
        !ids.contains(&2),
        "an already-expired fact must not be exported; a rebuild that carried it \
         would resurrect a fact the store had already retired, and the new store \
         would hand it back to the caller"
    );

    // And it really is the expiry that excluded it, not the raw write path: the
    // SAME fixture with a FUTURE expiry does come back.
    let future = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(future.path(), DIM).expect("open store");
    }
    seed_raw(
        future.path(),
        2,
        "a fact whose time has not passed",
        Some(4_000_000_000),
    );
    assert!(
        read_out(future.path(), "_semantic_memory")
            .iter()
            .any(|f| f.id == 2),
        "a fact under a FUTURE expiry must be exported — otherwise the exclusion \
         above was about the write, not about the expiry"
    );
}

#[test]
fn cursor_scan_survives_reorder_for_locality() {
    // `reorder_for_locality` rearranges the physical layout. A walk that paged
    // by POSITION would silently change what it returns; a cursor keyed on the
    // id must not. The ids are scrambled and non-contiguous so that physical
    // order and id order cannot coincide by luck.
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in SCRAMBLED {
            store
                .store_with_metadata(*id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed");
        }
    }
    let expected: BTreeSet<u64> = SCRAMBLED.iter().copied().collect();

    let before = read_out(dir.path(), "_semantic_memory");
    let before_ids: Vec<u64> = before.iter().map(|f| f.id).collect();
    assert_eq!(
        before_ids.iter().copied().collect::<BTreeSet<u64>>(),
        expected,
        "positive control: the walk must be complete BEFORE the reorder, or the \
         comparison after it means nothing"
    );

    {
        let db = velesdb_core::Database::open(dir.path()).expect("open");
        db.get_vector_collection("_semantic_memory")
            .expect("the seeded collection is a vector collection")
            .reorder_for_locality()
            .expect("reorder");
    }

    let after = read_out(dir.path(), "_semantic_memory");
    let after_ids: Vec<u64> = after.iter().map(|f| f.id).collect();
    assert_eq!(
        after_ids.iter().copied().collect::<BTreeSet<u64>>(),
        expected,
        "the reorder dropped or duplicated facts under the cursor walk"
    );
    let mut sorted = after_ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        after_ids, sorted,
        "the cursor is keyed on the id and must stay ASCENDING through a \
         reorder — an order that follows the physical layout is one a checkpoint \
         cannot resume from"
    );
    assert_eq!(
        after, before,
        "the reorder must not change a single payload either"
    );
}

type StoredEdge = (u64, u64, u64, String);
const EDGE_TRIPLETS: &[(u64, u64, &str)] = &[
    (1, 2, "mentions"),
    (2, 1, "mentions"),
    (1, 2, "contradicts"),
    (1, 3, "mentions"),
];

fn source_with_edges() -> (tempfile::TempDir, Vec<StoredEdge>) {
    let source = tempfile::tempdir().expect("tempdir");
    let mut expected = Vec::new();
    {
        let store = NativeStore::open(source.path(), DIM).expect("open source");
        for id in 1..=3_u64 {
            store
                .store_with_metadata(id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed");
        }
        for &(from, to, label) in EDGE_TRIPLETS {
            let edge_id = store.relate(from, to, label).expect("relate");
            expected.push((edge_id, from, to, label.to_owned()));
        }
    }
    assert_eq!(
        expected
            .iter()
            .map(|(id, ..)| *id)
            .collect::<BTreeSet<u64>>()
            .len(),
        EDGE_TRIPLETS.len(),
        "positive control: the four triplets must yield four DISTINCT edge ids, \
         or the comparison below cannot tell them apart"
    );
    (source, expected)
}

fn rebuild_source_facts(source: &std::path::Path) -> tempfile::TempDir {
    let out = read_out(source, "_semantic_memory");
    let dest = destination();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        for fact in &out {
            reinsert(&db, "_semantic_memory", fact, &NEW_EMBEDDING).expect("reinsert");
        }
    }
    dest
}

fn rebuild_edges_and_check_endpoints(
    destination: &std::path::Path,
    expected: &[StoredEdge],
) -> Vec<StoredEdge> {
    let store = NativeStore::open(destination, NEW_DIM).expect("open destination store");
    let mut rebuilt = Vec::new();
    for &(from, to, label) in EDGE_TRIPLETS {
        let edge_id = store.relate(from, to, label).expect("re-relate");
        rebuilt.push((edge_id, from, to, label.to_owned()));
    }
    for (edge_id, from, to, label) in expected {
        let found = store
            .relations(*from)
            .expect("relations")
            .into_iter()
            .find(|edge| edge.id == *edge_id)
            .unwrap_or_else(|| {
                panic!("edge {edge_id} ({from}->{to} :{label}) is missing from the destination")
            });
        assert_eq!(
            (found.from, found.to, found.relation.as_str()),
            (*from, *to, label.as_str()),
            "the edge came back under the right id but the wrong endpoints or label"
        );
    }
    rebuilt
}

#[test]
fn edge_ids_and_endpoints_survive_the_round_trip() {
    // Both directions and two labels prove ids are derived from the complete,
    // ordered triplet rather than only from the endpoints.
    let (source, expected) = source_with_edges();
    let dest = rebuild_source_facts(source.path());
    let rebuilt = rebuild_edges_and_check_endpoints(dest.path(), &expected);
    assert_eq!(
        rebuilt, expected,
        "re-relating the same triplet must yield the SAME edge id; a different \
         one severs every reference the store holds to that edge"
    );
}

fn facts_by_id(facts: &[RawFact]) -> std::collections::BTreeMap<u64, Value> {
    facts
        .iter()
        .map(|fact| {
            (
                fact.id,
                serde_json::from_str(&fact.payload).expect("payload JSON"),
            )
        })
        .collect()
}

fn reinsert_clean_batch(out: &[RawFact], batch: &[(RawFact, Vec<f32>)]) -> tempfile::TempDir {
    let dest = destination();
    let db = velesdb_core::Database::open(dest.path()).expect("open destination");
    let outcome = super::reinsert_batch(&db, "_semantic_memory", batch).expect("batch");
    assert_eq!(
        outcome.inserted,
        out.len() as u64,
        "every fact of the batch must land; a short count is the loss this test exists to catch"
    );
    assert!(
        outcome.collisions.is_empty(),
        "an empty destination has nothing to collide with, got {:?}",
        outcome.collisions
    );
    drop(db);
    dest
}

fn reinsert_batch_with_one_collision(
    out: &[RawFact],
    batch: &[(RawFact, Vec<f32>)],
) -> tempfile::TempDir {
    let mixed = destination();
    let db = velesdb_core::Database::open(mixed.path()).expect("open");
    let first = out
        .iter()
        .find(|fact| fact.id == 1)
        .expect("fact 1")
        .clone();
    super::reinsert_batch(&db, "_semantic_memory", &[(first, NEW_EMBEDDING.to_vec())])
        .expect("seed one");
    let outcome = super::reinsert_batch(&db, "_semantic_memory", batch).expect("batch");
    assert_eq!(
        outcome.collisions,
        vec![1],
        "the occupied id must be reported, and only it"
    );
    assert_eq!(
        outcome.inserted,
        out.len() as u64 - 1,
        "one collision must not cost the batch its other facts"
    );
    drop(db);
    mixed
}

#[test]
fn a_batch_reinsertion_loses_no_id_reserved_key_or_ttl() {
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    assert!(
        out.len() > 1,
        "positive control: a batch needs several facts"
    );
    let batch: Vec<(RawFact, Vec<f32>)> = out
        .iter()
        .map(|fact| (fact.clone(), NEW_EMBEDDING.to_vec()))
        .collect();

    let dest = reinsert_clean_batch(&out, &batch);
    assert_eq!(
        facts_by_id(&read_out(dest.path(), "_semantic_memory")),
        facts_by_id(&out),
        "a batched write must preserve every id, reserved key and absolute expiry"
    );

    let mixed = reinsert_batch_with_one_collision(&out, &batch);
    assert_eq!(
        facts_by_id(&read_out(mixed.path(), "_semantic_memory")),
        facts_by_id(&out),
        "the collided fact must be the one already there, unchanged"
    );
}
