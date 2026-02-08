---
description: Review Rust code against best practices using Context7 documentation. Only for Rust projects (auto-detected via Cargo.toml).
---

# /rust-review

Review Rust code against idiomatic patterns and best practices from official Rust documentation.

**Usage:** `/rust-review` or `/rust-review [file.rs]`

## Pre-flight Check

```bash
# Only run if this is a Rust project
[ -f Cargo.toml ] || { echo "SKIP: Not a Rust project (no Cargo.toml)"; exit 0; }
```

If `Cargo.toml` is not found in the project root, **skip this workflow silently**.

## When to Use

- Before committing Rust code changes
- During refactoring
- When debugging non-idiomatic patterns
- Code review

## Process

### Step 1: Identify Target Code

If file specified, read that file. Otherwise, check recent edits:

```bash
git diff --name-only HEAD | grep "\.rs$" | head -5
```

### Step 2: Load Best Practices Reference

**Option A: Local file (always available)**

Read `.book/rust_best_practices_complet.md` — comprehensive French guide covering:
- Ownership, borrowing, lifetimes
- Error handling (`Result`, `Option`, `?` operator)
- Traits and generics
- Concurrency patterns
- Testing best practices
- Unsafe Rust guidelines

```bash
# Check local reference exists
[ -f .book/rust_best_practices_complet.md ] && echo "LOCAL_REF_AVAILABLE"
```

**Option B: Context7 MCP (if available)**

Use Context7 for additional/updated patterns. Library IDs:

| Topic | Library ID |
|-------|------------|
| **Comprehensive Rust** | `/websites/google_github_io_comprehensive-rust` |
| **The Rust Book** | `/websites/doc_rust-lang_book` |
| **Rust Stdlib** | `/websites/rustwiki_zh-cn_std` |
| **egui** | `/emilk/egui` |

**External Tutorial (egui beginners):**
- [Rust egui Step-by-Step Tutorial](https://hackmd.io/@Hamze/Sys9nvF6Jl) — Covers egui basics, widget usage, layout patterns

Query examples:
- `error handling Result Option ? operator`
- `ownership borrowing lifetime patterns`
- `async await tokio patterns`
- `trait implementation derive macros`
- `egui widget layout ui patterns`

**Priority:** Use local file first, complement with Context7 if needed.

### Step 3: Review Against Patterns

Check code for:

#### Error Handling
- [ ] Uses `?` operator for propagation (not manual `match`)
- [ ] Uses `thiserror` for library errors
- [ ] Uses `anyhow` for application errors with context
- [ ] No `unwrap()` in production paths (only after validation)
- [ ] Errors have `#[derive(Debug, Error)]`

#### Ownership & Borrowing
- [ ] Prefers `&str` over `String` for read-only params
- [ ] Uses `Cow<str>` when ownership conditional
- [ ] Avoids unnecessary `.clone()`
- [ ] Uses references where ownership not needed

#### Async Patterns
- [ ] Avoids blocking in async contexts
- [ ] Uses `tokio::spawn` for concurrent tasks
- [ ] Handles cancellation properly
- [ ] Uses channels for cross-task communication

#### Code Style
- [ ] Uses iterators over manual loops when clearer
- [ ] Uses `if let` / `let else` for single-arm matches
- [ ] Prefers explicit types on public APIs
- [ ] Uses `#[must_use]` on functions that return important values

#### egui Patterns (if applicable)
- [ ] Uses `ui.horizontal()` / `ui.vertical()` for layout grouping
- [ ] Avoids heavy computation in `update()` — defer to background tasks
- [ ] Uses `ctx.request_repaint()` only when state changes
- [ ] Prefers `egui::RichText` for styled text
- [ ] Uses `ui.add_space()` for consistent spacing
- [ ] Implements `eframe::App` trait correctly with proper `update()` signature

### Step 4: Output Review

Format:

```markdown
## Rust Review: [file.rs]

### ✅ Good Patterns
- [Pattern observed]

### ⚠️ Improvements
| Line | Issue | Fix |
|------|-------|-----|
| 45 | Manual error match | Use `?` operator |
| 78 | `unwrap()` in prod | Use `ok_or()` or `?` |

### 📚 References
- [Context7 snippet link or pattern name]
```

## Quick Checks

For fast validation without full review:

```bash
# Find unwrap/expect in non-test code
rg "\.unwrap\(\)|\.expect\(" --glob "!**/tests/**" --glob "*.rs"

# Find manual Result matches
rg "match.*Ok\(|match.*Err\(" --glob "*.rs"
```

## Example Query Flow

```
1. User: /rust-review src/services/api_service.rs

2. AI reads file, identifies patterns

3. AI queries Context7:
   - /websites/google_github_io_comprehensive-rust
   - Query: "async error handling Result anyhow context"

4. AI compares code against patterns

5. AI outputs review with specific improvements
```
