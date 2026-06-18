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
        let entry = entry.expect("TTL entry should exist after set_ttl");
        let now = MemoryTtl::now();
        assert!(
            entry.expires_at >= now + 3599 && entry.expires_at <= now + 3601,
            "expires_at should be ~now+3600, got {} (now={})",
            entry.expires_at,
            now
        );
        assert!(
            entry.created_at <= entry.expires_at,
            "created_at must precede expiry"
        );
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

        let orig_sem = ttl.get(SEM, 1).expect("sem entry");
        let orig_epi = ttl.get(MemoryKind::Episodic, 2).expect("epi entry");

        let data = ttl.serialize();
        let restored = MemoryTtl::deserialize(&data).expect("Failed to deserialize");

        assert!(restored.get(SEM, 1).is_some());
        assert!(restored.get(MemoryKind::Episodic, 2).is_some());
        // The kind must round-trip: episodic id 2 must NOT be visible as semantic.
        assert!(restored.get(SEM, 2).is_none());

        // The expires_at / created_at values must survive the binary round-trip.
        let r_sem = restored.get(SEM, 1).expect("restored sem");
        assert_eq!(r_sem.expires_at, orig_sem.expires_at);
        assert_eq!(r_sem.created_at, orig_sem.created_at);
        let r_epi = restored.get(MemoryKind::Episodic, 2).expect("restored epi");
        assert_eq!(r_epi.expires_at, orig_epi.expires_at);
        assert_eq!(r_epi.created_at, orig_epi.created_at);
        // The two distinct TTLs (3600 vs 7200) must not collapse into one.
        assert!(r_epi.expires_at > r_sem.expires_at);
    }

    #[test]
    fn test_eviction_config_default() {
        let config = EvictionConfig::default();
        // Consolidation must be ENABLED by default — memory.rs gates on `> 0`,
        // so a default of 0 would silently disable episodic->semantic consolidation.
        assert!(
            config.consolidation_age_threshold > 0,
            "default must keep consolidation enabled"
        );
        assert_eq!(
            config.consolidation_age_threshold,
            7 * 24 * 60 * 60,
            "default consolidation window is 7 days"
        );
        // Confidence threshold must be a valid probability in (0, 1).
        assert!(config.min_confidence_threshold > 0.0 && config.min_confidence_threshold < 1.0);
        // Per-cycle cap must be a positive bound (0 would evict nothing / loop).
        assert!(config.max_entries_per_cycle > 0);
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
