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
            self.expand_pattern(
                *start_id,
                0,
                pattern,
                edge_store,
                ctx,
                &mut bindings,
                &mut path,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_pattern(
        &self,
        current_id: u64,
        rel_idx: usize,
        pattern: &GraphPattern,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
        bindings: &mut HashMap<String, u64>,
        path: &mut Vec<u64>,
    ) -> Result<()> {
        if rel_idx >= pattern.relationships.len() {
            return self.accept_pattern_match(current_id, bindings, path, ctx);
        }

        let rel = &pattern.relationships[rel_idx];
        let (min_hops, max_hops) = rel.range.unwrap_or((1, 1));
        self.expand_relationship(
            current_id, rel_idx, 0, min_hops, max_hops, pattern, rel, edge_store, ctx, bindings,
            path,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_relationship(
        &self,
        current_id: u64,
        rel_idx: usize,
        hops: u32,
        min_hops: u32,
        max_hops: u32,
        pattern: &GraphPattern,
        rel: &RelationshipPattern,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
        bindings: &mut HashMap<String, u64>,
        path: &mut Vec<u64>,
    ) -> Result<()> {
        if hops >= min_hops {
            self.try_bind_next_node(
                current_id, rel_idx, pattern, edge_store, ctx, bindings, path,
            )?;
        }
        if hops >= max_hops || ctx.all_results.len() >= ctx.limit {
            return Ok(());
        }
        for edge in Self::matching_edges(edge_store, current_id, rel) {
            self.follow_edge(
                current_id, &edge, rel_idx, hops, min_hops, max_hops, pattern, edge_store, ctx,
                bindings, path,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn follow_edge(
        &self,
        current_id: u64,
        edge: &GraphEdge,
        rel_idx: usize,
        hops: u32,
        min_hops: u32,
        max_hops: u32,
        pattern: &GraphPattern,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
        bindings: &mut HashMap<String, u64>,
        path: &mut Vec<u64>,
    ) -> Result<()> {
        if ctx.all_results.len() >= ctx.limit
            || !Self::edge_matches(edge, &pattern.relationships[rel_idx])
        {
            return Ok(());
        }
        let next_id = Self::edge_next_node(edge, current_id);
        path.push(edge.id());
        *ctx.iteration_count += 1;
        self.check_depth_and_periodic_guardrails(path.len() as u32, ctx)?;
        self.expand_relationship(
            next_id,
            rel_idx,
            hops.saturating_add(1),
            min_hops,
            max_hops,
            pattern,
            &pattern.relationships[rel_idx],
            edge_store,
            ctx,
            bindings,
            path,
        )?;
        path.pop();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn try_bind_next_node(
        &self,
        node_id: u64,
        rel_idx: usize,
        pattern: &GraphPattern,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
        bindings: &mut HashMap<String, u64>,
        path: &mut Vec<u64>,
    ) -> Result<()> {
        let Some(node) = pattern.nodes.get(rel_idx + 1) else {
            return Ok(());
        };
        if !Self::node_matches_bound_pattern(node_id, node, ctx.payload_guard) {
            return Ok(());
        }
        let inserted = Self::bind_node_alias(bindings, node, node_id);
        let (AliasBinding::Unchanged | AliasBinding::Inserted(_)) = inserted else {
            return Ok(());
        };
        self.expand_pattern(
            node_id,
            rel_idx + 1,
            pattern,
            edge_store,
            ctx,
            bindings,
            path,
        )?;
        if let AliasBinding::Inserted(alias) = inserted {
            bindings.remove(&alias);
        }
        Ok(())
    }

    fn accept_pattern_match(
        &self,
        node_id: u64,
        bindings: &HashMap<String, u64>,
        path: &[u64],
        ctx: &mut TraversalCtx<'_>,
    ) -> Result<()> {
        if let Some(ref where_clause) = ctx.match_clause.where_clause {
            if !self.evaluate_where_condition(
                node_id,
                Some(bindings),
                where_clause,
                ctx.params,
                ctx.payload_guard,
            )? {
                return Ok(());
            }
        }

        let signature = Self::binding_signature(bindings);
        if !ctx.seen_bindings.insert(signature) {
            return Ok(());
        }

        let mut result = MatchResult::new(node_id, path.len() as u32, path.to_vec());
        result.bindings.clone_from(bindings);
        result.projected =
            self.project_properties(bindings, &ctx.match_clause.return_clause, ctx.payload_guard);
        ctx.all_results.push(result);
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
