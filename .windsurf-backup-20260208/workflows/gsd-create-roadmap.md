---
description: Create project roadmap with phases mapped to requirements
---

# /gsd-create-roadmap

Create project roadmap with phase breakdown. Phases map to requirements from REQUIREMENTS.md.

## Pre-flight Checks

```bash
[ -f .planning/PROJECT.md ] || { echo "ERROR: No PROJECT.md. Run /gsd-new-project first."; exit 1; }
[ -f .planning/REQUIREMENTS.md ] || { echo "ERROR: No REQUIREMENTS.md. Run /gsd-define-requirements first."; exit 1; }
[ -f .planning/ROADMAP.md ] && echo "ROADMAP_EXISTS" || echo "NO_ROADMAP"
```

If ROADMAP.md exists, ask user what to do (view/replace/cancel).

## Load Context

Read:
- `.planning/PROJECT.md` — Vision and approach
- `.planning/REQUIREMENTS.md` — v1 requirements to map
- `.planning/config.json` — Depth setting
- `.planning/research/SUMMARY.md` (if exists) — Research implications

## Phase Identification

### Step 1: Group Requirements

Analyze v1 requirements and group by:
- **Technical dependency** — What must exist before what?
- **User flow** — What enables what experience?
- **Risk** — What should be validated early?

### Step 2: Determine Phase Count

Based on config.json depth:
- **Quick:** 3-5 phases
- **Standard:** 5-8 phases
- **Comprehensive:** 8-12 phases

### Step 3: Define Phases

For each phase, identify:
- **Name:** Action-oriented (e.g., "Authentication System")
- **Goal:** What's TRUE when phase completes (observable outcome)
- **Requirements:** Which REQ-IDs this phase addresses
- **Dependencies:** Which phases must complete first

### Step 4: Validate Coverage

**CRITICAL:** Every v1 requirement must map to exactly one phase.

```
Check: All REQ-001 through REQ-0XX appear in exactly one phase
```

If orphaned requirements found → add to appropriate phase or create new phase.

## Create ROADMAP.md

```markdown
# Roadmap

## Overview

**Project:** [name]
**Milestone:** v1.0
**Created:** [date]
**Phases:** [count]

## Progress

```
Phase 1  ░░░░░░░░░░  0%
Phase 2  ░░░░░░░░░░  0%
Phase 3  ░░░░░░░░░░  0%
```

## Phases

### Phase 1: [Name]

**Goal:** [Observable outcome — what's TRUE when done]

**Requirements:** REQ-001, REQ-002, REQ-003

**Success Criteria:**
- [ ] [Specific testable behavior 1]
- [ ] [Specific testable behavior 2]
- [ ] [Specific testable behavior 3]

**Research flag:** [None | Recommended | Required]

---

### Phase 2: [Name]

**Goal:** [Observable outcome]

**Requirements:** REQ-004, REQ-005

**Depends on:** Phase 1

**Success Criteria:**
- [ ] [Behavior 1]
- [ ] [Behavior 2]

**Research flag:** [flag]

---

[Continue for all phases...]

## Requirement Coverage

| Requirement | Phase | Status |
|-------------|-------|--------|
| REQ-001 | 1 | Pending |
| REQ-002 | 1 | Pending |
| REQ-003 | 2 | Pending |

## Timeline Estimate

| Phase | Complexity | Est. Plans |
|-------|------------|------------|
| 1 | [Low/Med/High] | [1-3] |
| 2 | [Low/Med/High] | [1-3] |

---
*Last updated: [date]*
```

## Create STATE.md

```markdown
# Project State

## Current Position

**Milestone:** v1.0
**Phase:** 1 of [total] ([Phase 1 Name])
**Plan:** Not started
**Status:** Ready to plan

**Progress:**
```
░░░░░░░░░░░░░░░░░░░░ 0%
```

**Last activity:** [date] - Roadmap created

## Session Continuity

**Last session:** [date]
**Stopped at:** Roadmap creation
**Resume file:** None

## Decisions

| Decision | Rationale | Date | Phase |
|----------|-----------|------|-------|
| [From PROJECT.md] | [Why] | [date] | — |

## Blockers & Concerns

None yet.

## Context Notes

[Any important context for future sessions]

---
*Auto-updated by GSD workflows*
```

## Create Phase Directories

```bash
mkdir -p .planning/phases/01-[phase-1-name]
mkdir -p .planning/phases/02-[phase-2-name]
# ... for each phase
```

## Update REQUIREMENTS.md Traceability

Update the Traceability table with phase assignments:

```markdown
| Requirement | Phase | Plan | Status |
|-------------|-------|------|--------|
| REQ-001 | 1 | TBD | Pending |
| REQ-002 | 1 | TBD | Pending |
```

## Commit

```bash
git add .planning/ROADMAP.md .planning/STATE.md .planning/REQUIREMENTS.md .planning/phases/
git commit -m "docs: create roadmap

[count] phases for v1.0
Requirements mapped to phases"
```

## Completion

```
Roadmap created:

- Roadmap: .planning/ROADMAP.md
- State: .planning/STATE.md
- Phases: [count] phases defined

---

## ▶ Next Up

**Phase 1: [Name]** — [Goal from ROADMAP.md]

`/gsd-plan-phase 1`

<sub>`/clear` first → fresh context window</sub>

---

**Also available:**
- `/gsd-discuss-phase 1` — Gather context first
- `/gsd-research-phase 1` — Investigate unknowns
- Review roadmap: `cat .planning/ROADMAP.md`

---
```

## Success Criteria

- [ ] PROJECT.md and REQUIREMENTS.md validated
- [ ] All v1 requirements mapped to phases (no orphans)
- [ ] Each phase has observable goal and success criteria
- [ ] ROADMAP.md created with phases
- [ ] STATE.md initialized
- [ ] REQUIREMENTS.md traceability updated
- [ ] Phase directories created
- [ ] Committed to git
