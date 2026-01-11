# VelesDB v1.1.0 - Guide d'Usage Complet

> **Document pour revue par les 3 relecteurs**  
> **Date** : 11 janvier 2026  
> **EPICs** : EPIC-CORE-003 (SOTA Optimizations) + EPIC-CORE-005 (Full Coverage)

---

## 📋 Table des matières

1. [Multi-Query Fusion (MQG)](#1-multi-query-fusion-mqg)
2. [Hybrid Search](#2-hybrid-search)
3. [Batch Search](#3-batch-search)
4. [Text Search (BM25)](#4-text-search-bm25)
5. [LIKE/ILIKE Filters](#5-likeilike-filters)
6. [Metadata-Only Collections](#6-metadata-only-collections)
7. [Hamming/Jaccard Metrics](#7-hammingjaccard-metrics)
8. [Cache & Performance (SOTA)](#8-cache--performance-sota)

---

## 1. Multi-Query Fusion (MQG)

### Scénario d'usage
RAG pipeline avec Multiple Query Generation : l'utilisateur pose une question, le LLM génère 3 reformulations, VelesDB fusionne les résultats.

### Rust (Core)

```rust
use velesdb_core::{Collection, FusionStrategy};

// Génération de 3 embeddings pour la même question
let queries = vec![
    embed("What is VelesDB?"),           // Query originale
    embed("VelesDB vector database"),     // Reformulation 1
    embed("How does VelesDB work?"),      // Reformulation 2
];

// RRF Fusion (recommandé pour RAG)
let results = collection.multi_query_search(
    &queries.iter().map(|v| v.as_slice()).collect::<Vec<_>>(),
    10,  // top_k
    FusionStrategy::RRF { k: 60 },
    None,  // no filter
)?;

// Weighted Fusion (style SearchXP)
let results = collection.multi_query_search(
    &query_refs,
    10,
    FusionStrategy::Weighted {
        avg_weight: 0.6,
        max_weight: 0.3,
        hit_weight: 0.1,
    },
    None,
)?;
```

### Python

```python
from velesdb import Database, FusionStrategy

db = Database("./data")
collection = db.get_collection("documents")

# Multi-query avec RRF
results = collection.multi_query_search(
    vectors=[query1, query2, query3],
    top_k=10,
    fusion=FusionStrategy.rrf(k=60)
)

# Avec filtre
results = collection.multi_query_search(
    vectors=[query1, query2],
    top_k=10,
    fusion=FusionStrategy.weighted(avg=0.6, max=0.3, hit=0.1),
    filter={"category": "tech"}
)
```

### Swift (iOS)

```swift
import VelesDB

let collection = try db.getCollection(name: "documents")!

// Multi-query search
let results = try collection.multiQuerySearch(
    vectors: [query1, query2, query3],
    limit: 10,
    strategy: .rrf(k: 60)
)

for result in results {
    print("ID: \(result.id), Score: \(result.score)")
}
```

### Kotlin (Android)

```kotlin
import com.velesdb.mobile.*

val collection = db.getCollection("documents")!!

// Multi-query search avec RRF
val results = collection.multiQuerySearch(
    vectors = listOf(query1, query2, query3),
    limit = 10u,
    strategy = FusionStrategy.Rrf(k = 60u)
)

results.forEach { result ->
    println("ID: ${result.id}, Score: ${result.score}")
}
```

### TypeScript SDK

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

// Multi-query search
const results = await db.multiQuerySearch('documents', [emb1, emb2, emb3], {
  k: 10,
  fusion: 'rrf',
  fusionParams: { k: 60 }
});

// Weighted fusion
const results = await db.multiQuerySearch('documents', [emb1, emb2], {
  k: 10,
  fusion: 'weighted',
  fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
});
```

### JavaScript (WASM Browser)

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

await init();
const store = new VectorStore(768, 'cosine');

// Insert vectors...

// Multi-query search (flatten vectors into single Float32Array)
const vectors = new Float32Array([...query1, ...query2, ...query3]);
const results = store.multi_query_search(vectors, 3, 10, 'rrf', 60);
```

### CLI

```bash
# Multi-query search avec RRF
velesdb multi-search ./data documents \
  --vectors '[[0.1, 0.2, ...], [0.3, 0.4, ...]]' \
  --top-k 10 \
  --strategy rrf \
  --rrf-k 60

# Output JSON
velesdb multi-search ./data documents \
  --vectors '[[...], [...]]' \
  --format json
```

### LangChain

```python
from langchain_velesdb import VelesDBVectorStore

vectorstore = VelesDBVectorStore(
    path="./data",
    embedding=OpenAIEmbeddings()
)

# Multi-query retriever
results = vectorstore.multi_query_search(
    queries=["query1", "query2", "query3"],
    k=10,
    fusion="rrf",
    fusion_k=60
)
```

### LlamaIndex

```python
from llamaindex_velesdb import VelesDBVectorStore

vector_store = VelesDBVectorStore(path="./data")

# Multi-query search
results = vector_store.multi_query_search(
    query_embeddings=[emb1, emb2, emb3],
    similarity_top_k=10,
    fusion_strategy="rrf"
)
```

---

## 2. Hybrid Search

### Scénario d'usage
Recherche combinant similarité vectorielle et recherche textuelle BM25 pour améliorer la pertinence.

### Rust

```rust
let results = collection.hybrid_search(
    &query_vector,
    "machine learning rust",  // text query
    10,                       // top_k
    Some(0.7),               // 70% vector, 30% text
)?;
```

### Python

```python
results = collection.hybrid_search(
    vector=query_vector,
    text_query="machine learning rust",
    top_k=10,
    vector_weight=0.7
)
```

### Swift

```swift
let results = try collection.hybridSearch(
    vector: queryVector,
    textQuery: "machine learning",
    limit: 10,
    vectorWeight: 0.7
)
```

### TypeScript

```typescript
const results = await db.hybridSearch(
  'documents',
  queryVector,
  'machine learning rust',
  { k: 10, vectorWeight: 0.7 }
);
```

### WASM

```javascript
const results = store.hybrid_search(
  queryVector,    // Float32Array
  "machine learning",
  10,             // k
  0.7,            // vector_weight
  null            // field (optional)
);
```

---

## 3. Batch Search

### Scénario d'usage
Recherche parallèle pour plusieurs requêtes en une seule opération (optimisation I/O).

### Rust

```rust
let searches = vec![
    SearchRequest { vector: v1, top_k: 5, filter: None },
    SearchRequest { vector: v2, top_k: 10, filter: Some(filter) },
];

let all_results = collection.search_batch(&searches)?;
// all_results[0] = résultats pour v1
// all_results[1] = résultats pour v2
```

### Python

```python
results = collection.batch_search([
    {"vector": v1, "top_k": 5},
    {"vector": v2, "top_k": 10, "filter": {"category": "tech"}},
])
```

### Swift

```swift
let searches = [
    IndividualSearchRequest(vector: v1, topK: 5, filter: nil),
    IndividualSearchRequest(vector: v2, topK: 10, filter: filterJson),
]
let results = try collection.batchSearch(searches: searches)
```

### TypeScript

```typescript
const results = await db.searchBatch('documents', [
  { vector: v1, k: 5 },
  { vector: v2, k: 10, filter: { category: 'tech' } },
]);
```

### WASM

```javascript
// Flatten all vectors
const vectors = new Float32Array([...v1, ...v2, ...v3]);
const results = store.batch_search(vectors, 3, 10);  // 3 vectors, top 10
```

---

## 4. Text Search (BM25)

### Scénario d'usage
Recherche full-text sans vecteurs, idéal pour recherche par mots-clés.

### Rust

```rust
let results = collection.text_search("rust programming", 10);
```

### Python

```python
results = collection.text_search("rust programming", top_k=10)
```

### Swift

```swift
let results = collection.textSearch(query: "rust programming", limit: 10)
```

### TypeScript

```typescript
const results = await db.textSearch('documents', 'rust programming', { k: 10 });
```

### WASM

```javascript
const results = store.text_search("rust programming", 10, null);
```

---

## 5. LIKE/ILIKE Filters

### Scénario d'usage
Filtrage par pattern sur les champs texte des métadonnées.

### Rust

```rust
use velesdb_core::{Filter, Condition};

// LIKE case-sensitive
let filter = Filter::new(Condition::Like {
    field: "title".to_string(),
    pattern: "%rust%".to_string(),
});

// ILIKE case-insensitive
let filter = Filter::new(Condition::ILike {
    field: "description".to_string(),
    pattern: "%machine learning%".to_string(),
});

let results = collection.search_with_filter(&query, 10, &filter)?;
```

### Python

```python
# ILIKE (case-insensitive)
results = collection.search(
    vector=query,
    top_k=10,
    filter={"description": {"$ilike": "%machine learning%"}}
)

# LIKE (case-sensitive)
results = collection.search(
    vector=query,
    top_k=10,
    filter={"code": {"$like": "PROD%"}}
)
```

### VelesQL

```sql
-- ILIKE in VelesQL
SELECT * FROM documents
WHERE title ILIKE '%rust%'
AND VECTOR NEAR $query
LIMIT 10;
```

### TypeScript

```typescript
const results = await db.search('documents', query, {
  k: 10,
  filter: {
    condition: { type: 'ilike', field: 'title', pattern: '%rust%' }
  }
});
```

---

## 6. Metadata-Only Collections

### Scénario d'usage
Stocker des données structurées sans vecteurs (tables de référence, configurations).

### Rust

```rust
use velesdb_core::{Database, CollectionType};

let db = Database::open("./data")?;

// Créer une collection metadata-only
db.create_collection_typed("settings", &CollectionType::MetadataOnly)?;

let collection = db.get_collection("settings").unwrap();

// Insérer des métadonnées sans vecteur
collection.upsert_metadata(vec![
    Point::metadata_only(1, json!({"key": "theme", "value": "dark"})),
    Point::metadata_only(2, json!({"key": "language", "value": "fr"})),
])?;

// Vérifier le type
assert!(collection.is_metadata_only());
```

### Python

```python
db = Database("./data")

# Créer collection metadata-only
db.create_metadata_collection("settings")

collection = db.get_collection("settings")
assert collection.is_metadata_only()

# Insérer métadonnées
collection.upsert_metadata([
    {"id": 1, "payload": {"key": "theme", "value": "dark"}},
    {"id": 2, "payload": {"key": "language", "value": "fr"}},
])
```

### Swift

```swift
// Créer collection metadata-only
try db.createMetadataCollection(name: "settings")

let collection = try db.getCollection(name: "settings")!
print("Is metadata-only: \(collection.isMetadataOnly())")
```

### CLI

```bash
# Créer collection metadata-only
velesdb create-metadata-collection ./data settings
```

### WASM

```javascript
// Créer un store metadata-only
const store = VectorStore.new_metadata_only();

// Vérifier le type
console.log(store.is_metadata_only);  // true
```

---

## 7. Hamming/Jaccard Metrics

### Scénario d'usage
Similarité pour vecteurs binaires (fingerprints, hash signatures).

### Rust

```rust
use velesdb_core::DistanceMetric;

// Créer collection avec métrique Hamming
let collection = db.create_collection(
    "fingerprints",
    128,  // dimension
    DistanceMetric::Hamming,
)?;

// Ou Jaccard pour similarité d'ensembles
let collection = db.create_collection(
    "sets",
    256,
    DistanceMetric::Jaccard,
)?;
```

### Python

```python
# Hamming pour vecteurs binaires
collection = db.create_collection(
    "fingerprints",
    dimension=128,
    metric="hamming"
)

# Jaccard pour similarité d'ensembles
collection = db.create_collection(
    "sets",
    dimension=256,
    metric="jaccard"
)
```

### Swift

```swift
try db.createCollection(
    name: "fingerprints",
    dimension: 128,
    metric: .hamming
)
```

### TypeScript

```typescript
await db.createCollection('fingerprints', {
  dimension: 128,
  metric: 'hamming'
});
```

### WASM

```javascript
const store = new VectorStore(128, 'hamming');
// ou
const store = new VectorStore(256, 'jaccard');
```

---

## 8. Cache & Performance (SOTA)

### LRU Cache (EPIC-CORE-003)

```rust
use velesdb_core::cache::LruCache;

// Cache thread-safe O(1)
let cache: LruCache<String, Vec<f32>> = LruCache::new(10000);

cache.insert("key1".to_string(), vec![0.1, 0.2]);
let value = cache.get(&"key1".to_string());

// Stats
let stats = cache.stats();
println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);
```

### Bloom Filter

```rust
use velesdb_core::cache::BloomFilter;

// Bloom filter avec 1% FPR
let bloom = BloomFilter::new(100000, 0.01);

bloom.insert(&"document_123");
if bloom.may_contain(&"document_123") {
    // Peut être présent (vérifier dans le store)
}
if !bloom.may_contain(&"unknown") {
    // Définitivement absent (pas de faux négatifs)
}
```

### Dictionary Compression

```rust
use velesdb_core::compression::DictionaryEncoder;

let mut encoder: DictionaryEncoder<String> = DictionaryEncoder::new();

// Encoder des valeurs répétées
let code1 = encoder.encode("France".to_string());
let code2 = encoder.encode("France".to_string());  // Même code
let code3 = encoder.encode("Germany".to_string());

// Décoder
let value = encoder.decode(code1);  // "France"

// Stats de compression
let stats = encoder.stats();
println!("Ratio: {:.2}x", stats.compression_ratio);
```

---

## 📊 Matrice de Couverture v1.1.0

| Feature | Core | Mobile | WASM | CLI | TS SDK | LangChain | LlamaIndex |
|---------|:----:|:------:|:----:|:---:|:------:|:---------:|:----------:|
| multi_query_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| hybrid_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| batch_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| text_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| LIKE/ILIKE | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hamming/Jaccard | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| metadata_only | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| get_by_id | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| FusionStrategy | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| LruCache | ✅ | - | - | - | - | - | - |
| BloomFilter | ✅ | - | - | - | - | - | - |
| DictionaryEncoder | ✅ | - | - | - | - | - | - |
| TrigramIndex | ✅ | - | - | - | - | - | - |

---

## ✅ Checklist pour les 3 relecteurs

### Relecteur 1 : Fonctionnalités
- [ ] Tous les exemples de code compilent/s'exécutent
- [ ] Les signatures d'API sont correctes
- [ ] Les retours de fonction sont documentés

### Relecteur 2 : Documentation
- [ ] Scénarios d'usage clairs et réalistes
- [ ] Cohérence entre les langages
- [ ] Exemples complets et fonctionnels

### Relecteur 3 : Tests & Couverture
- [ ] Matrice de couverture 100% verte
- [ ] Tests TDD présents pour chaque feature
- [ ] Aucun gap fonctionnel restant

---

*Document généré le 11/01/2026 pour VelesDB v1.1.0*
