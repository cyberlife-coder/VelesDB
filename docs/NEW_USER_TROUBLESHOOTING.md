# New user troubleshooting: `velesdb-core` + VelesQL (EN + FR)

This guide targets the **first-hour issues** new users commonly hit.

## EN — Symptom-driven troubleshooting

### 1) "It looks like SQL, but my SQL habits fail"

VelesQL is SQL-like and vector/graph oriented. It is not a drop-in replacement for every relational SQL dialect.

**Fix:** start from [VelesQL examples](VELESQL_SPEC.md#examples) and the [conformance test cases](../conformance/velesql_parser_cases.json), then adapt your SQL patterns incrementally.

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

Install the component you need:
- `cargo install velesdb-server` — REST API server (37 endpoints, OpenAPI)
- `cargo install velesdb-cli` — Interactive VelesQL REPL (binary name: `velesdb`)

### 5) "Benchmark numbers are different"

This is expected: results depend on hardware SIMD support, vector size, `ef_search`, payload/filter complexity, and data distribution.

**Fix:** benchmark with production-like data and realistic query mix.

---

## FR — Ce qui ne va pas le plus souvent

### 1) « VelesQL ne se comporte pas comme mon SQL habituel »

VelesQL est **SQL-like**, pas un clone exact de PostgreSQL/MySQL.

**Correction :** partir des [exemples VelesQL](VELESQL_SPEC.md#examples) et des [cas de conformance](../conformance/velesql_parser_cases.json), puis adapter progressivement vos requêtes.

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

Installer le composant adapté :
- `cargo install velesdb-server` — API REST (37 endpoints, OpenAPI)
- `cargo install velesdb-cli` — Shell interactif VelesQL (binaire : `velesdb`)

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

## Next steps / Prochaines étapes

- [VelesQL Specification](VELESQL_SPEC.md) — Full query language reference with examples
- [Search Modes Guide](guides/SEARCH_MODES.md) — How to choose between Fast, Balanced, Accurate, Perfect, and Adaptive
- [Tuning Guide](guides/TUNING_GUIDE.md) — HNSW parameters, quantization modes, memory estimation
- [API Reference](reference/API_REFERENCE.md) — All 37 REST endpoints with request/response examples
- [Installation Guide](guides/INSTALLATION.md) — All platforms: Linux, macOS, Windows, Docker, WASM, Mobile
- [E-commerce Example](../examples/ecommerce_recommendation/) — Full Vector + Graph + Filter demo in Rust
