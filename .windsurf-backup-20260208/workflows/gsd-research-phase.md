---
description: Deep research for unfamiliar domains before planning a specific phase
---

# /gsd-research-phase [N]

Comprehensive ecosystem research for a specific phase. Use for niche/complex domains (3D, games, audio, ML, etc.).

**Usage:** `/gsd-research-phase 3`

## Pre-flight Check

```bash
PHASE_NUM=$1
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }
```

## Load Context

1. `.planning/ROADMAP.md` — Phase goal and requirements
2. `.planning/PROJECT.md` — Overall vision
3. `$PHASE_DIR/CONTEXT.md` (if exists) — User's vision for phase

## Research Process

Use @gsd-researcher methodology.

### Step 1: Identify Research Questions

From phase goal, extract:
- What technologies are needed?
- What patterns are standard?
- What pitfalls exist?

### Step 2: Execute Research

For each question:
1. Check official documentation
2. Search current best practices
3. Cross-verify findings
4. Document with confidence levels

### Step 3: Create RESEARCH.md

Create `$PHASE_DIR/{phase}-RESEARCH.md`:

```markdown
# Phase [N]: [Name] - Research

**Researched:** [date]
**Domain:** [technology]
**Confidence:** [HIGH/MEDIUM/LOW]

## Summary

[2-3 paragraphs: what researched, standard approach, key recommendations]

**Primary recommendation:** [one-liner]

## Standard Stack

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| [name] | [ver] | [purpose] | [why] |

**Installation:**
```bash
npm install [packages]
```

## Architecture Patterns

### Recommended Structure
```
src/
├── [folder]/  # [purpose]
```

### Pattern: [Name]
**What:** [description]
**When:** [conditions]
**Example:**
```typescript
[code]
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead |
|---------|-------------|-------------|
| [problem] | [custom] | [library] |

## Common Pitfalls

### Pitfall: [Name]
**What:** [description]
**Why:** [root cause]
**Avoid:** [prevention]

## Code Examples

### [Operation]
```typescript
// From official docs
[code]
```

## State of the Art

| Old Approach | Current | Impact |
|--------------|---------|--------|
| [old] | [new] | [what it means] |

## Open Questions

1. **[Question]**
   - Known: [partial info]
   - Unclear: [gap]
   - Recommendation: [how to handle]

## Sources

**HIGH confidence:**
- [URL] — [what]

**MEDIUM confidence:**
- [URL] — [what]

**LOW confidence:**
- [URL] — [needs validation]
```

## Commit

```bash
git add "$PHASE_DIR"/*-RESEARCH.md
git commit -m "docs(phase-$PHASE_NUM): complete research

Domain: [domain]
Key finding: [most important]"
```

## Completion

```
Phase [N] research complete:

- Research: [path to RESEARCH.md]
- Confidence: [level]
- Key recommendation: [one-liner]

---

## ▶ Next Up

**Plan the phase** — Use research to create execution plans

`/gsd-plan-phase [N]`

<sub>`/clear` first → fresh context window</sub>

---
```

## Success Criteria

- [ ] Phase goal analyzed
- [ ] Research questions identified
- [ ] Standard stack documented
- [ ] Patterns documented
- [ ] Pitfalls identified
- [ ] RESEARCH.md created with confidence levels
- [ ] Committed to git
