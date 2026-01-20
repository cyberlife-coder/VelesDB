# Ecosystem Sync - EPIC-008: Vector-Graph Fusion

## 🎯 Feature à propager

**similarity()** function dans VelesQL permettant:
- Filtrage par seuil de similarité: `WHERE similarity(field, $vec) > 0.8`
- Opérateurs: `>`, `>=`, `<`, `<=`, `=`
- Fusion RRF/Weighted pour scores combinés

## APIs exposées dans Core

```rust
// VelesQL parsing
parse("SELECT * FROM docs WHERE similarity(embedding, $q) > 0.8")

// Collection::execute_query avec similarity_condition
collection.execute_query(&query, &params)?

// Fusion strategies
FusionStrategy::Rrf { k: 60 }
FusionStrategy::Weighted { vector_weight: 0.7, graph_weight: 0.3 }
```

## Checklist de propagation

| Composant | Type | Status | PR | Notes |
|-----------|------|--------|-----|-------|
| velesdb-core | Engine | ✅ DONE | #61 | Source - Parser + Executor |
| velesdb-wasm | SDK WASM | ✅ DONE | #61 | similarity_search() method |
| velesdb-server | API HTTP | 🔴 TODO | - | POST /query avec similarity |
| velesdb-python | SDK Python | ✅ DONE | - | Uses core execute_query directly |
| velesdb-cli | CLI | ✅ DONE | - | Uses core execute_query directly |
| sdks/typescript | SDK TypeScript | 🔴 TODO | - | HTTP client query() |
| integrations/langchain | LangChain | 🟡 PARTIAL | - | VectorStore exists, add similarity |
| integrations/llamaindex | LlamaIndex | 🔴 TODO | - | VectorStore avec similarity |
| tauri-plugin-velesdb | Plugin Tauri | 🔴 TODO | - | Tauri commands |
| velesdb-mobile | SDK Mobile | 🔴 TODO | - | UniFFI bindings |
| docs/ | Documentation | 🔴 TODO | - | VelesQL similarity guide |

## Priorité de propagation

1. **velesdb-server** - API HTTP = base pour clients
2. **velesdb-python** - SDK le plus utilisé
3. **velesdb-cli** - Debug/prototypage
4. **integrations/langchain** - Écosystème RAG
5. **sdks/typescript** - Web developers
6. **docs/** - Documentation utilisateur

## Tests cross-SDK requis

- [ ] Test Python: `collection.query("... similarity(...) > 0.8")`
- [ ] Test TypeScript: `client.query({ where: "similarity(...) > 0.8" })`
- [ ] Test CLI: `velesdb query "SELECT ... WHERE similarity(...) > 0.8"`
- [ ] Test E2E: Résultats identiques Core ↔ Python ↔ HTTP

## US créées pour propagation

→ Voir **EPIC-016/US-001**: Propager similarity() vers tous SDKs
