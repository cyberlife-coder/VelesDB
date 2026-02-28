# New user troubleshooting: `velesdb-core` + VelesQL (EN + FR)

This guide targets the **first-hour issues** new users commonly hit.

## EN — Symptom-driven troubleshooting

### 1) "It looks like SQL, but my SQL habits fail"

VelesQL is SQL-like and vector/graph oriented. It is not a drop-in replacement for every relational SQL dialect.

**Fix:** start from repository VelesQL examples, then migrate existing SQL patterns incrementally.

### 2) "I get empty results"

Typical causes:
- query vector dimension differs from collection dimension
- threshold/filter is too strict
- no data inserted (or wrong collection selected)

**Fix sequence:**
1. check dimensions
2. run broad query without strict threshold
3. add filters progressively

### 3) "Why can't I change metric/dimension later?"

Collection vector dimension and metric are fixed by design for indexing/search performance.

**Fix:** create a new collection and reindex if embedding model or metric changes.

### 4) "Where is HTTP API / REPL?"

`velesdb-core` is embedded library only.

Use additionally:
- `velesdb-server` for REST/OpenAPI
- `velesdb-cli` for interactive VelesQL shell

### 5) "Benchmark numbers are different"

This is expected: results depend on hardware SIMD support, vector size, `ef_search`, payload/filter complexity, and data distribution.

**Fix:** benchmark with production-like data and realistic query mix.

---

## FR — Ce qui ne va pas le plus souvent

### 1) « VelesQL ne se comporte pas comme mon SQL habituel »

VelesQL est **SQL-like**, pas un clone exact de PostgreSQL/MySQL.

**Correction :** partir des exemples VelesQL du dépôt, puis adapter progressivement vos requêtes.

### 2) « Ma requête ne retourne rien »

Causes fréquentes :
- dimension du vecteur de requête ≠ dimension de la collection
- seuil de similarité trop strict
- données absentes / mauvaise collection ciblée

**Correction rapide :**
1. vérifier la dimension
2. tester sans seuil strict
3. réintroduire les filtres étape par étape

### 3) « Je ne peux pas changer métrique/dimension »

C'est normal : ce choix est figé à la création pour conserver les performances HNSW/SIMD.

**Correction :** créer une nouvelle collection et réindexer.

### 4) « J'ai installé core, mais pas d'API HTTP ni REPL »

`velesdb-core` = moteur embarqué.

Ajouter selon le besoin :
- `velesdb-server` (API HTTP)
- `velesdb-cli` (shell interactif VelesQL)

### 5) « Les benchmarks ne correspondent pas »

Normal : dépend du CPU, de `ef_search`, des filtres/payloads et du dataset.

**Correction :** mesurer sur un jeu de données proche de la prod.

---

## Minimal sanity checklist

- [ ] Collection dimension equals embedding dimension
- [ ] Metric is correct for the embedding type
- [ ] At least one known vector is inserted and retrievable
- [ ] Query works without strict filters first
- [ ] Thresholds/filters are added progressively

---

## Priorités d'amélioration code (basées sur les incidents "first-hour")

### P0 — Réduire les erreurs silencieuses et les "empty results" incompris

1. **Validation explicite de dimension dans les handlers HTTP/CLI avant exécution de recherche**
   - Problème visé: requêtes qui retournent 0 résultat alors que la vraie cause est un mismatch de dimension.
   - Amélioration: vérifier `query.len()` vs dimension collection et renvoyer une erreur guidée avec suggestion corrective.
2. **Messages d'erreur actionnables pour seuil/filtre trop strict**
   - Problème visé: "0 résultat" sans feedback.
   - Amélioration: enrichir les erreurs/réponses avec hints (ex: "testez sans threshold", "élargissez filter").
3. **Endpoint/check de diagnostic rapide pour nouvelle collection**
   - Problème visé: données absentes ou mauvaise collection ciblée.
   - Amélioration: endpoint "sanity" (dimension, metric, point_count, exemple de recherche simple).

### P1 — Mieux guider la configuration figée (dimension/métrique)

1. **Préflight au `create_collection` avec avertissements orientés usage**
   - Problème visé: incompréhension du caractère immuable dimension/métrique.
   - Amélioration: warnings/documentation inline à la création + conseils de migration/reindex.
2. **Commande/outillage de reindex simplifié**
   - Problème visé: friction quand l'utilisateur change de modèle d'embedding.
   - Amélioration: utilitaire guidé pour copier payloads + recalcul embeddings + création nouvelle collection.

### P2 — Observabilité et onboarding

1. **Logs/metrics orientés "new user"**
   - Compteurs pour: mismatch dimension, recherches sans données, seuils éliminant tous les hits.
2. **Exemples exécutables "de zéro à premier résultat"**
   - Script unique: create → insert known vector → search sans filtre → search avec filtre.
3. **Messages d'installation plus explicites entre core/server/cli**
   - Clarifier plus tôt que `velesdb-core` n'expose ni HTTP API ni REPL.

### Ordre d'implémentation recommandé

1. P0.1 + P0.2 (impact immédiat sur DX)
2. P0.3 (diagnostic automatique)
3. P1.1 (prévention)
4. P1.2 (workflow de migration)
5. P2.* (observabilité et docs onboarding)
