//! CORS: the `[cors]` table and `[server]`'s `cors_enabled` / `cors_origins`,
//! resolved into a [`CorsConfig`] and turned into a `tower_http` layer.
//!
//! The types and functions here are re-exported from [`crate::config`],
//! where they lived before this module was split out of it.

use serde::Deserialize;

/// `[cors]` as written in the file; every field optional.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct CorsSection {
    pub(crate) allowed_origins: Option<Vec<String>>,
    pub(crate) allowed_methods: Option<Vec<String>>,
    pub(crate) allowed_headers: Option<Vec<String>>,
    pub(crate) allow_credentials: Option<bool>,
    pub(crate) max_age_secs: Option<u64>,
    #[serde(flatten)]
    pub(crate) unknown: crate::config::UnknownKeys,
}

/// CORS configuration for the server.
///
/// With no origin listed — the default — or `enabled = false`, the server
/// installs no CORS layer: no CORS header is sent, and browsers let only
/// same-origin pages read its responses. `["*"]`, written explicitly, is the
/// fully permissive policy (`CorsLayer::permissive()`). Any other list allows
/// exactly those origins, each compared byte for byte with the request's
/// `Origin` header.
///
/// The default was `["*"]` up to 7.1.0; a database reachable from every web
/// page its user visits is not a default (see `docs/guides/SERVER_SECURITY.md`).
#[derive(Debug, Clone, PartialEq)]
pub struct CorsConfig {
    /// Whether the server answers CORS at all (`[server] cors_enabled`).
    pub enabled: bool,
    /// Allowed origins; empty (the default) allows none, `["*"]` allows all.
    pub allowed_origins: Vec<String>,
    /// Allowed HTTP methods (e.g. `["GET", "POST"]`).
    pub allowed_methods: Vec<String>,
    /// Allowed request headers (e.g. `["Content-Type", "Authorization"]`).
    /// Use `["*"]` to allow any header.
    pub allowed_headers: Vec<String>,
    /// Whether to allow credentials (cookies, authorization headers).
    pub allow_credentials: bool,
    /// How long (in seconds) browsers may cache preflight responses.
    pub max_age_secs: u64,
}

/// Default preflight cache duration in seconds (1 hour).
const DEFAULT_CORS_MAX_AGE_SECS: u64 = 3600;

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_origins: Vec::new(),
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            allowed_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age_secs: DEFAULT_CORS_MAX_AGE_SECS,
        }
    }
}

impl CorsConfig {
    /// Returns `true` when the server answers CORS at all: enabled, with at
    /// least one origin listed.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.allowed_origins.is_empty()
    }

    /// Returns `true` when CORS is in fully permissive mode (any origin).
    pub fn is_permissive(&self) -> bool {
        self.allowed_origins.iter().any(|o| o == "*")
    }

    /// Rejects what [`build_cors_layer`] would otherwise have to drop in
    /// silence: an origin a browser can never send, `"*"` mixed into an
    /// explicit list (which would quietly allow everything), a setting the
    /// `"*"` policy ignores (see [`Self::validate_wildcard`]), and a method or
    /// header name that does not parse.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        self.validate_wildcard()?;
        for origin in self.allowed_origins.iter().filter(|o| *o != "*") {
            validate_origin(origin)?;
        }
        for method in &self.allowed_methods {
            method
                .parse::<axum::http::Method>()
                .map_err(|e| anyhow::anyhow!("cors.allowed_methods: '{method}': {e}"))?;
        }
        for header in self.allowed_headers.iter().filter(|h| *h != "*") {
            header
                .parse::<axum::http::HeaderName>()
                .map_err(|e| anyhow::anyhow!("cors.allowed_headers: '{header}': {e}"))?;
        }
        Ok(())
    }
}

impl CorsConfig {
    /// `"*"` must stand alone, and nothing may be set that the permissive
    /// policy it selects ([`tower_http::cors::CorsLayer::permissive`]) would
    /// ignore: it allows every method and header, has no preflight cache
    /// setting, and never allows credentials — a browser refuses
    /// `Access-Control-Allow-Credentials` next to a wildcard origin anyway.
    /// A setting left at its default value is not an instruction, so it
    /// passes.
    fn validate_wildcard(&self) -> anyhow::Result<()> {
        if !self.is_permissive() {
            return Ok(());
        }
        if self.allowed_origins.len() > 1 {
            anyhow::bail!(
                "CORS origins {:?}: \"*\" allows every origin and cannot be combined \
                 with an explicit list",
                self.allowed_origins
            );
        }
        let defaults = Self::default();
        let ignored: Vec<&str> = [
            ("allow_credentials", self.allow_credentials),
            (
                "allowed_methods",
                self.allowed_methods != defaults.allowed_methods,
            ),
            (
                "allowed_headers",
                self.allowed_headers != defaults.allowed_headers,
            ),
            ("max_age_secs", self.max_age_secs != defaults.max_age_secs),
        ]
        .into_iter()
        .filter_map(|(key, set)| set.then_some(key))
        .collect();
        if !ignored.is_empty() {
            anyhow::bail!(
                "CORS origins [\"*\"] select the permissive policy, which allows every \
                 method and header and never credentials, so [cors] {} would be \
                 ignored: remove {}, or list the allowed origins explicitly",
                ignored.join(", "),
                if ignored.len() == 1 { "it" } else { "them" }
            );
        }
        Ok(())
    }
}

/// An origin is `scheme://host[:port]` — what a browser puts in `Origin`.
/// Anything with a path, even a lone trailing `/`, can never equal that
/// header, so it would allow nothing while looking like it allows a site.
fn validate_origin(origin: &str) -> anyhow::Result<()> {
    let well_formed = origin.split_once("://").is_some_and(|(scheme, authority)| {
        !scheme.is_empty() && !authority.is_empty() && !authority.contains('/')
    }) && axum::http::HeaderValue::from_str(origin).is_ok();
    if !well_formed {
        anyhow::bail!(
            "CORS origin '{origin}' is not an origin: expected scheme://host[:port], \
             with no path and no trailing slash (it is compared exactly with the \
             browser's Origin header)"
        );
    }
    Ok(())
}

/// Merges the `[cors]` section and `[server]`'s `cors_enabled` /
/// `cors_origins` over `CorsConfig` defaults.
///
/// `[server] cors_origins` and `[cors] allowed_origins` are two names for one
/// list; setting both is refused rather than letting one win in silence.
pub(crate) fn resolve_cors(
    defaults: &CorsConfig,
    section: CorsSection,
    enabled: Option<bool>,
    server_origins: Option<Vec<String>>,
) -> anyhow::Result<CorsConfig> {
    if section.allowed_origins.is_some() && server_origins.is_some() {
        anyhow::bail!(
            "both [server] cors_origins and [cors] allowed_origins are set; \
             they configure the same list, keep one"
        );
    }
    let defaults = defaults.clone();
    Ok(CorsConfig {
        enabled: enabled.unwrap_or(defaults.enabled),
        allowed_origins: server_origins
            .or(section.allowed_origins)
            .unwrap_or(defaults.allowed_origins),
        allowed_methods: section.allowed_methods.unwrap_or(defaults.allowed_methods),
        allowed_headers: section.allowed_headers.unwrap_or(defaults.allowed_headers),
        allow_credentials: section
            .allow_credentials
            .unwrap_or(defaults.allow_credentials),
        max_age_secs: section.max_age_secs.unwrap_or(defaults.max_age_secs),
    })
}

/// Wraps `router` in the policy `cors` describes, on both sides of what a
/// browser enforces:
///
/// - the origin guard (`origin_guard`), which refuses with `403` a request
///   that would change state (any method but `GET`, `HEAD`, `OPTIONS`) from
///   an `Origin` the policy does not allow — CORS alone cannot, since a
///   request sent without a preflight runs before the browser hides its
///   answer;
/// - the CORS layer, when [`CorsConfig::is_active`]; otherwise no CORS
///   header is sent, and browsers refuse every cross-origin read.
pub(crate) fn apply_cors<S>(router: axum::Router<S>, cors: &CorsConfig) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let router = router.layer(axum::middleware::from_fn_with_state(
        crate::origin_guard::OriginPolicy::from_cors(cors),
        crate::origin_guard::origin_guard,
    ));
    if cors.is_active() {
        router.layer(build_cors_layer(cors))
    } else {
        router
    }
}

/// Builds a [`tower_http::cors::CorsLayer`] from the resolved CORS config.
///
/// When `allowed_origins` contains `"*"`, returns `CorsLayer::permissive()`
/// for full backward compatibility. Otherwise, constructs a restrictive
/// layer with the specified origins, methods, and headers.
pub fn build_cors_layer(cors: &CorsConfig) -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};

    if cors.is_permissive() {
        return CorsLayer::permissive();
    }

    let origins: Vec<axum::http::HeaderValue> = cors
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let methods: Vec<axum::http::Method> = cors
        .allowed_methods
        .iter()
        .filter_map(|m| m.parse().ok())
        .collect();

    let layer = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(methods)
        .max_age(std::time::Duration::from_secs(cors.max_age_secs));

    let layer = apply_cors_headers_policy(layer, cors);

    if cors.allow_credentials {
        layer.allow_credentials(true)
    } else {
        layer
    }
}

/// Applies the headers policy to a `CorsLayer`, honouring the CORS spec rule that
/// `allow_credentials=true` is incompatible with wildcard headers (browsers reject
/// the preflight). Logs a warning and falls back to default headers in that case.
fn apply_cors_headers_policy(
    layer: tower_http::cors::CorsLayer,
    cors: &CorsConfig,
) -> tower_http::cors::CorsLayer {
    use tower_http::cors::Any;

    let has_wildcard = cors.allowed_headers.iter().any(|h| h == "*");
    if has_wildcard && !cors.allow_credentials {
        return layer.allow_headers(Any);
    }
    if has_wildcard && cors.allow_credentials {
        tracing::warn!(
            "CORS: allow_credentials=true is incompatible with wildcard \
             headers per CORS spec. Falling back to default headers \
             (Content-Type, Authorization)."
        );
    }
    let headers: Vec<axum::http::HeaderName> = cors
        .allowed_headers
        .iter()
        .filter(|h| h.as_str() != "*")
        .filter_map(|h| h.parse().ok())
        .collect();
    if headers.is_empty() && cors.allow_credentials {
        layer.allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
    } else {
        layer.allow_headers(headers)
    }
}
