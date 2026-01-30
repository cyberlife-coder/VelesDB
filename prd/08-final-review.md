# Phase 8 - Final Review & Auto-correction (45 min)

## Analyse statique

- [ ] Exécuter `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- [ ] Si des warnings apparaissent, les corriger un par un
- [ ] Exécuter `cargo clippy -p velesdb-core -- -W clippy::unwrap_used -W clippy::expect_used` pour détecter les unwrap non justifiés
- [ ] Pour chaque unwrap trouvé, soit le remplacer par `?` soit ajouter un commentaire justifiant son usage

## Audit sécurité

- [ ] Exécuter `cargo deny check` pour vérifier les dépendances
- [ ] Si des advisories apparaissent, mettre à jour les dépendances concernées dans Cargo.toml
- [ ] Exécuter `cargo audit` si disponible pour double-vérification
- [ ] Vérifier que deny.toml est à jour avec les bonnes policies

## Formatage

- [ ] Exécuter `cargo fmt --all --check` pour vérifier le formatage
- [ ] Si des fichiers ne sont pas formatés, exécuter `cargo fmt --all`
- [ ] Vérifier que rustfmt.toml contient les règles du projet

## Tests complets

- [ ] Exécuter `cargo test --workspace` pour tous les tests
- [ ] Si des tests échouent, les corriger avant de continuer
- [ ] Exécuter `cargo test --workspace --release` pour tester en mode release
- [ ] Exécuter `cargo test --workspace -- --ignored` pour les tests ignorés si applicable

## Build release

- [ ] Exécuter `cargo build --release` pour vérifier la compilation release
- [ ] Vérifier qu'aucun warning n'apparaît pendant la compilation
- [ ] Vérifier que les binaires sont générés dans `target/release/`

## Détection de duplication

- [ ] Installer `cargo-machete` si pas présent: `cargo install cargo-machete`
- [ ] Exécuter `cargo machete` pour détecter les dépendances inutilisées
- [ ] Supprimer les dépendances inutilisées de Cargo.toml
- [ ] Utiliser un outil comme `jscpd` ou revue manuelle pour détecter duplication de code >10 lignes
- [ ] Si duplication trouvée, factoriser en fonction/module partagé

## Couverture de tests

- [ ] Installer llvm-cov si pas présent: `cargo install cargo-llvm-cov`
- [ ] Exécuter `cargo llvm-cov --workspace --html`
- [ ] Ouvrir `target/llvm-cov/html/index.html` pour voir le rapport
- [ ] Vérifier que la couverture globale est >= 80%
- [ ] Identifier les fichiers avec couverture < 70% et ajouter des tests si nécessaire

## Validation SOLID/DRY

- [ ] Vérifier Single Responsibility: chaque module/struct a une seule responsabilité
- [ ] Vérifier Open/Closed: le code est extensible sans modification
- [ ] Vérifier Liskov Substitution: les traits sont correctement implémentés
- [ ] Vérifier Interface Segregation: pas d'interfaces trop larges
- [ ] Vérifier Dependency Inversion: dépendances sur abstractions (traits)
- [ ] Vérifier DRY: pas de code dupliqué (utiliser les helpers créés en Phase 1)

## Auto-correction itérative

- [ ] Si des problèmes sont trouvés lors des vérifications ci-dessus, les corriger
- [ ] Ré-exécuter les vérifications jusqu'à ce que tout passe
- [ ] Maximum 5 itérations - si problèmes persistent, documenter et escalader

## Validation finale

- [ ] Exécuter `.\scripts\local-ci.ps1` si disponible pour validation complète
- [ ] Vérifier que tous les hooks pre-commit passent: `git commit --dry-run`
- [ ] Créer un commit de synthèse: `git add -A && git commit -m "refactor(core): complete refactoring per PRD phases 1-8"`
