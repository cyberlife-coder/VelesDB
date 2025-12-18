# VelesDB-Core - Règles TDD

**Test-Driven Development obligatoire**

---

## Principe fondamental

> **Écrire les tests AVANT le code**

Tout nouveau code dans VelesDB-Core doit suivre le cycle TDD :

```
1. RED    → Écrire un test qui échoue
2. GREEN  → Écrire le code minimal pour faire passer le test
3. REFACTOR → Améliorer le code sans casser les tests
```

---

## Structure des tests

### Tests unitaires (dans le même fichier)

```rust
// src/my_module.rs

pub fn calculate_distance(a: &[f32], b: &[f32]) -> Result<f32, Error> {
    // Implémentation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_distance_identical_vectors() {
        // Arrange
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];

        // Act
        let result = calculate_distance(&a, &b).unwrap();

        // Assert
        assert!((result - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_calculate_distance_different_dimensions() {
        // Arrange
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];

        // Act
        let result = calculate_distance(&a, &b);

        // Assert
        assert!(result.is_err());
    }
}
```

### Pattern Arrange-Act-Assert

| Phase | Description |
|-------|-------------|
| **Arrange** | Préparer les données de test |
| **Act** | Exécuter la fonction à tester |
| **Assert** | Vérifier le résultat |

---

## Conventions de nommage

```rust
#[test]
fn test_<fonction>_<scenario>() { ... }

// Exemples :
fn test_search_returns_top_k_results() { ... }
fn test_insert_with_invalid_dimension_fails() { ... }
fn test_delete_nonexistent_point_is_noop() { ... }
```

---

## Types de tests

### 1. Tests de succès

```rust
#[test]
fn test_upsert_single_point() {
    let collection = create_test_collection();
    let point = create_test_point(1);
    
    let result = collection.upsert(vec![point]);
    
    assert!(result.is_ok());
    assert_eq!(collection.count(), 1);
}
```

### 2. Tests d'erreur

```rust
#[test]
fn test_upsert_wrong_dimension() {
    let collection = create_test_collection(); // dim=768
    let wrong_point = Point::new(1, vec![0.0; 128], None); // dim=128
    
    let result = collection.upsert(vec![wrong_point]);
    
    assert!(matches!(result, Err(Error::DimensionMismatch { .. })));
}
```

### 3. Tests property-based (optionnel mais recommandé)

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_distance_is_symmetric(a in prop::collection::vec(-1.0f32..1.0, 768),
                                   b in prop::collection::vec(-1.0f32..1.0, 768)) {
        let d1 = distance(&a, &b);
        let d2 = distance(&b, &a);
        prop_assert!((d1 - d2).abs() < 1e-6);
    }
}
```

---

## Helpers de test

Créer des fonctions helper dans `#[cfg(test)]` :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_db() -> Database {
        let dir = tempdir().unwrap();
        Database::open(dir.path()).unwrap()
    }

    fn create_test_point(id: u64) -> Point {
        Point::new(id, vec![0.0; 768], Some(json!({"test": true})))
    }
}
```

---

## Couverture minimale

| Composant | Couverture cible |
|-----------|------------------|
| Fonctions publiques | 100% |
| Chemins d'erreur | 100% |
| Fonctions privées critiques | 80% |
| Code async | 80% |

---

## Vérification

Avant chaque commit :

```powershell
# Lancer tous les tests
cargo test --all-features

# Avec verbosité pour debug
cargo test --all-features -- --nocapture

# Un test spécifique
cargo test test_function_name
```

---

## Anti-patterns à éviter

| ❌ Ne pas faire | ✅ Faire |
|----------------|----------|
| `#[ignore]` sans raison | Fixer ou supprimer le test |
| Tests qui dépendent de l'ordre | Tests isolés et indépendants |
| `unwrap()` dans les tests sans message | `expect("description du contexte")` |
| Assertions vagues | Assertions précises et lisibles |
| Tests flaky (aléatoires) | Tests déterministes |

---

*Voir aussi : [CODING_RULES.md](./CODING_RULES.md)*
