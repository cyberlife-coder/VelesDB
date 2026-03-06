# Requirements: VelesDB v1.5 Open-Core

**Defined:** 2026-03-05
**Core Value:** Un seul moteur de connaissance pour les agents IA — Vector + Graph + ColumnStore, sub-milliseconde, offline, 15 Mo — sans glue code ni dependances cloud.

## v1 Requirements

Requirements pour la release v1.5. Chaque requirement mappe a une phase du roadmap.

### Quality & Security (QUAL)

- [x] **QUAL-01**: bincode RUSTSEC-2025-0141 migre vers une alternative sound (bitcode ou postcard) — wire-format compatible si possible
- [ ] **QUAL-02**: BUG-8 corrige — multi-alias FROM dans VelesQL produit des resultats incorrects
- [x] **QUAL-03**: `ProductQuantizer::train()` — `assert!` / `panic!` remplaces par `Result<_, VelesError>` — input invalide ne tue plus le serveur
- [x] **QUAL-04**: k-means++ init implemente pour PQ — remplace l'init sequentielle deterministe qui produit des codebooks degeneres sur donnees reelles
- [x] **QUAL-05**: `cargo audit || true` retire du CI — advisory reel = CI rouge
- [ ] **QUAL-06**: Criterion baseline v1.5 enregistre dans `benchmarks/baseline.json` — seuil 15% enforced sur toutes les 35+ suites
- [ ] **QUAL-07**: Coverage code >= 82% maintenue apres toutes les additions v1.5

### Product Quantization — EPIC-063 (PQ)

- [x] **PQ-01**: Codebook PQ entrainable avec k-means++ (m sous-espaces, k centroids configurables)
- [x] **PQ-02**: ADC (Asymmetric Distance Computation) avec SIMD — lookup table tient en cache L1 (m x k x 4 bytes, ~8KB pour m=8 k=256)
- [x] **PQ-03**: OPQ pre-rotation optionnelle via `ndarray` — ameliore recall ~5-15% sur donnees groupees
- [x] **PQ-04**: Phase de rescore — oversampling + rerank f32 active par defaut (evite le silent recall collapse)
- [x] **PQ-05**: Commande VelesQL `TRAIN QUANTIZER ON <collection> WITH (m=8, k=256)` — entrainement explicite, pas automatique
- [x] **PQ-06**: `QuantizationConfig` etendu avec variante PQ — retrocompatible avec SQ8/Binary existants, pas de breaking change
- [x] **PQ-07**: Suite Criterion dediee `pq_recall` — seuils de precision (recall@10 >= 92% pour m=8) enregistres dans baseline

### Sparse Vectors — EPIC-062 (SPARSE)

- [x] **SPARSE-01**: `WeightedPostingList` — inverted index avec poids f32 par terme, SPLADE-v2/BM42-compatible (format `{term_id: u32 -> weight: f32}`)
- [x] **SPARSE-02**: Index sparse persiste sur disque — nouveau fichier `sparse.idx` dans le repertoire collection, survit aux redemarrages
- [x] **SPARSE-03**: Recherche sparse ANN — inner product sur posting lists, support WAND ou scan lineaire selon densite
- [ ] **SPARSE-04**: Hybrid dense+sparse via RRF existant — `fusion/rrf_merge()` non modifie, fonctionne out-of-the-box
- [ ] **SPARSE-05**: Extension grammaire VelesQL — keyword `SPARSE` et clause `vector SPARSE_NEAR $sv` dans `velesql.pest`
- [ ] **SPARSE-06**: API REST — endpoints upsert avec champ `sparse_vector` + search sparse endpoint
- [ ] **SPARSE-07**: term_id u32 (4 milliards de termes — couvre tous les vocabulaires LLM modernes)

### Query Plan Cache — EPIC-065 (CACHE)

- [ ] **CACHE-01**: `CompiledPlanCache` deux niveaux — AST cache inchange + nouveau tier plan compile avec invalidation collection
- [ ] **CACHE-02**: `write_generation: AtomicU64` par collection — tout write incremente, invalide tous les plans cached pour cette collection
- [ ] **CACHE-03**: Invalidation lifecycle — drop ou recreate d'une collection invalide immediatement tous les plans associes
- [ ] **CACHE-04**: Metriques cache exposees — hit rate, miss rate, evictions accessibles via `/metrics` (Prometheus)

### Streaming Inserts — EPIC-064 (STREAM)

- [ ] **STREAM-01**: `StreamIngester` — bounded `tokio::sync::mpsc` channel, backpressure HTTP 429 quand buffer plein
- [ ] **STREAM-02**: Micro-batches draines dans HNSW — taille configurable (defaut 128 vecteurs), calibree contre le cout d'acquisition write lock HNSW
- [ ] **STREAM-03**: Delta buffer pour inserts pendant HNSW rebuild — vecteurs recus mid-rebuild incorpores sans perte
- [ ] **STREAM-04**: Insert-and-immediately-searchable garanti — recherche inclut le buffer en attente de drain
- [ ] **STREAM-05**: Exclusion WASM automatique via `#[cfg(feature = "persistence")]` — pas de tokio runtime en WASM

### SDK Parity (SDK)

- [ ] **SDK-01**: Python SDK — sparse upsert/search, PQ train/config, streaming insert propages dans velesdb-python
- [ ] **SDK-02**: TypeScript SDK — sparse vectors, PQ config, streaming insert dans `sdks/typescript`
- [ ] **SDK-03**: WASM module — sparse search sans persistence, plan cache actif (features compatibles no-persistence)
- [ ] **SDK-04**: Mobile iOS/Android — bindings UniFFI mis a jour pour API v1.5 (sparse + PQ)
- [ ] **SDK-05**: LangChain VectorStore — hybrid dense+sparse supporte nativement via le VectorStore officiel
- [ ] **SDK-06**: LlamaIndex integration — sparse + PQ config exposes dans le VectorStore
- [ ] **SDK-07**: Tauri plugin — synchronise avec API core v1.5

### Documentation (DOC)

- [ ] **DOC-01**: README v1.5 — metriques recalculees (PQ recall, sparse latency), features v1.5, exemples mis a jour
- [ ] **DOC-02**: rustdoc complet API publique `velesdb-core` — tous les types/fonctions publics ont doc comment
- [ ] **DOC-03**: OpenAPI spec v1.5 — nouveaux endpoints sparse + streaming documentes, generee depuis annotations
- [ ] **DOC-04**: Guide migration v1.4 -> v1.5 — breaking changes `QuantizationConfig`, VelesQL `SPARSE`, bincode wire-format
- [ ] **DOC-05**: `BENCHMARKS.md` v1.5 — resultats reels PQ recall@k, sparse search latency, streaming throughput
- [ ] **DOC-06**: `CHANGELOG.md` v1.5 — complet avec toutes les features, fixes, breaking changes

### Release (REL)

- [ ] **REL-01**: Tous crates versionnes `1.5.0`, publies sur crates.io dans l'ordre des dependances (velesdb-core en premier)
- [ ] **REL-02**: PyPI — wheels cross-platform via maturin CI matrix (linux-x86_64, linux-aarch64, macos-arm64, windows-x86_64)
- [ ] **REL-03**: npm — `@wiscale/velesdb` et `@wiscale/velesdb-wasm` publies avec version 1.5.0
- [ ] **REL-04**: GitHub Release — notes de release structurees + artefacts binaires (Linux, macOS ARM, macOS Intel, Windows)
- [ ] **REL-05**: CI release matrix — validation cross-platform automatique avant toute publication

## v2 Requirements

Deferred a une version ulterieure (v1.6+ ou premium). Some requirements promoted to v1.5 during Phase 2 planning.

### Engine

- **DIST-01**: Distributed Mode / multi-node clustering — reserve Premium
- **SPARSE-ADV-01**: WAND exact top-k traversal pour vocabulaires > 100K termes — v1.6
- **PQ-ADV-01**: RaBitQ (arXiv:2405.12497) — **PROMOTED to v1.5 Phase 2** (implemented as third quantization strategy alongside SQ8 and PQ)
- **QUANT-ADV-01**: Product Quantization GPU acceleration — **PROMOTED to v1.5 Phase 2** (GPU k-means assignment step for training acceleration)

### Premium boundary

- **PREM-01**: RBAC multi-tenant — via `DatabaseObserver`, repo premium
- **PREM-02**: Audit log — via `DatabaseObserver`, repo premium
- **PREM-03**: Admin UI — repo premium
- **PREM-04**: HA / failover — repo premium

## Out of Scope

| Feature | Reason |
|---------|--------|
| Distributed Mode open-core | Reserve Premium — architecture separee via observer |
| Cloud-hosted / SaaS | Non-goal open-core — local-first par design |
| Sparse GPU acceleration | Complexite trop elevee, yield faible pour DB locale |
| Real-time collaborative editing | Hors du domaine vector DB |
| Full SQL compliance (DDL, transactions) | VelesQL est intentionnellement SQL-like, pas SQL-complet |
| RBAC / multi-tenancy dans open-core | Observer pattern uniquement — pas de leak de logique premium |

## Traceability

Mapping requirements -> phases. Updated 2026-03-06 after Phase 2 plan revision (promoted PQ-ADV-01, QUANT-ADV-01).

| Requirement | Phase | Status |
|-------------|-------|--------|
| QUAL-01 | Phase 1: Quality Baseline & Security | Complete |
| QUAL-02 | Phase 1: Quality Baseline & Security | Pending |
| QUAL-03 | Phase 1: Quality Baseline & Security | Complete |
| QUAL-04 | Phase 1: Quality Baseline & Security | Complete |
| QUAL-05 | Phase 1: Quality Baseline & Security | Complete |
| QUAL-06 | Phase 1: Quality Baseline & Security | Pending |
| QUAL-07 | Phase 1: Quality Baseline & Security | Pending |
| PQ-01 | Phase 2: PQ Core Engine | Complete |
| PQ-02 | Phase 2: PQ Core Engine | Complete |
| PQ-03 | Phase 2: PQ Core Engine | Complete |
| PQ-04 | Phase 2: PQ Core Engine | Complete |
| PQ-ADV-01 | Phase 2: PQ Core Engine | Pending (promoted from v2) |
| QUANT-ADV-01 | Phase 2: PQ Core Engine | Pending (promoted from v2) |
| PQ-05 | Phase 3: PQ Integration | Complete |
| PQ-06 | Phase 3: PQ Integration | Complete |
| PQ-07 | Phase 3: PQ Integration | Complete |
| SPARSE-01 | Phase 4: Sparse Vector Engine | Complete |
| SPARSE-02 | Phase 4: Sparse Vector Engine | Complete |
| SPARSE-03 | Phase 4: Sparse Vector Engine | Complete |
| SPARSE-04 | Phase 5: Sparse Integration | Pending |
| SPARSE-05 | Phase 5: Sparse Integration | Pending |
| SPARSE-06 | Phase 5: Sparse Integration | Pending |
| SPARSE-07 | Phase 5: Sparse Integration | Pending |
| CACHE-01 | Phase 6: Query Plan Cache | Pending |
| CACHE-02 | Phase 6: Query Plan Cache | Pending |
| CACHE-03 | Phase 6: Query Plan Cache | Pending |
| CACHE-04 | Phase 6: Query Plan Cache | Pending |
| STREAM-01 | Phase 7: Streaming Inserts | Pending |
| STREAM-02 | Phase 7: Streaming Inserts | Pending |
| STREAM-03 | Phase 7: Streaming Inserts | Pending |
| STREAM-04 | Phase 7: Streaming Inserts | Pending |
| STREAM-05 | Phase 7: Streaming Inserts | Pending |
| SDK-01 | Phase 8: SDK Parity | Pending |
| SDK-02 | Phase 8: SDK Parity | Pending |
| SDK-03 | Phase 8: SDK Parity | Pending |
| SDK-04 | Phase 8: SDK Parity | Pending |
| SDK-05 | Phase 8: SDK Parity | Pending |
| SDK-06 | Phase 8: SDK Parity | Pending |
| SDK-07 | Phase 8: SDK Parity | Pending |
| DOC-01 | Phase 9: Documentation | Pending |
| DOC-02 | Phase 9: Documentation | Pending |
| DOC-03 | Phase 9: Documentation | Pending |
| DOC-04 | Phase 9: Documentation | Pending |
| DOC-05 | Phase 9: Documentation | Pending |
| DOC-06 | Phase 9: Documentation | Pending |
| REL-01 | Phase 10: Release Readiness | Pending |
| REL-02 | Phase 10: Release Readiness | Pending |
| REL-03 | Phase 10: Release Readiness | Pending |
| REL-04 | Phase 10: Release Readiness | Pending |
| REL-05 | Phase 10: Release Readiness | Pending |

**Coverage:**
- v1 requirements: 48 total
- Promoted from v2: 2 (PQ-ADV-01, QUANT-ADV-01)
- Mapped to phases: 50
- Unmapped: 0

---
*Requirements defined: 2026-03-05*
*Last updated: 2026-03-06 after Phase 2 plan revision*
