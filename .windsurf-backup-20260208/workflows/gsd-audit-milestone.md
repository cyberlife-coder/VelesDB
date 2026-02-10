---
description: Verify milestone completion - requirements coverage, cross-phase integration, E2E flows
---

# /gsd-audit-milestone

Comprehensive audit before completing a milestone. Verifies all requirements met, cross-phase integration works, and E2E flows complete.

## Process

### Step 1: Requirements Audit

```bash
# Check all requirements
cat .planning/REQUIREMENTS.md
```

For each v1 requirement:
- Is it marked complete in traceability?
- Which phase addressed it?
- Was it verified?

Present:
```markdown
## Requirements Coverage

| ID | Requirement | Phase | Status |
|----|-------------|-------|--------|
| REQ-001 | [desc] | 1 | ✓ Complete |
| REQ-002 | [desc] | 2 | ✓ Complete |
| REQ-003 | [desc] | 3 | ⚠ Incomplete |
```

### Step 2: Phase Verification Audit

```bash
find .planning/phases -name "*-VERIFICATION.md" -exec cat {} \;
```

Check each phase verification status:
- All passed?
- Any gaps remaining?
- Any human verification pending?

### Step 3: Cross-Phase Integration Check

Identify integration points between phases:
- Does Phase 2 use what Phase 1 built?
- Are the connections working?

```markdown
## Integration Points

| From | To | Integration | Status |
|------|----|-------------|--------|
| Phase 1 | Phase 2 | Auth → Protected routes | ✓ |
| Phase 2 | Phase 3 | API → Frontend | ✓ |
```

### Step 4: E2E Flow Verification

Identify key user flows and verify they work end-to-end:

```markdown
## E2E Flows

### Flow 1: [Name]

**Steps:**
1. [Action] → [Expected]
2. [Action] → [Expected]
3. [Action] → [Expected]

**Status:** [Verified | Needs Testing]

### Flow 2: [Name]

[Same structure]
```

### Step 5: Audit Report

Create `.planning/AUDIT.md`:

```markdown
# Milestone Audit

**Milestone:** [name]
**Audited:** [date]
**Status:** [Ready | Gaps Found]

## Requirements

- Total: [count]
- Complete: [count]
- Incomplete: [count]

## Phase Verifications

- Total: [count]
- Passed: [count]
- Gaps: [count]

## Integration Points

- Total: [count]
- Working: [count]
- Broken: [count]

## E2E Flows

- Total: [count]
- Verified: [count]
- Pending: [count]

## Issues Found

[List any issues]

## Recommendation

[Ready to complete | Address issues first]
```

### Step 6: Route Based on Results

**If ready:**
```
## ✓ Milestone Audit Passed

All requirements met, integrations working, E2E flows verified.

---

## ▶ Complete Milestone

`/gsd-complete-milestone [version]`

---
```

**If gaps found:**
```
## ⚠ Audit Found Issues

[List issues]

---

## ▶ Address Issues

[Specific recommendations]

---
```
