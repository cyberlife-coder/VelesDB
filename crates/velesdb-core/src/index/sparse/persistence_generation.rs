//! Two-slot sparse snapshot publication metadata.
//!
//! Compaction writes the inactive slot completely, then atomically replaces a
//! small manifest. The manifest is the only commit point, so restart never
//! combines files from different snapshot generations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::storage::atomic_write::atomic_write;

const MANIFEST_VERSION: u8 = 1;
const MAX_MANIFEST_BYTES: u64 = 64;

#[derive(Clone, Debug)]
pub(super) struct SnapshotPaths {
    pub(super) idx: PathBuf,
    pub(super) terms: PathBuf,
    pub(super) meta: PathBuf,
}

#[derive(Debug)]
pub(super) struct ActiveSnapshot {
    pub(super) paths: SnapshotPaths,
    pub(super) wal_generation: Option<u64>,
}

#[derive(Debug)]
pub(super) struct PendingSnapshot {
    pub(super) paths: SnapshotPaths,
    manifest: SnapshotManifest,
}

impl PendingSnapshot {
    pub(super) const fn generation(&self) -> u64 {
        self.manifest.generation
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct SnapshotManifest {
    version: u8,
    generation: u64,
    slot: u8,
}

pub(super) fn prepare_snapshot(dir: &Path, prefix: &str) -> Result<PendingSnapshot> {
    let current = read_manifest(dir, prefix)?;
    let (generation, slot) = match current {
        Some(manifest) => (
            manifest.generation.checked_add(1).ok_or_else(|| {
                Error::SparseIndexError("sparse snapshot generation overflow".to_string())
            })?,
            1 - manifest.slot,
        ),
        // Slot 0 is the manifest-free legacy fallback. The first manifested
        // snapshot must use slot 1 even for a new database: if the manifest is
        // lost in a crash, restart can then recover from the old slot-0 files
        // (when present) plus the WAL, or from the WAL alone.
        None => (1, 1),
    };
    Ok(PendingSnapshot {
        paths: slot_paths(dir, prefix, slot),
        manifest: SnapshotManifest {
            version: MANIFEST_VERSION,
            generation,
            slot,
        },
    })
}

pub(super) fn publish_snapshot(dir: &Path, prefix: &str, pending: &PendingSnapshot) -> Result<()> {
    let bytes = postcard::to_allocvec(&pending.manifest)
        .map_err(|e| sparse_error("manifest serialize", e))?;
    atomic_write(&manifest_path(dir, prefix), &bytes)
        .map_err(|e| sparse_error("manifest publish", e))
}

pub(super) fn active_snapshot(dir: &Path, prefix: &str) -> Result<Option<ActiveSnapshot>> {
    if let Some(manifest) = read_manifest(dir, prefix)? {
        return Ok(Some(ActiveSnapshot {
            paths: slot_paths(dir, prefix, manifest.slot),
            wal_generation: Some(manifest.generation),
        }));
    }
    let paths = legacy_paths(dir, prefix);
    Ok(paths.meta.exists().then_some(ActiveSnapshot {
        paths,
        wal_generation: None,
    }))
}

pub(super) fn committed_generation_for_wal(wal_path: &Path) -> Result<Option<u64>> {
    let parent = wal_path.parent().unwrap_or_else(|| Path::new("."));
    let prefix = wal_prefix(wal_path)?;
    Ok(read_manifest(parent, prefix)?.map(|manifest| manifest.generation))
}

fn read_manifest(dir: &Path, prefix: &str) -> Result<Option<SnapshotManifest>> {
    let path = manifest_path(dir, prefix);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(sparse_error("manifest metadata", error)),
    };
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(Error::SparseIndexError(format!(
            "sparse snapshot manifest exceeds {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| sparse_error("manifest read", e))?;
    let manifest: SnapshotManifest =
        postcard::from_bytes(&bytes).map_err(|e| sparse_error("manifest deserialize", e))?;
    validate_manifest(manifest).map(Some)
}

fn validate_manifest(manifest: SnapshotManifest) -> Result<SnapshotManifest> {
    if manifest.version != MANIFEST_VERSION || manifest.slot > 1 || manifest.generation == 0 {
        return Err(Error::SparseIndexError(format!(
            "invalid sparse snapshot manifest: version={}, generation={}, slot={}",
            manifest.version, manifest.generation, manifest.slot
        )));
    }
    Ok(manifest)
}

fn wal_prefix(wal_path: &Path) -> Result<&str> {
    wal_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".wal"))
        .ok_or_else(|| {
            Error::SparseIndexError(format!("invalid sparse WAL path: {}", wal_path.display()))
        })
}

fn manifest_path(dir: &Path, prefix: &str) -> PathBuf {
    dir.join(format!("{prefix}.snapshot"))
}

fn legacy_paths(dir: &Path, prefix: &str) -> SnapshotPaths {
    slot_paths(dir, prefix, 0)
}

fn slot_paths(dir: &Path, prefix: &str, slot: u8) -> SnapshotPaths {
    let stem = if slot == 0 {
        prefix.to_string()
    } else {
        format!(".{prefix}.next")
    };
    SnapshotPaths {
        idx: dir.join(format!("{stem}.idx")),
        terms: dir.join(format!("{stem}.terms")),
        meta: dir.join(format!("{stem}.meta")),
    }
}

fn sparse_error(context: &str, error: impl std::fmt::Display) -> Error {
    Error::SparseIndexError(format!("sparse snapshot {context}: {error}"))
}
