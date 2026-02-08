---
description: Research domain ecosystem before creating roadmap - discovers standard stacks, features, and pitfalls
---

# /gsd-research-project

Research the project's domain ecosystem before creating the roadmap. Discovers standard stacks, expected features, architecture patterns, and common pitfalls.

**Recommended before:** `/gsd-create-roadmap`

## Pre-flight Check

```bash
[ -f .planning/PROJECT.md ] || { echo "ERROR: No PROJECT.md. Run /gsd-new-project first."; exit 1; }
[ -d .planning/research ] && echo "RESEARCH_EXISTS" || echo "NO_RESEARCH"
```

If research exists, ask user: view/replace/cancel.

## Load Context

Read `.planning/PROJECT.md` to understand:
- What type of product
- Technology preferences
- Target users
- Constraints

## Research Process

Use @gsd-researcher methodology throughout.

### Step 1: Identify Research Domains

Based on PROJECT.md, identify what to research:

1. **Stack** — What technologies are standard for this type of product
2. **Features** — What users expect, table stakes vs differentiators
3. **Architecture** — How experts structure this type of system
4. **Pitfalls** — Common mistakes and how to avoid them

### Step 2: Execute Research

For each domain:

1. Search for current best practices (include year in searches)
2. Check official documentation
3. Cross-verify findings
4. Document with confidence levels

### Step 3: Create Research Outputs

Create `.planning/research/` directory with:

#### SUMMARY.md

```markdown
# Research Summary: [Project Name]

**Domain:** [type of product]
**Researched:** [date]
**Confidence:** [HIGH/MEDIUM/LOW]

## Executive Summary

[3-4 paragraphs synthesizing findings]

## Key Findings

**Stack:** [one-liner]
**Architecture:** [one-liner]
**Critical pitfall:** [most important]

## Implications for Roadmap

Based on research, suggested phase structure:

1. **[Phase name]** — [rationale]
2. **[Phase name]** — [rationale]

**Phase ordering rationale:** [why this order]

## Confidence Assessment

| Area | Level | Notes |
|------|-------|-------|
| Stack | [level] | [reason] |
| Features | [level] | [reason] |
| Architecture | [level] | [reason] |
| Pitfalls | [level] | [reason] |

## Gaps to Address

- [Areas needing more research]
```

#### STACK.md

```markdown
# Technology Stack

## Recommended Stack

| Layer | Technology | Version | Rationale |
|-------|------------|---------|-----------|
| Frontend | [tech] | [ver] | [why] |
| Backend | [tech] | [ver] | [why] |
| Database | [tech] | [ver] | [why] |
| Auth | [tech] | [ver] | [why] |

## Supporting Libraries

| Purpose | Library | Why |
|---------|---------|-----|
| [purpose] | [lib] | [rationale] |

## Alternatives Considered

| Instead of | Could Use | When |
|------------|-----------|------|
| [choice] | [alt] | [use case] |
```

#### FEATURES.md

```markdown
# Feature Landscape

## Table Stakes

Features users expect — must have for credibility:

| Feature | Why Required | Notes |
|---------|--------------|-------|
| [feature] | [reason] | [notes] |

## Differentiators

Features that set you apart:

| Feature | Competitive Advantage |
|---------|----------------------|
| [feature] | [why it matters] |

## Anti-Features

Things to explicitly NOT build:

| Feature | Why Not |
|---------|---------|
| [feature] | [reason] |
```

#### ARCHITECTURE.md

```markdown
# Architecture Patterns

## Recommended Structure

```
[directory structure]
```

## Key Patterns

### [Pattern Name]

**What:** [description]
**When to use:** [conditions]
**How:** [implementation notes]

## Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| [comp] | [what it does] | [other comps] |

## Data Flow

[Description of how data moves through system]
```

#### PITFALLS.md

```markdown
# Common Pitfalls

## Pitfall 1: [Name]

**What goes wrong:** [description]
**Why it happens:** [root cause]
**How to avoid:** [prevention]
**Warning signs:** [early indicators]

## Pitfall 2: [Name]

[Same structure]

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| [problem] | [custom solution] | [library] | [reason] |
```

## Commit

```bash
git add .planning/research/
git commit -m "docs: complete project research

Stack: [summary]
Key insight: [most important finding]"
```

## Completion

```
Research complete:

- Summary: .planning/research/SUMMARY.md
- Stack: .planning/research/STACK.md
- Features: .planning/research/FEATURES.md
- Architecture: .planning/research/ARCHITECTURE.md
- Pitfalls: .planning/research/PITFALLS.md

---

## ▶ Next Up

**Define requirements** — Use research to scope v1

`/gsd-define-requirements`

<sub>`/clear` first → fresh context window</sub>

---
```

## Success Criteria

- [ ] All four domains researched
- [ ] Findings documented with confidence levels
- [ ] All research files created
- [ ] Roadmap implications identified
- [ ] Committed to git
