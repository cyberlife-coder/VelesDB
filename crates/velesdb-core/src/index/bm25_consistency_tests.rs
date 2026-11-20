//! A score reads one consistent state of the index: the postings a term's
//! document frequency is read from and the corpus size and length it is
//! weighed against, never a term counted in more documents than the corpus
//! holds — which made every idf negative and dropped every hit.

use std::sync::Arc;

use super::bm25::{on_terms_resolved, Bm25Index};

fn ids(mut scored: Vec<(u64, f32)>) -> Vec<u64> {
    scored.sort_by_key(|&(id, _)| id);
    scored.into_iter().map(|(id, _)| id).collect()
}

/// Documents added while a score has resolved its terms but not read the
/// postings are read with the corpus that holds them: the score of every
/// match stays positive, and every match is a hit.
#[test]
fn documents_added_while_a_score_runs_are_weighed_against_the_corpus_holding_them() {
    let index = Arc::new(Bm25Index::new());
    index.add_document(1, "apple pie");
    let writer = Arc::clone(&index);
    on_terms_resolved(move || {
        for id in 2..=4 {
            writer.add_document(id, "apple tart");
        }
    });

    let scored = index.score_all("apple");

    assert!(scored.iter().all(|&(_, score)| score > 0.0), "{scored:?}");
    assert_eq!(ids(scored), [1, 2, 3, 4]);
}

/// Documents removed while a score runs leave the corpus and the postings
/// together too: the documents left are all hits.
#[test]
fn documents_removed_while_a_score_runs_leave_the_rest_hits() {
    let index = Arc::new(Bm25Index::new());
    for id in 1..=4 {
        index.add_document(id, "apple tart");
    }
    index.add_document(5, "pear");
    let writer = Arc::clone(&index);
    on_terms_resolved(move || {
        writer.remove_document(5);
        writer.remove_document(4);
    });

    assert_eq!(ids(index.score_all("apple")), [1, 2, 3]);
}

/// Under writers adding and removing documents that hold the term, every
/// score of it finds every document that holds it throughout.
#[test]
fn concurrent_writes_never_hide_a_stable_document() {
    let index = Arc::new(Bm25Index::new());
    for id in 1..=5 {
        index.add_document(id, "apple stable");
    }
    let writers: Vec<_> = (0..2_u64)
        .map(|w| {
            let index = Arc::clone(&index);
            std::thread::spawn(move || {
                for round in 0..2_000_u64 {
                    let id = 100 + w * 1_000 + round % 50;
                    if round % 3 == 0 {
                        index.remove_document(id);
                    } else {
                        index.add_document(id, "apple churn apple");
                    }
                }
            })
        })
        .collect();

    let mut misses = 0;
    for _ in 0..4_000 {
        let found = ids(index.score_all("apple"));
        if !(1..=5).all(|id| found.contains(&id)) {
            misses += 1;
        }
    }
    for writer in writers {
        writer.join().expect("test: writer");
    }

    assert_eq!(misses, 0, "a score missed a stable document");
}
