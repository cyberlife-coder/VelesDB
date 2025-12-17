# Contributing to VelesDB

First off, thank you for considering contributing to VelesDB! It's people like you that make VelesDB such a great tool.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How Can I Contribute?](#how-can-i-contribute)
- [Development Setup](#development-setup)
- [Pull Request Process](#pull-request-process)
- [Style Guidelines](#style-guidelines)

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

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`cargo test`)
5. Run lints (`cargo clippy`)
6. Format code (`cargo fmt`)
7. Commit your changes (`git commit -m 'Add amazing feature'`)
8. Push to the branch (`git push origin feature/amazing-feature`)
9. Open a Pull Request

## Development Setup

### Prerequisites

- Rust 1.75+ (stable)
- Docker (optional, for integration tests)

### Building from Source

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/velesdb.git
cd velesdb

# Build the project
cargo build

# Run tests
cargo test

# Run the server locally
cargo run --bin velesdb-server -- --data-dir ./data
```

### Running Benchmarks

```bash
cargo bench
```

## Pull Request Process

1. **Ensure all tests pass** - Run `cargo test` before submitting
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

Thank you for contributing to VelesDB! 🦀
