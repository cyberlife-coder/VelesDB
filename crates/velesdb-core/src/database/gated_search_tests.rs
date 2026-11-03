//! `run_dense` tuning tests (#2302): a caller or scope filter must not make
//! the read gate drop `ef` / `quality`.
//!
//! Which quality a filtered search runs at is made observable by capping
//! `max_perfect_mode_vectors` below the indexed count: `Perfect` is then
//! refused by the same guard rail the unfiltered path applies, so a
//! `GuardRail` proves the tuning reached the filtered leaf — silently
//! dropping it (the pre-fix shape) answered `Ok`.

use super::*;
use crate::config::{LimitsConfig, VelesConfig};
use crate::point::Point;
use crate::DistanceMetric;
use tempfile::tempdir;

/// A database whose collections refuse `Perfect` searches (cap of 1 below
/// the 5 seeded vectors) plus 5 docs split `lang` en/fr.
fn setup_perfect_refusing_db() -> (tempfile::TempDir, Database) {
    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_perfect_mode_vectors: 1,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();
    let coll = db.get_vector_collection("docs").unwrap();
    coll.upsert((0u64..5).map(|id| {
        let (lang, vector) = if id % 2 == 0 {
            ("en", vec![1.0, 0.0, 0.0, 0.0])
        } else {
            ("fr", vec![0.0, 1.0, 0.0, 0.0])
        };
        Point::new(id, vector, Some(serde_json::json!({"lang": lang})))
    }))
    .unwrap();
    (dir, db)
}

fn lang_en_filter() -> crate::filter::Filter {
    crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "lang".to_string(),
        value: serde_json::Value::String("en".to_string()),
    })
}

/// A filtered dense read carrying `quality` runs at that quality: `Perfect`
/// is refused over the cap, where the pre-fix dispatch dropped the tuning and
/// returned `Ok` through the plain filtered leaf.
#[test]
fn test_gated_dense_filtered_search_honours_quality() {
    let (_dir, db) = setup_perfect_refusing_db();
    let q = vec![1.0, 0.0, 0.0, 0.0];
    let filter = lang_en_filter();
    let err = db
        .gated_search(
            "docs",
            None,
            None,
            GatedRead::Dense {
                query: &q,
                k: 2,
                ef: None,
                quality: Some(crate::SearchQuality::Perfect),
                filter: Some(&filter),
            },
        )
        .expect_err("filtered Perfect search over cap must be refused");
    assert!(
        matches!(
            err,
            crate::Error::GuardRail {
                kind: crate::error::GuardRailKind::Capacity,
                ..
            }
        ),
        "expected GuardRail, got {err:?}"
    );
}

/// `ef` reaches the filtered path and still wins over `quality`: `ef = 64`
/// resolves to `Custom(64)`, not `Perfect`, so the same over-cap collection
/// serves the read — and the filter still narrows it.
#[test]
fn test_gated_dense_filtered_search_ef_wins_over_quality() {
    let (_dir, db) = setup_perfect_refusing_db();
    let q = vec![1.0, 0.0, 0.0, 0.0];
    let filter = lang_en_filter();
    let results = db
        .gated_search(
            "docs",
            None,
            None,
            GatedRead::Dense {
                query: &q,
                k: 3,
                ef: Some(64),
                quality: Some(crate::SearchQuality::Perfect),
                filter: Some(&filter),
            },
        )
        .expect("ef override must turn the Perfect mode into Custom(64)");
    assert!(
        !results.is_empty() && results.iter().all(|r| r.point.id % 2 == 0),
        "expected only lang=en ids, got {results:?}"
    );
}

/// `ef` alone on a filtered read returns the filtered set (functional shape
/// of the LangChain call that used to raise `TypeError`).
#[test]
fn test_gated_dense_filtered_search_with_ef_returns_filtered_results() {
    let (_dir, db) = setup_perfect_refusing_db();
    let q = vec![1.0, 0.0, 0.0, 0.0];
    let filter = lang_en_filter();
    let results = db
        .gated_search(
            "docs",
            None,
            None,
            GatedRead::Dense {
                query: &q,
                k: 5,
                ef: Some(64),
                quality: None,
                filter: Some(&filter),
            },
        )
        .expect("filtered search with ef override");
    assert_eq!(results.len(), 3, "lang=en docs are ids 0, 2, 4");
    assert!(results.iter().all(|r| r.point.id % 2 == 0));
}
