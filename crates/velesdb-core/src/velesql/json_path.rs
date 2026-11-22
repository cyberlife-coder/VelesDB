//! Nested field access in JSON payloads.
//!
//! [`payload_path`] and [`column_value`] are how every query clause reads a
//! dotted field path (`metadata.source`), in core and in the browser: object
//! keys only, no array indexing.
//!
//! [`JsonPath`] (EPIC-052 US-005) is a separate parser that also accepts
//! array indexing (`items[0].sku`). No query clause uses it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Error type for JSON path parsing.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum JsonPathError {
    /// Empty path provided.
    EmptyPath,
    /// Invalid array index (not a number).
    InvalidArrayIndex(String),
    /// Unclosed bracket.
    UnclosedBracket,
    /// Empty segment (double dot).
    EmptySegment,
}

impl std::fmt::Display for JsonPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "Empty JSON path"),
            Self::InvalidArrayIndex(s) => write!(f, "Invalid array index: '{s}'"),
            Self::UnclosedBracket => write!(f, "Unclosed bracket in JSON path"),
            Self::EmptySegment => write!(f, "Empty segment in JSON path (double dot)"),
        }
    }
}

impl std::error::Error for JsonPathError {}

/// A segment in a JSON path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PathSegment {
    /// Object property access: `.field`
    Property(String),
    /// Array index access: `[0]`
    Index(usize),
}

/// Parsed JSON path for nested field access.
///
/// # Examples
///
/// ```rust
/// use velesdb_core::velesql::json_path::JsonPath;
///
/// let path = JsonPath::parse("metadata.source").unwrap();
/// assert_eq!(path.segments.len(), 2);
///
/// let path = JsonPath::parse("items[0].sku").unwrap();
/// assert_eq!(path.segments.len(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JsonPath {
    /// The segments of the path.
    pub segments: Vec<PathSegment>,
}

impl JsonPath {
    /// Creates a new empty `JsonPath`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Creates a `JsonPath` from a single property name.
    #[must_use]
    pub fn from_property(name: &str) -> Self {
        Self {
            segments: vec![PathSegment::Property(name.to_string())],
        }
    }

    /// Parses a JSON path string like `"metadata.source"` or `"items[0].sku"`.
    ///
    /// # Errors
    ///
    /// Returns an error if the path is malformed.
    pub fn parse(input: &str) -> Result<Self, JsonPathError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(JsonPathError::EmptyPath);
        }

        let mut segments = Vec::new();
        let mut current = String::new();
        let mut chars = input.chars().peekable();
        let mut last_was_index = false;

        while let Some(c) = chars.next() {
            match c {
                '.' => Self::handle_dot(&mut segments, &mut current, &mut last_was_index)?,
                '[' => {
                    Self::flush_property(&mut segments, &mut current);
                    let index = Self::parse_bracket_index(&mut chars)?;
                    segments.push(PathSegment::Index(index));
                    last_was_index = true;
                }
                _ => {
                    current.push(c);
                    last_was_index = false;
                }
            }
        }

        if !current.is_empty() {
            segments.push(PathSegment::Property(current));
        }

        if segments.is_empty() {
            return Err(JsonPathError::EmptyPath);
        }

        Ok(JsonPath { segments })
    }

    /// Pushes the accumulated `current` buffer as a property segment and clears it.
    fn flush_property(segments: &mut Vec<PathSegment>, current: &mut String) {
        if !current.is_empty() {
            segments.push(PathSegment::Property(std::mem::take(current)));
        }
    }

    /// Handles a `.` separator: validates against empty segments, then flushes.
    fn handle_dot(
        segments: &mut Vec<PathSegment>,
        current: &mut String,
        last_was_index: &mut bool,
    ) -> Result<(), JsonPathError> {
        // After an index like [0], a dot is valid and just separates.
        if current.is_empty() && !*last_was_index && !segments.is_empty() {
            return Err(JsonPathError::EmptySegment);
        }
        Self::flush_property(segments, current);
        *last_was_index = false;
        Ok(())
    }

    /// Parses the contents of a `[...]` array index after the opening `[`.
    fn parse_bracket_index(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> Result<usize, JsonPathError> {
        let mut idx_str = String::new();
        let mut closed = false;
        for ch in chars.by_ref() {
            if ch == ']' {
                closed = true;
                break;
            }
            idx_str.push(ch);
        }
        if !closed {
            return Err(JsonPathError::UnclosedBracket);
        }
        idx_str
            .trim()
            .parse()
            .map_err(|_| JsonPathError::InvalidArrayIndex(idx_str))
    }

    /// Returns true if this is a simple (non-nested) path with a single property.
    #[must_use]
    pub fn is_simple(&self) -> bool {
        self.segments.len() == 1 && matches!(self.segments.first(), Some(PathSegment::Property(_)))
    }

    /// Returns the root property name, if the path starts with a property.
    #[must_use]
    pub fn root_property(&self) -> Option<&str> {
        match self.segments.first() {
            Some(PathSegment::Property(name)) => Some(name),
            _ => None,
        }
    }

    /// Returns a sub-path excluding the first segment.
    #[must_use]
    pub fn tail(&self) -> Self {
        Self {
            segments: self.segments.iter().skip(1).cloned().collect(),
        }
    }

    /// Extracts a value from a JSON document following this path.
    ///
    /// Returns `None` if any segment doesn't match.
    #[must_use]
    pub fn extract<'a>(&self, doc: &'a Value) -> Option<&'a Value> {
        let mut current = doc;

        for segment in &self.segments {
            current = match segment {
                PathSegment::Property(key) => current.get(key)?,
                PathSegment::Index(idx) => current.get(idx)?,
            };
        }

        Some(current)
    }

    /// Extracts a value and clones it, returning `Value::Null` if not found.
    #[must_use]
    pub fn extract_or_null(&self, doc: &Value) -> Value {
        self.extract(doc).cloned().unwrap_or(Value::Null)
    }
}

impl Default for JsonPath {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JsonPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for segment in &self.segments {
            match segment {
                PathSegment::Property(name) => {
                    if first {
                        write!(f, "{name}")?;
                    } else {
                        write!(f, ".{name}")?;
                    }
                }
                PathSegment::Index(idx) => {
                    write!(f, "[{idx}]")?;
                }
            }
            first = false;
        }
        Ok(())
    }
}

/// The payload value at dotted `path` (`meta.lang`): the one reading of a
/// field path that WHERE, ORDER BY, GROUP BY, DISTINCT, projection, window
/// functions, set operations, subqueries and MATCH share, in core and in the
/// browser. Each segment is an object key: a segment never indexes an array
/// nor reads through a scalar, so `tags.0` is absent. Unlike [`JsonPath`],
/// there is no `[i]` syntax.
#[must_use]
pub fn payload_path<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(payload, |current, key| match current {
            Value::Object(map) => map.get(key),
            _ => None,
        })
}

/// The value a row gives `column`: the point id for `id` (before any payload
/// `id`), else the payload's value at that path, `null` when absent.
#[must_use]
pub fn column_value(row: &crate::point::SearchResult, column: &str) -> Value {
    ProjectedField::Source(column).read(row).into_owned()
}

/// A source path or a value already published as a literal output key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ProjectedField<'a> {
    Source(&'a str),
    Published(&'a str),
}

impl<'a> ProjectedField<'a> {
    pub(super) fn name(self) -> &'a str {
        match self {
            Self::Source(name) | Self::Published(name) => name,
        }
    }

    pub(super) fn read(self, row: &crate::point::SearchResult) -> std::borrow::Cow<'_, Value> {
        let payload = row.point.payload.as_ref();
        let value = match self {
            Self::Source("id") => return std::borrow::Cow::Owned(Value::from(row.point.id)),
            Self::Source(path) => payload.and_then(|p| payload_path(p, path)),
            Self::Published(key) => payload.and_then(|p| p.get(key)),
        };
        std::borrow::Cow::Borrowed(value.unwrap_or(&Value::Null))
    }

    /// Align a later operand with the field the first operand's projection reads.
    pub(super) fn write(self, payload: &mut Value, value: Value) {
        match self {
            Self::Published(key) => payload[key] = value,
            Self::Source(path) => {
                let mut target = payload;
                for key in path.split('.') {
                    if !target.is_object() {
                        *target = Value::Object(serde_json::Map::new());
                    }
                    target = &mut target[key];
                }
                *target = value;
            }
        }
    }
}

// Tests moved to json_path_tests.rs per project rules
