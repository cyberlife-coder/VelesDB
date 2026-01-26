# Impact Matrix: US-005 Quoted Identifiers

## 📊 Dependency Graph

```
grammar.pest (identifier rule)
    │
    ├── parser/mod.rs (extract_identifier)
    │       │
    │       ├── parser/select.rs
    │       │   ├── FROM table name ✅
    │       │   ├── JOIN table name ✅
    │       │   ├── JOIN alias ✅
    │       │   ├── JOIN USING columns ✅
    │       │   ├── ORDER BY field ✅
    │       │   ├── GROUP BY columns ✅
    │       │   ├── Aggregation alias ✅
    │       │   ├── Column alias ✅
    │       │   ├── column_name (atomic) ✅ (strip_quotes_from_column_name)
    │       │   └── FUSION options ✅
    │       │
    │       ├── parser/conditions.rs
    │       │   ├── Compare expression ✅
    │       │   ├── IS NULL expression ✅
    │       │   ├── MATCH expression ✅ (Kaizen fix)
    │       │   ├── IN expression ✅ (Kaizen fix)
    │       │   ├── BETWEEN expression ✅ (Kaizen fix)
    │       │   └── LIKE/ILIKE expression ✅ (Kaizen fix)
    │       │
    │       └── parser/values.rs
    │           └── WITH clause identifiers ✅
    │
    └── DOWNSTREAM CONSUMERS
        ├── collection/search/query/mod.rs (execute queries)
        ├── collection/search/query/aggregation.rs
        ├── filter/conversion.rs (condition → filter)
        ├── velesdb-server (REST API)
        ├── velesdb-cli (REPL)
        ├── velesdb-python (PyO3 bindings)
        ├── velesdb-wasm (WASM bindings)
        ├── velesdb-mobile (mobile SDK)
        └── tauri-plugin-velesdb
```

## 🔄 Data Flow

```
User Query String
    │
    ▼
grammar.pest (PEST parser)
    │
    ▼
Parser::parse() → Query AST
    │
    ├── identifier fields contain CLEAN names (no quotes)
    │   Thanks to extract_identifier() and strip_quotes_from_column_name()
    │
    ▼
Collection::execute_velesql()
    │
    ├── Validation (validation.rs)
    ├── Filter conversion (filter/conversion.rs)
    ├── Query execution (query/mod.rs)
    └── Results
```

## 📦 Crates Impactés

| Crate | Impact | Raison |
|-------|--------|--------|
| `velesdb-core` | ✅ Direct | Parser modifié |
| `velesdb-server` | 🟢 Indirect | Consomme le parser |
| `velesdb-cli` | 🟢 Indirect | Consomme le parser |
| `velesdb-python` | 🟢 Indirect | Consomme velesdb-core |
| `velesdb-wasm` | 🟢 Indirect | Consomme velesdb-core |
| `velesdb-mobile` | 🟢 Indirect | Consomme velesdb-core |
| `tauri-plugin` | 🟢 Indirect | Consomme velesdb-core |

## 🔮 Vision Long Terme

### Évolutions Futures Impactées

| Feature Future | Impact US-005 | Status |
|----------------|---------------|--------|
| **EPIC-039 Correlated Subqueries** | ✅ Ready | Identifiers supportés |
| **EPIC-038 Temporal Functions** | ✅ Ready | Column names supportés |
| **SQL Standard Compliance** | ✅ Enhanced | Double-quote = standard |
| **Dynamic Schema** | ✅ Ready | Any field name possible |
| **LLM Query Generation** | ✅ Critical | LLMs peuvent générer des noms réservés |

### Cas d'Usage Débloqués

1. **LLM-generated queries**: Les LLMs génèrent souvent des colonnes comme `order`, `select`, `from`
2. **User-defined metadata**: Les utilisateurs peuvent utiliser n'importe quel nom de champ
3. **Migration depuis autres DBs**: Compatibilité avec PostgreSQL/MySQL schemas
4. **Agent memory**: Champs comme `action`, `type`, `value` sont maintenant safe

## ⚠️ Limitations Connues

### Non Supporté (By Design)

| Feature | Raison | Workaround |
|---------|--------|------------|
| `similarity_field` | Rule atomique spéciale | Utiliser `vector` uniquement |
| Nested dots in quotes | Complexité excessive | `"a.b"` → `a.b` (déjà supporté) |

### Règles Atomiques

Les règles marquées `@` (atomic) dans grammar.pest ne décomposent pas leurs inner rules:
- `similarity_field` - Intentionnel, limité à `vector`
- `column_name` - ✅ Géré via `strip_quotes_from_column_name()`

## 🧪 Couverture de Tests

| Contexte | Tests | Status |
|----------|-------|--------|
| FROM clause | `test_parse_backtick_identifier_from` | ✅ |
| WHERE compare | `test_parse_backtick_identifier_where` | ✅ |
| WHERE MATCH | `test_parse_quoted_identifier_match` | ✅ |
| WHERE IN | `test_parse_quoted_identifier_in` | ✅ |
| WHERE BETWEEN | `test_parse_quoted_identifier_between` | ✅ |
| WHERE LIKE | `test_parse_quoted_identifier_like` | ✅ |
| WHERE ILIKE | `test_parse_quoted_identifier_ilike` | ✅ |
| ORDER BY | `test_parse_quoted_identifier_order_by` | ✅ |
| GROUP BY | `test_parse_quoted_identifier_group_by` | ✅ |
| SELECT column | `test_parse_quoted_identifier_select_column` | ✅ |
| Column alias | `test_parse_quoted_identifier_column_alias` | ✅ |
| Reserved keywords (24) | `test_parse_reserved_keywords_as_identifiers` | ✅ |
| Mixed quotes | `test_parse_mixed_quoted_identifiers` | ✅ |
| Escaped quotes | `test_parse_doublequote_escaped_quote` | ✅ |

**Total: 15 tests dédiés + couverture indirecte via 1692 tests**

## 📋 Checklist Maintenance Future

Lors d'ajout de nouvelles clauses VelesQL:

- [ ] Si parsing d'identifier → utiliser `extract_identifier()`
- [ ] Si atomic rule avec identifier → implémenter strip_quotes
- [ ] Ajouter test avec quoted identifier
- [ ] Documenter dans cette matrice
