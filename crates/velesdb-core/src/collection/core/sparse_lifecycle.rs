//! Sparse-index discovery during collection open.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::index::sparse::{persistence, SparseInvertedIndex};
use crate::sparse_index::DEFAULT_SPARSE_INDEX_NAME;

pub(super) fn load_named_sparse_indexes(path: &Path) -> BTreeMap<String, SparseInvertedIndex> {
    let mut indexes = BTreeMap::new();
    load_default(path, &mut indexes);
    for name in discover_named_indexes(path) {
        load_named(path, name, &mut indexes);
    }
    indexes
}

fn load_default(path: &Path, indexes: &mut BTreeMap<String, SparseInvertedIndex>) {
    match persistence::load_from_disk(path) {
        Ok(Some(index)) => {
            indexes.insert(DEFAULT_SPARSE_INDEX_NAME.to_string(), index);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            "Failed to load default sparse index from {:?}: {}. Skipping.",
            path,
            error
        ),
    }
}

fn discover_named_indexes(path: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| sparse_name(&entry.file_name().to_string_lossy()))
        .collect()
}

fn sparse_name(file_name: &str) -> Option<String> {
    let suffix = file_name.strip_prefix("sparse-")?;
    suffix
        .strip_suffix(".snapshot")
        .or_else(|| suffix.strip_suffix(".meta"))
        .map(str::to_string)
}

fn load_named(path: &Path, name: String, indexes: &mut BTreeMap<String, SparseInvertedIndex>) {
    match persistence::load_named_from_disk(path, &name) {
        Ok(Some(index)) => {
            indexes.insert(name, index);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            "Failed to load sparse index '{}' from {:?}: {}. Skipping.",
            name,
            path,
            error
        ),
    }
}

#[cfg(test)]
#[path = "sparse_lifecycle_tests.rs"]
mod tests;
