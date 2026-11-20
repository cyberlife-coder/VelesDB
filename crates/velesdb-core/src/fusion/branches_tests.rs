//! The fused-vector count rule every fusing engine shares.

use super::*;

#[test]
fn a_fusion_takes_one_to_max_fused_vectors() {
    for count in [1, MAX_FUSED_VECTORS] {
        assert!(validate_fused_vector_count(count).is_ok(), "{count}");
    }
    let none = validate_fused_vector_count(0).expect_err("test: no vector");
    assert_eq!(
        none.to_string(),
        "[VELES-009] Configuration error: multi_query_search requires at least one vector"
    );
    let over = validate_fused_vector_count(MAX_FUSED_VECTORS + 1).expect_err("test: too many");
    assert_eq!(
        over.to_string(),
        format!(
            "[VELES-009] Configuration error: multi_query_search supports at most \
             {MAX_FUSED_VECTORS} vectors, got {}",
            MAX_FUSED_VECTORS + 1
        )
    );
}
