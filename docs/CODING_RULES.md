# VelesDB-Core - Règles de Développement

## 🎯 Objectif du Projet

VelesDB-Core est le **moteur de base de données vectorielles** open-source. Il fournit l'API publique et les fonctionnalités fondamentales consommées par VelesDB-Premium.

---

## 📐 Architecture

### Structure des Crates

```
velesdb-core/
├── crates/
│   ├── velesdb-core/      # Moteur principal (storage, indexing, search)
│   └── velesdb-server/    # API REST/gRPC
```

### Principes Architecturaux

- **Séparation des responsabilités** : Chaque module a une responsabilité unique
- **API stable** : Le Core est une dépendance versionnée du Premium
- **Zero-copy** : Privilégier `&[u8]`, `Bytes`, `memmap2` pour les performances
- **Async-first** : Utiliser `tokio` pour toutes les I/O

---

## 🧪 Test-Driven Development (TDD)

### Workflow Obligatoire

1. **Rouge** : Écrire le test qui échoue
2. **Vert** : Écrire le code minimal pour passer le test
3. **Bleu** : Refactoriser sans casser les tests

### Couverture Minimale

- **Objectif** : > 80% de couverture de code
- **Outil** : `cargo tarpaulin`

### Types de Tests

```rust
// Test unitaire (dans le même fichier)
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_feature_basic() {
        // Arrange
        // Act
        // Assert
    }
}

// Test d'intégration (dans tests/)
#[tokio::test]
async fn test_integration_scenario() {
    // ...
}

// Benchmark (dans benches/)
fn benchmark_search(c: &mut Criterion) {
    // ...
}
```

---

## 🔧 Standards de Code

### Formatage

```bash
cargo fmt --all -- --check
```

### Linting

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Conventions de Nommage

| Type | Convention | Exemple |
|------|------------|---------|
| Structs | PascalCase | `VectorIndex` |
| Traits | PascalCase | `Searchable` |
| Functions | snake_case | `find_nearest` |
| Constants | SCREAMING_SNAKE | `MAX_DIMENSIONS` |
| Modules | snake_case | `vector_storage` |

### Règles Spécifiques

- **Pas de `unwrap()`** en production (sauf après validation)
- **Gestion d'erreurs** avec `thiserror` et `anyhow`
- **Documentation** obligatoire sur l'API publique (`///`)
- **Fichiers < 500 lignes** : diviser si nécessaire

---

## 🔒 Sécurité

### Audit Automatique

```bash
cargo audit
cargo deny check
```

### Règles

- Pas de `unsafe` sans justification documentée
- Valider toutes les entrées utilisateur
- Pas de secrets dans le code

---

## 🚀 Performance

### Benchmarks

```bash
cargo bench --all-features
```

### Principes

- **Mesurer avant d'optimiser**
- Utiliser `criterion` pour les benchmarks
- Profiler avec `cargo flamegraph`

---

## 📦 Release

### Versioning Sémantique

| Type | Quand |
|------|-------|
| MAJOR | Changement d'API incompatible |
| MINOR | Nouvelle fonctionnalité compatible |
| PATCH | Correction de bug |

### Commande

```bash
./scripts/release.sh patch|minor|major
```

---

## ✅ Checklist Pre-commit

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] Documentation à jour
- [ ] Pas de secrets dans le code
