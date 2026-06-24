//! Structured error surface for the WASM bindings (backlog #22).
//!
//! Browser clients cannot narrow a bare `JsValue::from_str(message)` string —
//! [`docs/reference/ERROR_CODES.md`] promises an `error.code` (`VELES-XXX`) on
//! every client surface, yet the WASM layer historically pre-flattened every
//! core error to its `Display` string, dropping the machine-readable code.
//!
//! [`WasmError`] carries both the human-readable message and the canonical
//! `velesdb_core::Error::code()` string, and renders a structured JS `Error`
//! (a real `js_sys::Error` with a non-enumerable `code` property) at the FFI
//! boundary. The code is single-sourced from core — this module never invents
//! its own taxonomy.

use velesdb_core::velesql::ParseError;
use velesdb_core::Error as CoreError;

/// A WASM-boundary error carrying a machine-readable `VELES-XXX` code.
///
/// Built from a `velesdb_core::Error` (or a `velesql::ParseError`) so the
/// `code` is always the core's single source of truth. Holds owned `String`s
/// for native-target testability; the `wasm32` build converts to a structured
/// JS `Error` via [`WasmError::into_js_value`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WasmError {
    message: String,
    code: &'static str,
}

impl WasmError {
    /// The machine-readable `VELES-XXX` error code (single-sourced from core).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    /// The human-readable error message.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// Renders a structured JS `Error` whose `code` property is the
    /// `VELES-XXX` code. The property is defined as non-enumerable so it does
    /// not pollute `JSON.stringify(error)` yet stays readable as `error.code`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn into_js_value(self) -> wasm_bindgen::JsValue {
        use wasm_bindgen::JsValue;
        let err = js_sys::Error::new(&self.message);
        let descriptor = js_sys::Object::new();
        // `Object.defineProperty` is a safe wrapper; a failed `set` degrades to
        // a plain Error rather than panicking, so the results are discarded.
        let _ = js_sys::Reflect::set(
            &descriptor,
            &JsValue::from_str("value"),
            &JsValue::from_str(self.code),
        );
        let _ = js_sys::Reflect::set(
            &descriptor,
            &JsValue::from_str("enumerable"),
            &JsValue::FALSE,
        );
        let _ = js_sys::Reflect::set(&descriptor, &JsValue::from_str("writable"), &JsValue::FALSE);
        js_sys::Object::define_property(&err, &JsValue::from_str("code"), &descriptor);
        err.into()
    }

    /// Native-target fallback: `js_sys::Error`/`Object.defineProperty` have no
    /// JS runtime off `wasm32`, so the error degrades to a flat string carrying
    /// both the code and the message. The structured `code` is asserted in
    /// native tests via [`WasmError::code`]; the real JS `Error` is produced
    /// only in the browser build above.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn into_js_value(self) -> wasm_bindgen::JsValue {
        wasm_bindgen::JsValue::from_str(&format!("[{}] {}", self.code, self.message))
    }
}

impl From<CoreError> for WasmError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code(),
            message: err.to_string(),
        }
    }
}

impl From<ParseError> for WasmError {
    /// A parse failure surfaces as a query error (`VELES-010`) to match the
    /// client contract in `ERROR_CODES.md`, while keeping the rich position /
    /// fragment detail in the message.
    fn from(err: ParseError) -> Self {
        let message = format!(
            "VelesQL syntax error at position {}: {} (near '{}')",
            err.position, err.message, err.fragment
        );
        Self {
            code: CoreError::from(err).code(),
            message,
        }
    }
}

#[cfg(test)]
#[path = "wasm_error_tests.rs"]
mod tests;
