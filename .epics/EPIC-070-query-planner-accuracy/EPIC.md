# EPIC-070: Query Planner Accuracy

## Status: TODO
## Priority: MOYENNE
## Sprint: 2026-Q2

## Vision

Améliorer la précision des estimations du query planner pour de meilleurs plans d'exécution.

## Problème Actuel

- Estimations de sélectivité naïves (AND/OR)
- Pas d'histogrammes pour les colonnes
- Score fusion + filter peut donner ordre faux avec `higher_is_better()==false`

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Histogrammes pour estimations sélectivité | MOYENNE | 8h |
| US-002 | Fix score ordering avec higher_is_better | HAUTE | 3h |

## Fichiers Critiques

- `collection/query_cost/plan_generator.rs`
- `collection/query_cost/query_executor.rs`
- `fusion/strategy.rs`

## Critères d'Acceptation

- [ ] Histogrammes collectés lors de analyze()
- [ ] Estimations sélectivité multi-colonne améliorées
- [ ] Tests avec métriques de distance (Euclidean) validés
