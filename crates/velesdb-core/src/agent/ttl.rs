#![allow(missing_docs)] // Documentation will be added in follow-up PR
//! TTL (Time-To-Live) and eviction management for `AgentMemory`.
//!
//! Provides automatic expiration and eviction policies for memory entries:
//! - TTL-based expiration for all memory subsystems
//! - Consolidation policy: migrate old episodic events to semantic memory
//! - Confidence-based eviction for procedural memory

// SAFETY: u64 to usize casts are for deserialization counts. Data is created/loaded
// on the same architecture, and counts represent actual serialized entry counts.
// These values are validated against buffer bounds before use.
#![allow(clippy::cast_possible_truncation)]

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// TTL entry tracking expiration time and metadata.
#[derive(Debug, Clone)]
pub struct TtlEntry {
    /// Expiration timestamp (Unix seconds).
    pub expires_at: u64,
    /// Original timestamp when the entry was created.
    pub created_at: u64,
}

/// Manages TTL for memory entries.
///
/// Thread-safe TTL tracker that can be shared across memory subsystems.
pub struct MemoryTtl {
    /// Map of entry ID to TTL information.
    entries: RwLock<FxHashMap<u64, TtlEntry>>,
}

impl Default for MemoryTtl {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryTtl {
    /// Creates a new TTL manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(FxHashMap::default()),
        }
    }

    /// Returns the current Unix timestamp in seconds.
    #[must_use]
    pub fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Sets a TTL on an entry.
    ///
    /// # Arguments
    ///
    /// * `id` - Entry identifier
    /// * `ttl_seconds` - Time-to-live in seconds from now
    pub fn set_ttl(&self, id: u64, ttl_seconds: u64) {
        let now = Self::now();
        let entry = TtlEntry {
            expires_at: now.saturating_add(ttl_seconds),
            created_at: now,
        };
        self.entries.write().insert(id, entry);
    }

    /// Sets a TTL with a specific creation timestamp.
    ///
    /// Useful for entries that were created in the past.
    pub fn set_ttl_with_created_at(&self, id: u64, ttl_seconds: u64, created_at: u64) {
        let entry = TtlEntry {
            expires_at: created_at.saturating_add(ttl_seconds),
            created_at,
        };
        self.entries.write().insert(id, entry);
    }

    /// Removes TTL tracking for an entry.
    pub fn remove(&self, id: u64) {
        self.entries.write().remove(&id);
    }

    /// Returns IDs of all expired entries.
    #[must_use]
    pub fn get_expired(&self) -> Vec<u64> {
        let now = Self::now();
        self.entries
            .read()
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Returns IDs of entries older than the specified age.
    ///
    /// # Arguments
    ///
    /// * `max_age_seconds` - Maximum age in seconds
    #[must_use]
    pub fn get_older_than(&self, max_age_seconds: u64) -> Vec<u64> {
        let cutoff = Self::now().saturating_sub(max_age_seconds);
        self.entries
            .read()
            .iter()
            .filter(|(_, entry)| entry.created_at < cutoff)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Removes expired entries from tracking and returns their IDs.
    pub fn expire(&self) -> Vec<u64> {
        let now = Self::now();
        let mut entries = self.entries.write();
        let expired: Vec<u64> = entries
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(&id, _)| id)
            .collect();

        for id in &expired {
            entries.remove(id);
        }

        expired
    }

    /// Returns the number of tracked entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Returns true if no entries are being tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// Checks if an entry has expired.
    #[must_use]
    pub fn is_expired(&self, id: u64) -> bool {
        let now = Self::now();
        self.entries
            .read()
            .get(&id)
            .is_some_and(|entry| entry.expires_at <= now)
    }

    /// Returns the TTL entry for an ID if it exists.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<TtlEntry> {
        self.entries.read().get(&id).cloned()
    }

    /// Clears all TTL entries.
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// Merges entries from another `MemoryTtl` instance.
    pub fn merge_from(&self, other: &MemoryTtl) {
        let other_entries = other.entries.read();
        let mut self_entries = self.entries.write();
        for (&id, entry) in other_entries.iter() {
            self_entries.insert(id, entry.clone());
        }
    }

    /// Replaces all entries with those from another `MemoryTtl` instance.
    pub fn replace_from(&self, other: &MemoryTtl) {
        let other_entries = other.entries.read();
        let mut self_entries = self.entries.write();
        self_entries.clear();
        for (&id, entry) in other_entries.iter() {
            self_entries.insert(id, entry.clone());
        }
    }

    /// Serializes TTL state to bytes for snapshot support.
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let entries = self.entries.read();
        let count = entries.len();
        let mut buf = Vec::with_capacity(8 + count * 24);

        buf.extend_from_slice(&(count as u64).to_le_bytes());

        for (&id, entry) in entries.iter() {
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&entry.expires_at.to_le_bytes());
            buf.extend_from_slice(&entry.created_at.to_le_bytes());
        }

        buf
    }

    /// Deserializes TTL state from bytes.
    ///
    /// # Errors
    ///
    /// Returns `None` if the data is malformed.
    #[must_use]
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let count = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;
        let expected_len = 8 + count * 24;

        if data.len() != expected_len {
            return None;
        }

        let mut entries = FxHashMap::default();
        entries.reserve(count);

        for i in 0..count {
            let offset = 8 + i * 24;
            let id = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let expires_at = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            let created_at = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().ok()?);

            entries.insert(
                id,
                TtlEntry {
                    expires_at,
                    created_at,
                },
            );
        }

        Some(Self {
            entries: RwLock::new(entries),
        })
    }
}

/// Result of an auto-expire operation.
#[derive(Debug, Default)]
pub struct ExpireResult {
    /// Number of semantic memory entries expired.
    pub semantic_expired: usize,
    /// Number of episodic memory entries expired.
    pub episodic_expired: usize,
    /// Number of procedural memory entries expired.
    pub procedural_expired: usize,
    /// Number of episodic entries consolidated to semantic memory.
    pub episodic_consolidated: usize,
    /// Number of procedural entries evicted due to low confidence.
    pub procedural_evicted: usize,
}

/// Configuration for eviction policies.
#[derive(Debug, Clone)]
pub struct EvictionConfig {
    /// Age threshold for episodic-to-semantic consolidation (seconds).
    /// Events older than this are candidates for consolidation.
    pub consolidation_age_threshold: u64,

    /// Minimum confidence threshold for procedural memory.
    /// Procedures below this confidence are candidates for eviction.
    pub min_confidence_threshold: f32,

    /// Maximum number of entries to process per eviction cycle.
    pub max_entries_per_cycle: usize,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            consolidation_age_threshold: 7 * 24 * 60 * 60, // 7 days
            min_confidence_threshold: 0.1,
            max_entries_per_cycle: 1000,
        }
    }
}
