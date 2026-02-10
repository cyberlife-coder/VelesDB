---
name: gsd-researcher
description: Research methodology for investigating technologies, ecosystems, and implementation patterns. Invoke when researching before planning.
---

# GSD Researcher

Systematic research methodology for discovering standard stacks, architecture patterns, and pitfalls.

## Core Philosophy

**Claude's training data is 6-18 months stale.** Treat pre-existing knowledge as hypothesis, not fact.

- Verify before asserting
- Date your knowledge
- Prefer current sources
- Flag uncertainty

## Research Modes

### Mode 1: Ecosystem
**When:** "What tools exist for X?"

- What libraries/frameworks exist
- What's the standard stack
- What's SOTA vs deprecated

### Mode 2: Feasibility
**When:** "Can we do X?"

- Is it technically achievable
- What constraints exist
- What's the effort/complexity

### Mode 3: Implementation
**When:** "How do we implement X?"

- Specific patterns
- Code examples
- Configuration
- Common pitfalls

### Mode 4: Comparison
**When:** "Compare A vs B"

- Feature comparison
- Performance comparison
- Clear recommendation

## Source Hierarchy

| Level | Sources | Trust |
|-------|---------|-------|
| HIGH | Official docs, releases | State as fact |
| MEDIUM | Multiple sources agree | State with attribution |
| LOW | Single source, unverified | Flag for validation |

**Priority order:**
1. Official documentation
2. Official GitHub (README, releases)
3. Web search (verified)
4. Web search (unverified) — mark LOW

## Research Process

### Step 1: Identify Domains

- Core technology
- Ecosystem/stack
- Patterns
- Pitfalls
- SOTA check

### Step 2: Execute Research

For each domain:
1. Check official docs first
2. Search for current best practices (include year)
3. Cross-verify findings
4. Document confidence levels

### Step 3: Quality Check

- [ ] All items investigated
- [ ] Negative claims verified
- [ ] Multiple sources for critical claims
- [ ] URLs provided
- [ ] Publication dates checked
- [ ] Confidence levels assigned

## Output: RESEARCH.md

```markdown
# Phase [X]: [Name] - Research

**Domain:** [technology]
**Confidence:** [HIGH/MEDIUM/LOW]

## Summary

[What was researched, standard approach, key recommendations]

**Primary recommendation:** [one-liner]

## Standard Stack

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| [name] | [ver] | [purpose] | [why] |

## Architecture Patterns

### Recommended Structure
```
src/
├── [folder]/  # [purpose]
└── [folder]/  # [purpose]
```

### Pattern: [Name]
**What:** [description]
**When:** [conditions]
**Example:** [code]

## Don't Hand-Roll

| Problem | Don't Build | Use Instead |
|---------|-------------|-------------|
| [problem] | [custom] | [library] |

## Common Pitfalls

### Pitfall: [Name]
**What goes wrong:** [description]
**How to avoid:** [prevention]

## Sources

- [URL] — [what, confidence]
```

## Verification Protocol

### Red Flags

- Every investigation succeeds perfectly (unrealistic)
- All findings equally certain (no confidence levels)
- "According to docs" without URL
- "X cannot do Y" without citation

### Checklist

- [ ] All enumerated items investigated
- [ ] Negative claims verified with official docs
- [ ] Multiple sources for critical claims
- [ ] URLs provided for authoritative sources
- [ ] Publication dates checked
- [ ] Confidence levels assigned honestly
- [ ] "What might I have missed?" review
