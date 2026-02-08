---
trigger: always_on
---

# Stratégie de Tests VelesDB

## 📊 Pyramide des Tests

```
        /\
       /E2E\        ← Tests End-to-End (API HTTP, CLI, SDKs)
      /------\
     /Intégra-\     ← Tests d'Intégration (Scénarios métier complets)
    /  tion    \
   /------------\
  /    Unitaires  \  ← Tests Unitaires (Fonctions isolées)
 /________________\
```

## 🎯 Niveaux de Tests

### 1. Tests Unitaires (`src/*_tests.rs`)

**Objectif**: Tester les fonctions isolées, logique pure.

```rust
// Nommage: test_[fonction]_[scenario]_[resultat_attendu]
#[test]
fn test_extract_trigrams_empty_string_returns_empty() { ... }
```

**Règles**:
- Pas d'I/O disque
- Pas de dépendances externes
- Exécution < 100ms par test
- Mocks pour les dépendances

### 2. Tests d'Intégration (`tests/*.rs`)

**Objectif**: Tester les scénarios métier réels (BDD-style).

```rust
// tests/use_cases_integration_tests.rs
mod use_case_1_contextual_rag {
    #[test]
    fn test_contextual_rag_basic_similarity() {
        // GIVEN: Une collection avec des documents
        // WHEN: Recherche par similarité
        // THEN: Documents pertinents retournés
    }
}
```

**Règles**:
- Utiliser `tempfile::TempDir` pour isolation
- Tester le workflow complet (create → insert → search → delete)
- Nommer par use case métier, pas par fonction technique

### 3. Tests E2E (`crates/velesdb-server/tests/`)

**Objectif**: Tester l'API HTTP comme un client réel.

```rust
// tests/api_integration.rs
#[tokio::test]
async fn test_create_collection_via_http() {
    let client = reqwest::Client::new();
    let resp = client.post("/collections").json(&body).send().await;
    assert_eq!(resp.status(), 201);
}
```

## 🧪 Format BDD/Gherkin pour US

Chaque User Story DOIT avoir des scénarios Gherkin:

```gherkin
Feature: Vector similarity search

  Scenario: Find similar documents
    Given a collection "docs" with 100 documents
    And each document has a 384-dim embedding
    When I search with a query embedding
    Then I get top-k results sorted by similarity
    And each result has score between 0 and 1

  Scenario: Filter by metadata
    Given a collection with documents tagged by category
    When I search with filter category="tech"
    Then only documents with category="tech" are returned
```

## ✅ Checklist Tests par Type de Changement

### Nouveau Module/Feature

- [ ] Tests unitaires pour chaque fonction publique
- [ ] Tests d'intégration pour le use case principal
- [ ] Scénarios Gherkin dans la US
- [ ] Mise à jour de la couverture (target: 85%)

### Bug Fix

- [ ] Test de régression reproduisant le bug AVANT fix
- [ ] Vérification que le test passe APRÈS fix
- [ ] Test edge cases similaires identifiés

### API Publique Modifiée

- [ ] Tests d'intégration couvrant le changement
- [ ] Tests SDK (Python, WASM, TS) mis à jour
- [ ] Tests E2E API HTTP si endpoint concerné

## 📁 Structure des Tests

```
crates/velesdb-core/
├── src/
│   ├── module.rs
│   └── module_tests.rs       ← Tests unitaires (même dossier)
└── tests/
    ├── integration_scenarios.rs   ← Scénarios métier
    └── use_cases_integration_tests.rs  ← 10 use cases documentés

crates/velesdb-server/
└── tests/
    ├── api_integration.rs    ← Tests HTTP E2E
    └── test_metrics_feature.rs
```

## 🔄 Workflow TDD

```
1. RED:   Écrire le test qui échoue (scénario Gherkin → code test)
2. GREEN: Implémenter le minimum pour passer
3. REFACTOR: Nettoyer en gardant tests verts
```

## 📈 Métriques de Couverture

| Cible | Seuil |
|-------|-------|
| **Global** | ≥ 85% |
| **Modules critiques** | ≥ 90% |
| **API publique** | 100% |

### Commande de couverture

```powershell
cargo llvm-cov --workspace --html
```

## ⚠️ Anti-Patterns à Éviter

| ❌ Anti-Pattern | ✅ Bonne Pratique |
|-----------------|-------------------|
| Tests dépendant de l'ordre | Tests isolés et indépendants |
| Tests avec données hardcodées | Fixtures/factories générées |
| Tests flaky (aléatoires) | Tests déterministes |
| Tests trop longs (>1s) | Parallélisation ou mock |
| Tests sans assertions | Au moins 1 assertion par test |

## 🏃 Exécution des Tests

```powershell
# Tous les tests
cargo test --workspace

# Tests d'intégration uniquement
cargo test --test integration_scenarios
cargo test --test use_cases_integration_tests

# Tests E2E API
cargo test --package velesdb-server --test api_integration

# Tests avec logs
RUST_LOG=debug cargo test -- --nocapture
```
