//! The stable error codes the JS side branches on, and how a `MemoryError` becomes one.

use crate::wasm_error::structured_js_error;
use velesdb_memory::{ErrorCategory, MemoryError};
use wasm_bindgen::JsValue;

const CODE_INVALID_INPUT: &str = "INVALID_INPUT";
const CODE_NOT_FOUND: &str = "NOT_FOUND";
pub(super) const CODE_INTERNAL: &str = "INTERNAL";
const CODE_UNSUPPORTED: &str = "UNSUPPORTED";

// --- Errors ------------------------------------------------------------

fn category_code(e: &MemoryError) -> &'static str {
    known_category_code(e.category()).unwrap_or(CODE_INTERNAL)
}

/// The explicit half of the mapping, split from the fallback so the coverage
/// test over [`velesdb_memory::ErrorCategory::ALL`] can tell them apart:
/// `ErrorCategory` is `non_exhaustive`, so a new category no longer fails this
/// match at compile time — the test does instead. The runtime fallback
/// (`INTERNAL`, the coarsest bucket) only ever serves a binding built against
/// a newer `velesdb-memory` than this file.
pub(super) fn known_category_code(category: ErrorCategory) -> Option<&'static str> {
    Some(match category {
        ErrorCategory::InvalidInput => CODE_INVALID_INPUT,
        ErrorCategory::NotFound => CODE_NOT_FOUND,
        ErrorCategory::Internal => CODE_INTERNAL,
        ErrorCategory::Unsupported => CODE_UNSUPPORTED,
        _ => return None,
    })
}

pub(super) fn to_js_err(e: MemoryError) -> JsValue {
    structured_js_error(category_code(&e), &e.to_string())
}

pub(super) fn invalid_input(msg: impl AsRef<str>) -> JsValue {
    structured_js_error(CODE_INVALID_INPUT, msg.as_ref())
}
