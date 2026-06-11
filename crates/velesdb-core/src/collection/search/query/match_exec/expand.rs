//! Recursive pattern-walker for MATCH traversal (EPIC-045 US-002).
//!
//! Split out of `match_exec/mod.rs` to keep each file under the 500 NLOC bar.
//! Holds the per-relationship expansion engine: relationship ordering,
//! direction, multi-type, multi-hop / variable-length, and binding acceptance.

use super::{AliasBinding, MatchResult, TraversalCtx};
use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::velesql::{Direction, GraphPattern, RelationshipPattern};
use std::collections::HashMap;

/// Ambient state threaded through a single pattern walk.
///
/// Bundling the invariant pattern/edge-store references with the mutable
/// traversal context, bindings, and path keeps the recursive walker functions
/// well under the argument-count limit (they previously took 12 parameters).
struct Walk<'a, 't> {
    pattern: &'a GraphPattern,
    edge_store: &'a crate::collection::graph::ConcurrentEdgeStore,
    ctx: &'a mut TraversalCtx<'t>,
    bindings: &'a mut HashMap<String, u64>,
    /// Bound relationship aliases (alias -> traversed edge id).
    edge_bindings: &'a mut HashMap<String, u64>,
    /// Variable-length relationship aliases (alias -> ordered edge-id list).
    edge_paths: &'a mut HashMap<String, Vec<u64>>,
    path: &'a mut Vec<u64>,
}

/// Backtracking record for an edge-alias binding made by `bind_edge_alias`.
enum EdgeAliasSave {
    /// The relationship has no alias — nothing to restore.
    None,
    /// Fixed-length alias: restore the previous scalar binding (if any).
    Scalar(String, Option<u64>),
    /// Variable-length alias: pop the edge id pushed onto the alias's list.
    PathPushed(String),
}

impl Collection {
    /// Traverses a single graph pattern via BFS for each start node.
    pub(super) fn traverse_pattern(
        &self,
        pattern: &GraphPattern,
        start_nodes: &[(u64, HashMap<String, u64>)],
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
    ) -> Result<()> {
        for (start_id, start_bindings) in start_nodes {
            if ctx.all_results.len() >= ctx.limit {
                break;
            }

            let mut path = Vec::new();
            let mut bindings = start_bindings.clone();
            let mut edge_bindings = HashMap::new();
            let mut edge_paths = HashMap::new();
            let mut walk = Walk {
                pattern,
                edge_store,
                ctx: &mut *ctx,
                bindings: &mut bindings,
                edge_bindings: &mut edge_bindings,
                edge_paths: &mut edge_paths,
                path: &mut path,
            };
            self.expand_pattern(&mut walk, *start_id, 0)?;
        }
        Ok(())
    }

    fn expand_pattern(
        &self,
        walk: &mut Walk<'_, '_>,
        current_id: u64,
        rel_idx: usize,
    ) -> Result<()> {
        if rel_idx >= walk.pattern.relationships.len() {
            return self.accept_pattern_match(walk, current_id);
        }
        self.expand_relationship(walk, current_id, rel_idx, 0)
    }

    fn expand_relationship(
        &self,
        walk: &mut Walk<'_, '_>,
        current_id: u64,
        rel_idx: usize,
        hops: u32,
    ) -> Result<()> {
        let (min_hops, max_hops) = walk.pattern.relationships[rel_idx].range.unwrap_or((1, 1));
        if hops >= min_hops {
            self.try_bind_next_node(walk, current_id, rel_idx)?;
        }
        if hops >= max_hops || walk.ctx.all_results.len() >= walk.ctx.limit {
            return Ok(());
        }
        let edges = Self::matching_edges(
            walk.edge_store,
            current_id,
            &walk.pattern.relationships[rel_idx],
        );
        for edge in edges {
            self.follow_edge(walk, current_id, &edge, rel_idx, hops)?;
        }
        Ok(())
    }

    fn follow_edge(
        &self,
        walk: &mut Walk<'_, '_>,
        current_id: u64,
        edge: &GraphEdge,
        rel_idx: usize,
        hops: u32,
    ) -> Result<()> {
        // Relationship isomorphism (Cypher semantics): an edge may be
        // traversed at most once per matched path. `walk.path` is exactly
        // the in-progress path's edge list (push/pop backtracking below),
        // so a linear scan gives per-path visited-edge tracking with no
        // extra state; paths are short (bounded by the pattern's max hops
        // and the depth guard-rail).
        if walk.ctx.all_results.len() >= walk.ctx.limit
            || walk.path.contains(&edge.id())
            || !Self::edge_matches(edge, &walk.pattern.relationships[rel_idx])
        {
            return Ok(());
        }
        let next_id = Self::edge_next_node(edge, current_id);
        walk.path.push(edge.id());
        let saved_alias = Self::bind_edge_alias(walk, rel_idx, edge.id());
        *walk.ctx.iteration_count += 1;
        let depth = walk.path.len() as u32;
        self.check_depth_and_periodic_guardrails(depth, walk.ctx)?;
        self.expand_relationship(walk, next_id, rel_idx, hops.saturating_add(1))?;
        Self::restore_edge_alias(walk, saved_alias);
        walk.path.pop();
        Ok(())
    }

    /// Binds the relationship alias (if any) to the traversed edge so
    /// `WHERE r.prop` / `RETURN r.prop` resolve against the edge.
    ///
    /// Fixed-length aliases bind a single edge id. Variable-length aliases
    /// (`[r*1..3]`) follow openCypher list semantics: each traversed hop is
    /// appended to the alias's ordered edge-id list.
    fn bind_edge_alias(walk: &mut Walk<'_, '_>, rel_idx: usize, edge_id: u64) -> EdgeAliasSave {
        let rel = &walk.pattern.relationships[rel_idx];
        let Some(alias) = rel.alias.clone() else {
            return EdgeAliasSave::None;
        };
        if rel.range.is_some() {
            walk.edge_paths
                .entry(alias.clone())
                .or_default()
                .push(edge_id);
            return EdgeAliasSave::PathPushed(alias);
        }
        let previous = walk.edge_bindings.insert(alias.clone(), edge_id);
        EdgeAliasSave::Scalar(alias, previous)
    }

    /// Restores a relationship alias binding saved by [`Self::bind_edge_alias`].
    fn restore_edge_alias(walk: &mut Walk<'_, '_>, saved: EdgeAliasSave) {
        match saved {
            EdgeAliasSave::None => {}
            EdgeAliasSave::Scalar(alias, Some(edge_id)) => {
                walk.edge_bindings.insert(alias, edge_id);
            }
            EdgeAliasSave::Scalar(alias, None) => {
                walk.edge_bindings.remove(&alias);
            }
            EdgeAliasSave::PathPushed(alias) => {
                if let Some(list) = walk.edge_paths.get_mut(&alias) {
                    list.pop();
                    if list.is_empty() {
                        walk.edge_paths.remove(&alias);
                    }
                }
            }
        }
    }

    fn try_bind_next_node(
        &self,
        walk: &mut Walk<'_, '_>,
        node_id: u64,
        rel_idx: usize,
    ) -> Result<()> {
        let Some(node) = walk.pattern.nodes.get(rel_idx + 1) else {
            return Ok(());
        };
        if !Self::node_matches_bound_pattern(node_id, node, walk.ctx.payload_guard) {
            return Ok(());
        }
        let inserted = Self::bind_node_alias(walk.bindings, node, node_id);
        let (AliasBinding::Unchanged | AliasBinding::Inserted(_)) = inserted else {
            return Ok(());
        };
        self.expand_pattern(walk, node_id, rel_idx + 1)?;
        if let AliasBinding::Inserted(alias) = inserted {
            walk.bindings.remove(&alias);
        }
        Ok(())
    }

    fn accept_pattern_match(&self, walk: &mut Walk<'_, '_>, node_id: u64) -> Result<()> {
        if let Some(where_clause) = walk.ctx.match_clause.where_clause.as_ref() {
            if !self.evaluate_where_condition(
                node_id,
                Some(&*walk.bindings),
                super::where_eval::EdgeAliasBindings {
                    scalar: Some(&*walk.edge_bindings),
                    paths: Some(&*walk.edge_paths),
                },
                where_clause,
                walk.ctx.params,
                walk.ctx.payload_guard,
            )? {
                return Ok(());
            }
        }

        let signature = Self::binding_signature(walk.bindings, walk.edge_bindings, walk.edge_paths);
        if !walk.ctx.seen_bindings.insert(signature) {
            return Ok(());
        }

        let mut result = MatchResult::new(node_id, walk.path.len() as u32, walk.path.clone());
        result.bindings.clone_from(walk.bindings);
        result.edge_bindings.clone_from(walk.edge_bindings);
        result.edge_paths.clone_from(walk.edge_paths);
        result.projected = self.project_properties(
            walk.bindings,
            walk.edge_bindings,
            walk.edge_paths,
            &walk.ctx.match_clause.return_clause,
            walk.ctx.payload_guard,
        );
        walk.ctx.all_results.push(result);
        Ok(())
    }

    fn check_depth_and_periodic_guardrails(
        &self,
        depth: u32,
        ctx: &mut TraversalCtx<'_>,
    ) -> Result<()> {
        if let Some(guardrail) = ctx.guardrail {
            guardrail
                .check_depth(depth)
                .map_err(|e| Error::GuardRail(e.to_string()))?;
        }
        self.check_periodic_guardrails(
            ctx.guardrail,
            *ctx.iteration_count,
            ctx.all_results,
            ctx.reported_cardinality,
        )
    }

    fn matching_edges(
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        node_id: u64,
        rel: &RelationshipPattern,
    ) -> Vec<GraphEdge> {
        match rel.direction {
            Direction::Outgoing => Self::typed_edges(edge_store, node_id, rel, true),
            Direction::Incoming => Self::typed_edges(edge_store, node_id, rel, false),
            Direction::Both => {
                let mut edges = Self::typed_edges(edge_store, node_id, rel, true);
                edges.extend(Self::typed_edges(edge_store, node_id, rel, false));
                edges
            }
        }
    }

    fn typed_edges(
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        node_id: u64,
        rel: &RelationshipPattern,
        outgoing: bool,
    ) -> Vec<GraphEdge> {
        if rel.types.is_empty() {
            return if outgoing {
                edge_store.get_outgoing(node_id)
            } else {
                edge_store.get_incoming(node_id)
            };
        }
        rel.types
            .iter()
            .flat_map(|label| {
                if outgoing {
                    edge_store.get_outgoing_by_label(node_id, label)
                } else {
                    edge_store.get_incoming_by_label(node_id, label)
                }
            })
            .collect()
    }

    fn edge_matches(edge: &GraphEdge, rel: &RelationshipPattern) -> bool {
        if !rel.types.is_empty() && !rel.types.iter().any(|t| t == edge.label()) {
            return false;
        }
        rel.properties.iter().all(|(key, expected)| {
            edge.property(key)
                .is_some_and(|v| Self::values_match(expected, v))
        })
    }

    fn edge_next_node(edge: &GraphEdge, current_id: u64) -> u64 {
        if edge.source() == current_id {
            edge.target()
        } else {
            edge.source()
        }
    }

    fn bind_node_alias(
        bindings: &mut HashMap<String, u64>,
        node: &crate::velesql::NodePattern,
        node_id: u64,
    ) -> AliasBinding {
        let Some(alias) = node.alias.as_ref() else {
            return AliasBinding::Unchanged;
        };
        if let Some(existing) = bindings.get(alias) {
            return if *existing == node_id {
                AliasBinding::Unchanged
            } else {
                AliasBinding::Conflict
            };
        }
        bindings.insert(alias.clone(), node_id);
        AliasBinding::Inserted(alias.clone())
    }

    /// Dedup signature over node bindings, edge bindings, and edge paths.
    ///
    /// Edge bindings participate so parallel edges between the same node
    /// pair yield distinct rows when the relationship is aliased (audit
    /// 2026-06: parallel edges previously collapsed to one row). Prefixes
    /// keep node and edge aliases from colliding in the flat key space.
    fn binding_signature(
        bindings: &HashMap<String, u64>,
        edge_bindings: &HashMap<String, u64>,
        edge_paths: &HashMap<String, Vec<u64>>,
    ) -> Vec<(String, u64)> {
        let mut signature: Vec<(String, u64)> = bindings
            .iter()
            .map(|(k, v)| (format!("n:{k}"), *v))
            .collect();
        signature.extend(edge_bindings.iter().map(|(k, v)| (format!("e:{k}"), *v)));
        for (alias, edge_ids) in edge_paths {
            signature.extend(
                edge_ids
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (format!("p:{alias}:{i}"), *id)),
            );
        }
        signature.sort_by(|a, b| a.0.cmp(&b.0));
        signature
    }
}
