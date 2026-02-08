---
name: ecosystem-sync
description: Propage une feature Core vers tous les SDKs et intégrations de l'écosystème
---

# /ecosystem-sync EPIC-XXX

Workflow OBLIGATOIRE après toute feature Core pour garantir la parité écosystème.

## 🎯 Objectif

> **Toute feature Core DOIT être propagée dans l'ensemble de l'écosystème**

## Étape 1: Identifier la feature

Lire l'EPIC indiquée et identifier:
- Nom de la feature
- APIs/fonctions exposées
- Breaking changes éventuels

## Étape 2: Checklist écosystème

Créer/mettre à jour `.epics/EPIC-XXX/ecosystem-sync.md`:

```markdown
# Ecosystem Sync - EPIC-XXX: [Nom Feature]

## Checklist de propagation

| Composant | Type | Status | PR | Notes |
|-----------|------|--------|-----|-------|
| velesdb-core | Engine | ✅ DONE | - | Source |
| velesdb-server | API HTTP | 🔴 TODO | - | Endpoint /api/... |
| velesdb-python | SDK Python | 🔴 TODO | - | PyO3 bindings |
| velesdb-wasm | SDK WASM | 🔴 TODO | - | wasm-bindgen |
| velesdb-mobile | SDK Mobile | 🔴 TODO | - | UniFFI |
| sdks/typescript | SDK TypeScript | 🔴 TODO | - | HTTP client |
| tauri-plugin-velesdb | Plugin Tauri | 🔴 TODO | - | Tauri commands |
| integrations/langchain | LangChain | 🔴 TODO | - | VectorStore/Retriever |
| integrations/llamaindex | LlamaIndex | 🔴 TODO | - | VectorStore |
| velesdb-cli | CLI | 🔴 TODO | - | Commandes |
| docs/ | Documentation | 🔴 TODO | - | Guides/API docs |
| tests/e2e_complete.rs | Tests E2E Core | 🔴 TODO | - | API interne |
| examples/ | Examples Rust/Python | 🔴 TODO | - | Exemples documentés |

## Tests cross-SDK

- [ ] Test Python: `pytest tests/test_[feature].py`
- [ ] Test WASM: `npm test -- [feature]`
- [ ] Test TypeScript: `npm test -- [feature]`
- [ ] Test E2E: API → Python → résultats identiques
```

## Étape 3: Créer les US de propagation

Pour chaque composant à mettre à jour:

1. Créer une US dans l'EPIC-016 (SDK Ecosystem Sync):
   ```
   .epics/EPIC-016-sdk-ecosystem-sync/US-XXX-propagate-[feature]-[sdk].md
   ```

2. Ou ajouter une US dans l'EPIC originale avec préfixe `[ECO]`

## Étape 4: Priorisation

Ordre de propagation recommandé:
1. **velesdb-server** (API HTTP = base pour tous les SDKs)
2. **velesdb-python** (SDK le plus utilisé)
3. **velesdb-cli** (debug/prototypage)
4. **integrations/langchain** (écosystème RAG)
5. **sdks/typescript** (web developers)
6. **velesdb-wasm** (browser)
7. **tauri-plugin-velesdb** (desktop)
8. **velesdb-mobile** (mobile)
9. **integrations/llamaindex** (écosystème RAG)

## Étape 5: Validation Tests Internes

**OBLIGATOIRE** - Vérifier que les tests internes utilisent la nouvelle API:
// turbo
```powershell
# Tests E2E Core (CRITIQUE - souvent oubliés!)
cargo check --test e2e_complete
cargo test --test e2e_complete -- --test-threads=1

# Examples Rust
cargo check -p mini_recommender
cd examples/rust && cargo check
```

## Étape 5b: Vérification SDK TypeScript (CRITIQUE!)

**Le SDK TypeScript est un client HTTP - il DOIT correspondre aux routes du serveur.**

Vérifier la correspondance:

| Opération | Server Route | SDK Endpoint (`rest.ts`) |
|-----------|--------------|-------------------------|
| Insert | `/collections/{name}/points` | `insert()` → `/points` |
| Search | `/collections/{name}/search` | `search()` → `/search` |
| Text Search | `/collections/{name}/search/text` | `textSearch()` → `/search/text` |
| Hybrid Search | `/collections/{name}/search/hybrid` | `hybridSearch()` → `/search/hybrid` |
| Multi Search | `/collections/{name}/search/multi` | `multiQuerySearch()` → `/search/multi` |
| Graph Edges | `/collections/{name}/graph/edges` | `addEdge()` / `getEdges()` |
| Traverse | `/collections/{name}/graph/traverse` | `traverseGraph()` |
| Empty | `/collections/{name}/empty` | `isEmpty()` |
| Flush | `/collections/{name}/flush` | `flush()` |

// turbo
```powershell
# Vérifier la compilation
cd sdks/typescript && npm run build

# Vérifier les tests
cd sdks/typescript && npm test

# Vérifier le README contient les nouvelles fonctionnalités
# Lire sdks/typescript/README.md
```

**Checklist SDK TypeScript:**
- [ ] Endpoints correspondent aux routes serveur (`main.rs` vs `rest.ts`)
- [ ] Body format correspond (ex: `{points: [...]}` vs `{id, vector}`)
- [ ] Tests passent (142+ tests)
- [ ] README.md documente les nouvelles features
- [ ] Types exportés dans `types.ts`

## Étape 6: Validation SDKs

Pour chaque SDK propagé:
// turbo
```powershell
# Python
cd crates/velesdb-python && maturin develop && pytest

# TypeScript (VÉRIFIÉ en Étape 5b)
cd sdks/typescript && npm run build && npm test

# WASM
cd crates/velesdb-wasm && wasm-pack test --headless --chrome

# LangChain
cd integrations/langchain && pytest

# LlamaIndex
cd integrations/llamaindex && pytest
```

**Documentation à vérifier pour chaque SDK:**
- `README.md` → Exemples de code à jour
- Types/interfaces → Correspondent à l'API
- Changelog → Nouvelles features documentées

## Étape 7: Mise à jour matrice

Mettre à jour la matrice de parité dans:
- `.epics/EPIC-016-sdk-ecosystem-sync/EPIC.md`
- `.epics/ROADMAP-2026-STRATEGY.md`

## Étape 8: Résumé

Afficher:
- ✅ Feature: [nom]
- 📊 SDKs propagés: X/10
- 🔴 SDKs restants: [liste]
- 📝 US créées pour propagation

---

## ⚠️ Règle obligatoire

**Une feature Core n'est PAS terminée tant que la propagation écosystème n'est pas planifiée.**

Le workflow `/complete-us` vérifiera automatiquement si `ecosystem-sync.md` existe pour les US Core.
