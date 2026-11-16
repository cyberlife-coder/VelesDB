//! MATCH (graph pattern matching) execution for the WASM executor (S4-13).
//!
//! Split out of `velesql_graph.rs` to keep each module under the 500 NLOC
//! cap. A single pattern of any fixed length is expanded depth-first, one
//! relationship per step, with core's relationship-isomorphism rule: an edge
//! is traversed at most once per matched path. Multi-pattern MATCH and
//! variable-length relationships (`[*1..3]`) are rejected with a clear
//! message.

use velesdb_core::velesql::{
    Condition, Direction, GraphPattern, NodePattern, Query, RelationshipPattern,
};

use crate::database::DatabaseInner;
use crate::graph_store::{WasmEdge, WasmGraphNode, WasmGraphStore};
use crate::velesql_match_nodes::MatchNodes;
use crate::velesql_match_orderby::{
    order_and_limit, BoundElement, MatchBinding, MatchCandidate, MatchOrder,
};
use crate::velesql_match_score::{where_holds, where_score, BoundNode, MatchScoring};
use crate::velesql_match_sources::Annotations;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Runs a MATCH and returns its rows as ordered, limited candidates — the
/// alias-keyed row together with what it binds ([`MatchBinding`]). Callers
/// go through [`crate::velesql_graph::match_candidates_gated`], which puts
/// the read gate in front.
///
/// Only single-pattern MATCH is supported in the WASM executor. A query
/// with multiple comma-separated patterns (e.g.
/// `MATCH (a:X), (b:Y) RETURN a, b`) surfaces a clear error so callers
/// know to use the persistent core backend (Devin Review Finding K) —
/// rather than silently dropping all patterns after the first.
pub(crate) fn match_candidates(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
    scoring: Option<&MatchScoring>,
) -> Result<Vec<MatchCandidate>, String> {
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
    // Inline pattern properties take the caller's `$name` values, as in core.
    let clause =
        &velesdb_core::velesql::bind_match_clause(clause, params).map_err(|e| e.to_string())?;
    let pattern = &clause.patterns[0];
    validate_pattern(pattern)?;
    let annotations = Annotations::resolve(db, clause)?;
    let (name, store) = inferred_graph_store(db, params)?;
    let collection = db.get_shared_store(&name).ok();
    let graph = store.borrow();
    let points = collection.as_ref().map(|c| c.borrow());
    let nodes = MatchNodes::new(&graph, points.as_deref());
    if let Some(scoring) = scoring {
        scoring.validate(&nodes)?;
    }
    let walk = PathWalk {
        pattern,
        where_clause: clause.where_clause.as_ref(),
        params,
        annotations,
        scoring,
        seen: std::cell::RefCell::default(),
    };
    let mut out = Vec::new();
    let mut path = PathState::default();
    // The label index narrows the start nodes; every pattern check is the
    // full one ([`PathWalk::node_matches`]).
    let first_label = pattern.nodes[0].labels.first().map(String::as_str);
    for start in nodes.candidates(first_label) {
        walk.start_from(&nodes, start, &mut path, &mut out)?;
    }
    let higher_is_better = nodes
        .vector_space()
        .is_none_or(|(_, metric)| metric.higher_is_better());
    if scoring.is_some() {
        // REST `/match` with a query vector: rows by score, before ORDER BY.
        sort_by_score(&mut out, higher_is_better);
    }
    let order = MatchOrder {
        higher_is_better,
        nodes: Some(&nodes),
        params,
    };
    order_and_limit(
        &mut out,
        clause.return_clause.order_by.as_deref(),
        clause.return_clause.limit,
        &order,
    )?;
    Ok(out)
}

/// Most similar first: descending score, ascending distance.
fn sort_by_score(candidates: &mut [MatchCandidate], higher_is_better: bool) {
    let key = |c: &MatchCandidate| match c.score {
        Some(score) if higher_is_better => score,
        Some(distance) => -distance,
        None => f32::NEG_INFINITY,
    };
    candidates.sort_by(|a, b| key(b).total_cmp(&key(a)));
}

/// Rejects pattern shapes this executor would otherwise misread.
fn validate_pattern(pattern: &GraphPattern) -> Result<(), String> {
    if pattern.nodes.is_empty() {
        return Err("MATCH pattern has no node".to_string());
    }
    if pattern.relationships.len() + 1 != pattern.nodes.len() {
        return Err(format!(
            "MATCH pattern with {} nodes must have {} relationships, got {}",
            pattern.nodes.len(),
            pattern.nodes.len() - 1,
            pattern.relationships.len()
        ));
    }
    if pattern.relationships.iter().any(|rel| rel.range.is_some()) {
        return Err(
            "Variable-length relationships (e.g. [*1..3]) are not yet supported in WASM. \
             Spell the hops out, or use core (persistence-enabled)."
                .to_string(),
        );
    }
    Ok(())
}

/// The immutable inputs of one MATCH pattern walk.
struct PathWalk<'p> {
    pattern: &'p GraphPattern,
    where_clause: Option<&'p Condition>,
    params: &'p Params,
    /// The collections the pattern's `@collection` annotations name.
    annotations: Annotations,
    /// REST `/match`'s query vector and threshold, when given.
    scoring: Option<&'p MatchScoring>,
    /// Signatures of the rows already emitted: one row per distinct binding
    /// of the named elements, as core dedups (`binding_signature`).
    seen: std::cell::RefCell<std::collections::HashSet<Vec<(u8, String, u64)>>>,
}

/// What scoring decides for a row.
enum RowScore {
    /// The row stays, with its score if it has one.
    Kept(Option<f32>),
    /// The row's last node cannot be scored against REST `/match`'s query
    /// vector, or falls short of its threshold.
    Dropped,
}

/// The path being built: bound node ids and the edges walked to reach them.
#[derive(Default)]
struct PathState {
    nodes: Vec<u64>,
    edges: Vec<u64>,
}

impl PathWalk<'_> {
    fn start_from(
        &self,
        nodes: &MatchNodes<'_>,
        start: u64,
        path: &mut PathState,
        out: &mut Vec<MatchCandidate>,
    ) -> Result<(), String> {
        // A lone node must exist to be returned; as a path anchor an id known
        // only through its edges still starts a walk, as it always has.
        if self.pattern.nodes.len() == 1 && nodes.get(start).is_none() {
            return Ok(());
        }
        if !self.node_matches(&self.pattern.nodes[0], start, nodes.get(start)) {
            return Ok(());
        }

        path.nodes.push(start);
        let result = self.extend(nodes, path, out);
        path.nodes.pop();
        result
    }

    /// Depth-first: binds the next pattern node through every compatible
    /// edge of the current one, or emits the row once all nodes are bound.
    fn extend(
        &self,
        nodes: &MatchNodes<'_>,
        path: &mut PathState,
        out: &mut Vec<MatchCandidate>,
    ) -> Result<(), String> {
        let depth = path.nodes.len();
        if depth == self.pattern.nodes.len() {
            return self.emit(nodes, path, out);
        }
        let anchor = path.nodes[depth - 1];
        let rel = &self.pattern.relationships[depth - 1];
        for edge in directed_filter_edges(nodes.graph(), anchor, rel) {
            // Relationship isomorphism (Cypher, and core's `follow_edge`).
            if path.edges.contains(&edge.id) {
                continue;
            }
            if !edge_matches(&edge, rel) {
                continue;
            }
            let next = other_endpoint(&edge, anchor);
            let Some(next_node) = nodes.get(next) else {
                continue;
            };
            if !self.node_matches(&self.pattern.nodes[depth], next, Some(next_node))
                || !self.alias_consistent(path, depth, next)
            {
                continue;
            }
            path.nodes.push(next);
            path.edges.push(edge.id);
            let result = self.extend(nodes, path, out);
            path.edges.pop();
            path.nodes.pop();
            result?;
        }
        Ok(())
    }

    /// Whether node `id` (`data`, when a store holds it) satisfies the
    /// pattern node `node`, as core decides it: every label, and every
    /// inline property read on the node's annotated properties.
    fn node_matches(&self, node: &NodePattern, id: u64, data: Option<&WasmGraphNode>) -> bool {
        if node.labels.is_empty() && node.properties.is_empty() {
            return true;
        }
        let Some(data) = data else {
            return false;
        };
        if !velesdb_core::velesql::has_every_label(&node.labels, &data.labels) {
            return false;
        }
        if node.properties.is_empty() {
            return true;
        }
        let properties = self.annotations.properties(node, id, data.payload.as_ref());
        velesdb_core::velesql::inline_properties_hold(&node.properties, |key| {
            velesdb_core::velesql::element_property(id, key, |k| properties.as_ref()?.get(k))
        })
    }

    /// Whether binding node `id` at `position` keeps every alias naming one
    /// node: an alias repeated in the pattern (`(a)-[:R]->(b)-[:R]->(a)`)
    /// requires a cycle, as in core (`bind_node_alias`).
    fn alias_consistent(&self, path: &PathState, position: usize, id: u64) -> bool {
        let Some(alias) = self.pattern.nodes[position].alias.as_deref() else {
            return true;
        };
        self.pattern.nodes[..position]
            .iter()
            .zip(&path.nodes)
            .all(|(node, &bound)| node.alias.as_deref() != Some(alias) || bound == id)
    }

    /// Evaluates WHERE with every element bound, keeps one row per distinct
    /// binding of the named elements, then builds the row.
    fn emit(
        &self,
        nodes: &MatchNodes<'_>,
        path: &PathState,
        out: &mut Vec<MatchCandidate>,
    ) -> Result<(), String> {
        // Each bound node as every clause reads it: its labels, and its
        // properties united with its annotated collection's point.
        let bound: Vec<(&NodePattern, u64, WasmGraphNode)> = self
            .pattern
            .nodes
            .iter()
            .zip(&path.nodes)
            .map(|(pattern, &id)| {
                let data = nodes.get(id);
                let node = WasmGraphNode {
                    payload: self.annotations.properties(
                        pattern,
                        id,
                        data.and_then(|d| d.payload.as_ref()),
                    ),
                    labels: data.map(|d| d.labels.clone()).unwrap_or_default(),
                };
                (pattern, id, node)
            })
            .collect();
        let edges = self.bound_edges(nodes, path);
        let scope: Vec<BoundNode<'_>> = bound
            .iter()
            .map(|(pattern, id, _)| BoundNode {
                alias: pattern.alias.as_deref(),
                id: *id,
            })
            .collect();
        if !self.where_holds(nodes, &bound, &edges, &scope)? {
            return Ok(());
        }
        if !self.seen.borrow_mut().insert(row_signature(&bound, &edges)) {
            return Ok(());
        }
        let RowScore::Kept(score) = self.row_score(nodes, path, &scope)? else {
            return Ok(());
        };
        out.push(build_match_row(&bound, edges, score)?);
        Ok(())
    }

    /// The row's score: REST `/match`'s query vector on the row's last node
    /// when one is given, else the first WHERE `similarity()` leaf's.
    fn row_score(
        &self,
        nodes: &MatchNodes<'_>,
        path: &PathState,
        scope: &[BoundNode<'_>],
    ) -> Result<RowScore, String> {
        let Some(scoring) = self.scoring else {
            return where_score(self.where_clause, scope, nodes, self.params).map(RowScore::Kept);
        };
        let last = *path.nodes.last().ok_or("MATCH row binds no node")?;
        Ok(scoring
            .score(nodes, last)
            .map_or(RowScore::Dropped, |score| RowScore::Kept(Some(score))))
    }

    /// The row's relationships, each with its alias and properties.
    fn bound_edges(&self, nodes: &MatchNodes<'_>, path: &PathState) -> Vec<BoundElement> {
        path.edges
            .iter()
            .zip(&self.pattern.relationships)
            .map(|(&id, rel)| BoundElement {
                alias: rel.alias.clone(),
                id,
                payload: nodes
                    .graph()
                    .edges()
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.payload.clone()),
            })
            .collect()
    }

    /// The WHERE clause over this row: `similarity()` leaves scored on node
    /// vectors, every other leaf on [`where_scope`]'s payload.
    fn where_holds(
        &self,
        nodes: &MatchNodes<'_>,
        bound: &[(&NodePattern, u64, WasmGraphNode)],
        edges: &[BoundElement],
        scope: &[BoundNode<'_>],
    ) -> Result<bool, String> {
        let Some(cond) = self.where_clause else {
            return Ok(true);
        };
        let Some((_, target, _)) = bound.last() else {
            // Defensive: a MATCH always has at least one bound node.
            return Ok(false);
        };
        let payload = where_scope(bound, edges);
        where_holds(cond, scope, nodes, self.params, &|leaf| {
            crate::velesql_where::truth(leaf, *target, Some(&payload), self.params)
        })
    }
}

/// Returns edges incident to `anchor` in the relationship's direction; its
/// types and inline properties are [`edge_matches`]'s to decide.
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
    match rel.direction {
        Direction::Outgoing => store
            .filter_edges(Some(anchor), None, None)
            .cloned()
            .collect(),
        Direction::Incoming => store
            .filter_edges(None, Some(anchor), None)
            .cloned()
            .collect(),
        Direction::Both => {
            let mut edges: Vec<WasmEdge> = store
                .filter_edges(Some(anchor), None, None)
                .cloned()
                .collect();
            let mut seen: std::collections::HashSet<u64> = edges.iter().map(|e| e.id).collect();
            for e in store.filter_edges(None, Some(anchor), None) {
                if seen.insert(e.id) {
                    edges.push(e.clone());
                }
            }
            edges
        }
        _ => store
            .filter_edges(Some(anchor), None, None)
            .cloned()
            .collect(),
    }
}

fn other_endpoint(edge: &WasmEdge, anchor: u64) -> u64 {
    if edge.source == anchor {
        edge.target
    } else {
        edge.source
    }
}

/// Whether `edge` satisfies the relationship pattern `rel`, as core's
/// `edge_matches` decides it: one of its types (`[:A|B]`), and every inline
/// property read on the edge's properties.
fn edge_matches(edge: &WasmEdge, rel: &RelationshipPattern) -> bool {
    if !rel.types.is_empty() && !rel.types.contains(&edge.label) {
        return false;
    }
    velesdb_core::velesql::inline_properties_hold(&rel.properties, |key| {
        velesdb_core::velesql::element_property(edge.id, key, |k| edge.payload.as_ref()?.get(k))
    })
}

/// The collection (graph store name) a MATCH reads: the `_collection`
/// param — how core and the server name a MATCH target, since MATCH has no
/// `FROM` — else the default `graph` store. A node's `@collection`
/// annotation never selects it: it adds that collection's properties to the
/// node ([`Annotations`]), as in core. The read gate and the executor both
/// resolve the target here, so a policy is always asked about the graph
/// actually read.
///
/// # Errors
///
/// Returns an error when `_collection` is present but not a string.
pub(crate) fn match_target(params: &Params) -> Result<String, String> {
    match params.get(MATCH_TARGET_PARAM) {
        None => Ok(DEFAULT_GRAPH.to_string()),
        Some(serde_json::Value::String(name)) => Ok(name.clone()),
        Some(other) => Err(format!(
            "MATCH param '{MATCH_TARGET_PARAM}' must be a collection name (string), got {other}"
        )),
    }
}

/// Resolves the graph store a MATCH runs against (see [`match_target`]),
/// returned with its name: the collection of that name supplies the points
/// MATCH binds as nodes ([`MatchNodes`]).
fn inferred_graph_store(
    db: &mut DatabaseInner,
    params: &Params,
) -> Result<(String, std::rc::Rc<std::cell::RefCell<WasmGraphStore>>), String> {
    let name = match_target(params)?;
    // A collection with points and no edge yet still has nodes to bind; it
    // gets an empty, unregistered graph store for this MATCH.
    let store = match db.get_graph_store(&name) {
        Some(store) => store,
        None if db.contains(&name) => {
            std::rc::Rc::new(std::cell::RefCell::new(WasmGraphStore::new()))
        }
        None => return Err(format!("Graph '{name}' is empty; no data to match")),
    };
    Ok((name, store))
}

/// Param naming the collection a MATCH targets (core's `_collection`).
const MATCH_TARGET_PARAM: &str = "_collection";
/// Graph store a MATCH reads when nothing names one.
const DEFAULT_GRAPH: &str = "graph";

/// The payload a MATCH WHERE leaf reads, as core resolves its columns:
///
/// - `alias.prop` reads the element `alias` binds — a named node, or a named
///   relationship's properties (core's `edge_targets`);
/// - a bare column reads the walk's target, the last node bound (core's
///   `resolve_target_id`);
/// - `id` reads the element's id, whatever its payload holds.
///
/// Unnamed elements are not addressable. An alias wins a collision with a
/// target property of the same name.
fn where_scope(
    bound: &[(&NodePattern, u64, WasmGraphNode)],
    edges: &[BoundElement],
) -> serde_json::Value {
    let mut map = match bound.last() {
        Some((_, id, node)) => with_id(node.payload.as_ref(), *id),
        None => serde_json::Map::new(),
    };
    for (pattern, id, node) in bound {
        if let Some(alias) = &pattern.alias {
            map.insert(
                alias.clone(),
                serde_json::Value::Object(with_id(node.payload.as_ref(), *id)),
            );
        }
    }
    for edge in edges {
        if let Some(alias) = &edge.alias {
            map.insert(
                alias.clone(),
                serde_json::Value::Object(with_id(edge.payload.as_ref(), edge.id)),
            );
        }
    }
    serde_json::Value::Object(map)
}

/// The object of `payload` with `id` set to the element's id (a non-object
/// payload reads as empty).
fn with_id(
    payload: Option<&serde_json::Value>,
    id: u64,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = match payload {
        Some(serde_json::Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    map.insert("id".to_string(), serde_json::json!(id));
    map
}

/// What makes two rows the same row: the binding of every named node and
/// named relationship, as core's `binding_signature` keys its dedup.
fn row_signature(
    bound: &[(&NodePattern, u64, WasmGraphNode)],
    edges: &[BoundElement],
) -> Vec<(u8, String, u64)> {
    let mut signature: Vec<(u8, String, u64)> = bound
        .iter()
        .filter_map(|(pattern, id, _)| Some((0, pattern.alias.clone()?, *id)))
        .chain(
            edges
                .iter()
                .filter_map(|edge| Some((1, edge.alias.clone()?, edge.id))),
        )
        .collect();
    signature.sort_unstable();
    signature.dedup();
    signature
}

// --- Row builders --------------------------------------------------------

/// The row of a match, keyed as core orders and returns it:
///
/// - the alias-keyed JSON holds each *named* node as every clause read it
///   (its annotated properties included);
/// - the `executeQuery` row carries the walk's target as its id and, as its
///   score, the row's score or else `1 / (hops + 1)` (core's
///   `match_results_to_search_results`);
/// - the ORDER BY tie-break baseline is core's `(target id, hops, edge ids)`
///   (`sort_match_baseline`).
fn build_match_row(
    bound: &[(&NodePattern, u64, WasmGraphNode)],
    edges: Vec<BoundElement>,
    score: Option<f32>,
) -> Result<MatchCandidate, String> {
    let mut map = serde_json::Map::new();
    let mut nodes = Vec::with_capacity(bound.len());
    for (pattern, id, data) in bound {
        if let Some(alias) = &pattern.alias {
            map.insert(alias.clone(), node_json(*id, data));
        }
        nodes.push(BoundElement {
            alias: pattern.alias.clone(),
            id: *id,
            payload: data.payload.clone(),
        });
    }
    let target = bound.last().map_or(0, |(_, id, _)| *id);
    let hops = edges.len();
    let baseline: Vec<u64> = [target, hops as u64]
        .into_iter()
        .chain(edges.iter().map(|edge| edge.id))
        .collect();
    // A pattern's hop count fits a u16 exactly (and an f32 from it).
    let walk_score = 1.0 / (f32::from(u16::try_from(hops).unwrap_or(u16::MAX)) + 1.0);
    let value = serde_json::Value::Object(map);
    let row = QueryResultRow::synthetic(value.clone())?
        .with_id(target)
        .with_score(score.unwrap_or(walk_score));
    Ok(MatchCandidate {
        baseline,
        value,
        row,
        binding: MatchBinding { nodes, edges },
        score,
    })
}

/// Builds the JSON shape returned for a single MATCH-bound node.
///
/// Layout: payload fields are flattened at the alias root so the returned
/// object is symmetric with the MATCH WHERE scope (where `a.name` resolves
/// at the root of the alias — see [`where_scope`]). Without this
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

#[cfg(test)]
#[path = "velesql_match_path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "velesql_match_parity_tests.rs"]
mod parity_tests;

#[cfg(test)]
#[path = "velesql_match_points_tests.rs"]
mod points_tests;
