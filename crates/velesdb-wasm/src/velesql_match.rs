//! MATCH (graph pattern matching) execution for the WASM executor (S4-13).
//!
//! Split out of `velesql_graph.rs` to keep each module under the 500 NLOC
//! cap. Handles 1- to 2-hop patterns; longer patterns are rejected with a
//! clear message.

use velesdb_core::velesql::{
    Condition, Direction, GraphPattern, MatchClause, NodePattern, Query, RelationshipPattern,
};

use crate::database::{DatabaseInner, SharedStore};
use crate::graph_store::{WasmEdge, WasmGraphNode, WasmGraphStore};
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// A resolved `@collection` cross-reference: an aliased node's alias paired
/// with the store of the collection it references for payload enrichment.
type CrossRef = (String, SharedStore);

/// Collects `@collection` cross-references from a MATCH clause.
///
/// Mirrors core's `enrich_match_results_cross_collection`: only nodes with
/// **both** an explicit alias and a collection annotation are enriched, and a
/// reference to a missing collection is silently skipped (the MATCH still
/// returns its graph results). The referenced collection's store is resolved
/// once here and reused for every result row.
///
/// WASM note: lacking a `FROM` clause, the executor also reuses the *first*
/// node's `@collection` annotation to pick the graph store (see
/// [`inferred_graph_store`]). Enrichment therefore applies to any annotated
/// node that names an existing **vector** collection — in practice the
/// non-anchor nodes (`b`, `c`); the anchor enriches only when a same-named
/// vector collection also exists, otherwise its annotation just selects the
/// graph and the lookup is skipped.
fn resolve_cross_refs(db: &DatabaseInner, clause: &MatchClause) -> Vec<CrossRef> {
    clause
        .patterns
        .iter()
        .flat_map(|p| p.nodes.iter())
        .filter_map(|n| {
            let alias = n.alias.clone()?;
            let coll = n.collection.as_deref()?;
            let store = db.get_shared_store(coll).ok()?;
            Some((alias, store))
        })
        .collect()
}

/// Merges cross-collection payloads into a freshly built MATCH row.
///
/// For each cross-reference, the node id is read back from the alias object's
/// `id` field (every row builder writes it), the referenced collection's
/// payload for that id is looked up, and its fields are merged into the alias
/// object. The graph node's own fields — including the reserved `id`/`labels` —
/// win on collision, so enrichment never overwrites graph identity.
fn enrich_row(map: &mut serde_json::Map<String, serde_json::Value>, cross: &[CrossRef]) {
    for (alias, store) in cross {
        let Some(id) = map
            .get(alias)
            .and_then(|v| v.get("id"))
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let borrowed = store.borrow();
        let Some(serde_json::Value::Object(cross_obj)) = borrowed.payload_for_id(id) else {
            continue;
        };
        let Some(serde_json::Value::Object(alias_obj)) = map.get_mut(alias) else {
            continue;
        };
        for (key, value) in cross_obj {
            alias_obj
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
}

/// Executes a MATCH query.
///
/// Only single-pattern MATCH is supported in the WASM executor. A query
/// with multiple comma-separated patterns (e.g.
/// `MATCH (a:X), (b:Y) RETURN a, b`) surfaces a clear error so callers
/// know to use the persistent core backend (Devin Review Finding K) —
/// rather than silently dropping all patterns after the first.
pub(crate) fn execute_match(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let Some(clause) = query.match_clause.as_ref() else {
        return Err("MATCH clause missing".to_string());
    };
    if clause.patterns.is_empty() {
        return Ok(Vec::new());
    }
    if clause.patterns.len() > 1 {
        return Err(format!(
            "Multi-pattern MATCH is not yet supported in WASM ({} patterns in this query). \
             Use a single pattern or use core (persistence-enabled) for multi-pattern MATCH.",
            clause.patterns.len()
        ));
    }
    let pattern = &clause.patterns[0];
    match pattern.nodes.len() {
        1 => execute_single_node(db, clause, pattern, params),
        2 => execute_1_hop(db, clause, pattern, params),
        3 => execute_2_hop(db, clause, pattern, params),
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
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let node = &pattern.nodes[0];
    let label = first_label(node);
    let cross = resolve_cross_refs(db, clause);
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let candidates = borrowed.candidate_nodes(label.as_deref());
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    for nid in candidates {
        let Some(node_data) = borrowed.get_node(nid) else {
            continue;
        };
        let bindings = [make_binding(node, nid, node_data, "a")];
        if !matches_where_in_match_scope(clause.where_clause.as_ref(), &bindings, params)? {
            continue;
        }
        if (out.len() as u64) >= limit {
            break;
        }
        out.push(build_match_row_single(node, nid, node_data, &cross)?);
    }
    Ok(out)
}

fn execute_1_hop(
    db: &mut DatabaseInner,
    clause: &MatchClause,
    pattern: &GraphPattern,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if pattern.relationships.len() != 1 {
        return Err(format!(
            "Expected 1 relationship for 1-hop pattern, got {}",
            pattern.relationships.len()
        ));
    }
    let cross = resolve_cross_refs(db, clause);
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let ctx = OneHopContext {
        na: &pattern.nodes[0],
        nb: &pattern.nodes[1],
        rel: &pattern.relationships[0],
        la: first_label(&pattern.nodes[0]),
        lb: first_label(&pattern.nodes[1]),
        cross,
    };
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    for sid in borrowed.candidate_nodes(ctx.la.as_deref()) {
        expand_one_hop(
            &borrowed,
            sid,
            &ctx,
            clause.where_clause.as_ref(),
            params,
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
    cross: Vec<CrossRef>,
}

#[allow(clippy::too_many_arguments)]
fn expand_one_hop(
    store: &WasmGraphStore,
    sid: u64,
    ctx: &OneHopContext<'_>,
    where_clause: Option<&Condition>,
    params: &Params,
    limit: u64,
    out: &mut Vec<QueryResultRow>,
) -> Result<(), String> {
    let default_node = WasmGraphNode::default();
    let a_node = store.get_node(sid).unwrap_or(&default_node);
    for edge in directed_filter_edges(store, sid, ctx.rel) {
        let other = other_endpoint(&edge, sid);
        let Some(b_node) = store.get_node(other) else {
            continue;
        };
        if !matches_label(b_node, ctx.lb.as_deref()) {
            continue;
        }
        // WHERE is evaluated with both aliases bound so predicates on `b`
        // (e.g. `WHERE b.name = 'Bob'`) work the same as on `a`.
        let bindings = [
            make_binding(ctx.na, sid, a_node, "a"),
            make_binding(ctx.nb, other, b_node, "b"),
        ];
        if !matches_where_in_match_scope(where_clause, &bindings, params)? {
            continue;
        }
        if (out.len() as u64) >= limit {
            return Ok(());
        }
        out.push(build_match_row_pair(ctx, sid, a_node, other, b_node)?);
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
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if pattern.relationships.len() != 2 {
        return Err(format!(
            "Expected 2 relationships for 2-hop pattern, got {}",
            pattern.relationships.len()
        ));
    }
    let cross = resolve_cross_refs(db, clause);
    let store = inferred_graph_store(db, pattern)?;
    let borrowed = store.borrow();
    let ctx = TwoHopContext::new(pattern, cross);
    let limit = clause.return_clause.limit.unwrap_or(u64::MAX);
    let where_clause = clause.where_clause.as_ref();
    let mut out = Vec::new();
    for a_id in borrowed.candidate_nodes(ctx.la.as_deref()) {
        expand_from_a(&borrowed, a_id, &ctx, where_clause, params, limit, &mut out)?;
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
    cross: Vec<CrossRef>,
}

impl<'p> TwoHopContext<'p> {
    fn new(pattern: &'p GraphPattern, cross: Vec<CrossRef>) -> Self {
        Self {
            na: &pattern.nodes[0],
            nb: &pattern.nodes[1],
            nc: &pattern.nodes[2],
            la: first_label(&pattern.nodes[0]),
            lb: first_label(&pattern.nodes[1]),
            lc: first_label(&pattern.nodes[2]),
            r1: &pattern.relationships[0],
            r2: &pattern.relationships[1],
            cross,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_from_a(
    store: &WasmGraphStore,
    a_id: u64,
    ctx: &TwoHopContext<'_>,
    where_clause: Option<&Condition>,
    params: &Params,
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
        expand_from_b(
            store,
            a_id,
            b_id,
            b_node,
            ctx,
            where_clause,
            params,
            limit,
            out,
        )?;
        if (out.len() as u64) >= limit {
            return Ok(());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn expand_from_b(
    store: &WasmGraphStore,
    a_id: u64,
    b_id: u64,
    b_node: &WasmGraphNode,
    ctx: &TwoHopContext<'_>,
    where_clause: Option<&Condition>,
    params: &Params,
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
        // WHERE evaluates with all three aliases bound; mid-/end-node
        // predicates (e.g. `WHERE b.age > 30`, `WHERE c.name = 'X'`)
        // resolve against the correct node.
        let bindings = [
            make_binding(ctx.na, a_id, a_node, "a"),
            make_binding(ctx.nb, b_id, b_node, "b"),
            make_binding(ctx.nc, c_id, c_node, "c"),
        ];
        if !matches_where_in_match_scope(where_clause, &bindings, params)? {
            continue;
        }
        if (out.len() as u64) >= limit {
            return Ok(());
        }
        out.push(build_match_row_triple(
            ctx, a_id, a_node, b_id, b_node, c_id, c_node,
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

/// One alias → (id, payload) binding used during WHERE evaluation.
///
/// At most 3 bindings exist (1-, 2-, or 3-node MATCH pattern). Owned
/// `String` alias avoids lifetime plumbing in the bindings slice; payload
/// is a cloned `serde_json::Value` since the matcher consumes references.
struct AliasBinding {
    alias: String,
    id: u64,
    payload: serde_json::Value,
}

/// Evaluates a MATCH WHERE clause in a scope where multiple node aliases
/// are bound.
///
/// Strategy: build a merged JSON payload keyed by alias
/// (`{"a": <a_payload>, "b": <b_payload>, ...}`) and reuse
/// [`crate::velesql_where::matches`]. Dotted column references like
/// `b.name` resolve naturally via `get_nested_field` (which splits on
/// `.`). Bare column references (no alias prefix) fall back to the
/// first-bound node — backward compatible with pre-fix tests that write
/// `WHERE name = ...` without a prefix.
///
/// An unbound alias prefix (`z.x` with no binding `z`) silently yields
/// `false` via the same "missing column" semantics that
/// `velesql_where::matches` already applies to missing payload fields.
///
/// `params` is the query-level parameter map, threaded through the MATCH
/// executor so `$param` placeholders inside the WHERE clause resolve to
/// their bound JSON value. An unbound `$param` surfaces as an `Err` from
/// the inner matcher and is propagated up to the caller of
/// `execute_match` (no silent zero-row result).
fn matches_where_in_match_scope(
    where_clause: Option<&Condition>,
    bindings: &[AliasBinding],
    params: &Params,
) -> Result<bool, String> {
    let Some(cond) = where_clause else {
        return Ok(true);
    };
    let Some(first) = bindings.first() else {
        // Defensive: a MATCH always has at least one bound node.
        return Ok(false);
    };
    let merged = build_merged_payload(bindings, &first.payload, first.id);
    crate::velesql_where::matches(cond, first.id, Some(&merged), params)
}

/// Builds a flat JSON object suitable for alias-scoped WHERE evaluation.
///
/// Layout:
/// - copies all top-level fields of the first-bound node at the root
///   (enables bare `WHERE name = ...` to resolve against node `a`);
/// - inserts each bound alias as a top-level key (enables
///   `WHERE b.name = ...` to resolve via `get_nested_field` walking
///   `b` → `name`). Aliases are inserted AFTER the first-node fields
///   so an alias name always wins a collision.
///
/// Node-id injection: for every binding, the node `id` is injected as
/// an `id` field inside its alias object so predicates like
/// `WHERE a.id = 1` / `WHERE b.id = $x` resolve correctly. The bare
/// root gets the first binding's id too, so `WHERE id = 1` (no alias
/// prefix, backward-compatible) still matches the starting node. Node
/// id always wins a collision with a payload field of the same name —
/// a MATCH WHERE targets the graph node identifier, not an arbitrary
/// user-defined "id" field.
fn build_merged_payload(
    bindings: &[AliasBinding],
    first_payload: &serde_json::Value,
    first_id: u64,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(obj) = first_payload.as_object() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    // Bare `id` at the root resolves to the starting node's id (node
    // id wins over any payload field called "id").
    map.insert("id".to_string(), serde_json::json!(first_id));
    for b in bindings {
        map.insert(b.alias.clone(), payload_with_id(&b.payload, b.id));
    }
    serde_json::Value::Object(map)
}

/// Clones `payload`, injecting the node `id` as a top-level field.
///
/// - `Null` / non-object payload → `{"id": <id>}`.
/// - Object payload → same object with `"id"` set to the node id. Any
///   pre-existing `"id"` field in the user payload is overwritten: the
///   graph node identifier is the semantic anchor of a MATCH WHERE,
///   never a coincidentally-named payload key.
fn payload_with_id(payload: &serde_json::Value, id: u64) -> serde_json::Value {
    let mut obj = match payload {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    obj.insert("id".to_string(), serde_json::json!(id));
    serde_json::Value::Object(obj)
}

/// Builds a single binding from a `(NodePattern, id, data)` triple.
///
/// Falls back to alias `a`/`b`/`c` by position when the pattern omits an
/// explicit alias — matches the convention already used by the row
/// builders.
fn make_binding(node: &NodePattern, id: u64, data: &WasmGraphNode, fallback: &str) -> AliasBinding {
    let alias = node.alias.clone().unwrap_or_else(|| fallback.to_string());
    let payload = data.payload.clone().unwrap_or(serde_json::Value::Null);
    AliasBinding { alias, id, payload }
}

// --- Row builders --------------------------------------------------------

fn build_match_row_single(
    node: &NodePattern,
    id: u64,
    data: &WasmGraphNode,
    cross: &[CrossRef],
) -> Result<QueryResultRow, String> {
    let alias = node.alias.clone().unwrap_or_else(|| "a".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias, node_json(id, data));
    enrich_row(&mut map, cross);
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

fn build_match_row_pair(
    ctx: &OneHopContext<'_>,
    a_id: u64,
    a_data: &WasmGraphNode,
    b_id: u64,
    b_data: &WasmGraphNode,
) -> Result<QueryResultRow, String> {
    let alias_a = ctx.na.alias.clone().unwrap_or_else(|| "a".to_string());
    let alias_b = ctx.nb.alias.clone().unwrap_or_else(|| "b".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias_a, node_json(a_id, a_data));
    map.insert(alias_b, node_json(b_id, b_data));
    enrich_row(&mut map, &ctx.cross);
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

fn build_match_row_triple(
    ctx: &TwoHopContext<'_>,
    a_id: u64,
    a_data: &WasmGraphNode,
    b_id: u64,
    b_data: &WasmGraphNode,
    c_id: u64,
    c_data: &WasmGraphNode,
) -> Result<QueryResultRow, String> {
    let alias_a = ctx.na.alias.clone().unwrap_or_else(|| "a".to_string());
    let alias_b = ctx.nb.alias.clone().unwrap_or_else(|| "b".to_string());
    let alias_c = ctx.nc.alias.clone().unwrap_or_else(|| "c".to_string());
    let mut map = serde_json::Map::new();
    map.insert(alias_a, node_json(a_id, a_data));
    map.insert(alias_b, node_json(b_id, b_data));
    map.insert(alias_c, node_json(c_id, c_data));
    enrich_row(&mut map, &ctx.cross);
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

/// Builds the JSON shape returned for a single MATCH-bound node.
///
/// Layout: payload fields are flattened at the alias root so the returned
/// object is symmetric with the MATCH WHERE scope (where `a.name` resolves
/// at the root of the alias — see [`build_merged_payload`]). Without this
/// symmetry, `WHERE a.name = 'Alice'` filtered by root-level `name` but
/// the returned JSON required `a.payload.name` — a scope mismatch for JS
/// callers (Devin Review Finding O).
///
/// Collision rule: `id` and `labels` are reserved — the graph node id and
/// the node's own label list always win over any same-named payload key.
/// A MATCH targets the graph node identifier / label set, never a
/// coincidentally-named payload field.
fn node_json(id: u64, data: &WasmGraphNode) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(serde_json::Value::Object(obj)) = &data.payload {
        for (k, v) in obj {
            // Skip payload keys that would collide with reserved graph
            // identifiers; they are re-inserted authoritatively below.
            if k == "id" || k == "labels" {
                continue;
            }
            map.insert(k.clone(), v.clone());
        }
    }
    map.insert("id".to_string(), serde_json::json!(id));
    map.insert("labels".to_string(), serde_json::json!(data.labels.clone()));
    serde_json::Value::Object(map)
}
