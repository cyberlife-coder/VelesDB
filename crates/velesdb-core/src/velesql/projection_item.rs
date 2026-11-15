//! Parsing of a MATCH `RETURN` item into what it projects.
//!
//! Pure VelesQL: it reads the item's text only. The core MATCH executor
//! (`collection::search::query::match_exec`, which re-exports these names)
//! and the WASM executor both project through it, so the two builds read
//! `RETURN` alike.

/// A parsed RETURN clause projection item (Fix #489).
///
/// Replaces the former `parse_property_path()` that silently returned `None`
/// for wildcards, function calls, and bare aliases — leaving `projected` empty.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectionItem<'a> {
    /// `RETURN *` — project all properties from all bound aliases.
    Wildcard,
    /// `RETURN similarity()` — a function call expression.
    /// The inner `&str` is the function name (e.g., `"similarity"`).
    FunctionCall(&'a str),
    /// `RETURN n.name` — a dotted property path.
    PropertyPath {
        /// The alias portion (e.g., `"n"`).
        alias: &'a str,
        /// The property portion (e.g., `"name"` or `"metadata.category"`).
        property: &'a str,
    },
    /// `RETURN n` — a bare alias referring to a bound node.
    BareAlias(&'a str),
}

/// Parses a RETURN clause expression into a [`ProjectionItem`] (Fix #489).
///
/// Handles four patterns:
/// - `"*"` → [`ProjectionItem::Wildcard`]
/// - `"similarity()"` → `ProjectionItem::FunctionCall("similarity")`
/// - `"n.name"` → `ProjectionItem::PropertyPath { alias: "n", property: "name" }`
/// - `"n"` → `ProjectionItem::BareAlias("n")`
#[must_use]
pub fn parse_projection_item(expression: &str) -> ProjectionItem<'_> {
    if expression == "*" {
        return ProjectionItem::Wildcard;
    }

    // Function calls contain '(' — extract the name before the parenthesis.
    if let Some(paren_pos) = expression.find('(') {
        let name = &expression[..paren_pos];
        return ProjectionItem::FunctionCall(name);
    }

    // Dotted property path: split on first dot (both halves must be non-empty).
    if let Some(dot_pos) = expression.find('.') {
        let alias = &expression[..dot_pos];
        let property = &expression[dot_pos + 1..];
        if !alias.is_empty() && !property.is_empty() {
            return ProjectionItem::PropertyPath { alias, property };
        }
    }

    // Everything else is a bare alias (including edge cases like ".x" or "x.").
    ProjectionItem::BareAlias(expression)
}

/// Parses a property path expression like "alias.property" (EPIC-058 US-007).
///
/// Returns `Some((alias, property))` if valid, `None` otherwise.
/// For nested paths like "doc.metadata.category", returns `("doc", "metadata.category")`.
///
/// **Prefer [`parse_projection_item`]** for RETURN clause projection — this function
/// only handles `PropertyPath` cases and returns `None` for wildcards, function calls,
/// and bare aliases.
#[must_use]
pub fn parse_property_path(expression: &str) -> Option<(&str, &str)> {
    match parse_projection_item(expression) {
        ProjectionItem::PropertyPath { alias, property } => Some((alias, property)),
        _ => None,
    }
}
