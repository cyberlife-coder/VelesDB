//! Tests for the `[hnsw]` configuration wiring (issue #2087).
//!
//! The validation bar the issue sets for a wired knob is two-sided: the
//! configured value must reach the engine, **and** the stronger precedence
//! levels must still win over it. Both directions are covered here for each
//! of the two wired fields, on both collection kinds that build an HNSW
//! index, plus the property that makes the wiring safe to ship — a default
//! `[hnsw]` section changes nothing at all.

use crate::config::VelesConfig;
use crate::index::hnsw::HnswParams;
use crate::quantization::StorageMode;
use crate::{Database, DistanceMetric};
use tempfile::tempdir;

/// A config whose `[hnsw]` section pins both wired knobs to values no
/// auto-tuned default produces, so a passing assertion cannot be a
/// coincidence: `HnswParams::auto` yields (24, 300) at dim ≤ 256 and
/// (32, 400) above.
fn config_with_hnsw(m: Option<usize>, ef_construction: Option<usize>) -> VelesConfig {
    let mut config = VelesConfig::default();
    config.hnsw.m = m;
    config.hnsw.ef_construction = ef_construction;
    config
}

const DIM: usize = 128;

// -------------------------------------------------------------------------
// Level 2 — the `[hnsw]` section applies
// -------------------------------------------------------------------------

#[test]
fn test_hnsw_config_applies_to_vector_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    db.create_vector_collection("cfg", DIM, DistanceMetric::Cosine)
        .unwrap();

    let params = db
        .get_vector_collection("cfg")
        .unwrap()
        .config()
        .hnsw_params
        .expect("a config-resolved collection persists its params");
    assert_eq!(params.max_connections, 40);
    assert_eq!(params.ef_construction, 500);
}

#[test]
fn test_hnsw_config_applies_to_graph_collection_with_embeddings() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    db.create_graph_collection_with_embeddings(
        "g",
        crate::collection::GraphSchema::default(),
        DIM,
        DistanceMetric::Cosine,
    )
    .unwrap();

    // A graph collection with embeddings builds a real HNSW index over its
    // node vectors, so leaving it on the auto-tuned defaults would be the
    // same silent half-wiring #2087 exists to remove.
    let params = db
        .get_graph_collection("g")
        .unwrap()
        .inner
        .config()
        .hnsw_params
        .expect("a graph collection with embeddings persists its params");
    assert_eq!(params.max_connections, 40);
    assert_eq!(params.ef_construction, 500);
}

#[test]
fn test_hnsw_config_fields_resolve_independently() {
    let dir = tempdir().unwrap();
    // Only `m` is configured; `ef_construction` must fall through to the
    // dimension-tuned default rather than to some section-wide all-or-nothing.
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), None)).unwrap();

    db.create_vector_collection("partial", DIM, DistanceMetric::Cosine)
        .unwrap();

    let params = db
        .get_vector_collection("partial")
        .unwrap()
        .config()
        .hnsw_params
        .unwrap();
    assert_eq!(params.max_connections, 40);
    assert_eq!(
        params.ef_construction,
        HnswParams::auto(DIM).ef_construction,
        "an unset section field must leave the auto-tuned value alone"
    );
}

// -------------------------------------------------------------------------
// Level 1 — per-collection creation arguments still win
// -------------------------------------------------------------------------

#[test]
fn test_creation_argument_overrides_hnsw_config() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    db.create_vector_collection_with_hnsw(
        "explicit",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        Some(12),
        Some(90),
    )
    .unwrap();

    let params = db
        .get_vector_collection("explicit")
        .unwrap()
        .config()
        .hnsw_params
        .unwrap();
    assert_eq!(params.max_connections, 12);
    assert_eq!(params.ef_construction, 90);
}

#[test]
fn test_partial_creation_argument_takes_the_other_field_from_config() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    // Pins `m` only. The chain is per-field, so `ef_construction` must come
    // from the section — not silently revert to the auto-tuned default.
    db.create_vector_collection_with_hnsw(
        "mixed",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        Some(12),
        None,
    )
    .unwrap();

    let params = db
        .get_vector_collection("mixed")
        .unwrap()
        .config()
        .hnsw_params
        .unwrap();
    assert_eq!(params.max_connections, 12);
    assert_eq!(params.ef_construction, 500);
}

#[test]
fn test_create_with_params_ignores_hnsw_config_entirely() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    let explicit = HnswParams::custom(48, 400, 250_000).with_alpha(1.5);
    db.create_vector_collection_with_params(
        "full",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
        explicit,
        Some(8),
    )
    .unwrap();

    // A fully specified params object is already a complete answer: merging
    // a deployment default into it would make the result depend on a file
    // this caller never mentioned.
    let params = db
        .get_vector_collection("full")
        .unwrap()
        .config()
        .hnsw_params
        .unwrap();
    assert_eq!(params.max_connections, 48);
    assert_eq!(params.ef_construction, 400);
    assert_eq!(params.max_elements, 250_000);
}

// -------------------------------------------------------------------------
// The property that makes the wiring safe to ship
// -------------------------------------------------------------------------

#[test]
fn test_default_hnsw_config_persists_exactly_the_unwired_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_vector_collection("plain", DIM, DistanceMetric::Cosine)
        .unwrap();

    let cfg = db.get_vector_collection("plain").unwrap().config();

    // `None` — "nobody ever chose" — must survive, exactly as
    // `pq_rescore_oversampling: None` does a field away. Persisting a snapshot
    // of today's auto-tuned defaults here would build the identical index and
    // destroy the distinction a later migration reads.
    assert_eq!(
        cfg.hnsw_params, None,
        "an untouched [hnsw] section must not materialize a params snapshot"
    );

    // And the PQ rescore factor must stay at the engine default. Routing
    // through `create_with_params` with `None` would have persisted "no
    // explicit override" instead, silently changing what migrations read.
    assert_eq!(cfg.pq_rescore_oversampling, Some(4));

    // The index itself is still built from the auto-tuned defaults — the
    // `None` above records the absence of a choice, not the absence of a
    // topology.
    assert_eq!(
        HnswParams::from_config(DIM, &crate::config::HnswConfig::default()),
        HnswParams {
            storage_mode: StorageMode::Full,
            ..HnswParams::auto(DIM)
        }
    );
}

#[test]
fn test_graph_collection_without_embeddings_is_unaffected() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_config(dir.path(), config_with_hnsw(Some(40), Some(500))).unwrap();

    db.create_graph_collection("no_emb", crate::collection::GraphSchema::default())
        .unwrap();

    // No dimension means no HNSW index to configure; resolving params for a
    // zero-dimension collection would persist a topology nothing ever builds,
    // sized from dimension 0 at that.
    assert!(db
        .get_graph_collection("no_emb")
        .unwrap()
        .inner
        .config()
        .hnsw_params
        .is_none());
}

// -------------------------------------------------------------------------
// The mapping point itself
// -------------------------------------------------------------------------

#[test]
fn test_params_from_default_config_equals_auto() {
    let default = crate::config::HnswConfig::default();
    for dim in [64_usize, 128, 384, 768] {
        assert_eq!(
            HnswParams::from_config(dim, &default),
            HnswParams::auto(dim),
            "a default [hnsw] section must be indistinguishable from no section at dim {dim}"
        );
    }
}

#[test]
fn test_params_from_config_leaves_unmapped_fields_alone() {
    // `max_layers` has no engine counterpart; setting it must not move any
    // field of the resolved params.
    let hnsw = crate::config::HnswConfig {
        m: Some(40),
        ef_construction: Some(500),
        max_layers: 12,
    };

    let params = HnswParams::from_config(DIM, &hnsw);
    let auto = HnswParams::auto(DIM);
    assert_eq!(params.max_connections, 40);
    assert_eq!(params.ef_construction, 500);
    assert_eq!(params.max_elements, auto.max_elements);
    assert_eq!(params.storage_mode, auto.storage_mode);
    assert!((params.alpha - auto.alpha).abs() < f32::EPSILON);
}
