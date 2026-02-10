---
description: Resume work from previous session with full context restoration
---

# /gsd-resume-work

Resume work from previous session. Reads STATE.md and .continue-here to restore context.

## Process

### Step 1: Load State

```bash
cat .planning/STATE.md
cat .planning/.continue-here 2>/dev/null
```

### Step 2: Present Context

```markdown
# Resuming Work

**Last session:** [from STATE.md]
**Stopped at:** [from STATE.md]

## Position

**Phase:** [N] - [Name]
**Plan:** [M] - [Name]

## Progress

```
[progress bar from STATE.md]
```

## Where You Left Off

[From .continue-here or STATE.md]

## Recent Commits

```
[git log --oneline -5]
```

## Uncommitted Changes

```
[git status --short]
```
```

### Step 3: Recommend Next Action

Based on state, recommend:

**If mid-plan:**
```
---

## ▶ Continue Execution

Resume plan execution from task [N].

Reference the plan: `[path to current plan]`

---
```

**If plan complete, more in phase:**
```
---

## ▶ Next Plan

Execute next plan in phase.

`/gsd-execute-plan`

Reference: `[path to next plan]`

---
```

**If phase complete:**
```
---

## ▶ Next Phase

Plan or research next phase.

`/gsd-plan-phase [N+1]`

---
```

### Step 4: Clean Up

After resuming, remove .continue-here:

```bash
rm .planning/.continue-here 2>/dev/null
git add -u .planning/.continue-here
git commit -m "docs: resume work session" --allow-empty
```

## Success Criteria

- [ ] STATE.md read
- [ ] .continue-here read if exists
- [ ] Context presented clearly
- [ ] Appropriate next action recommended
