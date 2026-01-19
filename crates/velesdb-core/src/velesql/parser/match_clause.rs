//! MATCH clause parser for graph pattern matching.

use crate::velesql::ast::{
    CompareOp, Comparison, Condition, Direction, GraphPattern, MatchClause, NodePattern,
    RelationshipPattern, ReturnClause, ReturnItem, Value,
};
use crate::velesql::error::ParseError;
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
        Some(parse_where_condition(
            after_match[wp + 5..return_pos].trim(),
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
    if input.contains('[') && input.contains(']') {
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

fn parse_properties(input: &str) -> Result<HashMap<String, Value>, ParseError> {
    let mut props = HashMap::new();
    for prop in input.split(',') {
        if let Some(c) = prop.find(':') {
            props.insert(
                prop[..c].trim().to_string(),
                parse_value(prop[c + 1..].trim())?,
            );
        }
    }
    Ok(props)
}

fn parse_value(input: &str) -> Result<Value, ParseError> {
    if input.starts_with('\'') && input.ends_with('\'') {
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
    for (i, c) in input[start..].chars().enumerate() {
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
    let (col, op, vs) = if let Some(p) = input.find(">=") {
        (&input[..p], CompareOp::Gte, input[p + 2..].trim())
    } else if let Some(p) = input.find("<=") {
        (&input[..p], CompareOp::Lte, input[p + 2..].trim())
    } else if let Some(p) = input.find('>') {
        (&input[..p], CompareOp::Gt, input[p + 1..].trim())
    } else if let Some(p) = input.find('<') {
        (&input[..p], CompareOp::Lt, input[p + 1..].trim())
    } else if let Some(p) = input.find('=') {
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

fn find_keyword(input: &str, kw: &str) -> Option<usize> {
    let ui = input.to_uppercase();
    let uk = kw.to_uppercase();
    let mut p = 0;
    while let Some(f) = ui[p..].find(&uk) {
        let ap = p + f;
        let bok = ap == 0 || !input.chars().nth(ap - 1).unwrap_or(' ').is_alphanumeric();
        let aok = ap + kw.len() >= input.len()
            || !input
                .chars()
                .nth(ap + kw.len())
                .unwrap_or(' ')
                .is_alphanumeric();
        if bok && aok {
            return Some(ap);
        }
        p = ap + 1;
    }
    None
}
