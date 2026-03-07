# 🎯 Search Modes - Guide de Configuration du Recall

*Version 1.5.0 -- Mars 2026*

Guide complet pour configurer le compromis **recall vs latence** dans VelesDB. Couvre la recherche dense (HNSW), sparse (SPLADE/BM42), et hybride (dense+sparse avec fusion). Comparaison avec les pratiques Milvus, OpenSearch et Qdrant.

---

## Table des Matières

1. [Vue d'ensemble](#vue-densemble)
2. [Les 5 Modes de Recherche](#les-5-modes-de-recherche)
3. [Parametres HNSW detailles](#paramètres-hnsw-détaillés)
4. [Sparse Vector Search](#sparse-vector-search)
5. [Hybrid Search](#hybrid-search)
6. [Fusion Strategies](#fusion-strategies)
7. [Comparaison avec la concurrence](#comparaison-avec-la-concurrence)
8. [Guide de configuration par cas d'usage](#guide-de-configuration-par-cas-dusage)
9. [API et exemples](#api-et-exemples)
10. [Benchmarks](#benchmarks)
11. [FAQ](#faq)

---

## Vue d'ensemble

### Qu'est-ce que le Recall ?

Le **recall@k** mesure le pourcentage de vrais voisins les plus proches retrouvés parmi les k résultats retournés.

```
Recall@10 = (Nombre de vrais top-10 retrouvés) / 10 × 100%
```

| Recall | Signification |
|--------|---------------|
| **100%** | Tous les vrais voisins trouvés (recherche exacte) |
| **95-99%** | Excellent, suffisant pour 99% des cas RAG/recommandation |
| **90-95%** | Acceptable pour exploration/prototypage |
| **< 90%** | Risque de résultats manquants importants |

### Le compromis fondamental

```
                    Latence
                        ↑
                        │
          Fast ●────────┤  < 1ms    (~90% recall)
                        │
      Balanced ●────────┤  ~2ms     (~98% recall)
                        │
      Accurate ●────────┤  ~5ms     (~99% recall)
                        │
    HighRecall ●────────┤  ~15ms    (~99.7% recall)
                        │
       Perfect ●────────┤  ~50ms+   (100% recall, bruteforce)
                        │
        ────────────────┴────────────────→ Recall
                   90%      95%      99%   100%
```

---

## Les 5 Modes de Recherche

VelesDB expose 5 **presets** prédéfinis via l'enum `SearchQuality` :

### 1. Fast — Latence minimale

| Paramètre | Valeur |
|-----------|--------|
| `ef_search` | `max(64, k × 2)` |
| Recall typique | ~90% |
| Latence (100K vecs, 768D) | < 1 ms |

**Cas d'usage :**
- Autocomplétion temps réel
- Suggestions "as-you-type"
- Prototypage rapide

```rust
collection.search_with_quality(&query, 10, SearchQuality::Fast);
```

---

### 2. Balanced — Défaut recommandé ⭐

| Paramètre | Valeur |
|-----------|--------|
| `ef_search` | `max(128, k × 4)` |
| Recall typique | ~98% |
| Latence (100K vecs, 768D) | ~2 ms |

**Cas d'usage :**
- RAG / Retrieval-Augmented Generation
- Recherche sémantique générale
- Chatbots avec contexte

```rust
// Défaut si non spécifié
collection.search(&query, 10);
```

---

### 3. Accurate — Haute précision

| Paramètre | Valeur |
|-----------|--------|
| `ef_search` | `max(256, k × 8)` |
| Recall typique | ~99% |
| Latence (100K vecs, 768D) | ~5 ms |

**Cas d'usage :**
- Recherche de documents légaux
- E-commerce (recommandations produit)
- Détection de plagiat

```rust
collection.search_with_quality(&query, 10, SearchQuality::Accurate);
```

---

### 4. HighRecall — Précision maximale ANN

| Paramètre | Valeur |
|-----------|--------|
| `ef_search` | `max(1024, k × 32)` |
| Recall typique | ~99.7% |
| Latence (100K vecs, 768D) | ~15 ms |

**Cas d'usage :**
- Recherche médicale/scientifique
- Audit de conformité
- Déduplication critique

```rust
collection.search_with_quality(&query, 10, SearchQuality::HighRecall);
```

---

### 5. Perfect — 100% Recall garanti

| Paramètre | Valeur |
|-----------|--------|
| Algorithme | **Brute-force SIMD** (pas HNSW) |
| Recall | **100%** garanti |
| Latence (100K vecs, 768D) | ~50 ms |
| Latence (1M vecs, 768D) | ~500 ms |

**Cas d'usage :**
- Validation/benchmark du recall HNSW
- Recherche légale/médico-légale
- Petits datasets critiques (< 50K vecteurs)

```rust
collection.search_with_quality(&query, 10, SearchQuality::Perfect);
```

> ⚠️ **Attention** : Le mode Perfect effectue un scan complet de tous les vecteurs. À éviter pour les datasets > 500K vecteurs en temps réel.

---

## Paramètres HNSW détaillés

### Paramètres de construction (index-time)

| Paramètre | Description | Défaut VelesDB | Impact |
|-----------|-------------|----------------|--------|
| `M` | Connexions par nœud | **32-64** (auto) | ↑ M = ↑ recall, ↑ mémoire |
| `ef_construction` | Taille du pool de candidats à la construction | **400-800** (auto) | ↑ ef = ↑ qualité index, ↑ temps build |

### Paramètres de recherche (query-time)

| Paramètre | Description | Range | Impact |
|-----------|-------------|-------|--------|
| `ef_search` | Taille du pool de candidats à la recherche | 64 - 2048+ | ↑ ef = ↑ recall, ↑ latence |
| `k` | Nombre de résultats demandés | 1 - 1000 | Doit être ≤ ef_search |

### Règle d'or

```
ef_search ≥ k × multiplicateur

Multiplicateur recommandé par mode:
- Fast:      2x
- Balanced:  4x
- Accurate:  8x
- HighRecall: 32x
```

### Auto-scaling de VelesDB

VelesDB ajuste automatiquement `M` et `ef_construction` selon la dimension des vecteurs :

| Dimension | M | ef_construction | Justification |
|-----------|---|-----------------|---------------|
| 0-256 | 24 | 300 | Petits embeddings (word2vec) |
| 257-768 | 32 | 400 | Embeddings standards (BERT, OpenAI) |
| 769-1536 | 48 | 600 | Grands embeddings (text-embedding-3-large) |
| > 1536 | 64 | 800 | Très grandes dimensions |

---

## Sparse Vector Search

### Vue d'ensemble

La recherche sparse utilise des vecteurs creux (sparse vectors) ou seules quelques dimensions ont des valeurs non nulles. Ce format est typique des modeles de recherche par mots-cles comme **SPLADE**, **BM42**, ou **TF-IDF**.

```
Dense vector:  [0.12, 0.45, 0.03, 0.67, 0.22, ...]  (toutes dimensions)
Sparse vector: {42: 0.8, 156: 0.3, 891: 0.5}         (quelques dimensions)
```

### Format des Sparse Vectors

VelesDB stocke les sparse vectors comme des paires `(indice, valeur)` :

```json
{
  "sparse_vector": {
    "default": {
      "indices": [42, 156, 891, 2048],
      "values":  [0.8, 0.3, 0.5, 0.1]
    }
  }
}
```

Les sparse vectors supportent les **named vectors** : un point peut avoir plusieurs sparse vectors nommes (par exemple `"bm25"`, `"splade"`).

### Scoring

La similarite sparse est calculee par **inner product** (produit scalaire) sur les dimensions partagees :

```
score = sum(query[i] * doc[i]) pour tout i ou les deux vecteurs ont une valeur
```

### Algorithmes de Recherche

VelesDB selectionne automatiquement l'algorithme optimal :

| Algorithme | Condition | Description |
|-----------|-----------|-------------|
| **MaxScore DAAT** | Defaut | Document-At-A-Time avec early termination. Trie les termes par contribution et ignore les termes qui ne peuvent pas ameliorer le top-K |
| **Linear Scan** | > 30% couverture | Scan lineaire quand la requete couvre plus de 30% des documents (total_postings > 0.3 * doc_count * query_nnz) |

### Accumulateur

| Taille du corpus | Accumulateur |
|-----------------|-------------|
| <= 10M documents | Dense array (O(1) acces) |
| > 10M documents | FxHashMap (memoire proportionnelle aux hits) |

### Exemple VelesQL

```sql
-- Recherche sparse uniquement
SELECT * FROM docs WHERE vector SPARSE_NEAR $keywords LIMIT 10

-- Avec filtre metadata
SELECT * FROM docs
WHERE vector SPARSE_NEAR $bm25_query AND category = 'tech'
LIMIT 20
```

### Exemple REST API

```bash
curl -X POST http://localhost:8080/collections/docs/search/sparse \
  -H "Content-Type: application/json" \
  -d '{
    "sparse_vector": {"indices": [42, 156, 891], "values": [0.8, 0.3, 0.5]},
    "top_k": 10
  }'
```

### Exemple Python SDK

```python
from velesdb import VelesDB

db = VelesDB("./data")
coll = db.collection("docs")

results = coll.search(
    sparse_vector={"indices": [42, 156, 891], "values": [0.8, 0.3, 0.5]},
    top_k=10
)
```

---

## Hybrid Search

### Vue d'ensemble

La recherche hybride combine la recherche **dense** (embeddings semantiques) et **sparse** (mots-cles) pour obtenir le meilleur des deux mondes :

- **Dense** : comprend le sens semantique ("voiture" ~ "automobile")
- **Sparse** : precision sur les termes exacts ("RUSTSEC-2025-0141")
- **Hybrid** : combine les deux pour un recall superieur

### Quand utiliser chaque mode

| Mode | Forces | Faiblesses | Cas d'usage |
|------|--------|-----------|-------------|
| **Dense seul** | Semantique, langues, paraphrases | Termes techniques rares | RAG general, chatbots |
| **Sparse seul** | Termes exacts, acronymes, codes | Pas de comprehension semantique | Recherche de logs, codes erreur |
| **Hybride** | Combine les deux | Plus de calcul | Production RAG, e-commerce |

### Exemple VelesQL

```sql
-- Hybrid search avec RRF
SELECT * FROM products
WHERE vector NEAR $embedding AND vector SPARSE_NEAR $bm25
FUSE BY RRF(k=60)
LIMIT 10

-- Hybrid search avec poids explicites
SELECT * FROM docs
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
FUSE BY RSF(dense_weight=0.7, sparse_weight=0.3)
LIMIT 20
```

### Exemple REST API

```bash
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.1, 0.2, 0.3, ...],
    "sparse_vector": {"indices": [42, 156], "values": [0.8, 0.3]},
    "top_k": 10
  }'
```

Quand les deux champs `vector` et `sparse_vector` sont fournis, VelesDB execute automatiquement une recherche hybride avec fusion RRF (k=60) par defaut.

### Exemple Python SDK

```python
results = coll.search(
    vector=[0.1, 0.2, 0.3, ...],
    sparse_vector={"indices": [42, 156], "values": [0.8, 0.3]},
    top_k=10
)
```

### Execution parallele

Avec le feature flag `persistence` active (defaut), les branches dense et sparse sont executees en parallele via `rayon::join`. Sans `persistence`, elles sont executees sequentiellement.

---

## Fusion Strategies

### RRF (Reciprocal Rank Fusion)

RRF combine les resultats par **position dans le classement**. Le score fusionne est :

```
score_rrf(d) = 1/(k + rank_dense(d)) + 1/(k + rank_sparse(d))
```

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `k` | 60 | Constante de ranking. k plus petit = plus de poids aux premiers rangs |

**Avantages :**
- Pas besoin de normaliser les scores
- Robuste aux differences d'echelle entre dense et sparse
- Defaut recommande pour la plupart des cas

**VelesQL :**
```sql
FUSE BY RRF(k=60)
```

### RSF (Reciprocal Score Fusion)

RSF combine les resultats par **scores normalises** avec des poids explicites :

```
score_rsf(d) = dense_weight * norm(score_dense(d)) + sparse_weight * norm(score_sparse(d))
```

La normalisation est min-max par branche. `dense_weight + sparse_weight` doit valoir 1.0.

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `dense_weight` | 0.5 | Poids du score dense |
| `sparse_weight` | 0.5 | Poids du score sparse |

**Avantages :**
- Controle fin sur l'importance relative de chaque source
- Utile quand une source est systematiquement plus fiable

**VelesQL :**
```sql
FUSE BY RSF(dense_weight=0.7, sparse_weight=0.3)
```

### Comparaison RRF vs RSF

| Aspect | RRF | RSF |
|--------|-----|-----|
| **Parametrage** | 1 parametre (k) | 2 parametres (poids) |
| **Normalisation** | Par rang (implicite) | Par score (min-max) |
| **Quand utiliser** | Defaut, pas de tuning | Quand une source est plus fiable |
| **Robustesse** | Tres robuste | Sensible a la distribution des scores |

---

## Comparaison avec la concurrence

### VelesDB vs Milvus

| Aspect | VelesDB | Milvus |
|--------|---------|--------|
| **Presets** | 5 modes nommés (Fast→Perfect) | Pas de presets, `search_params` manuels |
| **100% recall** | `SearchQuality::Perfect` (bruteforce) | `FLAT` index séparé |
| **Paramètre principal** | `SearchQuality` enum | `params={"ef": N}` |
| **Auto-tuning** | ✅ Basé sur dimension | ❌ Manuel |

**Équivalence Milvus :**
```python
# Milvus
search_params = {"metric_type": "COSINE", "params": {"ef": 128}}

# VelesDB équivalent
SearchQuality::Balanced  // ef_search = 128
```

### VelesDB vs OpenSearch

| Aspect | VelesDB | OpenSearch k-NN |
|--------|---------|-----------------|
| **Presets** | 5 modes | Pas de presets |
| **100% recall** | Mode Perfect | `"method": "exact"` dans mapping |
| **Paramètre** | `SearchQuality` | `ef_search` dans query |
| **Approche** | Query-time | Query-time ou index-time |

**Équivalence OpenSearch :**
```json
// OpenSearch
{
  "query": {
    "knn": {
      "vector_field": {
        "vector": [...],
        "k": 10,
        "ef_search": 256
      }
    }
  }
}

// VelesDB équivalent
SearchQuality::Accurate  // ef_search = 256
```

### VelesDB vs Qdrant

| Aspect | VelesDB | Qdrant |
|--------|---------|--------|
| **Presets** | 5 modes | Pas de presets officiels |
| **100% recall** | Mode Perfect | `exact: true` dans search |
| **Paramètre** | `SearchQuality` | `hnsw_ef` dans search params |
| **Quantization** | SQ8, Binary | Scalar, Product |

**Équivalence Qdrant :**
```json
// Qdrant
{
  "vector": [...],
  "limit": 10,
  "params": { "hnsw_ef": 128, "exact": false }
}

// VelesDB équivalent
SearchQuality::Balanced
```

### Tableau récapitulatif des équivalences

| VelesDB Mode | ef_search | Milvus ef | OpenSearch ef_search | Qdrant hnsw_ef |
|--------------|-----------|-----------|----------------------|----------------|
| Fast | 64 | 64 | 64 | 64 |
| Balanced | 128 | 128 | 128 | 128 |
| Accurate | 256 | 256 | 256 | 256 |
| HighRecall | 1024 | 1024 | 1024 | 1024 |
| Perfect | N/A (bruteforce) | FLAT index | `"exact": true` | `"exact": true` |

---

## Guide de configuration par cas d'usage

### 🤖 RAG / Chatbot

```rust
// Configuration recommandée
SearchQuality::Balanced  // 98% recall, ~2ms

// Si réponses critiques (médical, légal)
SearchQuality::Accurate  // 99% recall, ~5ms
```

### 🛒 E-commerce / Recommandations

```rust
// Suggestions temps réel
SearchQuality::Fast  // 90% recall, < 1ms

// Page produit (précision importante)
SearchQuality::Balanced  // 98% recall
```

### 🔍 Recherche documentaire

```rust
// Recherche exploratoire
SearchQuality::Balanced

// Recherche légale / audit
SearchQuality::HighRecall  // ou Perfect pour petits corpus
```

### 🧬 Recherche scientifique/médicale

```rust
// Papers, séquences génomiques
SearchQuality::HighRecall  // 99.7% recall

// Validation finale
SearchQuality::Perfect  // 100% recall garanti
```

### 📱 Mobile / Edge / IoT

```rust
// Latence critique, batterie limitée
SearchQuality::Fast

// Avec quantization binaire pour mémoire
HnswParams::with_binary(dimension)
```

### 🔄 Déduplication / Near-duplicate detection

```rust
// Détection de duplicatas exacts
SearchQuality::Perfect  // Aucun faux négatif

// Détection approximative (OK si quelques doublons échappent)
SearchQuality::Accurate
```

---

## API et exemples

### Rust

```rust
use velesdb_core::{Collection, SearchQuality};

// Méthode 1: Mode par défaut (Balanced)
let results = collection.search(&query_vector, 10)?;

// Méthode 2: Mode explicite
let results = collection.search_with_quality(
    &query_vector, 
    10, 
    SearchQuality::HighRecall
)?;

// Méthode 3: ef_search personnalisé
let results = collection.search_with_ef(&query_vector, 10, 512)?;

// Méthode 4: Mode parfait (bruteforce)
let results = collection.search_with_quality(
    &query_vector, 
    10, 
    SearchQuality::Perfect
)?;
```

### REST API

```bash
# Mode par défaut (Balanced)
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10}'

# ef_search personnalisé
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10, "ef_search": 512}'
```

### VelesQL (v0.8.0+)

```sql
-- Mode par défaut
SELECT * FROM my_collection 
WHERE vector NEAR COSINE $query 
LIMIT 10;

-- Avec mode explicite (syntaxe proposée)
SELECT * FROM my_collection 
WHERE vector NEAR COSINE $query 
LIMIT 10
WITH (mode = 'high_recall');

-- Avec ef_search personnalisé
SELECT * FROM my_collection 
WHERE vector NEAR COSINE $query 
LIMIT 10
WITH (ef_search = 512);
```

### CLI REPL (v0.8.0+)

```
velesdb> \set search_mode balanced
Search mode set to: Balanced (ef_search=128)

velesdb> \set ef_search 256
ef_search set to: 256

velesdb> \show settings
┌─────────────────┬──────────┐
│ Setting         │ Value    │
├─────────────────┼──────────┤
│ search_mode     │ Balanced │
│ ef_search       │ 256      │
│ default_limit   │ 10       │
└─────────────────┴──────────┘

velesdb> SEARCH TOP 10 IN products WHERE vector NEAR $v;
```

---

## Benchmarks

### Conditions de test

- **CPU** : AMD Ryzen 9 5900X (12 cores)
- **RAM** : 64 GB DDR4
- **Dataset** : 100K vecteurs, 768 dimensions (OpenAI embeddings)
- **Métrique** : Cosine similarity

### Résultats

| Mode | ef_search | Recall@10 | Latence p50 | Latence p99 | QPS |
|------|-----------|-----------|-------------|-------------|-----|
| Fast | 64 | 89.2% | 0.8 ms | 1.5 ms | 12,500 |
| Balanced | 128 | 97.8% | 1.9 ms | 3.2 ms | 5,200 |
| Accurate | 256 | 99.1% | 4.1 ms | 6.8 ms | 2,400 |
| HighRecall | 1024 | 99.7% | 14.2 ms | 22.1 ms | 700 |
| Perfect | N/A | 100.0% | 48.3 ms | 52.1 ms | 207 |

### Scaling avec le dataset

| Dataset Size | Balanced Latency | Perfect Latency | Ratio |
|--------------|------------------|-----------------|-------|
| 10K | 0.4 ms | 5 ms | 12x |
| 100K | 1.9 ms | 48 ms | 25x |
| 500K | 3.2 ms | 240 ms | 75x |
| 1M | 4.8 ms | 480 ms | 100x |

> **Observation** : Le mode Perfect scale linéairement O(n), tandis que HNSW scale en O(log n). Pour les grands datasets, préférer HighRecall au lieu de Perfect.

---

## FAQ

### Q: Quel mode choisir pour du RAG ?

**R:** `Balanced` (défaut) convient à 95% des cas RAG. Si vous avez des exigences légales/médicales, utilisez `Accurate`.

### Q: Le mode Perfect est-il vraiment 100% recall ?

**R:** Oui, garanti. Il effectue un calcul de distance brute-force sur tous les vecteurs, sans approximation HNSW.

### Q: Puis-je utiliser Perfect en production ?

**R:** Oui, mais avec précautions :
- Datasets < 50K : Acceptable (~25ms)
- Datasets 50K-200K : Cas critiques seulement
- Datasets > 200K : Recommandé uniquement en batch/offline

### Q: Comment mesurer le recall de mon index ?

**R:** Comparez les résultats ANN vs Perfect sur un échantillon :

```rust
// Benchmark recall
let ann_results = collection.search_with_quality(&query, 10, SearchQuality::Balanced);
let exact_results = collection.search_with_quality(&query, 10, SearchQuality::Perfect);

let recall = calculate_recall(&ann_results, &exact_results);
println!("Recall@10: {:.1}%", recall * 100.0);
```

### Q: ef_search peut-il être > nombre de vecteurs ?

**R:** Oui, mais c'est équivalent à un bruteforce. VelesDB bascule automatiquement sur Perfect si `ef_search` > seuil.

### Q: Milvus utilise `ef` et VelesDB `ef_search`, c'est pareil ?

**R:** Oui, c'est la même chose. `ef_search` est le nom standard dans la littérature HNSW.

---

## Ressources

- [HNSW Paper original (Malkov & Yashunin, 2018)](https://arxiv.org/abs/1603.09320)
- [Milvus HNSW tuning guide](https://milvus.io/docs/index-with-milvus.md)
- [OpenSearch k-NN performance guide](https://opensearch.org/docs/latest/search-plugins/knn/performance-tuning/)
- [Qdrant HNSW configuration](https://qdrant.tech/documentation/concepts/indexing/)

---

*Documentation VelesDB -- Mars 2026*
