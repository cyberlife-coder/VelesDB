//! TDD Tests for Compression (US-CORE-003-06, US-CORE-003-07)

use super::*;

// ========== Dictionary Encoding Tests ==========

#[test]
fn test_dictionary_encoder_new() {
    let encoder: DictionaryEncoder<String> = DictionaryEncoder::new();
    assert!(encoder.is_empty());
    assert_eq!(encoder.len(), 0);
}

#[test]
fn test_dictionary_encode_single() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let code = encoder.encode("hello".to_string());

    assert_eq!(code, 0);
    assert_eq!(encoder.len(), 1);
}

#[test]
fn test_dictionary_encode_duplicate() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let code1 = encoder.encode("hello".to_string());
    let code2 = encoder.encode("hello".to_string());

    assert_eq!(code1, code2);
    assert_eq!(encoder.len(), 1); // No duplicate entry
}

#[test]
fn test_dictionary_encode_multiple() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let code_a = encoder.encode("a".to_string());
    let code_b = encoder.encode("b".to_string());
    let code_c = encoder.encode("c".to_string());

    assert_eq!(code_a, 0);
    assert_eq!(code_b, 1);
    assert_eq!(code_c, 2);
    assert_eq!(encoder.len(), 3);
}

#[test]
fn test_dictionary_decode() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let code = encoder.encode("hello".to_string());
    let decoded = encoder.decode(code);

    assert_eq!(decoded, Some(&"hello".to_string()));
}

#[test]
fn test_dictionary_decode_invalid() {
    let encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    assert_eq!(encoder.decode(999), None);
}

#[test]
fn test_dictionary_batch_encode() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let values = vec![
        "France".to_string(),
        "Spain".to_string(),
        "France".to_string(),
        "Italy".to_string(),
        "France".to_string(),
    ];

    let codes = encoder.encode_batch(&values);

    assert_eq!(codes, vec![0, 1, 0, 2, 0]);
    assert_eq!(encoder.len(), 3); // Only 3 unique values
}

#[test]
fn test_dictionary_batch_decode() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    let values = vec!["a".to_string(), "b".to_string(), "a".to_string()];
    let codes = encoder.encode_batch(&values);
    let decoded = encoder.decode_batch(&codes);

    assert_eq!(decoded, values);
}

// ========== Compression Stats Tests ==========

#[test]
fn test_compression_stats_ratio() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    // Encode 1000 values with only 10 unique
    let values: Vec<String> = (0..1000).map(|i| format!("value_{}", i % 10)).collect();

    let _ = encoder.encode_batch(&values);
    let stats = encoder.stats();

    // Should have good compression ratio
    assert!(
        stats.compression_ratio > 1.0,
        "Ratio {} should be > 1.0",
        stats.compression_ratio
    );
    assert_eq!(stats.unique_values, 10);
    assert_eq!(stats.total_values, 1000);
}

#[test]
fn test_compression_stats_memory() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    encoder.encode("hello".to_string());
    encoder.encode("world".to_string());

    let stats = encoder.stats();

    assert!(stats.dictionary_size_bytes > 0);
    assert!(stats.encoded_size_bytes > 0);
}

#[test]
fn test_dictionary_clear() {
    let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

    encoder.encode("hello".to_string());
    encoder.clear();

    assert!(encoder.is_empty());
}

// ========== Integer Dictionary Tests ==========

#[test]
fn test_dictionary_integer_values() {
    let mut encoder: DictionaryEncoder<i64> = DictionaryEncoder::new();

    let codes = encoder.encode_batch(&[100, 200, 100, 300, 200]);

    assert_eq!(codes, vec![0, 1, 0, 2, 1]);
    assert_eq!(encoder.len(), 3);
}
