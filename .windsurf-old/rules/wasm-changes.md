---
trigger: glob
glob: crates/velesdb-wasm/**/*.rs
---

# Règles WASM (velesdb-wasm)

## Patterns Obligatoires

### 1. Async Constructor Interdit
```rust
// ❌ JAMAIS de constructeur async
#[wasm_bindgen(constructor)]
pub async fn new() -> Result<Self, JsValue> { ... }

// ✅ Toujours: new() sync + init() async
pub fn new() -> Self { ... }
pub async fn init(&mut self) -> Result<(), JsValue> { ... }
```

### 2. Méthodes Internes
Pour exposer des méthodes uniquement à Rust (pas JavaScript):
```rust
/// Internal methods (not exposed to WASM).
impl MyStruct {
    pub(crate) fn method_internal(&self) -> T { ... }
}
```

### 3. Sérialisation
Utiliser `serde_wasm_bindgen` pour conversion JS ↔ Rust:
```rust
let js_value = serde_wasm_bindgen::to_value(&rust_struct)?;
let rust_struct: MyType = serde_wasm_bindgen::from_value(js_value)?;
```

### 4. IndexedDB Persistence
Namespaced keys pour multi-entité:
```rust
let key = format!("{namespace}:{id}");
```

## Validation Avant Commit

```powershell
cargo check -p velesdb-wasm
cargo test -p velesdb-wasm
cargo clippy -p velesdb-wasm -- -D warnings
```

## Fichiers Critiques

| Fichier | Responsabilité |
|---------|----------------|
| `graph.rs` | GraphStore, GraphNode, GraphEdge |
| `graph_persistence.rs` | IndexedDB persistence |
| `lib.rs` | Exports publics |
| `persistence.rs` | VectorStore IndexedDB |

## Tests Browser

```powershell
wasm-pack test --headless --chrome crates/velesdb-wasm
```

## Checklist PR

- [ ] Pas de constructeur async
- [ ] Méthodes internes dans impl block séparé
- [ ] `Result<T, JsValue>` pour toutes les erreurs
- [ ] Tests avec `#[wasm_bindgen_test]`
- [ ] Documentation backticks pour Clippy (`IndexedDB` → `\`IndexedDB\``)
