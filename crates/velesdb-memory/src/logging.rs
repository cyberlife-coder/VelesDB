//! Per-request observability, gated by `VELESDB_MEMORY_LOG` (#1780).
//!
//! The daemon used to emit nothing per request: no `tracing` subscriber was
//! ever installed, so both this crate's events and everything rmcp already
//! emits about session lifecycles (idle timeouts, dead channels — exactly
//! the signals #1727 needed) were discarded. #1727 was then diagnosed twice
//! on a wrong cause, and settling it took a throwaway HTTP probe written
//! outside the repository.
//!
//! This module is the deliberately narrow fix: one env var, silent by
//! default.
//!
//! - `VELESDB_MEMORY_LOG` unset (or blank) installs **no subscriber at
//!   all** — the daemon behaves byte-for-byte as before.
//! - Set, its value is a standard `EnvFilter` directive list (e.g. `info`
//!   or `info,rmcp=debug`), rendered to **stderr only**: on the stdio
//!   transport stdout carries the MCP protocol itself, and one log byte
//!   there would corrupt the stream. The HTTP daemon's stderr is already
//!   captured by launchd (`~/Library/Logs/velesdb-memory/daemon.err.log`),
//!   so a log line lands where an operator already looks.
//!
//! What gets traced lives at the call sites (`http::trace_mcp_http`,
//! `mcp`'s `call_tool`): tool name, session id, verdict, duration — never
//! an argument, a payload, or fact content (`tests/daemon_logging.rs`
//! proves that with canaries). This module also owns the vocabulary those
//! two events share — the absent-session placeholder, the duration helper —
//! so the pair cannot drift apart.

use std::time::Instant;

/// The env var that turns logging on. Named (rather than `RUST_LOG`) so an
/// ambient `RUST_LOG` in a developer's shell cannot make the daemon
/// talkative by accident — enabling logs here is an explicit, per-daemon
/// decision.
pub const LOG_ENV_VAR: &str = "VELESDB_MEMORY_LOG";

/// The filter an operator should run to diagnose a session incident (#1727):
/// this crate's per-request events, plus rmcp's session-lifecycle signals —
/// and **no client content, ever**, which is the property that makes it safe
/// to leave on in a deployed daemon (`scripts/install-memory-daemon.sh`
/// wires exactly this string into the launchd plist; a test below refuses
/// drift). Directive by directive:
///
/// - `info` — this crate's own per-request events (transport and tool).
/// - `rmcp::service=error` — NOT `warn` or the bare default: at `warn`,
///   rmcp's `response error` event dumps the whole `ErrorData`, and several
///   `MemoryError` messages quote client input verbatim (an invalid filter's
///   field name, the full offending JSON value). The #1780 review proved
///   that leak in execution; `tests/daemon_logging.rs` pins it with
///   error-path canaries. At `info` the same target also dumps
///   notifications. `error` keeps only content-free faults.
/// - `rmcp::transport::worker=debug` — carries `WorkerQuitReason`, including
///   the idle-timeout that is THE #1727 signal (a session whose worker died
///   of inactivity while the session stayed in the table).
/// - `rmcp::transport::streamable_http_server=debug` — session/channel
///   lifecycle (open, close, dead channel), all content-free at that level.
///
/// Broader rmcp verbosity DUMPS REQUEST CONTENT: `rmcp::service` logs every
/// request's full arguments — fact text included — at `debug`, and the
/// transport tower logs whole messages at `trace`. So `rmcp=debug` is NOT a
/// harmless step up from this preset; it is the payload firehose, acceptable
/// only for deliberate wire debugging on data that may land in a log file.
/// `tests/daemon_logging.rs` captures under THIS preset and asserts canaries
/// (fact content on the happy path, client input on the error path) never
/// reach the log — if a dependency upgrade (e.g. rmcp 3.x, #1789) moves a
/// dump to a level this preset admits, those tests go red before the leak
/// ships.
pub const INCIDENT_PRESET: &str = "info,rmcp::service=error,rmcp::transport::worker=debug,rmcp::transport::streamable_http_server=debug";

/// Read [`LOG_ENV_VAR`] and install the stderr subscriber it asks for.
/// Unset or blank installs nothing — see the module docs.
///
/// # Errors
/// A value that does not parse as `EnvFilter` directives, or a subscriber
/// already installed for this process. Both abort startup rather than run
/// the daemon with logging silently different from what the operator asked
/// for — same posture as the config file (`crate::config`): a daemon
/// quietly running on defaults the operator believes they overrode is worse
/// than a loud failure at boot.
pub fn init_from_env() -> Result<(), String> {
    match filter_from_raw(std::env::var(LOG_ENV_VAR).ok().as_deref())? {
        None => Ok(()),
        Some(filter) => install(filter),
    }
}

/// The parsing half of [`init_from_env`], taking the raw value instead of
/// reading it — same testability idiom as `http::keep_alive_from_raw`
/// (process-wide env vars are shared mutable state under a parallel test
/// runner).
///
/// `None` and blank mean "no logging requested" and yield `Ok(None)`; any
/// other value must be a valid `EnvFilter` directive list.
///
/// # Errors
/// A set, non-blank value that `EnvFilter` refuses, with the exact
/// directive text and the var's name in the message.
fn filter_from_raw(raw: Option<&str>) -> Result<Option<tracing_subscriber::EnvFilter>, String> {
    let Some(directives) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    tracing_subscriber::EnvFilter::try_new(directives)
        .map(Some)
        .map_err(|err| {
            format!(
                "{LOG_ENV_VAR}='{directives}' is not a valid filter ({err}) — use EnvFilter \
                 directives, e.g. 'info' or 'info,rmcp=debug', or unset it for silence"
            )
        })
}

/// Install the stderr `fmt` subscriber filtered by `filter`.
fn install(filter: tracing_subscriber::EnvFilter) -> Result<(), String> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|err| format!("{LOG_ENV_VAR}: cannot install the log subscriber: {err}"))
}

/// What the `session` field carries when a request has none (stdio, or an
/// `initialize` that hasn't been assigned one yet). A stable placeholder
/// rather than an omitted field, so `grep session=` matches every event.
pub(crate) const NO_SESSION: &str = "-";

/// Milliseconds since `started`, saturating instead of panicking — shared by
/// the transport- and tool-level trace events so the two report durations
/// that are comparable by construction.
pub(crate) fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::filter_from_raw;

    #[test]
    fn unset_or_blank_yields_no_filter() {
        // The "silent by default" half of the contract: no filter, so
        // `init_from_env` installs no subscriber — the daemon must behave
        // byte-for-byte as before.
        assert!(matches!(filter_from_raw(None), Ok(None)));
        assert!(matches!(filter_from_raw(Some("")), Ok(None)));
        assert!(matches!(filter_from_raw(Some("   ")), Ok(None)));
    }

    #[test]
    fn a_directive_list_yields_a_filter() {
        // Positive control for the silence test above: a function that
        // answered `None` to everything would pass it vacuously.
        assert!(matches!(filter_from_raw(Some("info")), Ok(Some(_))));
        assert!(matches!(
            filter_from_raw(Some("info,rmcp=debug")),
            Ok(Some(_))
        ));
    }

    #[test]
    fn the_incident_preset_parses() {
        // The preset is dead the day it stops parsing — the daemon would
        // refuse to boot with it, which is exactly when an operator needs it.
        assert!(matches!(
            filter_from_raw(Some(super::INCIDENT_PRESET)),
            Ok(Some(_))
        ));
    }

    #[test]
    fn the_installer_ships_the_incident_preset_verbatim() {
        // The plist is written by a shell script that cannot read this
        // constant, so the two CAN drift — and a drifted installer would
        // deploy a daemon logging either nothing or, worse, a payload-leaking
        // filter. Verbatim containment is the whole check.
        //
        // Read at runtime, not `include_str!`: `scripts/` is not packaged
        // into the published .crate, so a compile-time include would make
        // `cargo test` unbuildable from the .crate or a vendored source.
        // Outside the repository there is no installer to drift against, so
        // absence is a genuine pass — in the repository (and its CI, where
        // this guard matters) the file always exists.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/install-memory-daemon.sh"
        );
        let Ok(installer) = std::fs::read_to_string(path) else {
            return;
        };
        assert!(
            installer.contains(super::INCIDENT_PRESET),
            "scripts/install-memory-daemon.sh must wire VELESDB_MEMORY_LOG to the \
             incident preset exactly as src/logging.rs declares it"
        );
    }

    #[test]
    fn an_unparseable_value_is_refused_with_the_var_name() {
        // Refusal proven, not assumed: an operator who set a broken filter
        // asked for logs and must not silently get silence instead.
        let err = filter_from_raw(Some("velesdb=notalevel"))
            .expect_err("an invalid level must be refused");
        assert!(
            err.contains("VELESDB_MEMORY_LOG"),
            "the message must name the var to fix, got: {err}"
        );
    }
}
