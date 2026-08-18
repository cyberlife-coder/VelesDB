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
