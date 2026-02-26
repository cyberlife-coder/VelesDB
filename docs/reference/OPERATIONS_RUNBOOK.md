# VelesDB Operations Runbook (Hybrid Query Engine)

## 1. Objectif

Ce runbook couvre l’exploitation quotidienne du moteur hybride VelesDB (vector + graph + multi-column), avec focus sur:
- limites de requêtes,
- tuning HNSW/graph/column,
- incidents fréquents (query storm, corruption, régressions de perf).

## 2. Guardrails de requêtes

## 2.1 Paramètres recommandés (point de départ)
- `timeout_ms`: `30000`
- `max_depth`: `10`
- `max_cardinality`: `100000`
- `memory_limit_bytes`: `104857600` (100 MiB)
- `rate_limit_qps`: `100`

## 2.2 Politique de rejet
- Rejeter immédiatement les requêtes dépassant budget/limites.
- Retourner un message d’erreur explicite et actionnable.
- Logger les rejets avec tags: `tenant`, `query_hash`, `guardrail_type`.

## 3. Tuning par sous-moteur

## 3.1 Vector (HNSW)
- Monter `ef_search` pour recall, baisser pour latence.
- Monter `m` et `ef_construction` pour qualité d’index, au coût RAM/temps d’insert.
- Vérifier séparation profil **ingest** vs **serving**.

## 3.2 Graph
- Appliquer limite de profondeur stricte en prod multi-tenant.
- Favoriser filtres de labels/propriétés en amont des traversals profonds.
- Sur charges adversariales, réduire profondeur max et cardinalité intermédiaire.

## 3.3 Multi-column
- Activer pushdown des filtres sélectifs avant ANN/traversal.
- Prioriser colonnes de forte sélectivité pour réduire le set candidat.

## 4. Playbooks incident

## 4.1 Query storm / saturation CPU
1. Vérifier métriques de guardrails et timeouts.
2. Baisser temporairement `max_depth`, `max_cardinality`, `timeout_ms`.
3. Activer/renforcer rate-limit client/tenant.
4. Contrôler hit-rate cache parser; invalider cache si drift anormal.
5. Postmortem: requêtes top-N, pattern LIKE/ILIKE, traversals extrêmes.

## 4.2 Suspicion de corruption index/mmap
1. Isoler le nœud et passer en mode lecture contrôlée.
2. Lancer validation/snapshot health-check.
3. Restaurer snapshot sain + replay WAL.
4. Capturer artefacts (`index headers`, checksums, logs I/O).
5. Ouvrir RCA avec timeline et correctifs préventifs.

## 4.3 Régression de performance release
1. Comparer p50/p95/p99/recall avec baseline release-1.
2. Segmenter par opérateur (vector/graph/column/fusion).
3. Si régression > 5%, rollback ou waiver explicite documenté.

## 5. Checklist onboarding ops (< 1 journée)
- Comprendre limites de requêtes et modes de rejet.
- Exécuter un test de query storm contrôlé.
- Exécuter un exercice de recovery depuis snapshot + WAL.
- Lire dashboard latence/recall/error budget.
