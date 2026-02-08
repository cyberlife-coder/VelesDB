# VelesDB Core - Dependency Map

## Architecture des modules

```
velesdb-core/
├── src/
│   ├── lib.rs                 # Point d'entrée, réexporte tout
│   ├── collection/            # Collections vectorielles
│   │   ├── mod.rs
│   │   ├── core/              # Logique centrale
│   │   ├── search/            # Recherche et queries
│   │   └── config.rs
│   ├── index/                 # Indexation HNSW
│   │   ├── hnsw/
│   │   └── property/
│   ├── storage/               # Persistance
│   │   ├── mmap.rs
│   │   └── vector_bytes.rs
│   ├── velesql/               # Parser SQL
│   │   └── parser/
│   ├── graph/                 # Knowledge Graph
│   └── simd/                  # Optimisations SIMD
```

## Matrice de dépendances internes

| Module source | Dépend de | Utilisé par |
|---------------|-----------|-------------|
| `collection::core` | index, storage, config | search, lib.rs |
| `index::hnsw` | simd, storage | collection::core |
| `storage::mmap` | - | index, collection |
| `velesql::parser` | - | collection::search |
| `graph` | storage | collection (optionnel) |
| `simd` | - | index::hnsw, distance |

## Dépendances externes critiques

| Crate externe | Utilisé dans | Impact si upgrade |
|---------------|--------------|-------------------|
| `parking_lot` | Tous les RwLock | Élevé - Synchronisation |
| `serde` | Serialization | Moyen - Format données |
| `rayon` | Parallélisation | Faible - Optionnel |
| `wgpu` | GPU backend | Faible - Feature flag |

## Bindings et SDKs

### Python (PyO3)
| Fonction Rust | Binding Python | Fichier |
|---------------|----------------|---------|
| `Collection::new` | `VelesCollection.__init__` | `python/collection.rs` |
| `Collection::insert` | `VelesCollection.insert` | `python/collection.rs` |
| `Collection::search` | `VelesCollection.search` | `python/collection.rs` |

### WASM
| Fonction Rust | Export WASM | Fichier |
|---------------|-------------|---------|
| `WasmVectorStore::new` | `new()` | `velesdb-wasm/src/lib.rs` |
| `WasmVectorStore::search` | `search()` | `velesdb-wasm/src/lib.rs` |

### TypeScript SDK
| Endpoint REST | Méthode TS | Fichier |
|---------------|------------|---------|
| `POST /collections` | `createCollection()` | `sdks/typescript/src/client.ts` |
| `POST /vectors` | `insert()` | `sdks/typescript/src/client.ts` |
| `POST /search` | `search()` | `sdks/typescript/src/client.ts` |

## Fichiers à haut risque

| Fichier | Raison | Précautions |
|---------|--------|-------------|
| `src/lib.rs` | Point d'entrée public | Ne jamais modifier sans impact analysis |
| `collection/core/mod.rs` | Cœur de la logique | Tests exhaustifs requis |
| `index/hnsw/native/graph.rs` | Performance critique | Benchmarks obligatoires |
| `storage/mmap.rs` | Données persistantes | Compatibilité ascendante |

## Commandes de vérification

```powershell
# Vérifier que tous les exports sont utilisés
cargo +nightly udeps

# Graphe de dépendances
cargo tree --prefix none

# Symboles exportés
cargo doc --no-deps --document-private-items
```
