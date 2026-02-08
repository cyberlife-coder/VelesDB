---
description: Generate detailed task plans for a specific phase
---

# /gsd-plan-phase [N]

Create detailed execution plans for a specific phase. Breaks phase into concrete, actionable tasks with verification criteria.

**Usage:** `/gsd-plan-phase 1` or `/gsd-plan-phase 2`

## Pre-flight Checks

```bash
PHASE_NUM=$1  # The phase number argument

[ -f .planning/ROADMAP.md ] || { echo "ERROR: No ROADMAP.md. Run /gsd-create-roadmap first."; exit 1; }

# Find phase directory
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }

# Check for existing plans
ls "$PHASE_DIR"/*-PLAN.md 2>/dev/null && echo "PLANS_EXIST" || echo "NO_PLANS"
```

## Load Context

Read these files to understand the phase:

1. `.planning/PROJECT.md` — Vision and approach
2. `.planning/ROADMAP.md` — Phase goal and requirements
3. `.planning/STATE.md` — Current position and decisions
4. `.planning/REQUIREMENTS.md` — Requirement details
5. `$PHASE_DIR/CONTEXT.md` (if exists) — User's vision for phase
6. `$PHASE_DIR/*-RESEARCH.md` (if exists) — Research findings
7. `.planning/codebase/` (if exists) — Existing code patterns

## Plan Generation Process

### Step 1: Extract Phase Requirements

From ROADMAP.md, get:
- Phase goal (observable outcome)
- Requirements mapped to this phase (REQ-IDs)
- Success criteria

### Step 2: Decompose into Work Units

Break the phase into logical work units. Each unit becomes a plan.

**Grouping principles:**
- **Single concern** — One plan, one subsystem
- **2-3 tasks max** — Keep plans atomic
- **Verifiable** — Each task has clear done criteria

### Step 3: Determine Plan Count

Based on config.json depth and phase complexity:
- **Quick:** 1-3 plans per phase
- **Standard:** 3-5 plans per phase
- **Comprehensive:** 5-10 plans per phase

### Step 4: Identify Dependencies

Determine which plans depend on others:
- **Wave 1:** Independent plans (can run first)
- **Wave 2:** Depends on Wave 1
- **Wave 3:** Depends on Wave 2

## Create PLAN.md Files

For each plan, create `$PHASE_DIR/{phase}-{plan}-PLAN.md`:

```markdown
---
phase: [N]
plan: [M]
name: [Plan Name]
wave: [1|2|3]
depends_on: [list of plan IDs or "none"]
autonomous: true
---

# Phase [N] Plan [M]: [Name]

## Objective

[What this plan accomplishes — 2-3 sentences]

## Context

**Requirements addressed:** REQ-XXX, REQ-YYY
**Phase goal contribution:** [How this advances the phase goal]

## Tasks

### Task 1: [Action-oriented name]

**Files:** `src/path/file.ts`, `src/other/file.ts`

**Action:**
[Specific implementation instructions]
- What to create/modify
- Key decisions (use X library, follow Y pattern)
- What to avoid and why

**Verify:**
```bash
[Command to verify task completion]
```

**Done when:**
- [Observable outcome 1]
- [Observable outcome 2]

---

### Task 2: [Action-oriented name]

**Files:** `src/path/file.ts`

**Action:**
[Implementation instructions]

**Verify:**
```bash
[Verification command]
```

**Done when:**
- [Outcome]

---

### Task 3: [Action-oriented name]

[Same structure...]

---

## Verification

After all tasks complete:

```bash
[Overall verification commands]
```

## Success Criteria

- [ ] [Specific testable outcome 1]
- [ ] [Specific testable outcome 2]
- [ ] [Specific testable outcome 3]

## Output

**Files created:**
- `path/to/file.ts` — [purpose]

**Files modified:**
- `path/to/existing.ts` — [what changed]
```

## Task Writing Guidelines

**Good task:**
```markdown
### Task 1: Create login endpoint with JWT

**Files:** `src/app/api/auth/login/route.ts`

**Action:**
POST endpoint accepting {email, password}. 
- Query User by email
- Compare password with bcrypt
- On match: create JWT with jose library, set as httpOnly cookie, return 200
- On mismatch: return 401

**Verify:**
```bash
curl -X POST localhost:3000/api/auth/login -d '{"email":"test@test.com","password":"test"}' -H "Content-Type: application/json"
```

**Done when:**
- Valid credentials → 200 + Set-Cookie header
- Invalid credentials → 401
```

**Bad task:**
```markdown
### Task 1: Add authentication

**Action:** Implement auth

**Done when:** Auth works
```

## Update STATE.md

After creating plans:

```markdown
Phase: [N] of [total] ([Phase Name])
Plan: 0 of [plan count] (Not started)
Status: Ready to execute
Last activity: [date] - Created [count] plans for phase [N]
```

## Commit

```bash
git add "$PHASE_DIR"/*-PLAN.md .planning/STATE.md
git commit -m "docs(phase-$PHASE_NUM): create execution plans

Plans: [count]
Wave 1: [list]
Wave 2: [list]"
```

## Completion

```
Phase [N] planned:

- Plans: [count] created
- Wave 1: [plan names] (independent)
- Wave 2: [plan names] (sequential)

---

## ▶ Next Up

**Execute first plan**

`/gsd-execute-plan`

Then reference: `.planning/phases/[dir]/[phase]-01-PLAN.md`

<sub>`/clear` first → fresh context window</sub>

---

**Also available:**
- Review plans: `cat .planning/phases/[dir]/*-PLAN.md`
- `/gsd-execute-phase [N]` — Execute all plans sequentially

---
```

## Success Criteria

- [ ] Phase directory and requirements validated
- [ ] Plans decomposed into 2-3 task units
- [ ] Each task has files, action, verify, done criteria
- [ ] Wave dependencies identified
- [ ] STATE.md updated
- [ ] All plans committed to git
