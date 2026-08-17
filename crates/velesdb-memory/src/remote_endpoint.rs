//! Where a remote embedding/extraction backend's URL, model and credential
//! come from — one shape for both roles, on purpose. The two were configured
//! differently for historical reasons only — extraction by role, embedding by
//! product — and an operator who has configured one should not have to learn
//! the other (#1751, arbitration C1).
//!
//! Lives in the library, not the daemon binary, so every caller that resolves
//! the embedding role's remote backend reads the same four
//! `VELESDB_MEMORY_EMBEDDER*` variables the same way: the daemon and both
//! language bindings (#1886), instead of the bindings reimplementing (or, as
//! it was until #1886, simply never reading) the resolution the daemon
//! already had.

#[cfg(feature = "embedder-http")]
use crate::config::{alias_conflict_notice, resolve_alias};
use crate::http_client::Auth;

/// A remote backend's configuration, read from one role's environment.
#[derive(Debug)]
pub struct RemoteEndpoint {
    /// Server origin and port, no path. `None` when unset.
    pub url: Option<String>,
    /// Model identifier the server expects. `None` when unset.
    pub model: Option<String>,
    /// The credential, already resolved to what the transport puts on the wire.
    pub auth: Auth,
}

impl RemoteEndpoint {
    /// The URL and model, both **required** — the `openai` shape.
    ///
    /// Neither has a default, and that is the design rather than an omission:
    /// `openai` names a *protocol*, spoken by oMLX, llama.cpp, LM Studio, vLLM
    /// and a dozen hosted providers. Guessing a URL would pick one of them for
    /// the caller, and guessing a model would send a name no server on that
    /// list is obliged to know. Ollama keeps its defaults because it genuinely
    /// has one canonical local address.
    ///
    /// # Errors
    /// A message naming the exact variable that is missing, per role.
    pub fn require(self, prefix: &str) -> Result<(String, String, Auth), String> {
        let url = self.url.ok_or_else(|| {
            format!(
                "{prefix}=openai requires {prefix}_URL — the server's origin and port, \
                 no path (e.g. http://localhost:8020). There is no default: `openai` \
                 is a protocol, and only you know which server speaks it here."
            )
        })?;
        let model = self.model.ok_or_else(|| {
            format!(
                "{prefix}=openai requires {prefix}_MODEL — the model identifier the server expects"
            )
        })?;
        Ok((url, model, self.auth))
    }
}

/// A variable's value, or `None` when it is unset.
fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Read a role's API token and turn it into what the transport will send.
///
/// The token lives in the environment and **nowhere else** — never in the
/// TOML (arbitration B1, enforced by [`crate::config`]'s `deny_unknown_fields`
/// and its redacted refusal), and never as a language-binding constructor
/// argument either, for the same reason: an argument sits in the caller's own
/// source, one `git add .` away from a public history the way a TOML value
/// would be.
///
/// # Errors
/// A variable that is set to an empty or blank value. That is not the same as
/// unset: unset means "send no credential", while empty is a caller whose
/// shell expansion produced nothing, and silently sending no credential would
/// surface as a `401` they cannot explain.
pub fn role_auth(name: &str) -> Result<Auth, String> {
    match env_opt(name) {
        None => Ok(Auth::None),
        Some(token) if token.trim().is_empty() => Err(format!(
            "{name} is set but empty — unset it entirely to send no credential. An \
             empty token would go out as `Authorization: Bearer `, which a server \
             rejects as a bad credential rather than a missing one."
        )),
        Some(token) => Ok(Auth::Bearer(token)),
    }
}

/// The embedding role's endpoint, honouring the legacy `VELESDB_MEMORY_OLLAMA_*`
/// aliases (C1), plus an alias-conflict notice for the caller to print.
///
/// The notice is returned rather than printed here: a library must not write
/// to a caller's stderr on its behalf (the daemon prints it gated on
/// `VELESDB_MEMORY_QUIET`; a language binding embedded in someone else's
/// process gets to decide for itself, and today chooses not to).
///
/// # Errors
/// An `_API_TOKEN` that is set but empty.
#[cfg(feature = "embedder-http")]
pub fn embedder_env_endpoint() -> Result<(RemoteEndpoint, Option<String>), String> {
    let url = resolve_alias(
        env_opt("VELESDB_MEMORY_EMBEDDER_URL").as_deref(),
        env_opt("VELESDB_MEMORY_OLLAMA_URL").as_deref(),
    );
    let model = resolve_alias(
        env_opt("VELESDB_MEMORY_EMBEDDER_MODEL").as_deref(),
        env_opt("VELESDB_MEMORY_OLLAMA_MODEL").as_deref(),
    );
    let mut conflicts = Vec::new();
    if url.conflicting {
        conflicts.push(("VELESDB_MEMORY_EMBEDDER_URL", "VELESDB_MEMORY_OLLAMA_URL"));
    }
    if model.conflicting {
        conflicts.push((
            "VELESDB_MEMORY_EMBEDDER_MODEL",
            "VELESDB_MEMORY_OLLAMA_MODEL",
        ));
    }
    let endpoint = RemoteEndpoint {
        url: url.value,
        model: model.value,
        auth: role_auth("VELESDB_MEMORY_EMBEDDER_API_TOKEN")?,
    };
    Ok((endpoint, alias_conflict_notice(&conflicts)))
}

#[cfg(all(test, feature = "embedder-http"))]
#[path = "remote_endpoint_tests.rs"]
mod tests;
