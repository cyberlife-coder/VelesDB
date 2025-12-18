---
description: Vérification rapide avant commit (fmt, clippy, test)
---

# Workflow : Quick Check

Exécute les vérifications essentielles avant un commit.

## Étapes

// turbo
1. Formater le code
```powershell
cargo fmt --all
```

// turbo
2. Lancer clippy
```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

// turbo
3. Lancer les tests
```powershell
cargo test --all-features
```

## Alternative : Tout en un

// turbo
```powershell
cargo make check
```

## Si tout passe

Tu peux commit :
```powershell
git add .
git commit -m "feat/fix/docs: description"
```
