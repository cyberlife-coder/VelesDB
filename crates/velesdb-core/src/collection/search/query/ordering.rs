//! ORDER BY clause execution for VelesQL queries.
//!
//! Handles multi-column sorting with support for:
//! - Metadata field sorting (ASC/DESC)
//! - similarity() function sorting
//! - Arithmetic expression sorting (EPIC-042)
//! - Mixed type JSON value comparison with total ordering

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{evaluate_arithmetic, ScoreContext};
use std::cmp::Ordering;

/// The engine-shared ORDER BY value order (`NULLS LAST` ascending), kept at
/// this path for callers that import it from here.
pub use crate::velesql::compare_json_values;

impl Collection {
    /// Apply ORDER BY clause to results.
    ///
    /// Supports multiple ORDER BY columns with stable sorting.
    /// Each column is compared in order; ties are broken by subsequent columns.
    ///
    /// # Examples
    ///
    /// ```sql
    /// SELECT * FROM collection ORDER BY category ASC, priority DESC
    /// SELECT * FROM collection ORDER BY similarity() DESC, timestamp ASC
    /// ```
    pub(crate) fn apply_order_by(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        self.apply_order_by_with_let(results, order_by, params, &[], results.len())
    }

    /// Apply ORDER BY with pre-evaluated LET bindings (VelesQL v1.10 Phase 3),
    /// ordering only the `leading` first rows.
    ///
    /// `let_bindings` contains per-result pre-evaluated `(name, value)` pairs
    /// stored as `[result_idx][(name, value)]`. If empty, behaves identically
    /// to [`Self::apply_order_by`].
    ///
    /// The `leading` best rows of the whole slice come first, in order — the
    /// rows a window of `OFFSET + LIMIT` keeps — and the rest follow in no
    /// defined order: a selection over every row, then a sort of those
    /// `leading` only ([`rank_leading`]). `leading >= results.len()` sorts
    /// them all.
    pub(crate) fn apply_order_by_with_let(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
        per_result_let: &[Vec<(String, f32)>],
        leading: usize,
    ) -> Result<()> {
        let mut keys = self.precompute_similarity_scores(results, order_by, params)?;
        self.apply_prepared_order(results, order_by, &mut keys, per_result_let, leading);
        Ok(())
    }

    /// Applies similarity keys captured from these rows before window aliases
    /// were injected. Scalar and LET keys read the current rows.
    pub(super) fn apply_prepared_order(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        keys: &mut PreparedOrderKeys,
        per_result_let: &[Vec<(String, f32)>],
        leading: usize,
    ) {
        if let Some((scores, recomputed)) =
            self.rank_prepared_rows(results, order_by, keys, per_result_let, leading)
        {
            write_back_similarity_scores(results, scores, recomputed);
        }
    }

    /// Orders the `leading` best rows of `results` first, as
    /// [`Self::apply_order_by_with_let`] does, without writing any score
    /// back: the rows keep the scores their plan gave them, so ranking them
    /// again — a window narrowed in steps (`ordered_scan.rs`) — compares
    /// them as it did the first time. Returns the first similarity column's
    /// scores in output order, with whether that column recomputed them
    /// (`similarity(field, $v)`); `None` when there is none.
    pub(super) fn rank_rows(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
        per_result_let: &[Vec<(String, f32)>],
        leading: usize,
    ) -> Result<Option<(Vec<Option<f32>>, bool)>> {
        let mut keys = self.precompute_similarity_scores(results, order_by, params)?;
        Ok(self.rank_prepared_rows(results, order_by, &mut keys, per_result_let, leading))
    }

    fn rank_prepared_rows(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        keys: &mut PreparedOrderKeys,
        per_result_let: &[Vec<(String, f32)>],
        leading: usize,
    ) -> Option<(Vec<Option<f32>>, bool)> {
        if order_by.is_empty() {
            return None;
        }

        let higher_is_better = self.storage.config.read().metric.higher_is_better();
        let field_keys = decorate_field_keys(results, order_by, per_result_let);

        let mut indices: Vec<usize> = (0..results.len()).collect();
        rank_leading(&mut indices, leading, |&i, &j| {
            Self::compare_by_order_columns(
                i,
                j,
                results,
                order_by,
                keys,
                higher_is_better,
                per_result_let,
                &field_keys,
            )
            // Match the secondary index's ascending point IDs within each
            // equal-key bucket. A JOIN or UNION ALL can repeat an ID, in
            // which case equal rows retain their input order on every page.
            .then_with(|| results[i].point.id.cmp(&results[j].point.id))
            .then_with(|| i.cmp(&j))
        });

        // Pre-compute sorted scores before the permutation destroys the
        // original-position mapping (indices[i] = original slot for output i).
        let sorted_scores = extract_sorted_similarity_scores(order_by, &keys.similarity, &indices);

        // Apply the sort permutation in-place using cycle decomposition.
        // Avoids cloning every SearchResult (no heap allocation for vector data).
        keys.window.ordered(&indices[..indices.len().min(leading)]);
        apply_permutation_in_place(results, &mut indices);
        sorted_scores
    }

    /// Pre-computes similarity scores for all ORDER BY similarity() columns:
    /// per column, each row's score, `None` for a row the column cannot
    /// score (a null key).
    pub(super) fn precompute_similarity_scores(
        &self,
        results: &[SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<PreparedOrderKeys> {
        use crate::velesql::OrderByExpr;
        let mut map = std::collections::HashMap::new();
        for (idx, ob) in order_by.iter().enumerate() {
            match &ob.expr {
                OrderByExpr::Similarity(sim) => {
                    let order_vec = crate::velesql::bind_vector(&sim.vector, params)?;
                    let scores =
                        self.similarity_scores_for_field(results, &sim.field, &order_vec)?;
                    map.insert(idx, scores);
                }
                OrderByExpr::SimilarityBare => {
                    // Zero-arg similarity(): use existing search scores (no recompute).
                    let scores: Vec<Option<f32>> = results.iter().map(|r| Some(r.score)).collect();
                    map.insert(idx, scores);
                }
                OrderByExpr::Field(_) | OrderByExpr::Aggregate(_) | OrderByExpr::Arithmetic(_) => {}
            }
        }
        Ok(PreparedOrderKeys {
            similarity: map,
            window: crate::velesql::window_evaluator::WindowInputs::default(),
        })
    }

    /// Scores every result row against `order_vec` using the vector selected by
    /// `field`: its primary vector when `field == "vector"`, otherwise the
    /// named vector in its hydrated payload. A joined or compound row carries
    /// the values to rank even when another row shares its point id.
    ///
    /// A row whose vector is missing (a graph node without an embedding),
    /// `None`, or length-mismatched has no score: `None`, a null key that
    /// [`SelectOrderBy::directed_score`](crate::velesql::SelectOrderBy::directed_score)
    /// sorts as every ORDER BY sorts a null.
    fn similarity_scores_for_field(
        &self,
        results: &[SearchResult],
        field: &str,
        order_vec: &[f32],
    ) -> Result<Vec<Option<f32>>> {
        let metric = self.storage.config.read().metric;
        results
            .iter()
            .map(|r| {
                if field == crate::velesql::PRIMARY_VECTOR_FIELD {
                    return Ok(metric.checked_calculate(&r.point.vector, order_vec));
                }
                let Some(payload) = &r.point.payload else {
                    return Ok(None);
                };
                let vector = crate::velesql::payload_vector(payload, field)?;
                Ok(vector.and_then(|v| metric.checked_calculate(&v, order_vec)))
            })
            .collect()
    }

    /// Compares two result indices across all ORDER BY columns.
    #[allow(clippy::too_many_arguments)] // Reason: internal comparator; the decorated keys ride alongside the raw inputs
    fn compare_by_order_columns(
        i: usize,
        j: usize,
        results: &[SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        keys: &PreparedOrderKeys,
        higher_is_better: bool,
        per_result_let: &[Vec<(String, f32)>],
        field_keys: &[Option<Vec<Option<&serde_json::Value>>>],
    ) -> Ordering {
        for (idx, ob) in order_by.iter().enumerate() {
            let directed_cmp = if let Some(scores) = keys.similarity.get(&idx) {
                ob.directed_score(scores[i], scores[j], higher_is_better)
            } else if let Some(cmp) = keys.window.compare(idx, i, j) {
                ob.directed(cmp, higher_is_better)
            } else {
                let keys = field_keys.get(idx).and_then(Option::as_ref);
                let cmp = Self::compare_values(ob, keys, i, j, results, per_result_let);
                ob.directed(cmp, higher_is_better)
            };
            if directed_cmp != Ordering::Equal {
                return directed_cmp;
            }
        }
        Ordering::Equal
    }

    /// Compares two results on an ORDER BY item that is no similarity
    /// column, ascending: `keys` holds the decorated payload keys of a plain
    /// field column.
    fn compare_values(
        ob: &crate::velesql::SelectOrderBy,
        keys: Option<&Vec<Option<&serde_json::Value>>>,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        use crate::velesql::OrderByExpr;
        match (&ob.expr, keys) {
            (OrderByExpr::Field(_), Some(keys)) => compare_json_values(keys[i], keys[j]),
            (OrderByExpr::Field(field_name), None) => {
                Self::compare_field_expr(field_name, i, j, results, per_result_let)
            }
            // Design: Arithmetic ORDER BY uses direct numeric ordering without
            // distance-metric inversion. Users constructing custom formulas with
            // Euclidean/Hamming scores should account for lower-is-better semantics
            // in their expression (e.g., `ORDER BY -1 * vector_score + price ASC`).
            (OrderByExpr::Arithmetic(expr), _) => {
                Self::compare_arithmetic(expr, i, j, results, per_result_let)
            }
            // A similarity column is compared from its precomputed keys.
            (
                OrderByExpr::Aggregate(_)
                | OrderByExpr::Similarity(_)
                | OrderByExpr::SimilarityBare,
                _,
            ) => Ordering::Equal,
        }
    }

    /// Compares a payload field value between two results; `id` is the point
    /// id, as in every clause.
    fn compare_payload_field(
        field_name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
    ) -> Ordering {
        if field_name == "id" {
            return results[i].point.id.cmp(&results[j].point.id);
        }
        let val_i = results[i]
            .point
            .payload
            .as_ref()
            .and_then(|p| crate::velesql::row_field(p, field_name));
        let val_j = results[j]
            .point
            .payload
            .as_ref()
            .and_then(|p| crate::velesql::row_field(p, field_name));
        compare_json_values(val_i, val_j)
    }

    /// Compares a field name, checking LET bindings first, then payload.
    fn compare_field_or_let(
        field_name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        if let (Some(let_i), Some(let_j)) = (per_result_let.get(i), per_result_let.get(j)) {
            if let Some(vi) = let_i.iter().find(|(k, _)| k == field_name).map(|(_, v)| *v) {
                let vj = let_j
                    .iter()
                    .find(|(k, _)| k == field_name)
                    .map_or(0.0, |(_, v)| *v);
                return vi.total_cmp(&vj);
            }
        }
        Self::compare_payload_field(field_name, i, j, results)
    }

    /// Compares a bare `ORDER BY` field. A built-in score variable
    /// (`sparse_score`, `bm25_score`, …) resolves from each result's component
    /// scores — exactly as the arithmetic ORDER BY path does — so
    /// `ORDER BY sparse_score DESC` actually ranks instead of silently no-op'ing
    /// as an absent payload field. Anything else is a LET binding or payload.
    fn compare_field_expr(
        field_name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        if crate::velesql::is_builtin_score_variable(field_name) {
            Self::compare_score_variable(field_name, i, j, results, per_result_let)
        } else {
            Self::compare_field_or_let(field_name, i, j, results, per_result_let)
        }
    }

    /// Compares two results by a built-in score variable, resolving each via the
    /// same `ScoreContext::resolve_variable` the arithmetic path uses (LET first,
    /// then the component breakdown with the absent-component default).
    fn compare_score_variable(
        name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        let (ctx_i, ctx_j) = Self::score_context_pair(i, j, results, per_result_let);
        ctx_i
            .resolve_variable(name)
            .total_cmp(&ctx_j.resolve_variable(name))
    }

    /// Compares two results by an arithmetic expression with full context.
    fn compare_arithmetic(
        expr: &crate::velesql::ArithmeticExpr,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        let (ctx_i, ctx_j) = Self::score_context_pair(i, j, results, per_result_let);
        let val_i = evaluate_arithmetic(expr, &ctx_i);
        let val_j = evaluate_arithmetic(expr, &ctx_j);
        val_i.total_cmp(&val_j)
    }

    /// Builds the `ScoreContext` pair for comparing results `i` and `j`, each
    /// with its own LET bindings, score, payload, and component breakdown.
    fn score_context_pair<'a>(
        i: usize,
        j: usize,
        results: &'a [SearchResult],
        per_result_let: &'a [Vec<(String, f32)>],
    ) -> (ScoreContext<'a>, ScoreContext<'a>) {
        let ctx_i = ScoreContext::with_let_bindings(
            results[i].score,
            results[i].point.payload.as_ref(),
            results[i].component_scores.as_deref(),
            per_result_let.get(i).map(Vec::as_slice),
        )
        .for_point(results[i].point.id);
        let ctx_j = ScoreContext::with_let_bindings(
            results[j].score,
            results[j].point.payload.as_ref(),
            results[j].component_scores.as_deref(),
            per_result_let.get(j).map(Vec::as_slice),
        )
        .for_point(results[j].point.id);
        (ctx_i, ctx_j)
    }
}

/// Per similarity ORDER BY column (by its index in the clause), each row's
/// score, `None` for a row the column cannot score.
type SimilarityKeys = std::collections::HashMap<usize, Vec<Option<f32>>>;

pub(super) struct PreparedOrderKeys {
    similarity: SimilarityKeys,
    pub(super) window: crate::velesql::window_evaluator::WindowInputs,
}

/// Whether an ORDER BY `name` reads a payload field: neither a built-in
/// score variable nor `id`, the point id.
fn is_payload_field(name: &str) -> bool {
    name != "id" && !crate::velesql::is_builtin_score_variable(name)
}

/// Decorate-then-sort for plain payload fields: each row's key of every
/// plain payload-field column, resolved ONCE instead of re-walking the
/// (BTreeMap-backed) payload JSON inside the comparator — O(n) resolutions
/// where the comparator paid O(n log n) x2. LET-bound and builtin-score
/// fields keep the per-comparison path: they read per-row f32 contexts, not
/// payload JSON, so there is nothing to decorate (`None`).
fn decorate_field_keys<'r>(
    results: &'r [SearchResult],
    order_by: &[crate::velesql::SelectOrderBy],
    per_result_let: &[Vec<(String, f32)>],
) -> Vec<Option<Vec<Option<&'r serde_json::Value>>>> {
    order_by
        .iter()
        .map(|ob| match &ob.expr {
            crate::velesql::OrderByExpr::Field(field_name)
                if per_result_let.is_empty() && is_payload_field(field_name) =>
            {
                Some(
                    results
                        .iter()
                        .map(|r| {
                            r.point
                                .payload
                                .as_ref()
                                .and_then(|p| crate::velesql::row_field(p, field_name))
                        })
                        .collect(),
                )
            }
            _ => None,
        })
        .collect()
}

/// Puts the `leading` least elements of `indices` under `compare` first, in
/// order; the rest follow in no defined order. `compare` is a total order
/// (point ID and original position break any ties), so the leading elements are
/// exactly those a full sort puts there: a selection over all `n`, then a
/// sort of the `leading` alone, instead of sorting all `n`.
fn rank_leading(
    indices: &mut [usize],
    leading: usize,
    mut compare: impl FnMut(&usize, &usize) -> Ordering,
) {
    if leading == 0 {
        return;
    }
    if leading < indices.len() {
        indices.select_nth_unstable_by(leading - 1, &mut compare);
        indices[..leading].sort_unstable_by(compare);
    } else {
        indices.sort_unstable_by(compare);
    }
}

/// Applies a permutation to `slice` in-place using cycle decomposition.
///
/// `perm[i]` is the original index of the element that belongs at position `i`
/// in the output. Each cycle in the permutation is resolved by swapping elements
/// along the cycle, which requires only O(n) `usize` bookkeeping with no heap
/// allocation for the elements themselves.
///
/// `perm` is consumed (all entries set to their own index) as a side-effect.
fn apply_permutation_in_place<T>(slice: &mut [T], perm: &mut [usize]) {
    debug_assert_eq!(slice.len(), perm.len());
    for i in 0..perm.len() {
        let mut j = i;
        while perm[j] != i {
            let k = perm[j];
            slice.swap(j, k);
            perm[j] = j;
            j = k;
        }
        perm[j] = j;
    }
}

/// Returns the first similarity ORDER BY column's scores in output order,
/// with whether that column recomputed them (`similarity(field, $v)`) rather
/// than reading the search score back (`similarity()`); `None` when the query
/// has no similarity ORDER BY column.
///
/// Must be called before `apply_permutation_in_place` because that call
/// overwrites `indices` (the original-position mapping).
fn extract_sorted_similarity_scores(
    order_by: &[crate::velesql::SelectOrderBy],
    similarity_scores_map: &SimilarityKeys,
    indices: &[usize],
) -> Option<(Vec<Option<f32>>, bool)> {
    use crate::velesql::OrderByExpr;
    let (sim_idx, recomputed) =
        order_by
            .iter()
            .enumerate()
            .find_map(|(idx, ob)| match ob.expr {
                OrderByExpr::Similarity(_) => Some((idx, true)),
                OrderByExpr::SimilarityBare => Some((idx, false)),
                _ => None,
            })?;

    let scores = similarity_scores_map.get(&sim_idx)?;
    Some((
        indices.iter().map(|&orig| scores[orig]).collect(),
        recomputed,
    ))
}

/// Writes each row's ORDER BY similarity score back as its score.
///
/// A recomputed `similarity(field, $v)` score is a vector similarity, so a
/// row that carries no component breakdown — a metadata row, which only this
/// ORDER BY scored — is tagged with it as its `vector_score`, as a `NEAR`
/// row is. A row that already carries one (a `NEAR`, a hybrid) keeps it: its
/// components describe the search that found it. A row the column could not
/// score (a null key) keeps the score its plan gave it, untagged: no
/// non-finite stand-in is ever written back.
fn write_back_similarity_scores(
    results: &mut [SearchResult],
    scores: Vec<Option<f32>>,
    recomputed: bool,
) {
    for (result, score) in results.iter_mut().zip(scores) {
        let Some(score) = score.filter(|score| score.is_finite()) else {
            continue;
        };
        result.score = score;
        if recomputed && result.component_scores.is_none() {
            result.component_scores = Some(smallvec::smallvec![("vector_score", score)]);
        }
    }
}
