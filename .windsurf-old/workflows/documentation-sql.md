# /documentation-sql

Workflow pour documenter les changements VelesQL.

## Étape 1: Identifier les changements

// turbo
```powershell
git log --oneline -10 | Select-String "velesql|VelesQL|EPIC-040"
```

## Étape 2: Vérifier la grammaire

Lire `crates/velesdb-core/src/velesql/grammar.pest` pour identifier les syntaxes supportées.

## Étape 3: Mettre à jour VELESQL_SPEC.md

Fichier: `docs/reference/VELESQL_SPEC.md`

Sections à vérifier:
- Grammar (BNF)
- Data Types
- Operators
- Clauses (GROUP BY, HAVING, ORDER BY, JOIN, etc.)
- Set Operations (UNION, INTERSECT, EXCEPT)
- Limitations
- Examples

## Étape 4: Mettre à jour ARCHITECTURE.md

Fichier: `docs/reference/ARCHITECTURE.md`

- VelesQL Parser section
- Query Flow diagrams
- AST structure

## Étape 5: Mettre à jour README.md

- API Reference section (VelesQL endpoint)
- Quick examples

## Étape 6: Mettre à jour CHANGELOG.md

Ajouter les nouvelles features VelesQL dans la section [Unreleased].

## Étape 7: Vérifier les tests

// turbo
```powershell
cargo test --package velesdb-core velesql -- --nocapture 2>&1 | Select-Object -Last 20
```

## Étape 8: Commit

```powershell
git add docs/ README.md CHANGELOG.md
git commit -m "docs(velesql): update documentation for VelesQL changes"
```

## Checklist

- [ ] VELESQL_SPEC.md à jour
- [ ] ARCHITECTURE.md à jour
- [ ] README.md à jour
- [ ] CHANGELOG.md à jour
- [ ] Exemples testés
- [ ] Commit effectué
