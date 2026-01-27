//! JSON Path parser for nested field access (EPIC-052 US-005).
//!
//! Supports dot notation (`metadata.source`) and array indexing (`items[0].sku`)
//! for GROUP BY on nested JSON fields.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Error type for JSON path parsing.
#[derive(Debug, Clone, PartialEq)]
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
                '.' => {
                    // After an index like [0], a dot is valid and just separates
                    if current.is_empty() && !last_was_index && !segments.is_empty() {
                        return Err(JsonPathError::EmptySegment);
                    }
                    if !current.is_empty() {
                        segments.push(PathSegment::Property(current.clone()));
                        current.clear();
                    }
                    last_was_index = false;
                }
                '[' => {
                    if !current.is_empty() {
                        segments.push(PathSegment::Property(current.clone()));
                        current.clear();
                    }
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
                    let index: usize = idx_str
                        .trim()
                        .parse()
                        .map_err(|_| JsonPathError::InvalidArrayIndex(idx_str))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_simple() {
        let path = JsonPath::parse("name").unwrap();
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.segments[0], PathSegment::Property("name".to_string()));
        assert!(path.is_simple());
    }

    #[test]
    fn test_parse_nested() {
        let path = JsonPath::parse("metadata.source").unwrap();
        assert_eq!(path.segments.len(), 2);
        assert_eq!(
            path.segments[0],
            PathSegment::Property("metadata".to_string())
        );
        assert_eq!(
            path.segments[1],
            PathSegment::Property("source".to_string())
        );
        assert!(!path.is_simple());
    }

    #[test]
    fn test_parse_deep_nested() {
        let path = JsonPath::parse("a.b.c.d.e").unwrap();
        assert_eq!(path.segments.len(), 5);
    }

    #[test]
    fn test_parse_array_index() {
        let path = JsonPath::parse("items[0]").unwrap();
        assert_eq!(path.segments.len(), 2);
        assert_eq!(path.segments[0], PathSegment::Property("items".to_string()));
        assert_eq!(path.segments[1], PathSegment::Index(0));
    }

    #[test]
    fn test_parse_array_with_property() {
        let path = JsonPath::parse("items[0].sku").unwrap();
        assert_eq!(path.segments.len(), 3);
        assert_eq!(path.segments[0], PathSegment::Property("items".to_string()));
        assert_eq!(path.segments[1], PathSegment::Index(0));
        assert_eq!(path.segments[2], PathSegment::Property("sku".to_string()));
    }

    #[test]
    fn test_parse_empty_error() {
        assert!(matches!(JsonPath::parse(""), Err(JsonPathError::EmptyPath)));
        assert!(matches!(
            JsonPath::parse("   "),
            Err(JsonPathError::EmptyPath)
        ));
    }

    #[test]
    fn test_parse_double_dot_error() {
        assert!(matches!(
            JsonPath::parse("a..b"),
            Err(JsonPathError::EmptySegment)
        ));
    }

    #[test]
    fn test_parse_unclosed_bracket_error() {
        assert!(matches!(
            JsonPath::parse("items[0"),
            Err(JsonPathError::UnclosedBracket)
        ));
    }

    #[test]
    fn test_parse_invalid_index_error() {
        assert!(matches!(
            JsonPath::parse("items[abc]"),
            Err(JsonPathError::InvalidArrayIndex(_))
        ));
    }

    #[test]
    fn test_extract_simple() {
        let doc = json!({"name": "Alice", "age": 30});
        let path = JsonPath::parse("name").unwrap();
        assert_eq!(path.extract(&doc), Some(&json!("Alice")));
    }

    #[test]
    fn test_extract_nested() {
        let doc = json!({
            "metadata": {
                "source": "web",
                "campaign": "summer"
            }
        });
        let path = JsonPath::parse("metadata.source").unwrap();
        assert_eq!(path.extract(&doc), Some(&json!("web")));
    }

    #[test]
    fn test_extract_deep_nested() {
        let doc = json!({
            "profile": {
                "address": {
                    "city": "Paris",
                    "country": "FR"
                }
            }
        });
        let path = JsonPath::parse("profile.address.city").unwrap();
        assert_eq!(path.extract(&doc), Some(&json!("Paris")));
    }

    #[test]
    fn test_extract_array() {
        let doc = json!({
            "items": [
                {"sku": "A1", "qty": 2},
                {"sku": "B2", "qty": 1}
            ]
        });
        let path = JsonPath::parse("items[0].sku").unwrap();
        assert_eq!(path.extract(&doc), Some(&json!("A1")));

        let path = JsonPath::parse("items[1].sku").unwrap();
        assert_eq!(path.extract(&doc), Some(&json!("B2")));
    }

    #[test]
    fn test_extract_missing_returns_none() {
        let doc = json!({"name": "Alice"});
        let path = JsonPath::parse("nonexistent").unwrap();
        assert_eq!(path.extract(&doc), None);

        let path = JsonPath::parse("name.nested").unwrap();
        assert_eq!(path.extract(&doc), None);
    }

    #[test]
    fn test_extract_or_null() {
        let doc = json!({"name": "Alice"});
        let path = JsonPath::parse("nonexistent").unwrap();
        assert_eq!(path.extract_or_null(&doc), Value::Null);

        let path = JsonPath::parse("name").unwrap();
        assert_eq!(path.extract_or_null(&doc), json!("Alice"));
    }

    #[test]
    fn test_root_property() {
        let path = JsonPath::parse("metadata.source").unwrap();
        assert_eq!(path.root_property(), Some("metadata"));

        let path = JsonPath::parse("[0].field").unwrap();
        assert_eq!(path.root_property(), None);
    }

    #[test]
    fn test_tail() {
        let path = JsonPath::parse("a.b.c").unwrap();
        let tail = path.tail();
        assert_eq!(tail.segments.len(), 2);
        assert_eq!(tail.to_string(), "b.c");
    }

    #[test]
    fn test_display() {
        let path = JsonPath::parse("metadata.source").unwrap();
        assert_eq!(path.to_string(), "metadata.source");

        let path = JsonPath::parse("items[0].sku").unwrap();
        assert_eq!(path.to_string(), "items[0].sku");
    }

    #[test]
    fn test_serialization() {
        let path = JsonPath::parse("metadata.source").unwrap();
        let json = serde_json::to_string(&path).unwrap();
        let parsed: JsonPath = serde_json::from_str(&json).unwrap();
        assert_eq!(path, parsed);
    }
}
