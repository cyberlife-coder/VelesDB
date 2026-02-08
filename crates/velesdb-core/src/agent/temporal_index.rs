#![allow(missing_docs)] // Documentation will be added in follow-up PR
//! Temporal index for efficient time-based queries in EpisodicMemory.
//!
//! Replaces the naive O(N) scan with an indexed structure that provides:
//! - O(log N) range queries by timestamp
//! - O(1) insertion and deletion
//! - Efficient retrieval of recent events

// SAFETY: u64 to usize casts are for deserialization counts. Data is created/loaded
// on the same architecture, and counts represent actual serialized entry counts.
// These values are validated against buffer bounds before use.
#![allow(clippy::cast_possible_truncation)]

use parking_lot::RwLock;
use std::collections::BTreeMap;

/// Entry in the temporal index.
#[derive(Debug, Clone)]
pub struct TemporalEntry {
    /// Event ID.
    pub id: u64,
    /// Event timestamp (Unix seconds or milliseconds).
    pub timestamp: i64,
}

/// Temporal index using a B-tree for efficient range queries.
///
/// Maintains a sorted index of timestamps to event IDs, allowing
/// efficient retrieval of events within time ranges.
pub struct TemporalIndex {
    /// B-tree mapping timestamp to list of event IDs.
    /// Multiple events can have the same timestamp.
    by_timestamp: RwLock<BTreeMap<i64, Vec<u64>>>,
    /// Reverse mapping from ID to timestamp for efficient removal.
    id_to_timestamp: RwLock<std::collections::HashMap<u64, i64>>,
}

impl Default for TemporalIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalIndex {
    /// Creates a new empty temporal index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_timestamp: RwLock::new(BTreeMap::new()),
            id_to_timestamp: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Inserts an entry into the index.
    ///
    /// If the ID already exists, it will be updated with the new timestamp.
    pub fn insert(&self, id: u64, timestamp: i64) {
        let mut id_to_ts = self.id_to_timestamp.write();
        let mut by_ts = self.by_timestamp.write();

        if let Some(&old_ts) = id_to_ts.get(&id) {
            if old_ts == timestamp {
                return;
            }
            if let Some(ids) = by_ts.get_mut(&old_ts) {
                ids.retain(|&x| x != id);
                if ids.is_empty() {
                    by_ts.remove(&old_ts);
                }
            }
        }

        id_to_ts.insert(id, timestamp);
        by_ts.entry(timestamp).or_default().push(id);
    }

    /// Removes an entry from the index.
    pub fn remove(&self, id: u64) {
        let mut id_to_ts = self.id_to_timestamp.write();
        let mut by_ts = self.by_timestamp.write();

        if let Some(ts) = id_to_ts.remove(&id) {
            if let Some(ids) = by_ts.get_mut(&ts) {
                ids.retain(|&x| x != id);
                if ids.is_empty() {
                    by_ts.remove(&ts);
                }
            }
        }
    }

    /// Returns IDs of events within a timestamp range (inclusive).
    ///
    /// # Arguments
    ///
    /// * `start` - Start timestamp (inclusive)
    /// * `end` - End timestamp (inclusive)
    ///
    /// # Returns
    ///
    /// Vector of (id, timestamp) pairs within the range.
    #[must_use]
    pub fn range(&self, start: i64, end: i64) -> Vec<TemporalEntry> {
        let by_ts = self.by_timestamp.read();
        let mut results = Vec::new();

        for (&ts, ids) in by_ts.range(start..=end) {
            for &id in ids {
                results.push(TemporalEntry { id, timestamp: ts });
            }
        }

        results
    }

    /// Returns the most recent events up to a limit.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of events to return
    /// * `since_timestamp` - Optional filter for events after this time
    ///
    /// # Returns
    ///
    /// Vector of (id, timestamp) pairs, ordered by timestamp descending.
    #[must_use]
    pub fn recent(&self, limit: usize, since_timestamp: Option<i64>) -> Vec<TemporalEntry> {
        let by_ts = self.by_timestamp.read();
        let mut results = Vec::with_capacity(limit);

        for (&ts, ids) in by_ts.iter().rev() {
            if let Some(since) = since_timestamp {
                if ts <= since {
                    continue;
                }
            }

            for &id in ids.iter().rev() {
                results.push(TemporalEntry { id, timestamp: ts });
                if results.len() >= limit {
                    return results;
                }
            }
        }

        results
    }

    /// Returns events older than the specified timestamp.
    ///
    /// # Arguments
    ///
    /// * `before_timestamp` - Cutoff timestamp (exclusive)
    /// * `limit` - Maximum number of events to return
    ///
    /// # Returns
    ///
    /// Vector of (id, timestamp) pairs, ordered by timestamp ascending.
    #[must_use]
    pub fn older_than(&self, before_timestamp: i64, limit: usize) -> Vec<TemporalEntry> {
        let by_ts = self.by_timestamp.read();
        let mut results = Vec::with_capacity(limit);

        for (&ts, ids) in by_ts.iter() {
            if ts >= before_timestamp {
                break;
            }

            for &id in ids {
                results.push(TemporalEntry { id, timestamp: ts });
                if results.len() >= limit {
                    return results;
                }
            }
        }

        results
    }

    /// Returns the timestamp for an ID if it exists.
    #[must_use]
    pub fn get_timestamp(&self, id: u64) -> Option<i64> {
        self.id_to_timestamp.read().get(&id).copied()
    }

    /// Returns the total number of indexed entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_timestamp.read().len()
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id_to_timestamp.read().is_empty()
    }

    /// Clears all entries from the index.
    pub fn clear(&self) {
        let mut id_to_ts = self.id_to_timestamp.write();
        let mut by_ts = self.by_timestamp.write();
        by_ts.clear();
        id_to_ts.clear();
    }

    /// Returns all IDs in the index.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.id_to_timestamp.read().keys().copied().collect()
    }

    /// Serializes the index to bytes for snapshot support.
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let id_to_ts = self.id_to_timestamp.read();
        let count = id_to_ts.len();
        let mut buf = Vec::with_capacity(8 + count * 16);

        buf.extend_from_slice(&(count as u64).to_le_bytes());

        for (&id, &ts) in id_to_ts.iter() {
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&ts.to_le_bytes());
        }

        buf
    }

    /// Deserializes the index from bytes.
    ///
    /// # Returns
    ///
    /// `None` if the data is malformed.
    #[must_use]
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let count = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;
        let expected_len = 8 + count * 16;

        if data.len() != expected_len {
            return None;
        }

        let index = Self::new();

        for i in 0..count {
            let offset = 8 + i * 16;
            let id = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let ts = i64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            index.insert(id, ts);
        }

        Some(index)
    }

    /// Rebuilds the index from a collection of entries.
    ///
    /// This is useful for initializing the index from existing data.
    pub fn rebuild_from_entries(&self, entries: impl IntoIterator<Item = (u64, i64)>) {
        self.clear();
        for (id, timestamp) in entries {
            self.insert(id, timestamp);
        }
    }
}

/// Statistics about the temporal index.
#[derive(Debug, Clone, Default)]
pub struct TemporalIndexStats {
    /// Total number of entries.
    pub entry_count: usize,
    /// Number of unique timestamps.
    pub unique_timestamps: usize,
    /// Minimum timestamp in the index.
    pub min_timestamp: Option<i64>,
    /// Maximum timestamp in the index.
    pub max_timestamp: Option<i64>,
}

impl TemporalIndex {
    /// Returns statistics about the index.
    #[must_use]
    pub fn stats(&self) -> TemporalIndexStats {
        let by_ts = self.by_timestamp.read();
        let id_to_ts = self.id_to_timestamp.read();

        TemporalIndexStats {
            entry_count: id_to_ts.len(),
            unique_timestamps: by_ts.len(),
            min_timestamp: by_ts.keys().next().copied(),
            max_timestamp: by_ts.keys().next_back().copied(),
        }
    }
}
