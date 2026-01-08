//! Tests for `alloc_guard` module

use super::alloc_guard::*;
use std::alloc::{dealloc, Layout};

#[test]
fn test_alloc_guard_basic() {
    let layout = Layout::from_size_align(1024, 8).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");

    assert!(!guard.as_ptr().is_null());
    assert_eq!(guard.layout().size(), 1024);
    assert_eq!(guard.layout().align(), 8);
}

#[test]
fn test_alloc_guard_into_raw() {
    let layout = Layout::from_size_align(64, 8).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");
    let ptr = guard.into_raw();

    // Must manually deallocate
    assert!(!ptr.is_null());
    unsafe {
        dealloc(ptr, layout);
    }
}

#[test]
fn test_alloc_guard_zero_size() {
    let layout = Layout::from_size_align(0, 1).unwrap();
    assert!(AllocGuard::new(layout).is_none());
}

#[test]
fn test_alloc_guard_aligned() {
    // Cache-line aligned (64 bytes)
    let layout = Layout::from_size_align(256, 64).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");

    let addr = guard.as_ptr() as usize;
    assert_eq!(addr % 64, 0, "Not cache-line aligned");
}

#[test]
fn test_alloc_guard_cast() {
    let layout =
        Layout::from_size_align(std::mem::size_of::<f32>() * 10, std::mem::align_of::<f32>())
            .unwrap();

    let guard = AllocGuard::new(layout).expect("allocation failed");
    let float_ptr: *mut f32 = guard.cast();

    // Write some data
    #[allow(clippy::cast_precision_loss)]
    unsafe {
        for i in 0..10 {
            *float_ptr.add(i) = i as f32;
        }
    }

    // Read back
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    unsafe {
        for i in 0..10 {
            assert_eq!(*float_ptr.add(i), i as f32);
        }
    }
}

#[test]
fn test_alloc_guard_drop_frees_memory() {
    // This test verifies the guard deallocates on drop
    // We can't directly verify deallocation, but we can ensure no panic
    for _ in 0..1000 {
        let layout = Layout::from_size_align(1024, 8).unwrap();
        let _guard = AllocGuard::new(layout);
        // guard dropped here, memory freed
    }
}

#[test]
fn test_alloc_guard_panic_safety() {
    use std::panic;

    let layout = Layout::from_size_align(1024, 8).unwrap();

    // Simulate panic during operation
    let result = panic::catch_unwind(|| {
        let _guard = AllocGuard::new(layout).expect("allocation failed");
        panic!("simulated panic");
    });

    assert!(result.is_err());
    // Memory should have been freed by drop during unwind
}
