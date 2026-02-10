---
description: Execute a single GSD plan with atomic commits and verification
---

# /gsd-execute-plan

Execute a PLAN.md file with atomic commits per task, deviation handling, and SUMMARY.md creation.

**Usage:** After `/gsd-plan-phase`, reference the plan path in your message.

## Pre-flight Checks

```bash
[ -f .planning/STATE.md ] || { echo "ERROR: No STATE.md. Run /gsd-create-roadmap first."; exit 1; }
```

User should specify which plan to execute, e.g.:
> Execute `.planning/phases/01-foundation/01-01-PLAN.md`

## Load Context

1. Read `.planning/STATE.md` — Current position and decisions
2. Read the specified PLAN.md — Tasks to execute
3. Read `.planning/PROJECT.md` — Project vision (for alignment)

## Execution Flow

### Step 1: Parse Plan

Extract from PLAN.md:
- Phase and plan numbers
- Objective
- Tasks (with files, action, verify, done criteria)
- Success criteria

### Step 2: Record Start Time

Note the start time for duration tracking in SUMMARY.md.

### Step 3: Execute Each Task

For each task in order:

**A. Understand the task**
- Read the action instructions
- Identify files to create/modify
- Note verification criteria

**B. Implement**
- Create/modify the specified files
- Follow the action instructions exactly
- Apply @gsd-executor skill methodology if needed

**C. Verify**
- Run the verification command
- Confirm done criteria met

**D. Commit atomically**
```bash
# Stage ONLY task-related files (never git add . or git add -A)
git add src/specific/file.ts
git add src/other/file.ts

# Commit with conventional format
git commit -m "{type}({phase}-{plan}): {task description}

- Key change 1
- Key change 2"
```

**Commit types:**
| Type | When |
|------|------|
| `feat` | New feature |
| `fix` | Bug fix |
| `test` | Tests only |
| `refactor` | Code cleanup |
| `docs` | Documentation |
| `chore` | Config/deps |

**E. Record commit hash**
```bash
git rev-parse --short HEAD
```

### Step 4: Handle Deviations

While executing, you WILL discover work not in the plan. Apply these rules:

**Rule 1: Auto-fix bugs**
- Trigger: Code doesn't work (errors, wrong output)
- Action: Fix immediately, document in Summary
- No permission needed

**Rule 2: Auto-add critical functionality**
- Trigger: Missing error handling, validation, security
- Action: Add immediately, document in Summary
- No permission needed

**Rule 3: Auto-fix blockers**
- Trigger: Can't proceed (missing dependency, wrong config)
- Action: Fix to unblock, document in Summary
- No permission needed

**Rule 4: ASK about architectural changes**
- Trigger: New tables, schema changes, new services
- Action: STOP and ask user before proceeding
- User decision required

### Step 5: Run Overall Verification

After all tasks complete:
- Run verification commands from plan
- Confirm all success criteria met

## Create SUMMARY.md

Create `{phase}-{plan}-SUMMARY.md` in the same directory as the plan:

```markdown
---
phase: [N]
plan: [M]
completed: [date]
duration: [time]
---

# Phase [N] Plan [M]: [Name] — Summary

## One-liner

[Substantive description: "JWT auth with refresh rotation using jose library"]

## What Was Built

[2-3 paragraphs describing what was implemented]

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | [Name] | [hash] | [files] |
| 2 | [Name] | [hash] | [files] |
| 3 | [Name] | [hash] | [files] |

## Key Files

**Created:**
- `path/file.ts` — [purpose]

**Modified:**
- `path/existing.ts` — [what changed]

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| [Choice] | [Why] |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule N - Type] [Description]**
- Found during: Task [N]
- Issue: [what was wrong]
- Fix: [what was done]
- Files: [affected files]
- Commit: [hash]

*Or if none:* "None — plan executed exactly as written."

## Verification Results

```
[Output of verification commands]
```

## Next Phase Readiness

- [What's now possible]
- [Any concerns for next phase]

---
*Completed: [timestamp]*
```

## Update STATE.md

```markdown
Phase: [N] of [total] ([Phase Name])
Plan: [M] of [plan count]
Status: [In progress | Phase complete]
Last activity: [date] - Completed {phase}-{plan}-PLAN.md

Progress:
[Update progress bar]
```

Add any decisions to the Decisions table.

## Final Commit

```bash
git add .planning/phases/[dir]/{phase}-{plan}-SUMMARY.md
git add .planning/STATE.md
git commit -m "docs({phase}-{plan}): complete [plan-name] plan

Tasks completed: [N]/[N]
- Task 1 name
- Task 2 name

SUMMARY: .planning/phases/[dir]/{phase}-{plan}-SUMMARY.md"
```

## Completion

```
## PLAN COMPLETE

**Plan:** {phase}-{plan}
**Tasks:** {completed}/{total}
**Duration:** {time}
**SUMMARY:** {path}

**Commits:**
- {hash}: {message}
- {hash}: {message}

---

## ▶ Next Up

[If more plans in phase:]
**Next plan in phase**

`/gsd-execute-plan`
Reference: `.planning/phases/[dir]/{phase}-{next}-PLAN.md`

[If phase complete:]
**Verify phase completion**

`/gsd-verify-work {phase}`

<sub>`/clear` first → fresh context window</sub>

---
```

## Success Criteria

- [ ] All tasks executed
- [ ] Each task committed individually
- [ ] Deviations documented
- [ ] SUMMARY.md created with substantive content
- [ ] STATE.md updated
- [ ] Final metadata commit made
