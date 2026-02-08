# Checklist @merge-epic-prs

## Avant de Lancer

- [ ] Toutes les PRs de l'EPIC sont ouvertes
- [ ] `develop` local est à jour
- [ ] Pas de changements locaux non commités

## Pour Chaque PR

### Pré-merge
- [ ] PR mergeable (pas de conflits GitHub)
- [ ] Base correcte (develop ou feature parent)
- [ ] CI passée (si activée)

### Rebase
- [ ] Fetch origin develop
- [ ] Rebase sur develop
- [ ] Conflits résolus (si applicable)

### Validation
- [ ] `cargo fmt --check` OK
- [ ] `cargo clippy -- -D warnings` OK
- [ ] `cargo test --workspace` OK

### Merge
- [ ] Push force-with-lease
- [ ] Merge squash
- [ ] Branche supprimée

## Post-Merge

- [ ] develop local mis à jour
- [ ] Prochaine PR identifiée
- [ ] Aucune régression introduite
