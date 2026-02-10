//! Query plan rendering and formatting for EXPLAIN output.
//!
//! Extracted from `explain.rs` for maintainability (04-06 module splitting).
//! Handles tree rendering, JSON serialization, and Display formatting.

use std::fmt::{self, Write as _};

use super::{FilterStrategy, IndexType, PlanNode, QueryPlan};

impl QueryPlan {
    /// Renders the plan as a tree string.
    #[must_use]
    pub fn to_tree(&self) -> String {
        let mut output = String::from("Query Plan:\n");
        Self::render_node(&self.root, &mut output, "", true);

        let _ = write!(
            output,
            "\nEstimated cost: {:.3}ms\n",
            self.estimated_cost_ms
        );

        if let Some(ref idx) = self.index_used {
            let _ = writeln!(output, "Index used: {}", idx.as_str());
        }

        if self.filter_strategy != FilterStrategy::None {
            let _ = writeln!(output, "Filter strategy: {}", self.filter_strategy.as_str());
        }

        output
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn render_node(node: &PlanNode, output: &mut String, prefix: &str, is_last: bool) {
        let connector = if is_last { "└─ " } else { "├─ " };
        let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "│  " });

        match node {
            PlanNode::VectorSearch(vs) => {
                let _ = writeln!(output, "{prefix}{connector}VectorSearch");
                let _ = writeln!(output, "{child_prefix}├─ Collection: {}", vs.collection);
                let _ = writeln!(output, "{child_prefix}├─ ef_search: {}", vs.ef_search);
                let _ = writeln!(output, "{child_prefix}└─ Candidates: {}", vs.candidates);
            }
            PlanNode::Filter(f) => {
                let _ = writeln!(output, "{prefix}{connector}Filter");
                let _ = writeln!(output, "{child_prefix}├─ Conditions: {}", f.conditions);
                let _ = writeln!(
                    output,
                    "{child_prefix}└─ Selectivity: {:.1}%",
                    f.selectivity * 100.0
                );
            }
            PlanNode::Limit(l) => {
                let _ = writeln!(output, "{prefix}{connector}Limit: {}", l.count);
            }
            PlanNode::Offset(o) => {
                let _ = writeln!(output, "{prefix}{connector}Offset: {}", o.count);
            }
            PlanNode::TableScan(ts) => {
                let _ = writeln!(output, "{prefix}{connector}TableScan: {}", ts.collection);
            }
            PlanNode::IndexLookup(il) => {
                let _ = writeln!(
                    output,
                    "{prefix}{connector}IndexLookup({}.{})",
                    il.label, il.property
                );
                let _ = writeln!(output, "{child_prefix}└─ Value: {}", il.value);
            }
            PlanNode::Sequence(nodes) => {
                for (i, child) in nodes.iter().enumerate() {
                    Self::render_node(child, output, prefix, i == nodes.len() - 1);
                }
            }
            PlanNode::MatchTraversal(mt) => {
                let _ = writeln!(output, "{prefix}{connector}MatchTraversal");
                let _ = writeln!(output, "{child_prefix}├─ Strategy: {}", mt.strategy);
                if !mt.start_labels.is_empty() {
                    let _ = writeln!(
                        output,
                        "{child_prefix}├─ Start Labels: [{}]",
                        mt.start_labels.join(", ")
                    );
                }
                let _ = writeln!(output, "{child_prefix}├─ Max Depth: {}", mt.max_depth);
                let _ = writeln!(
                    output,
                    "{child_prefix}├─ Relationships: {}",
                    mt.relationship_count
                );
                if let Some(threshold) = mt.similarity_threshold {
                    let _ = writeln!(
                        output,
                        "{child_prefix}└─ Similarity Threshold: {:.2}",
                        threshold
                    );
                } else {
                    let _ = writeln!(
                        output,
                        "{child_prefix}└─ Similarity: {}",
                        if mt.has_similarity { "yes" } else { "no" }
                    );
                }
            }
            PlanNode::FusedSearch(fs) => {
                let _ = writeln!(output, "{prefix}{connector}FusedSearch");
                let _ = writeln!(output, "{child_prefix}├─ Collection: {}", fs.collection);
                let _ = writeln!(output, "{child_prefix}├─ Vectors: {}", fs.vector_count);
                let _ = writeln!(output, "{child_prefix}├─ Fusion: {}", fs.fusion_strategy);
                let _ = writeln!(output, "{child_prefix}└─ Candidates: {}", fs.candidates);
            }
            PlanNode::CrossStoreSearch(cs) => {
                let _ = writeln!(output, "{prefix}{connector}CrossStoreSearch");
                let _ = writeln!(output, "{child_prefix}├─ Collection: {}", cs.collection);
                let _ = writeln!(output, "{child_prefix}├─ Strategy: {}", cs.strategy);
                let _ = writeln!(
                    output,
                    "{child_prefix}├─ Over-fetch: {:.1}x",
                    cs.over_fetch_factor
                );
                let _ = writeln!(
                    output,
                    "{child_prefix}├─ Est. Cost: {:.2}ms",
                    cs.estimated_cost_ms
                );
                let _ = writeln!(
                    output,
                    "{child_prefix}└─ Has Filter: {}",
                    if cs.has_metadata_filter { "yes" } else { "no" }
                );
            }
        }
    }

    /// Renders the plan as JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl IndexType {
    /// Returns the index type as a string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Hnsw => "HNSW",
            Self::Flat => "Flat",
            Self::BinaryQuantization => "BinaryQuantization",
            Self::Property => "PropertyIndex",
        }
    }
}

impl FilterStrategy {
    /// Returns the filter strategy as a string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PreFilter => "pre-filtering (high selectivity)",
            Self::PostFilter => "post-filtering (low selectivity)",
        }
    }
}

impl super::super::ast::CompareOp {
    /// Returns the operator as a string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "!=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

impl fmt::Display for QueryPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_tree())
    }
}
