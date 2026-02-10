---
description: Create new milestone with phases for an existing project
---

# /gsd-new-milestone [name]

Create a new milestone with phases for an existing project.

**Usage:** `/gsd-new-milestone "v2.0 Features"`

## Process

### Step 1: Validate

```bash
[ -f .planning/PROJECT.md ] || { echo "ERROR: No PROJECT.md"; exit 1; }
```

### Step 2: Get Milestone Context

If no name provided, ask:
> What's the focus of this milestone?

Then ask:
> What are the main goals for this milestone?

### Step 3: Update PROJECT.md

Add new requirements to Active section (from v2 or new):
- Move relevant v2 requirements to Active
- Add any new requirements discovered

### Step 4: Create New ROADMAP.md

```markdown
# Roadmap

## Overview

**Project:** [name]
**Milestone:** [milestone name]
**Created:** [date]
**Phases:** [count]

## Progress

```
Phase 1  ░░░░░░░░░░  0%
```

## Phases

### Phase 1: [Name]

**Goal:** [outcome]
**Requirements:** [REQ-IDs]
**Success Criteria:**
- [ ] [behavior]

---

[Continue for all phases]
```

### Step 5: Reset STATE.md

```markdown
# Project State

## Current Position

**Milestone:** [milestone name]
**Phase:** 1 of [total]
**Plan:** Not started
**Status:** Ready to plan

**Progress:**
```
░░░░░░░░░░░░░░░░░░░░ 0%
```

**Last activity:** [date] - New milestone created

## Session Continuity

**Last session:** [date]
**Stopped at:** Milestone creation

## Decisions

[Carry forward relevant decisions]

## Blockers & Concerns

None yet.
```

### Step 6: Create Phase Directories

```bash
mkdir -p .planning/phases/01-[phase-1-name]
mkdir -p .planning/phases/02-[phase-2-name]
```

### Step 7: Commit

```bash
git add .planning/
git commit -m "docs: create milestone - [name]

Phases: [count]
Focus: [brief description]"
```

### Step 8: Completion

```
Milestone created:

- Milestone: [name]
- Phases: [count]
- Roadmap: .planning/ROADMAP.md

---

## ▶ Next Up

**Phase 1: [Name]**

`/gsd-plan-phase 1`

<sub>`/clear` first → fresh context window</sub>

---
```
