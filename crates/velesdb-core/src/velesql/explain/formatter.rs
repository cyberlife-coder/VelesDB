//! Query plan rendering and formatting for EXPLAIN output.
//!
//! Extracted from `explain.rs` for maintainability (04-06 module splitting).
//! Handles tree rendering, JSON serialization, and Display formatting.

use std::fmt::{self, Write as _};

use super::{
    FilterPlan, FilterStrategy, FusionInfo, IndexType, MatchTraversalPlan, PlanNode, QueryPlan,
};

impl QueryPlan {
    /// Renders the plan as a tree string.
    #[must_use]
    pub fn to_tree(&self) -> String {
        let mut output = String::from("Query Plan:\n");
        Self::render_node(&self.root, &mut output, "", true);

        Self::render_with_options(&self.with_options, &mut output);
        Self::render_let_bindings(&self.let_bindings, &mut output);
        Self::render_fusion_info(self.fusion_info.as_ref(), &mut output);

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

        if let Some(hit) = self.cache_hit {
            let _ = writeln!(output, "Cache hit: {hit}");
        }
        if let Some(count) = self.plan_reuse_count {
            let _ = writeln!(output, "Plan reuse count: {count}");
        }

        output
    }

    /// Renders WITH clause options into the tree output.
    fn render_with_options(options: &[(String, String)], output: &mut String) {
        if options.is_empty() {
            return;
        }
        let _ = writeln!(output, "\nWITH options:");
        for (key, value) in options {
            let _ = writeln!(output, "  {key} = {value}");
        }
    }

    /// Renders LET bindings into the tree output.
    fn render_let_bindings(bindings: &[String], output: &mut String) {
        if bindings.is_empty() {
            return;
        }
        let _ = writeln!(output, "\nLET bindings:");
        for binding in bindings {
            let _ = writeln!(output, "  {binding}");
        }
    }

    /// Renders FUSION info into the tree output.
    fn render_fusion_info(info: Option<&FusionInfo>, output: &mut String) {
        let Some(fi) = info else { return };
        let _ = writeln!(output, "\nFUSION:");
        let _ = writeln!(output, "  Strategy: {}", fi.strategy);
        if let Some(k) = fi.k {
            let _ = writeln!(output, "  k: {k}");
        }
        if let Some(ref w) = fi.weights {
            let _ = writeln!(output, "  Weights: {w}");
        }
    }

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
                Self::render_filter_node(f, output, prefix, connector, &child_prefix);
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
                Self::render_match_traversal_node(mt, output, prefix, connector, &child_prefix);
            }
        }
    }

    /// Renders a `Filter` plan node into the tree output.
    fn render_filter_node(
        f: &FilterPlan,
        output: &mut String,
        prefix: &str,
        connector: &str,
        child_prefix: &str,
    ) {
        let _ = writeln!(output, "{prefix}{connector}Filter");
        let _ = writeln!(output, "{child_prefix}├─ Conditions: {}", f.conditions);
        // R7: estimated_rows and estimation_method are rendered when present.
        if let Some(rows) = f.estimated_rows {
            let _ = writeln!(output, "{child_prefix}├─ Estimated rows: {rows}");
        }
        if let Some(ref method) = f.estimation_method {
            let _ = writeln!(output, "{child_prefix}├─ Estimation method: {method}");
        }
        let _ = writeln!(
            output,
            "{child_prefix}└─ Selectivity: {:.1}%",
            f.selectivity * 100.0
        );
    }

    /// Renders a `MatchTraversal` plan node into the tree output.
    fn render_match_traversal_node(
        mt: &MatchTraversalPlan,
        output: &mut String,
        prefix: &str,
        connector: &str,
        child_prefix: &str,
    ) {
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

/// Formats a `WithValue` for human-readable EXPLAIN display.
pub(super) fn format_with_value(v: &super::super::ast::WithValue) -> String {
    match v {
        super::super::ast::WithValue::String(s) | super::super::ast::WithValue::Identifier(s) => {
            s.clone()
        }
        super::super::ast::WithValue::Integer(i) => i.to_string(),
        super::super::ast::WithValue::Float(f) => f.to_string(),
        super::super::ast::WithValue::Boolean(b) => b.to_string(),
    }
}
