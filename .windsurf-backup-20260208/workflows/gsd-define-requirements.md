---
description: Define and scope project requirements into v1, v2, and out-of-scope categories
---

# /gsd-define-requirements

Scope what's v1, what's v2, and what's out of scope. Creates REQUIREMENTS.md with checkable requirements and traceability.

## Pre-flight Checks

```bash
[ -f .planning/PROJECT.md ] || { echo "ERROR: No PROJECT.md. Run /gsd-new-project first."; exit 1; }
[ -f .planning/REQUIREMENTS.md ] && echo "REQUIREMENTS_EXISTS" || echo "NO_REQUIREMENTS"
```

If REQUIREMENTS.md exists, ask:
> Requirements already exist. What would you like to do?
> - **View existing** — Show current requirements
> - **Replace** — Create new requirements (will overwrite)
> - **Cancel** — Keep existing

## Load Context

Read:
- `.planning/PROJECT.md` — Project vision and initial requirements
- `.planning/research/SUMMARY.md` (if exists) — Research findings
- `.planning/codebase/` (if exists) — Existing capabilities

## Requirements Gathering

### Step 1: Extract from PROJECT.md

Pull the Active requirements from PROJECT.md as starting point.

### Step 2: Expand with User

Ask:
> Looking at the requirements from PROJECT.md, let's make sure we have everything for v1. What else needs to be in the first version?

Follow threads to capture:
- Core functionality (must work)
- User-facing features (what they interact with)
- Technical requirements (performance, security, etc.)

### Step 3: Scope into Tiers

Present all gathered requirements and ask user to categorize:

**v1 (MVP):** Must ship for product to be usable
**v2 (Next):** Important but can wait
**Out of Scope:** Explicitly not building

For each requirement, help user decide tier by asking:
- "Can the product work without this?"
- "Would users accept this missing initially?"
- "Is this core to the value proposition?"

## Create REQUIREMENTS.md

```markdown
# Requirements

## Overview

**Project:** [name]
**Defined:** [date]
**Total:** [count] requirements ([v1 count] v1, [v2 count] v2)

## v1 Requirements (MVP)

### Core Functionality

| ID | Requirement | Priority | Phase |
|----|-------------|----------|-------|
| REQ-001 | [Description] | Must | TBD |
| REQ-002 | [Description] | Must | TBD |

### User Features

| ID | Requirement | Priority | Phase |
|----|-------------|----------|-------|
| REQ-003 | [Description] | Must | TBD |

### Technical Requirements

| ID | Requirement | Priority | Phase |
|----|-------------|----------|-------|
| REQ-004 | [Description] | Must | TBD |

## v2 Requirements (Next Version)

| ID | Requirement | Rationale for v2 |
|----|-------------|------------------|
| REQ-101 | [Description] | [Why not v1] |

## Out of Scope

| Item | Reason |
|------|--------|
| [Feature] | [Why excluded] |

## Traceability

| Requirement | Phase | Plan | Status |
|-------------|-------|------|--------|
| REQ-001 | TBD | TBD | Pending |
| REQ-002 | TBD | TBD | Pending |

---
*Last updated: [date]*
```

## Requirement ID Format

- **REQ-001 to REQ-099:** v1 requirements
- **REQ-101 to REQ-199:** v2 requirements
- Group by area: Core (001-019), Features (020-049), Technical (050-079)

## Commit

```bash
git add .planning/REQUIREMENTS.md
git commit -m "docs: define requirements

v1: [count] requirements
v2: [count] requirements
Out of scope: [count] items"
```

## Completion

```
Requirements defined:

- Requirements: .planning/REQUIREMENTS.md
- v1 scope: [count] requirements
- v2 backlog: [count] requirements

---

## ▶ Next Up

**Create roadmap** — Map requirements to phases

`/gsd-create-roadmap`

<sub>`/clear` first → fresh context window</sub>

---

**Also available:**
- `/gsd-research-project` — Research ecosystem first
- Review requirements: `cat .planning/REQUIREMENTS.md`

---
```

## Success Criteria

- [ ] PROJECT.md validated
- [ ] All requirements have unique IDs
- [ ] Clear v1/v2/out-of-scope separation
- [ ] Priority assigned to v1 requirements
- [ ] Traceability table initialized
- [ ] Committed to git
