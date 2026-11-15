//! MATCH (graph pattern matching) execution for the WASM executor (S4-13).
//!
//! Split out of `velesql_graph.rs` to keep each module under the 500 NLOC
//! cap. A single pattern of any fixed length is expanded depth-first, one
//! relationship per step, with core's relationship-isomorphism rule: an edge
//! is traversed at most once per matched path. Multi-pattern MATCH and
//! variable-length relationships (`[*1..3]`) are rejected with a clear
//! message.

use velesdb_core::velesql::{
    Condition, Direction, GraphPattern, MatchClause, NodePattern, Query, RelationshipPattern,
};

use crate::database::{DatabaseInner, SharedStore};
use crate::graph_store::{WasmEdge, WasmGraphNode, WasmGraphStore};
use crate::velesql_match_nodes::MatchNodes;
use crate::velesql_match_orderby::{order_and_limit, BoundElement, MatchBinding, MatchCandidate};
use crate::velesql_match_score::{where_holds, where_score, BoundNode, MatchScoring};
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
    let pattern = &clause.patterns[0];
    validate_pattern(pattern)?;
    let cross = resolve_cross_refs(db, clause);
    let (name, store) = inferred_graph_store(db, pattern, params)?;
    let collection = db.get_shared_store(&name).ok();
    let graph = store.borrow();
    let points = collection.as_ref().map(|c| c.borrow());
    let nodes = MatchNodes::new(&graph, points.as_deref());
    if let Some(scoring) = scoring {
        scoring.validate(&nodes)?;
    }
    let walk = PathWalk {
        pattern,
        labels: pattern.nodes.iter().map(first_label).collect(),
        where_clause: clause.where_clause.as_ref(),
        params,
        cross,
        scoring,
    };
    let mut out = Vec::new();
    let mut path = PathState::default();
    for start in nodes.candidates(walk.labels[0].as_deref()) {
        walk.start_from(&nodes, start, &mut path, &mut out)?;
    }
    let higher_is_better = nodes
        .vector_space()
        .is_none_or(|(_, metric)| metric.higher_is_better());
    if scoring.is_some() {
        // REST `/match` with a query vector: rows by score, before ORDER BY.
        sort_by_score(&mut out, higher_is_better);
    }
    finalize_candidates(out, clause, higher_is_better)
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
    /// First label of each pattern node, by position (`None` = any).
    labels: Vec<Option<String>>,
    where_clause: Option<&'p Condition>,
    params: &'p Params,
    cross: Vec<CrossRef>,
    /// REST `/match`'s query vector and threshold, when given.
    scoring: Option<&'p MatchScoring>,
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
            let next = other_endpoint(&edge, anchor);
            let Some(next_node) = nodes.get(next) else {
                continue;
            };
            if !matches_label(next_node, self.labels[depth].as_deref()) {
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

    /// Evaluates WHERE with every alias bound, then builds the row.
    fn emit(
        &self,
        nodes: &MatchNodes<'_>,
        path: &PathState,
        out: &mut Vec<MatchCandidate>,
    ) -> Result<(), String> {
        let default_node = WasmGraphNode::default();
        let bound: Vec<(&NodePattern, u64, &WasmGraphNode)> = self
            .pattern
            .nodes
            .iter()
            .zip(&path.nodes)
            .map(|(pattern, &id)| (pattern, id, nodes.get(id).unwrap_or(&default_node)))
            .collect();
        let bindings: Vec<AliasBinding> = bound
            .iter()
            .enumerate()
            .map(|(position, (pattern, id, data))| {
                make_binding(pattern, *id, data, &positional_alias(position))
            })
            .collect();
        if !self.where_holds(nodes, &bindings)? {
            return Ok(());
        }
        let bound_nodes: Vec<BoundNode<'_>> = bindings
            .iter()
            .map(|b| BoundNode {
                alias: &b.alias,
                id: b.id,
            })
            .collect();
        let score = match self.scoring {
            Some(scoring) => {
                let last = *path.nodes.last().ok_or("MATCH row binds no node")?;
                let Some(score) = scoring.score(nodes, last) else {
                    return Ok(());
                };
                Some(score)
            }
            None => where_score(self.where_clause, &bound_nodes, nodes, self.params)?,
        };
        let edges = path
            .edges
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
            .collect();
        let mut candidate = build_match_row(&bound, edges, &self.cross)?;
        candidate.score = score;
        out.push(candidate);
        Ok(())
    }

    /// The WHERE clause over this row: `similarity()` leaves scored on node
    /// vectors, every other leaf on the alias-scoped payload.
    fn where_holds(
        &self,
        nodes: &MatchNodes<'_>,
        bindings: &[AliasBinding],
    ) -> Result<bool, String> {
        let Some(cond) = self.where_clause else {
            return Ok(true);
        };
        let Some(first) = bindings.first() else {
            // Defensive: a MATCH always has at least one bound node.
            return Ok(false);
        };
        let merged = build_merged_payload(bindings, &first.payload, first.id);
        let bound: Vec<BoundNode<'_>> = bindings
            .iter()
            .map(|b| BoundNode {
                alias: &b.alias,
                id: b.id,
            })
            .collect();
        where_holds(cond, &bound, nodes, self.params, &|leaf| {
            crate::velesql_where::matches(leaf, first.id, Some(&merged), self.params)
        })
    }
}

/// The alias an unnamed pattern node gets: `a`, `b`, `c`… by position, then
/// `n26`, `n27`… past `z`.
fn positional_alias(position: usize) -> String {
    match u8::try_from(position) {
        Ok(offset) if offset < 26 => char::from(b'a' + offset).to_string(),
        _ => format!("n{position}"),
    }
}

/// Sorts the collected candidates per the RETURN `ORDER BY` (Finding 8: sort
/// BEFORE LIMIT), then truncates to LIMIT.
fn finalize_candidates(
    mut candidates: Vec<MatchCandidate>,
    clause: &MatchClause,
    higher_is_better: bool,
) -> Result<Vec<MatchCandidate>, String> {
    order_and_limit(
        &mut candidates,
        clause.return_clause.order_by.as_deref(),
        clause.return_clause.limit,
        higher_is_better,
    )?;
    Ok(candidates)
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

/// The collection (graph store name) a MATCH reads.
///
/// In order: the first node's `@collection` annotation (it names the node's
/// collection explicitly), then the `_collection` param — how core and the
/// server name a MATCH target, since MATCH has no `FROM` — then the default
/// `graph` store. The read gate and the executor both resolve the target
/// here, so a policy is always asked about the graph actually read.
///
/// # Errors
///
/// Returns an error when `_collection` is present but not a string.
pub(crate) fn match_target(pattern: &GraphPattern, params: &Params) -> Result<String, String> {
    if let Some(name) = pattern.nodes.first().and_then(|n| n.collection.clone()) {
        return Ok(name);
    }
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
    pattern: &GraphPattern,
    params: &Params,
) -> Result<(String, std::rc::Rc<std::cell::RefCell<WasmGraphStore>>), String> {
    let name = match_target(pattern, params)?;
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

fn first_label(n: &NodePattern) -> Option<String> {
    n.labels.first().cloned()
}

fn first_type(r: &RelationshipPattern) -> Option<String> {
    r.types.first().cloned()
}

/// One alias → (id, payload) binding used during WHERE evaluation.
///
/// One binding exists per pattern node. Owned
/// `String` alias avoids lifetime plumbing in the bindings slice; payload
/// is a cloned `serde_json::Value` since the matcher consumes references.
struct AliasBinding {
    alias: String,
    id: u64,
    payload: serde_json::Value,
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
/// Falls back to the [`positional_alias`] when the pattern omits an explicit
/// alias — the same convention [`build_match_row`] uses.
fn make_binding(node: &NodePattern, id: u64, data: &WasmGraphNode, fallback: &str) -> AliasBinding {
    let alias = node.alias.clone().unwrap_or_else(|| fallback.to_string());
    let payload = data.payload.clone().unwrap_or(serde_json::Value::Null);
    AliasBinding { alias, id, payload }
}

// --- Row builders --------------------------------------------------------

/// Builds a MATCH candidate (bound-node-id baseline + alias-keyed JSON + row)
/// so ORDER BY can sort the full set before LIMIT truncation (Finding 8).
/// `baseline` is the row's bound node ids in pattern order — a per-row-unique
/// deterministic tie-break, mirroring core keying on the full match identity.
fn candidate_from_map(
    baseline: Vec<u64>,
    map: serde_json::Map<String, serde_json::Value>,
    binding: MatchBinding,
) -> Result<MatchCandidate, String> {
    let value = serde_json::Value::Object(map);
    let row = QueryResultRow::synthetic(value.clone())?;
    Ok(MatchCandidate {
        baseline,
        value,
        row,
        binding,
        score: None,
    })
}

fn build_match_row(
    bound: &[(&NodePattern, u64, &WasmGraphNode)],
    edges: Vec<BoundElement>,
    cross: &[CrossRef],
) -> Result<MatchCandidate, String> {
    let mut map = serde_json::Map::new();
    for (position, (pattern, id, data)) in bound.iter().enumerate() {
        let alias = pattern
            .alias
            .clone()
            .unwrap_or_else(|| positional_alias(position));
        map.insert(alias, node_json(*id, data));
    }
    enrich_row(&mut map, cross);
    let binding = MatchBinding {
        nodes: bound
            .iter()
            .map(|(pattern, id, data)| BoundElement {
                alias: pattern.alias.clone(),
                id: *id,
                payload: data.payload.clone(),
            })
            .collect(),
        edges,
    };
    candidate_from_map(bound.iter().map(|(_, id, _)| *id).collect(), map, binding)
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

#[cfg(test)]
#[path = "velesql_match_path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "velesql_match_points_tests.rs"]
mod points_tests;
