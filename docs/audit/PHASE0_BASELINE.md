# Phase 0 — Baseline & cartographie (velesdb core)

**Date** : 2026-07-11 · **HEAD audité** : `develop` @ 3f4dcc11 (seam control-plane mergé, tag `v3.9.0` posé)

## 1. Cartographie des crates

| Crate | Version | Responsabilité | Dépend de |
|-------|---------|----------------|-----------|
| velesdb-core | 3.9.0 (workspace) | Moteur : Vector HNSW+SIMD, Graph CSR, ColumnStore, Sparse/BM25, VelesQL, observer/seam, WAL/snapshots | — |
| velesdb-server | 3.9.0 | REST Axum (54 endpoints), OpenAPI/utoipa, Prometheus | core(openapi) |
| velesdb-cli | 3.9.0 | REPL embarqué, binaire `velesdb` | server, migrate |
| velesdb-python | 3.9.0 | Bindings PyO3 (`velesdb`) — exclu clippy/test workspace | core, memory |
| velesdb-wasm | 3.9.0 | Build WASM (core sans default features) | core, memory |
| velesdb-migrate | 3.9.0 | Outil de migration (source redis par défaut) | core |
| velesdb-mobile | 3.9.0 | Bindings uniffi iOS/Android | core |
| velesdb-memory | **0.6.0** (cadence indépendante) | Mémoire d'agent MCP (sémantique/épisodique/procédurale, `why()`) | core |
| velesdb-node | **0.6.0** | npm `@wiscale/velesdb-memory-node` (napi) | memory |
| tauri-plugin-velesdb | 3.9.0 | Plugin Tauri | core |

**Features velesdb-core** : `persistence` (défaut), `gpu`, `update-check`, `openapi`, `internal-bench`, `bench-sift1m` (jamais shippé), `loom`, `test-fault-injection`. Features workspace CI : `persistence,gpu,update-check`.

## 2. Incohérences de version détectées

| Localisation | Valeur | Attendu | Statut |
|--------------|--------|---------|--------|
| `Cargo.toml:22` (workspace.package) | 3.9.0 | 3.9.0 | ✅ |
| `Cargo.toml:82` (workspace.dependencies velesdb-core) | **3.8.1** | 3.9.0 | ❌ à corriger (action C1, Phase 1) |
| sdks/typescript, integrations/* | 3.8.x | cadence indépendante | Différé (décision D4) : seules les mentions textuelles erronées seront corrigées |
| docs/openapi.{json,yaml} | 3.8.0 | 3.9.0 | Corrigé sur `origin/claude/admiring-fermi-ouji1h` (à lander) |

## 3. Inventaire CI ↔ critères DoD

| Critère DoD Core | Outillage existant |
|------------------|---------------------|
| clippy 0 warning | `ci.yml` job lint + `cargo make clippy` (pedantic, -D warnings, exclut velesdb-python) |
| Couverture ≥85 % | `ci.yml` job coverage : **cargo-llvm-cov → Codacy** (pas tarpaulin) |
| 0 code mort / deps inutiles | cargo udeps (nightly) — pas en CI, exécuté localement pour cette phase |
| Tests verts | `ci.yml` job test (single-threaded), suites BDD maison `tests/bdd/`, conformance JSON partagée core↔premium |
| Non-régression perf | `bench-regression.yml` (>20 % = fail), `perf-gate-e2e.yml` (recall + p50), baseline criterion locale `phase0` |
| Sécurité deps | `cargo audit` + cargo-deny (`deny.toml`, advisories v2) |
| Qualité structurelle | QUALITY_BAR.md : complexité ≤8, NLOC ≤50, dup <2 % (Codacy + jscpd), TSan nightly |

## 4. Audit statique (sorties brutes)

### 4.1 clippy (pedantic, -D warnings, features persistence,gpu,update-check)
`cargo make clippy` → **exit 0, 0 warning** ✅ (497,8 s, workspace complet hors velesdb-python)

### 4.2 cargo +nightly udeps (--workspace --features persistence,gpu,update-check --exclude velesdb-python)
**1 finding** (sans `--all-targets`, donc à confirmer) :
- `velesdb-core` : dépendances `ndarray` et `uuid` non utilisées par la cible lib.
  Note udeps : « might be used by other targets » — `ndarray` fait partie des deps de la
  feature `persistence` ; vérifier avec `--all-targets` en Phase 1 avant suppression ou
  ajout de `package.metadata.cargo-udeps.ignore`.

### 4.3 cargo audit → 3 advisories signalées — **résolues/déjà couvertes** (Phase 1)

`cargo audit` (tâche Makefile) ne lit pas la liste d'ignores de `deny.toml` ; le **gate CI réel est `cargo deny check advisories`**, qui était déjà vert.

| ID | Crate | Sévérité | Chemin | Résolution |
|----|-------|----------|--------|-----------|
| RUSTSEC-2026-0185 | quinn-proto 0.11.14 | 7.5 high (mem exhaustion QUIC) | dép transitive | ✅ **corrigé** : `cargo update -p quinn-proto` → 0.11.16 (commit dédié) |
| RUSTSEC-2026-0195 | quick-xml 0.38.4 | 7.5 high (DoS mémoire NsReader) | plist→tauri→tauri-plugin-velesdb | ✅ **déjà ignoré justifié** dans `deny.toml:45` (transitive build-time Tauri/plist, **absent des binaires serveur/CLI**, plist parse des Info.plist contrôlés au build ; plist 1.9.0 plafonne à quick-xml 0.39, pas de fix ≥0.41 possible) |
| RUSTSEC-2026-0194 | quick-xml 0.38.4 | 7.5 high (quadratic attrs) | idem | ✅ **déjà ignoré justifié** dans `deny.toml:44` |

Vérifié : `tauri` absent des dépendances de `velesdb-server` et `velesdb-cli` — les surfaces réseau ne compilent pas quick-xml. `cargo deny check advisories` = **advisories ok** après le bump quinn-proto.

### 4.4 cargo fmt --check
**exit 0** ✅

### 4.5 gitleaks (historique complet, 3573 commits, 90.77 MB)
**5 findings — tous faux positifs** (vérifiés ligne par ligne) :

| Rule | Fichier:ligne | Verdict |
|------|---------------|---------|
| generic-api-key | `.github/workflows/bench-sift1m-nightly.yml:51` (×2 commits) | FP — `shared-key: "velesdb-sift1m"` = clé de cache rust-cache, pas un secret |
| curl-auth-header | `crates/velesdb-server/README.md:326` | FP — exemple curl de doc |
| curl-auth-header | `docs/guides/SERVER_SECURITY.md:112` | FP — placeholder `Bearer key-for-app-a` |
| curl-auth-header | `docs/guides/SERVER_SECURITY.md:245` | FP — placeholder `Bearer my-secure-key` |

Action Phase 1 : ajouter `.gitleaksignore` (fingerprints) pour un scan futur propre.

## 5. Baseline performance

### 5.1 Criterion `simd_benchmark` — baseline `phase0` sauvegardée ✅
`cargo bench -p velesdb-core --bench simd_benchmark -- --save-baseline phase0 --noplot` (exit 0, machine au repos).
Échantillon : `batch_jaccard/batch_100/384` = 2,91 µs · `/768` = 6,94 µs · `/1536` = 10,83 µs. Baseline complète dans `target/criterion/`.

### 5.2 Gate QUALITY_BAR (équivalent perf-gate-e2e, 10K×384D, wheel release, macOS arm64 local)

| Métrique | Mesuré | Gate | Verdict |
|----------|--------|------|---------|
| recall@10 (mean, 100 requêtes, 50 clusters) | **0,985** | ≥0,95 | ✅ |
| recall@10 (min) | 0,80 | — | info |
| search p50 | **446 µs** | ≤450 µs | ✅ **marge 4 µs seulement** |
| search p99 | 883 µs | — | info |
| insert (10K, WAL ON, batch 5000) | 34,1 s (292 v/s) | — | info |
| taille DB | 31,4 Mo | — | info |

⚠️ La marge p50 est quasi nulle en local : toute modification Phase 1 touchant le chemin de recherche doit re-passer cette mesure sur machine au repos (décision D8/bench). Le run 100K (gate CI complet) est différé — coûteux, sera exécuté au moment de la gate Phase 1.

**Incohérence de version supplémentaire découverte pendant ce run** : `crates/velesdb-python/pyproject.toml:7` → `version = "3.8.1"` en dur (le wheel sort en 3.8.1 alors que le crate est 3.9.0). Ajouté au chantier d'harmonisation Phase 1.

## 6. Couverture (scope « code métier », décision D8)

Invocation identique à la CI (`.github/workflows/ci.yml` §coverage) :
`cargo llvm-cov --features persistence,gpu,update-check --workspace --exclude velesdb-python --exclude velesdb-node --lcov -- --test-threads=1` (exit 0).

| Scope | Lignes | Non couvertes | Couverture |
|-------|--------|---------------|-----------|
| Workspace total (hors python/node) | 135 970 | 21 314 | **84,32 %** |
| **velesdb-core/src (code métier, D8)** | 58 643 | 5 925 | **89,90 %** ✅ (≥85 %) |
| velesdb-core/src **hors `gpu/`** | 57 036 | 5 428 | **90,48 %** |
| velesdb-core/src/`gpu/` seul (exclusion structurelle : pas de device GPU dans l'env) | 1 607 | 497 | 69,1 % |

**Verdict D8** : le code métier du core dépasse déjà la cible de 85 % (89,90 %, ou 90,48 % en excluant les chemins GPU non exerçables sans device). Le critère DoD de couverture est **atteint dès la baseline** ; Phase 1 n'a donc pas de dette de couverture bloquante sur le core, seulement des cibles d'opportunité.

Fichiers core les moins couverts (>150 lignes, <70 %), candidats d'opportunité Phase 1 :
- `gpu/gpu_traversal_buffers.rs` 0 % (321 l.) et `gpu/gpu_traversal.rs` 53,6 % — **exclusion structurelle GPU documentée** (pas de device).
- `collection/any_collection.rs` 32,5 % (dispatch enum, chemins peu testés) — cible réelle.
- `collection/search/query/aggregation/mod.rs` 64,1 % et `graph_prefilter.rs` 67,7 % — cibles réelles.

La dérive workspace (84,32 %) vient surtout des crates périphériques (wasm, mobile, cli) hors scope métier ; le seuil CI est fixé à 78 % pour cette raison.
