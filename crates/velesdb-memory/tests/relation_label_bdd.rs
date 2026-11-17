//! A relation label is its trimmed text, on every store: the graph stores
//! trim an edge label (core's `GraphEdge::new`), so `relate` and `unrelate`
//! compare and write the same canonical label.

use velesdb_memory::{HashEmbedder, MemoryError, MemoryService, DEFAULT_DIMENSION};

#[test]
fn a_padded_relation_round_trips_through_relate_and_unrelate() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let svc =
        MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).expect("test: open");
    let a = svc
        .remember("Alice leads the parser work", &[], None)
        .expect("test: a");
    let b = svc
        .remember("The parser ships in March", &[], None)
        .expect("test: b");

    let first = svc
        .relate(a, b, " decided_in ")
        .expect("test: first relate");
    let again = svc
        .relate(a, b, " decided_in ")
        .expect("a repeated relate is idempotent, not a hash collision");
    assert_eq!(first, again);
    assert_eq!(svc.relate(a, b, "decided_in").expect("test: bare"), first);

    let outcome = svc.unrelate(a, b, " decided_in ").expect("test: unrelate");
    assert!(
        outcome.found,
        "unrelate with the label as given removes the edge"
    );
    assert!(
        !svc.unrelate(a, b, "decided_in").expect("test: again").found,
        "the edge is gone"
    );

    assert!(matches!(
        svc.relate(a, b, "   "),
        Err(MemoryError::InvalidRelation(_))
    ));
}
