use super::VelesError;

#[test]
fn core_error_carries_canonical_code_and_recoverability() {
    // A core Storage error (VELES-006) is recoverable.
    let err: VelesError = velesdb_core::Error::Storage("disk full".to_string()).into();
    let VelesError::Database {
        code, recoverable, ..
    } = err
    else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert_eq!(code, "VELES-006");
    assert!(recoverable);

    // An IndexCorrupted error (VELES-008) is non-recoverable.
    let err: VelesError = velesdb_core::Error::IndexCorrupted("bad header".to_string()).into();
    let VelesError::Database {
        code, recoverable, ..
    } = err
    else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert_eq!(code, "VELES-008");
    assert!(!recoverable);
}

#[test]
fn binding_level_error_has_no_core_code() {
    let err = VelesError::database("bad JSON".to_string());
    let VelesError::Database {
        code, recoverable, ..
    } = err
    else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert!(code.is_empty());
    assert!(recoverable);
}

#[test]
fn typed_variants_carry_cores_code_and_recoverability() {
    let err: VelesError = velesdb_core::Error::CollectionNotFound("docs".to_string()).into();
    let VelesError::Collection {
        message,
        code,
        recoverable,
    } = err
    else {
        panic!("expected VelesError::Collection, got {err:?}");
    };
    assert!(message.contains("docs"), "{message}");
    assert_eq!(code, "VELES-002");
    assert!(recoverable);

    let err: VelesError = velesdb_core::Error::CollectionExists("docs".to_string()).into();
    assert!(
        matches!(&err, VelesError::Collection { code, .. } if code == "VELES-001"),
        "{err:?}"
    );

    let err: VelesError = velesdb_core::Error::DimensionMismatch {
        expected: 384,
        actual: 2,
    }
    .into();
    let VelesError::DimensionMismatch {
        expected,
        actual,
        code,
        recoverable,
    } = err
    else {
        panic!("expected VelesError::DimensionMismatch, got {err:?}");
    };
    assert_eq!((expected, actual), (384, 2));
    assert_eq!(code, "VELES-004");
    assert!(recoverable);
}

#[test]
fn typed_variants_show_their_code_like_database() {
    let err: VelesError = velesdb_core::Error::DimensionMismatch {
        expected: 384,
        actual: 2,
    }
    .into();
    assert_eq!(
        err.to_string(),
        "[VELES-004] Dimension mismatch: expected 384, got 2"
    );
    let err: VelesError = velesdb_core::Error::CollectionNotFound("docs".to_string()).into();
    assert_eq!(
        err.to_string(),
        "[VELES-002] Collection error: Collection not found: docs"
    );
}

#[test]
fn a_collection_of_the_wrong_kind_is_coded_as_core_codes_it() {
    // Core's own lookups answer VELES-002 for a collection that exists but is
    // not of the kind asked (`resolve_graph_collection`, and the server's
    // `get_vector_collection_or_404`), with the kind in the message.
    let err = VelesError::wrong_collection_kind("kg", "not a vector collection".to_string());
    let VelesError::Collection {
        message,
        code,
        recoverable,
    } = err
    else {
        panic!("expected VelesError::Collection, got {err:?}");
    };
    assert_eq!(message, "not a vector collection");
    assert_eq!(code, "VELES-002");
    assert!(recoverable);
}
