# US-004: String Interning in Aggregator

## Description

Éviter les allocations de String dans le hot path de `Aggregator::process_value()` en utilisant le string interning.

## Problème Actuel

```rust
// aggregator.rs:92 - Clone à chaque appel!
*self.sums.entry(column.to_string()).or_insert(0.0) += num;
```

Chaque appel à `process_value()` clone la string du nom de colonne, causant des allocations inutiles.

## Critères d'Acceptation

- [ ] Utiliser `&str` avec lifetime ou string interning
- [ ] Pas de `.to_string()` dans le hot path
- [ ] Benchmark prouvant 15-25% d'amélioration
- [ ] Tous les tests existants passent

## Solution Proposée

Option A: Lifetime parameter
```rust
pub struct Aggregator<'a> {
    sums: HashMap<&'a str, f64>,
    // ...
}
```

Option B: String interning (si Option A trop complexe)
```rust
use rustc_hash::FxHasher;
use std::collections::HashMap;

pub struct Aggregator {
    column_ids: HashMap<String, usize>,  // Intern une seule fois
    sums: Vec<f64>,                       // Accès par index
    // ...
}
```

## Tests Requis

```rust
#[test]
fn test_aggregator_no_string_allocation_in_hot_path() {
    // Mesurer les allocations avec custom allocator
}

#[test]
fn test_aggregator_multiple_columns_performance() {
    // Benchmark 100K rows avec 10 colonnes
}
```

## Analyse Rust-Specific

### Ownership & Borrowing
- Les noms de colonnes viennent de `&str` slice
- Besoin de lifetime si on garde des références
- Alternative: interning pour éviter lifetimes complexes

### Types & Traits
- `Aggregator` doit rester `Send + Sync` pour parallélisation
- HashMap clé = `usize` (index) au lieu de `String`

### Error Handling
- Pas de changement nécessaire

### Concurrence
- Structure interne change mais API externe identique
- Tests parallèles doivent passer
