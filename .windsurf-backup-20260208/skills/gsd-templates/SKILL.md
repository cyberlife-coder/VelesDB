---
name: gsd-templates
description: Output templates for GSD artifacts - PROJECT.md, ROADMAP.md, PLAN.md, SUMMARY.md, STATE.md. Invoke when creating GSD documents.
---

# GSD Templates

Reference templates for all GSD output files.

## PROJECT.md Template

```markdown
# [Project Name]

## Vision

[2-3 sentences capturing the core idea and why it matters]

## Problem Statement

[What problem this solves, for whom]

## Target User

[Who uses this, what they care about]

## Success Criteria

- [Observable outcome 1]
- [Observable outcome 2]
- [Observable outcome 3]

## Technical Approach

**Stack:** [Technologies and why]
**Architecture:** [High-level approach]
**Constraints:** [Limitations or requirements]

## Requirements

### Validated
(None yet — ship to validate)

### Active
- [ ] [Requirement 1]
- [ ] [Requirement 2]

### Out of Scope
- [Exclusion] — [why]

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| [Choice] | [Why] | Pending |

---
*Last updated: [date]*
```

## REQUIREMENTS.md Template

```markdown
# Requirements

## Overview

**Project:** [name]
**Defined:** [date]
**Total:** [count] requirements

## v1 Requirements (MVP)

| ID | Requirement | Priority | Phase |
|----|-------------|----------|-------|
| REQ-001 | [Description] | Must | TBD |

## v2 Requirements

| ID | Requirement | Rationale for v2 |
|----|-------------|------------------|
| REQ-101 | [Description] | [Why not v1] |

## Out of Scope

| Item | Reason |
|------|--------|
| [Feature] | [Why] |

## Traceability

| Requirement | Phase | Plan | Status |
|-------------|-------|------|--------|
| REQ-001 | TBD | TBD | Pending |

---
*Last updated: [date]*
```

## ROADMAP.md Template

```markdown
# Roadmap

## Overview

**Project:** [name]
**Milestone:** v1.0
**Phases:** [count]

## Progress

```
Phase 1  ░░░░░░░░░░  0%
Phase 2  ░░░░░░░░░░  0%
```

## Phases

### Phase 1: [Name]

**Goal:** [Observable outcome]
**Requirements:** REQ-001, REQ-002
**Success Criteria:**
- [ ] [Testable behavior 1]
- [ ] [Testable behavior 2]

---

### Phase 2: [Name]

**Goal:** [Outcome]
**Requirements:** REQ-003
**Depends on:** Phase 1
**Success Criteria:**
- [ ] [Behavior]

---

## Requirement Coverage

| Requirement | Phase | Status |
|-------------|-------|--------|
| REQ-001 | 1 | Pending |

---
*Last updated: [date]*
```

## STATE.md Template

```markdown
# Project State

## Current Position

**Milestone:** v1.0
**Phase:** 1 of [total] ([Name])
**Plan:** Not started
**Status:** Ready to plan

**Progress:**
```
░░░░░░░░░░░░░░░░░░░░ 0%
```

**Last activity:** [date] - [description]

## Session Continuity

**Last session:** [date]
**Stopped at:** [description]
**Resume file:** None

## Decisions

| Decision | Rationale | Date | Phase |
|----------|-----------|------|-------|
| [Choice] | [Why] | [date] | — |

## Blockers & Concerns

None yet.

## Context Notes

[Important context for future sessions]

---
*Auto-updated by GSD workflows*
```

## PLAN.md Template

```markdown
---
phase: [N]
plan: [M]
name: [Name]
wave: [1|2|3]
depends_on: [plans or "none"]
autonomous: true
---

# Phase [N] Plan [M]: [Name]

## Objective

[What this plan accomplishes]

## Context

**Requirements:** REQ-XXX
**Phase goal contribution:** [How this advances the goal]

## Tasks

### Task 1: [Action-oriented name]

**Files:** `src/path/file.ts`

**Action:**
[Specific instructions]

**Verify:**
```bash
[verification command]
```

**Done when:**
- [Outcome]

---

### Task 2: [Name]

[Same structure]

---

## Verification

```bash
[Overall verification]
```

## Success Criteria

- [ ] [Testable outcome]
```

## SUMMARY.md Template

```markdown
---
phase: [N]
plan: [M]
completed: [date]
duration: [time]
---

# Phase [N] Plan [M]: [Name] — Summary

## One-liner

[Substantive: "JWT auth with refresh rotation using jose"]

## What Was Built

[2-3 paragraphs]

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | [Name] | [hash] | [files] |

## Key Files

**Created:**
- `path` — [purpose]

**Modified:**
- `path` — [changes]

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| [Choice] | [Why] |

## Deviations from Plan

[Auto-fixes or "None"]

## Verification Results

```
[Output]
```

---
*Completed: [timestamp]*
```

## VERIFICATION.md Template

```markdown
---
phase: [N]-[name]
verified: [timestamp]
status: [passed | gaps_found | human_needed]
score: N/M verified
---

# Phase [N] Verification Report

**Goal:** [from ROADMAP]
**Status:** [status]

## Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | [truth] | ✓/✗ | [evidence] |

## Required Artifacts

| Artifact | Exists | Substantive | Wired |
|----------|--------|-------------|-------|
| `path` | ✓ | ✓ | ✓ |

## Key Links

| From | To | Status |
|------|----|--------|
| [component] | [api] | [status] |

## Gaps

[If any]

## Human Verification

[If needed]
```
