//! The `@collection` annotations of a browser MATCH, read as core reads them.
//!
//! `(w:Warehouse@pricing)` binds a node of the MATCH's own graph whose
//! properties are the union of its own payload and the payload of the
//! `pricing` point with the same id — the node's own fields winning, so its
//! identity and labels are never overwritten. Labels stay the graph's. As in
//! core (`match_exec::sources` in `velesdb-core`):
//!
//! - a pattern node's inline properties read its own position's annotation
//!   ([`Annotations::properties`]);
//! - a named node read by WHERE, ORDER BY or RETURN (`w.price`) reads the
//!   annotation its alias carries at any occurrence — the first annotated
//!   one — so `(a@pricing)-[:R]->(a)` binds one `a`
//!   ([`Annotations::bound_properties`]);
//! - a bare WHERE column reads the target's own payload, no annotation;
//! - an annotation naming no collection refuses the MATCH.

use std::collections::HashMap;

use velesdb_core::velesql::{MatchClause, NodePattern};

use crate::database::{DatabaseInner, SharedStore};

/// One annotated collection: its store and the row of each of its ids.
struct Annotated {
    store: SharedStore,
    rows: HashMap<u64, usize>,
}

/// Every collection a MATCH's annotations name, resolved once per MATCH.
#[derive(Default)]
pub(crate) struct Annotations {
    /// `(annotation name, collection)`, one entry per distinct name.
    collections: Vec<(String, Annotated)>,
    /// `(node alias, annotation name)`: each aliased annotated node's first
    /// annotation, as core's `MatchSources::for_clause` records it.
    aliases: Vec<(String, String)>,
}

impl Annotations {
    /// The collections `clause`'s annotations name.
    ///
    /// # Errors
    ///
    /// Names an annotation that names no collection: a MATCH cannot read
    /// properties from it.
    pub(crate) fn resolve(db: &DatabaseInner, clause: &MatchClause) -> Result<Self, String> {
        let mut annotations = Self::default();
        for node in clause.patterns.iter().flat_map(|p| p.nodes.iter()) {
            let Some(name) = node.collection.as_deref() else {
                continue;
            };
            if let Some(alias) = node.alias.as_deref() {
                if annotations.aliases.iter().all(|(a, _)| a != alias) {
                    annotations
                        .aliases
                        .push((alias.to_string(), name.to_string()));
                }
            }
            if annotations.collections.iter().any(|(n, _)| n == name) {
                continue;
            }
            let store = db
                .get_shared_store(name)
                .map_err(|_| format!("[VELES-002] Collection '{name}' not found"))?;
            let rows = store
                .borrow()
                .ids
                .iter()
                .enumerate()
                .map(|(row, &id)| (id, row))
                .collect();
            annotations
                .collections
                .push((name.to_string(), Annotated { store, rows }));
        }
        Ok(annotations)
    }

    /// The properties of node `id` as the pattern node `node` checks its
    /// inline properties against: its own payload `own`, united with the
    /// point of the collection `node` itself is annotated with (the node's
    /// own fields win).
    pub(crate) fn properties(
        &self,
        node: &NodePattern,
        id: u64,
        own: Option<&serde_json::Value>,
    ) -> Option<serde_json::Value> {
        self.united(node.collection.as_deref(), id, own)
    }

    /// The properties of node `id` as WHERE, ORDER BY and RETURN read the
    /// node `alias` names: united with the annotation that alias carries at
    /// any occurrence (core's `PayloadMemo::get_bound`); an unnamed node's
    /// own payload.
    pub(crate) fn bound_properties(
        &self,
        alias: Option<&str>,
        id: u64,
        own: Option<&serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let name = alias.and_then(|alias| {
            self.aliases
                .iter()
                .find(|(a, _)| a == alias)
                .map(|(_, name)| name.as_str())
        });
        self.united(name, id, own)
    }

    /// `own` united with annotation `name`'s point `id`, when `name` is one.
    fn united(
        &self,
        name: Option<&str>,
        id: u64,
        own: Option<&serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let Some(annotated) = name
            .and_then(|name| self.collections.iter().find(|(n, _)| n == name))
            .map(|(_, annotated)| annotated)
        else {
            return own.cloned();
        };
        let store = annotated.store.borrow();
        let extra = annotated
            .rows
            .get(&id)
            .and_then(|&row| store.payloads.get(row))
            .and_then(Option::as_ref);
        union(own, extra)
    }
}

/// The union of a node's own payload and its annotated point's: the node's
/// own fields win. Either side alone is returned as it is; a non-object side
/// never merges.
fn union(
    own: Option<&serde_json::Value>,
    annotated: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let Some(serde_json::Value::Object(extra)) = annotated else {
        return own.cloned();
    };
    let mut merged = match own {
        Some(serde_json::Value::Object(map)) => map.clone(),
        Some(other) => return Some(other.clone()),
        None => serde_json::Map::new(),
    };
    for (key, value) in extra {
        merged.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Some(serde_json::Value::Object(merged))
}

#[cfg(test)]
#[path = "velesql_match_sources_tests.rs"]
mod tests;
