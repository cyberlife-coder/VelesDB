# VelesDB-Core - Structure du Projet

## Vue d'ensemble

VelesDB-Core est un **workspace Cargo** contenant plusieurs crates. C'est le moteur open-source de la base de données vectorielle.

```
velesdb-core/
│
├── Cargo.toml                 # Workspace principal
├── Cargo.lock                 # Lock des versions
│
├── rust-toolchain.toml        # Version Rust (stable)
├── rustfmt.toml               # Config formatage
├── clippy.toml                # Config linter
├── deny.toml                  # Audit sécurité deps
├── Makefile.toml              # Tasks cargo-make
│
├── .cargo/
│   └── config.toml            # Aliases cargo
│
├── .githooks/
│   └── pre-commit             # Hook pré-commit
│
├── .windsurf/
│   └── workflows/             # Workflows AI assistants
│       ├── rust-feature.md
│       ├── rust-debug.md
│       └── ...
│
├── crates/
│   ├── velesdb-core/          # Lib principale (moteur vectoriel)
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── collection/    # Gestion collections
│   │   │   ├── index/         # HNSW index
│   │   │   ├── storage/       # Persistence
│   │   │   ├── velesql/       # Query language parser
│   │   │   └── simd/          # SIMD optimizations
│   │   └── benches/
│   │
│   ├── velesdb-server/        # API REST (Axum)
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-cli/           # CLI / REPL VelesQL
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs
│   │
│   └── velesdb-python/        # Python bindings (PyO3)
│       ├── Cargo.toml
│       ├── src/
│       │   └── lib.rs
│       └── tests/
│           └── test_velesdb.py
│
├── integrations/
│   └── langchain-velesdb/     # LangChain VectorStore
│       ├── pyproject.toml
│       ├── README.md
│       ├── src/langchain_velesdb/
│       │   ├── __init__.py
│       │   └── vectorstore.py
│       └── tests/
│
├── docs/
│   ├── PROJECT_STRUCTURE.md   # Ce fichier
│   ├── CODING_RULES.md        # Règles de développement
│   ├── TDD_RULES.md           # Test-Driven Development
│   ├── api-reference.md
│   └── getting-started.md
│
├── scripts/
│   └── release.sh             # Script de release
│
└── examples/
    └── python_example.py
```

---

## Fichiers de configuration

### `Cargo.toml` (racine)

Définit le **workspace** et les dépendances communes :

```toml
[workspace]
resolver = "2"
members = ["crates/velesdb-core", "crates/velesdb-server"]

[workspace.package]
version = "0.1.0"
edition = "2021"
# ...

[workspace.dependencies]
tokio = { version = "1.42", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
# ...
```

### `crates/*/Cargo.toml`

Chaque crate **hérite** du workspace :

```toml
[package]
name = "velesdb-core"
version.workspace = true      # ← hérite de workspace.package.version
edition.workspace = true

[dependencies]
tokio = { workspace = true }  # ← hérite de workspace.dependencies
```

---

## Crates

### `velesdb-core`
Moteur vectoriel principal. Contient :
- **HNSW Index** : Recherche approximative des plus proches voisins
- **SIMD** : Calculs de distance optimisés (AVX2/SSE)
- **VelesQL** : Parser du langage de requête SQL-like
- **Storage** : Persistence avec WAL

### `velesdb-server`
Serveur REST API (Axum). Expose :
- Endpoints CRUD collections/points
- Endpoint `/search` et `/search/batch`
- Endpoint `/query` pour VelesQL

### `velesdb-cli`
Interface ligne de commande :
- `repl` : Mode interactif VelesQL
- `query` : Exécution requête unique
- `info` : Informations sur une base

### `velesdb-python`
Bindings Python via PyO3 :
- `velesdb.Database` / `velesdb.Collection`
- Support NumPy arrays (float32, float64)
- Tests pytest complets

---

## Integrations

### `langchain-velesdb`
Package Python pour LangChain :
- `VelesDBVectorStore` compatible LangChain
- Méthodes : `add_texts`, `similarity_search`, `as_retriever`
- Installation : `pip install langchain-velesdb`

### `rust-toolchain.toml`

Fixe la version de Rust pour tous les développeurs :

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

### `.cargo/config.toml`

Définit des **aliases** pour simplifier les commandes :

```toml
[alias]
lint = "clippy --all-targets --all-features -- -D warnings"
test-all = "test --all-features"
```

Usage : `cargo lint`, `cargo test-all`

### `Makefile.toml`

Tasks pour **cargo-make** :

```bash
cargo make check    # fmt + clippy + test
cargo make ci       # fmt + clippy + test + audit
cargo make fmt      # formate le code
```

### `.githooks/pre-commit`

Exécuté automatiquement avant chaque `git commit` :
- Vérifie le formatage
- Lance clippy
- Lance les tests
- Détecte les secrets

**Activation** : `git config core.hooksPath .githooks`

---

## Workflow de développement

```bash
# 1. Cloner
git clone https://github.com/cyberlife-coder/velesdb.git
cd velesdb

# 2. Setup (une seule fois)
rustup update stable
cargo install cargo-make cargo-audit cargo-deny
git config core.hooksPath .githooks

# 3. Développer
cargo check              # Vérifier la compilation
cargo make check         # fmt + clippy + test
cargo make ci            # CI complète locale

# 4. Commit (hook s'exécute automatiquement)
git add .
git commit -m "feat: add feature X"
```

---

## Relation avec VelesDB-Premium

```
┌─────────────────────┐
│   velesdb-premium   │  (repo privé)
│  Features payantes  │
└─────────┬───────────┘
          │ dépend via git
          ▼
┌─────────────────────┐
│    velesdb-core     │  (ce repo)
│  Moteur open-source │
└─────────────────────┘
```

Premium importe Core ainsi :

```toml
[workspace.dependencies]
velesdb-core = { git = "https://github.com/cyberlife-coder/velesdb.git", branch = "main" }
```
