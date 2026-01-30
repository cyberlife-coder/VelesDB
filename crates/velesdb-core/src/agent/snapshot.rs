//! Snapshot and versioning support for AgentMemory.
//!
//! Provides serialization/deserialization of AgentMemory state for:
//! - Persistence across restarts
//! - Rollback to previous versions
//! - State transfer between instances
//!
//! # Snapshot Format
//!
//! ```text
//! [Magic: "VAMM" 4 bytes]
//! [Version: 1 byte]
//! [Semantic state length: 8 bytes]
//! [Semantic state: N bytes]
//! [Episodic state length: 8 bytes]
//! [Episodic state: N bytes]
//! [Procedural state length: 8 bytes]
//! [Procedural state: N bytes]
//! [TTL state length: 8 bytes]
//! [TTL state: N bytes]
//! [CRC32: 4 bytes]
//! ```

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

/// Snapshot file magic bytes for AgentMemory.
pub const SNAPSHOT_MAGIC: &[u8; 4] = b"VAMM";

/// Current snapshot format version.
pub const SNAPSHOT_VERSION: u8 = 1;

/// Simple CRC32 implementation (IEEE 802.3 polynomial).
#[inline]
fn crc32_hash(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };

    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

/// Memory state for serialization.
#[derive(Debug, Clone)]
pub struct MemoryState {
    /// Serialized semantic memory entries.
    pub semantic: Vec<u8>,
    /// Serialized episodic memory entries.
    pub episodic: Vec<u8>,
    /// Serialized procedural memory entries.
    pub procedural: Vec<u8>,
    /// Serialized TTL state.
    pub ttl: Vec<u8>,
}

impl Default for MemoryState {
    fn default() -> Self {
        Self {
            semantic: Vec::new(),
            episodic: Vec::new(),
            procedural: Vec::new(),
            ttl: Vec::new(),
        }
    }
}

/// Snapshot metadata.
#[derive(Debug, Clone)]
pub struct SnapshotMetadata {
    /// Snapshot format version.
    pub version: u8,
    /// Total size in bytes.
    pub total_size: usize,
    /// CRC32 checksum.
    pub checksum: u32,
}

/// Error type for snapshot operations.
#[derive(Debug)]
pub enum SnapshotError {
    /// IO error during read/write.
    Io(io::Error),
    /// Invalid magic bytes.
    InvalidMagic,
    /// Unsupported version.
    UnsupportedVersion(u8),
    /// CRC checksum mismatch.
    ChecksumMismatch { expected: u32, actual: u32 },
    /// Data corruption or truncation.
    CorruptedData(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::InvalidMagic => write!(f, "Invalid snapshot magic bytes"),
            Self::UnsupportedVersion(v) => write!(f, "Unsupported snapshot version: {v}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "Checksum mismatch: expected {expected:08x}, got {actual:08x}"
                )
            }
            Self::CorruptedData(msg) => write!(f, "Corrupted data: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for SnapshotError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Creates a snapshot from memory state.
///
/// # Arguments
///
/// * `state` - Memory state to serialize
///
/// # Returns
///
/// Serialized snapshot bytes.
#[must_use]
pub fn create_snapshot(state: &MemoryState) -> Vec<u8> {
    let total_size = 4
        + 1
        + 8
        + state.semantic.len()
        + 8
        + state.episodic.len()
        + 8
        + state.procedural.len()
        + 8
        + state.ttl.len()
        + 4;
    let mut buf = Vec::with_capacity(total_size);

    buf.extend_from_slice(SNAPSHOT_MAGIC);
    buf.push(SNAPSHOT_VERSION);

    buf.extend_from_slice(&(state.semantic.len() as u64).to_le_bytes());
    buf.extend_from_slice(&state.semantic);

    buf.extend_from_slice(&(state.episodic.len() as u64).to_le_bytes());
    buf.extend_from_slice(&state.episodic);

    buf.extend_from_slice(&(state.procedural.len() as u64).to_le_bytes());
    buf.extend_from_slice(&state.procedural);

    buf.extend_from_slice(&(state.ttl.len() as u64).to_le_bytes());
    buf.extend_from_slice(&state.ttl);

    let crc = crc32_hash(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());

    buf
}

/// Loads a snapshot from bytes.
///
/// # Arguments
///
/// * `data` - Snapshot bytes
///
/// # Errors
///
/// Returns error if snapshot is invalid or corrupted.
pub fn load_snapshot(data: &[u8]) -> Result<MemoryState, SnapshotError> {
    const MIN_SIZE: usize = 4 + 1 + 8 + 8 + 8 + 8 + 4; // magic + version + 4 lengths + crc

    if data.len() < MIN_SIZE {
        return Err(SnapshotError::CorruptedData(
            "Snapshot too small".to_string(),
        ));
    }

    if &data[0..4] != SNAPSHOT_MAGIC {
        return Err(SnapshotError::InvalidMagic);
    }

    let version = data[4];
    if version != SNAPSHOT_VERSION {
        return Err(SnapshotError::UnsupportedVersion(version));
    }

    let stored_crc = u32::from_le_bytes(
        data[data.len() - 4..]
            .try_into()
            .map_err(|_| SnapshotError::CorruptedData("Invalid CRC bytes".to_string()))?,
    );
    let computed_crc = crc32_hash(&data[..data.len() - 4]);

    if stored_crc != computed_crc {
        return Err(SnapshotError::ChecksumMismatch {
            expected: stored_crc,
            actual: computed_crc,
        });
    }

    let mut offset = 5;

    let semantic_len = read_u64(&data[offset..])? as usize;
    offset += 8;
    if offset + semantic_len > data.len() - 4 {
        return Err(SnapshotError::CorruptedData(
            "Semantic data truncated".to_string(),
        ));
    }
    let semantic = data[offset..offset + semantic_len].to_vec();
    offset += semantic_len;

    let episodic_len = read_u64(&data[offset..])? as usize;
    offset += 8;
    if offset + episodic_len > data.len() - 4 {
        return Err(SnapshotError::CorruptedData(
            "Episodic data truncated".to_string(),
        ));
    }
    let episodic = data[offset..offset + episodic_len].to_vec();
    offset += episodic_len;

    let procedural_len = read_u64(&data[offset..])? as usize;
    offset += 8;
    if offset + procedural_len > data.len() - 4 {
        return Err(SnapshotError::CorruptedData(
            "Procedural data truncated".to_string(),
        ));
    }
    let procedural = data[offset..offset + procedural_len].to_vec();
    offset += procedural_len;

    let ttl_len = read_u64(&data[offset..])? as usize;
    offset += 8;
    if offset + ttl_len > data.len() - 4 {
        return Err(SnapshotError::CorruptedData(
            "TTL data truncated".to_string(),
        ));
    }
    let ttl = data[offset..offset + ttl_len].to_vec();

    Ok(MemoryState {
        semantic,
        episodic,
        procedural,
        ttl,
    })
}

/// Saves a snapshot to a file.
///
/// Uses atomic write (temp file + rename) for safety.
///
/// # Errors
///
/// Returns error if file operations fail.
pub fn save_snapshot_to_file<P: AsRef<Path>>(
    path: P,
    state: &MemoryState,
) -> Result<(), SnapshotError> {
    let path = path.as_ref();
    let snapshot_data = create_snapshot(state);

    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(&snapshot_data)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&temp_path, path)?;

    Ok(())
}

/// Loads a snapshot from a file.
///
/// # Errors
///
/// Returns error if file operations fail or snapshot is invalid.
pub fn load_snapshot_from_file<P: AsRef<Path>>(path: P) -> Result<MemoryState, SnapshotError> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    load_snapshot(&data)
}

/// Helper to read u64 from bytes.
fn read_u64(data: &[u8]) -> Result<u64, SnapshotError> {
    if data.len() < 8 {
        return Err(SnapshotError::CorruptedData(
            "Not enough bytes for u64".to_string(),
        ));
    }
    Ok(u64::from_le_bytes(data[0..8].try_into().map_err(|_| {
        SnapshotError::CorruptedData("Invalid u64 bytes".to_string())
    })?))
}

/// Snapshot manager for versioned snapshots.
pub struct SnapshotManager {
    /// Base directory for snapshots.
    base_path: std::path::PathBuf,
    /// Maximum number of snapshots to retain.
    max_snapshots: usize,
}

impl SnapshotManager {
    /// Creates a new snapshot manager.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory for storing snapshots
    /// * `max_snapshots` - Maximum number of snapshots to retain
    pub fn new<P: AsRef<Path>>(base_path: P, max_snapshots: usize) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            max_snapshots,
        }
    }

    /// Creates a new versioned snapshot.
    ///
    /// # Returns
    ///
    /// The version number of the created snapshot.
    ///
    /// # Errors
    ///
    /// Returns error if file operations fail.
    pub fn create_versioned_snapshot(&self, state: &MemoryState) -> Result<u64, SnapshotError> {
        std::fs::create_dir_all(&self.base_path)?;

        let version = self.next_version()?;
        let filename = format!("snapshot_{version:08}.vamm");
        let path = self.base_path.join(filename);

        save_snapshot_to_file(&path, state)?;
        self.cleanup_old_snapshots()?;

        Ok(version)
    }

    /// Loads the latest snapshot.
    ///
    /// # Errors
    ///
    /// Returns error if no snapshots exist or loading fails.
    pub fn load_latest(&self) -> Result<(u64, MemoryState), SnapshotError> {
        let version = self
            .latest_version()?
            .ok_or_else(|| SnapshotError::CorruptedData("No snapshots found".to_string()))?;
        let state = self.load_version(version)?;
        Ok((version, state))
    }

    /// Loads a specific snapshot version.
    ///
    /// # Errors
    ///
    /// Returns error if version doesn't exist or loading fails.
    pub fn load_version(&self, version: u64) -> Result<MemoryState, SnapshotError> {
        let filename = format!("snapshot_{version:08}.vamm");
        let path = self.base_path.join(filename);
        load_snapshot_from_file(&path)
    }

    /// Lists all available snapshot versions.
    ///
    /// # Errors
    ///
    /// Returns error if directory operations fail.
    pub fn list_versions(&self) -> Result<Vec<u64>, SnapshotError> {
        if !self.base_path.exists() {
            return Ok(Vec::new());
        }

        let mut versions = Vec::new();
        for entry in std::fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if filename_str.starts_with("snapshot_") && filename_str.ends_with(".vamm") {
                if let Some(version_str) = filename_str
                    .strip_prefix("snapshot_")
                    .and_then(|s| s.strip_suffix(".vamm"))
                {
                    if let Ok(version) = version_str.parse::<u64>() {
                        versions.push(version);
                    }
                }
            }
        }

        versions.sort();
        Ok(versions)
    }

    /// Returns the latest snapshot version.
    fn latest_version(&self) -> Result<Option<u64>, SnapshotError> {
        Ok(self.list_versions()?.into_iter().max())
    }

    /// Returns the next version number.
    fn next_version(&self) -> Result<u64, SnapshotError> {
        Ok(self.latest_version()?.map_or(1, |v| v + 1))
    }

    /// Removes old snapshots beyond the retention limit.
    fn cleanup_old_snapshots(&self) -> Result<(), SnapshotError> {
        let versions = self.list_versions()?;
        if versions.len() <= self.max_snapshots {
            return Ok(());
        }

        let to_remove = versions.len() - self.max_snapshots;
        for version in versions.into_iter().take(to_remove) {
            let filename = format!("snapshot_{version:08}.vamm");
            let path = self.base_path.join(filename);
            let _ = std::fs::remove_file(path);
        }

        Ok(())
    }
}
