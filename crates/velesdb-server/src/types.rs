//! Request/Response types for VelesDB REST API.
//!
//! Canonical DTOs are defined in `velesdb_core::api_types` and re-exported here.
//! Server-specific types that are not shared with other crates remain in this file.

// Re-export all canonical DTOs from core's api_types module.
// The `openapi` feature is enabled in this crate, providing ToSchema derives.
pub use velesdb_core::api_types::*;

/// The plain-text `422` axum answers for a request body it cannot deserialise.
///
/// The `Json<T>` extractor rejects a malformed body **before the handler
/// runs**, so the payload is a bare string, not an [`ErrorResponse`] — a
/// client that parses every error as JSON fails twice, once on the request
/// and once on the error.
///
/// Declared once and referenced by every operation that takes a JSON body.
/// #2276 wrote it inline on three of them and #2291 measured the other
/// twenty-one; one `components/responses` entry is what keeps a generated
/// client with a single type for it instead of twenty-four inline copies.
#[derive(utoipa::ToResponse)]
#[response(
    description = "A body the server cannot deserialise: a field of the wrong type, or JSON that does not match the request schema.",
    content_type = "text/plain"
)]
pub struct MalformedBody(#[allow(dead_code)] String);
