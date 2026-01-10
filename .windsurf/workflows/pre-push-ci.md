---
description: CI local obligatoire avant push - Évite les échecs GitHub Actions
---

# CI Local avant Push

**Objectif** : Valider localement AVANT de push pour éviter les échecs sur GitHub Actions (économie de temps et crédits).

---

## Validation rapide (< 2 min)

// turbo
1. **Formatting check**
```powershell
cargo fmt --all --check
```

// turbo
2. **Clippy strict** (identique à la CI)
```powershell
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
```

// turbo
3. **Tests rapides**
```powershell
cargo test --all-features --workspace
```

---

## Validation complète (optionnel, ~5 min)

4. **Security audit** (si dépendances modifiées)
```powershell
cargo audit --ignore RUSTSEC-2024-0320
```

5. **SIMD benchmarks** (si code SIMD/index modifié)
```powershell
cargo bench -p velesdb-core --bench simd_benchmark -- --noplot
```

---

## Corrections rapides

| Erreur | Commande de fix |
|--------|-----------------|
| Formatting | `cargo fmt --all` |
| Clippy warnings | Corriger manuellement selon suggestions |
| Tests échoués | Lire l'erreur, fixer le code |

---

## Checklist avant push

- [ ] `cargo fmt --all --check` ✅
- [ ] `cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic` ✅  
- [ ] `cargo test --all-features --workspace` ✅
- [ ] Commit message clair et descriptif

**⚠️ Ne JAMAIS push si une étape échoue. La CI GitHub rejettera de toute façon.**
