---
description: Create handoff file when stopping mid-phase for session continuity
---

# /gsd-pause-work

Create context handoff when pausing work mid-phase. Updates STATE.md and creates `.continue-here` file.

## Process

### Step 1: Gather Current State

```bash
# Current phase and plan
grep "^Phase:" .planning/STATE.md
grep "^Plan:" .planning/STATE.md

# Recent commits
git log --oneline -5

# Uncommitted changes
git status --short
```

### Step 2: Create .continue-here

Create `.planning/.continue-here`:

```markdown
# Continue Here

**Paused:** [timestamp]
**Session:** [date]

## Position

**Phase:** [N] - [Name]
**Plan:** [M] - [Name]
**Task:** [current task number]

## In Progress

[What was being worked on]

## Recent Commits

```
[git log output]
```

## Uncommitted Changes

```
[git status output]
```

## Next Steps

1. [Immediate next action]
2. [Following action]

## Context Notes

[Any important context for resuming]

## Files to Review

- `[path]` — [why relevant]
- `[path]` — [why relevant]
```

### Step 3: Update STATE.md

Update Session Continuity section:

```markdown
## Session Continuity

**Last session:** [timestamp]
**Stopped at:** [description]
**Resume file:** .planning/.continue-here
```

### Step 4: Commit

```bash
git add .planning/.continue-here .planning/STATE.md
git commit -m "docs: pause work - [brief description]

Position: Phase [N] Plan [M]
Resume with: /gsd-resume-work"
```

## Completion

```
Work paused:

- Continue file: .planning/.continue-here
- State updated: .planning/STATE.md

---

## To Resume

`/gsd-resume-work`

This will restore context and show next steps.

---
```
