//! MATCH query parsing module (EPIC-061/US-002 refactoring).
//!
//! Extracted from select.rs to reduce file size and improve modularity.

use super::{extract_identifier, Rule};
use crate::velesql::ast::{OrderByExpr, Query};
use crate::velesql::error::ParseError;
use crate::velesql::graph_pattern::{
    Direction, GraphPattern, MatchClause, NodePattern, RelationshipPattern, ReturnClause,
    ReturnItem,
};
use crate::velesql::Parser;

impl Parser {
    /// Parse a MATCH query (EPIC-045 US-001).
    pub(crate) fn parse_match_query(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut patterns = Vec::new();
        let mut where_clause = None;
        let mut return_clause = ReturnClause {
            items: Vec::new(),
            order_by: None,
            limit: None,
        };
        let mut limit = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::graph_pattern => {
                    patterns.push(Self::parse_graph_pattern(inner_pair)?);
                }
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(inner_pair)?);
                }
                Rule::return_clause => {
                    return_clause = Self::parse_return_clause(inner_pair)?;
                }
                Rule::order_by_clause => {
                    // EPIC-045 US-005: Parse ORDER BY for MATCH queries
                    let order_by = Self::parse_order_by_clause(inner_pair)?;
                    // Convert SelectOrderBy to graph_pattern::OrderByItem
                    return_clause.order_by = Some(
                        order_by
                            .into_iter()
                            .map(|ob| crate::velesql::graph_pattern::OrderByItem {
                                expression: match ob.expr {
                                    OrderByExpr::Field(f) => f,
                                    OrderByExpr::Similarity(s) => {
                                        let vec_str = match &s.vector {
                                            crate::velesql::ast::VectorExpr::Parameter(name) => {
                                                format!("${name}")
                                            }
                                            crate::velesql::ast::VectorExpr::Literal(vals) => {
                                                format!("{vals:?}")
                                            }
                                        };
                                        format!("similarity({}, {vec_str})", s.field)
                                    }
                                    OrderByExpr::Aggregate(a) => format!("{:?}()", a.function_type),
                                },
                                descending: ob.descending,
                            })
                            .collect(),
                    );
                }
                Rule::limit_clause => {
                    for lp in inner_pair.into_inner() {
                        if lp.as_rule() == Rule::integer {
                            limit = lp.as_str().parse().ok();
                        }
                    }
                }
                _ => {}
            }
        }

        // Apply limit to return_clause
        return_clause.limit = limit;

        let match_clause = MatchClause {
            patterns,
            where_clause,
            return_clause,
        };

        Ok(Query::new_match(match_clause))
    }

    /// Parse a graph pattern (EPIC-045 US-001).
    pub(super) fn parse_graph_pattern(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<GraphPattern, ParseError> {
        let mut nodes = Vec::new();
        let mut relationships = Vec::new();

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::node_pattern => {
                    nodes.push(Self::parse_node_pattern(inner_pair)?);
                }
                Rule::relationship_pattern => {
                    relationships.push(Self::parse_relationship_pattern(inner_pair)?);
                }
                _ => {}
            }
        }

        Ok(GraphPattern {
            name: None,
            nodes,
            relationships,
        })
    }

    /// Parse a node pattern (EPIC-045 US-001).
    fn parse_node_pattern(pair: pest::iterators::Pair<Rule>) -> Result<NodePattern, ParseError> {
        let mut node = NodePattern::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::node_spec {
                for spec_pair in inner_pair.into_inner() {
                    match spec_pair.as_rule() {
                        Rule::node_alias => {
                            node.alias = Some(spec_pair.as_str().to_string());
                        }
                        Rule::node_labels => {
                            for label_pair in spec_pair.into_inner() {
                                if label_pair.as_rule() == Rule::label_name {
                                    node.labels.push(label_pair.as_str().to_string());
                                }
                            }
                        }
                        Rule::node_properties => {
                            node.properties = Self::parse_node_properties(spec_pair)?;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(node)
    }

    /// Parse node properties (EPIC-045 US-001).
    pub(super) fn parse_node_properties(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<std::collections::HashMap<String, crate::velesql::Value>, ParseError> {
        use std::collections::HashMap;

        let mut props = HashMap::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::property_list {
                for prop_pair in inner_pair.into_inner() {
                    if prop_pair.as_rule() == Rule::property {
                        let mut key = String::new();
                        let mut value = crate::velesql::Value::Null;

                        for p in prop_pair.into_inner() {
                            match p.as_rule() {
                                Rule::identifier => {
                                    key = extract_identifier(&p);
                                }
                                Rule::property_value => {
                                    value = Self::parse_property_value(p)?;
                                }
                                _ => {}
                            }
                        }

                        if !key.is_empty() {
                            props.insert(key, value);
                        }
                    }
                }
            }
        }

        Ok(props)
    }

    /// Parse a property value (EPIC-045 US-001).
    #[allow(clippy::unnecessary_wraps)] // Consistent with other parse_* methods
    fn parse_property_value(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<crate::velesql::Value, ParseError> {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::string => {
                    let s = inner_pair.as_str();
                    return Ok(crate::velesql::Value::String(s[1..s.len() - 1].to_string()));
                }
                Rule::integer => {
                    return Ok(crate::velesql::Value::Integer(
                        inner_pair.as_str().parse().unwrap_or(0),
                    ));
                }
                Rule::float => {
                    return Ok(crate::velesql::Value::Float(
                        inner_pair.as_str().parse().unwrap_or(0.0),
                    ));
                }
                Rule::boolean => {
                    let val = inner_pair.as_str().to_uppercase() == "TRUE";
                    return Ok(crate::velesql::Value::Boolean(val));
                }
                Rule::null_value => {
                    return Ok(crate::velesql::Value::Null);
                }
                Rule::parameter => {
                    let name = inner_pair.as_str().trim_start_matches('$').to_string();
                    return Ok(crate::velesql::Value::Parameter(name));
                }
                _ => {}
            }
        }
        Ok(crate::velesql::Value::Null)
    }

    /// Parse a relationship pattern (EPIC-045 US-001).
    fn parse_relationship_pattern(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<RelationshipPattern, ParseError> {
        let mut direction = Direction::Outgoing;
        let mut rel = RelationshipPattern::new(direction);

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::rel_incoming => {
                    direction = Direction::Incoming;
                    rel = RelationshipPattern::new(direction);
                    Self::parse_rel_spec_inner(&mut rel, inner_pair)?;
                }
                Rule::rel_outgoing => {
                    direction = Direction::Outgoing;
                    rel = RelationshipPattern::new(direction);
                    Self::parse_rel_spec_inner(&mut rel, inner_pair)?;
                }
                Rule::rel_undirected => {
                    direction = Direction::Both;
                    rel = RelationshipPattern::new(direction);
                    Self::parse_rel_spec_inner(&mut rel, inner_pair)?;
                }
                _ => {}
            }
        }

        Ok(rel)
    }

    /// Parse relationship spec inner (EPIC-045 US-001).
    fn parse_rel_spec_inner(
        rel: &mut RelationshipPattern,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(), ParseError> {
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::rel_spec {
                for spec_pair in inner_pair.into_inner() {
                    if spec_pair.as_rule() == Rule::rel_details {
                        for detail_pair in spec_pair.into_inner() {
                            match detail_pair.as_rule() {
                                Rule::rel_alias => {
                                    rel.alias = Some(detail_pair.as_str().to_string());
                                }
                                Rule::rel_types => {
                                    for type_pair in detail_pair.into_inner() {
                                        if type_pair.as_rule() == Rule::rel_type_name {
                                            rel.types.push(type_pair.as_str().to_string());
                                        }
                                    }
                                }
                                Rule::rel_range => {
                                    rel.range = Self::parse_rel_range(detail_pair);
                                }
                                Rule::node_properties => {
                                    rel.properties = Self::parse_node_properties(detail_pair)?;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Parse relationship range (EPIC-045 US-001).
    #[allow(clippy::unnecessary_wraps)] // Option is for consistency with caller expectations
    fn parse_rel_range(pair: pest::iterators::Pair<Rule>) -> Option<(u32, u32)> {
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::range_spec {
                let text = inner_pair.as_str();
                if let Some(dot_pos) = text.find("..") {
                    let start: u32 = text[..dot_pos].parse().unwrap_or(1);
                    let end: u32 = text[dot_pos + 2..].parse().unwrap_or(u32::MAX);
                    return Some((start, end));
                } else if let Ok(exact) = text.parse::<u32>() {
                    return Some((exact, exact));
                }
            } else if inner_pair.as_rule() == Rule::integer {
                if let Ok(exact) = inner_pair.as_str().parse::<u32>() {
                    return Some((exact, exact));
                }
            }
        }
        // Default: unbounded
        Some((1, u32::MAX))
    }

    /// Parse RETURN clause (EPIC-045 US-001).
    #[allow(clippy::unnecessary_wraps)] // Consistent with other parse_* methods
    fn parse_return_clause(pair: pest::iterators::Pair<Rule>) -> Result<ReturnClause, ParseError> {
        let mut items = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::return_item_list {
                for item_pair in inner_pair.into_inner() {
                    if item_pair.as_rule() == Rule::return_item {
                        let mut expression = String::new();
                        let mut alias = None;

                        for p in item_pair.into_inner() {
                            match p.as_rule() {
                                Rule::return_expr => {
                                    expression = Self::parse_return_expr(p);
                                }
                                Rule::identifier => {
                                    alias = Some(extract_identifier(&p));
                                }
                                _ => {}
                            }
                        }

                        items.push(ReturnItem { expression, alias });
                    }
                }
            }
        }

        Ok(ReturnClause {
            items,
            order_by: None,
            limit: None,
        })
    }

    /// Parse RETURN expression (EPIC-045 US-001).
    fn parse_return_expr(pair: pest::iterators::Pair<Rule>) -> String {
        let text = pair.as_str().to_string();
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::similarity_return => {
                    return "similarity()".to_string();
                }
                Rule::property_access | Rule::identifier => {
                    return inner_pair.as_str().to_string();
                }
                _ => {}
            }
        }
        text
    }
}
