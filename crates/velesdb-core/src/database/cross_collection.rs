//! Cross-collection MATCH enrichment (Issue #495 Phase 2).
//!
//! After a MATCH query executes on the primary collection (the one with
//! graph edges), this module enriches results with payloads from other
//! collections referenced via `@collection` annotations on node patterns.

use crate::point::SearchResult;

impl super::Database {
    /// Enriches MATCH results with payloads from cross-collection node annotations.
    ///
    /// When a node pattern has `collection: Some("other_coll")`, this method
    /// looks up the node's payload from `other_coll` and merges the fields
    /// into the result's projected data under the node's alias prefix.
    ///
    /// No-op if no node patterns have collection annotations.
    pub(super) fn enrich_match_results_cross_collection(
        &self,
        match_clause: &crate::velesql::MatchClause,
        results: &mut [SearchResult],
    ) {
        let cross_refs: Vec<(&str, &str)> = match_clause
            .patterns
            .iter()
            .flat_map(|p| p.nodes.iter())
            .filter_map(|n| {
                let alias = n.alias.as_deref()?;
                let coll = n.collection.as_deref()?;
                Some((alias, coll))
            })
            .collect();

        if cross_refs.is_empty() {
            return;
        }

        for (alias, coll_name) in &cross_refs {
            let Ok(coll) = self.resolve_collection(coll_name) else {
                tracing::warn!(
                    collection = coll_name,
                    alias = alias,
                    "cross-collection enrichment: collection not found, skipping"
                );
                continue;
            };

            enrich_results_from_collection(&coll, results, alias);
        }
    }
}

/// Enriches all results from a single cross-referenced collection.
#[allow(deprecated)]
fn enrich_results_from_collection(
    coll: &crate::Collection,
    results: &mut [SearchResult],
    alias: &str,
) {
    for result in results.iter_mut() {
        if let Some(id) = extract_binding_id(result, alias) {
            if let Some(point) = coll.get(&[id]).into_iter().flatten().next() {
                if let Some(payload) = &point.payload {
                    merge_cross_payload(result, alias, payload);
                }
            }
        }
    }
}

/// Extracts a node ID from a result's `_bindings` map.
fn extract_binding_id(result: &SearchResult, alias: &str) -> Option<u64> {
    result
        .point
        .payload
        .as_ref()?
        .get("_bindings")?
        .get(alias)?
        .as_u64()
}

/// Merges cross-collection payload fields into a result's payload.
///
/// Fields are prefixed with `alias.` to avoid collisions with the
/// primary collection's fields.
fn merge_cross_payload(result: &mut SearchResult, alias: &str, payload: &serde_json::Value) {
    if let Some(ref mut existing) = result.point.payload {
        if let Some(obj) = existing.as_object_mut() {
            if let Some(cross_obj) = payload.as_object() {
                for (key, value) in cross_obj {
                    let prefixed_key = format!("{alias}.{key}");
                    obj.insert(prefixed_key, value.clone());
                }
            }
        }
    }
}
