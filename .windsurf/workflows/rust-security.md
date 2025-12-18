---
description: Audit de sécurité et correction de vulnérabilités Rust
---

# Workflow : Sécurité Rust

## 1. Audit des dépendances

1. Lancer cargo-audit :
```powershell
cargo audit
```

2. Lancer cargo-deny :
```powershell
cargo deny check
```

3. Analyser les vulnérabilités reportées

## 2. Correction des vulnérabilités

### Mise à jour de dépendance

1. Mettre à jour la version dans `Cargo.toml`
2. Vérifier la compatibilité :
// turbo
```powershell
cargo check
cargo test --all-features
```

### Si breaking change

1. Lire le changelog de la crate
2. Adapter le code si nécessaire
3. Tester exhaustivement

## 3. Revue du code sensible

### Patterns à vérifier

- **Input validation** : Toutes les entrées utilisateur sont validées
- **Error handling** : Pas de `unwrap()` sur des données externes
- **SQL/Command injection** : Utiliser des requêtes paramétrées
- **Path traversal** : Valider les chemins de fichiers
- **Overflow** : Utiliser `checked_*` pour les opérations arithmétiques

### Exemple de fix

```rust
// AVANT (vulnérable)
fn load_file(name: &str) -> Result<Vec<u8>> {
    std::fs::read(format!("data/{}", name))
}

// APRÈS (sécurisé)
fn load_file(name: &str) -> Result<Vec<u8>> {
    // Valider que le nom ne contient pas de path traversal
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(Error::InvalidFileName);
    }
    std::fs::read(format!("data/{}", name))
}
```

## 4. Secrets

1. Vérifier qu'aucun secret n'est hardcodé :
```powershell
git diff --cached | Select-String -Pattern "(api_key|secret|password|token)\s*="
```

2. Utiliser des variables d'environnement :
```rust
let api_key = std::env::var("API_KEY")
    .map_err(|_| Error::MissingApiKey)?;
```

## 5. Validation

// turbo
```powershell
cargo audit
cargo deny check
cargo test --all-features
```

## 6. Documentation

1. Documenter les mesures de sécurité dans le code
2. Mettre à jour `SECURITY.md` si applicable
