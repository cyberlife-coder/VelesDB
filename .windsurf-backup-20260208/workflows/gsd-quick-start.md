---
description: Combined project initialization - vision, requirements, and roadmap in one session (credit-efficient)
---

# /gsd-quick-start [project-name]

Initialize a project with vision, requirements, and roadmap in a single session. Credit-efficient alternative to running new-project, define-requirements, and create-roadmap separately.

**Best for:** Projects with <10 features, familiar domains, or when you want to move fast.

## Process

### Step 1: Pre-flight Check

```bash
# Check for existing project
[ -f .planning/PROJECT.md ] && { echo "Project exists. Use /gsd-progress instead."; exit 1; }

# Check for git
git rev-parse --git-dir >/dev/null 2>&1 || git init
```

### Step 2: Gather Vision (2-3 questions max)

> **What are you building?**
> Give me a one-sentence description and who it's for.

> **What are the 3-5 must-have features for v1?**
> Just the essentials to ship something useful.

> **Any technical constraints?**
> (Stack preferences, integrations, deployment target)

### Step 3: Create All Planning Documents

Based on responses, create in one pass:

**`.planning/PROJECT.md`**
```markdown
# [Project Name]

## Vision

[One paragraph from user's description]

## Target Users

[Who it's for]

## Technical Stack

[Inferred or specified stack]

## Constraints

[Any constraints mentioned]
```

**`.planning/REQUIREMENTS.md`**
```markdown
# Requirements

## v1 — Must Ship

| ID | Requirement | Phase | Status |
|----|-------------|-------|--------|
| REQ-01 | [Feature 1] | 1 | Pending |
| REQ-02 | [Feature 2] | 1-2 | Pending |
| REQ-03 | [Feature 3] | 2 | Pending |

## v2 — Later

- [Nice to have 1]
- [Nice to have 2]

## Out of Scope

- [Explicit exclusion]
```

**`.planning/ROADMAP.md`**
```markdown
# Roadmap

## Overview

**Project:** [name]
**Phases:** [count]
**Created:** [date]

## Progress

```
Phase 1  ░░░░░░░░░░  0%
Phase 2  ░░░░░░░░░░  0%
```

## Phases

### Phase 1: [Name]

**Goal:** [Observable outcome]
**Requirements:** REQ-01, REQ-02
**Success Criteria:**
- [ ] [Testable behavior]

---

### Phase 2: [Name]

**Goal:** [Observable outcome]
**Requirements:** REQ-03
**Success Criteria:**
- [ ] [Testable behavior]
```

**`.planning/STATE.md`**
```markdown
# Project State

## Current Position

**Phase:** 1 of [total]
**Plan:** Not started
**Status:** Ready to plan

## Progress

```
░░░░░░░░░░░░░░░░░░░░ 0%
```

## Last Activity

[date] — Project initialized via quick-start

## Decisions

None yet.

## Blockers

None.
```

### Step 4: Create Phase Directories

```bash
mkdir -p .planning/phases/01-[phase-1-slug]
mkdir -p .planning/phases/02-[phase-2-slug]
```

### Step 5: Commit

```bash
git add .planning/
git commit -m "docs: initialize project - [name]

Quick-start: vision, requirements, roadmap in one pass"
```

### Step 6: Completion

```
Project initialized:

- Vision: .planning/PROJECT.md
- Requirements: .planning/REQUIREMENTS.md  
- Roadmap: .planning/ROADMAP.md ([N] phases)
- State: .planning/STATE.md

---

## ▶ Next Up

**Plan Phase 1: [Name]**

`/gsd-plan-phase 1`

---
```

## When NOT to Use Quick-Start

- **Complex domains** — Use `/gsd-research-project` first
- **Unclear requirements** — Use separate `/gsd-define-requirements` for deeper discussion
- **Brownfield projects** — Use `/gsd-map-codebase` first
- **Large projects (10+ features)** — Standard flow gives more control

## Success Criteria

- [ ] Vision captured
- [ ] Requirements categorized (v1/v2/out)
- [ ] Phases defined with goals
- [ ] All documents created
- [ ] Committed to git
