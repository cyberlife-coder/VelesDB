---
description: User acceptance test of phase or plan work
---

# /gsd-verify-work [N]

Manual acceptance testing of completed phase or plan. Uses @gsd-verifier methodology.

**Usage:** `/gsd-verify-work 2` (verify phase 2)

## Process

### Step 1: Load Phase Context

```bash
PHASE_NUM=$1
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }
```

Read:
- Phase goal from ROADMAP.md
- Success criteria from ROADMAP.md
- SUMMARY.md files from phase

### Step 2: Generate Test Checklist

Using @gsd-verifier, derive testable items:

```markdown
# Phase [N] Verification

**Goal:** [from ROADMAP.md]

## Automated Checks

Running verification commands from plans...

[Results of automated verification]

## Manual Testing Checklist

### Core Functionality

- [ ] **[Test 1]**
  - Steps: [what to do]
  - Expected: [what should happen]

- [ ] **[Test 2]**
  - Steps: [what to do]
  - Expected: [what should happen]

### Edge Cases

- [ ] **[Edge case 1]**
  - Steps: [what to do]
  - Expected: [what should happen]

### User Experience

- [ ] **[UX check 1]**
  - What to verify: [description]

---

**Mark items as you test. Report any failures.**

---
```

### Step 3: Collect Results

After user tests, ask:
> Did all tests pass? Any issues found?

### Step 4: Handle Results

**If all pass:**
- Update VERIFICATION.md with results
- Mark phase verified in STATE.md

**If issues found:**
- Document issues
- Create todos or debug sessions
- Recommend next steps

### Step 5: Completion

```
Verification complete:

**Status:** [Passed | Issues Found]
**Tested:** [count] items

[If issues:]
**Issues to address:**
- [issue 1]
- [issue 2]

---

## ▶ Next Steps

[Based on results]

---
```
