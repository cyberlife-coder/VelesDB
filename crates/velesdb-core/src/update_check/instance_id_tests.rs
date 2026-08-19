use super::*;
use tempfile::TempDir;

#[test]
fn test_instance_hash_stable_across_calls() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let hash1 = compute_instance_hash(dir.path());
    let hash2 = compute_instance_hash(dir.path());
    assert_eq!(hash1, hash2, "Hash should be stable across calls");
}

#[test]
fn test_instance_hash_different_for_different_dirs() {
    let dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let dir2 = TempDir::new().expect("Failed to create temp dir 2");
    let hash1 = compute_instance_hash(dir1.path());
    let hash2 = compute_instance_hash(dir2.path());
    assert_ne!(hash1, hash2, "Different dirs should have different hashes");
}

#[test]
fn test_instance_hash_is_sha256_hex() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let hash = compute_instance_hash(dir.path());
    assert_eq!(hash.len(), 64, "SHA256 hex should be 64 chars");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "Hash should be hex"
    );
}

#[test]
fn test_get_machine_id_does_not_panic() {
    // Just ensure it doesn't panic - result may be None on CI
    let _ = get_machine_id();
}

#[test]
fn test_fallback_id_is_non_empty() {
    let fallback = get_fallback_id();
    assert!(!fallback.is_empty());
    assert!(fallback.starts_with("fallback:"));
}
