//! Is a configured remote inference backend actually reachable? (#1751, D2)
//!
//! ## What this closes, and what it deliberately does not
//!
//! The daemon already refuses to start when `autograph` is on and no
//! extraction backend is **configured** — a deterministic misconfiguration the
//! operator can fix before anything runs. It never checked that a configured
//! backend is **reachable**, and that gap is what let an extractor stay broken
//! for weeks: `autograph` degrading in-flight is the correct default (losing
//! the enrichment beats losing the fact), which is exactly why nothing ever
//! said so. Silent in-flight is right; silent *forever* is the defect.
//!
//! So this module produces a **signal**, never a refusal. The issue enumerated
//! three options — a startup warning, a queryable failure counter, or nothing
//! — and the arbitration chose the warning and rejected the counter ("a
//! counter you have to ask for is worth nothing here"). Refusing to start was
//! never on that list, and two facts argue against inventing it:
//!
//!  1. the arbitration's own first property forbids turning a successful
//!     `remember` into an error;
//!  2. unreachable is **transient**. A service manager can start this daemon
//!     before the model server is up, and a daemon that refuses to boot for
//!     that reason is worse than the silence it replaces.
//!
//! The signal is a startup snapshot. It does **not** re-probe: a backend that
//! comes up a minute later simply works, with no further word, because the
//! in-flight path was never the thing that was broken.
//!
//! ## Why a listing and not a generation
//!
//! `GET /v1/models` is served by every OpenAI-compatible server, answers from
//! a table, and loads nothing. Asking `/v1/chat/completions` "are you there"
//! would pull a 35-billion-parameter model into memory to answer — turning a
//! startup check into exactly the cold-load cost tracked separately in #1727.
//!
//! ## Four verdicts, because they lead to four different actions
//!
//! Collapsing them into "not working" is what sends an operator to check a
//! port that is fine. Nothing answered, something answered but does not have
//! the model, something answered and rejected the credential, and something
//! answered but does not serve a listing at all are four different mornings.

use std::time::Duration;

/// What a probe found. Every variant is one distinct next action.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive] // diagnosis outcomes grow as failure modes are learned; matching externally requires a wildcard arm
pub enum Reachability {
    /// The server answered and lists the configured model.
    Reachable,
    /// Nothing answered: wrong port, wrong host, server down.
    Unreachable {
        /// The transport's own words, for the operator's log.
        detail: String,
    },
    /// The server answered and refused the credential.
    Unauthorized,
    /// The server answered, and the configured name is not among the ids it
    /// advertises. **Not** a proven absence: a server may route an alias it
    /// does not list. Measured on 2026-08-02 against a local oMLX server —
    /// `/v1/models` answered 200 with seven ids, none containing `ornith`,
    /// while `/v1/chat/completions` accepted `ornith-35b` and echoed it back
    /// (#1782). The old name for this variant asserted that absence, and the
    /// line it produced sent an operator to repair a healthy configuration.
    ModelNotAdvertised {
        /// How many models it did list — `0` reads very differently from `12`.
        listed: usize,
    },
    /// Something answered but serves no listing. Not a fault by itself: a
    /// gateway may expose only the endpoints it proxies.
    ListingUnsupported,
}

/// Listing endpoint, relative to a normalised base URL.
const MODELS_PATH: &str = "/v1/models";

/// Ask `base_url` whether it is there and whether it has `model`.
///
/// `timeout` is the caller's, and it is the whole budget: a startup path may
/// not wait on a stalled server. Never returns an error — a probe that could
/// fail would need its own error handling at every call site, and the verdict
/// it produces is already the answer.
#[must_use]
pub fn probe_openai(
    base_url: &str,
    model: &str,
    token: Option<&str>,
    timeout: Duration,
) -> Reachability {
    let url = format!("{}{MODELS_PATH}", crate::openai::base_url(base_url));
    // `uniform` also sets the OVERALL deadline, which this construction never
    // did: the doc above has always promised "the whole budget", but without
    // `.timeout()` the worst case was one budget each for connect, write and
    // read — three times the promise. Aligning the code with its own contract
    // is the one behavioral change of the agent consolidation.
    let agent =
        crate::http_client::bounded_agent(crate::http_client::AgentBudget::uniform(timeout));
    let mut request = agent.get(&url);
    if let Some(secret) = token {
        request = request.set("Authorization", &format!("Bearer {secret}"));
    }
    match request.call() {
        Ok(response) => classify_listing(&response.into_string().unwrap_or_default(), model),
        Err(ureq::Error::Status(401 | 403, _)) => Reachability::Unauthorized,
        Err(ureq::Error::Status(404 | 405, _)) => Reachability::ListingUnsupported,
        Err(ureq::Error::Status(code, _)) => Reachability::Unreachable {
            detail: format!("the server answered HTTP {code}"),
        },
        Err(ureq::Error::Transport(transport)) => Reachability::Unreachable {
            detail: transport.to_string(),
        },
    }
}

/// Read an OpenAI-shaped listing without pulling in a JSON parser for four
/// characters of structure: the ids are `"id":"…"` and nothing else in that
/// response has that key.
fn classify_listing(body: &str, model: &str) -> Reachability {
    let ids: Vec<&str> = body
        .split("\"id\"")
        .skip(1)
        .filter_map(|chunk| chunk.split('"').nth(1))
        .collect();
    if ids.is_empty() {
        return Reachability::ListingUnsupported;
    }
    if ids.iter().any(|id| names_the_same_model(id, model)) {
        Reachability::Reachable
    } else {
        Reachability::ModelNotAdvertised { listed: ids.len() }
    }
}

/// Do a listed id and a configured name mean the same model?
///
/// Ollama's listing carries the implicit tag it applies on pull: measured on
/// 2026-08-02, `/v1/models` answers `bge-m3:latest` for a configuration that
/// says `bge-m3`. Strict equality would report a loaded, serving model as
/// absent — and a warning that fires when everything is fine is a warning
/// people learn to skip, which costs more than the silence this replaces.
///
/// The tolerance is that one implicit tag and nothing else. `bge-m3:v2` and
/// `bge-m3:v3` are different models, and a prefix rule would call them equal.
fn names_the_same_model(listed: &str, configured: &str) -> bool {
    listed == configured
        || listed.strip_suffix(":latest") == Some(configured)
        || configured.strip_suffix(":latest") == Some(listed)
}

/// The one line an operator should see at startup, or `None` when there is
/// nothing to say.
///
/// `None` on success is the point: a warning that also fires when everything
/// works is a warning people filter out. Nothing here interpolates the
/// credential — not the value, not the header name — because this line's
/// destination is a log file.
/// What the probe found, and what an operator can do about it.
fn finding(outcome: &Reachability) -> Option<(String, &'static str)> {
    match outcome {
        Reachability::Reachable => None,
        Reachability::Unreachable { detail } => Some((
            format!("unreachable ({detail})"),
            "start the server, or correct the URL",
        )),
        Reachability::Unauthorized => Some((
            "refused the credential".to_owned(),
            "set the role's _API_TOKEN in the environment (never in the TOML)",
        )),
        Reachability::ModelNotAdvertised { listed } => Some((
            format!(
                "answered, but does not advertise this model alias among the \
                 {listed} it lists — the alias may still be routable by the server"
            ),
            "no action if the server routes this alias; otherwise name one it lists",
        )),
        Reachability::ListingUnsupported => Some((
            "answered, but serves no model listing — reachability unconfirmed".to_owned(),
            "no action if this is a gateway; otherwise check the URL's base path",
        )),
    }
}

/// Whether the verdict ESTABLISHES that the backend cannot serve writes, or
/// merely fails to confirm that it can.
///
/// The distinction is the whole of #1782. `/v1/models` is a light signal, and
/// a light signal has limits: it proves a server is up, and it proves nothing
/// about an alias it does not mention. Stating degradation as a fact on that
/// basis is what turned a healthy oMLX configuration into a bug report. The
/// probe stays light either way — confirming an alias would mean asking
/// `/v1/chat/completions`, which pulls a 35-billion-parameter model at startup.
fn proves_backend_unusable(outcome: &Reachability) -> bool {
    match outcome {
        Reachability::Unreachable { .. } | Reachability::Unauthorized => true,
        Reachability::Reachable
        | Reachability::ModelNotAdvertised { .. }
        | Reachability::ListingUnsupported => false,
    }
}

#[must_use]
pub fn warning_line(role: &str, url: &str, model: &str, outcome: &Reachability) -> Option<String> {
    let (what, action) = finding(outcome)?;
    let consequence = if proves_backend_unusable(outcome) {
        "Graph enrichment will degrade silently for every write until it is fixed"
    } else {
        "Whether graph enrichment works is therefore unconfirmed — this is not \
         proof that it is broken"
    };
    Some(format!(
        "velesdb-memory: the {role} backend at {url} (model {model}) {what}. \
         {consequence} — {action}, then restart. To run without it, unset \
         VELESDB_MEMORY_EXTRACTOR or turn autograph off."
    ))
}

#[cfg(test)]
#[path = "reachability_tests.rs"]
mod tests;
