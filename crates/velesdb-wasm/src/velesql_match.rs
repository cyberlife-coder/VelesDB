//! MATCH (graph pattern matching) execution for the WASM executor (S4-13).
//!
//! Split out of `velesql_graph.rs` to keep each module under the 500 NLOC
//! cap. Handles 1- to 2-hop patterns; longer patterns are rejected with a
//! clear message.

use velesdb_core::velesql::{
    Condition, Direction, GraphPattern, MatchClause, NodePattern, Query, RelationshipPattern,
};

use crate::database::DatabaseInner;
use crate::graph_store::{WasmEdge, WasmGraphNode, WasmGraphStore};
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Executes a MATCH query.
pub(crate) fn execute_match(
    db: &mut DatabaseInner,
    query: &Query,
    _params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let Some(clause) = query.match_clause.as_ref() else {
        return Err("MATCH clause missing".to_string());
    };
    if clause.patterns.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = &clause.patterns[0];
    match pattern.nodes.len() {
        1 => execute_single_node(db, clause, pattern),
        2 => execute_1_hop(db, clause, pattern),
        3 => execute_2_hop(db, clause, pattern),
        _ => Err(format!(
            "MATCH patterns with more than 2 hops are not yet supported in WASM ({} nodes)",
            pattern.nodes.len()
        )),
    }
}

fn execute_single_node(
    db: &mut DatabaseInner,
    clause: &MatchClause,
    pattern: &GraphPattern,
) -> Result<Vec<QueryResultRow>, String> {
    let node = &pattern.nodes[0];
    let label = first_label(node);
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let candidates = borrowed.candidate_nodes(label.as_deref());
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    for nid in candidates {
        let Some(node_data) = borrowed.get_node(nid) else {
            continue;
        };
        if !node_payload_passes_where(node, nid, node_data, clause.where_clause.as_ref()) {
            continue;
        }
        if (out.len() as u64) >= limit {
            break;
        }
        out.push(build_match_row_single(node, nid, node_data)?);
    }
    Ok(out)
}

fn execute_1_hop(
    db: &mut DatabaseInner,
    clause: &MatchClause,
    pattern: &GraphPattern,
) -> Result<Vec<QueryResultRow>, String> {
    if pattern.relationships.len() != 1 {
        return Err(format!(
            "Expected 1 relationship for 1-hop pattern, got {}",
            pattern.relationships.len()
        ));
    }
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let ctx = OneHopContext {
        na: &pattern.nodes[0],
        nb: &pattern.nodes[1],
        rel: &pattern.relationships[0],
        la: first_label(&pattern.nodes[0]),
        lb: first_label(&pattern.nodes[1]),
    };
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    for sid in borrowed.candidate_nodes(ctx.la.as_deref()) {
        expand_one_hop(
            &borrowed,
            sid,
            &ctx,
            clause.where_clause.as_ref(),
            limit,
            &mut out,
        )?;
        if (out.len() as u64) >= limit {
            break;
        }
    }
    Ok(out)
}

struct OneHopContext<'p> {
    na: &'p NodePattern,
    nb: &'p NodePattern,
    rel: &'p RelationshipPattern,
    la: Option<String>,
    lb: Option<String>,
}

fn expand_one_hop(
    store: &WasmGraphStore,
    sid: u64,
    ctx: &OneHopContext<'_>,
    where_clause: Option<&Condition>,
    limit: u64,
    out: &mut Vec<QueryResultRow>,
) -> Result<(), String> {
    let default_node = WasmGraphNode::default();
    let a_node = store.get_node(sid).unwrap_or(&default_node);
    if !node_payload_passes_where(ctx.na, sid, a_node, where_clause) {
        return Ok(());
    }
    for edge in directed_filter_edges(store, sid, ctx.rel) {
        let other = other_endpoint(&edge, sid);
        let Some(b_node) = store.get_node(other) else {
            continue;
        };
        if !matches_label(b_node, ctx.lb.as_deref()) {
            continue;
        }
        if (out.len() as u64) >= limit {
            return Ok(());
        }
        out.push(build_match_row_pair(
            ctx.na, sid, a_node, ctx.nb, other, b_node,
        )?);
    }
    Ok(())
}

/// Returns edges incident to `anchor` that are compatible with the
/// relationship's direction and (optionally) its type filter.
///
/// - `Outgoing`: edges where `source == anchor`.
/// - `Incoming`: edges where `target == anchor`.
/// - `Both`: union of outgoing and incoming, deduplicated by edge id. A
///   self-loop (`source == target == anchor`) appears once.
///
/// The helper returns owned `WasmEdge` values so the caller can continue
/// borrowing the store mutably/immutably without lifetime contention.
fn directed_filter_edges(
    store: &WasmGraphStore,
    anchor: u64,
    rel: &RelationshipPattern,
) -> Vec<WasmEdge> {
    let label = first_type(rel);
    let label_ref = label.as_deref();
    match rel.direction {
        Direction::Outgoing => store
            .filter_edges(Some(anchor), None, label_ref)
            .cloned()
            .collect(),
        Direction::Incoming => store
            .filter_edges(None, Some(anchor), label_ref)
            .cloned()
            .collect(),
        Direction::Both => {
            let mut edges: Vec<WasmEdge> = store
                .filter_edges(Some(anchor), None, label_ref)
                .cloned()
                .collect();
            let mut seen: std::collections::HashSet<u64> = edges.iter().map(|e| e.id).collect();
            for e in store.filter_edges(None, Some(anchor), label_ref) {
                if seen.insert(e.id) {
                    edges.push(e.clone());
                }
            }
            edges
        }
        _ => store
            .filter_edges(Some(anchor), None, label_ref)
            .cloned()
            .collect(),
    }
}

fn execute_2_hop(
    db: &mut DatabaseInner,
    clause: &MatchClause,
    pattern: &GraphPattern,
) -> Result<Vec<QueryResultRow>, String> {
    if pattern.relationships.len() != 2 {
        return Err(format!(
            "Expected 2 relationships for 2-hop pattern, got {}",
            pattern.relationships.len()
        ));
    }
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let ctx = TwoHopContext::new(pattern);
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    for a_id in borrowed.candidate_nodes(ctx.la.as_deref()) {
        expand_from_a(&borrowed, a_id, &ctx, limit, &mut out)?;
        if (out.len() as u64) >= limit {
            break;
        }
    }
    Ok(out)
}

struct TwoHopContext<'p> {
    na: &'p NodePattern,
    nb: &'p NodePattern,
    nc: &'p NodePattern,
    la: Option<String>,
    lb: Option<String>,
    lc: Option<String>,
    r1: &'p RelationshipPattern,
    r2: &'p RelationshipPattern,
}

impl<'p> TwoHopContext<'p> {
    fn new(pattern: &'p GraphPattern) -> Self {
        Self {
            na: &pattern.nodes[0],
            nb: &pattern.nodes[1],
            nc: &pattern.nodes[2],
            la: first_label(&pattern.nodes[0]),
            lb: first_label(&pattern.nodes[1]),
            lc: first_label(&pattern.nodes[2]),
            r1: &pattern.relationships[0],
            r2: &pattern.relationships[1],
        }
    }
}

fn expand_from_a(
    store: &WasmGraphStore,
    a_id: u64,
    ctx: &TwoHopContext<'_>,
    limit: u64,
    out: &mut Vec<QueryResultRow>,
) -> Result<(), String> {
    for edge_ab in directed_filter_edges(store, a_id, ctx.r1) {
        let b_id = other_endpoint(&edge_ab, a_id);
        let Some(b_node) = store.get_node(b_id) else {
            continue;
        };
        if !matches_label(b_node, ctx.lb.as_deref()) {
            continue;
        }
        expand_from_b(store, a_id, b_id, b_node, ctx, limit, out)?;
        if (out.len() as u64) >= limit {
            return Ok(());
        }
    }
    Ok(())
}

fn expand_from_b(
    store: &WasmGraphStore,
    a_id: u64,
    b_id: u64,
    b_node: &WasmGraphNode,
    ctx: &TwoHopContext<'_>,
    limit: u64,
    out: &mut Vec<QueryResultRow>,
) -> Result<(), String> {
    let default_node = WasmGraphNode::default();
    let a_node = store.get_node(a_id).unwrap_or(&default_node);
    for edge_bc in directed_filter_edges(store, b_id, ctx.r2) {
        let c_id = other_endpoint(&edge_bc, b_id);
        let Some(c_node) = store.get_node(c_id) else {
            continue;
        };
        if !matches_label(c_node, ctx.lc.as_deref()) {
            continue;
        }
        if (out.len() as u64) >= limit {
            return Ok(());
        }
        out.push(build_match_row_triple(
            ctx.na, a_id, a_node, ctx.nb, b_id, b_node, ctx.nc, c_id, c_node,
        )?);
    }
    Ok(())
}

fn other_endpoint(edge: &WasmEdge, anchor: u64) -> u64 {
    if edge.source == anchor {
        edge.target
    } else {
        edge.source
    }
}

fn matches_label(node: &WasmGraphNode, label: Option<&str>) -> bool {
    let Some(l) = label else {
        return true;
    };
    node.labels.iter().any(|x| x == l)
}

fn inferred_graph_store(
    db: &mut DatabaseInner,
    pattern: &GraphPattern,
) -> Result<std::rc::Rc<std::cell::RefCell<WasmGraphStore>>, String> {
    let name = pattern
        .nodes
        .first()
        .and_then(|n| n.collection.clone())
        .unwrap_or_else(|| "graph".to_string());
    db.get_graph_store(&name)
        .ok_or_else(|| format!("Graph '{name}' is empty; no data to match"))
}

fn first_label(n: &NodePattern) -> Option<String> {
    n.labels.first().cloned()
}

fn first_type(r: &RelationshipPattern) -> Option<String> {
    r.types.first().cloned()
}

fn node_payload_passes_where(
    node: &NodePattern,
    id: u64,
    node_data: &WasmGraphNode,
    where_clause: Option<&Condition>,
) -> bool {
    let Some(cond) = where_clause else {
        return true;
    };
    let alias = node.alias.as_deref();
    let rewritten = rewrite_alias_prefix(cond, alias);
    let payload = node_data.payload.clone().unwrap_or(serde_json::Value::Null);
    crate::velesql_where::matches(&rewritten, id, Some(&payload), &Params::new()).unwrap_or(false)
}

/// Strips `alias.` prefixes on column references inside a WHERE condition.
fn rewrite_alias_prefix(cond: &Condition, alias: Option<&str>) -> Condition {
    let Some(alias) = alias else {
        return cond.clone();
    };
    let prefix = format!("{alias}.");
    match cond {
        Condition::Comparison(c) => {
            let mut nc = c.clone();
            if let Some(stripped) = nc.column.strip_prefix(&prefix) {
                nc.column = stripped.to_string();
            }
            Condition::Comparison(nc)
        }
        Condition::And(l, r) => Condition::And(
            Box::new(rewrite_alias_prefix(l, Some(alias))),
            Box::new(rewrite_alias_prefix(r, Some(alias))),
        ),
        Condition::Or(l, r) => Condition::Or(
            Box::new(rewrite_alias_prefix(l, Some(alias))),
            Box::new(rewrite_alias_prefix(r, Some(alias))),
        ),
        Condition::Not(inner) => Condition::Not(Box::new(rewrite_alias_prefix(inner, Some(alias)))),
        Condition::Group(inner) => {
            Condition::Group(Box::new(rewrite_alias_prefix(inner, Some(alias))))
        }
        other => other.clone(),
    }
}

// --- Row builders --------------------------------------------------------

fn build_match_row_single(
    node: &NodePattern,
    id: u64,
    data: &WasmGraphNode,
) -> Result<QueryResultRow, String> {
    let alias = node.alias.clone().unwrap_or_else(|| "a".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias, node_json(id, data));
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

fn build_match_row_pair(
    na: &NodePattern,
    a_id: u64,
    a_data: &WasmGraphNode,
    nb: &NodePattern,
    b_id: u64,
    b_data: &WasmGraphNode,
) -> Result<QueryResultRow, String> {
    let alias_a = na.alias.clone().unwrap_or_else(|| "a".to_string());
    let alias_b = nb.alias.clone().unwrap_or_else(|| "b".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias_a, node_json(a_id, a_data));
    map.insert(alias_b, node_json(b_id, b_data));
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

#[allow(clippy::too_many_arguments)]
fn build_match_row_triple(
    na: &NodePattern,
    a_id: u64,
    a_data: &WasmGraphNode,
    nb: &NodePattern,
    b_id: u64,
    b_data: &WasmGraphNode,
    nc: &NodePattern,
    c_id: u64,
    c_data: &WasmGraphNode,
) -> Result<QueryResultRow, String> {
    let alias_a = na.alias.clone().unwrap_or_else(|| "a".to_string());
    let alias_b = nb.alias.clone().unwrap_or_else(|| "b".to_string());
    let alias_c = nc.alias.clone().unwrap_or_else(|| "c".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias_a, node_json(a_id, a_data));
    map.insert(alias_b, node_json(b_id, b_data));
    map.insert(alias_c, node_json(c_id, c_data));
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

fn node_json(id: u64, data: &WasmGraphNode) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "labels": data.labels.clone(),
        "payload": data.payload.clone().unwrap_or(serde_json::Value::Null),
    })
}
