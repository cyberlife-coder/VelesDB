# EPIC-071: Memory & Performance Optimization

## Status: TODO
## Priority: MOYENNE
## Sprint: 2026-Q2

## Vision

Optimiser l'utilisation mémoire et explorer les opportunités GPU.

## Problème Actuel

- BFS iterator ne libère pas `visited` HashSet avant return
- Duplication de buffers dans `dual_precision.rs`
- Opportunité GPU pour K>1000 vectors (3-4× speedup potentiel)

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Pool/clear visited HashSet dans BFS | MOYENNE | 2h |
| US-002 | Éliminer duplication buffers quantizer | MOYENNE | 3h |
| US-003 | Prototype GPU kernel cosine/dot | BASSE | 16h |

## Fichiers Critiques

- `collection/graph/streaming.rs` - BfsIterator
- `index/hnsw/native/dual_precision.rs` - quantizer buffers
- `gpu/gpu_backend.rs` - GPU kernels

## Critères d'Acceptation

- [ ] Benchmarks mémoire avant/après
- [ ] Pas de régression performance
- [ ] GPU kernel optionnel (feature flag)
