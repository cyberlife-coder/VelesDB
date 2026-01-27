//! Tests for `bloom` module - Bloom filter implementation.

use super::bloom::*;

#[test]
fn test_bloom_new() {
    let bloom = BloomFilter::new(1000, 0.01);
    assert_eq!(bloom.count(), 0);
}

#[test]
fn test_bloom_insert_and_contains() {
    let bloom = BloomFilter::new(1000, 0.01);

    bloom.insert(&"hello");

    assert!(bloom.contains(&"hello"));
    assert_eq!(bloom.count(), 1);
}

#[test]
fn test_bloom_definitely_not_contains() {
    let bloom = BloomFilter::new(1000, 0.01);

    bloom.insert(&"hello");

    assert!(bloom.definitely_not_contains(&"world"));
}

#[test]
fn test_bloom_no_false_negatives() {
    let bloom = BloomFilter::new(10_000, 0.01);

    for i in 0..1000 {
        bloom.insert(&i);
    }

    for i in 0..1000 {
        assert!(bloom.contains(&i), "Item {i} should be found");
    }
}

#[test]
fn test_bloom_false_positive_rate() {
    let bloom = BloomFilter::new(1000, 0.01);

    for i in 0..1000 {
        bloom.insert(&i);
    }

    let mut false_positives = 0;
    for i in 1000..11000 {
        if bloom.contains(&i) {
            false_positives += 1;
        }
    }

    let fpr = f64::from(false_positives) / 10000.0;
    assert!(fpr < 0.10, "FPR {fpr} should be < 10%");
}

#[test]
fn test_bloom_clear() {
    let bloom = BloomFilter::new(1000, 0.01);

    bloom.insert(&"hello");
    bloom.clear();

    assert_eq!(bloom.count(), 0);
    assert!(!bloom.contains(&"hello"));
}

#[test]
fn test_bloom_integer_keys() {
    let bloom = BloomFilter::new(1000, 0.01);

    bloom.insert(&42u64);
    bloom.insert(&123u64);

    assert!(bloom.contains(&42u64));
    assert!(bloom.contains(&123u64));
    assert!(bloom.definitely_not_contains(&999u64));
}
