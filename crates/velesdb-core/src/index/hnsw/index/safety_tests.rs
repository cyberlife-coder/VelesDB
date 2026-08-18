use super::*;

/// Compile-time assertion that `io_holder` field is declared AFTER `inner`.
#[test]
fn test_field_order_io_holder_after_inner() {
    use std::mem::offset_of;

    let inner_offset = offset_of!(HnswIndex, inner);
    let io_holder_offset = offset_of!(HnswIndex, io_holder);

    assert!(
        inner_offset < io_holder_offset,
        "CRITICAL: io_holder must be declared AFTER inner for correct drop order"
    );
}
