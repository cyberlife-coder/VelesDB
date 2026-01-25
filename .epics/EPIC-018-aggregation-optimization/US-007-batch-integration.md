# US-007: Integrate process_batch into execute_aggregate

## Status: ❌ ABANDONED

## Description

Tentative d'intégrer `process_batch()` (US-006) dans `execute_aggregate()`.

## Résultat des Benchmarks

| Dataset | Avant | Après | Impact |
|---------|-------|-------|--------|
| 1K | 2.67 ms | 2.77 ms | **+4% régression** |
| 10K | 26 ms | 28 ms | **+9% régression** |
| 50K | 177 ms | 191 ms | **+8% régression** |

## Analyse

L'intégration de `process_batch()` dans `execute_aggregate()` **ajoute de l'overhead**:

1. **Allocation supplémentaire**: HashMap + Vec par colonne pour collecter les valeurs
2. **Double itération**: Une fois pour collecter, une fois pour process_batch
3. **Bottleneck ailleurs**: Le parsing JSON des payloads domine le temps (~95%)

Le gain de 46-58x mesuré sur `process_batch()` isolé ne se traduit pas car:
- Le benchmark isolé partait de `Vec<f64>` pré-parsés
- Dans `execute_aggregate()`, on part de `serde_json::Value` qu'il faut parser

## Conclusion

**L'optimisation batch ne s'applique PAS à execute_aggregate.**

Le gain 46-58x de US-006 reste valide pour les cas d'usage où l'utilisateur:
- A déjà des données numériques pré-parsées
- Utilise directement `Aggregator::process_batch()` dans son code

## Leçon Apprise

Toujours benchmarker dans le contexte réel, pas seulement le composant isolé.
