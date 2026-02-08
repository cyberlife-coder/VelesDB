# Checklist @implement-epic

## Avant de Lancer

- [ ] EPIC existe dans `.epics/EPIC-XXX-*/`
- [ ] `EPIC.md` contient la liste des US
- [ ] `progress.md` existe et est à jour
- [ ] Branche `develop` à jour (`git pull origin develop`)

## Pour Chaque US

### Démarrage
- [ ] Branche créée : `feature/EPIC-XXX-US-YYY`
- [ ] Critères d'acceptation lus et compris
- [ ] `progress.md` mis à jour → IN PROGRESS

### Implémentation TDD
- [ ] Tests écrits AVANT le code
- [ ] Tests échouent (RED)
- [ ] Code minimum implémenté
- [ ] Tests passent (GREEN)
- [ ] Code refactoré (REFACTOR)

### Qualité (/fou-furieux)
- [ ] Debug : `cargo test` passe
- [ ] Code Smells : fichiers < 500 lignes, fonctions < 30 lignes
- [ ] Sécurité : pas de `unsafe` non documenté, `cargo deny check` OK
- [ ] Performance : pas d'allocation dans boucles critiques
- [ ] Multithreading : lock ordering respecté

### Validation (/pre-commit)
- [ ] `cargo fmt --all` → pas de changements
- [ ] `cargo clippy -- -D warnings` → 0 warnings
- [ ] `cargo test --workspace` → 100% GREEN
- [ ] `cargo deny check` → 0 vulnérabilités

### Finalisation
- [ ] Commit avec message formaté : `type(scope): desc [EPIC-XXX/US-YYY]`
- [ ] `progress.md` → DONE
- [ ] `US-YYY.md` → DONE

## Clôture EPIC

- [ ] Toutes les US = DONE
- [ ] Tests finaux passent
- [ ] Dossier renommé → `EPIC-XXX-nom-done`
- [ ] Commit de clôture créé

## Résumé Final

- [ ] Statistiques affichées
- [ ] Prochaines actions proposées (PR, ecosystem-sync)
