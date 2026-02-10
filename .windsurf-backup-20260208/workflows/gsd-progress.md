---
description: Check project status and get intelligent next action recommendation
---

# /gsd-progress

Check project status and intelligently route to next action. Shows progress, recent work, and recommends what to do next.

## Pre-flight Check

```bash
[ -d .planning ] || { echo "No .planning/ directory. Run /gsd-new-project to start."; exit 1; }
```

## Gather Status

### Step 1: Read State

```bash
cat .planning/STATE.md 2>/dev/null
```

Extract:
- Current phase and plan
- Last activity
- Status

### Step 2: Calculate Progress

```bash
# Count total plans across all phases
TOTAL_PLANS=$(find .planning/phases -name "*-PLAN.md" 2>/dev/null | wc -l)

# Count completed plans (those with SUMMARY.md)
COMPLETED_PLANS=$(find .planning/phases -name "*-SUMMARY.md" 2>/dev/null | wc -l)

# Calculate percentage
if [ "$TOTAL_PLANS" -gt 0 ]; then
  PROGRESS=$((COMPLETED_PLANS * 100 / TOTAL_PLANS))
else
  PROGRESS=0
fi
```

### Step 3: Check Current Phase Status

```bash
# Find current phase from STATE.md
CURRENT_PHASE=$(grep "^Phase:" .planning/STATE.md | head -1)

# Check if current phase has incomplete plans
```

### Step 4: Get Recent Activity

```bash
# Find most recent SUMMARY.md files
ls -t .planning/phases/*/*-SUMMARY.md 2>/dev/null | head -3
```

## Present Status

```markdown
# Project Progress

**Project:** [from PROJECT.md]
**Milestone:** [from ROADMAP.md]

## Progress

```
[████████░░░░░░░░░░░░] 40%
```

**Phases:** [completed]/[total]
**Plans:** [completed]/[total]

## Current Position

**Phase:** [N] of [total] — [Phase Name]
**Plan:** [M] of [phase total]
**Status:** [status]

**Last activity:** [date] — [description]

## Recent Work

| When | What | Files |
|------|------|-------|
| [date] | [summary from SUMMARY.md] | [key files] |
| [date] | [summary] | [files] |

## Decisions Made

| Decision | Phase | Date |
|----------|-------|------|
| [decision] | [N] | [date] |

## Blockers & Concerns

[From STATE.md or "None"]
```

## Route to Next Action

Based on current state, recommend next action:

### State: No Plans Yet

```
---

## ▶ Next Up

**Plan phase [N]** — [Phase Name]

`/gsd-plan-phase [N]`

<sub>`/clear` first → fresh context window</sub>

---
```

### State: Plans Exist, Not Started

```
---

## ▶ Next Up

**Execute first plan**

`/gsd-execute-plan`

Reference: `[path to first incomplete plan]`

<sub>`/clear` first → fresh context window</sub>

---
```

### State: Mid-Plan Execution

```
---

## ▶ Resume Work

**Continue plan execution**

`/gsd-resume-work`

Or start fresh with `/gsd-execute-plan`

---
```

### State: Plan Complete, More in Phase

```
---

## ▶ Next Up

**Execute next plan**

`/gsd-execute-plan`

Reference: `[path to next incomplete plan]`

<sub>`/clear` first → fresh context window</sub>

---
```

### State: Phase Complete, More Phases

```
---

## ▶ Next Up

**Plan next phase**

`/gsd-plan-phase [N+1]`

<sub>`/clear` first → fresh context window</sub>

---
```

### State: All Phases Complete

```
---

## 🎉 Milestone Ready!

All phases complete.

**Audit milestone** — Verify everything works together

`/gsd-audit-milestone`

Or skip to: `/gsd-complete-milestone [version]`

---
```

## Quick Actions

Always include:

```
---

**Quick actions:**
- `/gsd-help` — Command reference
- `/gsd-add-todo` — Capture an idea
- `/gsd-debug` — Debug an issue
- `cat .planning/STATE.md` — Full state
- `cat .planning/ROADMAP.md` — Full roadmap

---
```

## Success Criteria

- [ ] State read and parsed
- [ ] Progress calculated accurately
- [ ] Current position identified
- [ ] Recent activity summarized
- [ ] Appropriate next action recommended
