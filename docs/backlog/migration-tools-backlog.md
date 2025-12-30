# 🚀 VelesDB Migration Tools - Product Backlog

**Version**: 1.0  
**Date**: 2024-12-30  
**Product Owner**: VelesDB Team  
**Status**: Draft  

---

## 📋 Executive Summary

Ce backlog définit les outils de migration vers VelesDB depuis les principales bases de données vectorielles du marché. L'objectif est de fournir une expérience de migration **simple, performante et fiable** pour faciliter l'adoption de VelesDB.

### Objectifs Stratégiques

1. **Simplicité** : Migration en une seule commande CLI
2. **Performance** : Throughput > 10K vectors/sec pour les migrations
3. **Fiabilité** : Zero data loss, reprise après interruption
4. **Extensibilité** : Architecture modulaire pour ajouter de nouvelles sources

### Sources Prioritaires (par adoption marché)

| Priority | Source | Estimation Utilisateurs | Complexité |
|----------|--------|------------------------|------------|
| P0 | Supabase/pgvector | Très haute | Moyenne |
| P0 | Pinecone | Très haute | Haute |
| P1 | Qdrant | Haute | Moyenne |
| P1 | Weaviate | Haute | Haute |
| P2 | Milvus | Moyenne | Haute |
| P2 | ChromaDB | Moyenne | Basse |

---

## 🏗️ Architecture Technique

### Vue d'ensemble

```
┌─────────────────────────────────────────────────────────────────┐
│                     VelesDB Migration CLI                        │
│  velesdb-migrate [source] --config config.yaml --target velesdb  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Migration Engine                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Source    │  │  Transform  │  │      Destination        │  │
│  │  Connectors │─▶│   Pipeline  │─▶│  (VelesDB Bulk API)     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│         │                │                     │                 │
│         ▼                ▼                     ▼                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │  Checkpoint │  │   Metrics   │  │      Validation         │  │
│  │   Manager   │  │   Reporter  │  │       Engine            │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Composants Core

| Composant | Responsabilité | Crate |
|-----------|----------------|-------|
| `Source Connectors` | Extraction données depuis bases sources | `velesdb-migrate-connectors` |
| `Transform Pipeline` | Mapping schéma, conversion formats | `velesdb-migrate-transform` |
| `Destination Writer` | Écriture batch optimisée vers VelesDB | `velesdb-migrate-core` |
| `Checkpoint Manager` | Reprise après interruption | `velesdb-migrate-core` |
| `Validation Engine` | Vérification intégrité post-migration | `velesdb-migrate-core` |

---

## 📦 Epic 1: Migration Core Framework

> **Goal**: Créer le framework de base pour toutes les migrations

### US-1.1: CLI Migration de base

**En tant que** développeur  
**Je veux** une commande CLI simple pour migrer mes données  
**Afin de** pouvoir migrer vers VelesDB sans écrire de code

#### Acceptance Criteria
- [ ] Commande `velesdb-migrate` disponible après installation
- [ ] Support fichier de configuration YAML
- [ ] Progress bar avec ETA
- [ ] Logs structurés (JSON ou pretty)
- [ ] Code de retour approprié (0 = succès, 1 = erreur)

#### Technical Notes
```rust
// Structure CLI avec clap
#[derive(Parser)]
#[command(name = "velesdb-migrate")]
#[command(about = "Migrate vector data to VelesDB")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Migrate from a source database
    From {
        #[arg(value_enum)]
        source: SourceType,
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Validate a migration
    Validate { /* ... */ },
    /// Resume an interrupted migration
    Resume { checkpoint: PathBuf },
}
```

#### Story Points: 5
#### Priority: P0 (Bloquant)

---

### US-1.2: Checkpoint & Resume System

**En tant que** opérateur  
**Je veux** pouvoir reprendre une migration interrompue  
**Afin de** ne pas perdre la progression en cas de problème réseau/serveur

#### Acceptance Criteria
- [ ] Sauvegarde automatique du checkpoint toutes les N secondes (configurable)
- [ ] Fichier checkpoint avec: offset source, derniers IDs migrés, stats
- [ ] Commande `velesdb-migrate resume <checkpoint.json>`
- [ ] Détection automatique du dernier checkpoint dans le répertoire courant
- [ ] Validation de cohérence avant reprise

#### Technical Notes
```rust
#[derive(Serialize, Deserialize)]
struct MigrationCheckpoint {
    version: u32,
    source: SourceConfig,
    destination: DestinationConfig,
    state: MigrationState,
    last_update: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct MigrationState {
    /// Total vectors processed
    processed_count: u64,
    /// Source-specific offset (scroll_id, page token, etc.)
    source_offset: serde_json::Value,
    /// Last successfully migrated IDs per collection
    last_ids: HashMap<String, Vec<String>>,
    /// Error count per category
    errors: HashMap<String, u64>,
}
```

#### Story Points: 8
#### Priority: P0 (Bloquant)

---

### US-1.3: Validation Post-Migration

**En tant que** développeur  
**Je veux** vérifier l'intégrité de ma migration  
**Afin de** m'assurer que toutes les données ont été correctement transférées

#### Acceptance Criteria
- [ ] Commande `velesdb-migrate validate`
- [ ] Vérification count source vs destination
- [ ] Sampling aléatoire avec comparaison vecteurs (distance < epsilon)
- [ ] Rapport détaillé avec métriques de qualité
- [ ] Option `--strict` pour vérification exhaustive

#### Technical Notes
```rust
struct ValidationReport {
    source_count: u64,
    destination_count: u64,
    sampled_vectors: u64,
    matching_vectors: u64,
    max_distance_delta: f32,
    avg_distance_delta: f32,
    payload_mismatches: Vec<PayloadMismatch>,
    validation_time_ms: u64,
}
```

#### Story Points: 5
#### Priority: P1

---

### US-1.4: Metrics & Monitoring

**En tant que** opérateur  
**Je veux** des métriques en temps réel pendant la migration  
**Afin de** pouvoir monitorer et diagnostiquer les problèmes

#### Acceptance Criteria
- [ ] Throughput vectors/sec en temps réel
- [ ] Latence P50/P95/P99 par batch
- [ ] Erreurs par catégorie
- [ ] Export Prometheus metrics (optionnel)
- [ ] Output JSON pour intégration CI/CD

#### Technical Notes
```rust
struct MigrationMetrics {
    start_time: Instant,
    vectors_processed: AtomicU64,
    bytes_transferred: AtomicU64,
    batches_completed: AtomicU64,
    errors: DashMap<String, AtomicU64>,
    latencies: HdrHistogram,
}
```

#### Story Points: 5
#### Priority: P1

---

### US-1.5: Configuration YAML

**En tant que** développeur  
**Je veux** un fichier de configuration déclaratif  
**Afin de** réutiliser et versionner mes configurations de migration

#### Acceptance Criteria
- [ ] Format YAML avec validation schema
- [ ] Support variables d'environnement `${ENV_VAR}`
- [ ] Documentation inline avec commentaires
- [ ] Commande `velesdb-migrate config validate`
- [ ] Templates pour chaque source

#### Technical Notes
```yaml
# Example: migrate-pinecone.yaml
version: "1"
source:
  type: pinecone
  api_key: ${PINECONE_API_KEY}
  environment: us-west1-gcp
  index: my-embeddings
  namespace: production
  
destination:
  type: velesdb
  url: http://localhost:8080
  collection: migrated-embeddings
  create_if_missing: true
  dimension: 1536
  metric: cosine

migration:
  batch_size: 1000
  parallelism: 4
  checkpoint_interval_secs: 30
  retry:
    max_attempts: 3
    backoff_ms: 1000

transform:
  # Optional field mappings
  id_field: "id"
  vector_field: "values"
  payload_mapping:
    title: "metadata.title"
    category: "metadata.category"
```

#### Story Points: 3
#### Priority: P0 (Bloquant)

---

## 📦 Epic 2: Supabase/pgvector Migration

> **Goal**: Migration depuis PostgreSQL avec extension pgvector

### US-2.1: pgvector Connector

**En tant que** utilisateur Supabase  
**Je veux** migrer mes embeddings pgvector vers VelesDB  
**Afin de** bénéficier des performances microseconde de VelesDB

#### Acceptance Criteria
- [ ] Support connexion PostgreSQL standard (connection string)
- [ ] Support Supabase hosted (avec API key)
- [ ] Extraction batch avec cursor-based pagination
- [ ] Support types: `vector`, `halfvec`, `sparsevec`
- [ ] Mapping automatique des colonnes metadata → payload

#### Technical Notes

**Schéma pgvector typique:**
```sql
CREATE TABLE documents (
    id BIGSERIAL PRIMARY KEY,
    content TEXT,
    embedding vector(1536),
    metadata JSONB,
    created_at TIMESTAMP
);
```

**Query d'extraction optimisée:**
```sql
SELECT id, embedding::text, metadata, content
FROM documents
WHERE id > $1
ORDER BY id
LIMIT $2;
```

**Mapping vers VelesDB:**
```rust
struct PgvectorRow {
    id: i64,
    embedding: String, // "[0.1, 0.2, ...]"
    metadata: Option<serde_json::Value>,
    // ... autres colonnes
}

impl Into<velesdb_core::Point> for PgvectorRow {
    fn into(self) -> Point {
        Point::new(
            self.id as u64,
            parse_pgvector_string(&self.embedding),
            self.metadata,
        )
    }
}
```

#### Story Points: 8
#### Priority: P0

---

### US-2.2: Auto-Discovery Schema pgvector

**En tant que** développeur  
**Je veux** que l'outil détecte automatiquement mon schéma  
**Afin de** ne pas avoir à configurer manuellement chaque colonne

#### Acceptance Criteria
- [ ] Détection tables avec colonnes `vector`
- [ ] Affichage dimension des vecteurs
- [ ] Listing des colonnes metadata candidates
- [ ] Génération automatique de config YAML
- [ ] Commande `velesdb-migrate discover pgvector --url <conn_string>`

#### Technical Notes
```sql
-- Query de découverte
SELECT 
    t.table_name,
    c.column_name,
    c.udt_name,
    -- Extraction dimension depuis le type
    regexp_replace(c.character_maximum_length::text, '[^0-9]', '', 'g') as dimension
FROM information_schema.tables t
JOIN information_schema.columns c ON t.table_name = c.table_name
WHERE c.udt_name IN ('vector', 'halfvec', 'sparsevec')
AND t.table_schema = 'public';
```

#### Story Points: 5
#### Priority: P1

---

### US-2.3: Supabase Integration directe

**En tant que** utilisateur Supabase  
**Je veux** utiliser mon API key Supabase directement  
**Afin de** ne pas avoir à exposer ma connection string PostgreSQL

#### Acceptance Criteria
- [ ] Auth via Supabase API key (service role)
- [ ] Support Supabase REST API ou connection pooler
- [ ] Détection automatique de l'URL de projet
- [ ] Documentation des permissions requises

#### Story Points: 5
#### Priority: P1

---

## 📦 Epic 3: Pinecone Migration

> **Goal**: Migration depuis Pinecone (serverless et pods)

### US-3.1: Pinecone Connector

**En tant que** utilisateur Pinecone  
**Je veux** migrer mes index vers VelesDB  
**Afin de** réduire mes coûts et améliorer mes latences

#### Acceptance Criteria
- [ ] Support API REST Pinecone
- [ ] Pagination via `list()` + `fetch()` pattern
- [ ] Support namespaces multiples
- [ ] Extraction metadata complète
- [ ] Gestion rate limiting (429)

#### Technical Notes

**API Pinecone - Pattern d'extraction:**
```python
# Pseudo-code de l'algorithme
for ids in index.list(namespace=ns, limit=1000):
    vectors = index.fetch(ids=ids, namespace=ns)
    for id, vec in vectors.items():
        yield Point(id, vec.values, vec.metadata)
```

**Rust implementation:**
```rust
struct PineconeConnector {
    api_key: String,
    host: String,
    namespace: Option<String>,
    client: reqwest::Client,
}

impl SourceConnector for PineconeConnector {
    async fn scroll(&self, offset: Option<String>, limit: usize) 
        -> Result<(Vec<Point>, Option<String>)> 
    {
        // 1. List IDs with pagination
        let list_resp = self.list_vectors(offset.as_deref(), limit).await?;
        
        // 2. Fetch actual vectors
        let ids: Vec<_> = list_resp.vectors.iter().map(|v| v.id.clone()).collect();
        let fetch_resp = self.fetch_vectors(&ids).await?;
        
        // 3. Convert to Points
        let points = fetch_resp.vectors.into_iter()
            .map(|(id, v)| Point::new(
                id.parse().unwrap_or_else(|_| hash_string(&id)),
                v.values,
                v.metadata,
            ))
            .collect();
        
        Ok((points, list_resp.pagination.map(|p| p.next)))
    }
}
```

#### Story Points: 8
#### Priority: P0

---

### US-3.2: ID Mapping Pinecone → VelesDB

**En tant que** développeur  
**Je veux** conserver la correspondance entre mes IDs Pinecone (string) et VelesDB (u64)  
**Afin de** pouvoir retrouver mes données après migration

#### Acceptance Criteria
- [ ] Option 1: Hash déterministe string → u64 (par défaut)
- [ ] Option 2: Stockage ID original dans payload.`_original_id`
- [ ] Option 3: Export mapping file (CSV/JSON)
- [ ] Collision detection avec warning

#### Technical Notes
```rust
fn string_id_to_u64(id: &str) -> u64 {
    // FNV-1a hash for deterministic mapping
    let mut hasher = FnvHasher::default();
    hasher.write(id.as_bytes());
    hasher.finish()
}
```

#### Story Points: 3
#### Priority: P0

---

### US-3.3: Metric Mapping Pinecone

**En tant que** développeur  
**Je veux** que ma métrique de distance soit automatiquement mappée  
**Afin de** conserver la même sémantique de recherche

#### Acceptance Criteria
- [ ] Mapping automatique: `cosine` → `Cosine`, `euclidean` → `Euclidean`, `dotproduct` → `DotProduct`
- [ ] Warning si métrique source non supportée
- [ ] Override possible via config

#### Story Points: 2
#### Priority: P0

---

## 📦 Epic 4: Qdrant Migration

> **Goal**: Migration depuis Qdrant (cloud et self-hosted)

### US-4.1: Qdrant Connector

**En tant que** utilisateur Qdrant  
**Je veux** migrer mes collections vers VelesDB  
**Afin de** tester les performances comparées

#### Acceptance Criteria
- [ ] Support API REST Qdrant
- [ ] Support gRPC (optionnel, pour performance)
- [ ] Pagination via `scroll` endpoint
- [ ] Support multivecteurs → collections multiples
- [ ] Support filtres (migration partielle)

#### Technical Notes

**API Qdrant Scroll:**
```bash
POST /collections/{collection_name}/points/scroll
{
    "limit": 1000,
    "with_payload": true,
    "with_vector": true,
    "offset": "<last_offset>"
}
```

**Mapping types:**
| Qdrant | VelesDB |
|--------|---------|
| `point_id` (u64) | `id` (u64) |
| `point_id` (uuid) | hash → `id` |
| `vector` (dense) | `vector` |
| `vector` (sparse) | ⚠️ Non supporté (v1) |
| `payload` | `payload` |

```rust
struct QdrantConnector {
    url: String,
    api_key: Option<String>,
    collection: String,
    client: reqwest::Client,
}

impl SourceConnector for QdrantConnector {
    async fn scroll(&self, offset: Option<String>, limit: usize) 
        -> Result<(Vec<Point>, Option<String>)> 
    {
        let body = json!({
            "limit": limit,
            "with_payload": true,
            "with_vector": true,
            "offset": offset.and_then(|o| o.parse::<u64>().ok())
        });
        
        let resp: QdrantScrollResponse = self.client
            .post(format!("{}/collections/{}/points/scroll", self.url, self.collection))
            .json(&body)
            .send().await?
            .json().await?;
        
        let points = resp.result.points.into_iter()
            .map(|p| Point::new(
                p.id.as_u64(),
                p.vector.unwrap_or_default(),
                p.payload,
            ))
            .collect();
        
        Ok((points, resp.result.next_page_offset.map(|o| o.to_string())))
    }
}
```

#### Story Points: 8
#### Priority: P1

---

### US-4.2: Qdrant Collection Discovery

**En tant que** développeur  
**Je veux** lister mes collections Qdrant disponibles  
**Afin de** choisir lesquelles migrer

#### Acceptance Criteria
- [ ] Commande `velesdb-migrate discover qdrant --url <url>`
- [ ] Affichage: nom, dimension, métrique, count, taille
- [ ] Support clusters multi-shard

#### Story Points: 3
#### Priority: P1

---

## 📦 Epic 5: Weaviate Migration

> **Goal**: Migration depuis Weaviate

### US-5.1: Weaviate Connector

**En tant que** utilisateur Weaviate  
**Je veux** migrer mes classes vers VelesDB  
**Afin de** simplifier mon infrastructure

#### Acceptance Criteria
- [ ] Support API GraphQL Weaviate
- [ ] Extraction batch avec cursor
- [ ] Mapping Class → Collection
- [ ] Support multi-tenant
- [ ] Extraction vectors + properties

#### Technical Notes

**GraphQL Query:**
```graphql
{
  Get {
    MyClass(limit: 1000, after: "<cursor>") {
      _additional {
        id
        vector
      }
      title
      content
      # ... autres properties
    }
  }
}
```

```rust
struct WeaviateConnector {
    url: String,
    api_key: Option<String>,
    class_name: String,
    client: reqwest::Client,
}
```

#### Story Points: 8
#### Priority: P1

---

## 📦 Epic 6: Milvus Migration

> **Goal**: Migration depuis Milvus

### US-6.1: Milvus Connector

**En tant que** utilisateur Milvus  
**Je veux** migrer mes collections vers VelesDB  
**Afin de** évaluer VelesDB comme alternative

#### Acceptance Criteria
- [ ] Support API REST Milvus 2.x
- [ ] Support gRPC (optionnel)
- [ ] Pagination via `query` avec offset
- [ ] Support schémas dynamiques
- [ ] Mapping index types

#### Technical Notes

**API Milvus:**
```python
# Pattern d'extraction
results = collection.query(
    expr="id >= 0",
    output_fields=["id", "vector", "metadata"],
    offset=offset,
    limit=1000
)
```

#### Story Points: 8
#### Priority: P2

---

## 📦 Epic 7: ChromaDB Migration

> **Goal**: Migration depuis ChromaDB

### US-7.1: ChromaDB Connector

**En tant que** utilisateur ChromaDB  
**Je veux** migrer mes collections vers VelesDB  
**Afin de** passer à une solution plus performante

#### Acceptance Criteria
- [ ] Support API REST ChromaDB
- [ ] Support fichiers persistants locaux
- [ ] Extraction via `get()` avec pagination
- [ ] Mapping documents + embeddings + metadatas

#### Technical Notes

**API ChromaDB:**
```python
# Extraction
results = collection.get(
    include=["embeddings", "documents", "metadatas"],
    limit=1000,
    offset=offset
)
```

#### Story Points: 5
#### Priority: P2

---

## 📦 Epic 8: Performance & Scalabilité

> **Goal**: Garantir des migrations performantes à grande échelle

### US-8.1: Parallel Streaming

**En tant que** opérateur  
**Je veux** paralléliser l'extraction et l'insertion  
**Afin de** maximiser le throughput de migration

#### Acceptance Criteria
- [ ] Pipeline async avec backpressure
- [ ] Config `parallelism` (default: 4)
- [ ] Métriques par worker
- [ ] Auto-tuning basé sur latences destination

#### Technical Notes
```rust
async fn migrate_parallel(
    source: impl SourceConnector,
    destination: VelesDbClient,
    parallelism: usize,
) -> Result<MigrationStats> {
    let (tx, rx) = async_channel::bounded(parallelism * 2);
    
    // Producer: extract from source
    let producer = tokio::spawn(async move {
        let mut offset = None;
        loop {
            let (points, next) = source.scroll(offset, BATCH_SIZE).await?;
            if points.is_empty() { break; }
            tx.send(points).await?;
            offset = next;
            if next.is_none() { break; }
        }
        Ok::<_, Error>(())
    });
    
    // Consumers: insert to destination
    let consumers: Vec<_> = (0..parallelism)
        .map(|_| {
            let rx = rx.clone();
            let dest = destination.clone();
            tokio::spawn(async move {
                while let Ok(batch) = rx.recv().await {
                    dest.upsert_bulk(&batch).await?;
                }
                Ok::<_, Error>(())
            })
        })
        .collect();
    
    // Wait for completion
    producer.await??;
    for c in consumers { c.await??; }
    
    Ok(stats)
}
```

#### Story Points: 8
#### Priority: P1

---

### US-8.2: Batch Size Auto-Tuning

**En tant que** opérateur  
**Je veux** que le batch size s'adapte automatiquement  
**Afin de** maximiser le throughput sans OOM

#### Acceptance Criteria
- [ ] Détection dimension vecteurs
- [ ] Calcul taille mémoire batch
- [ ] Ajustement dynamique basé sur latence destination
- [ ] Respect limite 100MB API VelesDB

#### Technical Notes
```rust
fn optimal_batch_size(dimension: usize, target_mb: usize) -> usize {
    let vector_bytes = dimension * 4; // f32
    let overhead = 100; // ID + payload estimate
    let point_size = vector_bytes + overhead;
    let target_bytes = target_mb * 1024 * 1024;
    target_bytes / point_size
}
```

#### Story Points: 3
#### Priority: P2

---

### US-8.3: Memory-Efficient Streaming

**En tant que** opérateur  
**Je veux** migrer des milliards de vecteurs sans saturer la RAM  
**Afin de** pouvoir exécuter la migration sur une machine modeste

#### Acceptance Criteria
- [ ] Streaming sans buffer illimité
- [ ] Memory footprint < 500MB pour n'importe quelle taille de dataset
- [ ] Métriques mémoire en temps réel

#### Story Points: 5
#### Priority: P1

---

## 📦 Epic 9: Developer Experience

> **Goal**: Rendre l'expérience de migration aussi simple que possible

### US-9.1: One-Liner Migration

**En tant que** développeur  
**Je veux** migrer en une seule commande  
**Afin de** ne pas perdre de temps en configuration

#### Acceptance Criteria
- [ ] `velesdb-migrate from pinecone --api-key $KEY --index my-index`
- [ ] Création automatique collection destination
- [ ] Détection automatique dimension et métrique
- [ ] Output clair avec résumé final

#### Story Points: 5
#### Priority: P0

---

### US-9.2: Dry-Run Mode

**En tant que** développeur  
**Je veux** simuler une migration sans écrire de données  
**Afin de** valider ma configuration avant exécution

#### Acceptance Criteria
- [ ] Option `--dry-run`
- [ ] Affichage: source count, schema mapping, estimated time
- [ ] Validation credentials source et destination
- [ ] Pas d'écriture vers destination

#### Story Points: 3
#### Priority: P1

---

### US-9.3: Interactive Mode

**En tant que** développeur occasionnel  
**Je veux** un mode interactif guidé  
**Afin de** ne pas avoir à lire toute la documentation

#### Acceptance Criteria
- [ ] `velesdb-migrate interactive`
- [ ] Wizard: sélection source → config → preview → execute
- [ ] Génération fichier config pour réutilisation
- [ ] Aide contextuelle à chaque étape

#### Story Points: 5
#### Priority: P2

---

### US-9.4: Error Messages & Recovery Hints

**En tant que** développeur  
**Je veux** des messages d'erreur clairs avec suggestions  
**Afin de** résoudre rapidement les problèmes

#### Acceptance Criteria
- [ ] Messages d'erreur avec contexte (source, batch, ID)
- [ ] Suggestions de résolution pour erreurs communes
- [ ] Liens vers documentation
- [ ] Error codes documentés

#### Story Points: 3
#### Priority: P1

---

## 📦 Epic 10: Documentation & Exemples

> **Goal**: Documentation complète pour adoption rapide

### US-10.1: Guide Migration par Source

**En tant que** développeur  
**Je veux** un guide étape par étape pour ma base source  
**Afin de** migrer rapidement sans erreur

#### Acceptance Criteria
- [ ] Guide Pinecone (Starter + Enterprise)
- [ ] Guide Supabase/pgvector
- [ ] Guide Qdrant (Cloud + Self-hosted)
- [ ] Guide Weaviate
- [ ] Guide Milvus
- [ ] Guide ChromaDB

#### Story Points: 8
#### Priority: P0

---

### US-10.2: Exemples de Scripts

**En tant que** développeur  
**Je veux** des scripts d'exemple prêts à l'emploi  
**Afin de** adapter rapidement à mon cas d'usage

#### Acceptance Criteria
- [ ] Script Python pour chaque source
- [ ] Script Bash pour CI/CD
- [ ] Docker Compose pour migration locale
- [ ] GitHub Actions workflow

#### Story Points: 5
#### Priority: P1

---

### US-10.3: Troubleshooting Guide

**En tant que** opérateur  
**Je veux** un guide de dépannage  
**Afin de** résoudre les problèmes courants

#### Acceptance Criteria
- [ ] Section "Common Issues"
- [ ] Checklist pré-migration
- [ ] FAQ
- [ ] Contact support

#### Story Points: 3
#### Priority: P1

---

## 🧪 QA Strategy

### Test Categories

| Category | Coverage | Automation |
|----------|----------|------------|
| Unit Tests | 80%+ | CI obligatoire |
| Integration Tests | Sources mockées | CI obligatoire |
| E2E Tests | Sources réelles | Nightly |
| Performance Tests | Benchmarks | Weekly |
| Chaos Tests | Interruptions réseau | Monthly |

### Test Scenarios par Source

#### Pinecone Tests
- [ ] Migration index vide
- [ ] Migration avec namespaces multiples
- [ ] Rate limiting (429) recovery
- [ ] IDs string avec caractères spéciaux
- [ ] Metadata nested objects
- [ ] Migration 1M+ vectors (perf)

#### pgvector Tests
- [ ] Types vector, halfvec, sparsevec
- [ ] Colonnes JSONB metadata
- [ ] Tables sans PK (warning)
- [ ] Supabase connection pooler
- [ ] Migration 10M+ rows (perf)

#### Qdrant Tests
- [ ] Collections avec UUID IDs
- [ ] Multivecteurs → multi-collections
- [ ] Sharded clusters
- [ ] Filtres de migration
- [ ] Migration 1M+ points (perf)

### Performance Benchmarks

| Source | Target Throughput | Max Latency P99 |
|--------|-------------------|-----------------|
| Pinecone | 5K vec/s | 500ms/batch |
| pgvector | 10K vec/s | 200ms/batch |
| Qdrant | 8K vec/s | 300ms/batch |
| Weaviate | 5K vec/s | 500ms/batch |

### QA Acceptance Matrix

| Critère | P0 (Bloquant) | P1 (Important) | P2 (Nice-to-have) |
|---------|---------------|----------------|-------------------|
| Zero data loss | ✅ | ✅ | ✅ |
| Resume after interrupt | ✅ | ✅ | |
| Progress reporting | ✅ | | |
| Validation post-migration | ✅ | | |
| Dry-run mode | | ✅ | |
| Memory < 500MB | | ✅ | |
| Throughput target | | ✅ | |

---

## 📅 Release Planning

### Phase 1: MVP (v0.1) - 4 semaines
- [ ] US-1.1: CLI de base
- [ ] US-1.2: Checkpoint system
- [ ] US-1.5: Config YAML
- [ ] US-2.1: pgvector connector
- [ ] US-3.1: Pinecone connector
- [ ] US-9.1: One-liner migration
- [ ] US-10.1: Guides Pinecone + pgvector

### Phase 2: Production Ready (v0.2) - 3 semaines
- [ ] US-1.3: Validation post-migration
- [ ] US-1.4: Metrics & Monitoring
- [ ] US-4.1: Qdrant connector
- [ ] US-8.1: Parallel streaming
- [ ] US-9.2: Dry-run mode
- [ ] US-10.2: Scripts exemples

### Phase 3: Enterprise (v0.3) - 3 semaines
- [ ] US-5.1: Weaviate connector
- [ ] US-6.1: Milvus connector
- [ ] US-7.1: ChromaDB connector
- [ ] US-8.3: Memory-efficient streaming
- [ ] US-9.3: Interactive mode

---

## 📊 Métriques de Succès

| KPI | Target | Mesure |
|-----|--------|--------|
| Time to First Migration | < 5 min | From install to data migrated |
| Migration Success Rate | > 99% | Completed / Started |
| Data Integrity | 100% | Validation pass rate |
| Throughput | > 10K vec/s | Vecteurs / seconde |
| Support Tickets | < 5/week | Post-launch |

---

## 🔗 Références

- [VelesDB API Documentation](../API.md)
- [Pinecone API Reference](https://docs.pinecone.io/reference)
- [Qdrant API Reference](https://qdrant.tech/documentation/interfaces/)
- [pgvector Documentation](https://github.com/pgvector/pgvector)
- [Weaviate GraphQL API](https://weaviate.io/developers/weaviate/api/graphql)
- [Milvus API Reference](https://milvus.io/api-reference)
- [ChromaDB API](https://docs.trychroma.com/reference)

---

*Document généré le 2024-12-30 par VelesDB Product Team*
