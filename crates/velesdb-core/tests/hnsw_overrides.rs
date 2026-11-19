//! A caller that pins some HNSW parameters gets core's choice for the rest.
//!
//! Every binding merged its own: the server and Tauri over
//! `HnswParams::auto(dimension)`, Python over `HnswParams::default()` — the
//! 768-dimension profile, whatever the collection's dimension — and none of
//! them read the `[hnsw]` section, which `create_vector_collection_with_hnsw`
//! honours (#2087). Core resolves it once now: a pinned field wins, then
//! `[hnsw]`, then auto-tuning for the collection's own dimension.

#![cfg(feature = "persistence")]

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, HnswOverrides, HnswParams, StorageMode, VelesConfig};

const DIM: usize = 64;

fn db_with_hnsw_m(m: Option<usize>) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let mut config = VelesConfig::default();
    config.hnsw.m = m;
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    (dir, db)
}

fn persisted(db: &Database, name: &str) -> Option<HnswParams> {
    db.get_vector_collection(name)
        .expect("test: collection")
        .config()
        .hnsw_params
}

#[test]
fn unpinned_fields_come_from_the_collections_own_dimension() {
    let (_dir, db) = db_with_hnsw_m(None);
    let overrides = HnswOverrides {
        alpha: Some(1.2),
        ..HnswOverrides::default()
    };
    db.create_vector_collection_with_overrides(
        "docs",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        overrides,
        None,
    )
    .expect("test: create");

    let auto = HnswParams::auto(DIM);
    let params = persisted(&db, "docs").expect("pinned params are persisted");
    assert_eq!(params.max_connections, auto.max_connections);
    assert_eq!(params.ef_construction, auto.ef_construction);
    assert_eq!(params.max_elements, auto.max_elements);
    assert!((params.alpha - 1.2).abs() < f32::EPSILON);
}

#[test]
fn the_hnsw_section_fills_a_field_the_caller_left_unset() {
    let (_dir, db) = db_with_hnsw_m(Some(48));
    let overrides = HnswOverrides {
        ef_construction: Some(300),
        ..HnswOverrides::default()
    };
    db.create_vector_collection_with_overrides(
        "docs",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        overrides,
        None,
    )
    .expect("test: create");

    let params = persisted(&db, "docs").expect("pinned params are persisted");
    assert_eq!(params.max_connections, 48, "[hnsw] m");
    assert_eq!(params.ef_construction, 300, "the caller's pin");
}

#[test]
fn a_pinned_field_beats_the_hnsw_section() {
    let (_dir, db) = db_with_hnsw_m(Some(48));
    let overrides = HnswOverrides {
        m: Some(12),
        ..HnswOverrides::default()
    };
    db.create_vector_collection_with_overrides(
        "docs",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        overrides,
        None,
    )
    .expect("test: create");
    assert_eq!(persisted(&db, "docs").map(|p| p.max_connections), Some(12));
}

#[test]
fn no_pin_at_all_persists_no_choice() {
    let (_dir, db) = db_with_hnsw_m(None);
    db.create_vector_collection_with_overrides(
        "docs",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        HnswOverrides::default(),
        None,
    )
    .expect("test: create");
    assert_eq!(persisted(&db, "docs"), None, "nobody chose");
}

#[test]
fn an_out_of_range_pin_is_refused_before_anything_is_created() {
    let (_dir, db) = db_with_hnsw_m(None);
    let overrides = HnswOverrides {
        alpha: Some(0.5),
        ..HnswOverrides::default()
    };
    let err = db
        .create_vector_collection_with_overrides(
            "docs",
            DIM,
            DistanceMetric::Cosine,
            StorageMode::Full,
            overrides,
            None,
        )
        .expect_err("alpha < 1.0 is refused");
    assert_eq!(err.code(), "VELES-009", "{err}");
    assert!(db.get_vector_collection("docs").is_none());
}
