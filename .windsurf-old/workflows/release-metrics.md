---
description: Relancer benchmarks, tests, coverage et mettre à jour TOUS les fichiers de métriques
---

# /release-metrics - Mise à jour complète des métriques

Ce workflow relance tous les benchmarks, tests et coverage, puis met à jour **tous les fichiers** contenant des métriques de performance.

---

## 📋 Fichiers à mettre à jour

| Fichier | Contenu |
|---------|---------|
| `README.md` | Section Performance, badges, métriques principales |
| `docs/BENCHMARKS.md` | Benchmarks détaillés SIMD, HNSW, Hybrid |
| `docs/RELEASE_METRICS_v{VERSION}.md` | Métriques complètes de la release |
| `docs/SEARCH_MODES.md` | Latences par mode de recherche |
| `crates/velesdb-core/README.md` | Métriques spécifiques au core |
| `docs/guides/USE_CASES.md` | Chiffres dans les use cases |
| `docs/contributing/BENCHMARKING_GUIDE.md` | Référence des benchmarks |
| `CHANGELOG.md` | Section Performance de la version |

---

## Étape 1: Exécuter les tests complets

```powershell
# Tous les tests du workspace
cargo test --workspace --release 2>&1 | Tee-Object -FilePath test_results.txt

# Compter les tests passés/échoués
$passed = (Select-String -Path test_results.txt -Pattern "test result: ok").Count
$failed = (Select-String -Path test_results.txt -Pattern "FAILED").Count
Write-Host "Test suites passed: $passed, failed: $failed"
```

**Résultat attendu**: Nombre total de tests, tests passés, tests ignorés

### ⚠️ Si des tests échouent → Déclencher /fix-failed-tests

Si `$failed > 0`, **STOP** et exécuter le workflow de correction automatique:

```
/fix-failed-tests
```

Ce workflow va:
1. Parser les tests qui échouent depuis `test_results.txt`
2. Analyser la cause racine de chaque échec
3. Appliquer les corrections (code ou assertions)
4. Re-exécuter les tests jusqu'à 100% passing
5. Revenir à `/release-metrics` une fois tous les tests verts

**Ne pas continuer tant que tous les tests ne passent pas.**

---

## Étape 2: Générer le coverage

```powershell
# Installer cargo-llvm-cov si nécessaire
cargo install cargo-llvm-cov --locked

# Générer le rapport de coverage
cargo llvm-cov --workspace --html --output-dir coverage/

# Afficher le résumé
cargo llvm-cov --workspace --text | Tee-Object -FilePath coverage_summary.txt

# Extraire les métriques clés
Select-String -Path coverage_summary.txt -Pattern "TOTAL"
```

**Métriques à extraire**:
- Line coverage %
- Function coverage %
- Region coverage %

---

## Étape 3: Exécuter les benchmarks SIMD

```powershell
# Benchmarks distance (SIMD)
cargo bench --bench distance_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_simd.txt

# Extraire les résultats
Select-String -Path bench_simd.txt -Pattern "time:"
```

**Métriques à extraire**:
- Dot Product: latence (ns) et throughput
- Euclidean: latence (ns) et throughput
- Cosine: latence (ns) et throughput
- Hamming: latence (ns) et throughput
- Jaccard: latence (ns) et throughput

Pour chaque dimension: 128D, 384D, 768D, 1536D, 3072D

---

## Étape 4: Exécuter les benchmarks HNSW (Vector Index)

```powershell
# Benchmarks HNSW search
cargo bench --bench hnsw_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_hnsw.txt

# Extraire les résultats
Select-String -Path bench_hnsw.txt -Pattern "time:"
```

**Métriques Vector à extraire**:

| Catégorie | Métriques |
|-----------|----------|
| **Search Latency** | k=10, k=100, k=1000 |
| **Recall** | Recall@10, Recall@100 par ef_search (64, 128, 256, 512) |
| **Insert** | Bulk insert throughput (points/sec) |
| **Scale** | 1K, 10K, 100K, 1M vectors |
| **Memory** | Index size (MB) per 10K vectors |
| **Dimensions** | 128D, 384D, 768D, 1536D |

---

## Étape 5: Exécuter les benchmarks Graph (Knowledge Graph)

```powershell
# Benchmarks Graph traversal
cargo bench --bench graph_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_graph.txt

# Extraire les résultats
Select-String -Path bench_graph.txt -Pattern "time:"
```

**Métriques Graph à extraire**:

| Catégorie | Métriques |
|-----------|----------|
| **Edge Operations** | add_edge, remove_edge, get_edge (ns) |
| **Traversal BFS** | Latence par depth (1, 2, 3, 5 hops) |
| **Traversal DFS** | Latence par depth (1, 2, 3, 5 hops) |
| **Edge Count** | Throughput get_outgoing, get_incoming |
| **Scale** | 1K, 10K, 100K, 1M edges |
| **Concurrent** | Multi-thread read/write ops/sec |
| **Memory** | EdgeStore size (MB) per 10K edges |
| **Streaming** | BFS stream throughput (edges/sec) |

---

## Étape 6: Exécuter les benchmarks ColumnStore (Multicolumn)

```powershell
# Benchmarks ColumnStore filtering
cargo bench --bench columnstore_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_columnstore.txt

# Extraire les résultats
Select-String -Path bench_columnstore.txt -Pattern "time:"
```

**Métriques ColumnStore à extraire**:

| Catégorie | Métriques |
|-----------|----------|
| **Filter Ops** | eq, neq, gt, lt, gte, lte (ns) |
| **Complex Filters** | AND, OR, NOT combinations |
| **String Filters** | contains, starts_with, ends_with, LIKE, ILIKE |
| **Range Queries** | BETWEEN, IN list |
| **Throughput** | Filtered rows/sec (1K, 10K, 100K rows) |
| **vs JSON** | Speedup factor vs serde_json filtering |
| **Projection** | SELECT specific columns latency |
| **Aggregations** | COUNT, SUM, AVG, MIN, MAX latency |
| **GROUP BY** | Grouping throughput |
| **Memory** | Column index size (MB) per 10K rows |

---

## Étape 7: Exécuter les benchmarks Hybrid/Fusion

```powershell
# Benchmarks Hybrid search (Vector + Graph + Filter)
cargo bench --bench hybrid_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_hybrid.txt

# Parser benchmarks
cargo bench --bench parser_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_parser.txt
```

**Métriques Hybrid/Fusion à extraire**:

| Catégorie | Métriques |
|-----------|----------|
| **Vector + Filter** | similarity() + WHERE clause |
| **Vector + Graph** | NEAR + MATCH traversal |
| **Graph + Filter** | MATCH + WHERE clause |
| **Triple Fusion** | Vector + Graph + Filter combined |
| **Fusion Strategies** | RRF, Average, Maximum, Minimum scores |
| **Multi-Query** | Batch search latency (10, 100 queries) |
| **Text + Vector** | BM25 + similarity() hybrid |

---

## Étape 8: Exécuter les benchmarks VelesQL Parser

```powershell
# Parser benchmarks
cargo bench --bench parser_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_parser.txt
```

**Métriques Parser à extraire**:

| Catégorie | Métriques |
|-----------|----------|
| **Parse Simple** | SELECT * FROM x latency |
| **Parse Complex** | Multi-JOIN, subqueries |
| **Cache Hit** | QueryCache lookup latency |
| **Cache Miss** | Full parse latency |
| **Throughput** | Queries parsed/sec |
| **MATCH Parse** | Cypher-like MATCH clause |
| **Validation** | Semantic validation latency |

---

## Étape 9: Exécuter les benchmarks E2E

```powershell
# Benchmarks end-to-end
cargo bench --bench e2e_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_e2e.txt

# Parser benchmarks
cargo bench --bench parser_benchmarks -- --save-baseline current 2>&1 | Tee-Object -FilePath bench_parser.txt
```

**Métriques à extraire**:
- Hybrid search latency
- Text search (BM25) latency
- VelesQL parser latency (parse, cache hit)
- ColumnStore filtering throughput

---

## Étape 10: Collecter les statistiques du codebase

```powershell
# Compter les lignes de code Rust
Get-ChildItem -Recurse -Include "*.rs" | Where-Object { $_.FullName -notmatch "target|node_modules" } | ForEach-Object { (Get-Content $_).Count } | Measure-Object -Sum

# Compter les fichiers
Get-ChildItem -Recurse -Include "*.rs" | Where-Object { $_.FullName -notmatch "target|node_modules" } | Measure-Object

# Compter les benchmarks
Get-ChildItem -Recurse -Path "benches" -Include "*.rs" | Measure-Object

# Compter les tests d'intégration
Get-ChildItem -Recurse -Path "tests" -Include "*.rs" | Measure-Object
```

---

## Étape 11: Vérifier les dépendances

```powershell
# Audit de sécurité
cargo deny check

# Compter les dépendances
cargo tree --depth 1 | Measure-Object -Line
```

---

## Étape 12: Mesurer les tailles de packages

```powershell
# Build release
cargo build --release --workspace

# Mesurer les tailles
Get-Item target/release/velesdb-cli.exe | Select-Object Name, @{N='Size (MB)';E={[math]::Round($_.Length/1MB, 2)}}
Get-Item target/release/velesdb-server.exe | Select-Object Name, @{N='Size (MB)';E={[math]::Round($_.Length/1MB, 2)}}
```

---

## Étape 13: Mettre à jour les fichiers

### 13.1 README.md

Mettre à jour la section "Performance" avec:
- Badge coverage
- **Vector**: SIMD headline, HNSW search, Recall
- **Graph**: BFS/DFS traversal, Edge throughput
- **ColumnStore**: Filter throughput, vs JSON speedup
- **Hybrid**: Triple fusion latency
- Nombre de tests

### 13.2 docs/BENCHMARKS.md

Mettre à jour:
- Date "Last updated"
- **Section VECTOR**: SIMD Performance, HNSW Recall Profiles
- **Section GRAPH**: Edge Operations, Traversal Performance
- **Section COLUMNSTORE**: Filter Operations, Aggregations
- **Section HYBRID**: Fusion Strategies, Combined Queries
- Tableau VelesQL Parser

### 13.3 docs/RELEASE_METRICS_v{VERSION}.md

Créer/mettre à jour avec TOUTES les métriques (voir template complet ci-dessous):
- Test Coverage (total, passing, line%, function%, region%)
- **Vector Performance** (SIMD par dimension, HNSW par ef_search)
- **Graph Performance** (Edge ops, BFS/DFS par depth)
- **ColumnStore Performance** (Filters, Aggregations, GROUP BY)
- **Hybrid Performance** (toutes combinaisons)
- Codebase Statistics (LoC, files, crates, benchmarks)
- Dependencies (count, security advisories)
- Package Sizes

### 13.4 Autres fichiers

- `docs/SEARCH_MODES.md`: Latences par mode (Vector, Graph, Hybrid)
- `docs/GRAPH_API.md`: Métriques traversal (si existe)
- `crates/velesdb-core/README.md`: Métriques core
- `CHANGELOG.md`: Section performance de la version actuelle

---

## Étape 14: Valider les changements

```powershell
# Vérifier que les fichiers sont cohérents
git diff --stat

# Commit
git add -A
git commit -m "docs: update release metrics v{VERSION}"
```

---

## 📊 Templates de métriques complètes

### Format pour README.md (Headline)

```markdown
## ⚡ Performance

VelesDB = **Vector + Graph + ColumnStore** en un seul moteur.

| Domain | Benchmark | Result |
|--------|-----------|--------|
| **Vector** | SIMD Dot Product (768D) | XX ns |
| **Vector** | HNSW Search (10K, k=10) | XX µs |
| **Vector** | Recall@10 (ef=128) | XX% |
| **Graph** | BFS Traversal (depth=3) | XX µs |
| **Graph** | Edge throughput | XX M edges/sec |
| **ColumnStore** | Filter throughput | XX M rows/sec |
| **ColumnStore** | vs JSON filtering | XXx faster |
| **Hybrid** | Vector + Graph + Filter | XX µs |
| **Coverage** | Line coverage | XX% |
```

### Format pour RELEASE_METRICS (Complet)

```markdown
# VelesDB v{VERSION} Release Metrics

## 📊 Test Coverage
| Metric | Value |
|--------|-------|
| **Total Tests** | **X,XXX** |
| **Tests Passing** | **X,XXX** (XX.X%) |
| **Line Coverage** | **XX.XX%** |

## ⚡ Vector Performance (SIMD + HNSW)

### SIMD Distance (768D)
| Operation | Latency | Throughput |
|-----------|---------|------------|
| Dot Product | XX ns | XX M/sec |
| Euclidean | XX ns | XX M/sec |
| Cosine | XX ns | XX M/sec |
| Hamming | XX ns | XX M/sec |
| Jaccard | XX ns | XX M/sec |

### HNSW Search (10K vectors, 128D)
| ef_search | Recall@10 | Latency P50 |
|-----------|-----------|-------------|
| 64 | XX% | XX µs |
| 128 | XX% | XX µs |
| 256 | XX% | XX µs |

## 🕸️ Graph Performance

### Edge Operations
| Operation | Latency |
|-----------|---------|
| add_edge | XX ns |
| remove_edge | XX ns |
| get_outgoing | XX ns |
| get_incoming | XX ns |

### Traversal (10K edges)
| Depth | BFS | DFS |
|-------|-----|-----|
| 1 hop | XX µs | XX µs |
| 2 hops | XX µs | XX µs |
| 3 hops | XX µs | XX µs |
| 5 hops | XX µs | XX µs |

## 📊 ColumnStore Performance

### Filter Operations (100K rows)
| Filter | Throughput |
|--------|------------|
| eq | XX M/sec |
| range (gt/lt) | XX M/sec |
| IN list | XX M/sec |
| LIKE | XX M/sec |
| AND/OR | XX M/sec |

### Aggregations
| Operation | Latency |
|-----------|---------|
| COUNT | XX µs |
| SUM/AVG | XX µs |
| GROUP BY | XX µs |

## 🔀 Hybrid Performance
| Combination | Latency |
|-------------|---------|
| Vector + Filter | XX µs |
| Vector + Graph | XX µs |
| Graph + Filter | XX µs |
| Vector + Graph + Filter | XX µs |
```

---

## ⚠️ Notes importantes

1. **Exécuter sur la même machine** pour des résultats comparables
2. **Mode release** obligatoire pour les benchmarks
3. **Fermer les autres applications** pour éviter les interférences
4. **Vérifier les régressions** avant de committer
5. **Mettre à jour la date** dans tous les fichiers modifiés

---

## 🔄 Fréquence recommandée

- **Avant chaque release majeure/mineure**
- **Après optimisations de performance significatives**
- **Trimestriellement** pour les releases patch

