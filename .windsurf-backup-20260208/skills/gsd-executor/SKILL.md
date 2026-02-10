---
name: gsd-executor
description: Methodology for executing GSD plans with atomic commits, deviation handling, and verification. Invoke when executing PLAN.md files.
---

# GSD Executor

You are executing a GSD plan. Follow this methodology for consistent, high-quality execution.

## Core Principles

1. **Atomic commits** — One commit per task, never batch
2. **Deviation handling** — Auto-fix bugs/blockers, ask about architecture
3. **Verification first** — Don't proceed until verification passes
4. **Document everything** — Deviations go in SUMMARY.md

## Execution Flow

### For Each Task

1. **Read task completely** before starting
2. **Implement** following the action instructions
3. **Verify** using the verification command
4. **Commit** only task-related files
5. **Record** commit hash for Summary

### Commit Protocol

```bash
# NEVER use git add . or git add -A
# Stage files individually
git add src/specific/file.ts
git add src/another/file.ts

# Commit with conventional format
git commit -m "{type}({phase}-{plan}): {task description}"
```

**Types:** `feat`, `fix`, `test`, `refactor`, `docs`, `chore`

## Deviation Rules

### Rule 1: Auto-fix Bugs
**Trigger:** Code doesn't work (errors, wrong output, crashes)

**Action:** Fix immediately, track for Summary

**Examples:**
- Wrong logic, off-by-one errors
- Type errors, null pointer exceptions
- Broken validation

**No permission needed.** Bugs must be fixed.

### Rule 2: Auto-add Critical Functionality
**Trigger:** Missing essential security/correctness features

**Action:** Add immediately, track for Summary

**Examples:**
- Missing error handling
- No input validation
- Missing auth checks
- No rate limiting

**No permission needed.** These are requirements for correctness.

### Rule 3: Auto-fix Blockers
**Trigger:** Can't proceed without fixing

**Action:** Fix to unblock, track for Summary

**Examples:**
- Missing dependency
- Wrong import path
- Missing environment variable
- Build config error

**No permission needed.** Can't complete task without fix.

### Rule 4: ASK About Architectural Changes
**Trigger:** Significant structural modification needed

**Action:** STOP and present to user

**Examples:**
- Adding new database table
- Major schema changes
- Switching libraries/frameworks
- New service layer

**User decision required.** These affect system design.

### Priority Order

1. If Rule 4 applies → STOP and ask
2. If Rules 1-3 apply → Fix and document
3. If unsure → Apply Rule 4 (ask)

## Tracking Deviations

For each auto-fix, record:

```markdown
**[Rule N - Type] [Description]**
- Found during: Task [N]
- Issue: [what was wrong]
- Fix: [what was done]
- Files: [affected]
- Commit: [hash]
```

## Verification Patterns

### Rust Projects (if Cargo.toml exists)

Before committing `.rs` files, verify against best practices:

```bash
# Check if Rust project
[ -f Cargo.toml ] || skip

# Compile check
cargo build --lib 2>&1 | head -20

# Run tests
cargo test --lib

# Clippy lints
cargo clippy --lib -- -D warnings
```

**Context7 Best Practices Check:**
Query `/websites/google_github_io_comprehensive-rust` for patterns relevant to changes:
- **Error handling:** Uses `?` operator, `thiserror`/`anyhow`, no `unwrap()` in prod
- **Ownership:** Prefers `&str` over `String` for params, avoids unnecessary `.clone()`
- **Async:** No blocking in async, uses channels for cross-task communication

Flag violations before commit. Reference `/rust-review` workflow for detailed checks.

### Code Verification (TypeScript/JS)
```bash
# TypeScript compiles
npx tsc --noEmit

# Tests pass
npm test

# Linting passes
npm run lint
```

### API Verification
```bash
# Endpoint responds
curl -X POST localhost:3000/api/endpoint -d '{}' -H "Content-Type: application/json"

# Returns expected status
# Check response body
```

### Component Verification
```bash
# Build succeeds
npm run build

# Dev server starts
npm run dev
```

## Summary Creation

After all tasks, create SUMMARY.md with:

1. **One-liner** — Substantive, not generic
   - Good: "JWT auth with refresh rotation using jose"
   - Bad: "Authentication implemented"

2. **What was built** — 2-3 paragraphs

3. **Tasks table** — Task, commit hash, files

4. **Deviations** — All auto-fixes documented

5. **Decisions** — Any choices made during execution

## Quality Checklist

Before marking plan complete:

- [ ] All tasks executed
- [ ] All verifications pass
- [ ] Each task has its own commit
- [ ] Deviations documented
- [ ] SUMMARY.md is substantive
- [ ] No uncommitted changes
- [ ] Rust best practices verified (if `.rs` files modified)
