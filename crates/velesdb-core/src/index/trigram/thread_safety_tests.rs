//! Thread-Safety Tests for Trigram Index (US-CORE-003-11)
//!
//! Validates concurrent access patterns and absence of data races.
//! Run with: cargo test --package velesdb-core trigram::thread_safety

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::RwLock;

use super::index::TrigramIndex;

/// Thread-safe wrapper for TrigramIndex
pub struct ConcurrentTrigramIndex {
    inner: RwLock<TrigramIndex>,
}

impl ConcurrentTrigramIndex {
    /// Create a new concurrent trigram index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(TrigramIndex::new()),
        }
    }

    /// Insert a document (write lock).
    pub fn insert(&self, doc_id: u64, text: &str) {
        self.inner.write().insert(doc_id, text);
    }

    /// Remove a document (write lock).
    pub fn remove(&self, doc_id: u64) {
        self.inner.write().remove(doc_id);
    }

    /// Search for documents (read lock).
    #[must_use]
    pub fn search_like(&self, pattern: &str) -> roaring::RoaringBitmap {
        self.inner.read().search_like(pattern)
    }

    /// Get document count (read lock).
    #[must_use]
    pub fn doc_count(&self) -> u64 {
        self.inner.read().stats().doc_count
    }
}

impl Default for ConcurrentTrigramIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ========== Thread-Safety Tests ==========

#[test]
fn test_concurrent_inserts() {
    let index = Arc::new(ConcurrentTrigramIndex::new());
    let mut handles = vec![];

    // 4 threads, each inserting 100 documents
    for t in 0..4 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let doc_id = t * 1000 + i;
                let text = format!("document {doc_id} from thread {t}");
                index_clone.insert(doc_id, &text);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // All 400 documents should be indexed
    assert_eq!(index.doc_count(), 400);
}

#[test]
fn test_concurrent_reads() {
    let index = Arc::new(ConcurrentTrigramIndex::new());

    // Pre-populate
    for i in 0..100 {
        index.insert(i, &format!("document number {i}"));
    }

    let mut handles = vec![];

    // 8 threads doing concurrent reads
    for _ in 0..8 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let results = index_clone.search_like("document");
                assert!(!results.is_empty());
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_concurrent_read_write() {
    let index = Arc::new(ConcurrentTrigramIndex::new());

    // Pre-populate
    for i in 0..50 {
        index.insert(i, &format!("initial document {i}"));
    }

    let mut handles = vec![];

    // 2 writer threads
    for t in 0..2 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                let doc_id = 1000 + t * 100 + i;
                index_clone.insert(doc_id, &format!("new document {doc_id}"));
            }
        }));
    }

    // 4 reader threads
    for _ in 0..4 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                let _ = index_clone.search_like("document");
                let _ = index_clone.doc_count();
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Should have initial 50 + 2*50 = 150 documents
    assert_eq!(index.doc_count(), 150);
}

#[test]
fn test_concurrent_insert_remove() {
    let index = Arc::new(ConcurrentTrigramIndex::new());

    // Pre-populate with documents 0-99
    for i in 0..100 {
        index.insert(i, &format!("document {i}"));
    }

    let mut handles = vec![];

    // Thread 1: remove even documents
    let index_clone = Arc::clone(&index);
    handles.push(thread::spawn(move || {
        for i in (0..100).step_by(2) {
            index_clone.remove(i);
        }
    }));

    // Thread 2: insert new documents
    let index_clone = Arc::clone(&index);
    handles.push(thread::spawn(move || {
        for i in 100..150 {
            index_clone.insert(i, &format!("new document {i}"));
        }
    }));

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Should have 50 odd + 50 new = 100 documents
    assert_eq!(index.doc_count(), 100);
}

#[test]
fn test_stress_many_threads() {
    let index = Arc::new(ConcurrentTrigramIndex::new());
    let mut handles = vec![];

    // 16 threads doing mixed operations
    for t in 0..16 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                let doc_id = t * 100 + i;

                // Insert
                index_clone.insert(doc_id, &format!("stress test doc {doc_id}"));

                // Read
                let _ = index_clone.search_like("stress");

                // Sometimes remove
                if i % 3 == 0 {
                    index_clone.remove(doc_id);
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Index should still be consistent (no crashes)
    let count = index.doc_count();
    assert!(count > 0, "Index should have some documents");
}

#[test]
fn test_no_data_corruption_under_contention() {
    let index = Arc::new(ConcurrentTrigramIndex::new());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut handles = vec![];

    // 8 threads starting simultaneously
    for t in 0..8 {
        let index_clone = Arc::clone(&index);
        let barrier_clone = Arc::clone(&barrier);

        handles.push(thread::spawn(move || {
            // Wait for all threads to be ready
            barrier_clone.wait();

            // All threads insert same doc IDs (intentional contention)
            for i in 0..100 {
                index_clone.insert(i, &format!("thread {t} doc {i}"));
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Should have exactly 100 unique doc IDs (last write wins for each)
    assert_eq!(index.doc_count(), 100);
}

#[test]
fn test_search_consistency_during_writes() {
    let index = Arc::new(ConcurrentTrigramIndex::new());

    // Insert known documents
    for i in 0..100 {
        index.insert(i, "searchable content here");
    }

    let found_inconsistency = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut handles = vec![];

    // Writer thread
    let index_clone = Arc::clone(&index);
    handles.push(thread::spawn(move || {
        for i in 100..200 {
            index_clone.insert(i, "searchable content here");
            thread::sleep(Duration::from_micros(10));
        }
    }));

    // Reader threads checking consistency
    for _ in 0..4 {
        let index_clone = Arc::clone(&index);
        let found_clone = Arc::clone(&found_inconsistency);

        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let results = index_clone.search_like("searchable");
                let count = index_clone.doc_count();

                // Results should never be more than doc_count
                if results.len() as u64 > count {
                    found_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                }

                thread::sleep(Duration::from_micros(5));
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    assert!(
        !found_inconsistency.load(std::sync::atomic::Ordering::SeqCst),
        "Found inconsistency between search results and doc count"
    );
}

#[test]
fn test_rwlock_no_writer_starvation() {
    let index = Arc::new(ConcurrentTrigramIndex::new());
    let writes_completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut handles = vec![];

    // Many reader threads
    for _ in 0..8 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                let _ = index_clone.doc_count();
            }
        }));
    }

    // Single writer thread
    let index_clone = Arc::clone(&index);
    let writes_clone = Arc::clone(&writes_completed);
    handles.push(thread::spawn(move || {
        for i in 0..100 {
            index_clone.insert(i, &format!("doc {i}"));
            writes_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }));

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Writer should complete all writes (not starved)
    assert_eq!(
        writes_completed.load(std::sync::atomic::Ordering::SeqCst),
        100
    );
}
