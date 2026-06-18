//! Unit tests for the `license` module.
//!
//! Extracted from `license.rs` into a sibling `*_tests.rs` file per the
//! project test convention (tests live beside their module). `super` resolves
//! to the `license` module, so the original imports are unchanged.

use super::*;

#[test]
fn test_license_tier_display() {
    assert_eq!(LicenseTier::Professional.to_string(), "Professional");
    assert_eq!(LicenseTier::Team.to_string(), "Team");
    assert_eq!(LicenseTier::Enterprise.to_string(), "Enterprise");
}

#[test]
fn test_license_info_has_feature() {
    let info = LicenseInfo {
        key: "TEST-KEY".to_string(),
        tier: LicenseTier::Professional,
        organization: "Test Corp".to_string(),
        expires_at: u64::MAX,
        max_instances: 1,
        features: vec![PremiumFeature::Snapshots],
    };

    assert!(info.has_feature(PremiumFeature::Snapshots));
    assert!(!info.has_feature(PremiumFeature::MultiTenancy));
}

#[test]
fn test_premium_feature_enum_only_contains_true_premium_variants() {
    // After #390: HybridSearch, AdvancedFiltering, GpuAcceleration are free
    // in open-source core and must NOT be in the PremiumFeature enum.
    // The remaining 6 variants are genuinely premium. This guard fails loudly
    // if the set ever changes (count assert + exhaustive match with no wildcard).
    let all_premium = [
        PremiumFeature::EncryptionAtRest,
        PremiumFeature::Snapshots,
        PremiumFeature::MultiTenancy,
        PremiumFeature::RBAC,
        PremiumFeature::SSO,
        PremiumFeature::AuditLogging,
    ];
    assert_eq!(
        all_premium.len(),
        6,
        "PremiumFeature must have exactly 6 variants"
    );
    for feature in &all_premium {
        // Exhaustive match (no `_` arm): adding/removing a variant breaks
        // compilation here, forcing this guard to be updated deliberately.
        match feature {
            PremiumFeature::EncryptionAtRest
            | PremiumFeature::Snapshots
            | PremiumFeature::MultiTenancy
            | PremiumFeature::RBAC
            | PremiumFeature::SSO
            | PremiumFeature::AuditLogging => {}
        }
    }
}

#[test]
fn test_premium_feature_display_values() {
    assert_eq!(
        PremiumFeature::EncryptionAtRest.to_string(),
        "Encryption at Rest"
    );
    assert_eq!(PremiumFeature::Snapshots.to_string(), "Snapshots & Backups");
    assert_eq!(PremiumFeature::MultiTenancy.to_string(), "Multi-Tenancy");
    assert_eq!(PremiumFeature::RBAC.to_string(), "RBAC");
    assert_eq!(PremiumFeature::SSO.to_string(), "SSO");
    assert_eq!(PremiumFeature::AuditLogging.to_string(), "Audit Logging");
}

#[test]
fn test_premium_feature_serde_roundtrip() {
    let features = vec![
        PremiumFeature::EncryptionAtRest,
        PremiumFeature::Snapshots,
        PremiumFeature::RBAC,
    ];
    let json = serde_json::to_string(&features).unwrap();
    let deserialized: Vec<PremiumFeature> = serde_json::from_str(&json).unwrap();
    assert_eq!(features, deserialized);
}

#[test]
fn test_legacy_license_with_removed_features_deserializes() {
    // Existing license payloads signed by GetAppSuite may contain
    // "HybridSearch", "AdvancedFiltering", or "GpuAcceleration".
    // These must be silently ignored (the features are now free).
    let legacy_json = r#"{
        "key": "LEGACY-KEY",
        "tier": "Professional",
        "organization": "Legacy Corp",
        "expires_at": 9999999999,
        "max_instances": 5,
        "features": ["HybridSearch", "Snapshots", "AdvancedFiltering", "GpuAcceleration", "RBAC"]
    }"#;
    let info: LicenseInfo = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(
        info.features.len(),
        2,
        "Only Snapshots and RBAC are true premium"
    );
    assert!(info.has_feature(PremiumFeature::Snapshots));
    assert!(info.has_feature(PremiumFeature::RBAC));
}

#[test]
fn test_license_info_is_expired() {
    let expired = LicenseInfo {
        key: "TEST-KEY".to_string(),
        tier: LicenseTier::Professional,
        organization: "Test Corp".to_string(),
        expires_at: 1_000_000, // Very old timestamp
        max_instances: 1,
        features: vec![],
    };

    assert!(expired.is_expired());

    let valid = LicenseInfo {
        key: "TEST-KEY".to_string(),
        tier: LicenseTier::Professional,
        organization: "Test Corp".to_string(),
        expires_at: u64::MAX,
        max_instances: 1,
        features: vec![],
    };

    assert!(!valid.is_expired());
}

#[test]
fn test_signed_license_parse_invalid_format() {
    let result = SignedLicense::parse("invalid-format");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid license format"));
}

#[test]
fn test_signed_license_parse_invalid_base64() {
    let result = SignedLicense::parse("not-base64.also-not-base64");
    assert!(result.is_err());
}
