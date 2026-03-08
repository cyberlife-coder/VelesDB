# VelesDB — Open-Core v1.5

## What This Is

VelesDB est un moteur de base de données local-first écrit en Rust, fusionnant Vector + Graph + ColumnStore dans un seul binaire de 15 Mo. Il cible les workloads d'agents IA (RAG, semantic search, knowledge graphs) avec une latence sub-milliseconde via HNSW natif + SIMD AVX-512/AVX2/NEON. v1.5 ajoute Product Quantization (SQ8 4x / Binary 32x), sparse vectors (SPLADE/BM25 + hybrid RRF), streaming inserts, query plan caching, et des intégrations officielles LangChain/LlamaIndex. Le modèle open-core expose les fonctionnalités cœur sous licence ELv2 ; les fonctionnalités premium (HA, multi-tenancy, RBAC, audit) s'ajoutent via un repo séparé héritant du trait `DatabaseObserver`.

## Core Value

Un seul moteur de connaissance pour les agents IA — Vector + Graph + ColumnStore, sub-milliseconde, offline, 15 Mo — sans glue code ni dépendances cloud.

## Requirements

### Validated

— Tout ce qui suit a été livré et est opérationnel :

- ✓ HNSW vector search avec SIMD (AVX-512/AVX2/NEON), dispatch runtime — v1.4
- ✓ Knowledge graph (MATCH, BFS/DFS, parallel traversal) — v1.4
- ✓ ColumnStore CRUD avec JOIN cross-store — v1.4
- ✓ VelesQL v2.0 (GROUP BY, HAVING, ORDER BY, UNION, INTERSECT, EXCEPT, JOIN, FUSION RRF) — v1.4
- ✓ REST API server 22+ endpoints (Axum), EXPLAIN, SSE — v1.4
- ✓ Python bindings PyO3 + NumPy — v1.4
- ✓ WASM module (browser-side vector search) — v1.4
- ✓ Mobile bindings iOS/Android (UniFFI) — v1.4
- ✓ TypeScript SDK (Node.js + Browser) — v1.4
- ✓ LangChain VectorStore + LlamaIndex integrations — v1.4
- ✓ Tauri v2 plugin — v1.4
- ✓ Migration tooling (Qdrant, Pinecone, Supabase) — v1.4
- ✓ SQ8 (4x) + Binary (32x) quantization — v1.4
- ✓ RRF score fusion (multi-score) — v1.4
- ✓ Agent Memory SDK (semantic, episodic, procedural) — v1.4
- ✓ CI/CD pipeline complet (clippy, fmt, tests 3300+, coverage 82%) — v1.4
- ✓ DatabaseObserver trait pour extensions premium — v1.4
- ✓ Product Quantization engine (SQ8 4x + Binary 32x, VelesQL TRAIN/SEARCH, multi-distribution recall) — v1.5
- ✓ Sparse Vectors (SPLADE/BM25, inverted index, hybrid RRF fusion) — v1.5
- ✓ Streaming Inserts (batched WAL, auto-reindex, REST/SDK support) — v1.5
- ✓ Query Plan Cache (LRU, compiled plan reuse, write-generation invalidation, EXPLAIN wiring) — v1.5
- ✓ SDK Parity — Python/WASM/TypeScript/Mobile/Tauri updated for v1.5 features — v1.5
- ✓ Official LangChain & LlamaIndex packages with v1.5 parity — v1.5
- ✓ Full documentation audit (README, rustdoc, OpenAPI, CHANGELOG, migration guide) — v1.5
- ✓ Release readiness (crates.io, PyPI, npm, GitHub release) — v1.5
- ✓ Criterion baseline registered with 15% regression threshold — v1.5

### Active

(No active requirements — next milestone not yet defined)

### Out of Scope candidates for next milestone review

### Out of Scope

- Distributed Mode (EPIC-061) — réservé Premium (multi-node, clustering HA)
- RBAC, audit log, multi-tenancy — réservés Premium via `DatabaseObserver`
- Administration UI — réservée Premium
- Cloud-hosted / SaaS mode — non-goal open-core
- Sparse vector GPU acceleration — complexité trop élevée pour v1.5

## Context

VelesDB v1.5.0 shipped 2026-03-08. Le workspace Rust contient 8 crates de production (~228K LoC). v1.5 a ajouté Product Quantization, sparse vectors, streaming inserts, query plan caching, et des intégrations officielles LangChain/LlamaIndex en 16 phases et 42 plans sur 82 jours. La vision open-core / premium est établie : le repo premium hérite du cœur via le trait `DatabaseObserver` injecté dans `Database::open_with_observer()`.

Benchmarks baseline registered dans `benchmarks/baseline.json` avec seuil 15% enforced en CI. Coverage tests maintenu. README audité et mis à jour avec métriques réelles v1.5.

## Constraints

- **License** : ELv2 — open-core, pas de managed service
- **Architecture** : Rust workspace multi-crates, pas de dépendances cycliques entre crates
- **Premium boundary** : Toute feature premium passe UNIQUEMENT par `DatabaseObserver` / repo séparé
- **WASM** : Feature `persistence` désactivée pour wasm32-unknown-unknown
- **Tests** : `--test-threads=1` obligatoire (isolation filesystem)
- **TODO governance** : Format `// TODO(EPIC-XXX):` ou `// TODO(US-XXX):` — bare TODO/FIXME rejetés par CI
- **Unsafe** : Tout bloc `unsafe` doit avoir un commentaire `// SAFETY:`
- **CI** : Valider localement avec `scripts/local-ci.ps1` avant push

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Observer pattern pour le premium | Évite le couplage entre open-core et premium, héritée proprement | ✓ Good |
| parking_lot::RwLock partout | Performances > std::sync::RwLock, ordre de lock déterministe | ✓ Good |
| pest-based VelesQL parser | Grammaire déclarative, conformance cross-crate via JSON | ✓ Good |
| SIMD dispatch runtime | Détection AVX-512/AVX2/NEON à l'exécution, 0 recompilation | ✓ Good |
| ELv2 licence | Permet usage libre sans managed service, protège le modèle commercial | — Pending |
| rust-elite-architect comme agent primaire | Toute modification Rust passe par cet agent spécialisé | ✓ Good |
| PQ with unified SQ8+Binary pipeline | Single quantization module, VelesQL TRAIN integration | ✓ Good |
| Sparse via inverted index + RRF fusion | Hybrid search without separate sparse engine | ✓ Good |
| Query plan cache with write-generation invalidation | Zero stale plans, automatic invalidation on writes | ✓ Good |
| Official LangChain/LlamaIndex packages | pip-installable, v1.5 feature parity | ✓ Good |

---
*Last updated: 2026-03-08 after v1.5 milestone*
