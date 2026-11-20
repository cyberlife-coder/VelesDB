//! Tests for the engine error: the code JS reads comes from the variant.

use super::ExecError;
use velesdb_core::Error as CoreError;

#[test]
fn a_core_error_keeps_its_variant_its_code_and_its_message() {
    let err = ExecError::from(CoreError::QueryParameter {
        name: "v".to_string(),
        reason: "not bound".to_string(),
    });
    assert_eq!(err.code(), Some("VELES-038"));
    assert_eq!(err.to_string(), "[VELES-038] Query parameter $v: not bound");
    assert!(matches!(
        err,
        ExecError::Core(CoreError::QueryParameter { ref name, .. }) if name == "v"
    ));
}

#[test]
fn a_refusal_carries_no_code_whatever_its_text() {
    for text in [
        "TRAIN QUANTIZER requires the persistence feature",
        "[VELES-002] a refusal that looks coded is still a refusal",
    ] {
        let err = ExecError::refused(text);
        assert_eq!(err.code(), None, "{text}");
        assert_eq!(err.to_string(), text);
    }
}
