---
description: Refactorer du code Rust en toute sécurité
---

# Workflow : Refactoring Rust

## 1. Préparation

1. S'assurer que les tests passent AVANT le refactoring :
// turbo
```powershell
cargo test --all-features
```

2. Identifier clairement le scope du refactoring
3. NE PAS mélanger refactoring et nouvelles features

## 2. Analyse

1. Lister les fichiers impactés
2. Vérifier les dépendances avec `code_search`
3. Identifier les usages publics vs privés

## 3. Refactoring par étapes

### Renommage

1. Utiliser les outils Rust :
   - `cargo clippy` pour suggestions
   - IDE rename (si disponible)

2. Vérifier la compilation après chaque changement :
// turbo
```powershell
cargo check
```

### Extraction de fonction

```rust
// Avant
fn complex_function() {
    // ... beaucoup de code ...
    let result = {
        // logique complexe
    };
    // ... suite ...
}

// Après
fn extracted_logic() -> Result {
    // logique complexe
}

fn complex_function() {
    // ... 
    let result = extracted_logic();
    // ...
}
```

### Extraction de module

1. Créer le nouveau fichier `src/new_module.rs`
2. Déplacer le code
3. Ajouter `mod new_module;` dans `lib.rs`
4. Mettre à jour les imports

## 4. Validation continue

Après chaque étape significative :
// turbo
```powershell
cargo check
cargo test --all-features
```

## 5. Cleanup

1. Supprimer le code mort
2. Mettre à jour les imports inutilisés
3. Formater :
// turbo
```powershell
cargo fmt --all
```

## 6. Validation finale

// turbo
```powershell
cargo make check
```

## 7. Commit

Message de commit :
```
refactor: extract storage module from lib.rs

- Move Storage struct to dedicated module
- No functional changes
```
