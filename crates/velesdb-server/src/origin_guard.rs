//! Refuses a state-changing request from a page the CORS policy does not
//! allow.
//!
//! CORS decides what a page may *read*; a request a browser sends without a
//! preflight still reaches the server and still runs. The routes that take a
//! body are out of reach through their `Content-Type`
//! (`handlers::content_type`), but a body-less `POST` (flush, analyze, index
//! rebuild, vacuum, compact, locality reorder) is a simple request a page on
//! any site could fire blind. This guard closes that for every route at once:
//! a request with a method other than `GET`, `HEAD` or `OPTIONS` whose
//! `Origin` header the effective CORS policy does not allow gets `403`.
//!
//! - No `Origin` header — curl, the SDKs, server to server: not the guard's
//!   business.
//! - Same origin — a page the server serves itself, such as Swagger UI:
//!   allowed. A browser sends `Origin` on a same-origin `POST` too; it is
//!   recognised by its authority (`host[:port]`) equalling the request's
//!   `Host` header. This concedes nothing the browser does not: a page that
//!   reaches the server under its own host name (DNS rebinding) is
//!   same-origin for the browser as well, and only API keys stop it.
//! - Otherwise the CORS policy decides: none configured (the default) or
//!   `cors_enabled = false` admits no foreign origin, an explicit list admits
//!   exactly its entries, `["*"]` admits every origin.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::config::CorsConfig;

/// The origins a state-changing request may come from, besides the server's
/// own.
#[derive(Debug, Clone)]
pub(crate) enum OriginPolicy {
    /// `cors_origins = ["*"]`.
    Any,
    /// Exactly these (empty: none — the default).
    Only(Arc<[String]>),
}

impl OriginPolicy {
    pub(crate) fn from_cors(cors: &CorsConfig) -> Self {
        if cors.is_active() && cors.is_permissive() {
            Self::Any
        } else if cors.is_active() {
            Self::Only(cors.allowed_origins.clone().into())
        } else {
            Self::Only(Arc::from([]))
        }
    }

    fn admits(&self, origin: &str, headers: &HeaderMap) -> bool {
        match self {
            Self::Any => true,
            Self::Only(origins) => {
                is_same_origin(origin, headers) || origins.iter().any(|o| o == origin)
            }
        }
    }
}

/// Whether `origin`'s host and port are the `Host` the request was sent to,
/// the scheme's default port (80, 443) filled in where either leaves it out.
/// Trusted only because the `Host` check (`host_guard`) runs first: without
/// it, a DNS-rebinding page would pass as same-origin here.
fn is_same_origin(origin: &str, headers: &HeaderMap) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => "80",
        "https" => "443",
        _ => return false,
    };
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|host| {
            with_port(authority, default_port).eq_ignore_ascii_case(&with_port(host, default_port))
        })
}

/// `authority` with `default_port` appended when it names none.
fn with_port(authority: &str, default_port: &str) -> String {
    let has_port = match authority.rfind(']') {
        Some(end) => authority[end + 1..].starts_with(':'),
        None => authority.contains(':'),
    };
    if has_port {
        authority.to_string()
    } else {
        format!("{authority}:{default_port}")
    }
}

/// The middleware; see the module docs.
pub(crate) async fn origin_guard(
    State(policy): State<OriginPolicy>,
    request: Request,
    next: Next,
) -> Response {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }
    let refused = match request.headers().get(header::ORIGIN) {
        None => None,
        Some(origin) => match origin.to_str() {
            Ok(origin) if policy.admits(origin, request.headers()) => None,
            Ok(origin) => Some(origin.to_string()),
            Err(_) => Some("(not valid text)".to_string()),
        },
    };
    match refused {
        None => next.run(request).await,
        Some(origin) => crate::handlers::helpers::error_response(
            StatusCode::FORBIDDEN,
            format!(
                "requests that change state are refused from origin {origin}: the CORS \
                 policy does not allow it. List it in [server] cors_origins (or [cors] \
                 allowed_origins) and leave cors_enabled true \
                 (see docs/guides/SERVER_SECURITY.md)"
            ),
        ),
    }
}
