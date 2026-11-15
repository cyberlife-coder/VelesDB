//! The collections a MATCH reads besides its own target, and the one place
//! their locks are taken.
//!
//! `(w:Warehouse@pricing)` binds a node of the MATCH's own graph whose
//! properties are the union of its own payload and the payload of the
//! `pricing` point with the same id — the node's own fields winning, so its
//! identity and labels are never overwritten. WHERE, ORDER BY, RETURN and
//! the pattern's inline properties all read that one union through
//! [`super::PayloadMemo`]. Labels are the graph's: they are checked on the
//! node's own payload, so an annotation never makes a node match a label.
//!
//! # Lock order across collections
//!
//! Reading `pricing` during the walk means holding a read guard of a second
//! collection next to the target's own. Every MATCH takes all of them through
//! [`MatchReads::acquire`], in one total order: collection *instance*
//! (the address of its `payload_storage`), then lock rank — `vector_storage`
//! (2) before `payload_storage` (3) within a collection. A thread therefore
//! never waits for a lower-ordered lock while holding a higher one, which is
//! what rules out the cycle two cross-annotating MATCH queries with queued writers
//! would otherwise form. The handles are resolved by the caller *before*
//! [`MatchReads::acquire`] — never under a held guard — because resolving a
//! name can take the database's collection-registry lock, which is ordered
//! before every collection lock. A collection is locked once however many
//! annotations name it, and never when it is the target itself:
//! `parking_lot`'s `RwLock` is not re-entrant. See
//! `docs/CONCURRENCY_MODEL.md`.

use std::sync::Arc;

use crate::collection::types::Collection;
use crate::error::Result;
use crate::lock_audit::ReadGuard;
use crate::storage::{LogPayloadStorage, MmapStorage};
use crate::velesql::MatchClause;

/// The `@collection` annotations of a MATCH: each annotated collection under
/// the name the annotation gives, and the alias of each annotated node,
/// resolved by the database before any guard is taken.
///
/// Empty for the collection-level entry points, which cannot see other
/// collections: there an annotated node reads its own payload only.
#[derive(Clone, Default)]
pub(crate) struct MatchSources {
    /// `(annotation name, collection)`, one entry per distinct name.
    collections: Vec<(String, Collection)>,
    /// `(node alias, annotation name)` for each aliased annotated node.
    aliases: Vec<(String, String)>,
}

impl MatchSources {
    /// The sources of `clause`'s annotations, each name resolved by
    /// `resolve` once. A clause without annotations resolves nothing.
    ///
    /// # Errors
    ///
    /// Returns the error `resolve` returns for a name, e.g. a collection that
    /// does not exist: a MATCH cannot read properties from it.
    pub(crate) fn for_clause(
        clause: &MatchClause,
        mut resolve: impl FnMut(&str) -> Result<Collection>,
    ) -> Result<Self> {
        let mut sources = Self::default();
        for node in clause.patterns.iter().flat_map(|p| p.nodes.iter()) {
            let Some(name) = node.collection.as_deref() else {
                continue;
            };
            if sources.collections.iter().all(|(n, _)| n != name) {
                sources.collections.push((name.to_string(), resolve(name)?));
            }
            if let Some(alias) = node.alias.as_deref() {
                if sources.aliases.iter().all(|(a, _)| a != alias) {
                    sources.aliases.push((alias.to_string(), name.to_string()));
                }
            }
        }
        Ok(sources)
    }
}

/// Identity of a collection instance for the lock order: the address of
/// its `payload_storage`, shared by every handle cloned from it.
fn instance_key(collection: &Collection) -> usize {
    Arc::as_ptr(&collection.storage.payload_storage).cast::<()>() as usize
}

/// Every storage read guard one MATCH execution holds: the target's vector
/// and payload storage, and the payload storage of each distinct annotated
/// collection, taken in the total order the module docs set out.
pub(in crate::collection::search::query) struct MatchReads<'s> {
    pub(in crate::collection::search::query) vector: ReadGuard<'s, MmapStorage>,
    pub(in crate::collection::search::query) payload: ReadGuard<'s, LogPayloadStorage>,
    /// One guard per distinct annotated collection instance other than the
    /// target, in acquisition order.
    others: Vec<ReadGuard<'s, LogPayloadStorage>>,
    /// Annotation name → where its payload is read: `None` for the target
    /// itself, else an index into `others`.
    names: Vec<(&'s str, Option<usize>)>,
    sources: &'s MatchSources,
}

impl<'s> MatchReads<'s> {
    /// Acquires the target's guards and each annotated collection's payload
    /// guard in (instance, rank) order.
    #[must_use]
    pub(in crate::collection::search::query) fn acquire(
        target: &'s Collection,
        sources: &'s MatchSources,
    ) -> Self {
        let own = instance_key(target);
        // Distinct annotated instances other than the target, by key.
        let mut distinct: Vec<(usize, &'s Collection)> = Vec::new();
        for (_, collection) in &sources.collections {
            let key = instance_key(collection);
            if key != own && distinct.iter().all(|(k, _)| *k != key) {
                distinct.push((key, collection));
            }
        }
        distinct.sort_unstable_by_key(|(key, _)| *key);

        let below = distinct.partition_point(|(key, _)| *key < own);
        let mut others = Vec::with_capacity(distinct.len());
        for (_, collection) in &distinct[..below] {
            others.push(collection.storage.payload_storage.read());
        }
        let vector = target.storage.vector_storage.read();
        let payload = target.storage.payload_storage.read();
        for (_, collection) in &distinct[below..] {
            others.push(collection.storage.payload_storage.read());
        }

        let names = sources
            .collections
            .iter()
            .map(|(name, collection)| {
                let key = instance_key(collection);
                (name.as_str(), distinct.iter().position(|(k, _)| *k == key))
            })
            .collect();
        Self {
            vector,
            payload,
            others,
            names,
            sources,
        }
    }

    /// What [`super::PayloadMemo::with_annotations`] reads: each annotation
    /// name with its payload storage, and each annotated alias with its
    /// annotation name.
    pub(in crate::collection::search::query) fn annotations(&self) -> Annotations<'_> {
        Annotations {
            storages: self
                .names
                .iter()
                .map(|(name, index)| {
                    let storage = index.map_or(&*self.payload, |i| &*self.others[i]);
                    (*name, storage)
                })
                .collect(),
            aliases: self
                .sources
                .aliases
                .iter()
                .map(|(alias, name)| (alias.as_str(), name.as_str()))
                .collect(),
        }
    }
}

/// The annotation lookups a [`super::PayloadMemo`] answers from.
#[derive(Default)]
pub(in crate::collection::search::query) struct Annotations<'a> {
    /// `(annotation name, payload storage it reads)`.
    storages: Vec<(&'a str, &'a LogPayloadStorage)>,
    /// `(node alias, annotation name)`.
    aliases: Vec<(&'a str, &'a str)>,
}

impl<'a> Annotations<'a> {
    /// Index of the annotation `name`.
    pub(super) fn by_name(&self, name: &str) -> Option<usize> {
        self.storages.iter().position(|(n, _)| *n == name)
    }

    /// Index of the annotation the node `alias` carries.
    pub(super) fn by_alias(&self, alias: &str) -> Option<usize> {
        let (_, name) = self.aliases.iter().find(|(a, _)| *a == alias)?;
        self.by_name(name)
    }

    /// The payload storage of annotation `index`.
    pub(super) fn storage(&self, index: usize) -> &'a LogPayloadStorage {
        self.storages[index].1
    }
}

/// The union of a node's own payload and its annotated point's: the node's
/// own fields win, so an annotation adds properties and never overwrites
/// the node's identity or labels. Either side alone is returned as it is.
pub(super) fn union_payload(
    own: Option<Arc<serde_json::Value>>,
    annotated: Option<serde_json::Value>,
) -> Option<Arc<serde_json::Value>> {
    let Some(serde_json::Value::Object(extra)) = annotated else {
        return own;
    };
    let mut merged = match own.as_deref() {
        Some(serde_json::Value::Object(map)) => map.clone(),
        Some(_) => return own,
        None => serde_json::Map::new(),
    };
    for (key, value) in extra {
        merged.entry(key).or_insert(value);
    }
    Some(Arc::new(serde_json::Value::Object(merged)))
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
