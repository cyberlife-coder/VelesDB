# US-001: Remplacer partial_cmp().unwrap() par total_cmp()

## Status: TODO
## Priorité: CRITIQUE
## Estimation: 2h

## Description

Remplacer tous les usages de `partial_cmp().unwrap()` et `partial_cmp().unwrap_or(Ordering::Equal)` par `total_cmp()` pour éviter les panics sur NaN.

## Contexte Technique

`f32::partial_cmp()` retourne `None` si l'un des opérandes est NaN, ce qui cause un panic avec `.unwrap()`. `f32::total_cmp()` (stable depuis Rust 1.62) fournit un ordre total incluant NaN.

## Fichiers à Modifier

1. `fusion/strategy.rs` - 4 occurrences (lignes 197, 216, 241, 286)
2. `distance.rs` - 2 occurrences (lignes 98, 101)
3. `collection/search/text.rs` - 2 occurrences (lignes 180, 269)
4. `index/bm25.rs` - 3 occurrences (lignes 304, 307, 309)
5. `index/hnsw/native/dual_precision.rs` - 1 occurrence (ligne 211)
6. `collection/search/query/parallel_traversal.rs` - 1 occurrence (ligne 278)
7. `index/trigram/index.rs` - 1 occurrence (ligne 296)
8. `collection/graph/range_index.rs` - 1 occurrence (ligne 39) - OK (gère NaN explicitement)

## Pattern de Remplacement

```rust
// AVANT (dangereux)
results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

// APRÈS (safe)
results.sort_by(|a, b| b.1.total_cmp(&a.1));
```

## Critères d'Acceptation

- [ ] Aucun `partial_cmp().unwrap()` dans le code production
- [ ] Tests avec valeurs NaN passent sans panic
- [ ] Benchmarks montrent performance équivalente ou meilleure
- [ ] `cargo clippy -- -D warnings` passe

## Tests Requis

```rust
#[test]
fn test_sort_with_nan_values() {
    let mut scores = vec![1.0, f32::NAN, 0.5, 2.0];
    scores.sort_by(|a, b| b.total_cmp(a));
    // NaN should sort consistently (at end or beginning)
    assert!(!scores[0].is_nan()); // Best score first
}
```

## Scénarios Gherkin

```gherkin
Scenario: Sort similarity scores with NaN values
  Given a list of similarity scores including NaN
  When I sort by descending score
  Then the sort completes without panic
  And NaN values are placed consistently at the end

Scenario: Fusion strategy handles NaN scores
  Given multiple result sets with some NaN scores
  When I apply RRF fusion
  Then the fusion completes successfully
  And NaN scores are treated as lowest priority
```
