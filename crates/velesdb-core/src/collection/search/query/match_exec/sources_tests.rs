use super::*;

use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

use crate::distance::DistanceMetric;
use crate::velesql::Parser;

fn collection(dir: &tempfile::TempDir, name: &str) -> Collection {
    Collection::create(dir.path().join(name), 2, DistanceMetric::Cosine).expect("test: collection")
}

fn clause(sql: &str) -> MatchClause {
    Parser::parse(sql)
        .expect("test: parse")
        .match_clause
        .expect("test: MATCH clause")
}

// --- union_payload --------------------------------------------------------

#[test]
fn the_union_adds_annotated_fields_and_keeps_the_nodes_own() {
    let own = Some(Arc::new(json!({"_labels": ["W"], "name": "Paris"})));
    let annotated = Some(json!({"name": "SHADOWED", "price": 9.5, "_labels": ["X"]}));
    let united = union_payload(own, annotated).expect("test: union");
    assert_eq!(
        *united,
        json!({"_labels": ["W"], "name": "Paris", "price": 9.5})
    );
}

#[test]
fn either_side_alone_is_the_union() {
    let own = Arc::new(json!({"a": 1}));
    assert_eq!(
        union_payload(Some(Arc::clone(&own)), None).as_deref(),
        Some(&*own)
    );
    assert_eq!(
        union_payload(None, Some(json!({"b": 2}))).as_deref(),
        Some(&json!({"b": 2}))
    );
    assert_eq!(union_payload(None, None), None);
}

#[test]
fn a_non_object_side_never_merges() {
    let scalar = Arc::new(json!(7));
    assert_eq!(
        union_payload(Some(Arc::clone(&scalar)), Some(json!({"b": 2}))).as_deref(),
        Some(&*scalar)
    );
    let own = Arc::new(json!({"a": 1}));
    assert_eq!(
        union_payload(Some(Arc::clone(&own)), Some(json!([1, 2]))).as_deref(),
        Some(&*own)
    );
}

// --- MatchSources::for_clause --------------------------------------------

#[test]
fn a_clause_without_annotations_resolves_nothing() {
    let sources = MatchSources::for_clause(&clause("MATCH (a:P)-[:R]->(b:P) RETURN a"), |name| {
        panic!("resolved {name}")
    })
    .expect("test: sources");
    assert!(sources.collections.is_empty() && sources.aliases.is_empty());
}

#[test]
fn each_annotation_name_is_resolved_once_and_every_aliased_node_mapped() {
    let dir = tempfile::tempdir().expect("test: dir");
    let pricing = collection(&dir, "pricing");
    let mut resolved = Vec::new();
    let sources = MatchSources::for_clause(
        &clause("MATCH (a:P@pricing)-[:R]->(b:P@pricing)-[:R]->(:P@pricing) RETURN a"),
        |name| {
            resolved.push(name.to_string());
            Ok(pricing.clone())
        },
    )
    .expect("test: sources");
    assert_eq!(resolved, vec!["pricing"]);
    assert_eq!(
        sources.aliases,
        vec![
            ("a".to_string(), "pricing".to_string()),
            ("b".to_string(), "pricing".to_string()),
        ]
    );
}

#[test]
fn an_unresolvable_annotation_fails_the_clause() {
    let err = MatchSources::for_clause(&clause("MATCH (a:P@nowhere) RETURN a"), |name| {
        Err(crate::error::Error::CollectionNotFound(name.to_string()))
    })
    .err()
    .expect("test: refused");
    assert!(err.to_string().contains("nowhere"), "{err}");
}

// --- MatchReads::acquire --------------------------------------------------

fn sources_of(pairs: &[(&str, &str, &Collection)]) -> MatchSources {
    let mut sources = MatchSources::default();
    for (alias, name, collection) in pairs {
        if sources.collections.iter().all(|(n, _)| n != name) {
            sources
                .collections
                .push(((*name).to_string(), (*collection).clone()));
        }
        sources
            .aliases
            .push(((*alias).to_string(), (*name).to_string()));
    }
    sources
}

// The guards are what the test counts: they live to its end.
#[expect(clippy::significant_drop_tightening)]
#[test]
fn the_target_is_never_locked_twice_by_its_own_annotation() {
    let dir = tempfile::tempdir().expect("test: dir");
    let target = collection(&dir, "g");
    let sources = sources_of(&[("a", "g", &target)]);
    let reads = MatchReads::acquire(&target, &sources);
    assert_eq!(crate::lock_audit::guards_held(), 2, "vector + payload only");
    let annotations = reads.annotations();
    let index = annotations.by_alias("a").expect("test: annotated alias");
    assert!(std::ptr::eq(
        annotations.storage(index),
        std::ptr::from_ref(&*reads.payload)
    ));
}

// The guards are what the test counts: they live to its end.
#[expect(clippy::significant_drop_tightening)]
#[test]
fn a_collection_named_by_several_annotations_is_locked_once() {
    let dir = tempfile::tempdir().expect("test: dir");
    let target = collection(&dir, "g");
    let other = collection(&dir, "o");
    // Two names for one instance, as two racing opens of a name can give.
    let sources = sources_of(&[("a", "o", &other), ("b", "o2", &other)]);
    let reads = MatchReads::acquire(&target, &sources);
    assert_eq!(crate::lock_audit::guards_held(), 3);
    let annotations = reads.annotations();
    let (a, b) = (
        annotations.by_alias("a").expect("test: a"),
        annotations.by_alias("b").expect("test: b"),
    );
    assert!(std::ptr::eq(annotations.storage(a), annotations.storage(b)));
}

/// Two MATCH queries annotating each other's collection, with writers queued on
/// both payload locks: in any order but the total one, parking_lot's fair
/// `RwLock` parks each reader behind the other's queued writer.
#[test]
fn cross_annotating_matches_never_deadlock_against_queued_writers() {
    let dir = tempfile::tempdir().expect("test: dir");
    let a = collection(&dir, "a");
    let b = collection(&dir, "b");
    let (done_tx, done_rx) = mpsc::channel();
    let mut handles = Vec::new();
    for (target, other) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
        let done = done_tx.clone();
        handles.push(std::thread::spawn(move || {
            let sources = sources_of(&[("x", "other", &other)]);
            for _ in 0..2_000 {
                let reads = MatchReads::acquire(&target, &sources);
                std::hint::black_box(reads.annotations().storages.len());
            }
            done.send(()).expect("test: report");
        }));
    }
    for collection in [a, b] {
        let done = done_tx.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..2_000 {
                drop(collection.storage.payload_storage.write());
            }
            done.send(()).expect("test: report");
        }));
    }
    for _ in 0..handles.len() {
        done_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("test: a MATCH and a writer deadlocked");
    }
    for handle in handles {
        handle.join().expect("test: thread");
    }
}
