---
description: Ajouter un nouvel endpoint API REST avec axum
---

# Workflow : Nouvel Endpoint API

## 1. Définition

1. Définir le endpoint :
   - Méthode HTTP (GET, POST, PUT, DELETE)
   - Path (ex: `/collections/:name/search`)
   - Request body (si applicable)
   - Response body

## 2. Types Request/Response

1. Créer les structs dans `main.rs` ou module dédié :
   ```rust
   #[derive(Debug, Deserialize)]
   struct MyRequest {
       field: String,
       #[serde(default)]
       optional_field: Option<i32>,
   }

   #[derive(Debug, Serialize)]
   struct MyResponse {
       result: Vec<Item>,
   }
   ```

## 3. Handler

1. Implémenter le handler :
   ```rust
   async fn my_handler(
       State(state): State<Arc<AppState>>,
       Path(name): Path<String>,
       Json(req): Json<MyRequest>,
   ) -> impl IntoResponse {
       // Validation
       // Logic
       // Response
   }
   ```

2. Gérer les erreurs proprement :
   ```rust
   match result {
       Ok(data) => Json(data).into_response(),
       Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse {
           error: e.to_string(),
       })).into_response(),
   }
   ```

## 4. Route

1. Ajouter la route dans le Router :
   ```rust
   let app = Router::new()
       .route("/my/endpoint", post(my_handler))
       // ...
   ```

## 5. Tests

1. Écrire un test d'intégration (optionnel mais recommandé) :
   ```rust
   #[tokio::test]
   async fn test_my_endpoint() {
       // Setup
       // Call endpoint
       // Assert response
   }
   ```

2. Vérifier que ça compile :
// turbo
```powershell
cargo check
```

## 6. Documentation

1. Mettre à jour `docs/api-reference.md` avec le nouvel endpoint
2. Ajouter des exemples curl si utile

## 7. Validation

// turbo
```powershell
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```
