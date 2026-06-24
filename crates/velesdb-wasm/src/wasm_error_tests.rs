//! Native-target tests for the structured WASM error surface (backlog #22).
//!
//! `wasm32` rejections cross the FFI boundary as a `js_sys::Error` whose `code`
//! property carries the `VELES-XXX` code; these native tests inspect the
//! pre-FFI [`WasmError`] (same data) so the code mapping is locked without a
//! browser. The values are single-sourced from `velesdb_core::Error::code()`.

use super::WasmError;
use velesdb_core::velesql::Parser;
use velesdb_core::Error as CoreError;

#[test]
fn dimension_mismatch_carries_veles_004() {
    let err =
        velesdb_core::validate_dimension_match(768, 384).expect_err("test: 768 != 384 must reject");
    let wasm: WasmError = err.into();
    assert_eq!(wasm.code(), "VELES-004");
    assert!(wasm.message().contains("dimension"));
}

#[test]
fn invalid_collection_name_carries_veles_034() {
    let err = velesdb_core::validate_collection_name("../etc/passwd")
        .expect_err("test: path-traversal name must reject");
    let wasm: WasmError = err.into();
    assert_eq!(wasm.code(), "VELES-034");
}

#[test]
fn parse_error_carries_veles_010() {
    let err = Parser::parse("SELEC * FROM docs").expect_err("test: bad keyword must reject");
    let wasm: WasmError = err.into();
    assert_eq!(wasm.code(), "VELES-010");
    assert!(wasm.message().contains("position"));
}

#[test]
fn search_path_dimension_check_carries_veles_004() {
    // The structured check is the single source feeding every `search*` FFI
    // method's `validate_dimension`; a 4-d store queried with a 2-d vector
    // must reject with the machine-readable VELES-004.
    let err = crate::store_search::validate_dimension_structured(2, 4)
        .expect_err("test: 2 != 4 must reject");
    assert_eq!(err.code(), "VELES-004");
}

#[test]
fn create_collection_validates_name_with_veles_034() {
    // The inner create path flattens the structured error to a string, but the
    // structured code is preserved on the validation single source itself.
    let err = crate::database::DatabaseInner::validate_name("../escape")
        .expect_err("test: traversal name must reject");
    assert_eq!(err.code(), "VELES-034");
}

#[test]
fn core_error_code_is_preserved_verbatim() {
    let err = CoreError::CollectionNotFound("missing".to_string());
    let wasm: WasmError = err.into();
    assert_eq!(wasm.code(), "VELES-002");
}
