//! Session configuration for VelesDB REPL.
//!
//! Manages session-level settings that can be modified with `\set` and viewed with `\show`.

use std::collections::HashMap;
use velesdb_core::SearchQuality;

/// Session settings for the REPL.
#[derive(Debug, Clone)]
pub struct SessionSettings {
    /// Current search mode.
    mode: SearchQuality,
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
            mode: SearchQuality::Balanced,
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

    /// Gets the current search mode.
    #[must_use]
    pub fn mode(&self) -> SearchQuality {
        self.mode
    }

    /// Gets the effective ef_search value.
    ///
    /// Uses a default `k=10` for the quality profile's ef calculation.
    #[must_use]
    #[allow(dead_code)] // Reason: public API for session-aware query execution (used in tests)
    pub fn effective_ef_search(&self) -> usize {
        self.ef_search.unwrap_or_else(|| self.mode.ef_search(10))
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
    #[allow(dead_code)] // Reason: public API for session-aware query execution (used in tests)
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
                self.mode = parse_mode(value)?;
                self.ef_search = None; // Reset ef_search when mode changes
                Ok(())
            }
            "ef_search" => {
                let ef = value
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid integer: {value}"))?;
                if !(16..=4096).contains(&ef) {
                    return Err(format!("ef_search must be between 16 and 4096, got {ef}"));
                }
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
                "mode" => self.mode = SearchQuality::Balanced,
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

    /// Returns all settings as displayable key-value pairs.
    #[must_use]
    pub fn all_settings(&self) -> Vec<(String, String)> {
        let mut settings = vec![
            ("mode".to_string(), format_quality(self.mode)),
            (
                "ef_search".to_string(),
                self.ef_search.map_or_else(
                    || format!("auto ({})", self.mode.ef_search(10)),
                    |v| v.to_string(),
                ),
            ),
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

    /// Gets a single setting value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key.to_lowercase().as_str() {
            "mode" => Some(format_quality(self.mode)),
            "ef_search" => Some(self.ef_search.map_or_else(
                || format!("auto ({})", self.mode.ef_search(10)),
                |v| v.to_string(),
            )),
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
fn format_quality(q: SearchQuality) -> String {
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

fn parse_mode(value: &str) -> Result<SearchQuality, String> {
    let lower = value.to_lowercase();
    match lower.as_str() {
        "fast" => Ok(SearchQuality::Fast),
        "balanced" => Ok(SearchQuality::Balanced),
        "accurate" => Ok(SearchQuality::Accurate),
        "perfect" => Ok(SearchQuality::Perfect),
        "autotune" => Ok(SearchQuality::AutoTune),
        _ => parse_parameterized_mode(&lower),
    }
}

/// Parses `custom:<ef>` and `adaptive:<min>:<max>` mode strings.
fn parse_parameterized_mode(value: &str) -> Result<SearchQuality, String> {
    if let Some(ef_str) = value.strip_prefix("custom:") {
        let ef = ef_str
            .parse::<usize>()
            .map_err(|_| format!("Invalid ef value in 'custom:{ef_str}'"))?;
        return Ok(SearchQuality::Custom(ef));
    }
    if let Some(rest) = value.strip_prefix("adaptive:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err("adaptive format: adaptive:<min_ef>:<max_ef>".to_string());
        }
        let min_ef = parts[0]
            .parse::<usize>()
            .map_err(|_| format!("Invalid min_ef in '{value}'"))?;
        let max_ef = parts[1]
            .parse::<usize>()
            .map_err(|_| format!("Invalid max_ef in '{value}'"))?;
        return Ok(SearchQuality::Adaptive { min_ef, max_ef });
    }
    Err(format!(
        "Invalid mode '{value}'. Valid: fast, balanced, accurate, \
         perfect, autotune, custom:<ef>, adaptive:<min>:<max>"
    ))
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
mod tests {
    use super::*;

    #[test]
    fn test_session_defaults() {
        let session = SessionSettings::new();
        assert_eq!(session.mode(), SearchQuality::Balanced);
        assert_eq!(session.effective_ef_search(), 160);
        assert_eq!(session.timeout_ms(), 30000);
        assert!(session.rerank());
        assert_eq!(session.max_results(), 100);
        assert!(session.active_collection().is_none());
    }

    #[test]
    fn test_set_mode() {
        let mut session = SessionSettings::new();
        session.set("mode", "fast").unwrap();
        assert_eq!(session.mode(), SearchQuality::Fast);
        assert_eq!(session.effective_ef_search(), 96);
    }

    #[test]
    fn test_set_ef_search() {
        let mut session = SessionSettings::new();
        session.set("ef_search", "512").unwrap();
        assert_eq!(session.effective_ef_search(), 512);
    }

    #[test]
    fn test_set_ef_search_invalid_range() {
        let mut session = SessionSettings::new();
        assert!(session.set("ef_search", "10").is_err());
        assert!(session.set("ef_search", "5000").is_err());
    }

    #[test]
    fn test_set_timeout() {
        let mut session = SessionSettings::new();
        session.set("timeout_ms", "5000").unwrap();
        assert_eq!(session.timeout_ms(), 5000);
    }

    #[test]
    fn test_set_rerank() {
        let mut session = SessionSettings::new();
        session.set("rerank", "off").unwrap();
        assert!(!session.rerank());
        session.set("rerank", "true").unwrap();
        assert!(session.rerank());
    }

    #[test]
    fn test_use_collection() {
        let mut session = SessionSettings::new();
        session.use_collection(Some("documents".to_string()));
        assert_eq!(session.active_collection(), Some("documents"));
    }

    #[test]
    fn test_reset_single() {
        let mut session = SessionSettings::new();
        session.set("mode", "fast").unwrap();
        session.reset(Some("mode"));
        assert_eq!(session.mode(), SearchQuality::Balanced);
    }

    #[test]
    fn test_reset_all() {
        let mut session = SessionSettings::new();
        session.set("mode", "fast").unwrap();
        session.set("ef_search", "512").unwrap();
        session.reset(None);
        assert_eq!(session.mode(), SearchQuality::Balanced);
        assert!(session.ef_search.is_none());
    }

    #[test]
    fn test_all_settings() {
        let session = SessionSettings::new();
        let settings = session.all_settings();
        assert!(settings.iter().any(|(k, _)| k == "mode"));
        assert!(settings.iter().any(|(k, _)| k == "ef_search"));
    }

    #[test]
    fn test_get_setting() {
        let session = SessionSettings::new();
        assert_eq!(session.get("mode"), Some("balanced".to_string()));
        assert!(session.get("unknown").is_none());
    }

    #[test]
    fn test_custom_setting() {
        let mut session = SessionSettings::new();
        session.set("custom_key", "custom_value").unwrap();
        assert_eq!(session.get("custom_key"), Some("custom_value".to_string()));
    }

    #[test]
    fn test_set_mode_autotune() {
        let mut session = SessionSettings::new();
        session.set("mode", "autotune").unwrap();
        assert_eq!(session.mode(), SearchQuality::AutoTune);
    }

    #[test]
    fn test_set_mode_custom() {
        let mut session = SessionSettings::new();
        session.set("mode", "custom:256").unwrap();
        assert_eq!(session.mode(), SearchQuality::Custom(256));
        assert_eq!(session.effective_ef_search(), 256);
    }

    #[test]
    fn test_set_mode_adaptive() {
        let mut session = SessionSettings::new();
        session.set("mode", "adaptive:32:512").unwrap();
        assert_eq!(
            session.mode(),
            SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 512
            }
        );
    }

    #[test]
    fn test_set_mode_invalid() {
        let mut session = SessionSettings::new();
        assert!(session.set("mode", "nonexistent").is_err());
        assert!(session.set("mode", "custom:abc").is_err());
        assert!(session.set("mode", "adaptive:32").is_err());
    }
}
