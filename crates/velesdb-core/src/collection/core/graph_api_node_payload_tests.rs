//! Tests for the batched node-payload path (#2153).
//!
//! The defect this covers is invisible to a result assertion: the loop and the
//! batch both store every payload and both return `Ok`. What differed was the
//! barrier count, the lock hold time, and — once a batch exists at all — what
//! happens to the label and property indexes when ids repeat or a payload is
//! rejected. So the bar here is equivalence against the single-node path,
//! asserted on observable index state rather than on return values.
//!
//! The barrier count itself is NOT asserted here, deliberately: nothing in the
//! tree counts fsyncs, and a same-process reopen reads back through the OS page
//! cache, so it cannot separate `store_batch` from `store_batch_deferred`. That
//! property is covered by the timing measurement recorded in the pull request
//! (~20x at the default `Fsync` on 500 and 2 000 nodes), not by a wall-clock
//! assertion that would be flaky in CI.

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::collection::graph::{GraphSchema, NodeType};
use crate::collection::types::Collection;
use crate::DistanceMetric;

fn graph_collection() -> (Collection, TempDir) {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let collection = Collection::create_graph_collection(
        temp_dir.path().to_path_buf(),
        "nodes",
        GraphSchema::schemaless(),
        None,
        DistanceMetric::Cosine,
    )
    .expect("test: create graph collection");
    (collection, temp_dir)
}

/// Every id a label currently resolves to, sorted — the label index as a
/// caller can observe it.
fn label_members(collection: &Collection, label: &str) -> Vec<u64> {
    collection
        .graph
        .label_index
        .read()
        .lookup(label)
        .map(|bitmap| bitmap.iter().map(u64::from).collect())
        .unwrap_or_default()
}

/// Every populated range-index key with the node ids it resolves to, read
/// through an unbounded lookup rather than counted. Compared between the loop
/// and the batch so a property left indexed against a superseded payload names
/// the node it is wrongly attached to, instead of showing up as a bare count.
fn range_index_shape(collection: &Collection) -> Vec<(String, Vec<u64>)> {
    // Scoped so the read guard is released before the sort, not held to the
    // end of the function.
    let mut shape: Vec<(String, Vec<u64>)> = {
        let indexes = collection.graph.graph_range_indexes.read();
        indexes
            .iter()
            .map(|(key, index)| {
                let mut ids = index.lookup_range(None, None);
                ids.sort_unstable();
                (key.clone(), ids)
            })
            .collect()
    };
    shape.sort();
    shape
}

/// Node ids in these tests are small; the conversion is spelled out rather
/// than cast so a pedantic wrap warning never has to be silenced.
fn age_of(node_id: u64) -> i64 {
    i64::try_from(node_id).expect("test: node id fits in i64")
}

fn person(name: &str, age: i64) -> Value {
    json!({ "_labels": ["Person"], "name": name, "age": age })
}

#[test]
fn a_batch_of_new_nodes_indexes_exactly_as_the_loop_did() {
    let payloads: Vec<(u64, Value)> = (0..32u64)
        .map(|i| (i, person(&format!("n{i}"), age_of(i))))
        .collect();

    let (looped, _looped_dir) = graph_collection();
    for (id, payload) in &payloads {
        looped
            .store_node_payload(*id, payload)
            .expect("test: single store");
    }

    let (batched, _batched_dir) = graph_collection();
    let entries: Vec<(u64, &Value)> = payloads.iter().map(|(id, p)| (*id, p)).collect();
    batched
        .store_node_payloads(&entries)
        .expect("test: batch store");

    assert_eq!(
        label_members(&batched, "Person"),
        label_members(&looped, "Person"),
        "the batch must leave the same label index the loop did"
    );
    assert_eq!(
        range_index_shape(&batched),
        range_index_shape(&looped),
        "the batch must leave the same property indexes the loop did"
    );
    for (id, payload) in &payloads {
        assert_eq!(
            batched.get_node_payload(*id).expect("test: retrieve"),
            Some(payload.clone()),
            "node {id}"
        );
    }
}

#[test]
fn updating_existing_nodes_removes_the_old_labels_and_property_entries() {
    let (looped, _looped_dir) = graph_collection();
    let (batched, _batched_dir) = graph_collection();

    // Seed both the same way, one node at a time.
    for collection in [&looped, &batched] {
        for id in 0..8u64 {
            collection
                .store_node_payload(id, &person(&format!("old{id}"), age_of(id)))
                .expect("test: seed");
        }
    }

    // Relabel every node, changing both the label and the indexed property.
    let updates: Vec<(u64, Value)> = (0..8u64)
        .map(|id| {
            (
                id,
                json!({ "_labels": ["Company"], "name": format!("new{id}"), "age": 900 + age_of(id) }),
            )
        })
        .collect();

    for (id, payload) in &updates {
        looped
            .store_node_payload(*id, payload)
            .expect("test: single update");
    }
    let entries: Vec<(u64, &Value)> = updates.iter().map(|(id, p)| (*id, p)).collect();
    batched
        .store_node_payloads(&entries)
        .expect("test: batch update");

    assert!(
        label_members(&batched, "Person").is_empty(),
        "the superseded label must be gone, as it is after the loop"
    );
    assert_eq!(
        label_members(&batched, "Company"),
        label_members(&looped, "Company")
    );
    assert_eq!(range_index_shape(&batched), range_index_shape(&looped));
}

#[test]
fn a_duplicate_id_resolves_to_the_last_payload_with_no_entry_from_the_first() {
    let (collection, _dir) = graph_collection();

    let first = json!({ "_labels": ["Person"], "age": 1 });
    let last = json!({ "_labels": ["Company"], "age": 2 });
    collection
        .store_node_payloads(&[(7, &first), (7, &last)])
        .expect("test: batch with a duplicate id");

    assert_eq!(
        collection.get_node_payload(7).expect("test: retrieve"),
        Some(last),
        "last write must win"
    );
    assert!(
        label_members(&collection, "Person").is_empty(),
        "the first payload's label must not survive — the un-index step reads \
         the node's pre-batch payload, so a naive batch would leave this behind"
    );
    assert_eq!(label_members(&collection, "Company"), vec![7]);

    // The superseded property value must not still be indexed. Compare against
    // the state a single write of `last` alone produces.
    let (reference, _reference_dir) = graph_collection();
    reference
        .store_node_payload(7, &json!({ "_labels": ["Company"], "age": 2 }))
        .expect("test: reference store");
    assert_eq!(
        range_index_shape(&collection),
        range_index_shape(&reference)
    );
}

#[test]
fn a_schema_violation_anywhere_in_the_batch_commits_nothing() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let collection = Collection::create_graph_collection(
        temp_dir.path().to_path_buf(),
        "nodes",
        GraphSchema::new().with_node_type(NodeType::new("Person")),
        None,
        DistanceMetric::Cosine,
    )
    .expect("test: create strict graph collection");

    // Node 1 already exists. It is the one that makes this test load-bearing:
    // the un-index step strips a node's old labels and property entries before
    // the write, so validating anywhere *after* that point rejects the batch
    // having already de-indexed a row it then leaves in place — a collection
    // whose indexes no longer describe its contents. Seeding an existing node
    // is what distinguishes upfront validation from late validation; a batch of
    // only-new nodes cannot tell them apart, because there is nothing to strip.
    let seeded = json!({ "_labels": ["Person"], "name": "seeded", "age": 30 });
    collection
        .store_node_payload(1, &seeded)
        .expect("test: seed an existing node");

    let update = json!({ "_labels": ["Person"], "name": "updated", "age": 31 });
    let good_b = json!({ "_labels": ["Person"], "name": "b" });
    let undeclared = json!({ "_labels": ["Alien"], "name": "c" });

    collection
        .store_node_payloads(&[(1, &update), (2, &good_b), (3, &undeclared)])
        .expect_err("test: an undeclared label must reject the batch");

    // The existing node keeps both its payload and its index entries.
    assert_eq!(
        collection.get_node_payload(1).expect("test: retrieve"),
        Some(seeded),
        "the batch failed, so node 1 must still carry its pre-batch payload"
    );
    assert_eq!(
        label_members(&collection, "Person"),
        vec![1],
        "node 1 was de-indexed by a batch that then refused to write"
    );
    assert!(
        range_index_shape(&collection)
            .iter()
            .any(|(_, ids)| ids == &vec![1]),
        "node 1's property entries were stripped by a batch that failed"
    );

    // And the per-node loop's other failure mode: it wrote each node as it went
    // and stopped at the bad one, leaving the prefix committed.
    for id in [2u64, 3] {
        assert_eq!(
            collection.get_node_payload(id).expect("test: retrieve"),
            None,
            "node {id} was committed by a batch that failed"
        );
    }
}

/// The batch's records reach the WAL and replay on open.
///
/// This deliberately does **not** claim to prove the fsync. A reopen in the
/// same process reads back through the OS page cache, so
/// `store_batch_deferred` — a buffer flush with no `sync_all` — passes it just
/// as `store_batch` does; only a machine-level crash separates the two, which
/// no in-process test reaches. Verified by mutation: swapping in the deferred
/// variant leaves this test green. What it does cover is that the batch wrote
/// well-formed records at all and that replay reconstructs every payload —
/// the barrier itself is covered by the measurement in the pull request.
#[test]
fn the_batch_replays_after_a_close_and_reopen() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let payloads: Vec<(u64, Value)> = (0..16u64)
        .map(|i| (i, person(&format!("n{i}"), age_of(i))))
        .collect();

    {
        let collection = Collection::create_graph_collection(
            temp_dir.path().to_path_buf(),
            "nodes",
            GraphSchema::schemaless(),
            None,
            DistanceMetric::Cosine,
        )
        .expect("test: create graph collection");
        let entries: Vec<(u64, &Value)> = payloads.iter().map(|(id, p)| (*id, p)).collect();
        collection
            .store_node_payloads(&entries)
            .expect("test: batch store");
    }

    let reopened =
        Collection::open(temp_dir.path().to_path_buf()).expect("test: reopen collection");
    for (id, payload) in &payloads {
        assert_eq!(
            reopened.get_node_payload(*id).expect("test: retrieve"),
            Some(payload.clone()),
            "node {id} did not survive the reopen"
        );
    }
}

#[test]
fn an_empty_batch_is_a_no_op() {
    let (collection, _dir) = graph_collection();
    collection
        .store_node_payloads(&[])
        .expect("test: empty batch");
    assert!(label_members(&collection, "Person").is_empty());
}

#[test]
fn the_single_node_entry_point_still_behaves_as_a_batch_of_one() {
    // `store_node_payload` now delegates; this pins that the delegation did not
    // change what a single write does to the indexes.
    let (collection, _dir) = graph_collection();
    let payload = person("solo", 42);
    collection
        .store_node_payload(3, &payload)
        .expect("test: single store");

    assert_eq!(
        collection.get_node_payload(3).expect("test: retrieve"),
        Some(payload)
    );
    assert_eq!(label_members(&collection, "Person"), vec![3]);
}
