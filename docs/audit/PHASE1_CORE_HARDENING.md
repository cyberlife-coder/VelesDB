# Phase 1 — Durcissement core : rapport de gate

**Date** : 2026-07-11 · **Base** : `develop` @ 3f4dcc11 (v3.9.0) · **Branches livrées** : PR #1350 (fix télémétrie), PR #1351 (harmonisation + sécurité + udeps)

> Ce rapport vérifie chaque critère de la Definition of Done — Core individuellement, avec la commande exécutée et sa sortie brute. Positionnement : le durcissement protège les deux promesses produit (mémoire d'agent explicable + performance embarquée) et l'intégrité du seam open-core.

## Travaux livrés (commits atomiques, sans attribution IA, auteur cyberlife-coder)

| Commit | Objet |
|--------|-------|
| `b6f8b01c` | fix(server): stop double-firing on_query telemetry on /query (contrat exactly-once du seam + test de régression) |
| `28c2020b` | fix(server): regenerate OpenAPI 3.9.0 + clippy doc_markdown |
| `01e3701a` | fix(workspace): harmonize velesdb-core version to 3.9.0 (C1) + docs |
| `68103fe6` | chore(security): .gitleaksignore (5 FP vérifiés) |
| `91950bf8` | chore(security): bump quinn-proto 0.11.14→0.11.16 (RUSTSEC-2026-0185) |
| `dfe3c8a9` | refactor(core): drop unused ndarray + uuid deps |
| `168131a1` | refactor: drop unused tokio-test dev-dep |

PR #1350 (fix télémétrie, autonome) et #1351 (empilée : harmonisation + sécurité + udeps) ouvertes vers `develop`. **Merge en attente d'action humaine** (le garde-fou du harness interdit à l'agent de merger une PR auto-écrite sans revue).

## Definition of Done — Core : vérification critère par critère

| # | Critère | Commande | Résultat | Statut |
|---|---------|----------|----------|--------|
| 1 | `cargo udeps` = 0 | `cargo +nightly udeps --workspace --all-targets --features persistence,gpu,update-check --exclude velesdb-python` | « All deps seem to have been used » | ✅ |
| 2 | clippy -D warnings = 0 | `cargo make clippy` (pedantic, exclut velesdb-python) | exit 0, 0 warning | ✅ |
| 3 | 0 bug connu ouvert | fix double-télémétrie livré + test de régression `observer_lifecycle_tests.rs` | contrat exactly-once verrouillé | ✅ |
| 4 | Tests verts + couverture ≥85 % métier | `cargo llvm-cov` (invocation CI, single-thread) | velesdb-core/src = **89,90 %** (90,48 % hors gpu) | ✅ |
| 5 | Pas de régression perf vs baseline phase0 | gate 10K recall/p50 | recall@10 **0,985** (≥0,95), p50 **446 µs** (≤450) | ✅ |
| 6 | Documentation à jour | CORE_PREMIUM_SPLIT (Truth 2/5, C1 résolu), doc-comments ndarray corrigés, guides→3.9.0 | cohérente | ✅ |
| 7 | Versions harmonisées | `Cargo.toml:82` + `pyproject.toml` → 3.9.0 ; `Cargo.lock` core = 3.9.0 | plus aucune réf. core 3.8.x in-scope | ✅ |
| 8 | Changelog breaking changes | CHANGELOG `[3.9.0]` (seam, hash_id, LockRank, WalCursor, conformance) | complet et daté 2026-07-07 | ✅ |
| 9 | Sécurité deps | `cargo deny check advisories` | « advisories ok » (quinn-proto corrigé ; quick-xml ignoré justifié) | ✅ |
| 10 | Onboarding <15 min env vierge | conteneur Docker propre, chronométré | Python 3 s · dev Rust 70 s | ✅ |
| 11 | CI GitHub verte | PR #1350 / #1351 | #1350 = 29✅ · #1351 = **32✅ 0 échec** | ✅ |

## Onboarding (preuves conteneur vierge)

**Chemin Python (headline README « 60 s ») — `python:3.12-slim` vierge** :
```
pip install velesdb + curl hello_velesdb.py + python hello_velesdb.py
→ python_onboard_seconds=3
→ sortie identique au README (scores tech/music attendus)
→ velesdb version: 3.8.1 (release PyPI publiée ; 3.9.0 pas encore publié)
```
✅ **3 secondes**, très en-deçà des 15 min. La promesse d'adoption sans friction (funnel open-core) est tenue sur le chemin recommandé.

**Chemin dev Rust (clone → build from source) — `rust:1.89-bookworm` vierge** :
```
git clone --depth 1 file:///src /work   (source = 44 Mo, sans target/)
cargo build -p velesdb-core --features persistence   → core_build_seconds=27
cargo build -p velesdb-cli                            → total_build_seconds=70
```
✅ **70 secondes** (core + binaire CLI, compilation cold complète) — très en-deçà des 15 min.
Note méthodo : le 1er essai (`cp -r` du checkout) a échoué (`No space left on device`) car le `target/` de l'hôte fait **201 Go** ; `git clone` (fichiers suivis uniquement) est la bonne méthode et donne une mesure représentative d'un nouveau contributeur.

**Verdict onboarding** : les deux parcours (Python 3 s, dev Rust 70 s) valident le critère <15 min sur environnement vierge. ✅

## Décisions notables (journal)

- **C1 muté** : l'incohérence documentée `3.8.0` était en fait `3.8.1` (après le bump de release 3.8.1) — corrigée en 3.9.0, comme `pyproject.toml` (le wheel sortait en 3.8.1 depuis des sources 3.9.0).
- **URLs de download INSTALLATION.md laissées en v3.8.1** : dernière release GitHub publiée avec artefacts ; le tag `v3.9.0` n'a pas encore de release. À publier au moment de la gate.
- **quick-xml (RUSTSEC-2026-0194/0195)** : déjà ignoré-justifié dans deny.toml (transitive build-time Tauri/plist, absent des binaires serveur/CLI) ; `cargo audit` les signalait car il ne lit pas deny.toml — le gate CI réel est cargo-deny.
- **ndarray/uuid/tokio-test** : supprimés après confirmation (grep zéro usage + `cargo check --all-features --all-targets` vert).

## Action de gate (post-merge, requiert action humaine)

1. Merger #1350 puis #1351 dans `develop`.
2. Comme des changements observables ont eu lieu depuis le tag v3.9.0 (fix télémétrie /query, régén OpenAPI), couper une **release 3.9.1** : `release/3.9.1` → PR `main` → tag `v3.9.1` + **publier la release GitHub** (artefacts) → re-pin premium sur `v3.9.1`.
3. Alternative si l'on considère le fix comme pré-release de 3.9.0 non encore publiée : publier directement la release `v3.9.0` depuis le develop mergé. **Recommandation : v3.9.1** (le tag v3.9.0 est déjà consommé par le premium ; ne pas retagger).
