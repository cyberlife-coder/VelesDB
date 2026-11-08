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
    assert!(info.features.contains(&PremiumFeature::Snapshots));
    assert!(info.features.contains(&PremiumFeature::RBAC));
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

// ==== Golden fixtures signed by the velesdb-premium issuer (#2680) ====
//
// The `.lic` payloads under `tests/fixtures/license/` were produced by the
// actual premium issuer (`LicenseSigner` + premium `LicenseInfo` serde, via
// the `veles-license` tool in velesdb-private) with a dedicated fixture
// keypair; only its public half is checked in (see the README there for how
// to regenerate). These tests pin mechanically what the `LicenseInfo`,
// `LicenseTier` and `PremiumFeature` comments used to state in prose:
// bytes the premium serializer emits must verify and decode here. If either
// side's license types drift, they fail.

const ISSUER_PUBKEY: &str = include_str!("../tests/fixtures/license/issuer_public.key");
const WRONG_ISSUER_PUBKEY: &str = include_str!("../tests/fixtures/license/wrong_issuer_public.key");
const GOLDEN_PROFESSIONAL: &str = include_str!("../tests/fixtures/license/golden_professional.lic");
const GOLDEN_TEAM: &str = include_str!("../tests/fixtures/license/golden_team.lic");
const GOLDEN_ENTERPRISE: &str = include_str!("../tests/fixtures/license/golden_enterprise.lic");
const GOLDEN_PERPETUAL: &str = include_str!("../tests/fixtures/license/golden_perpetual.lic");
const EXPIRED: &str = include_str!("../tests/fixtures/license/expired.lic");

fn verify_golden(license: &str) -> LicenseInfo {
    let signed = SignedLicense::parse(license.trim()).unwrap();
    signed.verify_with_key(ISSUER_PUBKEY.trim()).unwrap();
    signed.extract_info().unwrap()
}

#[test]
fn test_golden_professional_verifies_with_all_features_lenient_dropped() {
    let info = verify_golden(GOLDEN_PROFESSIONAL);
    assert_eq!(info.tier, LicenseTier::Professional);
    assert_eq!(info.organization, "VelesDB Golden Fixtures");
    assert_eq!(info.expires_at, 4_102_444_799);
    assert_eq!(info.max_instances, 1);
    assert!(!info.is_expired());
    // All 8 premium professional features are unknown to this mirror; the
    // lenient deserializer drops them rather than rejecting the license.
    assert!(info.features.is_empty());
}

#[test]
fn test_golden_team_verifies_and_filters_to_known_features() {
    let info = verify_golden(GOLDEN_TEAM);
    assert_eq!(info.tier, LicenseTier::Team);
    assert_eq!(info.max_instances, 5);
    // 11 premium features signed; only the two this mirror knows survive.
    assert_eq!(
        info.features,
        [PremiumFeature::Snapshots, PremiumFeature::EncryptionAtRest]
    );
}

#[test]
fn test_golden_enterprise_u32_max_instances_decodes() {
    // Enterprise's default cap is `u32::MAX` (premium's "unlimited") — a
    // value the previous `i32` field could not deserialize.
    let info = verify_golden(GOLDEN_ENTERPRISE);
    assert_eq!(info.tier, LicenseTier::Enterprise);
    assert_eq!(info.max_instances, i64::from(u32::MAX));
    assert_eq!(info.expires_at_formatted(), "2099-12-31");
    assert_eq!(
        info.features,
        [
            PremiumFeature::Snapshots,
            PremiumFeature::EncryptionAtRest,
            PremiumFeature::MultiTenancy,
        ]
    );
}

#[test]
fn test_golden_perpetual_never_expires_or_wraps_to_1969() {
    // Premium perpetual licenses carry `expires_at = u64::MAX`; the display
    // contract requires "never expires", not a wrapped i64 cast to 1969.
    let info = verify_golden(GOLDEN_PERPETUAL);
    assert!(!info.is_expired());
    assert_eq!(info.expires_at_formatted(), "Never");
    assert_eq!(info.max_instances, i64::from(u32::MAX));
}

#[test]
fn test_signed_expired_fixture_rejected_by_validate_license() {
    let err = validate_license(EXPIRED.trim(), ISSUER_PUBKEY.trim()).unwrap_err();
    assert!(
        err.to_string().contains("expired"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_golden_fixture_tampered_payload_fails_verification() {
    // Flip one byte inside the signed payload: the signature must no longer hold.
    let mut parts = GOLDEN_TEAM.trim().split('.');
    let payload_b64 = parts.next().unwrap();
    let sig_b64 = parts.next().unwrap();
    let mut payload = BASE64.decode(payload_b64).unwrap();
    let mid = payload.len() / 2;
    payload[mid] ^= 0x01;
    let tampered = format!("{}.{}", BASE64.encode(&payload), sig_b64);
    let signed = SignedLicense::parse(&tampered).unwrap();
    assert!(signed.verify_with_key(ISSUER_PUBKEY.trim()).is_err());
}

#[test]
fn test_golden_fixture_wrong_issuer_key_fails_verification() {
    let signed = SignedLicense::parse(GOLDEN_TEAM.trim()).unwrap();
    assert!(signed.verify_with_key(WRONG_ISSUER_PUBKEY.trim()).is_err());
}

#[test]
fn test_golden_fixture_truncated_signature_rejected() {
    let trimmed = GOLDEN_TEAM.trim();
    let truncated = &trimmed[..trimmed.len() - 4];
    assert!(SignedLicense::parse(truncated).is_err());
}
