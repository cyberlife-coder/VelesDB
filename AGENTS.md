# VelesDB Core - Instructions Cascade

## 📜 Licence & Contact

- **Licence**: Elastic License 2.0 (ELv2) - Source Available
- **Email**: contact@wiscale.fr
- **Website**: https://velesdb.com
- **Repository**: https://github.com/cyberlife-coder/VelesDB

## 🏗️ Architecture

- Projet Rust monorepo (workspace Cargo)
- `velesdb-premium` ÉTEND ce projet, jamais l'inverse
- EPICs et US stockées localement dans `.epics/`

## 📁 Structure Projet

```
velesdb-core/
├── .epics/                    # Gestion projet (EPICs, US)
├── .windsurf/                 # Configuration Cascade
├── crates/                    # Crates Rust
│   ├── velesdb-core/          # Engine principal
│   ├── velesdb-server/        # API HTTP
│   ├── velesdb-cli/           # CLI
│   └── ...
├── docs/                      # Documentation
└── benchmarks/                # Benchmarks performance
```

## 📊 Gestion Projet (EPICs/US)

- EPICs: `.epics/EPIC-XXX-nom/EPIC.md`
- User Stories: `.epics/EPIC-XXX-nom/US-YYY-nom.md`
- Suivi: `.epics/EPIC-XXX-nom/progress.md`
- Templates: `.epics/_templates/`

## 🌿 Stratégie de Branching

<git_workflow>
### Branches Principales
- `main`: production, toujours stable et déployable
- `develop`: intégration, base pour les features

### Branches de Travail
- `feature/EPIC-XXX-US-YYY-description`: nouvelles fonctionnalités (depuis develop)
- `bugfix/XXX-description`: corrections (depuis develop)
- `hotfix/XXX-description`: correctifs urgents prod (depuis main)

### Règles Strictes
1. JAMAIS de commit direct sur `main` ou `develop`
2. Features créées depuis `develop`
3. PR feature → develop (après /fou-furieux complet)
4. PR develop → main (release uniquement)
5. Hotfix: main → hotfix → PR vers main ET develop

### Nommage
- `feature/EPIC-001-US-002-audit-viewer`
- `bugfix/fix-deadlock-graph`
- `hotfix/security-patch-auth`

### Commits
Format: `type(scope): description [EPIC-XXX/US-YYY]`
Types: feat, fix, docs, refactor, test, perf, chore
</git_workflow>

## 🛠️ Standards de Développement

<dev_standards>
### TDD Obligatoire (STRICT)
- **Toute refactorisation** = cycle TDD complet:
  1. **AVANT**: Vérifier tests GREEN (baseline)
  2. **PENDANT**: Refactorer sans casser les tests
  3. **APRÈS**: Vérifier tests GREEN + clippy clean
- Tests AVANT implémentation (RED → GREEN → REFACTOR)
- **JAMAIS** de refactoring sans tests passants avant/après

### Tests dans Fichiers SÉPARÉS (OBLIGATOIRE)
- **Nouveaux fichiers**: tests dans `module_tests.rs` ou `tests/module.rs`
- **Fichiers existants**: extraire tests inline vers fichiers séparés
- **Exception unique**: tests nécessitant accès aux champs privés (`#[cfg(test)]` inline)
- Structure: `module.rs` (code) + `module_tests.rs` (tests)
- Nommage: `test_[fonction]_[scenario]_[resultat_attendu]`

### Modularité & Taille
- Fichier < 500 lignes (sinon découper immédiatement)
- Fonction < 30 lignes
- Un module = une responsabilité unique

### Anti Sur-ingénierie
- Solution la plus SIMPLE qui répond au besoin
- Pas d'abstraction prématurée
- YAGNI: pas de code "au cas où"

### Qualité Code
- DRY: factoriser toute duplication (>3 occurrences)
- SOLID: une responsabilité par module/fonction
- Nommage explicite et auto-documentant
- Pas de `unwrap()` en production (utiliser `?` ou `expect`)
</dev_standards>

## 🔄 Refactoring (Méthode Martin Fowler)

Pour tout refactoring de fichier > 500 lignes, utiliser `/refactor-module`:

<refactoring_principles>
### Principes Fondamentaux
1. **Tiny Steps**: Chaque changement minime et vérifiable
2. **Tests GREEN avant/après**: Ne jamais refactorer sans tests passants
3. **Commits séparés**: Moves/renames SÉPARÉS des edits logiques
4. **"Make the change easy, then make the easy change"** (Kent Beck)

### Workflow d'Extraction
1. Baseline tests GREEN
2. Analyser structure et dépendances
3. Créer module vide + `mod module;`
4. Déplacer UNE fonction à la fois + `cargo check`
5. Un commit par déplacement
6. Validation finale `/fou-furieux`

### Cas PyO3
- `#[pyclass]` dans sous-module du même crate
- Re-export pattern: `pub use module::Struct;`
</refactoring_principles>

## 🔬 Recherche & Innovation

Avant toute optimisation performance ou algorithme complexe:
1. Rechercher les derniers algorithmes (internet, arXiv, papers 2024-2025)
2. Documenter les alternatives dans `.research/`
3. Benchmarker avant/après implémentation

## 🔴 Cycle "Fous Furieux"

Après chaque implémentation, boucle de validation:
1. **Debug**: tests passent, pas d'erreurs runtime
2. **Code Smells**: fichiers <500 lignes, clean code
3. **Sécurité**: pas de vulnérabilités, entrées validées
4. **Performance**: pas de régressions, O(n) acceptable
5. **Multithreading**: pas de deadlock, lock ordering respecté

BOUCLER jusqu'à tous les contrôles ✅

## 🔍 Review Experts (EPIC Completion)

Avant merge d'une EPIC complète, lancer `/expert-review` pour validation multi-disciplinaire:

| Expert | Focus | Critères |
|--------|-------|----------|
| 🔧 **Architecte** | Structure, modularité | Fichiers <500L, SOLID, DRY |
| 🛡️ **SecDev** | Sécurité | unsafe documenté, pas unwrap prod, cargo deny |
| 🧪 **QA** | Tests | Couverture >80%, edge cases |
| ⚡ **Perf** | Performance | Latences objectifs, benchmarks |

### Workflow de Review

```
1. /fou-furieux complet
2. /pre-commit validé
3. /expert-review multi-experts
4. Commit final + push
5. PR vers develop
```

### Verdict

| Verdict | Action |
|---------|--------|
| ✅ APPROUVÉ | Merge autorisé |
| ⚠️ À AMÉLIORER | Corrections mineures avant merge |
| ❌ REJETÉ | Refactoring requis |

## 💰 Optimisation Coûts GitHub Actions

**Principe**: Validation locale OBLIGATOIRE avant push vers origin.

### Setup (une seule fois)
```powershell
.\scripts\setup-hooks.ps1
```

### Workflow de développement
```
1. Développer sur branche feature
2. git commit (pre-commit valide fmt/clippy/tests)
3. /local-ci                    # Validation complète
4. git push origin <branch>     # pre-push valide tout
```

### Scripts disponibles
```powershell
.\scripts\local-ci.ps1          # Validation complète
.\scripts\local-ci.ps1 -Quick   # Mode rapide (fmt + clippy)
```

### CI GitHub Actions
- **Déclenché sur**: push main/develop uniquement
- **PR désactivées**: économie ~80% des minutes
- **Path filtering**: crates/**, Cargo.toml, Cargo.lock
- **Coverage/Benchmarks**: main uniquement

## 🔧 Commandes Essentielles

```bash
cargo fmt --all                              # Formatage
cargo clippy -- -D warnings                  # Linting
cargo test --workspace                       # Tests
cargo deny check                             # Audit sécurité
cargo bench                                  # Benchmarks
.\scripts\local-ci.ps1                       # CI local complet
```

## 🧩 Écosystème & Propagation (OBLIGATOIRE)

> **Règle fondamentale**: Toute feature Core DOIT être propagée dans l'ensemble de l'écosystème.

### Composants de l'écosystème

| Composant | Type | Chemin |
|-----------|------|--------|
| velesdb-core | Engine | `crates/velesdb-core/` |
| velesdb-server | API HTTP | `crates/velesdb-server/` |
| velesdb-cli | CLI | `crates/velesdb-cli/` |
| velesdb-python | SDK Python | `crates/velesdb-python/` |
| velesdb-wasm | SDK WASM | `crates/velesdb-wasm/` |
| velesdb-mobile | SDK Mobile | `crates/velesdb-mobile/` |
| tauri-plugin-velesdb | Plugin Tauri | `crates/tauri-plugin-velesdb/` |
| TypeScript SDK | SDK TS | `sdks/typescript/` |
| LangChain | Intégration | `integrations/langchain/` |
| LlamaIndex | Intégration | `integrations/llamaindex/` |

### Workflow de propagation

Après toute feature Core:
1. Exécuter `/ecosystem-sync EPIC-XXX`
2. Créer `ecosystem-sync.md` dans le dossier EPIC
3. Créer US de propagation pour chaque SDK impacté
4. Mettre à jour matrice de parité dans EPIC-016

### Checklist de propagation

```markdown
| SDK | Status | PR | Notes |
|-----|--------|-----|-------|
| velesdb-server | 🔴 TODO | - | Endpoint API |
| velesdb-python | 🔴 TODO | - | PyO3 bindings |
| velesdb-wasm | 🔴 TODO | - | wasm-bindgen |
| velesdb-mobile | 🔴 TODO | - | UniFFI |
| sdks/typescript | 🔴 TODO | - | HTTP client |
| tauri-plugin | 🔴 TODO | - | Tauri commands |
| langchain | 🔴 TODO | - | Retriever |
| llamaindex | 🔴 TODO | - | VectorStore |
| velesdb-cli | 🔴 TODO | - | Commandes |
```

---

## 🔒 SecDev Checklist (OBLIGATOIRE)

Chaque US/implémentation DOIT inclure:

### Avant implémentation
- Threat modeling: quels vecteurs d'attaque?
- Input validation: quelles entrées utilisateur?
- Error handling: quelles erreurs exposées?

### Pendant implémentation
- Pas de `unwrap()` sur données utilisateur
- Pas de secrets hardcodés
- Logs sans données sensibles
- Bounds checking sur arrays/vecteurs

### Avant CHAQUE commit
```powershell
cargo fmt --all                    # Formatage
cargo clippy -- -D warnings        # Linting strict
cargo deny check                   # Audit sécurité
cargo test --workspace             # Tests
```

**⚠️ AUCUN commit si une de ces commandes échoue.**

---

## 📚 Fichiers Critiques (ne pas modifier sans review)

- `Cargo.toml`: workspace et features
- `deny.toml`: politique de sécurité dépendances
- `.github/workflows/`: CI/CD
- `crates/velesdb-core/src/index/hnsw/`: algorithme critique
