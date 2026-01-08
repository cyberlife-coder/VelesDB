//! Tests for `backend` module

use super::backend::*;
use hnsw_rs::prelude::Neighbour;
use std::path::Path;

// -------------------------------------------------------------------------
// Trait Definition Tests
// -------------------------------------------------------------------------

/// Verify trait is object-safe (can be used as `dyn HnswBackend`)
#[test]
fn test_trait_is_object_safe() {
    fn accepts_dyn_backend(_backend: &dyn HnswBackend) {}
    // If this compiles, the trait is object-safe
    let mock = MockBackend::default();
    accepts_dyn_backend(&mock);
}

/// Verify trait requires Send + Sync
#[test]
fn test_trait_is_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    // These will fail to compile if trait doesn't require Send + Sync
    fn check_bounds<T: HnswBackend>() {
        assert_send::<T>();
        assert_sync::<T>();
    }

    // Actually call to avoid unused warnings
    check_bounds::<MockBackend>();
}

// -------------------------------------------------------------------------
// Mock Backend for Testing
// -------------------------------------------------------------------------

/// Mock backend that records method calls for testing
#[derive(Default)]
struct MockBackend {
    search_calls: std::cell::RefCell<Vec<(usize, usize)>>, // (k, ef)
    insert_calls: std::cell::RefCell<Vec<usize>>,          // indices
    searching_mode: std::cell::RefCell<bool>,
}

// MockBackend is Send + Sync because RefCell contents are only accessed
// in single-threaded test contexts
unsafe impl Send for MockBackend {}
unsafe impl Sync for MockBackend {}

impl HnswBackend for MockBackend {
    fn search(&self, _query: &[f32], k: usize, ef_search: usize) -> Vec<Neighbour> {
        self.search_calls.borrow_mut().push((k, ef_search));
        // Return mock neighbors
        #[allow(clippy::cast_precision_loss)]
        (0..k.min(3))
            .map(|i| Neighbour {
                d_id: i,
                p_id: hnsw_rs::prelude::PointId::default(),
                distance: i as f32 * 0.1,
            })
            .collect()
    }

    fn insert(&self, data: (&[f32], usize)) {
        self.insert_calls.borrow_mut().push(data.1);
    }

    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        for (_, idx) in data {
            self.insert_calls.borrow_mut().push(*idx);
        }
    }

    fn set_searching_mode(&mut self, mode: bool) {
        *self.searching_mode.borrow_mut() = mode;
    }

    fn file_dump(&self, _path: &Path, _basename: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn transform_score(&self, raw_distance: f32) -> f32 {
        raw_distance // Simple passthrough for mock
    }
}

#[test]
fn test_mock_backend_search() {
    // Arrange
    let backend = MockBackend::default();
    let query = vec![1.0, 2.0, 3.0];

    // Act
    let results = backend.search(&query, 5, 100);

    // Assert
    assert_eq!(results.len(), 3); // Mock returns min(k, 3)
    assert_eq!(backend.search_calls.borrow().len(), 1);
    assert_eq!(backend.search_calls.borrow()[0], (5, 100));
}

#[test]
fn test_mock_backend_insert() {
    // Arrange
    let backend = MockBackend::default();
    let vector = vec![1.0, 2.0, 3.0];

    // Act
    backend.insert((&vector, 42));

    // Assert
    assert_eq!(backend.insert_calls.borrow().len(), 1);
    assert_eq!(backend.insert_calls.borrow()[0], 42);
}

#[test]
fn test_mock_backend_parallel_insert() {
    // Arrange
    let backend = MockBackend::default();
    let v1 = vec![1.0, 2.0];
    let v2 = vec![3.0, 4.0];
    let data: Vec<(&Vec<f32>, usize)> = vec![(&v1, 0), (&v2, 1)];

    // Act
    backend.parallel_insert(&data);

    // Assert
    assert_eq!(backend.insert_calls.borrow().len(), 2);
}

#[test]
fn test_mock_backend_searching_mode() {
    // Arrange
    let mut backend = MockBackend::default();

    // Act
    backend.set_searching_mode(true);

    // Assert
    assert!(*backend.searching_mode.borrow());
}

#[test]
fn test_mock_backend_file_dump() {
    // Arrange
    let backend = MockBackend::default();
    let path = std::path::Path::new("/tmp");

    // Act
    let result = backend.file_dump(path, "test");

    // Assert
    assert!(result.is_ok());
}

#[test]
fn test_mock_backend_transform_score() {
    // Arrange
    let backend = MockBackend::default();

    // Act
    let score = backend.transform_score(0.5);

    // Assert
    assert!((score - 0.5).abs() < f32::EPSILON);
}

// -------------------------------------------------------------------------
// Generic Function Tests (proves trait is usable)
// -------------------------------------------------------------------------

fn generic_search<B: HnswBackend>(backend: &B, query: &[f32], k: usize) -> Vec<Neighbour> {
    backend.search(query, k, 100)
}

#[test]
fn test_generic_function_with_mock() {
    // Arrange
    let backend = MockBackend::default();
    let query = vec![0.0; 8];

    // Act
    let results = generic_search(&backend, &query, 5);

    // Assert
    assert!(!results.is_empty());
}
