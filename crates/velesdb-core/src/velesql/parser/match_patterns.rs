//! Graph pattern parsing for MATCH clauses (node, relationship, path patterns).
//!
//! Extracted from `match_clause.rs` to comply with the 500 NLOC file limit.
//! These functions parse the graph pattern elements of a MATCH query:
//! node patterns `(n:Label {props})`, relationship patterns `-[r:TYPE*1..3]->`,
//! and composite path patterns linking them together.

use crate::velesql::error::ParseError;
use crate::velesql::graph_pattern::{Direction, GraphPattern, NodePattern, RelationshipPattern};

use super::match_clause::split_with_braces;

/// Parses a single node pattern.
///
/// # Errors
///
/// Returns [`ParseError`] when delimiters are invalid or properties cannot be parsed.
pub fn parse_node_pattern(input: &str) -> Result<NodePattern, ParseError> {
    let input = input.trim();
    validate_node_delimiters(input)?;
    let inner = input[1..input.len() - 1].trim();
    if inner.is_empty() {
        return Ok(NodePattern::new());
    }
    let mut node = NodePattern::new();
    let (main_part, properties) = split_with_braces(inner, input, "node pattern")?;
    node.properties = properties;
    apply_alias_and_labels(main_part, &mut node);
    Ok(node)
}

/// Validates that a node pattern string starts with `(` and ends with `)`.
fn validate_node_delimiters(input: &str) -> Result<(), ParseError> {
    if !input.starts_with('(') {
        return Err(ParseError::syntax(
            0,
            input,
            "Node pattern must start with '('",
        ));
    }
    if !input.ends_with(')') {
        return Err(ParseError::syntax(input.len(), input, "Expected ')'"));
    }
    Ok(())
}

/// Extracts alias, labels, and optional collection from a node identifier.
///
/// Supports:
/// - `n:Person` -> alias=n, labels=\[Person\]
/// - `n:Person:Author` -> alias=n, labels=\[Person, Author\]
/// - `n:Person@products` -> alias=n, labels=\[Person\], collection=products
/// - `:Product@catalog` -> labels=\[Product\], collection=catalog
fn apply_alias_and_labels(main_part: &str, node: &mut NodePattern) {
    if main_part.is_empty() {
        return;
    }

    let (part_without_coll, collection) = extract_collection_annotation(main_part);

    let parts: Vec<&str> = part_without_coll.split(':').collect();
    if !parts[0].trim().is_empty() {
        node.alias = Some(parts[0].trim().to_string());
    }
    for label in &parts[1..] {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            node.labels.push(trimmed.to_string());
        }
    }

    node.collection = collection;
}

/// Extracts `@collection` annotation from a node identifier string.
///
/// Returns `(identifier_without_annotation, Some(collection_name))` if found,
/// or `(original, None)` if no `@` is present.
fn extract_collection_annotation(input: &str) -> (&str, Option<String>) {
    if let Some(at_pos) = input.rfind('@') {
        let before = &input[..at_pos];
        let after = input[at_pos + 1..].trim();
        if !after.is_empty() {
            return (before, Some(after.to_string()));
        }
    }
    (input, None)
}

/// Parses a relationship pattern.
///
/// # Errors
///
/// Returns [`ParseError`] when direction/brackets are malformed or relationship
/// details cannot be parsed.
pub fn parse_relationship_pattern(input: &str) -> Result<RelationshipPattern, ParseError> {
    let input = input.trim();
    let (direction, is, ie) = detect_direction_and_brackets(input)?;
    let mut rel = RelationshipPattern::new(direction);

    validate_bracket_matching(input)?;

    if input.contains('[') && input.contains(']') {
        parse_bracket_contents(input, is, ie, &mut rel)?;
    }
    Ok(rel)
}

/// Detects relationship direction and returns bracket positions.
fn detect_direction_and_brackets(input: &str) -> Result<(Direction, usize, usize), ParseError> {
    if input.starts_with("<-") && input.ends_with('-') {
        Ok((
            Direction::Incoming,
            input.find('[').unwrap_or(2),
            input.rfind(']').unwrap_or(input.len() - 1),
        ))
    } else if input.starts_with('-') && input.ends_with("->") {
        Ok((
            Direction::Outgoing,
            input.find('[').unwrap_or(1),
            input.rfind(']').unwrap_or(input.len() - 2),
        ))
    } else if input.starts_with('-') && input.ends_with('-') {
        Ok((
            Direction::Both,
            input.find('[').unwrap_or(1),
            input.rfind(']').unwrap_or(input.len() - 1),
        ))
    } else {
        Err(ParseError::syntax(
            0,
            input,
            "Invalid relationship direction",
        ))
    }
}

/// Validates that brackets are matched (both present or both absent).
fn validate_bracket_matching(input: &str) -> Result<(), ParseError> {
    let has_open = input.contains('[');
    let has_close = input.contains(']');
    if has_open != has_close {
        return Err(ParseError::syntax(
            0,
            input,
            if has_open {
                "Missing closing ']' in relationship pattern"
            } else {
                "Missing opening '[' in relationship pattern"
            },
        ));
    }
    Ok(())
}

/// Parses the contents between brackets in a relationship pattern.
fn parse_bracket_contents(
    input: &str,
    is: usize,
    ie: usize,
    rel: &mut RelationshipPattern,
) -> Result<(), ParseError> {
    if ie <= is {
        return Err(ParseError::syntax(
            is,
            input,
            "Mismatched brackets in relationship pattern",
        ));
    }
    let inner = input[is + 1..ie].trim();
    if inner.is_empty() {
        return Ok(());
    }
    if let Some(sp) = inner.find('*') {
        if let Some((s, e)) = parse_range(&inner[sp + 1..]) {
            rel.range = Some((s, e));
        }
        parse_rel_details(inner[..sp].trim(), rel)?;
    } else {
        parse_rel_details(inner, rel)?;
    }
    Ok(())
}

fn parse_rel_details(input: &str, rel: &mut RelationshipPattern) -> Result<(), ParseError> {
    if input.is_empty() {
        return Ok(());
    }
    let (main_part, props) = split_with_braces(input, input, "relationship properties")?;
    rel.properties = props;
    if let Some(stripped) = main_part.strip_prefix(':') {
        parse_rel_types(stripped, rel);
    } else if let Some(cp) = main_part.find(':') {
        rel.alias = Some(main_part[..cp].trim().to_string());
        parse_rel_types(&main_part[cp + 1..], rel);
    } else if !main_part.is_empty() {
        rel.alias = Some(main_part.to_string());
    }
    Ok(())
}

fn parse_rel_types(input: &str, rel: &mut RelationshipPattern) {
    for t in input.split('|') {
        if !t.trim().is_empty() {
            rel.types.push(t.trim().to_string());
        }
    }
}

/// Parses variable-length range after `*`.
fn parse_range(input: &str) -> Option<(u32, u32)> {
    let input = input.trim();
    if input.is_empty() {
        return Some((1, u32::MAX));
    }
    if let Some(d) = input.find("..") {
        Some((
            input[..d].trim().parse().unwrap_or(1),
            input[d + 2..].trim().parse().unwrap_or(u32::MAX),
        ))
    } else {
        input.parse::<u32>().ok().map(|n| (n, n))
    }
}

/// Parses a comma-separated list of graph patterns from the MATCH clause body.
pub(super) fn parse_pattern_list(input: &str) -> Result<Vec<GraphPattern>, ParseError> {
    let (name, ps) = if let Some(eq) = input.find('=') {
        let b = input[..eq].trim();
        if b.chars().all(|c| c.is_alphanumeric() || c == '_') {
            (Some(b.to_string()), input[eq + 1..].trim())
        } else {
            (None, input)
        }
    } else {
        (None, input)
    };
    let mut pattern = parse_path_pattern(ps)?;
    pattern.name = name;
    Ok(vec![pattern])
}

fn parse_path_pattern(input: &str) -> Result<GraphPattern, ParseError> {
    let mut nodes = Vec::new();
    let mut rels = Vec::new();
    let mut pos = 0;
    let input = input.trim();
    while pos < input.len() {
        if let Some(s) = input[pos..].find('(') {
            let abs = pos + s;
            let end = find_matching_paren(input, abs)?;
            nodes.push(parse_node_pattern(&input[abs..=end])?);
            pos = end + 1;
            if pos < input.len() {
                let rem = &input[pos..];
                if rem.starts_with('-') || rem.starts_with('<') {
                    if let Some(np) = rem.find('(') {
                        rels.push(parse_relationship_pattern(&rem[..np])?);
                        pos += np;
                    }
                }
            }
        } else {
            break;
        }
    }
    Ok(GraphPattern {
        name: None,
        nodes,
        relationships: rels,
    })
}

fn find_matching_paren(input: &str, start: usize) -> Result<usize, ParseError> {
    let mut d = 0;
    for (i, c) in input[start..].char_indices() {
        match c {
            '(' => d += 1,
            ')' => {
                d -= 1;
                if d == 0 {
                    return Ok(start + i);
                }
            }
            _ => {}
        }
    }
    Err(ParseError::syntax(start, input, "Expected ')'"))
}
