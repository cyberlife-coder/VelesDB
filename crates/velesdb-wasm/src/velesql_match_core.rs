//! MATCH rows shaped as core's `MatchResult`: `bindings`, `depth`,
//! `projected` — the rows of REST `POST /collections/{name}/match`.
//!
//! `executeQuery` returns one alias-keyed object per row and leaves the
//! RETURN projection to the caller. This module runs the same MATCH (same
//! read gate, same walk, same ORDER BY and LIMIT) and shapes each row the
//! way core's `project_properties` does, reusing core's own RETURN item
//! parser so both builds read RETURN alike:
//!
//! - `alias.property` projects under the item's `AS` alias, else its text;
//!   `alias.id` is the element id; a relationship alias reads the traversed
//!   edge's properties; an absent property leaves the key out.
//! - a bare node alias projects every payload property as `alias.key`, plus
//!   `alias.id`; `*` does so for every bound node.
//! - `similarity()` projects nothing: the browser path computes no scores,
//!   so rows carry no `score` either.
//!
//! `bindings` holds the explicitly aliased nodes only, and `depth` the number
//! of relationships walked, as in core.

use std::collections::HashMap;

use velesdb_core::velesql::{
    parse_projection_item, Parser, ProjectionItem, QueryValidator, ReturnClause, ReturnItem,
};

use crate::database::DatabaseInner;
use crate::velesql_match_orderby::{BoundElement, MatchBinding};
use crate::velesql_value::parse_params;

/// Runs the MATCH in `sql` and returns its rows in core's shape.
///
/// # Errors
///
/// Returns an error when `sql` does not parse or validate, is not a MATCH,
/// `params_json` is not a JSON object, the read gate denies a collection, or
/// the MATCH itself fails.
pub(crate) fn execute_match_rows(
    db: &mut DatabaseInner,
    sql: &str,
    params_json: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let query = Parser::parse(sql).map_err(|e| {
        format!(
            "VelesQL parse error at position {}: {} (near '{}')",
            e.position, e.message, e.fragment
        )
    })?;
    QueryValidator::validate(&query).map_err(|e| e.to_string())?;
    let Some(clause) = query.match_clause.as_ref() else {
        return Err("executeMatch runs MATCH statements only; use executeQuery".to_string());
    };
    let params = parse_params(params_json)?;
    let candidates = crate::velesql_graph::match_candidates_gated(db, &query, &params)?;
    Ok(candidates
        .iter()
        .map(|c| core_row(&c.binding, &clause.return_clause))
        .collect())
}

/// One row in core's shape.
fn core_row(binding: &MatchBinding, return_clause: &ReturnClause) -> serde_json::Value {
    let bindings: serde_json::Map<String, serde_json::Value> = binding
        .nodes
        .iter()
        .filter_map(|n| Some((n.alias.clone()?, serde_json::json!(n.id))))
        .collect();
    let mut projected = HashMap::new();
    for item in &return_clause.items {
        project_item(binding, item, &mut projected);
    }
    serde_json::json!({
        "bindings": bindings,
        "depth": binding.edges.len(),
        "projected": projected,
    })
}

fn project_item(
    binding: &MatchBinding,
    item: &ReturnItem,
    projected: &mut HashMap<String, serde_json::Value>,
) {
    match parse_projection_item(&item.expression) {
        ProjectionItem::Wildcard => {
            for node in binding.nodes.iter().filter(|n| n.alias.is_some()) {
                project_all(node, projected);
            }
        }
        ProjectionItem::FunctionCall(_) => {}
        ProjectionItem::PropertyPath { alias, property } => {
            let Some(element) = find(&binding.nodes, alias).or_else(|| find(&binding.edges, alias))
            else {
                return;
            };
            if let Some(value) = element_property(element, property) {
                projected.insert(projection_key(item), value);
            }
        }
        ProjectionItem::BareAlias(alias) => {
            if let Some(node) = find(&binding.nodes, alias) {
                project_all(node, projected);
            }
        }
        // A RETURN form core adds later projects nothing here until it is
        // mirrored, rather than a guess.
        _ => {}
    }
}

/// The element bound to `alias`, if any.
fn find<'b>(elements: &'b [BoundElement], alias: &str) -> Option<&'b BoundElement> {
    elements.iter().find(|e| e.alias.as_deref() == Some(alias))
}

/// Core's `element_property`: `id` is the element id, whatever the payload
/// holds; any other property is a (dot-nested) path in the payload.
fn element_property(element: &BoundElement, property: &str) -> Option<serde_json::Value> {
    if property == "id" {
        return Some(serde_json::json!(element.id));
    }
    let payload = element.payload.as_ref()?;
    crate::filter::get_nested_field(payload, property).cloned()
}

/// Core's `project_all_node_properties`: every payload property but `id`
/// under `alias.key`, then the node id under `alias.id`.
fn project_all(node: &BoundElement, projected: &mut HashMap<String, serde_json::Value>) {
    let Some(alias) = node.alias.as_deref() else {
        return;
    };
    if let Some(serde_json::Value::Object(map)) = &node.payload {
        for (key, value) in map.iter().filter(|(key, _)| key.as_str() != "id") {
            projected.insert(format!("{alias}.{key}"), value.clone());
        }
    }
    projected.insert(format!("{alias}.id"), serde_json::json!(node.id));
}

/// Core's `projection_key`: the item's `AS` alias, else its expression.
fn projection_key(item: &ReturnItem) -> String {
    item.alias
        .clone()
        .unwrap_or_else(|| item.expression.clone())
}

#[cfg(test)]
#[path = "velesql_match_core_tests.rs"]
mod tests;
