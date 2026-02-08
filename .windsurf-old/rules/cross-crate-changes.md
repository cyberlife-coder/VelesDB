---
trigger: model_decision
description: Appliquer lors de modifications impactant plusieurs crates du workspace
---

# Modifications Cross-Crate

## Détection automatique

Cette règle s'applique quand :
- Une fonction/struct de `velesdb-core` est modifiée
- Un trait partagé est modifié
- Une dépendance du workspace Cargo.toml change

## Checklist d'impact

### 1. Identifier les crates impactés

```powershell
# Liste des crates du workspace
cargo metadata --format-version 1 | jq '.packages[].name'

# Dépendances inverses
cargo tree --invert -p velesdb-core
```

### 2. Matrice de propagation

| Crate source | Crates dépendants | Action |
|--------------|-------------------|--------|
| velesdb-core | server, cli, wasm, python, mobile | Propager les changements |
| velesdb-server | - | Point final |
| velesdb-wasm | - | Point final |

### 3. Ordre de modification

1. **velesdb-core** - Module central
2. **velesdb-server** - API HTTP
3. **velesdb-cli** - Interface CLI
4. **velesdb-wasm** - Bindings WebAssembly
5. **velesdb-python** - Bindings Python (PyO3)
6. **velesdb-mobile** - UniFFI mobile

### 4. Tests par crate

```powershell
# Test individuel
cargo test -p velesdb-core
cargo test -p velesdb-server
cargo test -p velesdb-wasm

# Test workspace complet
cargo test --workspace
```

## Breaking Changes Protocol

Si la modification casse une API publique :

1. **Documenter** dans `CHANGELOG.md`
2. **Deprecate** avant de supprimer (2 versions minimum)
3. **Migration guide** dans `docs/`
4. **Version bump** approprié (SemVer)

```rust
#[deprecated(since = "1.2.0", note = "Use `new_api` instead")]
pub fn old_api() { ... }
```
