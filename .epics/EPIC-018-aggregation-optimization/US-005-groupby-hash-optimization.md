# US-005: GROUP BY Hash Optimization

## Description

Remplacer la sérialisation JSON des clés de groupe par un hashing direct pour améliorer les performances du GROUP BY.

## Problème Actuel

```rust
// aggregation.rs:365-373 - Sérialisation JSON pour chaque groupe!
fn extract_group_key(payload: Option<&serde_json::Value>, group_by_columns: &[String]) -> String {
    let values: Vec<serde_json::Value> = ...;
    serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
}
```

Chaque row fait une sérialisation JSON complète pour la clé de groupe.

## Critères d'Acceptation

- [ ] Hashing direct sans sérialisation JSON
- [ ] Gestion correcte des types (String, Number, Null)
- [ ] Collision handling via equals check si hash identique
- [ ] Benchmark prouvant 20-40% d'amélioration sur GROUP BY
- [ ] Tous les tests GROUP BY existants passent

## Solution Proposée

```rust
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

/// Clé de groupe avec hash pré-calculé
#[derive(Clone)]
struct GroupKey {
    values: Vec<serde_json::Value>,
    hash: u64,
}

impl GroupKey {
    fn new(values: Vec<serde_json::Value>) -> Self {
        let hash = Self::compute_hash(&values);
        Self { values, hash }
    }
    
    fn compute_hash(values: &[serde_json::Value]) -> u64 {
        let mut hasher = FxHasher::default();
        for v in values {
            match v {
                Value::String(s) => {
                    0u8.hash(&mut hasher);
                    s.hash(&mut hasher);
                }
                Value::Number(n) => {
                    1u8.hash(&mut hasher);
                    n.as_f64().map(|f| f.to_bits().hash(&mut hasher));
                }
                Value::Bool(b) => {
                    2u8.hash(&mut hasher);
                    b.hash(&mut hasher);
                }
                Value::Null => {
                    3u8.hash(&mut hasher);
                }
                _ => {
                    4u8.hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
}

impl Hash for GroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.values == other.values
    }
}
impl Eq for GroupKey {}
```

## Tests Requis

```rust
#[test]
fn test_groupkey_hash_consistency() {
    // Même valeurs = même hash
}

#[test]
fn test_groupkey_collision_handling() {
    // Hash collision avec valeurs différentes
}

#[test]
fn test_groupby_performance_100k() {
    // Benchmark avec 100K rows, 5 catégories
}
```

## Analyse Rust-Specific

### Ownership & Borrowing
- `GroupKey` possède ses valeurs (clone nécessaire)
- Alternative: `GroupKey<'a>` avec références (plus complexe)

### Types & Traits
- Implémenter `Hash`, `Eq`, `PartialEq` pour `GroupKey`
- `Clone` pour la sérialisation du résultat

### Error Handling
- Pas de panic possible (hash toujours calculable)

### Concurrence
- `GroupKey` doit être `Send + Sync`
- FxHasher est thread-safe
