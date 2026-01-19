//! MATCH clause parser for graph pattern matching.

use crate::velesql::ast::{CompareOp, Comparison, Condition, Value};
use crate::velesql::error::ParseError;
use crate::velesql::graph_pattern::{
    Direction, GraphPattern, MatchClause, NodePattern, RelationshipPattern, ReturnClause,
    ReturnItem,
};
use std::collections::HashMap;

/// Parses a complete MATCH clause.
pub fn parse_match_clause(input: &str) -> Result<MatchClause, ParseError> {
    let input = input.trim();
    if !input.to_uppercase().starts_with("MATCH ") {
        return Err(ParseError::syntax(0, input, "Expected MATCH keyword"));
    }
    let after_match = input[6..].trim_start();
    let return_pos = find_keyword(after_match, "RETURN")
        .ok_or_else(|| ParseError::syntax(input.len(), input, "Expected RETURN clause"))?;
    let where_pos = find_keyword(&after_match[..return_pos], "WHERE");
    let pattern_end = where_pos.unwrap_or(return_pos);
    let pattern_str = after_match[..pattern_end].trim();
    if pattern_str.is_empty() {
        return Err(ParseError::syntax(6, input, "Expected pattern after MATCH"));
    }
    let patterns = parse_pattern_list(pattern_str)?;
    let where_clause = if let Some(wp) = where_pos {
        // Validate slice bounds: wp + 5 (after "WHERE") must be <= return_pos
        let where_end = wp + 5;
        if where_end > return_pos {
            return Err(ParseError::syntax(wp, input, "Empty WHERE condition"));
        }
        Some(parse_where_condition(
            after_match[where_end..return_pos].trim(),
        )?)
    } else {
        None
    };
    let return_clause = parse_return_clause(after_match[return_pos + 6..].trim());
    Ok(MatchClause {
        patterns,
        where_clause,
        return_clause,
    })
}

/// Parses a single node pattern.
pub fn parse_node_pattern(input: &str) -> Result<NodePattern, ParseError> {
    let input = input.trim();
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
    let inner = input[1..input.len() - 1].trim();
    if inner.is_empty() {
        return Ok(NodePattern::new());
    }
    let mut node = NodePattern::new();
    let (main_part, properties) = if let Some(ps) = inner.find('{') {
        let pe = inner
            .rfind('}')
            .ok_or_else(|| ParseError::syntax(ps, input, "Expected '}'"))?;
        if pe <= ps {
            return Err(ParseError::syntax(
                ps,
                input,
                "Mismatched braces in node pattern",
            ));
        }
        (inner[..ps].trim(), parse_properties(&inner[ps + 1..pe])?)
    } else {
        (inner, HashMap::new())
    };
    node.properties = properties;
    if !main_part.is_empty() {
        let parts: Vec<&str> = main_part.split(':').collect();
        if !parts[0].trim().is_empty() {
            node.alias = Some(parts[0].trim().to_string());
        }
        for label in &parts[1..] {
            if !label.trim().is_empty() {
                node.labels.push(label.trim().to_string());
            }
        }
    }
    Ok(node)
}

/// Parses a relationship pattern.
pub fn parse_relationship_pattern(input: &str) -> Result<RelationshipPattern, ParseError> {
    let input = input.trim();
    let (direction, is, ie) = if input.starts_with("<-") && input.ends_with('-') {
        (
            Direction::Incoming,
            input.find('[').unwrap_or(2),
            input.rfind(']').unwrap_or(input.len() - 1),
        )
    } else if input.starts_with('-') && input.ends_with("->") {
        (
            Direction::Outgoing,
            input.find('[').unwrap_or(1),
            input.rfind(']').unwrap_or(input.len() - 2),
        )
    } else if input.starts_with('-') && input.ends_with('-') {
        (
            Direction::Both,
            input.find('[').unwrap_or(1),
            input.rfind(']').unwrap_or(input.len() - 1),
        )
    } else {
        return Err(ParseError::syntax(
            0,
            input,
            "Invalid relationship direction",
        ));
    };
    let mut rel = RelationshipPattern::new(direction);

    // Validate bracket matching
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

    if has_open && has_close {
        if ie <= is {
            return Err(ParseError::syntax(
                is,
                input,
                "Mismatched brackets in relationship pattern",
            ));
        }
        let inner = input[is + 1..ie].trim();
        if !inner.is_empty() {
            if let Some(sp) = inner.find('*') {
                if let Some((s, e)) = parse_range(&inner[sp + 1..]) {
                    rel.range = Some((s, e));
                }
                parse_rel_details(inner[..sp].trim(), &mut rel)?;
            } else {
                parse_rel_details(inner, &mut rel)?;
            }
        }
    }
    Ok(rel)
}

fn parse_rel_details(input: &str, rel: &mut RelationshipPattern) -> Result<(), ParseError> {
    if input.is_empty() {
        return Ok(());
    }
    let (main_part, props) = if let Some(ps) = input.find('{') {
        let pe = input
            .rfind('}')
            .ok_or_else(|| ParseError::syntax(ps, input, "Expected '}'"))?;
        if pe <= ps {
            return Err(ParseError::syntax(
                ps,
                input,
                "Mismatched braces in relationship properties",
            ));
        }
        (input[..ps].trim(), parse_properties(&input[ps + 1..pe])?)
    } else {
        (input, HashMap::new())
    };
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

/// Splits properties respecting string literals (commas inside quotes are preserved).
fn parse_properties(input: &str) -> Result<HashMap<String, Value>, ParseError> {
    let mut props = HashMap::new();
    let mut in_string = false;
    let mut start = 0;

    for (i, ch) in input.char_indices() {
        if ch == '\'' {
            in_string = !in_string;
        } else if ch == ',' && !in_string {
            let prop = input[start..i].trim();
            if let Some(c) = prop.find(':') {
                props.insert(
                    prop[..c].trim().to_string(),
                    parse_value(prop[c + 1..].trim())?,
                );
            }
            start = i + 1;
        }
    }

    let prop = input[start..].trim();
    if let Some(c) = prop.find(':') {
        props.insert(
            prop[..c].trim().to_string(),
            parse_value(prop[c + 1..].trim())?,
        );
    }

    Ok(props)
}

fn parse_value(input: &str) -> Result<Value, ParseError> {
    if input.len() >= 2 && input.starts_with('\'') && input.ends_with('\'') {
        Ok(Value::String(input[1..input.len() - 1].to_string()))
    } else if input.eq_ignore_ascii_case("true") {
        Ok(Value::Boolean(true))
    } else if input.eq_ignore_ascii_case("false") {
        Ok(Value::Boolean(false))
    } else if input.eq_ignore_ascii_case("null") {
        Ok(Value::Null)
    } else if let Ok(i) = input.parse::<i64>() {
        Ok(Value::Integer(i))
    } else if let Ok(f) = input.parse::<f64>() {
        Ok(Value::Float(f))
    } else {
        Err(ParseError::syntax(
            0,
            input,
            format!("Invalid value: {input}"),
        ))
    }
}

fn parse_pattern_list(input: &str) -> Result<Vec<GraphPattern>, ParseError> {
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
    // Use char_indices() to get byte indices, not character indices
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

fn parse_where_condition(input: &str) -> Result<Condition, ParseError> {
    // Order matters: check multi-char operators before single-char ones
    // Use string-literal-aware search to avoid matching operators inside quotes
    let (col, op, vs) = if let Some(p) = find_operator(input, "!=") {
        (&input[..p], CompareOp::NotEq, input[p + 2..].trim())
    } else if let Some(p) = find_operator(input, "<>") {
        (&input[..p], CompareOp::NotEq, input[p + 2..].trim())
    } else if let Some(p) = find_operator(input, ">=") {
        (&input[..p], CompareOp::Gte, input[p + 2..].trim())
    } else if let Some(p) = find_operator(input, "<=") {
        (&input[..p], CompareOp::Lte, input[p + 2..].trim())
    } else if let Some(p) = find_operator(input, ">") {
        (&input[..p], CompareOp::Gt, input[p + 1..].trim())
    } else if let Some(p) = find_operator(input, "<") {
        (&input[..p], CompareOp::Lt, input[p + 1..].trim())
    } else if let Some(p) = find_operator(input, "=") {
        (&input[..p], CompareOp::Eq, input[p + 1..].trim())
    } else {
        return Err(ParseError::syntax(0, input, "Invalid WHERE"));
    };
    Ok(Condition::Comparison(Comparison {
        column: col.trim().to_string(),
        operator: op,
        value: parse_value(vs)?,
    }))
}

/// Finds an operator in the input string, respecting string literal boundaries.
/// Returns the byte position of the operator, or None if not found outside quotes.
fn find_operator(input: &str, op: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let op_bytes = op.as_bytes();
    let op_len = op_bytes.len();

    if op_len == 0 || bytes.len() < op_len {
        return None;
    }

    let mut in_string = false;
    let mut i = 0;

    while i <= bytes.len() - op_len {
        let b = bytes[i];

        // Track string literal boundaries
        if b == b'\'' {
            in_string = !in_string;
            i += 1;
            continue;
        }

        // Skip if inside a string literal
        if in_string {
            i += 1;
            continue;
        }

        // Check if operator matches at this position
        if &bytes[i..i + op_len] == op_bytes {
            return Some(i);
        }

        i += 1;
    }

    None
}

fn parse_return_clause(input: &str) -> ReturnClause {
    let (is, limit) = if let Some(lp) = find_keyword(input, "LIMIT") {
        (&input[..lp], input[lp + 5..].trim().parse().ok())
    } else {
        (input, None)
    };
    let items = is
        .split(',')
        .map(|i| {
            let i = i.trim();
            if let Some(ap) = find_keyword(i, "AS") {
                ReturnItem {
                    expression: i[..ap].trim().to_string(),
                    alias: Some(i[ap + 2..].trim().to_string()),
                }
            } else {
                ReturnItem {
                    expression: i.to_string(),
                    alias: None,
                }
            }
        })
        .collect();
    ReturnClause {
        items,
        order_by: None,
        limit,
    }
}

/// Finds a keyword in the input string, respecting string literal boundaries.
/// Uses ASCII-only case-insensitive matching to avoid Unicode index issues.
fn find_keyword(input: &str, kw: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let kw_bytes = kw.as_bytes();
    let kw_len = kw_bytes.len();

    if kw_len == 0 || bytes.len() < kw_len {
        return None;
    }

    let mut in_string = false;
    let mut i = 0;

    while i <= bytes.len() - kw_len {
        let b = bytes[i];

        // Track string literal boundaries
        if b == b'\'' {
            in_string = !in_string;
            i += 1;
            continue;
        }

        // Skip if inside a string literal
        if in_string {
            i += 1;
            continue;
        }

        // Check if keyword matches at this position (ASCII case-insensitive)
        if bytes[i..i + kw_len]
            .iter()
            .zip(kw_bytes.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            // Check word boundaries (underscore is part of identifiers)
            let before_ok =
                i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let after_ok = i + kw_len >= bytes.len()
                || !(bytes[i + kw_len].is_ascii_alphanumeric() || bytes[i + kw_len] == b'_');

            if before_ok && after_ok {
                return Some(i);
            }
        }

        i += 1;
    }

    None
}
