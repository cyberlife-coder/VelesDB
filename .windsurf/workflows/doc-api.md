---
description: Documenter l'API REST VelesDB
---

# Workflow : Documentation API

## 1. Identifier les changements

1. Lister les endpoints modifiés/ajoutés
2. Vérifier les types Request/Response

## 2. Mettre à jour api-reference.md

1. Ouvrir `docs/api-reference.md`
2. Ajouter/modifier l'endpoint :

```markdown
### POST /collections/:name/search

Recherche les vecteurs les plus proches.

**Request Body:**
```json
{
  "vector": [0.1, 0.2, ...],
  "top_k": 10
}
```

**Response:**
```json
{
  "results": [
    {"id": 1, "score": 0.95, "payload": {...}},
    ...
  ]
}
```

**Errors:**
- `404` - Collection not found
- `400` - Invalid vector dimension
```

## 3. Exemples curl

Ajouter des exemples pratiques :

```bash
# Créer une collection
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "test", "dimension": 768, "metric": "cosine"}'

# Recherche
curl -X POST http://localhost:8080/collections/test/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 5}'
```

## 4. Générer la doc Rust

// turbo
```powershell
cargo doc --all-features --no-deps --open
```

## 5. Vérifier la cohérence

- Les types dans la doc correspondent au code
- Les exemples sont testables
- Les erreurs sont documentées
