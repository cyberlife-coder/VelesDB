//! Tests for temporal index functionality.

#[cfg(test)]
mod tests {
    use super::super::temporal_index::*;

    #[test]
    fn test_temporal_index_insert_and_get() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);

        assert_eq!(index.get_timestamp(1), Some(1000));
        assert_eq!(index.get_timestamp(2), Some(2000));
        assert_eq!(index.get_timestamp(3), Some(3000));
        assert_eq!(index.get_timestamp(4), None);
    }

    #[test]
    fn test_temporal_index_remove() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);

        index.remove(1);

        assert_eq!(index.get_timestamp(1), None);
        assert_eq!(index.get_timestamp(2), Some(2000));
    }

    #[test]
    fn test_temporal_index_recent() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);
        index.insert(4, 4000);

        let recent = index.recent(2, None);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, 4);
        assert_eq!(recent[1].id, 3);
    }

    #[test]
    fn test_temporal_index_recent_with_since() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);
        index.insert(4, 4000);

        let recent = index.recent(10, Some(2000));
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, 4);
        assert_eq!(recent[1].id, 3);
    }

    #[test]
    fn test_temporal_index_older_than() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);
        index.insert(4, 4000);

        let old = index.older_than(3000, 10);
        assert_eq!(old.len(), 2);
        assert_eq!(old[0].id, 1);
        assert_eq!(old[1].id, 2);
    }

    #[test]
    fn test_temporal_index_range() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);
        index.insert(4, 4000);

        let range = index.range(2000, 3000);
        assert_eq!(range.len(), 2);
    }

    #[test]
    fn test_temporal_index_serialize_deserialize() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);

        let data = index.serialize();
        let restored = TemporalIndex::deserialize(&data).expect("Failed to deserialize");

        assert_eq!(restored.get_timestamp(1), Some(1000));
        assert_eq!(restored.get_timestamp(2), Some(2000));
        assert_eq!(restored.get_timestamp(3), Some(3000));
    }

    /// #897: a binary header whose `count * entry_size` overflows `usize` so the
    /// product wraps to match `data.len()` must be rejected, not blindly trusted
    /// (which would `reserve(count)` and abort/OOM).
    #[test]
    fn test_deserialize_rejects_overflowing_count() {
        // entry_size = 16 for TemporalIndex. With an 8-byte buffer (zero payload),
        // count = 2^60 makes count*16 wrap to 0 (mod 2^64), so the pre-fix length
        // check `8 + count*16 == data.len()` spuriously passed.
        let count: u64 = 1u64 << 60;
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&count.to_le_bytes());

        let restored = TemporalIndex::deserialize(&data);
        assert!(
            restored.is_none(),
            "overflowing count header must be rejected"
        );
    }

    /// #897: a huge caller-supplied `limit` must not pre-allocate beyond the number
    /// of indexed entries.
    #[test]
    fn test_recent_huge_limit_does_not_overallocate() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);

        // A `limit` near usize::MAX must not abort/OOM; results are bounded by data.
        let recent = index.recent(usize::MAX, None);
        assert_eq!(recent.len(), 2);

        let older = index.older_than(i64::MAX, usize::MAX);
        assert_eq!(older.len(), 2);
    }

    #[test]
    fn test_temporal_index_stats() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 2000);
        index.insert(3, 3000);

        let stats = index.stats();
        assert_eq!(stats.entry_count, 3);
        assert_eq!(stats.unique_timestamps, 3);
        assert_eq!(stats.min_timestamp, Some(1000));
        assert_eq!(stats.max_timestamp, Some(3000));
    }

    /// Multiple ids sharing one timestamp bucket: `entry_count` counts every id
    /// while `unique_timestamps` counts the single bucket. Removing one id keeps
    /// the bucket alive until the last id leaves.
    #[test]
    fn test_temporal_index_shared_timestamp_bucket() {
        let index = TemporalIndex::new();
        index.insert(1, 5000);
        index.insert(2, 5000);
        index.insert(3, 5000);

        let stats = index.stats();
        assert_eq!(stats.entry_count, 3, "three ids");
        assert_eq!(stats.unique_timestamps, 1, "one shared bucket");

        // Removing one id leaves the bucket populated.
        index.remove(2);
        let after = index.stats();
        assert_eq!(after.entry_count, 2);
        assert_eq!(after.unique_timestamps, 1);
        assert_eq!(index.get_timestamp(2), None);

        // Draining the bucket entirely drops it.
        index.remove(1);
        index.remove(3);
        let empty = index.stats();
        assert_eq!(empty.entry_count, 0);
        assert_eq!(empty.unique_timestamps, 0);
        assert_eq!(empty.min_timestamp, None);
    }

    /// Re-inserting the same id with a new timestamp must move it out of its old
    /// bucket (no ghost left behind) and into the new one.
    #[test]
    fn test_temporal_index_reinsert_moves_bucket() {
        let index = TemporalIndex::new();
        index.insert(1, 1000);
        index.insert(2, 1000); // shares the 1000 bucket with id 1

        // Move id 1 to a new timestamp.
        index.insert(1, 9000);

        assert_eq!(index.get_timestamp(1), Some(9000));
        // The old bucket still holds id 2 only.
        let at_1000 = index.range(1000, 1000);
        assert_eq!(at_1000.len(), 1);
        assert_eq!(at_1000[0].id, 2);
        // The new bucket holds exactly id 1 (no duplicate ghost).
        let at_9000 = index.range(9000, 9000);
        assert_eq!(at_9000.len(), 1);
        assert_eq!(at_9000[0].id, 1);

        let stats = index.stats();
        assert_eq!(stats.entry_count, 2, "still two distinct ids, no ghost");
        assert_eq!(stats.unique_timestamps, 2);
    }

    /// #1042: `stats()` must lock `id_to_timestamp` before `by_timestamp`, the
    /// same order as every mutator. Interleaving `stats()` with `insert`/`remove`
    /// on background threads must not deadlock. Under `--test-threads=1` the
    /// threads spawned here still run concurrently with each other.
    #[test]
    fn test_stats_lock_order_no_deadlock_under_contention() {
        use std::sync::Arc;
        use std::thread;

        let index = Arc::new(TemporalIndex::new());
        for id in 0i64..50 {
            index.insert(u64::try_from(id).unwrap(), id);
        }

        let writer = {
            let index = Arc::clone(&index);
            thread::spawn(move || {
                for round in 0i64..1000 {
                    let id = u64::try_from(round % 50).unwrap();
                    index.insert(id, round);
                    index.remove(id);
                }
            })
        };
        let reader = {
            let index = Arc::clone(&index);
            thread::spawn(move || {
                let mut last = 0;
                for _ in 0..1000 {
                    last = index.stats().entry_count;
                }
                last
            })
        };

        writer.join().expect("writer thread panicked");
        // If stats() inverted lock order, this join would hang (deadlock).
        let _ = reader.join().expect("reader thread panicked");
    }
}
