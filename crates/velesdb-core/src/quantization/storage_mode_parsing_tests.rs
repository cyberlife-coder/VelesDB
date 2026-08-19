use super::{StorageMode, STORAGE_MODE_NAMES};

/// Forces this test to be revisited whenever a variant is added: the
/// exhaustive `match` (no wildcard arm) fails to compile until the new
/// variant is listed here, which in turn flags the missing const entry.
fn ordinal(mode: StorageMode) -> usize {
    match mode {
        StorageMode::Full => 0,
        StorageMode::SQ8 => 1,
        StorageMode::Binary => 2,
        StorageMode::ProductQuantization => 3,
        StorageMode::RaBitQ => 4,
    }
}

#[test]
fn storage_mode_names_is_exhaustive_and_canonical() {
    let variants = [
        StorageMode::Full,
        StorageMode::SQ8,
        StorageMode::Binary,
        StorageMode::ProductQuantization,
        StorageMode::RaBitQ,
    ];
    assert_eq!(variants.len(), STORAGE_MODE_NAMES.len());
    for (i, variant) in variants.into_iter().enumerate() {
        assert_eq!(ordinal(variant), i);
        assert_eq!(STORAGE_MODE_NAMES[i], variant.canonical_name());
    }
}

#[test]
fn test_parse_all_canonical_names() {
    assert_eq!("full".parse::<StorageMode>().unwrap(), StorageMode::Full);
    assert_eq!("sq8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
    assert_eq!(
        "binary".parse::<StorageMode>().unwrap(),
        StorageMode::Binary
    );
    assert_eq!(
        "pq".parse::<StorageMode>().unwrap(),
        StorageMode::ProductQuantization
    );
    assert_eq!(
        "rabitq".parse::<StorageMode>().unwrap(),
        StorageMode::RaBitQ
    );
}

#[test]
fn test_parse_aliases() {
    assert_eq!("f32".parse::<StorageMode>().unwrap(), StorageMode::Full);
    assert_eq!("int8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
    assert_eq!("bit".parse::<StorageMode>().unwrap(), StorageMode::Binary);
    assert_eq!(
        "product_quantization".parse::<StorageMode>().unwrap(),
        StorageMode::ProductQuantization
    );
}

#[test]
fn test_parse_case_insensitive() {
    assert_eq!("SQ8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
    assert_eq!("FULL".parse::<StorageMode>().unwrap(), StorageMode::Full);
    assert_eq!(
        "RaBitQ".parse::<StorageMode>().unwrap(),
        StorageMode::RaBitQ
    );
}

#[test]
fn test_parse_unknown_returns_error() {
    assert!("unknown".parse::<StorageMode>().is_err());
    assert!("".parse::<StorageMode>().is_err());
}

#[test]
fn test_canonical_name_roundtrip() {
    for mode in [
        StorageMode::Full,
        StorageMode::SQ8,
        StorageMode::Binary,
        StorageMode::ProductQuantization,
        StorageMode::RaBitQ,
    ] {
        let name = mode.canonical_name();
        assert_eq!(name.parse::<StorageMode>().unwrap(), mode);
    }
}

#[test]
fn test_display_uses_canonical_name() {
    assert_eq!(format!("{}", StorageMode::Full), "full");
    assert_eq!(format!("{}", StorageMode::SQ8), "sq8");
    assert_eq!(format!("{}", StorageMode::Binary), "binary");
    assert_eq!(format!("{}", StorageMode::ProductQuantization), "pq");
    assert_eq!(format!("{}", StorageMode::RaBitQ), "rabitq");
}
