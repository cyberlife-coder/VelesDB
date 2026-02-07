---
name: pre-commit
description: Lance toutes les vérifications avant commit
---

# /pre-commit

Vérifications obligatoires avant tout commit.

## Check 1: Formatage

// turbo
`powershell
cargo fmt --all --check
`

Si échec: proposer cargo fmt --all

## Check 2: Linting Standard

// turbo
`powershell
cargo clippy --workspace --all-targets -- -D warnings
`

## Check 2.1: 🦀 Linting Rust-AI Strict

// turbo
`powershell
cargo clippy --workspace --all-targets -- -D warnings -W clippy::unwrap_used -W clippy::expect_used -W clippy::clone_on_ref_ptr -W clippy::cast_possible_truncation
`

**Règles Rust-AI activées:**
- `unwrap_used`: Détecte les `unwrap()` potentiellement dangereux
- `expect_used`: Encourage `?` au lieu de `expect()`
- `clone_on_ref_ptr`: Préférer `Arc::clone(&x)` à `x.clone()`
- `cast_possible_truncation`: Alerter sur `as u32` dangereux

## Check 3: Tests

// turbo
`powershell
cargo test --workspace
`

## Check 4: Audit Sécurité

// turbo
`powershell
cargo deny check
`

## Check 5: Dead Code Detection

// turbo
```powershell
cargo clippy --workspace -- -W dead_code -W unused_variables -W unused_imports
```

## Check 6: Vérifications Manuelles (PR Review Lessons)

### Checklist Code Quality

1. **CHANGELOG.md** mis à jour?
2. **Fichiers modifiés** < 500 lignes?

### Checklist Anti-Patterns (Issues PR #116)

3. **Enum Match Exhaustif**: Tous les variants sémantiquement équivalents sont couverts?
   - Ex: Si on traite `Similarity`, aussi traiter `VectorSearch` et `VectorFusedSearch`
   - Vérifier les `_ => ...` catch-all qui pourraient cacher des oublis

4. **Struct Validation Complète**: Toutes les sous-structures optionnelles sont validées?
   - Ex: `query.select.where_clause` ET `query.compound.right.where_clause`
   - Chercher les `Option<T>` imbriqués non traités

5. **Dead Fields/Params**: Tous les champs définis sont utilisés?
   - Pas de `pub field: Type` jamais lu
   - YAGNI: supprimer ce qui n'est pas utilisé

## Check 7: Couverture LLVM-Cov

// turbo
```powershell
cargo llvm-cov --workspace --fail-under 85
```

> Génère également `target/llvm-cov/html/index.html` pour inspection. La couverture doit rester ≥ 85% globalement et ≥ 90% pour les crates critiques documentées dans `AGENTS.md`.

## Check 8: Scan des sorties interdites (`println!`, `dbg!`, `eprintln!`)

// turbo
```powershell
rg --color never --line-number "(println!|dbg!|eprintln!)" crates sdks integrations | Out-File -FilePath .tmp\println_scan.txt
if ((Get-Content .tmp\println_scan.txt).Trim()) {
    Get-Content .tmp\println_scan.txt
    throw "Des macros println!/dbg!/eprintln! ont été détectées. Utiliser tracing::info!/debug!/warn!"
}
```

### Checklist Anti-Patterns (Issue PR #118)

6. **Multiple Validation Modules Sync**: Si logique de validation dupliquée, TOUS les modules sont mis à jour?
   - Ex: `velesql/validation.rs` (public API) ET `collection/search/query/validation.rs` (internal)
   - Chercher tous les fichiers avec `validation` dans le nom: `find_by_name("*validation*")`
   - Vérifier que les règles sont identiques entre modules

## Résumé

| Check | Status |
|-------|--------|
| Formatage | OK/FAIL |
| Linting | OK/FAIL |
| Linting Rust-AI | OK/FAIL |
| Tests | OK/FAIL |
| Sécurité | OK/FAIL |
| Dead Code | OK/FAIL |
| Couverture LLVM-Cov | OK/FAIL |
| Scan println!/dbg! | OK/FAIL |
| Enum Exhaustif | OK/FAIL |
| Struct Complet | OK/FAIL |
| Dead Fields | OK/FAIL |
| Validation Sync | OK/FAIL |

## ✅ Success Criteria (Gate de Validation)

**TOUS les critères doivent être verts avant commit:**

| # | Critère | Status |
|---|---------|--------|
| 1 | Build sans erreurs | ✅/❌ |
| 2 | Zéro erreurs Rust | ✅/❌ |
| 3 | Zéro warnings Clippy | ✅/❌ |
| 4 | Code formaté | ✅/❌ |
| 5 | Zéro code mort/unused | ✅/❌ |
| 6 | Couverture ≥ 85% | ✅/❌ |
| 7 | Zéro duplication | ✅/❌ |
| 8 | Tests passants | ✅/❌ |
| 9 | Build release OK | ✅/❌ |
|10 | Audit sécurité OK | ✅/❌ |
|11 | Hooks passants | ✅/❌ |

### Décision

- **10/10 ✅** → Commit autorisé
- **< 10 ✅** → **BLOQUER** - corriger d'abord

## Si Succès

Proposer message de commit:
`type(scope): description [EPIC-XXX/US-YYY]`

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `chore`
