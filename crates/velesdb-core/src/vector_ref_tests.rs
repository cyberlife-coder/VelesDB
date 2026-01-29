//! Tests for `vector_ref` module - Zero-copy vector references.

#![allow(clippy::float_cmp)]
#![allow(unstable_name_collisions)]

use super::vector_ref::*;
use std::borrow::Cow;

fn generic_sum<V: VectorRef>(v: &V) -> f32 {
    v.as_slice().iter().sum()
}

#[test]
fn test_vector_ref_slice() {
    let data: &[f32] = &[1.0, 2.0, 3.0];
    assert_eq!(data.as_slice(), &[1.0, 2.0, 3.0]);
    assert_eq!(data.dimension(), 3);
    assert!(!data.is_empty());
}

#[test]
fn test_vector_ref_vec() {
    let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
    assert_eq!(data.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(data.dimension(), 4);
}

#[test]
fn test_vector_ref_cow_borrowed() {
    let original = vec![1.0, 2.0];
    let cow: Cow<[f32]> = Cow::Borrowed(&original);
    assert_eq!(cow.as_slice(), &[1.0, 2.0]);
    assert_eq!(cow.dimension(), 2);
}

#[test]
fn test_vector_ref_cow_owned() {
    let cow: Cow<[f32]> = Cow::Owned(vec![1.0, 2.0, 3.0]);
    assert_eq!(cow.as_slice(), &[1.0, 2.0, 3.0]);
}

#[test]
fn test_vector_ref_empty() {
    let data: &[f32] = &[];
    assert!(data.is_empty());
    assert_eq!(data.dimension(), 0);
}

#[test]
fn test_borrowed_vector_new() {
    let data = [1.0f32, 2.0, 3.0];
    let borrowed = BorrowedVector::new(&data);
    assert_eq!(borrowed.data(), &[1.0, 2.0, 3.0]);
    assert_eq!(borrowed.dimension(), 3);
}

#[test]
fn test_borrowed_vector_deref() {
    let data = [1.0f32, 2.0, 3.0];
    let borrowed = BorrowedVector::new(&data);
    let sum: f32 = borrowed.iter().sum();
    assert_eq!(sum, 6.0);
}

#[test]
fn test_borrowed_vector_as_ref() {
    let data = [1.0f32, 2.0];
    let borrowed = BorrowedVector::new(&data);
    let slice: &[f32] = borrowed.as_ref();
    assert_eq!(slice, &[1.0, 2.0]);
}

#[test]
fn test_vector_guard_basic() {
    let data = [1.0f32, 2.0, 3.0, 4.0];
    let guard = ();
    let vector_guard = VectorGuard::new(guard, &data);
    assert_eq!(vector_guard.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(vector_guard.dimension(), 4);
}

#[test]
fn test_vector_guard_deref() {
    let data = [1.0f32, 2.0, 3.0];
    let guard = VectorGuard::new((), &data);
    let max = guard.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(max, 3.0);
}

#[test]
fn test_vector_guard_with_real_lock() {
    use parking_lot::RwLock;
    static DATA: [f32; 3] = [1.0, 2.0, 3.0];
    let lock = RwLock::new(());
    let read_guard = lock.read();
    let vector_guard = VectorGuard::new(read_guard, &DATA);
    assert_eq!(vector_guard.as_slice(), &[1.0, 2.0, 3.0]);
}

#[test]
fn test_generic_function_with_slice() {
    let data: &[f32] = &[1.0, 2.0, 3.0];
    assert_eq!(generic_sum(&data), 6.0);
}

#[test]
fn test_generic_function_with_vec() {
    let data = vec![1.0f32, 2.0, 3.0, 4.0];
    assert_eq!(generic_sum(&data), 10.0);
}

#[test]
fn test_generic_function_with_borrowed() {
    let data = [1.0f32, 2.0];
    let borrowed = BorrowedVector::new(&data);
    assert_eq!(generic_sum(&borrowed), 3.0);
}

#[test]
fn test_generic_function_with_cow() {
    let cow: Cow<[f32]> = Cow::Owned(vec![1.0, 2.0, 3.0]);
    assert_eq!(generic_sum(&cow), 6.0);
}
