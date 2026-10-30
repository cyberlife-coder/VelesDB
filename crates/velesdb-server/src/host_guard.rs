//! Refuses a request addressed to a host name the server does not answer to.
//!
//! DNS rebinding: a page on `evil.example.com` makes that name resolve to
//! `127.0.0.1` once it has loaded. Its requests then reach a loopback server
//! while the browser still treats them as same-origin, so neither CORS nor
//! the origin guard can tell them apart — and the default server, bound to
//! `127.0.0.1:8080` without API keys, would answer reads and writes alike.
//! The one thing the page cannot change is the `Host` header, which still
//! names `evil.example.com`. So every request, whatever its method, must be
//! addressed to:
//!
//! - `localhost`, an address in `127.0.0.0/8`, or `[::1]` — answered with no
//!   configuration, so a loopback server needs none;
//! - the bind host, unless it is a wildcard (`0.0.0.0`, `::`), which names
//!   no host;
//! - an entry of `[server] allowed_hosts`, or anything when that list is
//!   `["*"]`.
//!
//! Matching ignores case and the port. `X-Forwarded-Host` is never read: any
//! client can send it. `/health` and `/ready` are exempt — orchestrator probes
//! address the server by IP, and those two say only "up". A request without
//! any host (no `Host` header, no URI authority) comes from no browser and
//! passes.
//!
//! The refusal is `421 Misdirected Request`, RFC 9110's status for a request
//! sent to a server that does not answer for its authority; it also tells an
//! operator which guard fired, the origin guard's being `403`.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::config::ServerConfig;

/// Wraps `router` in the browser-facing policy `config` describes, outermost
/// first: the `Host` check, then the origin guard and the CORS layer
/// ([`crate::config::apply_cors`]). The binary calls this; an embedder of
/// [`crate::routes::api_routes`] should too.
pub fn apply_browser_policy<S>(router: axum::Router<S>, config: &ServerConfig) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    crate::cors::apply_cors(router, &config.cors).layer(axum::middleware::from_fn_with_state(
        HostAllowlist::from_config(config),
        host_guard,
    ))
}

/// The host names a request may be addressed to.
#[derive(Debug, Clone)]
pub(crate) enum HostAllowlist {
    /// `allowed_hosts = ["*"]`: the check is off.
    Any,
    /// The loopback names, plus these (lower case, brackets kept for IPv6).
    Names(Arc<[String]>),
}

impl HostAllowlist {
    fn from_config(config: &ServerConfig) -> Self {
        if config.allowed_hosts.iter().any(|h| h == "*") {
            return Self::Any;
        }
        let bind = Some(as_host_header(&config.host)).filter(|h| !is_wildcard(h));
        let names = config
            .allowed_hosts
            .iter()
            .map(|h| as_host_header(h))
            .chain(bind)
            .collect::<Vec<_>>();
        Self::Names(names.into())
    }

    fn admits(&self, host: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Names(names) => {
                let name = host_name(host);
                is_loopback(&name) || names.contains(&name)
            }
        }
    }
}

/// A configured host (the bind host, an `allowed_hosts` entry) as it
/// appears in a `Host` header: lower case, an IPv6 address in brackets.
fn as_host_header(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host
    }
}

fn is_wildcard(name: &str) -> bool {
    matches!(name, "0.0.0.0" | "[::]")
}

/// `Host` without its port, lower case: `[::1]:8080` → `[::1]`,
/// `LocalHost:80` → `localhost`.
fn host_name(host: &str) -> String {
    let host = host.trim();
    let name = if let Some(end) = host.find(']') {
        &host[..=end]
    } else {
        host.rsplit_once(':').map_or(host, |(name, _)| name)
    };
    name.to_ascii_lowercase()
}

fn is_loopback(name: &str) -> bool {
    if name == "localhost" {
        return true;
    }
    if let Some(v6) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        return v6.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    name.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// The probes an orchestrator sends by IP; they answer any `Host`.
fn is_probe(path: &str) -> bool {
    matches!(path, "/health" | "/ready" | "/v1/health" | "/v1/ready")
}

/// The middleware; see the module docs.
async fn host_guard(
    State(allowlist): State<HostAllowlist>,
    request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned)
        .or_else(|| request.uri().authority().map(|a| a.as_str().to_owned()));
    match host {
        Some(host) if !is_probe(request.uri().path()) && !allowlist.admits(&host) => {
            crate::handlers::helpers::error_response(
                StatusCode::MISDIRECTED_REQUEST,
                format!(
                    "this server does not answer to host {host}; add it to [server] \
                     allowed_hosts (VELESDB_ALLOWED_HOSTS) if it is yours \
                     (see docs/guides/SERVER_SECURITY.md)"
                ),
            )
        }
        _ => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{as_host_header, host_name, is_loopback};

    #[test]
    fn test_host_name_drops_the_port_and_the_case() {
        assert_eq!(host_name("LocalHost:8080"), "localhost");
        assert_eq!(host_name("[::1]:8080"), "[::1]");
        assert_eq!(host_name("[::1]"), "[::1]");
        assert_eq!(host_name("db.example.com"), "db.example.com");
    }

    #[test]
    fn test_loopback_covers_the_whole_ipv4_block_and_ipv6() {
        for name in ["localhost", "127.0.0.1", "127.255.0.9", "[::1]"] {
            assert!(is_loopback(name), "{name}");
        }
        for name in ["128.0.0.1", "[::2]", "localhost.evil.com", "0.0.0.0"] {
            assert!(!is_loopback(name), "{name}");
        }
    }

    #[test]
    fn test_configured_hosts_are_spelt_as_a_host_header_would() {
        assert_eq!(as_host_header("::"), "[::]");
        assert_eq!(as_host_header("fe80::1"), "[fe80::1]");
        assert_eq!(as_host_header("My-Box.local"), "my-box.local");
    }
}
