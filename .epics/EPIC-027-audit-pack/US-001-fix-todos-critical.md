# US-001: Résoudre les TODOs Critiques

## Description

Résoudre les TODOs techniques identifiés par l'audit Devin+SonarCloud.

## Contexte

Audit du 22/01/2026 a identifié 5 TODOs techniques critiques affectant la correctness ou performance.

## Critères d'Acceptation

### AC-1: QueryPlanner Integration
```rust
// query.rs:11 - Intégrer QueryPlanner::choose_hybrid_strategy()
// Utiliser le cost-based optimizer pour les requêtes complexes
```

### AC-2: Distance Metrics Threshold Semantics
```rust
// query.rs:476 - Clarifier sémantique pour Euclidean/Hamming
// Pour similarity() > 0.5 avec Euclidean, c'est "distance > 0.5" = moins similaire
// Option: inverser comparaison basée sur metric.higher_is_better()
```

### AC-3: Sort Direction for Distance Metrics
```rust
// vector.rs:213 - Tri DESC par défaut incorrect pour Euclidean/Hamming
// Utiliser metric.higher_is_better() pour déterminer direction tri
```

### AC-4: Documentation
Documenter clairement le comportement pour les métriques de distance vs similarité.

## Fichiers Impactés

- `crates/velesdb-core/src/collection/search/query.rs`
- `crates/velesdb-core/src/collection/search/vector.rs`
- `crates/velesdb-core/src/velesql/planner.rs`

## Definition of Done

- [ ] QueryPlanner intégré dans execute_query
- [ ] Sémantique threshold clarifiée et corrigée
- [ ] Sort direction basée sur metric type
- [ ] Tests couvrant tous les cas
- [ ] Documentation mise à jour

## Estimation

- **Complexité**: M (Medium)
- **Effort**: 4h

## Priorité

**P1 - Critique** - Affecte la correctness des résultats pour Euclidean/Hamming
