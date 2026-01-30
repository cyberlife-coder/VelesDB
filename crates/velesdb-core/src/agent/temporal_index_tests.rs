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
}
