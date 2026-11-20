use super::*;

// --- sort_scored_by_metric ---

#[test]
fn sort_scored_by_metric_descending_when_higher_is_better() {
    let mut results = vec![
        ScoredResult::new(1, 0.2),
        ScoredResult::new(2, 0.8),
        ScoredResult::new(3, 0.5),
    ];
    sort_scored_by_metric(&mut results, true);

    assert_eq!(results[0].id, 2);
    assert_eq!(results[1].id, 3);
    assert_eq!(results[2].id, 1);
}

#[test]
fn sort_scored_by_metric_ascending_when_lower_is_better() {
    let mut results = vec![
        ScoredResult::new(1, 0.2),
        ScoredResult::new(2, 0.8),
        ScoredResult::new(3, 0.5),
    ];
    sort_scored_by_metric(&mut results, false);

    assert_eq!(results[0].id, 1);
    assert_eq!(results[1].id, 3);
    assert_eq!(results[2].id, 2);
}

#[test]
fn sort_scored_by_metric_empty_slice_is_noop() {
    let mut results: Vec<ScoredResult> = vec![];
    sort_scored_by_metric(&mut results, true);
    assert!(results.is_empty());
}

// --- hydrate_accepted ---

/// Vector storage holding `[id as f32]` for every id, counting the vectors
/// it hands out.
#[derive(Default)]
struct CountingVectors {
    retrieved: std::sync::atomic::AtomicUsize,
}

impl crate::storage::VectorStorage for CountingVectors {
    fn store(&mut self, _: u64, _: &[f32]) -> std::io::Result<()> {
        Ok(())
    }
    fn store_batch(&mut self, vectors: &[(u64, &[f32])]) -> std::io::Result<usize> {
        Ok(vectors.len())
    }
    fn retrieve(&self, id: u64) -> std::io::Result<Option<Vec<f32>>> {
        self.retrieved
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(vec![f32::from(
            u16::try_from(id).expect("test: small id"),
        )]))
    }
    fn delete(&mut self, _: u64) -> std::io::Result<()> {
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn len(&self) -> usize {
        0
    }
    fn ids(&self) -> Vec<u64> {
        Vec::new()
    }
}

/// Payload storage giving every id `{"even": id % 2 == 0}`.
struct ParityPayloads;

impl crate::storage::PayloadStorage for ParityPayloads {
    fn store(&mut self, _: u64, _: &serde_json::Value) -> std::io::Result<()> {
        Ok(())
    }
    fn retrieve(&self, id: u64) -> std::io::Result<Option<serde_json::Value>> {
        Ok(Some(serde_json::json!({ "even": id.is_multiple_of(2) })))
    }
    fn delete(&mut self, _: u64) -> std::io::Result<()> {
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn ids(&self) -> Vec<u64> {
        Vec::new()
    }
}

/// A filtered hydration judges payloads first and copies only the vectors
/// of the points it returns: 3 kept from 100 candidates the filter halves
/// reads 3 vectors, not 50 or 100.
#[test]
fn hydrate_accepted_copies_only_the_vectors_it_returns() {
    let vectors = CountingVectors::default();
    let pairs = (0..100_u64).map(|id| (id, 1.0));

    let results = hydrate_accepted(
        pairs,
        3,
        0,
        RowUniverse::Vectors,
        &vectors,
        &ParityPayloads,
        |_id, payload| payload.and_then(|p| p.get("even")) == Some(&serde_json::Value::Bool(true)),
    );

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![0, 2, 4], "the first accepted, in candidate order");
    assert_eq!(
        vectors.retrieved.load(std::sync::atomic::Ordering::Relaxed),
        3,
        "one vector copy per returned point"
    );
}

/// Vector storage holding a vector for even ids only.
struct EvenVectors;

impl crate::storage::VectorStorage for EvenVectors {
    fn store(&mut self, _: u64, _: &[f32]) -> std::io::Result<()> {
        Ok(())
    }
    fn store_batch(&mut self, vectors: &[(u64, &[f32])]) -> std::io::Result<usize> {
        Ok(vectors.len())
    }
    fn retrieve(&self, id: u64) -> std::io::Result<Option<Vec<f32>>> {
        Ok(id.is_multiple_of(2).then(|| vec![1.0]))
    }
    fn delete(&mut self, _: u64) -> std::io::Result<()> {
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn len(&self) -> usize {
        0
    }
    fn ids(&self) -> Vec<u64> {
        Vec::new()
    }
}

/// A hydration admits the rows of its universe: a vector ranking only ids
/// storing a vector, a text search a graph's every stored node — one without
/// a vector read with an empty one.
#[test]
fn hydrate_accepted_admits_the_rows_of_its_universe() {
    let hydrate = |universe| -> Vec<(u64, usize)> {
        hydrate_accepted(
            (0..4_u64).map(|id| (id, 1.0)),
            10,
            0,
            universe,
            &EvenVectors,
            &ParityPayloads,
            |_, _| true,
        )
        .iter()
        .map(|r| (r.point.id, r.point.vector.len()))
        .collect()
    };
    assert_eq!(hydrate(RowUniverse::Vectors), vec![(0, 1), (2, 1)]);
    assert_eq!(
        hydrate(RowUniverse::StoredNodes),
        vec![(0, 1), (1, 0), (2, 1), (3, 0)]
    );
}
