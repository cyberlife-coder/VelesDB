//! Tests for the BM25 term dictionary, the versioned snapshot parser, and
//! the v1 → v2 migration (#2090).

use super::bm25::{Bm25Index, Bm25Params, BM25_SNAPSHOT_VERSION};
use super::bm25_term_dict::{parse_snapshot, TermDict};
use rustc_hash::FxHashMap;
use serde::Serialize;

/// Serializable mirror of the version-1 document wire shape. The real
/// [`super::bm25_term_dict::DocumentV1`] is deserialize-only, so tests
/// forge legacy bytes through this twin (postcard is positional: same
/// field order ⇒ same bytes).
#[derive(Serialize)]
struct DocV1Wire {
    term_freqs: FxHashMap<String, u32>,
    length: u32,
}

/// Serializable mirror of the version-1 snapshot wire shape.
#[derive(Serialize)]
struct SnapV1Wire {
    version: u32,
    params: Bm25Params,
    documents: FxHashMap<u64, DocV1Wire>,
    point_to_doc: FxHashMap<u64, u32>,
    doc_to_point: FxHashMap<u32, u64>,
    free_doc_ids: Vec<u32>,
    next_doc_id: u32,
    doc_count: usize,
    total_doc_length: u64,
}

const CORPUS: &[(u64, &str)] = &[
    (10, "rust programming language for systems programming"),
    (20, "python programming for data science"),
    (30, "database systems in rust"),
    (40, "graph database with vector search"),
];

const QUERIES: &[&str] = &[
    "rust database",
    "programming",
    "vector search systems",
    "rust rust programming", // duplicate query term
    "unknown_term rust",     // term absent from the corpus
    "totally unknown terms", // nothing matches
];

/// Replicates what the pre-#2090 `add_document` persisted, producing
/// version-1 snapshot bytes for the given corpus.
fn v1_snapshot_bytes(corpus: &[(u64, &str)]) -> Vec<u8> {
    let mut documents = FxHashMap::default();
    let mut point_to_doc = FxHashMap::default();
    let mut doc_to_point = FxHashMap::default();
    let mut total_doc_length = 0u64;
    for (i, (point_id, text)) in corpus.iter().enumerate() {
        let doc_id = u32::try_from(i).unwrap();
        let tokens = Bm25Index::tokenize(text);
        let mut term_freqs: FxHashMap<String, u32> = FxHashMap::default();
        for token in &tokens {
            *term_freqs.entry(token.clone()).or_insert(0) += 1;
        }
        let length = u32::try_from(tokens.len()).unwrap();
        total_doc_length += u64::from(length);
        documents.insert(*point_id, DocV1Wire { term_freqs, length });
        point_to_doc.insert(*point_id, doc_id);
        doc_to_point.insert(doc_id, *point_id);
    }
    let snap = SnapV1Wire {
        version: 1,
        params: Bm25Params::default(),
        documents,
        point_to_doc,
        doc_to_point,
        free_doc_ids: Vec::new(),
        next_doc_id: u32::try_from(corpus.len()).unwrap(),
        doc_count: corpus.len(),
        total_doc_length,
    };
    postcard::to_allocvec(&snap).unwrap()
}

/// Builds a fresh index through the public ingestion path.
fn fresh_index(corpus: &[(u64, &str)]) -> Bm25Index {
    let index = Bm25Index::new();
    for (point_id, text) in corpus {
        index.add_document(*point_id, text);
    }
    index
}

/// Asserts two indexes return identical results — same documents in the
/// same order with bit-identical scores — for every probe query.
fn assert_search_identical(a: &Bm25Index, b: &Bm25Index) {
    for query in QUERIES {
        let ra = a.search(query, 10);
        let rb = b.search(query, 10);
        let ka: Vec<(u64, u32)> = ra.iter().map(|(id, s)| (*id, s.to_bits())).collect();
        let kb: Vec<(u64, u32)> = rb.iter().map(|(id, s)| (*id, s.to_bits())).collect();
        assert_eq!(ka, kb, "results diverge for query {query:?}");
    }
}

#[test]
fn test_term_dict_intern_is_idempotent_and_positional() {
    let mut dict = TermDict::default();
    let a = dict.intern("alpha").unwrap();
    let b = dict.intern("beta").unwrap();
    assert_ne!(a, b);
    assert_eq!(dict.intern("alpha"), Some(a));
    assert_eq!(dict.get("alpha"), Some(a));
    assert_eq!(dict.get("gamma"), None);
    assert_eq!(
        dict.to_strings(),
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

#[test]
fn test_term_dict_string_roundtrip_preserves_ids() {
    let mut dict = TermDict::default();
    let ids: Vec<_> = ["one", "two", "three"]
        .iter()
        .map(|t| dict.intern(t).unwrap())
        .collect();
    let rebuilt = TermDict::from_strings(dict.to_strings());
    for (term, id) in ["one", "two", "three"].iter().zip(ids) {
        assert_eq!(rebuilt.get(term), Some(id), "id moved for {term:?}");
    }
    assert_eq!(rebuilt.to_strings(), dict.to_strings());
}

#[test]
fn test_current_snapshot_roundtrips_through_parse() {
    let index = fresh_index(CORPUS);
    let bytes = postcard::to_allocvec(&index.to_snapshot()).unwrap();
    let restored = Bm25Index::from_snapshot(parse_snapshot(&bytes).unwrap()).unwrap();
    assert_eq!(restored.len(), index.len());
    assert_eq!(restored.term_count(), index.term_count());
    assert_search_identical(&index, &restored);
}

#[test]
fn test_v1_snapshot_migrates_and_scores_identically() {
    let bytes = v1_snapshot_bytes(CORPUS);
    let snapshot = parse_snapshot(&bytes).unwrap();
    assert_eq!(snapshot.version, BM25_SNAPSHOT_VERSION);
    let migrated = Bm25Index::from_snapshot(snapshot).unwrap();
    let reference = fresh_index(CORPUS);
    assert_eq!(migrated.len(), reference.len());
    assert_eq!(migrated.term_count(), reference.term_count());
    assert_search_identical(&migrated, &reference);
}

#[test]
fn test_migrated_index_keeps_ingesting() {
    let migrated =
        Bm25Index::from_snapshot(parse_snapshot(&v1_snapshot_bytes(CORPUS)).unwrap()).unwrap();
    // New document mixing known terms (must reuse migrated ids) and a new
    // one (must intern past the migrated dictionary without collision).
    migrated.add_document(50, "rust database sharding");
    let results = migrated.search("sharding", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 50);
    // The updated corpus must behave exactly like one built from scratch.
    let mut corpus: Vec<(u64, &str)> = CORPUS.to_vec();
    corpus.push((50, "rust database sharding"));
    assert_search_identical(&migrated, &fresh_index(&corpus));
}

#[test]
fn test_unknown_snapshot_version_is_rejected() {
    let index = fresh_index(CORPUS);
    let mut snapshot = index.to_snapshot();
    snapshot.version = 99;
    let bytes = postcard::to_allocvec(&snapshot).unwrap();
    let err = parse_snapshot(&bytes).unwrap_err();
    assert!(
        err.to_string().contains("version mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_garbage_bytes_are_rejected() {
    assert!(parse_snapshot(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]).is_err());
}
