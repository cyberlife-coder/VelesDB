use std::collections::HashMap;

use serde_json::json;
use velesdb_core::GraphEdge;

use super::TestRig;
use crate::storage::MemoryStore;
use crate::Metadata;

#[test]
fn base_copy_reembeds_a_fact_into_the_destination() {
    let rig = TestRig::new();
    let id = rig.source.remember("alpha", &[], None).expect("remember");
    let copy = rig.start();

    let progress = copy.copy_base().expect("base copy");
    let stored = rig.destination.get(id).expect("get").expect("stored");
    assert_eq!(stored, ("alpha".to_owned(), vec![7.0, 8.0, 9.0]));
    assert_eq!(progress.facts, 1);
    copy.finish().expect("finish");
}

#[test]
fn base_copy_preserves_raw_payload_expiry_and_edge_properties() {
    let rig = TestRig::new();
    let mut metadata = Metadata::new();
    metadata.insert("tenant".to_owned(), json!("acme"));
    let from = rig
        .source
        .remember_with_ttl("from", &[], Some(&metadata), Some(3_600))
        .expect("from");
    let to = rig.source.remember("to", &[], None).expect("to");
    let edge = edge(from, to, "owns", json!(7));
    rig.source
        .migration_store()
        .migration_replace_edges(from, std::slice::from_ref(&edge), 8)
        .expect("seed edge");
    let source_payload = rig
        .source
        .migration_store()
        .migration_payload(from)
        .expect("source payload");
    let copy = rig.start();

    copy.copy_base().expect("base copy");
    let destination_payload = rig
        .destination
        .migration_payload(from)
        .expect("destination payload");
    let destination_edges = rig
        .destination
        .migration_live_edges(from, 8)
        .expect("destination edges");
    assert_eq!(destination_payload, source_payload);
    assert_eq!(destination_edges, vec![edge]);
    copy.finish().expect("finish");
}

#[test]
fn degree_above_the_cap_is_refused_before_edges_are_written() {
    let mut rig = TestRig::new();
    let from = rig.source.remember("from", &[], None).expect("from");
    let targets = ["one", "two", "three"]
        .map(|fact| rig.source.remember(fact, &[], None).expect("target fact"));
    let edges: Vec<_> = targets
        .into_iter()
        .enumerate()
        .map(|(index, to)| edge(from, to, &format!("r{index}"), json!(index)))
        .collect();
    rig.source
        .migration_store()
        .migration_replace_edges(from, &edges, 8)
        .expect("seed edges");
    rig.config.edge_cap = 2;
    let copy = rig.start();

    let error = copy.copy_base().expect_err("degree must be refused");
    assert!(error.to_string().contains("exceeds migration cap 2"));
    assert!(rig
        .destination
        .migration_live_edges(from, 2)
        .expect("destination edges")
        .is_empty());
    copy.finish().expect("finish");
}

fn edge(from: u64, to: u64, label: &str, value: serde_json::Value) -> GraphEdge {
    let id = velesdb_core::hash_edge_id(from, to, label);
    let mut properties = HashMap::new();
    properties.insert("weight".to_owned(), value);
    GraphEdge::new(id, from, to, label)
        .expect("edge")
        .with_properties(properties)
}
