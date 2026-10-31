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
//! Names and addresses are compared in canonical form: case-insensitively,
//! port ignored, an IP address as an address (`[FE80:0::1]` is `[fe80::1]`).
//! A `Host` that is not `name[:digits]` or `[IPv6][:digits]`, or not readable
//! text, is refused — never trimmed into the loopback name it starts with.
//! `X-Forwarded-Host` is never read: any client can send it. `/health` and
//! `/ready` (and their `/v1/` forms, exactly) are exempt — orchestrator
//! probes address the server by IP, and those two reveal the status and the
//! version, nothing about the data. A request without any host (no `Host`
//! header, no URI authority) comes from no browser and passes.
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

/// Wraps `router` in the browser-facing policy `config` describes. In the
/// order a request meets them: the `Host` check, then the CORS layer (which
/// answers preflights), then the origin guard. The origin guard's same-origin
/// rule is sound only behind the `Host` check, which is why this is the one
/// public way to install either. The binary calls it; an embedder of
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
    /// The loopback names, plus these (canonical, see [`canonical_entry`]).
    Names(Arc<[String]>),
}

impl HostAllowlist {
    fn from_config(config: &ServerConfig) -> Self {
        if config.allowed_hosts.iter().any(|h| h == "*") {
            return Self::Any;
        }
        let bind = canonical_entry(&config.host).filter(|h| !is_wildcard(h));
        let names = config
            .allowed_hosts
            .iter()
            .filter_map(|h| canonical_entry(h))
            .chain(bind)
            .collect::<Vec<_>>();
        Self::Names(names.into())
    }

    /// `name` is canonical (see [`parse_host`]).
    fn admits(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Names(names) => is_loopback(name) || names.iter().any(|n| n == name),
        }
    }
}

/// A configured host — an `allowed_hosts` entry or the bind host — in the
/// canonical form [`parse_host`] gives a `Host` header, or `None` when it is
/// not a bare name or address: a port, a scheme, a path, a pattern, a
/// trailing dot. An IPv6 address may come with or without brackets.
/// [`crate::config::ServerConfig::validate`] refuses an entry this rejects.
pub(crate) fn canonical_entry(entry: &str) -> Option<String> {
    match entry.parse::<Ipv6Addr>() {
        Ok(ip) => Some(format!("[{ip}]")),
        Err(_) => canonical_name(entry),
    }
}

/// Whether the bind host is a wildcard (`0.0.0.0`, `::`), which names no
/// host: a server bound to it answers only the loopback names and
/// `allowed_hosts`.
pub(crate) fn binds_wildcard(host: &str) -> bool {
    canonical_entry(host).is_some_and(|name| is_wildcard(&name))
}

fn is_wildcard(name: &str) -> bool {
    matches!(name, "0.0.0.0" | "[::]")
}

/// A `Host` value's name in canonical form — `[::1]:8080` → `[::1]`,
/// `LocalHost:80` → `localhost`, `[FE80:0::1]` → `[fe80::1]` — or `None`
/// when it is not `name[:digits]` or `[IPv6][:digits]`.
fn parse_host(host: &str) -> Option<String> {
    let (name, port) = match host.strip_prefix('[') {
        Some(rest) => {
            let end = rest.find(']')?;
            let after = &rest[end + 1..];
            let port = if after.is_empty() {
                None
            } else {
                Some(after.strip_prefix(':')?)
            };
            (&host[..end + 2], port)
        }
        None => match host.split_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        },
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok.then(|| canonical_name(name)).flatten()
}

/// `[IPv6]`, an IPv4 address or a DNS-style name, canonical; `None` for
/// anything else (including a trailing dot or a `*`).
fn canonical_name(name: &str) -> Option<String> {
    if let Some(v6) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        return v6.parse::<Ipv6Addr>().ok().map(|ip| format!("[{ip}]"));
    }
    if let Ok(ip) = name.parse::<Ipv4Addr>() {
        return Some(ip.to_string());
    }
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b));
    valid.then(|| name.to_ascii_lowercase())
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
    // Outer `None`: no host at all. Inner `None`: present but malformed or
    // not text, which fails closed.
    let host: Option<Option<String>> = match request.headers().get(header::HOST) {
        Some(value) => Some(value.to_str().ok().and_then(parse_host)),
        None => request.uri().authority().map(|a| parse_host(a.as_str())),
    };
    match host {
        Some(name)
            if !is_probe(request.uri().path())
                && !name.as_deref().is_some_and(|n| allowlist.admits(n)) =>
        {
            refusal(name.as_deref())
        }
        _ => next.run(request).await,
    }
}

/// The `421`, naming the refused host and every way to allow it.
fn refusal(name: Option<&str>) -> Response {
    let message = match name {
        Some(name) => format!(
            "this server does not answer to host '{name}' (DNS-rebinding protection). \
             If clients reach it under that name, allow it with [server] allowed_hosts = \
             [\"{name}\"], --allowed-hosts {name} or VELESDB_ALLOWED_HOSTS={name} \
             (see docs/guides/SERVER_SECURITY.md#dns-rebinding-and-the-host-check)"
        ),
        None => "the Host header is not a host name, an address or [IPv6], with an \
                 optional numeric port (DNS-rebinding protection)"
            .to_string(),
    };
    crate::handlers::helpers::error_response(StatusCode::MISDIRECTED_REQUEST, message)
}

#[cfg(test)]
#[path = "host_guard_tests.rs"]
mod host_guard_tests;
