---
description: Vérifie que tous les exemples du README.md sont testés et fonctionnent
---

# Test README Examples

## Objectif
S'assurer que chaque exemple de code dans README.md a un test correspondant qui prouve qu'il fonctionne.

## Étapes

### 1. Extraire les exemples du README

```powershell
# Lister les blocs de code
Select-String -Path "README.md" -Pattern '```(rust|sql|bash|python)' -Context 0,20
```

### 2. Vérifier les tests existants

```powershell
# Tests README existants
cargo test readme --list
cargo test integration_scenarios --list
```

### 3. Pour chaque exemple non testé

Créer un test dans `tests/readme_examples.rs` ou `tests/integration_scenarios.rs`:

```rust
#[test]
fn test_readme_example_XXX() {
    // Copier-coller EXACT du code README
    // Avec mocks si nécessaire
}
```

### 4. Valider

```powershell
cargo test readme_examples
cargo test integration_scenarios
```

## Checklist

- [ ] Tous les `\`\`\`rust` du README ont un test
- [ ] Tous les `\`\`\`sql` (VelesQL) du README ont un test parser
- [ ] Les exemples CLI sont testés dans velesdb-cli
- [ ] Les tests passent en CI

## Fichiers Concernés

- `README.md`
- `tests/readme_examples.rs`
- `tests/integration_scenarios.rs`
- `crates/velesdb-cli/tests/`

## Quand utiliser

- Avant chaque release
- Après modification du README.md
- Lors de l'ajout de nouvelles features documentées
