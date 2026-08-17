//! The optional TOML configuration file: one place to set every knob.
//!
//! Until now the daemon was configured exclusively through eighteen
//! `VELESDB_MEMORY_*` environment variables. That is workable for a one-shot
//! shell invocation and painful for a long-lived daemon: a launchd plist or a
//! systemd unit is the wrong place to keep a model name, and nothing there can
//! carry a comment explaining *why* a value is what it is.
//!
//! This module adds a file without taking anything away. It resolves the
//! config, then exports each setting into the process environment **only when
//! that variable is not already set**. Every existing reader keeps reading the
//! environment exactly as before, and the precedence falls out of that one
//! rule:
//!
//! ```text
//! command line  >  environment  >  config file  >  built-in default
//! ```
//!
//! So an operator can pin a model in the file and still override it for a
//! single run with `VELESDB_MEMORY_OLLAMA_MODEL=… velesdb-memory`, which is
//! the behaviour anyone who has used a dotfile-driven tool expects.
//!
//! The file is entirely optional: no file, or a file with only some keys, is
//! not an error. A file that exists but cannot be parsed **is** an error —
//! silently ignoring a malformed config is how a daemon ends up quietly
//! running on defaults the operator believes they overrode.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Environment variable naming the config file explicitly.
pub const CONFIG_PATH_VAR: &str = "VELESDB_MEMORY_CONFIG";

/// File name looked up in the default locations.
pub const CONFIG_FILE_NAME: &str = "velesdb-memory.toml";

/// Why a config file could not be used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // error enum, grows by nature; matching externally requires a wildcard arm
pub enum ConfigError {
    /// The file could not be read.
    #[error("config file {path} could not be read: {source}")]
    Read {
        /// The path that failed.
        path: PathBuf,
        /// The underlying I/O failure.
        source: std::io::Error,
    },
    /// The file is not valid TOML, or does not match the expected shape.
    #[error("config file {path} is not valid: {message}")]
    Parse {
        /// The path that failed.
        path: PathBuf,
        /// The parser's complaint.
        message: String,
    },
    /// A path list could not be joined into the platform's list syntax.
    #[error("config file {path}: {field} contains a path with the list separator in it")]
    PathList {
        /// The path that failed.
        path: PathBuf,
        /// The offending field.
        field: &'static str,
    },
}

/// Top-level shape of `velesdb-memory.toml`.
///
/// `deny_unknown_fields` is deliberate: a typo'd key (`mdoel = "…"`) that is
/// silently dropped leaves the operator convinced they set something they did
/// not. Failing loudly at startup is the whole reason to have a file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Store directory (`VELESDB_MEMORY_PATH`).
    pub path: Option<String>,
    /// Suppress the startup banner (`VELESDB_MEMORY_QUIET`).
    pub quiet: Option<bool>,
    /// Default TTL in seconds applied to facts with no explicit one
    /// (`VELESDB_MEMORY_DEFAULT_TTL`).
    pub default_ttl: Option<u64>,
    /// HTTP transport settings.
    #[serde(default)]
    pub http: HttpConfig,
    /// Embedding backend settings.
    #[serde(default)]
    pub embedder: EmbedderConfig,
    /// Extraction backend settings.
    #[serde(default)]
    pub extractor: ExtractorConfig,
    /// Context-compiler settings.
    #[serde(default)]
    pub context: ContextConfig,
    /// Knowledge-graph settings.
    #[serde(default)]
    pub graph: GraphConfig,
}

/// `[graph]` — how much structure the memory builds on its own.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphConfig {
    /// Let every `remember` also wire the entities, typed edges and attributes
    /// its text states (`VELESDB_MEMORY_AUTOGRAPH`).
    ///
    /// Off by default. It costs one generation per `remember`, so it is a
    /// deliberate choice, not something to inherit silently — and it needs an
    /// `[extractor]` backend to have anything to do.
    pub autograph: Option<bool>,
}

/// `[http]` — the streamable-HTTP transport (multi-client daemon mode).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    /// Serve over HTTP instead of stdio (`VELESDB_MEMORY_HTTP`).
    pub enabled: Option<bool>,
    /// Address to bind (`VELESDB_MEMORY_HTTP_BIND`).
    pub bind: Option<String>,
    /// Serve plaintext instead of TLS (`VELESDB_MEMORY_HTTP_INSECURE`).
    pub insecure: Option<bool>,
    /// Permit a non-loopback bind (`VELESDB_MEMORY_HTTP_ALLOW_REMOTE`).
    pub allow_remote: Option<bool>,
    /// Request body ceiling (`VELESDB_MEMORY_HTTP_MAX_BODY_BYTES`).
    pub max_body_bytes: Option<u64>,
    /// Concurrent session ceiling (`VELESDB_MEMORY_HTTP_MAX_SESSIONS`).
    pub max_sessions: Option<u64>,
    /// Directory holding the local CA and leaf certificate
    /// (`VELESDB_MEMORY_TLS_DIR`).
    pub tls_dir: Option<String>,
}

/// `[embedder]` — how text becomes vectors.
///
/// **No `api_token` field, deliberately** — see [`TOKEN_HINT`]. The two roles
/// carry the same four settings under the same names on purpose: an operator
/// who has configured one has configured the other.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedderConfig {
    /// `hash`, `ollama` or `openai` (`VELESDB_MEMORY_EMBEDDER`).
    pub backend: Option<String>,
    /// Embedding model (`VELESDB_MEMORY_EMBEDDER_MODEL`).
    pub model: Option<String>,
    /// Base URL, origin and port, no path (`VELESDB_MEMORY_EMBEDDER_URL`).
    pub url: Option<String>,
    /// How long Ollama keeps the model resident
    /// (`VELESDB_MEMORY_OLLAMA_KEEP_ALIVE`).
    ///
    /// The one setting here that keeps a product name, and legitimately: it is
    /// a field of Ollama's own wire protocol, not a role-level knob an
    /// OpenAI-compatible server would know what to do with.
    pub keep_alive: Option<String>,
}

/// `[extractor]` — the backend that reads facts, relations and attributes out
/// of raw text for `remember_extracted`.
///
/// **No `api_token` field, deliberately** — see [`TOKEN_HINT`].
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractorConfig {
    /// `outline`, `ollama`, `openai`, or absent for none
    /// (`VELESDB_MEMORY_EXTRACTOR`).
    pub backend: Option<String>,
    /// Generative model (`VELESDB_MEMORY_EXTRACTOR_MODEL`).
    pub model: Option<String>,
    /// Base URL, origin and port, no path (`VELESDB_MEMORY_EXTRACTOR_URL`).
    pub url: Option<String>,
}

/// `[context]` — the deterministic context compiler.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    /// Directories `path`-referenced fragments may be read from
    /// (`VELESDB_MEMORY_INGEST_ROOTS`). Written as a list here and joined
    /// into the platform's `PATH` syntax, so the file stays readable and
    /// portable where the raw variable is neither.
    pub ingest_roots: Option<Vec<String>>,
}

/// Where the config file was found, and what it asked for.
#[derive(Debug)]
pub struct LoadedConfig {
    /// The file that was read.
    pub path: PathBuf,
    /// The variables it defines, in `VELESDB_MEMORY_*` form.
    pub values: BTreeMap<String, String>,
}

/// Resolve the config file path: an explicit `--config`, then
/// [`CONFIG_PATH_VAR`], then `<store>/velesdb-memory.toml`, then
/// `./velesdb-memory.toml`.
///
/// The store directory is checked before the working directory on purpose: a
/// daemon's working directory is whatever launchd or systemd happened to give
/// it, which is not a location an operator would think to put a file in.
#[must_use]
pub fn resolve_path(explicit: Option<&str>, store_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return Some(PathBuf::from(explicit));
    }
    if let Ok(from_env) = std::env::var(CONFIG_PATH_VAR) {
        if !from_env.trim().is_empty() {
            return Some(PathBuf::from(from_env));
        }
    }
    if let Some(dir) = store_dir {
        let candidate = dir.join(CONFIG_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let cwd = PathBuf::from(CONFIG_FILE_NAME);
    cwd.is_file().then_some(cwd)
}

/// Read and parse `path` into the `VELESDB_MEMORY_*` variables it defines.
///
/// # Errors
/// Returns [`ConfigError`] if the file cannot be read or is not valid TOML.
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file: ConfigFile = toml::from_str(&text).map_err(|err| ConfigError::Parse {
        path: path.to_path_buf(),
        message: describe_parse_failure(&err.to_string()),
    })?;
    Ok(LoadedConfig {
        path: path.to_path_buf(),
        values: file.into_env(path)?,
    })
}

/// Where an API token belongs, quoted verbatim by the refusal that rejects one
/// found in the file.
///
/// `deny_unknown_fields` already refuses an `api_token` key, since no section
/// declares one. What it cannot do is say where the operator should put the
/// token they legitimately have — so the refusal is rewritten to carry this.
pub const TOKEN_HINT: &str = "an API token is read from the environment only, never from a \
     file: set VELESDB_MEMORY_EMBEDDER_API_TOKEN or \
     VELESDB_MEMORY_EXTRACTOR_API_TOKEN instead. A credential in a TOML is one \
     `git add .` away from a public history, and no pre-commit secret scan can \
     police a file it has never seen.";

/// Render a TOML parse failure, redacting it when it concerns a credential.
///
/// `toml`'s own error quotes the offending source line back — which is exactly
/// the right thing for `mdoel = "bge-m3"` and exactly the wrong thing for
/// `api_token = "sk-…"`: the daemon would print the secret to stderr, where a
/// launch agent's log file keeps it. So a failure naming `api_token` loses the
/// snippet and gains [`TOKEN_HINT`]; every other failure is untouched.
fn describe_parse_failure(rendered: &str) -> String {
    if rendered.contains("api_token") {
        return format!("unknown field `api_token` — {TOKEN_HINT}");
    }
    rendered.to_owned()
}

/// One setting reachable under two environment-variable names: the canonical,
/// role-named one and a legacy alias kept working for compatibility.
///
/// See [`resolve_alias`] for the precedence rule.
pub struct AliasResolution {
    /// The value to use, or `None` when neither name is set.
    pub value: Option<String>,
    /// Both names are set to **different** values. The caller is expected to
    /// gather these and emit a single [`alias_conflict_notice`].
    pub conflicting: bool,
}

/// Resolve a setting the caller can name two ways: canonical wins, the legacy
/// alias is the fallback (C1).
///
/// The embedding role's URL and model were named after a *product*
/// (`VELESDB_MEMORY_OLLAMA_URL`) while the extraction role's were named after
/// the *role* (`VELESDB_MEMORY_EXTRACTOR_URL`). Once a non-Ollama backend can
/// serve either role, a variable that says `OLLAMA` while pointing at oMLX is
/// a lie the operator has to hold in their head. This closes the asymmetry
/// without breaking a single existing setup: the alias keeps working, and only
/// a genuine disagreement between the two is worth a word.
///
/// Canonical wins **whatever the source** — including a role-named value that
/// came from the config file against a legacy one exported by the shell, which
/// is the one case where this rule and [`apply`]'s "environment outranks the
/// file" point different ways. That case is not silent: it is precisely what
/// [`alias_conflict_notice`] reports.
#[must_use]
pub fn resolve_alias(canonical: Option<&str>, legacy: Option<&str>) -> AliasResolution {
    AliasResolution {
        conflicting: matches!((canonical, legacy), (Some(role), Some(old)) if role != old),
        value: canonical.or(legacy).map(str::to_owned),
    }
}

/// One line naming every variable whose legacy alias disagrees with it, or
/// `None` when nothing disagrees.
///
/// **One notice, however many settings conflict.** A warning per variable is
/// how a startup log becomes noise an operator learns to scroll past, and the
/// operator's next action is the same for all of them. This is also
/// deliberately not a deprecation warning: the aliases are supported, and
/// shouting at someone whose setup works is how a message gets filtered out
/// before the day it finally matters.
#[must_use]
pub fn alias_conflict_notice(conflicts: &[(&str, &str)]) -> Option<String> {
    if conflicts.is_empty() {
        return None;
    }
    let pairs = conflicts
        .iter()
        .map(|(canonical, legacy)| format!("{canonical} over {legacy}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "[velesdb-memory] set under two names with different values — using {pairs}. \
         The role-named variable wins; unset the other to silence this."
    ))
}

/// Export `values` into the process environment, skipping any variable that is
/// already set. Returns the names actually applied, in order.
///
/// The skip is the precedence rule: the environment was set by whoever
/// launched the process, and that intent outranks a file on disk.
#[must_use]
pub fn apply(values: &BTreeMap<String, String>) -> Vec<String> {
    let mut applied = Vec::new();
    for (key, value) in values {
        if std::env::var_os(key).is_some() {
            continue;
        }
        std::env::set_var(key, value);
        applied.push(key.clone());
    }
    applied
}

impl ConfigFile {
    /// Flatten the typed sections into the `VELESDB_MEMORY_*` variables the
    /// rest of the binary already reads.
    fn into_env(self, path: &Path) -> Result<BTreeMap<String, String>, ConfigError> {
        let mut out = BTreeMap::new();
        let mut set = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                out.insert(key.to_string(), value);
            }
        };
        set("VELESDB_MEMORY_PATH", self.path);
        set("VELESDB_MEMORY_QUIET", self.quiet.map(flag));
        set(
            "VELESDB_MEMORY_DEFAULT_TTL",
            self.default_ttl.map(|v| v.to_string()),
        );

        set("VELESDB_MEMORY_HTTP", self.http.enabled.map(flag));
        set("VELESDB_MEMORY_HTTP_BIND", self.http.bind);
        set("VELESDB_MEMORY_HTTP_INSECURE", self.http.insecure.map(flag));
        set(
            "VELESDB_MEMORY_HTTP_ALLOW_REMOTE",
            self.http.allow_remote.map(flag),
        );
        set(
            "VELESDB_MEMORY_HTTP_MAX_BODY_BYTES",
            self.http.max_body_bytes.map(|v| v.to_string()),
        );
        set(
            "VELESDB_MEMORY_HTTP_MAX_SESSIONS",
            self.http.max_sessions.map(|v| v.to_string()),
        );
        set("VELESDB_MEMORY_TLS_DIR", self.http.tls_dir);

        set("VELESDB_MEMORY_EMBEDDER", self.embedder.backend);
        // The role-named variables, not the `VELESDB_MEMORY_OLLAMA_*` aliases:
        // the section is named after the role, so what it writes should be too.
        // The aliases stay readable from the environment (see
        // [`resolve_alias`]) for setups that already export them.
        set("VELESDB_MEMORY_EMBEDDER_MODEL", self.embedder.model);
        set("VELESDB_MEMORY_EMBEDDER_URL", self.embedder.url);
        set("VELESDB_MEMORY_OLLAMA_KEEP_ALIVE", self.embedder.keep_alive);

        set("VELESDB_MEMORY_EXTRACTOR", self.extractor.backend);
        set("VELESDB_MEMORY_EXTRACTOR_MODEL", self.extractor.model);
        set("VELESDB_MEMORY_EXTRACTOR_URL", self.extractor.url);

        set("VELESDB_MEMORY_AUTOGRAPH", self.graph.autograph.map(flag));

        if let Some(roots) = self.context.ingest_roots {
            let joined = std::env::join_paths(roots).map_err(|_| ConfigError::PathList {
                path: path.to_path_buf(),
                field: "context.ingest_roots",
            })?;
            out.insert(
                "VELESDB_MEMORY_INGEST_ROOTS".to_string(),
                joined.to_string_lossy().into_owned(),
            );
        }
        Ok(out)
    }
}

/// Render a boolean the way every reader in the binary tests for it: the
/// truthy form is the exact string `"1"`. `false` becomes `"0"` rather than
/// being omitted, so writing `enabled = false` in the file genuinely holds the
/// setting off instead of falling through to a default that might be on.
fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
