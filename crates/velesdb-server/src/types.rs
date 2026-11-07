//! Request/Response types for VelesDB REST API.
//!
//! Canonical DTOs are defined in `velesdb_core::api_types` and re-exported here.
//! Server-specific types that are not shared with other crates remain in this file.

// Re-export all canonical DTOs from core's api_types module.
// The `openapi` feature is enabled in this crate, providing ToSchema derives.
pub use velesdb_core::api_types::*;

/// `/search/multi`'s request: core's `MultiQuerySearchRequest`
/// plus the `branch_weights` `weighted_rrf` reads.
///
/// Kept server-side on purpose: the core DTO is a published,
/// externally-constructible struct, so a new pub field would break
/// downstream struct literals — the semver gate refuses it
/// (`constructible_struct_adds_field`). `#[serde(flatten)]` keeps the wire
/// body byte-identical.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct MultiQuerySearchRequestWithWeights {
    /// Every field core's DTO declares.
    #[serde(flatten)]
    pub base: velesdb_core::api_types::MultiQuerySearchRequest,
    /// `weighted_rrf` only: one weight per query vector, in vector order —
    /// the parameter the strategy cannot default. Ignored by every other
    /// strategy, like the scalar weight fields.
    #[serde(default)]
    pub branch_weights: Option<Vec<f32>>,
}

/// `/search/hybrid`'s request: core's `HybridSearchRequest` plus `rrf_k`.
///
/// Kept server-side for the same reason as `MultiQuerySearchRequestWithWeights`:
/// the core DTO is published and externally constructible, so a new pub
/// field is a major bump — `#[serde(flatten)]` keeps the wire body identical.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct HybridSearchRequestWithRrf {
    /// Every field core's DTO declares.
    #[serde(flatten)]
    pub base: velesdb_core::api_types::HybridSearchRequest,
    /// Optional reciprocal-rank-fusion constant (engine default when omitted).
    /// Lower values amplify rank differences between the vector and text
    /// branches.
    #[serde(default)]
    #[schema(example = 60)]
    pub rrf_k: Option<u32>,
}

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
