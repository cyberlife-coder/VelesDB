# US-001: Propager similarity() vers tous SDKs

**EPIC Parent**: EPIC-016
**Complexité**: L
**Status**: 🟡 IN PROGRESS

## 📋 Description

En tant que développeur utilisant n'importe quel SDK,
Je veux pouvoir utiliser similarity() dans mes requêtes,
Afin d'exploiter la fusion vector-graph quelle que soit ma plateforme.

## ✅ Critères d'Acceptation

### AC-1: Python SDK

```gherkin
GIVEN velesdb-python installé
WHEN j'exécute une requête avec similarity()
THEN les résultats sont filtrés par similarité
```

```python
results = db.query("""
    MATCH (d:Document)-[:AUTHORED_BY]->(p:Person)
    WHERE similarity(d.embedding, $v) > 0.8
    RETURN p.name
""", params={"v": query_vector})
```

### AC-2: WASM SDK

```gherkin
GIVEN velesdb-wasm chargé
WHEN j'exécute la même requête depuis JavaScript
THEN les résultats sont identiques
```

### AC-3: Mobile SDK

```gherkin
GIVEN velesdb-mobile (iOS/Android)
WHEN j'exécute similarity() depuis Swift/Kotlin
THEN la fonctionnalité est disponible
```

### AC-4: TypeScript SDK

```gherkin
GIVEN @velesdb/client
WHEN j'utilise le query builder avec similarity
THEN la requête est correctement construite
```

## 🧪 Tests Requis

Par SDK:
- [ ] Python: `test_similarity_query.py`
- [ ] WASM: `similarity.spec.ts`
- [ ] Mobile: `SimilarityTest.swift`, `SimilarityTest.kt`
- [ ] TypeScript: `similarity.test.ts`
- [ ] Tauri: `test_similarity.rs`
- [ ] LangChain: `test_similarity_retriever.py`
- [ ] LlamaIndex: `test_similarity_retriever.py`

## 📁 Fichiers Impactés

| SDK | Fichiers |
|-----|----------|
| Python | `velesdb-python/src/lib.rs`, `collection.rs` |
| WASM | `velesdb-wasm/src/lib.rs` |
| Mobile | `velesdb-mobile/src/lib.rs` |
| TypeScript | `sdks/typescript/src/query.ts` |
| Tauri | `tauri-plugin-velesdb/src/commands/query.rs` |
| LangChain | `integrations/langchain/src/.../retriever.py` |
| LlamaIndex | `integrations/llamaindex/src/.../retriever.py` |

## 📝 Checklist de propagation

| SDK | Implémenté | Testé | Documenté |
|-----|------------|-------|-----------|
| Python (PyO3) | ✅ | 🟡 | 🔴 |
| WASM | ✅ | ✅ | 🔴 |
| Mobile (UniFFI) | 🔴 | 🔴 | 🔴 |
| TypeScript | 🔴 | 🔴 | 🔴 |
| Tauri Plugin | 🔴 | 🔴 | 🔴 |
| LangChain | 🔴 | 🔴 | 🔴 |
| LlamaIndex | 🔴 | 🔴 | 🔴 |
| CLI | ✅ | 🔴 | 🔴 |

## 📅 Historique

| Date | Status | Notes |
|------|--------|-------|
| 2026-01-20 | 🔴 TODO | Créée - dépend de EPIC-008 |
| 2026-01-20 | 🟡 IN PROGRESS | EPIC-008 mergée, WASM déjà fait |
