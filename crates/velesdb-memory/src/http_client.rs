//! Authenticated JSON over HTTP — the transport layer under every remote
//! inference backend.
//!
//! # What this module deliberately does NOT know
//!
//! Not which *role* is calling: nothing here mentions embedding or extraction,
//! and the caller supplies its own [`ureq::Agent`] precisely because the two
//! roles need different ceilings (an embedding answers in a moment, a
//! generation can take minutes). A client that built its own agent would have
//! to know which of the two it was serving.
//!
//! Not which *vendor* is answering either. Once the path, the body and the
//! auth scheme all come from the caller, nothing OpenAI-specific is left —
//! which is the honest reason this is not called an "OpenAI client". The
//! OpenAI protocol lives one layer up, in [`crate::openai`]; Azure, Gemini or
//! Anthropic would each get their own protocol module over this same
//! transport.
//!
//! Not `velesdb-server`'s [`crate::http`] either, despite the neighbouring
//! name: that module *serves* MCP over HTTP, this one *calls* a model.

use crate::http_retry;

/// How a request proves who it is.
///
/// An enum rather than an `Option<String>` bearer token, because the shape
/// varies by provider and was already known to vary before the first
/// non-OpenAI one landed: Azure `OpenAI` authenticates with `api-key`, a local
/// server wants nothing at all, and a future provider will want something
/// else again. Widening an `Option<String>` later would break every caller;
/// adding a variant here does not.
#[non_exhaustive] // authentication schemes grow; matching externally requires a wildcard arm
pub enum Auth {
    /// Send no credential header at all.
    ///
    /// The default for a model running on the caller's own machine. "No
    /// token" must mean **no header**, not an empty one: a server that
    /// validates the header's *presence* rejects `Authorization: Bearer `
    /// with an error that reads like a bad credential rather than a missing
    /// one.
    None,
    /// `Authorization: Bearer <token>` — the `OpenAI` convention.
    Bearer(String),
    /// A verbatim `name: value` header, for a provider that authenticates
    /// some other way. No bearer is added alongside it.
    Header {
        /// Header name (e.g. `api-key`).
        name: String,
        /// Header value. Treated as a secret.
        value: String,
    },
}

/// Hand-written so a credential never reaches a log or a panic message
/// through the derive. The same reflex [`crate::ExtractorSelection`] applies
/// to a backend's URL, applied to something that matters more.
///
/// The header NAME survives: it is not a secret, and printing it is what
/// makes a provider misconfigured with the wrong scheme diagnosable at all.
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Bearer(_) => f.write_str("Bearer(<redacted>)"),
            Self::Header { name, .. } => {
                write!(f, "Header {{ name: {name:?}, value: <redacted> }}")
            }
        }
    }
}

/// Everything a failed call can say WITHOUT knowing what it was for.
///
/// The role-specific half of a good error message — which model was asked,
/// which environment variables repoint it — belongs to the caller, which is
/// why this carries only transport facts and leaves the rendering to the
/// provider-appropriate renderer in [`http_retry`].
pub(crate) struct HttpFailure {
    /// Full URL that was called, for the message the caller renders.
    pub url: String,
    /// How many attempts were spent before giving up.
    pub attempts: u32,
    /// Why the last attempt failed.
    pub cause: String,
}

/// A JSON-over-HTTP caller bound to one base URL and one credential.
pub struct HttpJsonClient {
    base_url: String,
    auth: Auth,
    agent: ureq::Agent,
}

/// Hand-written rather than derived: `ureq::Agent` is not `Debug`, and the
/// credential must go through [`Auth`]'s own redacting impl rather than
/// whatever a derive would have produced. The agent is omitted entirely —
/// its timeouts belong to the caller that built it.
impl std::fmt::Debug for HttpJsonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpJsonClient")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl HttpJsonClient {
    /// Bind a client to `base_url`, authenticating with `auth`.
    ///
    /// `base_url` is kept verbatim and only concatenated with the caller's
    /// path, so a non-standard port (`http://localhost:8020`) needs no special
    /// handling — it is already part of the string.
    ///
    /// `agent` is the caller's, not this module's: see the module docs.
    #[must_use]
    pub fn new(base_url: impl Into<String>, auth: Auth, agent: ureq::Agent) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            auth,
            agent,
        }
    }

    /// The URL `path` resolves to. Exposed for the caller's error messages,
    /// which name the endpoint they failed against.
    pub(crate) fn url_for(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// POST `body` as JSON to `path`, retrying transient transport failures,
    /// and return the raw response body.
    ///
    /// # Errors
    /// [`HttpFailure`] carrying the URL, the attempt count and the cause —
    /// never a rendered sentence, since only the caller knows the levers that
    /// would change the outcome.
    pub(crate) fn post_json(&self, path: &str, body: &str) -> Result<String, HttpFailure> {
        let url = self.url_for(path);
        let attempt = || {
            let response = self
                .authenticated(self.agent.post(&url))
                .set("Content-Type", "application/json")
                .send_string(body)
                .map_err(|err| Call::Transport(Box::new(err)))?;
            response.into_string().map_err(Call::Body)
        };

        http_retry::with_retry(&http_retry::HTTP_RETRIES, call_is_retryable, attempt).map_err(
            |(err, attempts)| HttpFailure {
                url,
                attempts,
                cause: match err {
                    Call::Transport(inner) => inner.to_string(),
                    Call::Body(inner) => format!("reading the response failed: {inner}"),
                },
            },
        )
    }

    /// Apply the credential — or, for [`Auth::None`], apply nothing.
    ///
    /// The `None` arm returns the request UNTOUCHED. That is the whole point
    /// of the variant and the thing the wire-level tests assert: not an empty
    /// header, not a header with an empty value, no header.
    fn authenticated(&self, request: ureq::Request) -> ureq::Request {
        match &self.auth {
            Auth::None => request,
            Auth::Bearer(token) => request.set("Authorization", &format!("Bearer {token}")),
            Auth::Header { name, value } => request.set(name, value),
        }
    }
}

/// How one attempt failed, kept apart just long enough to classify it —
/// mirrors the shape both role-specific backends already use.
enum Call {
    /// The request never completed (reset, refusal, timeout, error status).
    /// Boxed: `ureq::Error::Status` carries a whole `Response`.
    Transport(Box<ureq::Error>),
    /// The response arrived but its body could not be read.
    Body(std::io::Error),
}

/// Replay a transport hiccup; never replay a body the server finished sending.
fn call_is_retryable(err: &Call) -> bool {
    match err {
        Call::Transport(inner) => http_retry::is_retryable(inner),
        Call::Body(inner) => http_retry::io_is_retryable(inner),
    }
}

/// The three timeout ceilings a remote-inference agent is built from.
///
/// A budget, not an agent: the *values* are role knowledge (an embedding that
/// has not answered in a minute is not going to; a generation legitimately
/// takes hundreds of seconds), so they stay declared next to the role. What
/// lives here is the *shape* — which knobs exist and what they actually do —
/// because it was declared four times over three files, each copy one edit
/// away from drifting from the others.
#[derive(Debug, Clone, Copy)]
pub struct AgentBudget {
    /// Ceiling on establishing the TCP connection. The one knob that
    /// genuinely changes behavior over `ureq`'s defaults: its own connect
    /// default is 30 s (`agent.rs`) — sane for the open internet, absurd for
    /// a daemon on `localhost`, which either accepts immediately or is not
    /// running. With replays, that idle wait would be paid three times over.
    pub connect: std::time::Duration,
    /// Ceiling on writing the request. Applied to the socket at connect time,
    /// so — unlike the read deadline — it is in force independently of the
    /// overall budget.
    pub write: std::time::Duration,
    /// Whole-request deadline, connect included.
    pub overall: std::time::Duration,
}

impl AgentBudget {
    /// Budget for a model daemon expected on `localhost`: 2 s to connect,
    /// 10 s to write, `overall` to answer. The connect and write figures are
    /// shared by the embedding and extraction roles on purpose — they bound
    /// the *transport* to a local daemon, which is the same transport for
    /// both; only `overall` is role knowledge.
    #[must_use]
    pub const fn local_daemon(overall: std::time::Duration) -> Self {
        Self {
            connect: std::time::Duration::from_secs(2),
            write: std::time::Duration::from_secs(10),
            overall,
        }
    }

    /// One figure for every ceiling — for probes whose caller states a single
    /// whole budget and means it.
    #[must_use]
    pub const fn uniform(budget: std::time::Duration) -> Self {
        Self {
            connect: budget,
            write: budget,
            overall: budget,
        }
    }
}

/// Build the agent a budget describes.
///
/// # Precedence, stated plainly (the authoritative copy)
///
/// `ureq` documents that `.timeout()` "takes precedence over `.timeout_read()`
/// and `.timeout_write()`, but not `.timeout_connect()`", and its
/// `DeadlineStream` rewrites the socket read deadline to the remaining global
/// budget before every read. So the `.timeout_read()` set here is
/// **subordinate**: declared for the day the global bound is lifted, and not
/// to be read as a per-read ceiling today. `.timeout_connect()` and
/// `.timeout_write()` are the two that bite alongside the global deadline.
/// Saying otherwise in a doc — or writing a test that claimed to prove a
/// per-read bound — would be a reassurance with nothing behind it.
#[must_use]
pub fn bounded_agent(budget: AgentBudget) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(budget.connect)
        .timeout_write(budget.write)
        .timeout_read(budget.overall)
        .timeout(budget.overall)
        .build()
}

#[cfg(test)]
#[path = "http_client_tests.rs"]
mod tests;
