# EPIC-018: Aggregation Performance Optimization

## Vision

Optimiser les performances des agrégations VelesQL pour supporter efficacement des datasets de 100K+ lignes.

## Contexte

Les benchmarks ont identifié un hotspot critique:
- **Aggregation 100K rows**: 440ms (O(n) linéaire)
- **Target**: < 100ms avec parallélisation

## User Stories

| US | Titre | Priorité | Status |
|----|-------|----------|--------|
| US-001 | Parallel Aggregation avec Rayon | HIGH | TODO |
| US-002 | Pre-computed Collection Stats | MEDIUM | TODO |
| US-003 | Early Termination HAVING + LIMIT | LOW | TODO |

## Critères de Succès

- [ ] Benchmark 100K aggregation < 100ms (4x improvement)
- [ ] Pas de régression sur datasets < 10K
- [ ] Tests unitaires couvrant tous les cas

## Recherche

Voir `.research/2026-01-25-velesql-execution-benchmarks.md`
