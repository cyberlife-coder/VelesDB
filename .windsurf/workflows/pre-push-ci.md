---
description: CI local obligatoire avant push
---

# CI Local avant Push

**TOUJOURS exécuter ces commandes avant tout push/PR sur velesdb-core :**

// turbo
1. Formatting check
```bash
cargo fmt --all --check
```

// turbo
2. Clippy (pedantic avec warnings comme erreurs)
```bash
cargo clippy --all-targets --all-features -- -D warnings
```

// turbo
3. Tests
```bash
cargo test --all-features
```

Si une étape échoue :
- `cargo fmt --all` pour corriger le formatting
- Corriger les erreurs clippy manuellement
- Corriger les tests

**Ne JAMAIS push sans que ces 3 étapes passent.**
