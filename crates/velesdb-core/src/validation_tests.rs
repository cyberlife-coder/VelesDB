use super::*;

#[test]
fn accepts_simple_ascii_names() {
    for name in ["a", "abc", "my_coll", "docs-v2", "A1_b2-C3"] {
        validate_collection_name(name).unwrap();
    }
}

#[test]
fn accepts_max_length() {
    let name = "x".repeat(MAX_COLLECTION_NAME_LENGTH);
    validate_collection_name(&name).unwrap();
}

#[test]
fn rejects_empty() {
    assert!(validate_collection_name("").is_err());
}

#[test]
fn rejects_over_max_length() {
    let name = "x".repeat(MAX_COLLECTION_NAME_LENGTH + 1);
    assert!(validate_collection_name(&name).is_err());
}

#[test]
fn rejects_dot_and_dotdot() {
    assert!(validate_collection_name(".").is_err());
    assert!(validate_collection_name("..").is_err());
}

#[test]
fn rejects_path_separators() {
    assert!(validate_collection_name("a/b").is_err());
    assert!(validate_collection_name("a\\b").is_err());
    assert!(validate_collection_name("../x").is_err());
}

#[test]
fn rejects_leading_hyphen() {
    assert!(validate_collection_name("-bad").is_err());
    assert!(validate_collection_name("--bad").is_err());
}

#[test]
fn allows_interior_hyphens() {
    validate_collection_name("a-b").unwrap();
    validate_collection_name("a-b-c").unwrap();
}

#[test]
fn rejects_special_chars() {
    for name in ["a b", "a@b", "a.b", "a#b", "a$b", "a:b", "a*b"] {
        assert!(
            validate_collection_name(name).is_err(),
            "Should reject {:?}",
            name
        );
    }
}

#[test]
fn rejects_unicode() {
    assert!(validate_collection_name("café").is_err());
    assert!(validate_collection_name("日本").is_err());
}

#[test]
fn rejects_windows_reserved_case_insensitive() {
    for name in ["CON", "con", "Con", "PRN", "AUX", "NUL", "COM1", "LPT9"] {
        assert!(
            validate_collection_name(name).is_err(),
            "Should reject {:?}",
            name
        );
    }
}

#[test]
fn allows_names_containing_reserved_as_substring() {
    // "connection" contains "con" but is not the reserved name "CON"
    validate_collection_name("connection").unwrap();
    validate_collection_name("my_aux_data").unwrap();
    validate_collection_name("com10").unwrap();
}

#[test]
fn error_code_is_veles_034() {
    let err = validate_collection_name("").unwrap_err();
    assert_eq!(err.code(), "VELES-034");
}

#[test]
fn error_is_recoverable() {
    let err = validate_collection_name("").unwrap_err();
    assert!(err.is_recoverable());
}
