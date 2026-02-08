# /wasm-feature

Guide pour implémenter une feature dans le crate velesdb-wasm.

## Étape 1: Analyse des Patterns WASM

Avant de coder, vérifier les patterns appris:

### Checklist Patterns
- [ ] **Async constructor?** → Utiliser `new()` sync + `init()` async séparés
- [ ] **Méthodes internes?** → Impl block séparé sans `#[wasm_bindgen]`
- [ ] **Persistence IndexedDB?** → Namespaced keys pattern
- [ ] **Sérialisation?** → `serde_wasm_bindgen::to_value/from_value`

## Étape 2: Structure Module

```rust
// Fichier: crates/velesdb-wasm/src/my_feature.rs

use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

/// Public struct exposed to JavaScript.
#[wasm_bindgen]
pub struct MyFeature {
    // fields...
}

#[wasm_bindgen]
impl MyFeature {
    /// Sync constructor (required pattern for async init).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { /* ... */ }
    }

    /// Async initialization (call after new()).
    #[wasm_bindgen]
    pub async fn init(&mut self) -> Result<(), JsValue> {
        // Async operations here
        Ok(())
    }

    // Other #[wasm_bindgen] methods...
}

/// Internal methods (not exposed to WASM).
impl MyFeature {
    pub(crate) fn internal_method(&self) -> Vec<SomeType> {
        // Internal logic
    }
}
```

## Étape 3: Ajouter au lib.rs

```rust
// crates/velesdb-wasm/src/lib.rs
mod my_feature;
pub use my_feature::MyFeature;
```

## Étape 4: Vérifier Cargo.toml

Si nouvelles features web-sys nécessaires:

```toml
[dependencies.web-sys]
features = [
    # Features IndexedDB (déjà présentes)
    "IdbFactory",
    "IdbDatabase",
    "IdbObjectStore",
    # Ajouter selon besoin:
    # "Worker",
    # "MessageEvent",
]
```

## Étape 5: Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_my_feature_creation() {
        let feature = MyFeature::new();
        // assertions...
    }

    #[wasm_bindgen_test]
    async fn test_my_feature_async() {
        let mut feature = MyFeature::new();
        feature.init().await.expect("init should succeed");
        // assertions...
    }
}
```

## Étape 6: Validation

// turbo
```powershell
cargo check -p velesdb-wasm
cargo test -p velesdb-wasm
cargo clippy -p velesdb-wasm -- -D warnings
```

## Étape 7: Test Browser (optionnel)

```powershell
wasm-pack test --headless --chrome crates/velesdb-wasm
```

## Anti-Patterns à Éviter

| ❌ Anti-Pattern | ✅ Correct |
|----------------|-----------|
| `async fn new()` constructor | `fn new()` + `async fn init()` |
| Méthode interne dans `#[wasm_bindgen] impl` | Impl block séparé |
| `unwrap()` dans code WASM | `Result<T, JsValue>` |
| Clés IndexedDB sans namespace | `format!("{graph_name}:{id}")` |

## Références Memories

- "WASM: Éviter les constructeurs async"
- "WASM: Méthodes internes non exposées"
- "IndexedDB: Pattern Multi-Graph"
