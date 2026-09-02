//! Term dictionary for the BM25 index (#2090), plus the legacy snapshot
//! format it replaced and the migration from it.
//!
//! Before #2090 every [`Document`](super::bm25::Document) owned a `String`
//! per distinct term and the inverted index owned the same term again — the
//! corpus vocabulary materialized once per document. The dictionary stores
//! each term once as an `Arc<str>` shared between the id map's key and the
//! resolve table (the #2089 `LabelTable` treatment), and everything else
//! keys by [`TermId`].

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Interned id of a BM25 term. `u32` bounds the vocabulary at ~4 billion
/// distinct terms; the id is only meaningful against the index's own
/// [`TermDict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct TermId(u32);

/// Interning table mapping term strings to [`TermId`]s.
///
/// Grows monotonically: removing a document never un-interns its terms —
/// the vocabulary is bounded by the corpus's distinct-term count, which is
/// exactly what this table exists to stop paying per document.
#[derive(Debug, Default)]
pub(crate) struct TermDict {
    /// Interned terms indexed by [`TermId`]. Each entry shares its
    /// allocation with the `ids` key.
    terms: Vec<Arc<str>>,
    /// Reverse lookup: term -> id.
    ids: FxHashMap<Arc<str>, TermId>,
}

impl TermDict {
    /// Interns `term`, returning its id.
    ///
    /// Returns `None` when the dictionary is full (`u32::MAX` distinct
    /// terms) — the same silent-skip contract as BM25's doc-id overflow.
    pub(crate) fn intern(&mut self, term: &str) -> Option<TermId> {
        if let Some(&id) = self.ids.get(term) {
            return Some(id);
        }
        let id = TermId(u32::try_from(self.terms.len()).ok()?);
        let shared: Arc<str> = Arc::from(term);
        self.terms.push(Arc::clone(&shared));
        self.ids.insert(shared, id);
        Some(id)
    }

    /// Looks up `term` without interning.
    pub(crate) fn get(&self, term: &str) -> Option<TermId> {
        self.ids.get(term).copied()
    }

    /// Snapshot of the dictionary as plain strings, indexed by [`TermId`] —
    /// the wire representation.
    pub(crate) fn to_strings(&self) -> Vec<String> {
        self.terms.iter().map(ToString::to_string).collect()
    }

    /// Rebuilds a dictionary from its wire representation.
    ///
    /// Positions become ids, so a dictionary round-trips exactly.
    pub(crate) fn from_strings(terms: Vec<String>) -> Self {
        let mut dict = Self {
            terms: Vec::with_capacity(terms.len()),
            ids: FxHashMap::with_capacity_and_hasher(terms.len(), rustc_hash::FxBuildHasher),
        };
        for term in terms {
            let id = TermId(u32::try_from(dict.terms.len()).unwrap_or(u32::MAX));
            let shared: Arc<str> = Arc::from(term.as_str());
            dict.terms.push(Arc::clone(&shared));
            dict.ids.insert(shared, id);
        }
        dict
    }
}

// ---------------------------------------------------------------------------
// Legacy (version-1) snapshot wire format and its migration
// ---------------------------------------------------------------------------

use super::bm25::{Bm25Params, Bm25Snapshot, Document, BM25_SNAPSHOT_VERSION};

/// The version-1 document wire shape: term frequencies keyed by owned
/// `String`s — the layout #2090 retired.
#[derive(Debug, Deserialize)]
pub(crate) struct DocumentV1 {
    pub(crate) term_freqs: FxHashMap<String, u32>,
    pub(crate) length: u32,
}

/// The version-1 snapshot wire shape, kept only to read pre-#2090
/// `bm25.snapshot` files.
#[derive(Debug, Deserialize)]
pub(crate) struct Bm25SnapshotV1 {
    pub(crate) version: u32,
    pub(crate) params: Bm25Params,
    pub(crate) documents: FxHashMap<u64, DocumentV1>,
    pub(crate) point_to_doc: FxHashMap<u64, u32>,
    pub(crate) doc_to_point: FxHashMap<u32, u64>,
    pub(crate) free_doc_ids: Vec<u32>,
    pub(crate) next_doc_id: u32,
    pub(crate) doc_count: usize,
    pub(crate) total_doc_length: u64,
}

/// Migrates a version-1 snapshot to the current format by interning every
/// document's term keys into one dictionary.
///
/// Term-frequency VALUES and every counter are carried over untouched, so
/// scoring over a migrated snapshot is bit-identical to scoring over the
/// original (only the key representation moves — pinned by
/// `test_v1_snapshot_migrates_and_scores_identically`).
pub(crate) fn migrate_v1(v1: Bm25SnapshotV1) -> Bm25Snapshot {
    let mut dict = TermDict::default();
    let documents: FxHashMap<u64, Document> = v1
        .documents
        .into_iter()
        .map(|(point_id, doc)| {
            let term_freqs: FxHashMap<TermId, u32> = doc
                .term_freqs
                .into_iter()
                .filter_map(|(term, freq)| dict.intern(&term).map(|id| (id, freq)))
                .collect();
            (
                point_id,
                Document {
                    term_freqs,
                    length: doc.length,
                },
            )
        })
        .collect();
    Bm25Snapshot {
        version: BM25_SNAPSHOT_VERSION,
        params: v1.params,
        term_dict: dict.to_strings(),
        documents,
        point_to_doc: v1.point_to_doc,
        doc_to_point: v1.doc_to_point,
        free_doc_ids: v1.free_doc_ids,
        next_doc_id: v1.next_doc_id,
        doc_count: v1.doc_count,
        total_doc_length: v1.total_doc_length,
    }
}

/// Parses snapshot bytes, dispatching on the leading `version` field.
///
/// Postcard is positional, so the version is read first on its own and the
/// full buffer is then re-parsed with the matching struct (both wire shapes
/// begin with the same `u32 version` field).
///
/// # Errors
///
/// Returns [`crate::Error::IndexCorrupted`] for unknown versions and
/// undecodable bytes.
pub(crate) fn parse_snapshot(bytes: &[u8]) -> crate::Result<Bm25Snapshot> {
    let (version, _rest) = postcard::take_from_bytes::<u32>(bytes)
        .map_err(|e| crate::Error::IndexCorrupted(format!("BM25 snapshot header: {e}")))?;
    match version {
        1 => {
            let v1: Bm25SnapshotV1 = postcard::from_bytes(bytes).map_err(|e| {
                crate::Error::IndexCorrupted(format!("BM25 v1 snapshot deserialize: {e}"))
            })?;
            debug_assert_eq!(v1.version, 1);
            Ok(migrate_v1(v1))
        }
        BM25_SNAPSHOT_VERSION => postcard::from_bytes(bytes).map_err(|e| {
            crate::Error::IndexCorrupted(format!("BM25 snapshot deserialize: {e}"))
        }),
        other => Err(crate::Error::IndexCorrupted(format!(
            "BM25 snapshot version mismatch: got {other}, expected {BM25_SNAPSHOT_VERSION} (or 1 for migration)"
        ))),
    }
}
