# US-001: Parallel Aggregation avec Rayon

## Description

Implémenter une agrégation parallèle utilisant rayon pour diviser le travail sur plusieurs cœurs CPU.

## Critères d'Acceptation

- [ ] COUNT(*) parallélisé avec rayon::par_chunks
- [ ] SUM/AVG/MIN/MAX parallélisés
- [ ] GROUP BY parallélisé avec merge des résultats
- [ ] Seuil configurable: séquentiel si < 10K, parallèle si >= 10K
- [ ] Benchmark 100K < 150ms (3x improvement minimum)

## Tests Requis

```rust
#[test]
fn test_parallel_count_100k() {
    // COUNT(*) sur 100K rows < 150ms
}

#[test]
fn test_parallel_groupby_100k() {
    // GROUP BY sur 100K rows
}

#[test]
fn test_sequential_fallback_1k() {
    // Vérifier fallback séquentiel pour petits datasets
}
```

## Implémentation Proposée

```rust
// Dans aggregation.rs
const PARALLEL_THRESHOLD: usize = 10_000;

pub fn execute_aggregate_parallel(&self, points: &[Point]) -> AggregateResult {
    if points.len() < PARALLEL_THRESHOLD {
        return self.execute_aggregate_sequential(points);
    }
    
    // Parallel avec rayon
    let partial_results: Vec<PartialAggregate> = points
        .par_chunks(1000)
        .map(|chunk| self.aggregate_chunk(chunk))
        .collect();
    
    self.merge_partial_results(partial_results)
}
```

## Analyse Rust-Specific

### Ownership & Borrowing
- Points passés par référence `&[Point]`
- `PartialAggregate` doit être `Send + Sync` pour rayon

### Types & Traits
- `PartialAggregate`: nouveau type pour résultats partiels
- Implémenter `Send + Sync` pour thread-safety

### Error Handling
- Propagation avec `?` dans closures rayon (map_err si nécessaire)

### Concurrence
- rayon gère le thread pool
- Pas de Mutex nécessaire (map-reduce pattern)
