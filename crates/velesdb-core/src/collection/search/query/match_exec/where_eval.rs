//! WHERE clause evaluation for MATCH queries (EPIC-045 US-002).
//!
//! Handles condition evaluation, parameter resolution, and comparison operations.
//! Fix #492: metadata conditions (IN, BETWEEN, LIKE, IS NULL) are now evaluated
//! against node payloads instead of being silently ignored by a catch-all arm.

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::filter;
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use std::collections::HashMap;

/// Applies an ordering comparison operator to an `Ord` pair.
fn apply_ord_op<T: PartialOrd>(op: crate::velesql::CompareOp, a: &T, b: &T) -> bool {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Eq => a == b,
        CompareOp::NotEq => a != b,
        CompareOp::Lt => a < b,
        CompareOp::Gt => a > b,
        CompareOp::Lte => a <= b,
        CompareOp::Gte => a >= b,
    }
}

/// Compares two floats with epsilon tolerance for equality.
fn compare_floats(op: crate::velesql::CompareOp, actual: f64, expected: f64) -> bool {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Eq => (actual - expected).abs() < 0.001,
        CompareOp::NotEq => (actual - expected).abs() >= 0.001,
        _ => apply_ord_op(op, &actual, &expected),
    }
}

/// Compares a similarity score against a threshold, inverting for distance metrics.
fn compare_score(
    op: crate::velesql::CompareOp,
    score: f32,
    threshold: f32,
    higher_is_better: bool,
) -> bool {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Eq => (score - threshold).abs() < f32::EPSILON,
        CompareOp::NotEq => (score - threshold).abs() >= f32::EPSILON,
        CompareOp::Gt | CompareOp::Gte | CompareOp::Lt | CompareOp::Lte => {
            if higher_is_better {
                apply_ord_op(op, &score, &threshold)
            } else {
                // Invert: "similarity > X" ↔ "distance < X"
                apply_ord_op(invert_order(op), &score, &threshold)
            }
        }
    }
}

/// Flips a relational operator (Gt↔Lt, Gte↔Lte). Eq/NotEq pass through.
const fn invert_order(op: crate::velesql::CompareOp) -> crate::velesql::CompareOp {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Gt => CompareOp::Lt,
        CompareOp::Lt => CompareOp::Gt,
        CompareOp::Gte => CompareOp::Lte,
        CompareOp::Lte => CompareOp::Gte,
        other => other,
    }
}

/// Resolves a query vector from a `VectorExpr`, looking up parameters as needed.
pub(super) fn resolve_query_vector(
    vector: &crate::velesql::VectorExpr,
    params: &HashMap<String, serde_json::Value>,
) -> Result<Vec<f32>> {
    use crate::velesql::VectorExpr;

    match vector {
        VectorExpr::Literal(v) => Ok(v.clone()),
        VectorExpr::Parameter(name) => {
            let param_value = params
                .get(name)
                .ok_or_else(|| Error::Config(format!("Missing vector parameter: ${name}")))?;

            match param_value {
                serde_json::Value::Array(arr) => Ok(arr
                    .iter()
                    .filter_map(|v| {
                        v.as_f64().map(|f| {
                            #[allow(clippy::cast_possible_truncation)]
                            let r = f as f32;
                            r
                        })
                    })
                    .collect()),
                _ => Err(Error::Config(format!(
                    "Parameter ${name} must be a vector array"
                ))),
            }
        }
    }
}

/// Bundled per-node context for MATCH WHERE evaluation.
///
/// Groups the invariant evaluation state so the recursive condition walk
/// stays within the argument-count and complexity limits (mirrors
/// `WhereEvalCtx` in the SELECT-side `where_eval`).
struct MatchWhereCtx<'a> {
    node_id: u64,
    bindings: Option<&'a HashMap<String, u64>>,
    /// Bound relationship aliases (alias -> traversed edge id) so `r.prop`
    /// resolves against the EDGE's properties (audit 2026-06 F).
    edge_bindings: Option<&'a HashMap<String, u64>>,
    params: &'a HashMap<String, serde_json::Value>,
    payload_guard: &'a LogPayloadStorage,
}

impl Collection {
    /// Evaluates a WHERE condition against a node's payload (EPIC-045 US-002).
    ///
    /// Supports comparisons, logical operators, similarity, and metadata
    /// conditions (IN, BETWEEN, LIKE, ILIKE, IS NULL, IS NOT NULL, MATCH).
    /// Parameters are resolved from the `params` map.
    ///
    /// The caller must pass a pre-acquired `payload_guard` to avoid
    /// per-node lock acquisitions during BFS traversal.
    ///
    /// Fix #492: metadata conditions are now evaluated via the filter engine
    /// instead of being silently ignored by a catch-all arm.
    ///
    /// `edge_bindings` maps relationship aliases to traversed edge ids so
    /// `r.prop` resolves against the EDGE's properties (audit 2026-06 F).
    pub(crate) fn evaluate_where_condition(
        &self,
        node_id: u64,
        bindings: Option<&HashMap<String, u64>>,
        edge_bindings: Option<&HashMap<String, u64>>,
        condition: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
        payload_guard: &LogPayloadStorage,
    ) -> Result<bool> {
        let ctx = MatchWhereCtx {
            node_id,
            bindings,
            edge_bindings,
            params,
            payload_guard,
        };
        self.eval_match_condition(&ctx, condition)
    }

    /// Recursively evaluates a single condition node of a MATCH WHERE tree.
    fn eval_match_condition(
        &self,
        ctx: &MatchWhereCtx<'_>,
        condition: &crate::velesql::Condition,
    ) -> Result<bool> {
        use crate::velesql::Condition;

        match condition {
            Condition::Comparison(cmp) => self.evaluate_comparison_condition(ctx, cmp),
            Condition::And(left, right) => self.eval_match_and(ctx, left, right),
            Condition::Or(left, right) => self.eval_match_or(ctx, left, right),
            Condition::Not(inner) => Ok(!self.eval_match_condition(ctx, inner)?),
            Condition::Group(inner) => self.eval_match_condition(ctx, inner),
            Condition::Similarity(sim) => {
                self.evaluate_similarity_condition(ctx.node_id, sim, ctx.params)
            }
            // Fix #492: metadata conditions converted to filter engine evaluation.
            Condition::In(_)
            | Condition::Between(_)
            | Condition::Like(_)
            | Condition::IsNull(_)
            | Condition::Match(_)
            | Condition::ContainsText(_)
            | Condition::Contains(_)
            | Condition::GeoDistance(_)
            | Condition::GeoBbox(_) => self.evaluate_metadata_condition_for_node(ctx, condition),
            // VectorSearch, VectorFusedSearch, SparseVectorSearch, and GraphMatch
            // are handled separately in `execute_match_with_similarity`.
            Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::GraphMatch(_) => Ok(true),
        }
    }

    /// Evaluates AND with short-circuit: returns false immediately if left is false.
    fn eval_match_and(
        &self,
        ctx: &MatchWhereCtx<'_>,
        left: &crate::velesql::Condition,
        right: &crate::velesql::Condition,
    ) -> Result<bool> {
        if !self.eval_match_condition(ctx, left)? {
            return Ok(false);
        }
        self.eval_match_condition(ctx, right)
    }

    /// Evaluates OR with short-circuit: returns true immediately if left is true.
    fn eval_match_or(
        &self,
        ctx: &MatchWhereCtx<'_>,
        left: &crate::velesql::Condition,
        right: &crate::velesql::Condition,
    ) -> Result<bool> {
        if self.eval_match_condition(ctx, left)? {
            return Ok(true);
        }
        self.eval_match_condition(ctx, right)
    }

    /// Evaluates a single comparison condition against a node's payload,
    /// or against the traversed edge's properties when the column alias is
    /// a bound relationship alias (audit 2026-06 F).
    ///
    /// Uses the pre-acquired `payload_guard` instead of locking per-node.
    fn evaluate_comparison_condition(
        &self,
        ctx: &MatchWhereCtx<'_>,
        cmp: &crate::velesql::Comparison,
    ) -> Result<bool> {
        if let Some(edge_id) = resolve_edge_target(&cmp.column, ctx.edge_bindings) {
            return self.evaluate_edge_comparison(edge_id, cmp, ctx.params);
        }

        let target_id = resolve_target_id(&cmp.column, ctx.bindings, ctx.node_id);

        let Some(target_payload) = ctx.payload_guard.retrieve(target_id).ok().flatten() else {
            return Ok(false);
        };

        let column_path = strip_alias(&cmp.column, ctx.bindings);
        let Some(actual) = Self::json_get_path(&target_payload, column_path) else {
            return Ok(false);
        };

        let resolved_value = Self::resolve_where_param(&cmp.value, ctx.params)?;
        Self::evaluate_comparison(cmp.operator, actual, &resolved_value)
    }

    /// Evaluates a comparison whose column refers to a bound relationship
    /// alias (e.g. `r.since = 2020`) against the traversed edge's properties.
    fn evaluate_edge_comparison(
        &self,
        edge_id: u64,
        cmp: &crate::velesql::Comparison,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        let Some(edge) = self.edge_store.get_edge(edge_id) else {
            return Ok(false);
        };
        let property = cmp.column.split_once('.').map_or("", |(_, rest)| rest);
        let Some(actual) = edge_property_path(&edge, property) else {
            return Ok(false);
        };
        let resolved_value = Self::resolve_where_param(&cmp.value, params)?;
        Self::evaluate_comparison(cmp.operator, actual, &resolved_value)
    }

    /// Evaluates a metadata condition (IN, BETWEEN, LIKE, IS NULL, MATCH)
    /// against a node's payload by converting to the filter engine (Fix #492).
    ///
    /// Uses the pre-acquired `payload_guard` instead of locking per-node.
    /// The column name may be alias-prefixed (e.g. `n.category`); the alias
    /// is resolved to the correct node ID via bindings, and stripped before
    /// building the filter condition so the filter engine sees the bare field
    /// path.
    fn evaluate_metadata_condition_for_node(
        &self,
        ctx: &MatchWhereCtx<'_>,
        condition: &crate::velesql::Condition,
    ) -> Result<bool> {
        // Fix #486: Resolve the target node ID from the condition's column
        // alias, mirroring what evaluate_comparison_condition does. Without
        // this, `WHERE a.category IN (...)` would evaluate against node_id
        // (the traversal target) instead of the node bound to alias `a`.
        let column = column_of_metadata_condition(condition);

        // Audit 2026-06 F: a relationship alias resolves against the EDGE's
        // properties, not a node payload.
        if let Some(edge_id) = column.and_then(|col| resolve_edge_target(col, ctx.edge_bindings)) {
            return self.evaluate_metadata_condition_for_edge(edge_id, condition, ctx);
        }

        let target_id = column.map_or(ctx.node_id, |col| {
            resolve_target_id(col, ctx.bindings, ctx.node_id)
        });

        let Some(payload) = ctx.payload_guard.retrieve(target_id).ok().flatten() else {
            return Ok(false);
        };

        let rewritten = rewrite_condition_aliases(condition.clone(), ctx.bindings);
        // Resolve parameter placeholders (e.g. `IN ($a, $b)`) before the
        // filter conversion, which would otherwise turn them into NULL.
        let resolved = Self::resolve_condition_params(&rewritten, ctx.params)?;
        let filter_cond: filter::Condition = resolved.into();
        Ok(filter_cond.matches(&payload))
    }

    /// Mirrors `evaluate_metadata_condition_for_node` for relationship
    /// aliases: runs the filter engine against the bound edge's properties.
    fn evaluate_metadata_condition_for_edge(
        &self,
        edge_id: u64,
        condition: &crate::velesql::Condition,
        ctx: &MatchWhereCtx<'_>,
    ) -> Result<bool> {
        let Some(edge) = self.edge_store.get_edge(edge_id) else {
            return Ok(false);
        };
        let payload = serde_json::Value::Object(
            edge.properties()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        let rewritten = rewrite_condition_aliases(condition.clone(), ctx.edge_bindings);
        // Resolve parameter placeholders before the filter conversion, which
        // would otherwise turn them into NULL (same hardening as the node path).
        let resolved = Self::resolve_condition_params(&rewritten, ctx.params)?;
        let filter_cond: filter::Condition = resolved.into();
        Ok(filter_cond.matches(&payload))
    }

    /// Evaluates a similarity condition against a node's vector (EPIC-052 US-007).
    fn evaluate_similarity_condition(
        &self,
        node_id: u64,
        sim: &crate::velesql::SimilarityCondition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        let query_vector = resolve_query_vector(&sim.vector, params)?;
        if query_vector.is_empty() {
            return Ok(false);
        }

        let vector_storage = self.vector_storage.read();
        let Some(node_vector) = vector_storage.retrieve(node_id)? else {
            return Ok(false);
        };

        if node_vector.len() != query_vector.len() {
            return Ok(false);
        }

        let config = self.config.read();
        let metric = config.metric;
        let higher_is_better = metric.higher_is_better();
        drop(config);

        let score = metric.calculate(&node_vector, &query_vector);

        #[allow(clippy::cast_possible_truncation)]
        let threshold = sim.threshold as f32;

        Ok(compare_score(
            sim.operator,
            score,
            threshold,
            higher_is_better,
        ))
    }

    /// Resolves a Value for WHERE clause, substituting parameters from the params map.
    ///
    /// If the value is a Parameter, looks it up in params and converts to appropriate Value type.
    /// Otherwise, returns the value unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if a required parameter is missing.
    pub(crate) fn resolve_where_param(
        value: &crate::velesql::Value,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        use crate::velesql::Value;

        match value {
            Value::Parameter(name) => {
                let param_value = params
                    .get(name)
                    .ok_or_else(|| Error::Config(format!("Missing parameter: ${name}")))?;

                Ok(match param_value {
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::Integer(i)
                        } else if let Some(u) = n.as_u64() {
                            Value::UnsignedInteger(u)
                        } else if let Some(f) = n.as_f64() {
                            Value::Float(f)
                        } else {
                            Value::Null
                        }
                    }
                    serde_json::Value::String(s) => Value::String(s.clone()),
                    serde_json::Value::Bool(b) => Value::Boolean(*b),
                    serde_json::Value::Null => Value::Null,
                    _ => {
                        return Err(Error::Config(format!(
                            "Unsupported parameter type for ${name}: {param_value:?}",
                        )));
                    }
                })
            }
            other => Ok(other.clone()),
        }
    }

    /// Evaluates a comparison operation.
    #[allow(clippy::unnecessary_wraps)] // Consistent with other evaluation methods
    pub(crate) fn evaluate_comparison(
        operator: crate::velesql::CompareOp,
        actual: &serde_json::Value,
        expected: &crate::velesql::Value,
    ) -> Result<bool> {
        use crate::velesql::Value;

        Ok(match (actual, expected) {
            (serde_json::Value::Number(n), Value::Integer(i)) => n
                .as_i64()
                .is_some_and(|actual_i| apply_ord_op(operator, &actual_i, i)),
            (serde_json::Value::Number(n), Value::UnsignedInteger(u)) => n
                .as_u64()
                .is_some_and(|actual_u| apply_ord_op(operator, &actual_u, u)),
            (serde_json::Value::Number(n), Value::Float(f)) => n
                .as_f64()
                .is_some_and(|actual_f| compare_floats(operator, actual_f, *f)),
            (serde_json::Value::String(s), Value::String(expected_s)) => {
                apply_ord_op(operator, &s.as_str(), &expected_s.as_str())
            }
            (serde_json::Value::Bool(b), Value::Boolean(expected_b)) => {
                matches!(
                    (operator, b == expected_b),
                    (crate::velesql::CompareOp::Eq, true)
                        | (crate::velesql::CompareOp::NotEq, false)
                )
            }
            (serde_json::Value::Null, Value::Null) => {
                matches!(operator, crate::velesql::CompareOp::Eq)
            }
            (_, Value::Null) => matches!(operator, crate::velesql::CompareOp::NotEq),
            _ => false,
        })
    }

    pub(super) fn json_get_path<'a>(
        root: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        if path.is_empty() {
            return Some(root);
        }

        let mut current = root;
        for part in path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}

/// Resolves the target node ID from a column reference, using alias bindings if present.
fn resolve_target_id(
    column: &str,
    bindings: Option<&HashMap<String, u64>>,
    default_id: u64,
) -> u64 {
    column
        .split_once('.')
        .and_then(|(alias, _)| bindings?.get(alias).copied())
        .unwrap_or(default_id)
}

/// Resolves an alias-prefixed column against bound relationship aliases.
///
/// Returns the bound edge id when the column's alias prefix refers to a
/// relationship alias (e.g. `r.since` with `r` bound by the pattern walker).
fn resolve_edge_target(column: &str, edge_bindings: Option<&HashMap<String, u64>>) -> Option<u64> {
    let (alias, _) = column.split_once('.')?;
    edge_bindings?.get(alias).copied()
}

/// Looks up a (possibly nested) property path on an edge's properties.
pub(super) fn edge_property_path<'a>(
    edge: &'a GraphEdge,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let (first, rest) = path.split_once('.').map_or((path, ""), |(f, r)| (f, r));
    Collection::json_get_path(edge.property(first)?, rest)
}

/// Strips the alias prefix from a column path when the alias exists in bindings.
fn strip_alias<'a>(column: &'a str, bindings: Option<&HashMap<String, u64>>) -> &'a str {
    match column.split_once('.') {
        Some((alias, rest)) if bindings.and_then(|b| b.get(alias)).is_some() => rest,
        _ => column,
    }
}

/// Strips the alias prefix from a column name string.
///
/// Returns the bare field path (e.g. `"n.category"` → `"category"`) when the
/// prefix matches a bound alias, or the original string if no alias matches.
fn strip_alias_owned(column: &str, bindings: Option<&HashMap<String, u64>>) -> String {
    strip_alias(column, bindings).to_string()
}

/// Extracts the column name from a metadata condition variant.
///
/// Returns `Some(&str)` for condition types that carry a `column` field.
/// Non-metadata variants return `None`.
fn column_of_metadata_condition(condition: &crate::velesql::Condition) -> Option<&str> {
    use crate::velesql::Condition;
    match condition {
        Condition::In(ic) => Some(&ic.column),
        Condition::Between(btw) => Some(&btw.column),
        Condition::Like(lk) => Some(&lk.column),
        Condition::IsNull(isn) => Some(&isn.column),
        Condition::Match(m) => Some(&m.column),
        Condition::ContainsText(ct) => Some(&ct.column),
        Condition::Contains(c) => Some(&c.column),
        Condition::GeoDistance(gd) => Some(&gd.column),
        Condition::GeoBbox(gb) => Some(&gb.column),
        _ => None,
    }
}

/// Rewrites alias-prefixed column names in metadata conditions so the filter
/// engine receives bare field paths (Fix #492).
///
/// Only rewrites the leaf conditions that carry a `column` field; logical
/// combinators (And, Or, Not, Group) are not reachable here because the
/// caller dispatches them before reaching this function.
fn rewrite_condition_aliases(
    condition: crate::velesql::Condition,
    bindings: Option<&HashMap<String, u64>>,
) -> crate::velesql::Condition {
    use crate::velesql::Condition;

    match condition {
        Condition::In(mut ic) => {
            ic.column = strip_alias_owned(&ic.column, bindings);
            Condition::In(ic)
        }
        Condition::Between(mut btw) => {
            btw.column = strip_alias_owned(&btw.column, bindings);
            Condition::Between(btw)
        }
        Condition::Like(mut lk) => {
            lk.column = strip_alias_owned(&lk.column, bindings);
            Condition::Like(lk)
        }
        Condition::IsNull(mut isn) => {
            isn.column = strip_alias_owned(&isn.column, bindings);
            Condition::IsNull(isn)
        }
        Condition::Match(mut m) => {
            m.column = strip_alias_owned(&m.column, bindings);
            Condition::Match(m)
        }
        Condition::ContainsText(mut ct) => {
            ct.column = strip_alias_owned(&ct.column, bindings);
            Condition::ContainsText(ct)
        }
        Condition::Contains(mut c) => {
            c.column = strip_alias_owned(&c.column, bindings);
            Condition::Contains(c)
        }
        Condition::GeoDistance(mut gd) => {
            gd.column = strip_alias_owned(&gd.column, bindings);
            Condition::GeoDistance(gd)
        }
        Condition::GeoBbox(mut gb) => {
            gb.column = strip_alias_owned(&gb.column, bindings);
            Condition::GeoBbox(gb)
        }
        // Non-metadata conditions pass through unchanged.
        other => other,
    }
}
