//! Point preparation helpers for the migration pipeline.
//!
//! Extracted from `pipeline.rs` to keep module size under 500 NLOC.

use tracing::warn;

use crate::connectors::ExtractedPoint;
use crate::error::{Error, Result};
use crate::pipeline::MigrationStats;

/// Maps string IDs to deterministic u64 point IDs.
///
/// **Strategy:** Numeric strings (`"12345"`) are parsed directly to u64.
/// Non-numeric strings (UUIDs, slugs, etc.) are hashed via FNV-1a to a
/// deterministic u64. Hash collisions are theoretically possible but
/// extremely rare for typical ID spaces.
///
/// # Cross-version stability guarantee
///
/// Changing this function would silently corrupt checkpoint-resumed migrations
/// because IDs would no longer match previously-inserted points.
pub(crate) fn stable_point_id(id: &str) -> u64 {
    id.parse::<u64>()
        .unwrap_or_else(|_| super::pipeline::fnv1a64(id.as_bytes()))
}

/// Prepares extracted points for insertion, optionally using parallel workers.
pub(crate) fn prepare_points(
    points: Vec<ExtractedPoint>,
    workers: usize,
    continue_on_error: bool,
    stats: &mut MigrationStats,
) -> Result<Vec<velesdb_core::Point>> {
    if points.is_empty() {
        return Ok(Vec::new());
    }

    let prepared = if workers <= 1 || points.len() == 1 {
        points
            .into_iter()
            .map(build_point)
            .collect::<Vec<Result<velesdb_core::Point>>>()
    } else {
        let chunk_size = points.len().div_ceil(workers);
        let mut chunks = Vec::new();
        let mut iter = points.into_iter();

        loop {
            let chunk: Vec<ExtractedPoint> = iter.by_ref().take(chunk_size).collect();
            if chunk.is_empty() {
                break;
            }
            chunks.push(chunk);
        }

        std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .into_iter()
                            .map(build_point)
                            .collect::<Vec<Result<velesdb_core::Point>>>()
                    })
                })
                .collect();

            handles
                .into_iter()
                .flat_map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        vec![Err(Error::Transformation(
                            "Worker thread panicked while preparing points".to_string(),
                        ))]
                    })
                })
                .collect()
        })
    };

    let mut valid_points = Vec::with_capacity(prepared.len());
    for result in prepared {
        match result {
            Ok(point) => valid_points.push(point),
            Err(error) => {
                if !continue_on_error {
                    return Err(error);
                }
                stats.failed += 1;
                warn!("Skipping point during preparation: {}", error);
            }
        }
    }

    Ok(valid_points)
}

fn build_point(point: ExtractedPoint) -> Result<velesdb_core::Point> {
    if point.vector.iter().any(|value| !value.is_finite()) {
        return Err(Error::Transformation(format!(
            "Point '{}' contains non-finite vector values",
            point.id
        )));
    }

    let payload = if point.payload.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(
            point.payload.into_iter().collect(),
        ))
    };

    let sparse_vectors = point.sparse_vector.map(|pairs| {
        let sv = velesdb_core::sparse_index::SparseVector::new(pairs);
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            velesdb_core::index::sparse::DEFAULT_SPARSE_INDEX_NAME.to_string(),
            sv,
        );
        map
    });

    Ok(velesdb_core::Point::with_sparse(
        stable_point_id(&point.id),
        point.vector,
        payload,
        sparse_vectors,
    ))
}
