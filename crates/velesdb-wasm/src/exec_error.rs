//! The error of the in-memory engine behind `WasmDatabase`: its VelesQL
//! executor (`executeQuery`, `executeMatch`) and the collection and graph
//! operations it shares with them.
//!
//! An error core defines stays a [`velesdb_core::Error`] from where it is
//! raised to the FFI boundary — a `$name` core's binder refuses, a missing
//! collection, a dimension mismatch — so the boundary reads its `VELES-XXX`
//! code from the variant, never from the message. Where core has a coded
//! rule for a refusal, the executor raises core's variant with core's
//! message — by calling the rule in core wherever core exposes it
//! (`velesql::bind_insert_row`, `collection_param`,
//! `fusion::validate_fused_vector_count`, `velesql::resolve_storage_mode`,
//! `Error::from` for a parse, validation or fusion failure, …).
//!
//! [`ExecError::Refused`], a message with no code, is left for what core has
//! no rule for: a statement, clause or value core runs and WASM does not
//! (each says "not supported in WASM", or names the WASM-only reason, such
//! as a single-vector `USING FUSION` with no second branch); an input of the
//! WASM surface itself, which core never parses (the params JSON blob, a
//! decimal edge id, `createCollection`'s metric argument, `executeMatch` on
//! a statement that is not a MATCH); and the defensive arms for a
//! `#[non_exhaustive]` AST variant WASM has not mapped yet. An invariant the
//! executor relies on is core's `Error::Internal`.

use std::fmt;

/// An engine error: core's, kept whole, or the executor's own refusal.
#[derive(Debug)]
pub(crate) enum ExecError {
    /// An error core defines, with its code.
    Core(velesdb_core::Error),
    /// What core has no rule for (see the module doc); no core code.
    Refused(String),
}

impl ExecError {
    /// The executor's own refusal of what core has no rule for (see the
    /// module doc). Spelled out at every site, never converted from a bare
    /// string, so a message cannot become an uncoded refusal by accident
    /// where core has a coded variant.
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::Refused(message.into())
    }

    /// The `VELES-XXX` code JavaScript reads as `error.code`: core's, or
    /// `None` for the executor's own refusal.
    pub(crate) fn code(&self) -> Option<&'static str> {
        match self {
            Self::Core(err) => Some(err.code()),
            Self::Refused(_) => None,
        }
    }

    /// Renders the error for JavaScript: a core error as a JS `Error` whose
    /// `code` is [`Self::code`], as the store and collection surfaces raise
    /// theirs ([`crate::wasm_error::structured_js_error`]); a refusal as the
    /// bare message string the executor always threw. The message is the
    /// error's `Display`, a core error's code prefix included.
    pub(crate) fn into_js_value(self) -> wasm_bindgen::JsValue {
        let message = self.to_string();
        match self.code() {
            Some(code) => crate::wasm_error::structured_js_error(code, &message),
            None => wasm_bindgen::JsValue::from_str(&message),
        }
    }
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(err) => err.fmt(f),
            Self::Refused(message) => f.write_str(message),
        }
    }
}

impl From<velesdb_core::Error> for ExecError {
    fn from(err: velesdb_core::Error) -> Self {
        Self::Core(err)
    }
}

#[cfg(test)]
#[path = "exec_error_tests.rs"]
mod tests;
