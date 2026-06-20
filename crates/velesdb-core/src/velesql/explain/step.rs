//! Flattened, structured EXPLAIN step emission.
//!
//! [`QueryPlan::to_plan_steps`] walks the same [`PlanNode`] tree that
//! [`QueryPlan::to_tree`](super::formatter) renders, producing the canonical
//! structured step list consumed by the REST `/query/explain` endpoint. Both
//! the text tree and this step list derive from one source of truth — the plan
//! tree — so the server no longer reconstructs steps from the raw AST.

use super::types::{
    FilterPlan, JoinPlanNode, LimitPlan, PlanNode, PlanStep, PlanStepKind, QueryPlan,
};

use PlanStepKind as Kind;

const VECTOR_DESC: &str = "ANN search using HNSW index with NEAR clause";
const FILTER_DESC: &str = "Apply WHERE clause predicates";
const GROUP_DESC: &str = "Group rows by specified columns";
const AGGREGATE_DESC: &str = "Compute aggregate functions (COUNT, SUM, etc.)";
const SORT_DESC: &str = "Sort results by ORDER BY clause";

impl QueryPlan {
    /// Flattens the plan tree into ordered, structured EXPLAIN steps.
    ///
    /// When the plan has a `LIMIT`, a standalone `OFFSET` node is folded into
    /// that `LIMIT` step's description (`... OFFSET n`), matching the historical
    /// flat step list. A query with `OFFSET` but no `LIMIT` (compound/MATCH)
    /// instead surfaces a dedicated `Offset` step.
    #[must_use]
    pub fn to_plan_steps(&self) -> Vec<PlanStep> {
        let mut flat = Vec::new();
        Self::flatten_nodes(&self.root, &mut flat);
        let offset = pagination_offset(&flat);
        let has_limit = flat.iter().any(|n| matches!(n, PlanNode::Limit(_)));

        let mut steps = Vec::new();
        for node in flat {
            if let Some(step) = step_for_node(node, steps.len() + 1, offset, has_limit) {
                steps.push(step);
            }
        }
        steps
    }

    /// Collects nodes in pipeline order, expanding nested `Sequence` nodes.
    fn flatten_nodes<'a>(node: &'a PlanNode, out: &mut Vec<&'a PlanNode>) {
        match node {
            PlanNode::Sequence(children) => {
                for child in children {
                    Self::flatten_nodes(child, out);
                }
            }
            other => out.push(other),
        }
    }
}

impl PlanStep {
    /// Maps the step kind to the exact REST `operation` wire string.
    ///
    /// The vocabulary is preserved verbatim (e.g. `TableScan` → `"FullScan"`,
    /// `Join` → `"{Type}Join"`) so the `/query/explain` contract is additive.
    #[must_use]
    pub fn rest_operation(&self) -> String {
        match self.operation {
            Kind::VectorSearch => "VectorSearch".to_string(),
            Kind::TableScan => "FullScan".to_string(),
            Kind::IndexLookup => "IndexLookup".to_string(),
            Kind::Filter => "Filter".to_string(),
            Kind::Join => format!("{}Join", self.join_type.as_deref().unwrap_or_default()),
            Kind::GroupBy => "GroupBy".to_string(),
            Kind::Aggregate => "Aggregate".to_string(),
            Kind::Sort => "Sort".to_string(),
            Kind::Limit => "Limit".to_string(),
            Kind::Offset => "Offset".to_string(),
            Kind::MatchTraversal => "MatchTraversal".to_string(),
        }
    }
}

/// Returns the OFFSET count to fold into the Limit step (0 when absent).
fn pagination_offset(flat: &[&PlanNode]) -> u64 {
    flat.iter()
        .find_map(|n| match n {
            PlanNode::Offset(o) => Some(o.count),
            _ => None,
        })
        .unwrap_or(0)
}

/// Builds the structured step for a single leaf node.
///
/// Returns `None` for `Sequence` (flattened away) and for a standalone `Offset`
/// when the plan also has a `LIMIT` (the offset is folded into the Limit step).
/// An `Offset` with no `LIMIT` becomes its own `Offset` step.
fn step_for_node(node: &PlanNode, step: usize, offset: u64, has_limit: bool) -> Option<PlanStep> {
    let built = match node {
        PlanNode::Sequence(_) => return None,
        PlanNode::Offset(_) if has_limit => return None, // folded into the Limit step
        PlanNode::Offset(o) => plain(
            step,
            Kind::Offset,
            format!("Skip {} rows (OFFSET)", o.count),
        ),
        PlanNode::VectorSearch(_) => plain(step, Kind::VectorSearch, VECTOR_DESC.to_string()),
        PlanNode::TableScan(ts) => plain(
            step,
            Kind::TableScan,
            format!("Scan collection '{}'", ts.collection),
        ),
        PlanNode::IndexLookup(il) => plain(
            step,
            Kind::IndexLookup,
            format!(
                "Property index lookup {}.{} = {}",
                il.label, il.property, il.value
            ),
        ),
        PlanNode::GroupBy(_) => plain(step, Kind::GroupBy, GROUP_DESC.to_string()),
        PlanNode::Aggregate(_) => plain(step, Kind::Aggregate, AGGREGATE_DESC.to_string()),
        PlanNode::Sort(_) => plain(step, Kind::Sort, SORT_DESC.to_string()),
        PlanNode::MatchTraversal(mt) => plain(
            step,
            Kind::MatchTraversal,
            format!("Graph traversal: {}", mt.strategy),
        ),
        PlanNode::Filter(f) => filter_step(step, f),
        PlanNode::Join(j) => join_step(step, j),
        PlanNode::Limit(l) => limit_step(step, l, offset),
    };
    Some(built)
}

/// Builds a step with no join type, estimate, or estimation method.
fn plain(step: usize, operation: PlanStepKind, description: String) -> PlanStep {
    PlanStep {
        step,
        operation,
        join_type: None,
        description,
        estimated_rows: None,
        estimation_method: None,
    }
}

/// Builds a `Filter` step, carrying the core plan's native row estimate.
fn filter_step(step: usize, filter: &FilterPlan) -> PlanStep {
    PlanStep {
        step,
        operation: Kind::Filter,
        join_type: None,
        description: FILTER_DESC.to_string(),
        estimated_rows: filter.estimated_rows,
        estimation_method: filter.estimation_method.clone(),
    }
}

/// Builds a `Join` step, carrying the join-type label for the wire string.
fn join_step(step: usize, join: &JoinPlanNode) -> PlanStep {
    PlanStep {
        step,
        operation: Kind::Join,
        join_type: Some(join.join_type.clone()),
        description: format!("Join with '{}'", join.table),
        estimated_rows: None,
        estimation_method: None,
    }
}

/// Builds a `Limit` step, folding any `OFFSET` into its description.
fn limit_step(step: usize, limit: &LimitPlan, offset: u64) -> PlanStep {
    PlanStep {
        step,
        operation: Kind::Limit,
        join_type: None,
        description: limit_description(limit, offset),
        estimated_rows: Some(limit.count),
        estimation_method: None,
    }
}

/// Formats the LIMIT step description, folding OFFSET in (matches the prior
/// server template `Apply LIMIT N (default) OFFSET M`).
fn limit_description(limit: &LimitPlan, offset: u64) -> String {
    let marker = if limit.is_default { " (default)" } else { "" };
    format!("Apply LIMIT {}{marker} OFFSET {offset}", limit.count)
}
