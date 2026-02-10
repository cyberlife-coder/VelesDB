---
description: Map existing codebase for brownfield projects
---

# /gsd-map-codebase

Analyze an existing codebase to understand architecture, patterns, and conventions. Creates `.planning/codebase/` with 7 focused documents.

**Use before:** `/gsd-new-project` on existing codebases

## Process

### Step 1: Detect Codebase

```bash
# Check for existing code
find . -name "*.ts" -o -name "*.tsx" -o -name "*.js" -o -name "*.jsx" -o -name "*.py" -o -name "*.go" 2>/dev/null | grep -v node_modules | grep -v .git | head -50

# Check for package managers
ls package.json requirements.txt Cargo.toml go.mod pyproject.toml 2>/dev/null
```

### Step 2: Analyze Stack

Read dependency files and key config:

```bash
cat package.json 2>/dev/null
cat requirements.txt 2>/dev/null
cat tsconfig.json 2>/dev/null
```

Create `.planning/codebase/STACK.md`:

```markdown
# Technology Stack

## Languages

| Language | Version | Usage |
|----------|---------|-------|
| [lang] | [ver] | [where used] |

## Frameworks

| Framework | Version | Purpose |
|-----------|---------|---------|
| [framework] | [ver] | [purpose] |

## Dependencies

### Production

| Package | Version | Purpose |
|---------|---------|---------|
| [pkg] | [ver] | [what it does] |

### Development

| Package | Version | Purpose |
|---------|---------|---------|
| [pkg] | [ver] | [what it does] |

## Build Tools

| Tool | Config File | Purpose |
|------|-------------|---------|
| [tool] | [file] | [purpose] |
```

### Step 3: Analyze Architecture

Examine directory structure and key files:

```bash
find . -type d -not -path '*/node_modules/*' -not -path '*/.git/*' | head -30
```

Create `.planning/codebase/ARCHITECTURE.md`:

```markdown
# Architecture

## Overview

[High-level description of system architecture]

## Patterns

| Pattern | Where Used | Notes |
|---------|------------|-------|
| [pattern] | [locations] | [notes] |

## Layers

| Layer | Responsibility | Key Files |
|-------|---------------|-----------|
| [layer] | [what it does] | [files] |

## Data Flow

[Description of how data moves through system]

## External Services

| Service | Purpose | Integration Point |
|---------|---------|-------------------|
| [service] | [why] | [where] |
```

### Step 4: Analyze Structure

Create `.planning/codebase/STRUCTURE.md`:

```markdown
# Directory Structure

```
[tree output]
```

## Key Directories

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| [dir] | [purpose] | [files] |

## Entry Points

| Entry | Type | Purpose |
|-------|------|---------|
| [file] | [type] | [what it does] |
```

### Step 5: Analyze Conventions

Look for patterns in naming, formatting, etc.

Create `.planning/codebase/CONVENTIONS.md`:

```markdown
# Coding Conventions

## Naming

| Type | Convention | Example |
|------|------------|---------|
| Files | [pattern] | [example] |
| Functions | [pattern] | [example] |
| Variables | [pattern] | [example] |

## Code Style

- [Style rule 1]
- [Style rule 2]

## Formatting

| Aspect | Convention |
|--------|------------|
| Indent | [tabs/spaces] |
| Quotes | [single/double] |

## Linting

| Tool | Config |
|------|--------|
| [tool] | [file] |
```

### Step 6: Analyze Testing

Create `.planning/codebase/TESTING.md`:

```markdown
# Testing

## Framework

| Type | Framework | Config |
|------|-----------|--------|
| Unit | [framework] | [file] |
| Integration | [framework] | [file] |
| E2E | [framework] | [file] |

## Test Location

| Type | Location | Pattern |
|------|----------|---------|
| [type] | [dir] | [naming] |

## Running Tests

```bash
[commands]
```

## Coverage

[Coverage info if available]
```

### Step 7: Analyze Integrations

Create `.planning/codebase/INTEGRATIONS.md`:

```markdown
# External Integrations

## APIs

| API | Purpose | Auth Method |
|-----|---------|-------------|
| [api] | [why] | [how] |

## Services

| Service | Purpose | Config |
|---------|---------|--------|
| [service] | [why] | [where] |

## Environment Variables

| Variable | Purpose | Required |
|----------|---------|----------|
| [var] | [what] | [yes/no] |
```

### Step 8: Identify Concerns

Create `.planning/codebase/CONCERNS.md`:

```markdown
# Technical Concerns

## Tech Debt

| Area | Issue | Impact | Priority |
|------|-------|--------|----------|
| [area] | [what] | [impact] | [priority] |

## Known Issues

| Issue | Location | Workaround |
|-------|----------|------------|
| [issue] | [where] | [how] |

## Fragile Areas

| Area | Why Fragile | Recommendation |
|------|-------------|----------------|
| [area] | [reason] | [suggestion] |

## Security Considerations

| Area | Concern | Status |
|------|---------|--------|
| [area] | [what] | [status] |
```

### Step 9: Commit

```bash
mkdir -p .planning/codebase
git add .planning/codebase/
git commit -m "docs: map existing codebase

7 documents analyzing architecture, stack, and patterns"
```

### Step 10: Completion

```
Codebase mapped:

- Stack: .planning/codebase/STACK.md
- Architecture: .planning/codebase/ARCHITECTURE.md
- Structure: .planning/codebase/STRUCTURE.md
- Conventions: .planning/codebase/CONVENTIONS.md
- Testing: .planning/codebase/TESTING.md
- Integrations: .planning/codebase/INTEGRATIONS.md
- Concerns: .planning/codebase/CONCERNS.md

---

## ▶ Next Up

**Initialize project** — Now with codebase context

`/gsd-new-project`

---
```

## Success Criteria

- [ ] Stack analyzed (languages, frameworks, deps)
- [ ] Architecture documented (patterns, layers)
- [ ] Structure mapped (directories, entry points)
- [ ] Conventions identified (naming, style)
- [ ] Testing documented (framework, patterns)
- [ ] Integrations listed (APIs, services)
- [ ] Concerns identified (tech debt, issues)
- [ ] All 7 documents created
- [ ] Committed to git
