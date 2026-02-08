---
description: Appliquer lors de modifications au plugin Tauri VelesDB
trigger: model_decision
globs:
  - "crates/tauri-plugin-velesdb/**"
---

# Règles Plugin Tauri VelesDB

## Architecture Graph API

⚠️ **Le graph n'est PAS accessible via `collection.graph()`**

Le graph utilise un pattern `GraphService` séparé avec `EdgeStore` par collection.

Référence: `velesdb-server/src/handlers/graph/service.rs`

```rust
// Pattern correct pour exposer le graph
pub struct GraphService {
    stores: Arc<RwLock<HashMap<String, Arc<RwLock<EdgeStore>>>>>,
}
```

## Checklist Modification Plugin

### Ajout d'une nouvelle commande

1. **Types d'abord** (`types.rs`)
   - Définir Request struct avec `#[serde(rename_all = "camelCase")]`
   - Définir Response struct si nécessaire
   - Fonctions default si champs optionnels

2. **Command** (`commands.rs`)
   - Importer les types depuis `crate::types`
   - Implémenter avec `#[command]` macro
   - Pattern: `state.with_db(|db| { ... })`

3. **Export** (`lib.rs`)
   - Ajouter dans `invoke_handler(tauri::generate_handler![...])`

4. **Tests** (`commands_tests.rs`)
   - Test unitaire pour la nouvelle commande

### Validation

```powershell
cargo check -p tauri-plugin-velesdb
cargo test -p tauri-plugin-velesdb
cargo clippy -p tauri-plugin-velesdb -- -D warnings
```

## Dépendances Communes

- `dirs` - Pour `get_app_data_dir()` cross-platform
- `velesdb-core` - Core library (path local)
- `tauri` 2.0 - Framework Tauri

## API JavaScript (Frontend)

```javascript
import { invoke } from '@tauri-apps/api/core';

// Pattern d'appel
const result = await invoke('plugin:velesdb|command_name', {
  request: { /* params */ }
});
```
