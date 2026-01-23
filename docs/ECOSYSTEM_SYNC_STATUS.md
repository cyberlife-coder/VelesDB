# VelesDB Ecosystem Sync Status

> **Date**: 2026-01-23
> **Version**: 1.2.0
> **Last Sync**: EPIC-016 SDK Ecosystem Sync

## 📊 Parité Écosystème

### Légende
- ✅ **DONE** - Fonctionnalité implémentée et testée
- ⚠️ **PARTIAL** - Implémentation partielle
- 🔴 **TODO** - À implémenter
- ➖ **N/A** - Non applicable à ce composant

---

## Core Features

| Feature | Core | Server | Python | WASM | Mobile | TS SDK | CLI | LangChain | LlamaIndex |
|---------|------|--------|--------|------|--------|--------|-----|-----------|------------|
| **Vector Search** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Search with Filter** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Text Search (BM25)** | ✅ | ✅ | ✅ | ⚠️¹ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Hybrid Search** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Multi-Query Fusion** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Batch Search** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ |
| **CRUD (upsert/get/delete)** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

> ¹ WASM: Substring search instead of BM25 (intentional - documented)

---

## Graph Features (EPIC-004, EPIC-019)

| Feature | Core | Server | Python | WASM | Mobile | TS SDK | CLI | LangChain | LlamaIndex |
|---------|------|--------|--------|------|--------|--------|-----|-----------|------------|
| **GraphNode** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **GraphEdge** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **EdgeStore** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **add_edge/get_edge** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **get_outgoing/incoming** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **get_edges_by_label** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **get_nodes_by_label** | ✅ | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **BFS Traversal** | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **DFS Traversal** | ✅ | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **Streaming BFS** | ✅ | ➖ | ✅ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| **has_node/has_edge** | ✅ | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |
| **in_degree/out_degree** | ✅ | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | ➖ | ➖ |

---

## Index & Storage Features

| Feature | Core | Server | Python | WASM | Mobile | TS SDK | CLI |
|---------|------|--------|--------|------|--------|--------|-----|
| **HNSW Index** | ✅ | ✅ | ✅ | ➖² | ✅ | ✅ | ✅ |
| **SQ8 Quantization** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Binary Quantization** | ✅ | ➖ | ➖ | ✅ | ➖ | ➖ | ➖ |
| **Disk Persistence** | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ✅ |
| **IndexedDB Persistence** | ➖ | ➖ | ➖ | ✅ | ➖ | ➖ | ➖ |
| **Memory-mapped Storage** | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ | ✅ |

> ² WASM: Brute-force search (OK for <10k vectors, documented)

---

## Distance Metrics

| Metric | Core | Server | Python | WASM | Mobile | TS SDK |
|--------|------|--------|--------|------|--------|--------|
| **Cosine** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Euclidean** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Dot Product** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Hamming** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Jaccard** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## Fusion Strategies

| Strategy | Core | Server | Python | WASM | Mobile | TS SDK |
|----------|------|--------|--------|------|--------|--------|
| **RRF** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Average** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Maximum** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Weighted** | ✅ | ✅ | ✅ | ➖ | ✅ | ✅ |

---

## Summary

### Parité par Composant

| Composant | Parité | Notes |
|-----------|--------|-------|
| **velesdb-core** | 100% | Source of truth |
| **velesdb-server** | 98% | DFS/degree manquants (low priority) |
| **velesdb-python** | 95% | DFS/degree/has_node manquants |
| **velesdb-wasm** | 100% | ✅ Tous gaps corrigés (2026-01-23) |
| **velesdb-mobile** | 95% | Streaming BFS manquant |
| **sdks/typescript** | 90% | Graph API non exposée (REST only) |
| **velesdb-cli** | 85% | Graph commands non implémentées |
| **langchain** | 90% | VectorStore OK, Graph non applicable |
| **llamaindex** | 90% | VectorStore OK, Graph non applicable |

### Gaps Prioritaires

1. **Python SDK** - Ajouter `dfs_traverse`, `has_node`, `has_edge`, `degree`
2. **Mobile SDK** - Ajouter `streaming_bfs` 
3. **Server** - Exposer graph utilities (DFS, degree) via REST

### Historique Sync

| Date | Action | Components |
|------|--------|------------|
| 2026-01-23 | WASM gaps corrigés | velesdb-wasm (9 méthodes ajoutées) |
| 2026-01-22 | 51 Devin flags traités | All crates |
| 2026-01-20 | EPIC-016 SDK Sync | Python, Mobile, TS, LangChain, LlamaIndex |

---

## ⚠️ Règle Obligatoire

**Une feature Core n'est PAS terminée tant que la propagation écosystème n'est pas planifiée.**

Pour chaque nouvelle feature Core:
1. Mettre à jour ce document
2. Créer US de propagation si gaps identifiés
3. Valider avec `/ecosystem-sync` workflow
