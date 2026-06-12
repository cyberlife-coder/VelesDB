//! Tests for TTL functionality.

#[cfg(test)]
mod tests {
    use super::super::ttl::*;

    // Convenience alias so the existing single-namespace assertions stay terse.
    const SEM: MemoryKind = MemoryKind::Semantic;

    #[test]
    fn test_memory_ttl_set_and_get() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 3600);

        let entry = ttl.get(SEM, 1);
        assert!(entry.is_some());
    }

    #[test]
    fn test_memory_ttl_remove() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 3600);
        ttl.remove(SEM, 1);

        assert!(ttl.get(SEM, 1).is_none());
    }

    #[test]
    fn test_memory_ttl_is_expired() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 0);

        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(ttl.is_expired(SEM, 1));
    }

    #[test]
    fn test_memory_ttl_not_expired() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 3600);

        assert!(!ttl.is_expired(SEM, 1));
    }

    #[test]
    fn test_memory_ttl_expired_count() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 0);
        ttl.set_ttl(SEM, 2, 0);
        ttl.set_ttl(SEM, 3, 3600);

        std::thread::sleep(std::time::Duration::from_millis(10));
        assert_eq!(ttl.expired_count(SEM), 2);
        assert_eq!(ttl.expired_count(SEM), ttl.get_expired().len());
    }

    #[test]
    fn test_memory_ttl_expire() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 0);
        ttl.set_ttl(SEM, 2, 3600);

        std::thread::sleep(std::time::Duration::from_millis(10));
        let expired = ttl.expire();

        assert!(expired.contains(&(SEM, 1)));
        assert!(!expired.contains(&(SEM, 2)));
    }

    #[test]
    fn test_memory_ttl_serialize_deserialize() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(SEM, 1, 3600);
        ttl.set_ttl(MemoryKind::Episodic, 2, 7200);

        let data = ttl.serialize();
        let restored = MemoryTtl::deserialize(&data).expect("Failed to deserialize");

        assert!(restored.get(SEM, 1).is_some());
        assert!(restored.get(MemoryKind::Episodic, 2).is_some());
        // The kind must round-trip: episodic id 2 must NOT be visible as semantic.
        assert!(restored.get(SEM, 2).is_none());
    }

    #[test]
    fn test_eviction_config_default() {
        let config = EvictionConfig::default();
        assert_eq!(config.consolidation_age_threshold, 7 * 24 * 60 * 60);
        assert!((config.min_confidence_threshold - 0.1).abs() < 0.001);
        assert_eq!(config.max_entries_per_cycle, 1000);
    }

    #[test]
    fn test_memory_ttl_replace_from() {
        let ttl1 = MemoryTtl::new();
        ttl1.set_ttl(SEM, 1, 3600);

        let ttl2 = MemoryTtl::new();
        ttl2.set_ttl(SEM, 2, 7200);

        ttl1.replace_from(&ttl2);

        assert!(ttl1.get(SEM, 1).is_none());
        assert!(ttl1.get(SEM, 2).is_some());
    }

    /// #1041: the same numeric id in two subsystems must not cross-expire.
    /// A TTL on semantic id=5 must leave episodic id=5 untouched.
    #[test]
    fn test_ttl_kinds_do_not_cross_expire() {
        let ttl = MemoryTtl::new();
        ttl.set_ttl(MemoryKind::Semantic, 5, 0); // expires immediately
        ttl.set_ttl(MemoryKind::Episodic, 5, 3600); // long-lived

        std::thread::sleep(std::time::Duration::from_millis(10));

        assert!(ttl.is_expired(MemoryKind::Semantic, 5));
        assert!(
            !ttl.is_expired(MemoryKind::Episodic, 5),
            "episodic id 5 must NOT inherit semantic id 5's expiry"
        );

        // expired_count is scoped per-kind.
        assert_eq!(ttl.expired_count(MemoryKind::Semantic), 1);
        assert_eq!(ttl.expired_count(MemoryKind::Episodic), 0);

        // get_expired yields the owning kind so auto_expire deletes correctly.
        let expired = ttl.get_expired();
        assert_eq!(expired, vec![(MemoryKind::Semantic, 5)]);
    }

    /// Ids larger than 2^53 (beyond f64/JS integer precision) must key exactly
    /// and not collide with a neighbouring id.
    #[test]
    fn test_ttl_high_precision_ids_distinct() {
        let ttl = MemoryTtl::new();
        let big = (1u64 << 53) + 1;
        ttl.set_ttl(SEM, big, 3600);

        assert!(ttl.get(SEM, big).is_some());
        assert!(ttl.get(SEM, big + 1).is_none());
        assert!(ttl.get(SEM, 1u64 << 53).is_none());
    }
}
