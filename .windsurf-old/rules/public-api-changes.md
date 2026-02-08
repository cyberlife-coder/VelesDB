---
trigger: glob
globs: ["**/src/lib.rs", "**/src/**/mod.rs"]
description: Règles pour modifications d'API publique
---

# Modifications d'API Publique

Vous modifiez un fichier qui définit l'API publique du crate.

## Avant toute modification

1. **Vérifier les usages externes**:
   ```powershell
   rg "pub (fn|struct|enum|trait|type)" <fichier> --no-heading
   ```

2. **Lister les dépendants**:
   ```powershell
   cargo tree --invert -p velesdb-core
   ```

## Règles obligatoires

- [ ] **Breaking change?** Documenter dans CHANGELOG.md
- [ ] **Nouvelle fonction pub?** Ajouter documentation `///`
- [ ] **Signature modifiée?** Mettre à jour SDKs (Python, WASM, TS)
- [ ] **Trait modifié?** Vérifier toutes les implémentations

## Pattern de versioning

```rust
// Pour deprecation
#[deprecated(since = "1.2.0", note = "Use new_function instead")]
pub fn old_function() { ... }

// Pour feature gates
#[cfg(feature = "experimental")]
pub fn experimental_function() { ... }
```

## Tests requis

```powershell
cargo test --workspace
cargo doc --no-deps  # Vérifier que la doc compile
```
