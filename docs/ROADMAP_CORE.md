# 🗺️ VelesDB Core - Roadmap de Développement

*Version 1.0 - Décembre 2025*

---

## 🎯 Vision

> **"The fastest embedded vector database for AI applications"**
> Edge-first • Single Binary • Microsecond Latency

---

## 📋 EPICs et User Stories

### 🏔️ EPIC 1: SQ8 Scalar Quantization
**Objectif**: Réduire l'empreinte mémoire de 4x avec perte de recall minimale (<2%)

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **1.1** | **Types et structures de base** | `QuantizedVector`, `QuantizationParams`, sérialization | 2h |
| **1.2** | **Encode/Decode SIMD** | AVX2/SSE vectorisé, throughput >100M/s | 4h |
| **1.3** | **Distance SQ8** | Cosine/Euclidean/Dot sur vecteurs quantifiés | 3h |
| **1.4** | **Intégration HNSW** | `HnswIndex<QuantizedVector>` avec même API | 4h |
| **1.5** | **Benchmarks et non-régression** | Criterion, comparaison f32 vs SQ8, recall@10 | 2h |
| **1.6** | **Documentation et exemples** | Docs Rust, README, exemple Python | 1h |

**Critères de non-régression**:
- [ ] Recall@10 ≥ 97% (vs 99.4% en f32)
- [ ] Latence search ≤ 1.5x f32
- [ ] Tous les tests existants passent
- [ ] Benchmarks SIMD ne régressent pas

**Branche**: `feature/sq8-quantization`

---

### 🏔️ EPIC 2: ARM NEON SIMD
**Objectif**: Support natif Apple Silicon (M1/M2/M3), Raspberry Pi, Jetson

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **2.1** | **Détection runtime ARM** | `#[cfg(target_arch = "aarch64")]`, feature flags | 1h |
| **2.2** | **Kernels NEON distance** | Cosine, Euclidean, Dot avec intrinsics NEON | 4h |
| **2.3** | **Kernels NEON Hamming/Jaccard** | Binary ops optimisées ARM | 2h |
| **2.4** | **Tests cross-platform** | CI GitHub Actions avec qemu-aarch64 | 2h |
| **2.5** | **Benchmarks ARM** | Criterion sur Apple M1 (si dispo) ou RPi | 2h |
| **2.6** | **Documentation ARM** | Guide installation ARM, perf attendues | 1h |

**Critères de non-régression**:
- [ ] Tous tests x86_64 passent toujours
- [ ] Performance ARM ≥ 80% de x86_64 AVX2
- [ ] Compilation WASM non impactée

**Branche**: `feature/arm-neon-simd`

---

### 🏔️ EPIC 3: Binary Quantization (1-bit)
**Objectif**: Compression 32x pour fingerprints, hashes, dedup

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **3.1** | **BinaryVector type** | `Vec<u64>` packed bits, conversion depuis f32 | 2h |
| **3.2** | **Hamming sur BinaryVector** | POPCNT optimisé, >1B ops/s | 2h |
| **3.3** | **Seuillage adaptatif** | Médiane, moyenne, percentile configurable | 2h |
| **3.4** | **Intégration collection** | `metric: "binary"` dans API | 2h |
| **3.5** | **Tests et benchmarks** | Recall, latence, comparaison Hamming f32 | 2h |

**Critères de non-régression**:
- [ ] Hamming f32 existant non impacté
- [ ] API REST compatible

**Branche**: `feature/binary-quantization`

---

### 🏔️ EPIC 4: LlamaIndex Integration
**Objectif**: Support du 2ème framework RAG Python

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **4.1** | **VelesDBVectorStore class** | Implémente `VectorStore` protocol | 3h |
| **4.2** | **Retriever integration** | `VelesDBRetriever` avec filters | 2h |
| **4.3** | **Tests unitaires Python** | pytest, mock embeddings | 2h |
| **4.4** | **Exemple RAG complet** | Notebook Jupyter end-to-end | 2h |
| **4.5** | **Publication PyPI** | Package `llama-index-velesdb` | 1h |

**Critères de non-régression**:
- [ ] LangChain integration non impactée
- [ ] SDK Python core non modifié

**Branche**: `feature/llamaindex-integration`

---

### 🏔️ EPIC 5: Auto-Chunking Utilities
**Objectif**: Text splitters intégrés pour simplifier les pipelines RAG

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **5.1** | **SentenceSplitter** | Split par phrases, configurable | 2h |
| **5.2** | **TokenSplitter** | Split par tokens (tiktoken compatible) | 3h |
| **5.3** | **RecursiveSplitter** | Chunk avec overlap, multi-séparateurs | 3h |
| **5.4** | **API Rust publique** | Module `velesdb_core::chunking` | 2h |
| **5.5** | **Bindings Python** | Exposé via PyO3 | 2h |
| **5.6** | **Tests et documentation** | Edge cases, Unicode, exemples | 2h |

**Critères de non-régression**:
- [ ] Pas de nouvelle dépendance lourde
- [ ] Performance chunking >10MB/s

**Branche**: `feature/auto-chunking`

---

### 🏔️ EPIC 6: Sparse Vectors (SPLADE/BM42)
**Objectif**: Support vecteurs sparse pour hybrid search moderne

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **6.1** | **SparseVector type** | `HashMap<u32, f32>` ou CSR format | 3h |
| **6.2** | **Sparse dot product** | Intersection efficace, SIMD si dense | 3h |
| **6.3** | **Index sparse** | Inverted index pour top-k | 4h |
| **6.4** | **Hybrid dense+sparse** | Fusion RRF, weights configurables | 3h |
| **6.5** | **API REST sparse** | Endpoints search avec sparse vectors | 2h |
| **6.6** | **Tests et benchmarks** | BEIR dataset subset | 3h |

**Critères de non-régression**:
- [ ] Dense search non impacté
- [ ] BM25 existant compatible

**Branche**: `feature/sparse-vectors`

---

### 🏔️ EPIC 7: Reranking API
**Objectif**: Intégration cross-encoders pour meilleur recall

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **7.1** | **Reranker trait** | Interface générique pour rerankers | 2h |
| **7.2** | **HTTP Reranker** | Appel API externe (Cohere, Jina) | 2h |
| **7.3** | **Pipeline search+rerank** | Two-stage retrieval configurable | 3h |
| **7.4** | **Tests avec mock** | Reranker simulé, ordering correct | 2h |
| **7.5** | **Documentation** | Guide intégration, exemples | 1h |

**Critères de non-régression**:
- [ ] Search sans rerank identique
- [ ] Latence acceptable (<100ms avec rerank)

**Branche**: `feature/reranking-api`

---

### 🏔️ EPIC 8: VS Code Extension
**Objectif**: Developer Experience pour exploration et debug

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **8.1** | **Extension scaffold** | TypeScript, vscode API | 2h |
| **8.2** | **Collection explorer** | TreeView des collections | 3h |
| **8.3** | **VelesQL editor** | Syntax highlighting, autocomplete | 4h |
| **8.4** | **Query runner** | Exécution requêtes, résultats | 3h |
| **8.5** | **Publication Marketplace** | Package et CI | 2h |

**Critères de non-régression**:
- [ ] N/A (nouveau composant)

**Branche**: `feature/vscode-extension`

---

### 🏔️ EPIC 9: Electron Bindings (N-API)
**Objectif**: Support Electron pour apps desktop

| US | Description | Critères d'acceptation | Effort |
|:---:|:---|:---|:---:|
| **9.1** | **napi-rs setup** | Crate `velesdb-napi` | 2h |
| **9.2** | **Core bindings** | Collection, search, upsert | 4h |
| **9.3** | **TypeScript types** | Génération .d.ts automatique | 2h |
| **9.4** | **npm package** | `@velesdb/electron` | 2h |
| **9.5** | **Exemple Electron app** | Demo fonctionnelle | 3h |

**Critères de non-régression**:
- [ ] Tauri plugin non impacté
- [ ] WASM non impacté

**Branche**: `feature/electron-bindings`

---

## 📅 Planning par Trimestre

### Q1 2025 - Performance & Edge
| Semaine | EPIC | US |
|:---:|:---|:---|
| S1-S2 | EPIC 1: SQ8 | 1.1 → 1.6 |
| S3-S4 | EPIC 2: ARM NEON | 2.1 → 2.6 |
| S5-S6 | EPIC 3: Binary Quant | 3.1 → 3.5 |

### Q2 2025 - Developer Experience
| Semaine | EPIC | US |
|:---:|:---|:---|
| S1-S2 | EPIC 4: LlamaIndex | 4.1 → 4.5 |
| S3-S4 | EPIC 5: Auto-chunking | 5.1 → 5.6 |
| S5-S6 | Buffer / stabilisation | - |

### Q3 2025 - Recherche Avancée
| Semaine | EPIC | US |
|:---:|:---|:---|
| S1-S3 | EPIC 6: Sparse Vectors | 6.1 → 6.6 |
| S4-S5 | EPIC 7: Reranking | 7.1 → 7.5 |

### Q4 2025 - Écosystème Desktop
| Semaine | EPIC | US |
|:---:|:---|:---|
| S1-S3 | EPIC 8: VS Code | 8.1 → 8.5 |
| S4-S6 | EPIC 9: Electron | 9.1 → 9.5 |

---

## 🧪 Stratégie de Tests

### Tests Unitaires (TDD obligatoire)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_feature_basic() {
        // Arrange → Act → Assert
    }
}
```

### Tests de Non-Régression Performance
```rust
// benches/regression_benchmark.rs
use criterion::{criterion_group, Criterion};

fn bench_baseline(c: &mut Criterion) {
    // Mesurer avant chaque PR
}
```

### Seuils de Non-Régression
| Métrique | Baseline | Seuil Alerte |
|:---|:---:|:---:|
| SIMD Cosine 768D | 41ns | +10% max |
| HNSW Search 10K | 128µs | +15% max |
| VelesQL Parse | 570ns | +10% max |
| Recall@10 | 99.4% | -2% max |

---

## 🔄 Workflow Git

```
main ← stable, tagged releases
└── develop ← intégration
    ├── feature/sq8-quantization
    ├── feature/arm-neon-simd
    ├── feature/binary-quantization
    └── ...
```

**Avant chaque merge vers develop**:
```powershell
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --all-features
cargo bench --bench regression_benchmark
```

---

*Document maintenu par l'équipe VelesDB*
