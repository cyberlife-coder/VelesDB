//! Tests for TTL functionality.

#[cfg(test)]
mod tests {
    use super::super::ttl::*;

    #[test]
    fn test_memory_ttl_set_and_get() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 3600);

        let entry = ttl.get(1);
        assert!(entry.is_some());
    }

    #[test]
    fn test_memory_ttl_remove() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 3600);
        ttl.remove(1);

        assert!(ttl.get(1).is_none());
    }

    #[test]
    fn test_memory_ttl_is_expired() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 0);

        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(ttl.is_expired(1));
    }

    #[test]
    fn test_memory_ttl_not_expired() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 3600);

        assert!(!ttl.is_expired(1));
    }

    #[test]
    fn test_memory_ttl_expire() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 0);
        ttl.set_ttl(2, 3600);

        std::thread::sleep(std::time::Duration::from_millis(10));
        let expired = ttl.expire();

        assert!(expired.contains(&1));
        assert!(!expired.contains(&2));
    }

    #[test]
    fn test_memory_ttl_serialize_deserialize() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(1, 3600);
        ttl.set_ttl(2, 7200);

        let data = ttl.serialize();
        let restored = MemoryTtl::deserialize(&data).expect("Failed to deserialize");

        assert!(restored.get(1).is_some());
        assert!(restored.get(2).is_some());
    }

    #[test]
    fn test_eviction_config_default() {
        let config = EvictionConfig::default();
        assert_eq!(config.consolidation_age_threshold, 7 * 24 * 60 * 60);
        assert!((config.min_confidence_threshold - 0.1).abs() < 0.001);
        assert_eq!(config.max_entries_per_cycle, 1000);
    }

    #[test]
    fn test_memory_ttl_merge_from() {
        let ttl1 = MemoryTtl::new();
        ttl1.set_ttl(1, 3600);

        let ttl2 = MemoryTtl::new();
        ttl2.set_ttl(2, 7200);

        ttl1.merge_from(&ttl2);

        assert!(ttl1.get(1).is_some());
        assert!(ttl1.get(2).is_some());
    }

    #[test]
    fn test_memory_ttl_replace_from() {
        let ttl1 = MemoryTtl::new();
        ttl1.set_ttl(1, 3600);

        let ttl2 = MemoryTtl::new();
        ttl2.set_ttl(2, 7200);

        ttl1.replace_from(&ttl2);

        assert!(ttl1.get(1).is_none());
        assert!(ttl1.get(2).is_some());
    }
}
