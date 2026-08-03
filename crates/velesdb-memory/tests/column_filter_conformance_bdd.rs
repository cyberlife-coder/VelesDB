//! The native half of the shared `ColumnFilter` conformance suite.
//!
//! The table itself lives in
//! [`velesdb_memory::column_filter_conformance`] and is run verbatim by the
//! WASM backend too (`crates/velesdb-wasm/src/memory_store_tests.rs`). Adding
//! a case there enforces it on both backends at once — which is the whole
//! point, since `ne` on an absent field diverged between them unnoticed for
//! the API's entire life (#1759).
//!
//! This side goes through `NativeStore`, which translates `ColumnFilter` into
//! `VelesQL` — a completely different evaluator from the WASM one. Identical
//! results here and there is the parity the suite exists to prove.

#![cfg(feature = "persistence")]

use velesdb_memory::column_filter_conformance::{
    cases, cases_failed_under, fixture, NeSemantics, SCAFFOLDING,
};
use velesdb_memory::storage::NativeStore;
use velesdb_memory::MemoryStore;

const EMBEDDING: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// A store preloaded with the shared fixture.
fn seeded() -> (tempfile::TempDir, NativeStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = NativeStore::open(dir.path(), 4).expect("open store");
    for fact in fixture() {
        store
            .store_with_metadata(fact.id, fact.content, &EMBEDDING, &fact.metadata)
            .expect("seed fixture");
    }
    (dir, store)
}

#[test]
fn the_native_backend_satisfies_every_conformance_case() {
    let (_dir, store) = seeded();
    for case in cases() {
        let hits = store
            .query_columnar(&EMBEDDING, 50, &case.filters)
            .expect("query_columnar");
        let mut ids: Vec<u64> = hits.iter().map(|hit| hit.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, case.expected, "native: {}", case.name);
    }
}

#[test]
fn the_native_backend_never_returns_internal_scaffolding() {
    let (_dir, store) = seeded();
    for case in cases() {
        let hits = store
            .query_columnar(&EMBEDDING, 50, &case.filters)
            .expect("query_columnar");
        assert!(
            hits.iter().all(|hit| hit.id != SCAFFOLDING),
            "native: {} leaked internal scaffolding",
            case.name
        );
    }
}

/// The positive control. Without it the table could be green and worthless:
/// a suite that also accepts the behaviour it replaced has proven nothing.
#[test]
fn the_table_rejects_the_behaviour_it_replaced() {
    assert!(
        cases_failed_under(NeSemantics::Contract).is_empty(),
        "the contract itself must satisfy every case"
    );
    assert!(
        !cases_failed_under(NeSemantics::AbsentMatched).is_empty(),
        "a backend where `ne` matched an ABSENT field must fail at least one case, \
         or this suite cannot detect the native regression it was written for"
    );
    assert!(
        !cases_failed_under(NeSemantics::NullMatched).is_empty(),
        "a backend where `ne` matched an explicit NULL must fail at least one case, \
         or this suite cannot detect a regression on either backend"
    );
}
