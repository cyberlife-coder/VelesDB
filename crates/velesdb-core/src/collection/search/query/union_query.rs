//! Union query execution for similarity() OR metadata patterns (EPIC-044 US-002).
//!
//! Handles OR-based queries that combine vector similarity with metadata filters,
//! including nested AND/OR patterns.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

/// Maximum allowed LIMIT value (re-imported from parent for local use).
const MAX_LIMIT: usize = 100_000;

impl Collection {
    /// EPIC-044 US-002: Execute union query for similarity() OR metadata patterns.
    ///
    /// This method handles queries like:
    /// `WHERE similarity(v, $v) > 0.8 OR category = 'tech'`
    ///
    /// Issue #122: Also handles nested patterns like:
    /// `WHERE (similarity(v, $v) > 0.8 OR category = 'tech') AND status = 'active'`
    ///
    /// It executes:
    /// 1. Vector search for similarity matches
    /// 2. Metadata scan for non-similarity matches
    /// 3. Apply outer AND filters to both result sets
    /// 4. Merges results with deduplication (by point ID)
    ///
    /// Scoring:
    /// - Similarity matches: use similarity score
    /// - Metadata-only matches: use score 1.0
    /// - Both matching: use similarity score (higher priority)
    pub(crate) fn execute_union_query(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        use std::collections::HashMap;

        // Issue #122: Extract similarity, metadata, AND outer filter from condition
        let (similarity_cond, metadata_cond, outer_filter) =
            Self::split_or_condition_with_outer_filter(condition);

        let mut results_map: HashMap<u64, SearchResult> = HashMap::new();

        // 1. Execute similarity search if we have a similarity condition
        if let Some(sim_cond) = similarity_cond {
            self.collect_similarity_results(
                &sim_cond,
                params,
                limit,
                outer_filter.as_ref(),
                &mut results_map,
            )?;
        }

        // 2. Execute metadata scan if we have a metadata condition
        if let Some(meta_cond) = metadata_cond {
            self.collect_metadata_results(
                meta_cond,
                outer_filter.as_ref(),
                limit,
                &mut results_map,
            );
        }

        // 3. Collect and return results
        let mut results: Vec<SearchResult> = results_map.into_values().collect();

        // Sort by score descending (similarity matches first)
        results.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    /// Collects similarity search results into the results map, applying
    /// optional outer filter.
    fn collect_similarity_results(
        &self,
        sim_cond: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        outer_filter: Option<&crate::velesql::Condition>,
        results_map: &mut std::collections::HashMap<u64, SearchResult>,
    ) -> Result<()> {
        let similarity_conditions = self.extract_all_similarity_conditions(sim_cond, params)?;
        if let Some((field, vec, op, threshold)) = similarity_conditions.first() {
            let overfetch_factor = 10;
            let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
            let candidates = self.search(vec, candidates_k)?;

            let filter_k = limit.saturating_mul(2);
            let filtered =
                self.filter_by_similarity(candidates, field, vec, *op, *threshold, filter_k);

            for result in filtered {
                if let Some(outer) = outer_filter {
                    if !Self::matches_metadata_filter(&result.point, outer) {
                        continue;
                    }
                }
                results_map.insert(result.point.id, result);
            }
        }
        Ok(())
    }

    /// Collects metadata scan results into the results map, combining with
    /// optional outer filter. Existing entries (from similarity) are preserved.
    fn collect_metadata_results(
        &self,
        meta_cond: crate::velesql::Condition,
        outer_filter: Option<&crate::velesql::Condition>,
        limit: usize,
        results_map: &mut std::collections::HashMap<u64, SearchResult>,
    ) {
        let combined_cond = match outer_filter {
            Some(outer) => {
                crate::velesql::Condition::And(Box::new(meta_cond), Box::new(outer.clone()))
            }
            None => meta_cond,
        };
        let filter =
            crate::filter::Filter::new(crate::filter::Condition::from(combined_cond.clone()));
        let metadata_results = self.execute_scan_query(&filter, limit, Some(&combined_cond));

        for result in metadata_results {
            // Only add if not already found by similarity search
            // If already present, keep the similarity score (higher priority)
            results_map.entry(result.point.id).or_insert(result);
        }
    }

    /// Check if a point matches a metadata filter condition.
    /// Used for applying outer AND filters to similarity results.
    pub(crate) fn matches_metadata_filter(
        point: &crate::Point,
        condition: &crate::velesql::Condition,
    ) -> bool {
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(condition.clone()));
        match point.payload.as_ref() {
            Some(payload) => filter.matches(payload),
            None => false, // No payload means filter doesn't match
        }
    }

    /// Split an OR condition into similarity and metadata parts, extracting outer AND filters.
    ///
    /// For `similarity() > 0.8 OR category = 'tech'`, returns:
    /// - similarity_cond: Some(similarity() > 0.8)
    /// - metadata_cond: Some(category = 'tech')
    /// - outer_filter: None
    ///
    /// For `(similarity() > 0.8 OR category = 'tech') AND status = 'active'`, returns:
    /// - similarity_cond: Some(similarity() > 0.8)
    /// - metadata_cond: Some(category = 'tech')
    /// - outer_filter: Some(status = 'active')
    ///
    /// Issue #122: Handle nested AND/OR patterns correctly.
    pub(crate) fn split_or_condition_with_outer_filter(
        condition: &crate::velesql::Condition,
    ) -> (
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
    ) {
        match condition {
            crate::velesql::Condition::Or(left, right) => {
                Self::split_top_level_or(condition, left, right)
            }
            crate::velesql::Condition::And(left, right) => {
                Self::split_top_level_and(condition, left, right)
            }
            crate::velesql::Condition::Group(inner) => {
                // Unwrap group and recurse
                Self::split_or_condition_with_outer_filter(inner)
            }
            // Not an OR or AND condition - treat as similarity if it contains similarity
            _ => Self::classify_leaf_condition(condition),
        }
    }

    /// Handles a top-level `OR`: routes the similarity-bearing side to the
    /// similarity slot and the other to the metadata slot (Issue #122).
    fn split_top_level_or(
        condition: &crate::velesql::Condition,
        left: &crate::velesql::Condition,
        right: &crate::velesql::Condition,
    ) -> (
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
    ) {
        let left_has_sim = Self::count_similarity_conditions(left) > 0;
        let right_has_sim = Self::count_similarity_conditions(right) > 0;
        match (left_has_sim, right_has_sim) {
            (true, false) => (Some(left.clone()), Some(right.clone()), None),
            (false, true) => (Some(right.clone()), Some(left.clone()), None),
            _ => (Some(condition.clone()), None, None),
        }
    }

    /// Handles a top-level `AND`: when exactly one side carries a problematic
    /// OR, recurse into it and fold the other side into the outer filter.
    fn split_top_level_and(
        condition: &crate::velesql::Condition,
        left: &crate::velesql::Condition,
        right: &crate::velesql::Condition,
    ) -> (
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
    ) {
        match (
            Self::has_similarity_in_problematic_or(left),
            Self::has_similarity_in_problematic_or(right),
        ) {
            (true, false) => {
                let (sim, meta, inner_filter) = Self::split_or_condition_with_outer_filter(left);
                (
                    sim,
                    meta,
                    Some(Self::combine_outer_filter(inner_filter, right, true)),
                )
            }
            (false, true) => {
                let (sim, meta, inner_filter) = Self::split_or_condition_with_outer_filter(right);
                (
                    sim,
                    meta,
                    Some(Self::combine_outer_filter(inner_filter, left, false)),
                )
            }
            // Both or neither - treat as a leaf.
            _ => Self::classify_leaf_condition(condition),
        }
    }

    /// Combines a recursively-extracted inner filter with the AND's other side,
    /// preserving operand order (`inner_on_left` keeps the inner condition left).
    fn combine_outer_filter(
        inner_filter: Option<crate::velesql::Condition>,
        other: &crate::velesql::Condition,
        inner_on_left: bool,
    ) -> crate::velesql::Condition {
        match inner_filter {
            Some(inner) => {
                let (l, r) = if inner_on_left {
                    (Box::new(inner), Box::new(other.clone()))
                } else {
                    (Box::new(other.clone()), Box::new(inner))
                };
                crate::velesql::Condition::And(l, r)
            }
            None => other.clone(),
        }
    }

    /// Classifies a non-OR/AND condition: similarity slot if it contains a
    /// similarity predicate, otherwise the metadata slot.
    fn classify_leaf_condition(
        condition: &crate::velesql::Condition,
    ) -> (
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
    ) {
        if Self::count_similarity_conditions(condition) > 0 {
            (Some(condition.clone()), None, None)
        } else {
            (None, Some(condition.clone()), None)
        }
    }
}
