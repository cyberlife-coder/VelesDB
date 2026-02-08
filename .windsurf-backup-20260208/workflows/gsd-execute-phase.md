---
description: Execute all plans in a phase sequentially with verification
---

# /gsd-execute-phase [N]

Execute all incomplete plans in a phase sequentially. Since Windsurf doesn't have subagent spawning, plans run one after another with context resets between them.

**Usage:** `/gsd-execute-phase 1`

## Pre-flight Checks

```bash
PHASE_NUM=$1

[ -f .planning/ROADMAP.md ] || { echo "ERROR: No ROADMAP.md"; exit 1; }

# Find phase directory
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }

# Count plans and summaries
PLAN_COUNT=$(ls "$PHASE_DIR"/*-PLAN.md 2>/dev/null | wc -l)
SUMMARY_COUNT=$(ls "$PHASE_DIR"/*-SUMMARY.md 2>/dev/null | wc -l)

echo "Plans: $PLAN_COUNT, Completed: $SUMMARY_COUNT"
```

## Execution Strategy

Since Windsurf doesn't spawn parallel subagents, execution is sequential:

1. Identify all PLAN.md files without matching SUMMARY.md
2. Execute each in order (respecting wave dependencies)
3. Recommend `/clear` between plans for context management
4. Verify phase goal after all plans complete

## Process

### Step 1: Discover Incomplete Plans

```bash
# List all plans
for plan in "$PHASE_DIR"/*-PLAN.md; do
  base=$(basename "$plan" -PLAN.md)
  summary="$PHASE_DIR/${base}-SUMMARY.md"
  if [ ! -f "$summary" ]; then
    echo "INCOMPLETE: $plan"
  fi
done
```

### Step 2: Sort by Wave

Read `wave` from each plan's frontmatter and group:
- Wave 1 plans first (independent)
- Wave 2 plans after Wave 1 complete
- Wave 3 plans after Wave 2 complete

### Step 3: Execute Plans

For each incomplete plan in wave order:

**Option A: Execute in current context** (small plans)
- Run `/gsd-execute-plan` workflow inline
- Good for 1-2 task plans

**Option B: Guide user through sequential execution** (recommended)

Present to user:
```
## Phase [N] Execution Plan

**Plans to execute:** [count]
**Completed:** [count]
**Remaining:** [count]

### Execution Order

1. **[Plan 1 Name]** (Wave 1)
   Path: `.planning/phases/[dir]/[phase]-01-PLAN.md`
   
2. **[Plan 2 Name]** (Wave 1)  
   Path: `.planning/phases/[dir]/[phase]-02-PLAN.md`

3. **[Plan 3 Name]** (Wave 2, depends on 1 & 2)
   Path: `.planning/phases/[dir]/[phase]-03-PLAN.md`

---

## ▶ Start Execution

Execute the first incomplete plan:

`/gsd-execute-plan`

Then reference: `[first incomplete plan path]`

**After each plan:**
1. Run `/clear` to reset context
2. Run `/gsd-execute-plan` for next plan
3. Repeat until all complete

---
```

### Step 4: After All Plans Complete

When all plans have SUMMARY.md files:

**A. Verify Phase Goal**

Use @gsd-verifier methodology to check:
- All success criteria from ROADMAP.md met
- Required artifacts exist and are substantive
- Key integrations wired correctly

**B. Create VERIFICATION.md**

```markdown
---
phase: [N]-[name]
verified: [timestamp]
status: [passed | gaps_found | human_needed]
score: [N]/[M] must-haves verified
---

# Phase [N] Verification Report

**Phase Goal:** [from ROADMAP.md]
**Status:** [status]

## Goal Achievement

[Verification details using @gsd-verifier patterns]

## Gaps (if any)

[List what's missing]

## Human Verification Needed (if any)

[Things that need manual testing]
```

**C. Update ROADMAP.md Progress**

```markdown
Phase 1  ██████████  100%  ✓
Phase 2  ░░░░░░░░░░  0%
```

**D. Update REQUIREMENTS.md**

Mark phase requirements as Complete in traceability table.

**E. Commit Phase Completion**

```bash
git add .planning/ROADMAP.md .planning/STATE.md .planning/REQUIREMENTS.md
git add "$PHASE_DIR"/*-VERIFICATION.md
git commit -m "docs(phase-$PHASE_NUM): complete [phase-name] phase

Plans executed: [count]
Requirements completed: [REQ-IDs]"
```

## Completion

### If Verification Passed

```
## ✓ Phase [N]: [Name] Complete

All [count] plans finished. Phase goal verified.

---

## ▶ Next Up

**Phase [N+1]: [Name]** — [Goal]

`/gsd-plan-phase [N+1]`

<sub>`/clear` first → fresh context window</sub>

---

**Also available:**
- `/gsd-verify-work [N]` — Manual acceptance testing
- `/gsd-progress` — Check overall status

---
```

### If Gaps Found

```
## ⚠ Phase [N]: [Name] — Gaps Found

**Score:** [N]/[M] must-haves verified

### What's Missing

[Gap descriptions]

---

## ▶ Next Up

**Plan gap closure**

`/gsd-plan-phase [N]`

Then execute the new plans to close gaps.

---
```

### If Last Phase

```
🎉 ALL PHASES COMPLETE!

---

## ▶ Next Up

**Audit milestone** — Verify requirements, cross-phase integration

`/gsd-audit-milestone`

---
```

## Success Criteria

- [ ] All incomplete plans identified
- [ ] Plans executed in wave order
- [ ] Each plan has SUMMARY.md
- [ ] Phase goal verified
- [ ] VERIFICATION.md created
- [ ] STATE.md and ROADMAP.md updated
- [ ] Requirements marked complete
- [ ] Phase completion committed
