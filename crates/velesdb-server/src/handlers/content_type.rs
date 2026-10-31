//! `Content-Type` enforcement for the routes that read their body raw.
//!
//! A route with a `Json` extractor is already out of reach of a cross-origin
//! page: `application/json` is not one of the three types a browser sends
//! without a CORS preflight (`text/plain`, `application/x-www-form-urlencoded`,
//! `multipart/form-data`), and the server's default CORS policy refuses every
//! preflight. A route that reads `Bytes` or a raw `Body` checked nothing, so a
//! page could POST `text/plain` with `mode: 'no-cors'` — no preflight — and
//! the server would ingest it while the page could not even read the answer.
//! [`RequireContentType`] puts those routes behind the same wall: it accepts
//! only the media type the route documents, and none of them is a type a
//! preflight-free request can carry.

use std::marker::PhantomData;

use axum::{
    extract::FromRequestParts,
    http::{header::CONTENT_TYPE, request::Parts, StatusCode},
    response::Response,
};

use super::helpers::error_response;

/// A media type a route documents for its body.
pub trait BodyMediaType {
    /// The type's essence, lower case, without parameters.
    const ESSENCE: &'static str;
}

/// `application/x-ndjson`, the body of `POST .../points/stream`.
pub struct Ndjson;

impl BodyMediaType for Ndjson {
    const ESSENCE: &'static str = "application/x-ndjson";
}

/// `application/octet-stream`, the VRB1 body of `POST .../points/raw`.
pub struct OctetStream;

impl BodyMediaType for OctetStream {
    const ESSENCE: &'static str = "application/octet-stream";
}

/// Extracts nothing; refuses the request with `415 Unsupported Media Type`
/// unless its `Content-Type` is `M` (parameters such as `charset` allowed,
/// case-insensitive). Listed before the body extractor, it runs first.
pub struct RequireContentType<M>(PhantomData<M>);

impl<S, M> FromRequestParts<S> for RequireContentType<M>
where
    S: Send + Sync,
    M: BodyMediaType,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let essence = parts
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        match essence {
            Some(essence) if essence.eq_ignore_ascii_case(M::ESSENCE) => Ok(Self(PhantomData)),
            _ => Err(error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!(
                    "this route takes Content-Type: {}, got {}",
                    M::ESSENCE,
                    essence.unwrap_or("none")
                ),
            )),
        }
    }
}
