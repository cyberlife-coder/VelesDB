//! TTL (Time-To-Live) and eviction management for `AgentMemory`.
//!
//! Provides automatic expiration and eviction policies for memory entries:
//! - TTL-based expiration for all memory subsystems
//! - Consolidation policy: migrate old episodic events to semantic memory
//! - Confidence-based eviction for procedural memory

// Reason: u64 to usize casts are for deserialization counts. Data is created/loaded
// on the same architecture, and counts represent actual serialized entry counts.
// These values are validated against buffer bounds before use.
#![allow(clippy::cast_possible_truncation)]

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Identifies which memory subsystem owns a TTL entry.
///
/// The three subsystems (`Semantic`, `Episodic`, `Procedural`) allocate ids
/// independently, so a bare `u64` id can collide across them. Keying the TTL
/// map by `(MemoryKind, u64)` keeps each subsystem's expiry namespace isolated:
/// a TTL on semantic id `5` never marks episodic id `5` expired, and
/// `auto_expire` only deletes the row in the owning collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    /// Long-term knowledge facts (`SemanticMemory`).
    Semantic,
    /// Event timeline (`EpisodicMemory`).
    Episodic,
    /// Learned procedures (`ProceduralMemory`).
    Procedural,
}

impl MemoryKind {
    /// One-byte tag used in the serialized TTL format.
    const fn tag(self) -> u8 {
        match self {
            Self::Semantic => 0,
            Self::Episodic => 1,
            Self::Procedural => 2,
        }
    }

    /// Reconstructs a `MemoryKind` from its serialized tag.
    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Semantic),
            1 => Some(Self::Episodic),
            2 => Some(Self::Procedural),
            _ => None,
        }
    }
}

/// Composite key uniquely identifying a TTL entry across subsystems.
type TtlKey = (MemoryKind, u64);

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
/// Entries are keyed by `(MemoryKind, u64)` so the three subsystems never
/// cross-expire one another even when they reuse the same numeric id.
pub struct MemoryTtl {
    /// Map of `(subsystem, entry ID)` to TTL information.
    entries: RwLock<FxHashMap<TtlKey, TtlEntry>>,
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
            .map_or(0, |d| d.as_secs())
    }

    /// Sets a TTL on an entry.
    ///
    /// # Arguments
    ///
    /// * `kind` - Owning subsystem (keeps namespaces isolated)
    /// * `id` - Entry identifier
    /// * `ttl_seconds` - Time-to-live in seconds from now
    pub fn set_ttl(&self, kind: MemoryKind, id: u64, ttl_seconds: u64) {
        self.set_expiry(kind, id, Self::now().saturating_add(ttl_seconds));
    }

    /// Sets a TTL entry from a precomputed absolute expiry timestamp.
    ///
    /// Used by the durable-TTL write path (which persists `_veles_expires_at` in the
    /// point payload) and by the reopen path that rebuilds this map from
    /// payloads, so both sides share the exact same expiry instant.
    pub fn set_expiry(&self, kind: MemoryKind, id: u64, expires_at: u64) {
        let entry = TtlEntry {
            expires_at,
            created_at: Self::now(),
        };
        self.entries.write().insert((kind, id), entry);
    }

    /// Removes TTL tracking for an entry.
    pub fn remove(&self, kind: MemoryKind, id: u64) {
        self.entries.write().remove(&(kind, id));
    }

    /// Returns the `(kind, id)` keys of all expired entries.
    ///
    /// Each key carries its owning subsystem, so `auto_expire` deletes only in
    /// the collection that actually holds the id.
    #[must_use]
    pub fn get_expired(&self) -> Vec<TtlKey> {
        let now = Self::now();
        self.entries
            .read()
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(&key, _)| key)
            .collect()
    }

    /// Returns the number of currently-expired entries for a single subsystem.
    ///
    /// Used by the search path to over-fetch enough candidates so that
    /// filtering out expired-but-not-yet-deleted points still yields up to `k`
    /// live results. Scoped to `kind` so an unrelated subsystem's expired
    /// entries do not inflate the over-fetch.
    #[must_use]
    pub fn expired_count(&self, kind: MemoryKind) -> usize {
        let now = Self::now();
        self.entries
            .read()
            .iter()
            .filter(|((k, _), entry)| *k == kind && entry.expires_at <= now)
            .count()
    }

    /// Removes expired entries from tracking and returns their keys.
    pub fn expire(&self) -> Vec<TtlKey> {
        let now = Self::now();
        let mut entries = self.entries.write();
        let expired: Vec<TtlKey> = entries
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(&key, _)| key)
            .collect();

        for key in &expired {
            entries.remove(key);
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
    pub fn is_expired(&self, kind: MemoryKind, id: u64) -> bool {
        let now = Self::now();
        self.entries
            .read()
            .get(&(kind, id))
            .is_some_and(|entry| entry.expires_at <= now)
    }

    /// Returns the TTL entry for a `(kind, id)` if it exists.
    #[must_use]
    pub fn get(&self, kind: MemoryKind, id: u64) -> Option<TtlEntry> {
        self.entries.read().get(&(kind, id)).cloned()
    }

    /// Clears all TTL entries.
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// Replaces all entries with those from another `MemoryTtl` instance.
    pub fn replace_from(&self, other: &MemoryTtl) {
        let other_entries = other.entries.read();
        let mut self_entries = self.entries.write();
        self_entries.clear();
        for (&key, entry) in other_entries.iter() {
            self_entries.insert(key, entry.clone());
        }
    }

    /// Size in bytes of one serialized TTL entry: `tag(1) + id(8) + expires(8) + created(8)`.
    const ENTRY_SIZE: usize = 1 + 8 + 8 + 8;

    /// Serializes TTL state to bytes for snapshot support.
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let entries = self.entries.read();
        let count = entries.len();
        let mut buf = Vec::with_capacity(8 + count * Self::ENTRY_SIZE);

        buf.extend_from_slice(&(count as u64).to_le_bytes());

        for (&(kind, id), entry) in entries.iter() {
            buf.push(kind.tag());
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
        let count = super::memory_helpers::validate_binary_header(data, Self::ENTRY_SIZE)?;
        let mut entries = FxHashMap::default();
        entries.reserve(count);

        for i in 0..count {
            let offset = 8 + i * Self::ENTRY_SIZE;
            let kind = MemoryKind::from_tag(data[offset])?;
            let id = u64::from_le_bytes(data[offset + 1..offset + 9].try_into().ok()?);
            let expires_at = u64::from_le_bytes(data[offset + 9..offset + 17].try_into().ok()?);
            let created_at = u64::from_le_bytes(data[offset + 17..offset + 25].try_into().ok()?);

            entries.insert(
                (kind, id),
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
    /// `true` when consolidation processed the per-cycle cap and more old
    /// episodes remain — the caller should run `auto_expire` again to drain them.
    pub consolidation_truncated: bool,
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
