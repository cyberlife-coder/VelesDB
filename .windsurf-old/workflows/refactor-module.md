---
description: Refactoring profond d'un fichier trop large en modules - Méthode Martin Fowler adaptée Rust
---

# Workflow: Refactoring Module Extraction

## Principes Fondamentaux (Martin Fowler)

1. **Tiny Steps**: Chaque changement doit être minime et vérifiable
2. **Tests GREEN avant/après**: Ne jamais refactorer sans tests passants
3. **Commits séparés**: Moves/renames SÉPARÉS des edits
4. **"Make the change easy, then make the easy change"** (Kent Beck)

---

## Phase 1: Analyse et Préparation

### 1.1 Baseline des tests
```powershell
# Vérifier que tous les tests passent AVANT de commencer
cargo test -p <crate_name>
cargo clippy -p <crate_name> -- -D warnings
```
- Si tests échouent → STOP, fixer d'abord
- Commit: "chore: baseline tests green before refactoring"

### 1.2 Analyse structurelle du fichier
- Compter les lignes: `(Get-Content <file> | Measure-Object -Line).Lines`
- Identifier les groupes logiques (structs, impls, helpers)
- Dessiner les dépendances entre groupes
- Documenter dans un commentaire ou fichier temporaire

### 1.3 Plan d'extraction
Prioriser par:
1. **Faible couplage** → Plus facile à extraire
2. **Haute cohésion** → Forme un module logique
3. **Pas de macros complexes** → PyO3 #[pyclass] reste dans lib.rs

---

## Phase 2: Réorganisation (Commits séparés)

### 2.1 Grouper les méthodes par responsabilité
```rust
// Utiliser des commentaires de région
// === HELPERS ===
fn helper1() {}
fn helper2() {}

// === STRUCT A ===
pub struct A {}
impl A {}

// === STRUCT B ===
pub struct B {}
impl B {}
```
- Commit: "refactor: reorganize methods by responsibility [no logic change]"

### 2.2 Identifier les dépendances
Pour chaque groupe, lister:
- Imports utilisés
- Fonctions appelées depuis d'autres groupes
- Types partagés

---

## Phase 2.5: 🦀 Analyse Rust-Specific Avant Extraction

### Borrow Checker Considerations

Avant d'extraire, identifier:

1. **Références croisées entre structs**
   ```rust
   // ❌ PROBLÈME: Struct A référence Struct B dans le même fichier
   struct A<'a> { b_ref: &'a B }
   struct B { data: Vec<u8> }
   // → Extraction complexe: lifetimes à propager
   ```

2. **Méthodes avec `&self` / `&mut self`**
   - Si méthodes accèdent à plusieurs champs mutables → potentiel conflit après split
   - Solution: extraire des "helper functions" qui prennent les champs individuellement

3. **Visibilité des champs**
   ```rust
   // Avant: accès direct aux champs privés
   impl A {
       fn process(&self) { self.inner_field.do_thing() }
   }
   // Après extraction: besoin de pub(super) ou accesseurs
   ```

### Checklist Pré-Extraction
- [ ] Lister toutes les `&self` et `&mut self` méthodes
- [ ] Identifier les lifetimes implicites qui devront être explicites
- [ ] Vérifier que les traits `Send`/`Sync` seront préservés
- [ ] Prévoir les re-exports dans mod.rs

---

## Phase 3: Extraction Incrémentale

### 3.1 Créer le nouveau module (vide d'abord)
```rust
// new_module.rs
//! Description du module

// Imports nécessaires
use ...;

// TODO: Code à déplacer
```
- Dans lib.rs: `mod new_module;`
- Commit: "refactor: create empty new_module.rs"

### 3.2 Déplacer UNE fonction à la fois
1. Copier la fonction dans le nouveau module
2. Ajouter `pub` si nécessaire
3. Dans lib.rs: `use new_module::function_name;`
4. Supprimer l'ancienne fonction de lib.rs
5. `cargo check` → doit compiler
6. Commit: "refactor: move function_name to new_module"

### 3.3 Répéter pour chaque fonction/struct
- Un commit par déplacement
- Vérifier compilation après chaque move
- Ne jamais modifier la logique pendant un move

---

## Phase 4: Nettoyage

### 4.1 Optimiser les imports
```rust
// Avant (après extraction)
use new_module::func1;
use new_module::func2;
use new_module::func3;

// Après
use new_module::{func1, func2, func3};
```
- Commit: "refactor: consolidate imports"

### 4.2 Vérification finale
```powershell
cargo fmt --all
cargo clippy -p <crate_name> -- -D warnings
cargo test -p <crate_name>
```

### 4.3 Documenter le nouveau module
```rust
//! # Module Name
//! 
//! Description du module et son rôle.
//! 
//! ## Example
//! ```rust
//! use crate::new_module::...;
//! ```
```

---

## Phase 5: Validation Fou Furieux

// turbo
1. `cargo fmt --all -- --check`
// turbo
2. `cargo clippy -p <crate_name> -- -D warnings`
// turbo
3. `cargo test -p <crate_name>`
4. Vérifier métriques: fichier source < 500 lignes
5. Commit final si tout passe

---

## Cas Spéciaux

### PyO3 (#[pyclass], #[pymethods])
- Les structs avec `#[pyclass]` DOIVENT rester dans le même crate
- Possible d'extraire dans un sous-module du même crate
- Pattern: struct dans module, re-export dans lib.rs
```rust
// collection.rs
#[pyclass]
pub struct Collection { ... }

// lib.rs
mod collection;
pub use collection::Collection;
```

### Traits et Impls
- Le trait peut être dans un module séparé
- Les impls doivent être soit avec le trait, soit avec le type

---

## Checklist Finale

- [ ] Tests GREEN avant refactoring
- [ ] Chaque move dans un commit séparé
- [ ] Pas de changement de logique pendant les moves
- [ ] Tests GREEN après refactoring
- [ ] Clippy clean
- [ ] Fichier source < 500 lignes
- [ ] Nouveau module documenté
- [ ] Progress.md mis à jour
