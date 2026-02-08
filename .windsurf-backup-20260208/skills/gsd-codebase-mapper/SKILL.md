---
name: gsd-codebase-mapper
description: Codebase analysis methodology for brownfield projects. Creates 7 focused documents covering stack, architecture, structure, conventions, testing, integrations, and concerns.
---

# GSD Codebase Mapper

Systematic methodology for analyzing existing codebases.

## Analysis Domains

### 1. Stack (STACK.md)

**What to analyze:**
- Languages and versions
- Frameworks
- Dependencies (production and dev)
- Build tools

**How to discover:**
```bash
# Package managers
cat package.json
cat requirements.txt
cat Cargo.toml
cat go.mod

# Config files
cat tsconfig.json
cat .babelrc
cat webpack.config.js
```

### 2. Architecture (ARCHITECTURE.md)

**What to analyze:**
- Design patterns used
- Layer separation
- Data flow
- External service integration

**How to discover:**
```bash
# Directory structure
find . -type d -not -path '*/node_modules/*' -not -path '*/.git/*'

# Key files
find . -name "*.ts" -path "*/src/*" | head -20
```

**Patterns to look for:**
- MVC, MVVM, Clean Architecture
- Repository pattern
- Service layer
- Event-driven

### 3. Structure (STRUCTURE.md)

**What to analyze:**
- Directory layout
- Entry points
- Configuration locations
- Key file purposes

**How to discover:**
```bash
# Tree structure
tree -L 3 -I 'node_modules|.git'

# Entry points
cat package.json | grep -A5 "scripts"
cat package.json | grep "main"
```

### 4. Conventions (CONVENTIONS.md)

**What to analyze:**
- Naming patterns
- Code style
- Formatting rules
- Linting setup

**How to discover:**
```bash
# Linting config
cat .eslintrc*
cat .prettierrc*
cat tslint.json

# Sample files for patterns
head -50 src/**/*.ts
```

**Patterns to identify:**
- camelCase vs snake_case
- File naming (kebab-case, PascalCase)
- Import organization
- Comment style

### 5. Testing (TESTING.md)

**What to analyze:**
- Test framework
- Test location
- Coverage setup
- Test patterns

**How to discover:**
```bash
# Test config
cat jest.config.*
cat vitest.config.*
cat pytest.ini

# Test files
find . -name "*.test.*" -o -name "*.spec.*" | head -10
```

### 6. Integrations (INTEGRATIONS.md)

**What to analyze:**
- External APIs
- Third-party services
- Environment variables
- Configuration management

**How to discover:**
```bash
# Environment
cat .env.example
cat .env.sample

# API calls
grep -r "fetch\|axios\|http" src/ --include="*.ts" | head -20
```

### 7. Concerns (CONCERNS.md)

**What to analyze:**
- Technical debt
- Known issues
- Fragile areas
- Security considerations

**How to discover:**
```bash
# TODO/FIXME comments
grep -rn "TODO\|FIXME\|HACK\|XXX" src/

# Old dependencies
npm outdated
pip list --outdated

# Large files (potential complexity)
find . -name "*.ts" -exec wc -l {} \; | sort -rn | head -10
```

## Output Structure

Each document should follow the template in @gsd-templates.

**Key principles:**
- Be factual, not judgmental
- Note uncertainties
- Provide specific file paths
- Include relevant code snippets

## Quality Checklist

- [ ] All 7 domains analyzed
- [ ] Specific file paths referenced
- [ ] Patterns identified with examples
- [ ] Uncertainties noted
- [ ] Actionable insights provided
