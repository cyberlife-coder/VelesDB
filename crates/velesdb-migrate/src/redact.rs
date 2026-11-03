//! Redaction helpers for values that may carry credentials into log output.

/// Masks credentials embedded in a connection string so it is safe to log.
///
/// Two shapes are stripped:
/// - the `user[:pass]@` userinfo of a `scheme://` URL (`redis://:pw@host`
///   becomes `redis://***@host`), and
/// - any `?query` or `#fragment`, which is where `api-key=`-style secrets
///   are typically smuggled.
///
/// Plain file paths and credential-free URLs are returned unchanged.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let base = url.split(['?', '#']).next().unwrap_or(url);
    if let Some((scheme, rest)) = base.split_once("://") {
        if let Some((_userinfo, host)) = rest.split_once('@') {
            return format!("{scheme}://***@{host}");
        }
    }
    base.to_string()
}
