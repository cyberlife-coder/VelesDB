//! WITH clause types for query-time configuration.
//!
//! This module defines WITH clause options for overriding
//! search parameters on a per-query basis.

use serde::{Deserialize, Serialize};

/// Quantization mode for vector search (EPIC-055 US-005).
///
/// Controls the precision/speed tradeoff for similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum QuantizationMode {
    /// Use full f32 precision (exact, slower).
    F32,
    /// Use int8 quantization only (fast, approximate).
    Int8,
    /// Use dual-precision: int8 for candidate selection, f32 for reranking.
    Dual,
    /// Let the system decide based on index configuration.
    #[default]
    Auto,
}

impl QuantizationMode {
    /// Parses a quantization mode from a string (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "f32" | "full" | "exact" => Some(Self::F32),
            "int8" | "sq8" | "quantized" => Some(Self::Int8),
            "dual" | "hybrid" => Some(Self::Dual),
            "auto" | "default" => Some(Self::Auto),
            _ => None,
        }
    }

    /// Returns the string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::Int8 => "int8",
            Self::Dual => "dual",
            Self::Auto => "auto",
        }
    }
}

/// WITH clause for query-time configuration overrides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WithClause {
    /// Configuration options as key-value pairs.
    pub options: Vec<WithOption>,
}

impl WithClause {
    /// Creates a new empty WITH clause.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an option to the WITH clause.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: WithValue) -> Self {
        self.options.push(WithOption {
            key: key.into(),
            value,
        });
        self
    }

    /// Gets an option value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&WithValue> {
        self.options
            .iter()
            .find(|opt| opt.key.eq_ignore_ascii_case(key))
            .map(|opt| &opt.value)
    }

    /// Gets the search mode if specified.
    ///
    /// Checks both `mode` and `quality` keys (VelesQL v3.5 Phase 4).
    /// `quality` is an alias for `mode`; if both are set, `mode` takes precedence.
    #[must_use]
    pub fn get_mode(&self) -> Option<&str> {
        self.mode_value().and_then(WithValue::as_str)
    }

    /// The raw value `mode` asks for, `quality` being its alias, or `None`
    /// when neither key is set. `mode` wins when both are set, and a key's
    /// first entry when it repeats. This is the one reading of whether a query
    /// names a mode: [`Self::get_mode`] and [`Self::search_quality`] read it.
    /// A caller deciding whether to inject a default mode must test it rather
    /// than [`Self::get_mode`], which is `None` for a value that is not a
    /// string (#2267).
    #[must_use]
    pub fn mode_value(&self) -> Option<&WithValue> {
        self.mode_values().next()
    }

    /// Every value the clause gives the mode, in the order
    /// [`Self::mode_value`] prefers them: each `mode` entry, then each
    /// `quality` one.
    fn mode_values(&self) -> impl Iterator<Item = &WithValue> {
        ["mode", "quality"].into_iter().flat_map(move |key| {
            self.options
                .iter()
                .filter(move |opt| opt.key.eq_ignore_ascii_case(key))
                .map(|opt| &opt.value)
        })
    }

    /// The search quality `mode` asks for, `quality` being its alias, or
    /// `None` when neither is set, as [`Self::mode_value`] reads them. Every
    /// value the clause gives the mode is checked, even one another value
    /// shadows: a value that is not a string, or a string that
    /// [`crate::api_types::parse_search_mode`] cannot read, is an error naming
    /// the accepted forms, never a silent fall-back to the default quality or
    /// to another value (#2267).
    ///
    /// # Errors
    ///
    /// Returns the message to report for the first value given for the mode
    /// that is not a string or names none of the accepted forms.
    #[cfg(feature = "persistence")]
    pub fn search_quality(&self) -> Result<Option<crate::SearchQuality>, String> {
        let qualities = self
            .mode_values()
            .map(Self::parse_mode_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(qualities.first().copied())
    }

    /// Reads one value given for the mode as a search quality.
    #[cfg(feature = "persistence")]
    fn parse_mode_value(value: &WithValue) -> Result<crate::SearchQuality, String> {
        let Some(mode) = value.as_str() else {
            return Err(format!(
                "Search mode must be a string. {}",
                crate::api_types::SEARCH_MODE_FORMS
            ));
        };
        crate::api_types::parse_search_mode(mode)
    }

    /// Gets ef_search if specified.
    ///
    /// Silently drops a value it cannot read as a plain non-negative integer
    /// — a typo, a string, or a negative literal (the grammar accepts a
    /// leading `-`) — rather than reporting it. A caller that must refuse
    /// such a value instead of treating it as "not given" uses
    /// [`Self::ef_search`]; one that only needs to know whether an inline
    /// value was given at all (to decide whether to inject a default over
    /// it) uses [`Self::ef_search_value`], which sees a value this method
    /// cannot read (#2274).
    #[must_use]
    pub fn get_ef_search(&self) -> Option<usize> {
        self.get("ef_search")
            .and_then(WithValue::as_integer)
            .and_then(|v| usize::try_from(v).ok())
    }

    /// The raw value `WITH (ef_search = ...)` gives, or `None` when the
    /// option is absent; a repeated key's first entry applies. See
    /// [`Self::ef_search`] for the validated reading.
    #[must_use]
    pub fn ef_search_value(&self) -> Option<&WithValue> {
        self.ef_search_values().next()
    }

    /// Every value the clause gives `ef_search`, in order.
    fn ef_search_values(&self) -> impl Iterator<Item = &WithValue> {
        self.options
            .iter()
            .filter(|opt| opt.key.eq_ignore_ascii_case("ef_search"))
            .map(|opt| &opt.value)
    }

    /// The `ef_search` `WITH (ef_search = ...)` asks for, checked against the
    /// documented range (`docs/VELESQL_SPEC.md`), or `None` when the option
    /// is absent. Unlike [`Self::get_ef_search`], a value that is not an
    /// integer, or one outside `[16, 4096]` — `-1`, say, which
    /// [`Self::get_ef_search`] drops as though none were given — is an error
    /// here, never a silent fall-back to no override (#2274). Every value a
    /// repeated key gives is checked, even one the first shadows; the first
    /// applies, as [`Self::ef_search_value`] reads it.
    ///
    /// # Errors
    ///
    /// Returns a message naming the accepted type or range for the first value
    /// that is not an integer, or an integer outside the documented range.
    pub fn ef_search(&self) -> Result<Option<usize>, String> {
        let values = self
            .ef_search_values()
            .map(Self::parse_ef_search_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values.first().copied())
    }

    /// Reads one value given for `ef_search` against the documented range. A
    /// value that is not an integer gets the message an integer outside the
    /// range gets, naming the value in canonical `VelesQL` form (see
    /// [`WithValue`]'s `Display`).
    fn parse_ef_search_value(value: &WithValue) -> Result<usize, String> {
        let Some(raw) = value.as_integer() else {
            return Err(crate::api_types::ef_search_out_of_range(value));
        };
        crate::api_types::parse_with_ef_search(raw)
    }

    /// Gets timeout in milliseconds if specified.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn get_timeout_ms(&self) -> Option<u64> {
        self.get("timeout_ms")
            .and_then(WithValue::as_integer)
            .map(|v| v as u64)
    }

    /// Gets rerank option if specified.
    #[must_use]
    pub fn get_rerank(&self) -> Option<bool> {
        self.get("rerank").and_then(WithValue::as_bool)
    }

    /// Gets quantization mode if specified (EPIC-055 US-005).
    ///
    /// Supported values: 'f32', 'int8', 'dual', 'auto'.
    #[must_use]
    pub fn get_quantization(&self) -> Option<QuantizationMode> {
        self.get("quantization")
            .and_then(WithValue::as_str)
            .and_then(QuantizationMode::parse)
    }

    /// Gets oversampling ratio if specified (EPIC-055 US-005).
    ///
    /// Used with dual-precision mode to control candidate pool size.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn get_oversampling(&self) -> Option<usize> {
        self.get("oversampling")
            .and_then(WithValue::as_integer)
            .map(|v| v.max(1) as usize)
    }
}

/// A single option in a WITH clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithOption {
    /// Option key.
    pub key: String,
    /// Option value.
    pub value: WithValue,
}

/// Value type for WITH clause options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WithValue {
    /// String value.
    String(String),
    /// Integer value.
    Integer(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Boolean(bool),
    /// Identifier (unquoted string).
    Identifier(String),
}

/// Renders the value in canonical `VelesQL` form, which the parser reads back
/// as this same value in a `WITH` clause — not necessarily as a query wrote
/// it: `1.50` renders as `1.5` and `TRUE` as `true`.
///
/// - A string in single quotes, each quote inside doubled.
/// - An integer in decimal.
/// - A float in decimal with a fractional part and never an exponent, which
///   the grammar has no form for: `1e20` renders as
///   `100000000000000000000.0`. An infinite float renders as a literal too
///   large for an `f64`, which the parser reads back as that infinity. `NaN`
///   has no `VelesQL` form and renders as `NaN`; the parser never produces
///   one.
/// - `true` or `false`.
/// - An identifier bare when the parser reads it back bare as that
///   identifier, and otherwise in double quotes, each double quote inside
///   doubled: `TRUE`, `true_x` or `my option` would not read back bare.
impl std::fmt::Display for WithValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => write!(f, "'{}'", s.replace('\'', "''")),
            Self::Integer(v) => write!(f, "{v}"),
            Self::Float(v) => write_float_literal(f, *v),
            Self::Boolean(v) => write!(f, "{v}"),
            Self::Identifier(s) if crate::velesql::parser::reads_back_as_bare_identifier(s) => {
                f.write_str(s)
            }
            Self::Identifier(s) => write!(f, "\"{}\"", s.replace('"', "\"\"")),
        }
    }
}

/// Writes `v` as a `VelesQL` float literal (`-`? digits `.` digits), see
/// [`WithValue`]'s `Display`. `f64`'s own `Display` never uses an exponent
/// and writes the shortest decimal that reads back as `v`, but drops the
/// fractional part of a whole number (`100` for `100.0`), which the grammar
/// would read as an integer.
fn write_float_literal(f: &mut std::fmt::Formatter<'_>, v: f64) -> std::fmt::Result {
    if v.is_nan() {
        return write!(f, "{v}");
    }
    if v.is_infinite() {
        // Ten times `f64::MAX`, which no `f64` holds: it reads back as infinity.
        let sign = if v.is_sign_negative() { "-" } else { "" };
        return write!(f, "{sign}{}0.0", f64::MAX);
    }
    let digits = v.to_string();
    if digits.contains('.') {
        f.write_str(&digits)
    } else {
        write!(f, "{digits}.0")
    }
}

impl WithValue {
    /// Returns the value as a string if applicable.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Identifier(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as an integer.
    #[must_use]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns the value as a float.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            #[allow(clippy::cast_precision_loss)]
            Self::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Returns the value as a boolean.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}
