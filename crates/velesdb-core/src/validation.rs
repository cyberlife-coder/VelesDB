//! Unified validation helpers.
//!
//! Centralizes dimension range checks, collection name validation, and
//! mismatch validation used across collection creation, CRUD, and search paths.

use crate::error::{Error, Result};

/// Maximum allowed length for a collection name.
pub const MAX_COLLECTION_NAME_LENGTH: usize = 128;

/// Minimum valid vector dimension.
pub const MIN_DIMENSION: usize = 1;

/// Maximum valid vector dimension (65,536 — covers all known embedding models).
pub const MAX_DIMENSION: usize = 65_536;

/// Validates that a vector dimension is within the allowed range.
///
/// # Errors
///
/// Returns [`Error::InvalidDimension`] if `dimension` is outside
/// [`MIN_DIMENSION`]`..=`[`MAX_DIMENSION`].
pub fn validate_dimension(dimension: usize) -> Result<()> {
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&dimension) {
        return Err(Error::InvalidDimension {
            dimension,
            min: MIN_DIMENSION,
            max: MAX_DIMENSION,
        });
    }
    Ok(())
}

/// Validates that a vector's actual dimension matches the expected dimension.
///
/// # Errors
///
/// Returns [`crate::error::Error::DimensionMismatch`] if `actual != expected`.
pub fn validate_dimension_match(expected: usize, actual: usize) -> Result<()> {
    if actual != expected {
        return Err(Error::DimensionMismatch { expected, actual });
    }
    Ok(())
}

/// Validates that a collection name is safe for use as a filesystem directory.
///
/// # Rules
///
/// - Must not be empty.
/// - Must not exceed [`MAX_COLLECTION_NAME_LENGTH`] characters.
/// - Must contain only ASCII alphanumeric characters, underscores, or hyphens
///   (`[a-zA-Z0-9_-]`).
/// - Must not be `.` or `..` (path traversal).
/// - Must not start with a hyphen (avoids CLI flag confusion).
/// - Must not be a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`,
///   `COM1`–`COM9`, `LPT1`–`LPT9`).
///
/// # Errors
///
/// Returns [`Error::InvalidCollectionName`] with a human-readable reason.
///
/// # Examples
///
/// ```
/// use velesdb_core::validate_collection_name;
///
/// assert!(validate_collection_name("my_collection").is_ok());
/// assert!(validate_collection_name("docs-v2").is_ok());
/// assert!(validate_collection_name("").is_err());
/// assert!(validate_collection_name("../evil").is_err());
/// assert!(validate_collection_name("a/b").is_err());
/// ```
pub fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(invalid_name(name, "must not be empty"));
    }

    if name.len() > MAX_COLLECTION_NAME_LENGTH {
        return Err(invalid_name(
            name,
            &format!("exceeds maximum length of {MAX_COLLECTION_NAME_LENGTH} characters"),
        ));
    }

    if name == "." || name == ".." {
        return Err(invalid_name(name, "path traversal is not allowed"));
    }

    if name.starts_with('-') {
        return Err(invalid_name(name, "must not start with a hyphen"));
    }

    if let Some(bad) = name.chars().find(|c| !is_valid_name_char(*c)) {
        return Err(invalid_name(
            name,
            &format!(
                "contains forbidden character '{bad}'; \
                 only ASCII letters, digits, underscores, and hyphens are allowed"
            ),
        ));
    }

    if is_windows_reserved(name) {
        return Err(invalid_name(name, "is a Windows reserved device name"));
    }

    Ok(())
}

/// Returns `true` if `c` is allowed in a collection name.
fn is_valid_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Returns `true` if `name` matches a Windows reserved device name
/// (case-insensitive).
fn is_windows_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Convenience constructor for [`Error::InvalidCollectionName`].
fn invalid_name(name: &str, reason: &str) -> Error {
    Error::InvalidCollectionName {
        name: name.to_string(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_ascii_names() {
        for name in ["a", "abc", "my_coll", "docs-v2", "A1_b2-C3"] {
            validate_collection_name(name).unwrap();
        }
    }

    #[test]
    fn accepts_max_length() {
        let name = "x".repeat(MAX_COLLECTION_NAME_LENGTH);
        validate_collection_name(&name).unwrap();
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_collection_name("").is_err());
    }

    #[test]
    fn rejects_over_max_length() {
        let name = "x".repeat(MAX_COLLECTION_NAME_LENGTH + 1);
        assert!(validate_collection_name(&name).is_err());
    }

    #[test]
    fn rejects_dot_and_dotdot() {
        assert!(validate_collection_name(".").is_err());
        assert!(validate_collection_name("..").is_err());
    }

    #[test]
    fn rejects_path_separators() {
        assert!(validate_collection_name("a/b").is_err());
        assert!(validate_collection_name("a\\b").is_err());
        assert!(validate_collection_name("../x").is_err());
    }

    #[test]
    fn rejects_leading_hyphen() {
        assert!(validate_collection_name("-bad").is_err());
        assert!(validate_collection_name("--bad").is_err());
    }

    #[test]
    fn allows_interior_hyphens() {
        validate_collection_name("a-b").unwrap();
        validate_collection_name("a-b-c").unwrap();
    }

    #[test]
    fn rejects_special_chars() {
        for name in ["a b", "a@b", "a.b", "a#b", "a$b", "a:b", "a*b"] {
            assert!(
                validate_collection_name(name).is_err(),
                "Should reject {:?}",
                name
            );
        }
    }

    #[test]
    fn rejects_unicode() {
        assert!(validate_collection_name("café").is_err());
        assert!(validate_collection_name("日本").is_err());
    }

    #[test]
    fn rejects_windows_reserved_case_insensitive() {
        for name in ["CON", "con", "Con", "PRN", "AUX", "NUL", "COM1", "LPT9"] {
            assert!(
                validate_collection_name(name).is_err(),
                "Should reject {:?}",
                name
            );
        }
    }

    #[test]
    fn allows_names_containing_reserved_as_substring() {
        // "connection" contains "con" but is not the reserved name "CON"
        validate_collection_name("connection").unwrap();
        validate_collection_name("my_aux_data").unwrap();
        validate_collection_name("com10").unwrap();
    }

    #[test]
    fn error_code_is_veles_034() {
        let err = validate_collection_name("").unwrap_err();
        assert_eq!(err.code(), "VELES-034");
    }

    #[test]
    fn error_is_recoverable() {
        let err = validate_collection_name("").unwrap_err();
        assert!(err.is_recoverable());
    }
}
