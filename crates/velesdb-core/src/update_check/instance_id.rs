//! Instance ID Generation (US-001)
//!
//! Generates a stable, non-reversible hash unique to each VelesDB installation.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute a stable, non-reversible instance hash.
///
/// The hash is computed from:
/// - Machine ID (platform-specific)
/// - Data directory path (canonicalized)
///
/// This ensures:
/// - Same hash across restarts (stable)
/// - Different hash for different installations (unique)
/// - Cannot reverse to identify machine (privacy)
#[must_use]
pub fn compute_instance_hash(data_dir: &Path) -> String {
    let machine_id = get_machine_id().unwrap_or_else(get_fallback_id);

    let path_str = data_dir.canonicalize().map_or_else(
        |_| data_dir.to_string_lossy().to_string(),
        |p| p.to_string_lossy().to_string(),
    );

    let mut hasher = Sha256::new();
    hasher.update(b"velesdb-instance-v1:"); // Salt/version prefix
    hasher.update(machine_id.as_bytes());
    hasher.update(b":");
    hasher.update(path_str.as_bytes());

    hex::encode(hasher.finalize())
}

/// Get machine ID (platform-specific)
fn get_machine_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/machine-id")
            .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
            .ok()
            .map(|s| s.trim().to_string())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .ok()
            .and_then(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .find(|line| line.contains("IOPlatformUUID"))
                    .and_then(|line| line.split('"').nth(3))
                    .map(std::string::ToString::to_string)
            })
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("reg")
            .args([
                "query",
                "HKLM\\SOFTWARE\\Microsoft\\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .output()
            .ok()
            .and_then(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .find(|line| line.contains("MachineGuid"))
                    .and_then(|line| line.split_whitespace().last())
                    .map(std::string::ToString::to_string)
            })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Fallback ID when machine-id is not available
fn get_fallback_id() -> String {
    let hostname = hostname::get().map_or_else(
        |_| "unknown".to_string(),
        |h| h.to_string_lossy().to_string(),
    );

    let username = whoami::username();

    format!("fallback:{hostname}:{username}")
}

#[cfg(test)]
mod tests {
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
}
