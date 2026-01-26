# US-006: Vectorized Batch Aggregation

## Description

Implémenter le traitement par batch de 1024 valeurs dans l'Aggregator pour permettre l'auto-vectorisation SIMD par le compilateur.

## Problème Actuel

```rust
// Traitement row-by-row
for row in rows {
    aggregator.process_value("price", &row.payload["price"]);
}
```

Le traitement valeur par valeur empêche l'optimisation SIMD.

## Critères d'Acceptation

- [ ] Nouvelle méthode `process_batch(column: &str, values: &[f64])`
- [ ] Batch size configurable (default 1024)
- [ ] Auto-vectorisation vérifiée (check assembly)
- [ ] Benchmark prouvant 3-5x d'amélioration sur 100K+ rows
- [ ] Fallback sur process_value pour petits datasets

## Solution Proposée

```rust
const BATCH_SIZE: usize = 1024;

impl Aggregator {
    /// Process a batch of numeric values for SIMD-friendly aggregation
    pub fn process_batch(&mut self, column: &str, values: &[f64]) {
        if values.is_empty() {
            return;
        }
        
        // SUM - compiler will auto-vectorize this loop
        let batch_sum: f64 = values.iter().sum();
        *self.sums.entry(column.to_string()).or_insert(0.0) += batch_sum;
        
        // COUNT
        let batch_count = values.len() as u64;
        *self.counts.entry(column.to_string()).or_insert(0) += batch_count;
        
        // MIN - SIMD friendly with fold
        let batch_min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let min = self.mins.entry(column.to_string()).or_insert(batch_min);
        if batch_min < *min {
            *min = batch_min;
        }
        
        // MAX
        let batch_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let max = self.maxs.entry(column.to_string()).or_insert(batch_max);
        if batch_max > *max {
            *max = batch_max;
        }
    }
}
```

## Intégration dans execute_aggregate

```rust
// Collect values in batches before processing
let mut batch_buffer: Vec<f64> = Vec::with_capacity(BATCH_SIZE);

for payload in payloads.iter() {
    if let Some(num) = extract_number(&payload, column) {
        batch_buffer.push(num);
        
        if batch_buffer.len() >= BATCH_SIZE {
            aggregator.process_batch(column, &batch_buffer);
            batch_buffer.clear();
        }
    }
}

// Process remaining
if !batch_buffer.is_empty() {
    aggregator.process_batch(column, &batch_buffer);
}
```

## Tests Requis

```rust
#[test]
fn test_batch_sum_correctness() {
    let values: Vec<f64> = (1..=1000).map(|x| x as f64).collect();
    // Expected sum = 500500
}

#[test]
fn test_batch_min_max_correctness() {
    let values = vec![5.0, 1.0, 9.0, 3.0, 7.0];
    // min=1, max=9
}

#[test]
fn test_batch_vs_sequential_equivalence() {
    // Même résultat batch vs valeur par valeur
}

#[test]
fn bench_batch_aggregation_100k() {
    // Doit être 3x+ plus rapide que séquentiel
}
```

## Analyse Rust-Specific

### Ownership & Borrowing
- `&[f64]` slice emprunté, pas de copie
- Buffer réutilisable entre itérations

### Types & Traits
- Pas de nouveaux traits
- `f64` pour précision (pas `f32`)

### Error Handling
- Pas d'erreur possible (opérations arithmétiques)
- NaN handling: `fold` avec `f64::min/max` gère NaN

### Concurrence
- Buffer local au thread (pas de partage)
- Compatible avec parallélisation existante

### SIMD Verification
```bash
# Vérifier auto-vectorisation
RUSTFLAGS="-C target-cpu=native" cargo build --release
cargo asm velesdb_core::velesql::aggregator::Aggregator::process_batch
# Chercher instructions SIMD: vaddpd, vmaxpd, vminpd
```
