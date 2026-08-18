use super::*;
use std::collections::HashMap;

#[test]
fn test_gpu_trigram_accelerator_creation() {
    // May fail if no GPU available (CI)
    let result = GpuTrigramAccelerator::new();
    if result.is_ok() {
        println!("GPU trigram accelerator created successfully");
    } else {
        println!("No GPU available: {:?}", result.err());
    }
}

#[test]
fn test_gpu_is_available() {
    // Should not panic
    let _ = GpuTrigramAccelerator::is_available();
}

#[test]
fn test_batch_extract_trigrams() {
    if let Ok(gpu) = GpuTrigramAccelerator::new() {
        let docs = vec!["hello", "world", "test"];
        let results = gpu.batch_extract_trigrams(&docs);

        assert_eq!(results.len(), 3);
        // "hello" has trigrams: hel, ell, llo
        assert!(results[0].contains(b"hel"));
        assert!(results[0].contains(b"ell"));
        assert!(results[0].contains(b"llo"));
    }
}

#[test]
fn test_batch_extract_trigrams_short_text() {
    if let Ok(gpu) = GpuTrigramAccelerator::new() {
        let docs = vec!["ab", "a", ""];
        let results = gpu.batch_extract_trigrams(&docs);

        assert_eq!(results.len(), 3);
        assert!(results[0].is_empty()); // "ab" too short
        assert!(results[1].is_empty()); // "a" too short
        assert!(results[2].is_empty()); // empty
    }
}

#[test]
fn test_batch_search_empty_patterns() {
    if let Ok(gpu) = GpuTrigramAccelerator::new() {
        let index: HashMap<[u8; 3], RoaringBitmap> = HashMap::new();
        let results = gpu.batch_search(&[], &index);
        assert!(results.is_empty());
    }
}

#[test]
fn test_batch_search_with_matches() {
    if let Ok(gpu) = GpuTrigramAccelerator::new() {
        let mut index: HashMap<[u8; 3], RoaringBitmap> = HashMap::new();

        // Add trigrams for "hello" in doc 0 and 1
        let mut bitmap = RoaringBitmap::new();
        bitmap.insert(0);
        bitmap.insert(1);
        index.insert(*b"hel", bitmap.clone());
        index.insert(*b"ell", bitmap.clone());
        index.insert(*b"llo", bitmap);

        let patterns = vec!["hello"];
        let results = gpu.batch_search(&patterns, &index);

        assert_eq!(results.len(), 1);
        assert!(results[0].contains(0));
        assert!(results[0].contains(1));
    }
}

#[test]
fn test_batch_search_no_matches() {
    if let Ok(gpu) = GpuTrigramAccelerator::new() {
        let index: HashMap<[u8; 3], RoaringBitmap> = HashMap::new();
        let patterns = vec!["hello"];
        let results = gpu.batch_search(&patterns, &index);

        assert_eq!(results.len(), 1);
        assert!(results[0].is_empty());
    }
}
