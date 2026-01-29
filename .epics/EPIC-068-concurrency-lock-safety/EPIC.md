# EPIC-068: Concurrency & Lock Safety

## Status: TODO
## Priority: HAUTE
## Sprint: 2026-Q1

## Vision

Garantir la sécurité des accès concurrents et éliminer les risques de deadlock/data race.

## Problème Actuel

- **184 usages** de `RwLock`/`Mutex` dans 36 fichiers
- `PlanCache` utilise `parking_lot::RwLock` mais sans métriques de contention
- `MmapStorage::reserve_capacity()` manipule offset sous lock global
- Graph streaming iterators avec `HashSet` muté potentiellement exposé en async

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Audit PlanCache contention + métriques | HAUTE | 3h |
| US-002 | Sécuriser MmapStorage reserve_capacity avec CAS | HAUTE | 4h |
| US-003 | Documenter Send+Sync pour graph iterators | MOYENNE | 2h |

## Fichiers Critiques

- `collection/core/lifecycle.rs` - 25 usages RwLock/Mutex
- `index/hnsw/native/graph.rs` - 10 usages
- `storage/mmap.rs` - 5 usages
- `collection/query_cost/query_executor.rs` - 3 usages (PlanCache)

## Critères d'Acceptation

- [ ] Tests loom pour scénarios de contention
- [ ] Métriques de lock contention exposées
- [ ] Documentation des invariants de concurrence
- [ ] Aucun deadlock détecté par tests de stress
