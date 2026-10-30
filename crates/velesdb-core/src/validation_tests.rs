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

// -- payload depth: the boundary is serde_json's own --
#[cfg(feature = "persistence")] // validate_payload_depth is.
mod payload_depth {
    use super::*;

    fn nested_arrays(depth: usize) -> serde_json::Value {
        let mut value = serde_json::json!(1);
        for _ in 0..depth {
            value = serde_json::json!([value]);
        }
        value
    }

    /// The deepest JSON document core writes a payload into: the agent-memory
    /// snapshot's envelope, its point list, then the point.
    fn in_memory_snapshot(payload: &serde_json::Value) -> String {
        serde_json::to_string(&serde_json::json!({ "points": [{ "id": 1, "payload": payload }] }))
            .unwrap()
    }

    #[test]
    fn test_the_deepest_accepted_payload_reads_back_from_every_document() {
        let payload = nested_arrays(MAX_VALUE_DEPTH);

        validate_payload_depth(1, &payload).unwrap();
        let log_record = serde_json::to_string(&payload).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&log_record).unwrap(),
            payload
        );
        let snapshot: serde_json::Value =
            serde_json::from_str(&in_memory_snapshot(&payload)).unwrap();
        assert_eq!(snapshot["points"][0]["payload"], payload);
    }

    #[test]
    fn test_one_level_deeper_is_refused_and_is_what_the_memory_snapshot_cannot_read() {
        let payload = nested_arrays(MAX_VALUE_DEPTH + 1);

        let err = validate_payload_depth(7, &payload).unwrap_err();

        assert!(
            matches!(err, Error::SchemaValidation(ref m) if m.contains("point 7")),
            "{err}"
        );
        assert!(serde_json::from_str::<serde_json::Value>(&in_memory_snapshot(&payload)).is_err());
    }

    #[test]
    fn test_payload_depth_follows_the_deepest_branch_through_objects() {
        let mut deep = serde_json::json!({});
        for _ in 1..=MAX_VALUE_DEPTH {
            deep = serde_json::json!({ "next": deep });
        }
        let payload = serde_json::json!({ "shallow": [1, 2], "deep": deep });

        assert!(validate_payload_depth(1, &payload).is_err());
        assert!(validate_payload_depth(1, &serde_json::json!({"a": [1, {"b": null}]})).is_ok());
        assert!(validate_payload_depth(1, &serde_json::json!("scalar")).is_ok());
    }
}
