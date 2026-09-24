//! Session configuration for VelesDB REPL.
//!
//! Manages session-level settings that can be modified with `\set` and viewed with `\show`.

use std::collections::HashMap;
use velesdb_core::SearchQuality;

/// Session settings for the REPL.
#[derive(Debug, Clone)]
pub struct SessionSettings {
    /// The search mode `\set` this session, if any. `None` leaves every
    /// search at the configured default (`[search]` in `velesdb.toml`), which
    /// a session that never ran `\set mode` must not override (#2303).
    mode: Option<SearchQuality>,
    /// Override ef_search (None = use mode default).
    ef_search: Option<usize>,
    /// Query timeout in milliseconds.
    timeout_ms: u64,
    /// Enable reranking after quantized search.
    rerank: bool,
    /// Maximum results per query.
    max_results: usize,
    /// Active collection (for \use command).
    active_collection: Option<String>,
    /// Custom settings.
    custom: HashMap<String, String>,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            mode: None,
            ef_search: None,
            timeout_ms: 30000,
            rerank: true,
            max_results: 100,
            active_collection: None,
            custom: HashMap::new(),
        }
    }
}

impl SessionSettings {
    /// Creates new session settings with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The session's `mode` as the `WITH(mode=...)` string the core parser
    /// understands (`fast`/`balanced`/`accurate`/`perfect`/`autotune`/
    /// `custom:<ef>`/`adaptive:<min>:<max>`), or `None` when it was never
    /// `\set`.
    #[must_use]
    pub fn mode_str(&self) -> Option<String> {
        self.mode.map(format_quality)
    }

    /// Gets the explicitly-set ef_search override, if any.
    ///
    /// The REPL injects one quality setting into a query that names none:
    /// this `ef_search` when set, else the `mode` when set, else nothing.
    #[must_use]
    pub fn ef_search(&self) -> Option<usize> {
        self.ef_search
    }

    /// The quality this session sets for a search, or `None` when it sets
    /// none and the configured default applies: its `ef_search` when set (as
    /// `SearchQuality::Custom`, which is what core's `search_with_ef` runs),
    /// else its `mode` when set. The rule the query path applies when it
    /// injects the session into a `WITH` clause, for the commands that search
    /// directly (`.bench`).
    #[must_use]
    pub fn search_quality(&self) -> Option<SearchQuality> {
        self.ef_search.map(SearchQuality::Custom).or(self.mode)
    }

    /// Gets the query timeout in milliseconds.
    #[must_use]
    #[allow(dead_code)] // Reason: public API for session-aware query execution (used in tests)
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Gets the rerank setting.
    #[must_use]
    #[allow(dead_code)] // Reason: public API for session-aware query execution (used in tests)
    pub fn rerank(&self) -> bool {
        self.rerank
    }

    /// Gets max results.
    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results
    }

    /// Gets the active collection.
    #[must_use]
    pub fn active_collection(&self) -> Option<&str> {
        self.active_collection.as_deref()
    }

    /// Sets a session parameter.
    ///
    /// # Returns
    ///
    /// Ok(()) if the parameter was set, Err(message) if invalid.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key.to_lowercase().as_str() {
            "mode" => {
                self.mode = Some(parse_mode(value)?);
                self.ef_search = None; // Reset ef_search when mode changes
                Ok(())
            }
            "ef_search" => {
                let ef = value
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid integer: {value}"))?;
                // Same range the config file and every query-time `WITH
                // (ef_search = ...)` enforce (#2274) — one definition,
                // `velesdb_core::api_types::validate_ef_search`.
                velesdb_core::api_types::validate_ef_search(ef)?;
                self.ef_search = Some(ef);
                Ok(())
            }
            "timeout_ms" | "timeout" => {
                let ms = value
                    .parse::<u64>()
                    .map_err(|_| format!("Invalid integer: {value}"))?;
                if ms < 100 {
                    return Err("timeout_ms must be at least 100".to_string());
                }
                self.timeout_ms = ms;
                Ok(())
            }
            "rerank" => {
                self.rerank = parse_bool(value)?;
                Ok(())
            }
            "max_results" => {
                let max = value
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid integer: {value}"))?;
                if max == 0 || max > 10000 {
                    return Err("max_results must be between 1 and 10000".to_string());
                }
                self.max_results = max;
                Ok(())
            }
            _ => {
                self.custom.insert(key.to_string(), value.to_string());
                Ok(())
            }
        }
    }

    /// Sets the active collection.
    pub fn use_collection(&mut self, name: Option<String>) {
        self.active_collection = name;
    }

    /// Resets a specific setting or all settings.
    pub fn reset(&mut self, key: Option<&str>) {
        match key {
            None => {
                *self = Self::default();
            }
            Some(k) => match k.to_lowercase().as_str() {
                "mode" => self.mode = None,
                "ef_search" => self.ef_search = None,
                "timeout_ms" | "timeout" => self.timeout_ms = 30000,
                "rerank" => self.rerank = true,
                "max_results" => self.max_results = 100,
                "collection" => self.active_collection = None,
                _ => {
                    self.custom.remove(k);
                }
            },
        }
    }

    /// Returns all settings as displayable key-value pairs. `configured` is
    /// the database's default search quality, shown for a `mode` this
    /// session never set.
    #[must_use]
    pub fn all_settings(&self, configured: SearchQuality) -> Vec<(String, String)> {
        let mut settings = vec![
            ("mode".to_string(), self.shown_mode(configured)),
            ("ef_search".to_string(), self.shown_ef_search(configured)),
            ("timeout_ms".to_string(), self.timeout_ms.to_string()),
            ("rerank".to_string(), self.rerank.to_string()),
            ("max_results".to_string(), self.max_results.to_string()),
            (
                "collection".to_string(),
                self.active_collection
                    .clone()
                    .unwrap_or_else(|| "(none)".to_string()),
            ),
        ];

        for (k, v) in &self.custom {
            settings.push((k.clone(), v.clone()));
        }

        settings
    }

    /// Gets a single setting value, `configured` as in [`Self::all_settings`].
    #[must_use]
    pub fn get(&self, key: &str, configured: SearchQuality) -> Option<String> {
        match key.to_lowercase().as_str() {
            "mode" => Some(self.shown_mode(configured)),
            "ef_search" => Some(self.shown_ef_search(configured)),
            "timeout_ms" | "timeout" => Some(self.timeout_ms.to_string()),
            "rerank" => Some(self.rerank.to_string()),
            "max_results" => Some(self.max_results.to_string()),
            "collection" => Some(
                self.active_collection
                    .clone()
                    .unwrap_or_else(|| "(none)".to_string()),
            ),
            _ => self.custom.get(key).cloned(),
        }
    }
}

/// Formats a `SearchQuality` for display in session settings.
pub(crate) fn format_quality(q: SearchQuality) -> String {
    match q {
        SearchQuality::Fast => "fast".to_string(),
        SearchQuality::Balanced => "balanced".to_string(),
        SearchQuality::Accurate => "accurate".to_string(),
        SearchQuality::Perfect => "perfect".to_string(),
        SearchQuality::AutoTune => "autotune".to_string(),
        SearchQuality::Custom(ef) => format!("custom:{ef}"),
        SearchQuality::Adaptive { min_ef, max_ef } => {
            format!("adaptive:{min_ef}:{max_ef}")
        }
        _ => format!("{q:?}").to_lowercase(),
    }
}

impl SessionSettings {
    /// The `mode` as `\show` prints it: the session's, or the configured
    /// default it leaves in force, marked as such.
    fn shown_mode(&self, configured: SearchQuality) -> String {
        self.mode_str()
            .unwrap_or_else(|| format!("{} (configured default)", format_quality(configured)))
    }

    /// The `ef_search` as `\show` prints it: the session's override, or the
    /// value the mode in force resolves to at `k = 10`.
    fn shown_ef_search(&self, configured: SearchQuality) -> String {
        self.ef_search.map_or_else(
            || format!("auto ({})", self.mode.unwrap_or(configured).ef_search(10)),
            |v| v.to_string(),
        )
    }
}

/// Parses a `\set mode` value with the parser the server and the query
/// engine use, so the REPL refuses at `\set` time what every later query
/// would refuse: a typo, or `adaptive` with `min_ef` above `max_ef` (#2267).
fn parse_mode(value: &str) -> Result<SearchQuality, String> {
    velesdb_core::api_types::parse_search_mode(value)
}

/// Returns `true` for settings that `\set` stores and displays but that have no
/// channel into `Database::execute_query` today, so the REPL warns they are
/// display-only rather than silently claiming they affect queries.
///
/// `mode`, `ef_search` and `max_results` ARE wired (injected into the query AST
/// / applied as a LIMIT cap); `timeout_ms` and `rerank` are not.
#[must_use]
pub fn is_unwired_setting(key: &str) -> bool {
    matches!(
        key.to_lowercase().as_str(),
        "timeout_ms" | "timeout" | "rerank"
    )
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_lowercase().as_str() {
        "true" | "on" | "1" | "yes" => Ok(true),
        "false" | "off" | "0" | "no" => Ok(false),
        _ => Err(format!(
            "Invalid boolean '{}'. Use true/false, on/off, 1/0",
            value
        )),
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
