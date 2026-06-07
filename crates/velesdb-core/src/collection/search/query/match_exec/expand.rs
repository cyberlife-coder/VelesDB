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
    path: &'a mut Vec<u64>,
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
            let mut walk = Walk {
                pattern,
                edge_store,
                ctx: &mut *ctx,
                bindings: &mut bindings,
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
        if walk.ctx.all_results.len() >= walk.ctx.limit
            || !Self::edge_matches(edge, &walk.pattern.relationships[rel_idx])
        {
            return Ok(());
        }
        let next_id = Self::edge_next_node(edge, current_id);
        walk.path.push(edge.id());
        *walk.ctx.iteration_count += 1;
        let depth = walk.path.len() as u32;
        self.check_depth_and_periodic_guardrails(depth, walk.ctx)?;
        self.expand_relationship(walk, next_id, rel_idx, hops.saturating_add(1))?;
        walk.path.pop();
        Ok(())
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
                where_clause,
                walk.ctx.params,
                walk.ctx.payload_guard,
            )? {
                return Ok(());
            }
        }

        let signature = Self::binding_signature(walk.bindings);
        if !walk.ctx.seen_bindings.insert(signature) {
            return Ok(());
        }

        let mut result = MatchResult::new(node_id, walk.path.len() as u32, walk.path.clone());
        result.bindings.clone_from(walk.bindings);
        result.projected = self.project_properties(
            walk.bindings,
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

    fn binding_signature(bindings: &HashMap<String, u64>) -> Vec<(String, u64)> {
        let mut signature: Vec<(String, u64)> =
            bindings.iter().map(|(k, v)| (k.clone(), *v)).collect();
        signature.sort_by(|a, b| a.0.cmp(&b.0));
        signature
    }
}
