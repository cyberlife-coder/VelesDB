//! `MemoryError` → `napi::Error` mapping.
//!
//! JavaScript has no exception-class hierarchy to mirror `PyO3`'s typed errors, so
//! the category travels as a stable code token prefixed onto the message
//! (`[NOT_FOUND] memory 7 does not exist`) plus a coarse napi `Status`. JS
//! callers branch on the prefix; the status keeps `err.code` meaningful too.

use napi::{Error, Status};
use velesdb_memory::{ErrorCategory, MemoryError};

/// Bad caller input (empty fact, reserved key, malformed filter, bad id).
pub const CODE_INVALID_INPUT: &str = "INVALID_INPUT";
/// A referenced memory id does not exist (mirrors `PyO3`'s `KeyError`).
pub const CODE_NOT_FOUND: &str = "NOT_FOUND";
/// An internal/storage/embedding/extraction failure.
pub const CODE_INTERNAL: &str = "INTERNAL";

/// Map a [`MemoryError`] to a `napi::Error` carrying a stable code, driven by
/// its [`ErrorCategory`] so the JS-facing taxonomy stays identical to the MCP
/// server's and the `PyO3` binding's.
pub fn to_napi_err(e: MemoryError) -> Error {
    let msg = e.to_string();
    let (status, code) = match e.category() {
        ErrorCategory::InvalidInput => (Status::InvalidArg, CODE_INVALID_INPUT),
        ErrorCategory::NotFound => (Status::InvalidArg, CODE_NOT_FOUND),
        ErrorCategory::Internal => (Status::GenericFailure, CODE_INTERNAL),
    };
    Error::new(status, format!("[{code}] {msg}"))
}

/// Build an `INVALID_INPUT` napi error for adapter-side validation failures
/// (id parsing, op parsing, cap checks) that never reach the domain layer.
pub fn invalid_input(msg: impl AsRef<str>) -> Error {
    Error::new(
        Status::InvalidArg,
        format!("[{CODE_INVALID_INPUT}] {}", msg.as_ref()),
    )
}
