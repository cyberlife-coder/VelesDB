use super::known_category_mapping;
use velesdb_memory::ErrorCategory;

/// `ErrorCategory` is `non_exhaustive`, so the compiler no longer fails this
/// adapter when a category is added upstream — this test does, by walking the
/// authoritative list only the defining crate can produce. A category served
/// by the runtime fallback is a taxonomy hole, not a mapping.
#[test]
fn every_category_is_mapped_explicitly() {
    for category in ErrorCategory::ALL {
        assert!(
            known_category_mapping(*category).is_some(),
            "category {category:?} would fall through to the INTERNAL fallback; \
             map it explicitly in known_category_mapping; \
             the PyO3 adapter (velesdb-python agent_memory_service.rs) has the \
             same mapping and no test of its own — update it in the same change"
        );
    }
}
