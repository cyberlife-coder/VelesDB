# Contributing to VelesDB

First off, thank you for considering contributing to VelesDB! It's people like you that make VelesDB such a great tool.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How Can I Contribute?](#how-can-i-contribute)
- [Development Setup](#development-setup)
- [Pull Request Process](#pull-request-process)
- [Style Guidelines](#style-guidelines)
- [Release Process](#release-process)

## Code of Conduct

This project and everyone participating in it is governed by our [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## How Can I Contribute?

### Reporting Bugs

Before creating bug reports, please check the existing issues to avoid duplicates. When you create a bug report, include as many details as possible:

- **Use a clear and descriptive title**
- **Describe the exact steps to reproduce the problem**
- **Provide specific examples** (code snippets, configuration files)
- **Describe the behavior you observed and what you expected**
- **Include your environment details** (OS, Rust version, VelesDB version)

### Suggesting Enhancements

Enhancement suggestions are tracked as GitHub issues. When creating an enhancement suggestion:

- **Use a clear and descriptive title**
- **Provide a detailed description of the proposed enhancement**
- **Explain why this enhancement would be useful**
- **List any alternatives you've considered**

### Your First Code Contribution

Unsure where to begin? Look for issues labeled:

- `good first issue` - Simple issues perfect for newcomers
- `help wanted` - Issues where we need community help
- `documentation` - Documentation improvements

### Pull Requests

This project follows **Git Flow**. Feature and fix branches must target `develop`, not `main`.

| Branch prefix | Target | Examples |
|---------------|--------|---------|
| `feature/*`, `feat/*` | `develop` | `feature/amazing-feature` |
| `fix/*`, `bugfix/*` | `develop` | `fix/crash-on-empty-index` |
| `refactor/*`, `chore/*`, `ci/*`, `docs/*`, `style/*`, `perf/*`, `test/*`, `build/*` | `develop` | `docs/update-api-guide` |
| `release/*`, `hotfix/*`, `support/*` | `main` | `release/1.2.0` |

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`cargo test`)
5. Run lints (`cargo clippy`)
6. Format code (`cargo fmt`)
7. Commit your changes (`git commit -m 'Add amazing feature'`)
8. Push to the branch (`git push origin feature/amazing-feature`)
9. Open a Pull Request **targeting `develop`** (not `main`)

## Architecture Quick Start

New to the codebase? Start with these documents (in order):

1. **[Project Structure](docs/contributing/PROJECT_STRUCTURE.md)** — Workspace layout, crate responsibilities (~15 min)
2. **[Architecture](docs/reference/ARCHITECTURE.md)** — 3-layer design (Client, API, Core), data flow (~30 min)
3. **[Concurrency Model](docs/CONCURRENCY_MODEL.md)** — Lock ordering, shard strategy, deadlock prevention (~20 min)
4. **[Storage Format](docs/STORAGE_FORMAT.md)** — On-disk layout, WAL, mmap (~15 min)
5. **[Soundness](docs/SOUNDNESS.md)** — Unsafe code audit with invariant proofs (reference)

### Workspace Crate Map

| Crate | Purpose |
|-------|---------|
| `velesdb-core` | Core engine: HNSW, SIMD, VelesQL, collections, storage |
| `velesdb-server` | Axum REST API server (47 endpoints, OpenAPI optional) |
| `velesdb-cli` | Interactive REPL for VelesQL |
| `velesdb-python` | PyO3 bindings with NumPy support |
| `velesdb-wasm` | Browser-side vector search (no `persistence` feature) |
| `velesdb-mobile` | iOS/Android bindings via UniFFI |
| `velesdb-migrate` | Schema/data migration tooling |
| `tauri-plugin-velesdb` | Tauri desktop integration |

## Quality Gates

All new/modified code must satisfy these limits (enforced by Codacy and CI):

| Metric | Limit | Enforcement |
|--------|-------|-------------|
| Cyclomatic complexity | **<= 8** per function | Codacy |
| Function NLOC | **<= 50** lines | Codacy |
| File NLOC | **<= 500** lines | Code review |
| Code duplication | **< 2%** | jscpd |
| Unsafe blocks | Must have `// SAFETY:` comment | CI (`verify_unsafe_safety_template.py`) |
| TODO format | `// TODO(EPIC-XXX):` only | CI (`check-todo-annotations.py`) |
| `.unwrap()` | Forbidden in production code | Code review |
| Recall@10 | **>= 0.95** (if search path modified) | CI + local validation |

### Concurrency Rules

- Use `parking_lot::RwLock` / `Mutex` (never `std::sync` — no poisoning, no `.unwrap()` on locks)
- Follow lock ordering documented in [CONCURRENCY_MODEL.md](docs/CONCURRENCY_MODEL.md)
- Tests MUST run single-threaded: `--test-threads=1` (file system isolation)

## Pre-Push Validation

Run this sequence **before every push** (CI does not run on PRs):

```bash
# 1. Format
cargo fmt --all

# 2. Lint (strict — mirrors CI)
cargo clippy --workspace --all-targets --features persistence,gpu,update-check \
  --exclude velesdb-python -- -D warnings -D clippy::pedantic

# 3. Tests
cargo test -p velesdb-core --features persistence -- --test-threads=1

# 4. (If search path modified) Recall gate
cargo test -p velesdb-core --features persistence test_recall -- --test-threads=1

# 5. Feature gate check
cargo check --no-default-features
cargo check -p velesdb-wasm --no-default-features --target wasm32-unknown-unknown

# 6. (Optional) Codacy CLI via WSL
wsl -- bash -c "cd /mnt/d/Projets-dev/velesDB/velesdb-core && codacy-cli analyze 2>&1"
```

Or use the local CI script: `.\scripts\local-ci.ps1` (Full) or `.\scripts\local-ci.ps1 -Quick` (fmt + clippy only).

Git hooks are provided in `.githooks/` — activate with: `git config core.hooksPath .githooks`

## Development Setup

### Prerequisites

- Rust 1.83+ (stable) — enforced as MSRV
- Docker (optional, for integration tests)

### Building from Source

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/velesdb.git
cd velesdb

# Build the project
cargo build --workspace

# Run tests (single-threaded — required for file system isolation)
cargo test --workspace --features persistence,gpu,update-check \
  --exclude velesdb-python -- --test-threads=1

# Lint (strict — mirrors CI)
cargo clippy --workspace --all-targets --features persistence,gpu,update-check \
  --exclude velesdb-python -- -D warnings -D clippy::pedantic

# Run the server locally
cargo run --bin velesdb-server -- --data-dir ./data
```

### Running Benchmarks

```bash
cargo bench -p velesdb-core --features internal-bench -- --noplot
```

## Pull Request Process

1. **Ensure all tests pass** - Run `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1` before submitting
2. **Update documentation** - If you're adding new features, update the relevant docs
3. **Follow the style guidelines** - Run `cargo fmt` and `cargo clippy`
4. **Write meaningful commit messages** - Follow conventional commits format
5. **Keep PRs focused** - One feature or fix per PR
6. **Be responsive** - Address review feedback promptly

### Commit Message Format

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

Types:
- `feat`: A new feature
- `fix`: A bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

Examples:
```
feat(search): add hybrid search support
fix(storage): resolve mmap alignment issue on ARM
docs(readme): update quick start guide
```

## Style Guidelines

### Rust Code Style

- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `rustfmt` for formatting (default configuration)
- Use `clippy` for linting (fix all warnings)
- Write documentation for public APIs
- Keep functions under 50 lines when possible
- Prefer composition over inheritance

### Documentation Style

- Use clear, concise language
- Include code examples where appropriate
- Keep README focused on getting started
- Put detailed docs in the `/docs` folder

## Recognition

Contributors will be recognized in:
- The project's README
- Release notes for significant contributions
- Our Discord community

## Release Process

VelesDB utilise **3 workflows GitHub Actions simplifiés** :

| Workflow | Fonction |
|----------|----------|
| `ci.yml` | Tests, lint, security |
| `release.yml` | Publication complète (binaries, crates.io, PyPI, npm) |
| `bench-regression.yml` | Benchmarks |

### Publier une release

```bash
# 1. Mettre à jour version dans Cargo.toml
# 2. Commit et tag
git commit -am "release: v1.11.1"
git tag v1.11.1
git push origin main v1.11.1
```

Le workflow `release.yml` publie automatiquement sur :
- GitHub Releases (binaries)
- crates.io
- PyPI
- npm

📖 Guide complet : [docs/contributing/RELEASE.md](docs/contributing/RELEASE.md)

---

Thank you for contributing to VelesDB! 🦀
