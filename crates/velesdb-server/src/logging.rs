//! The `[logging]` section of `velesdb.toml`, and the `tracing` subscriber it
//! configures.
//!
//! The level resolves in one place, [`filter_directives`], from strongest to
//! weakest:
//!
//! ```text
//! RUST_LOG  >  VELESDB_LOGGING_LEVEL  >  [logging] level  >  "info,tower_http=debug"
//! ```
//!
//! `RUST_LOG` stays on top because it was the only knob before `[logging]`
//! was read, it takes full `tracing` directives (`velesdb_core=debug,...`)
//! rather than one level, and a deployment that sets it must keep the output
//! it had. Below it, the order is the server's usual one: environment over
//! file over default. A variable set to the empty string counts as unset, so
//! `RUST_LOG=` in a container hands the level back to the file.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer, Registry};

/// The filter used when neither `RUST_LOG`, `VELESDB_LOGGING_LEVEL` nor
/// `[logging] level` is set: INFO, plus the per-request DEBUG events of
/// `tower_http`'s trace layer. It predates the `[logging]` section and is
/// kept so an unconfigured server logs exactly what it always did.
pub const DEFAULT_FILTER: &str = "info,tower_http=debug";

/// The environment variable that overrides `[logging] level`.
pub const LEVEL_ENV: &str = "VELESDB_LOGGING_LEVEL";

/// `[logging]` as written in the file. Every field is optional; keys this
/// struct does not name land in `unknown` and are reported at startup.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct LoggingSection {
    pub(crate) level: Option<String>,
    pub(crate) format: Option<String>,
    pub(crate) file: Option<String>,
    #[serde(flatten)]
    pub(crate) unknown: crate::config::UnknownKeys,
}

/// One of the levels `[logging] level` accepts: exactly
/// [`velesdb_core::config::LOG_LEVELS`], the list the engine validates its
/// own `logging.level` against — one definition, so the two cannot drift.
/// Each name is also the one `tracing`'s filter syntax uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogLevel(&'static str);

impl LogLevel {
    /// Parses `value` for the setting named `source` (used in the error).
    ///
    /// # Errors
    ///
    /// Returns an error naming `source` when `value` is not one of
    /// [`velesdb_core::config::LOG_LEVELS`], spelt as there.
    pub fn parse(value: &str, source: &str) -> anyhow::Result<Self> {
        let levels = velesdb_core::config::LOG_LEVELS;
        match levels.iter().find(|level| **level == value) {
            Some(level) => Ok(Self(level)),
            None => {
                anyhow::bail!("{source}: value '{value}' is invalid, expected one of: {levels:?}")
            }
        }
    }

    /// The level's name, as `tracing`'s filter syntax spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// How each log event is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Human-readable lines (`tracing_subscriber`'s default format).
    #[default]
    Text,
    /// One JSON object per line (`tracing_subscriber`'s JSON format).
    Json,
}

impl LogFormat {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => anyhow::bail!(
                "logging.format: value '{value}' is invalid, expected one of: [\"text\", \"json\"]"
            ),
        }
    }
}

/// The resolved `[logging]` configuration, `RUST_LOG` aside: that one is
/// read when the subscriber is installed (see [`init`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoggingConfig {
    /// `VELESDB_LOGGING_LEVEL`, else `[logging] level`; `None` when neither
    /// is set, which selects [`DEFAULT_FILTER`].
    pub level: Option<LogLevel>,
    /// `[logging] format`.
    pub format: LogFormat,
    /// `[logging] file`; `None` (the key absent or `""`) writes to stdout.
    pub file: Option<PathBuf>,
}

impl LoggingConfig {
    /// Resolves the section against `env_level`, the value of
    /// [`LEVEL_ENV`] (already filtered for emptiness by the caller).
    pub(crate) fn resolve(
        section: &LoggingSection,
        env_level: Option<&str>,
    ) -> anyhow::Result<Self> {
        let level = match (env_level, section.level.as_deref()) {
            (Some(env), _) => Some(LogLevel::parse(env, LEVEL_ENV)?),
            (None, Some(file)) => Some(LogLevel::parse(file, "logging.level")?),
            (None, None) => None,
        };
        let format = section
            .format
            .as_deref()
            .map_or(Ok(LogFormat::Text), LogFormat::parse)?;
        let file = section
            .file
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        Ok(Self {
            level,
            format,
            file,
        })
    }
}

/// Reads [`LEVEL_ENV`], treating an empty value as unset.
#[must_use]
pub fn level_from_env() -> Option<String> {
    non_empty_var(LEVEL_ENV)
}

fn non_empty_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The `tracing` filter directives the precedence chain in the module docs
/// selects: `rust_log` when set, else the configured level, else
/// [`DEFAULT_FILTER`].
#[must_use]
pub fn filter_directives(rust_log: Option<&str>, level: Option<LogLevel>) -> String {
    match (rust_log, level) {
        (Some(directives), _) => directives.to_string(),
        (None, Some(level)) => level.as_str().to_string(),
        (None, None) => DEFAULT_FILTER.to_string(),
    }
}

/// The formatting layer for `format`, writing to `writer`. ANSI colours are
/// only emitted when `ansi` is set: never into a file, never into JSON.
pub fn fmt_layer<S>(
    format: LogFormat,
    writer: BoxMakeWriter,
    ansi: bool,
) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    let layer = tracing_subscriber::fmt::layer().with_writer(writer);
    match format {
        LogFormat::Text => layer.with_ansi(ansi).boxed(),
        LogFormat::Json => layer.json().boxed(),
    }
}

/// Opens `path` for appending, creating it if needed. The file is shared
/// across threads as `Arc<File>`: `&File` is `Write`, and each event reaches
/// it in a single `write` on a descriptor opened with `O_APPEND`.
fn append_writer(path: &std::path::Path) -> anyhow::Result<BoxMakeWriter> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| anyhow::anyhow!("logging.file: cannot open {}: {e}", path.display()))?;
    Ok(BoxMakeWriter::new(Arc::new(file)))
}

/// Installs the global `tracing` subscriber for `config`, reading `RUST_LOG`
/// for the filter (see the module docs for the precedence). Returns whether
/// `RUST_LOG` overrode a configured level, so the caller can say so once the
/// subscriber is there to say it with.
///
/// # Errors
///
/// Returns an error when `[logging] file` cannot be opened for appending.
pub fn init(config: &LoggingConfig) -> anyhow::Result<bool> {
    let rust_log = non_empty_var("RUST_LOG");
    let filter =
        tracing_subscriber::EnvFilter::new(filter_directives(rust_log.as_deref(), config.level));
    let (writer, ansi) = match &config.file {
        Some(path) => (append_writer(path)?, false),
        None => (BoxMakeWriter::new(std::io::stdout), true),
    };
    let _ = tracing_subscriber::registry()
        .with(fmt_layer::<Registry>(config.format, writer, ansi))
        .with(filter)
        .try_init();
    Ok(rust_log.is_some() && config.level.is_some())
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
