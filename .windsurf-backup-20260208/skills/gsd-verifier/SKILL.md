---
name: gsd-verifier
description: Goal-backward verification methodology for checking phase completion. Verifies codebase delivers what phase promised, not just that tasks completed.
---

# GSD Verifier

Verify that a phase achieved its GOAL, not just completed its TASKS.

## Core Principle

**Task completion ≠ Goal achievement**

A task "create chat component" can be marked complete when the component is a placeholder. You verify what ACTUALLY exists in the code, not what SUMMARY.md claims.

## Goal-Backward Verification

Start from the outcome and work backwards:

1. **What must be TRUE** for the goal to be achieved?
2. **What must EXIST** for those truths to hold?
3. **What must be WIRED** for those artifacts to function?

## Verification Process

### Step 1: Establish Must-Haves

From the phase goal in ROADMAP.md, derive:

**Observable Truths** (3-7 items)
- What behaviors should a user observe?
- Each truth should be testable

**Required Artifacts**
- What files must exist?
- Be specific: `src/components/Chat.tsx`, not "chat component"

**Key Links**
- What connections must exist?
- Component → API, API → Database, Form → Handler

### Step 2: Verify Artifacts (Three Levels)

#### Level 1: Existence
```bash
[ -f "src/path/file.ts" ] && echo "EXISTS" || echo "MISSING"
```

#### Level 2: Substantive
```bash
# Check line count (min thresholds)
wc -l < "src/path/file.ts"
# Component: 15+ lines, API route: 10+, Schema: 5+

# Check for stub patterns
grep -E "TODO|FIXME|placeholder|not implemented" "src/path/file.ts"

# Check for empty returns
grep -E "return null|return \{\}|return \[\]" "src/path/file.ts"
```

#### Level 3: Wired
```bash
# Is it imported?
grep -r "import.*ComponentName" src/ --include="*.ts" --include="*.tsx"

# Is it used?
grep -r "ComponentName" src/ --include="*.ts" --include="*.tsx" | grep -v "import"
```

### Step 3: Verify Key Links

#### Component → API
```bash
# Check for fetch/axios call
grep -E "fetch\(['\"].*api/endpoint|axios\.(get|post)" "src/component.tsx"

# Check response is used
grep -A 5 "fetch\|axios" "src/component.tsx" | grep -E "await|\.then|setState"
```

#### API → Database
```bash
# Check for DB query
grep -E "prisma\.|db\." "src/api/route.ts"

# Check result is returned
grep -E "return.*json" "src/api/route.ts"
```

#### Form → Handler
```bash
# Check for real handler (not just preventDefault)
grep -A 10 "onSubmit" "src/component.tsx" | grep -E "fetch|axios|mutate"
```

## Stub Detection Patterns

**Red flags in React components:**
```javascript
return <div>Component</div>
return <div>Placeholder</div>
return null
onClick={() => {}}
onSubmit={(e) => e.preventDefault()}  // Only prevents default
```

**Red flags in API routes:**
```typescript
return Response.json({ message: "Not implemented" })
return Response.json([])  // Empty with no DB query
console.log(data)  // Only logs, no action
```

**Red flags in wiring:**
```typescript
fetch('/api/endpoint')  // No await, no assignment
await prisma.find()
return { ok: true }  // Returns static, not query result
```

## Determine Status

| Status | Condition |
|--------|-----------|
| `passed` | All truths verified, all artifacts pass 3 levels, all links wired |
| `gaps_found` | One or more truths failed, artifacts missing/stub, links broken |
| `human_needed` | Automated checks pass but needs manual verification (visual, UX) |

## Output: VERIFICATION.md

```markdown
---
phase: XX-name
verified: [timestamp]
status: [passed | gaps_found | human_needed]
score: N/M must-haves verified
---

# Phase [X] Verification Report

**Phase Goal:** [from ROADMAP.md]
**Status:** [status]

## Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | [truth] | ✓ VERIFIED | [evidence] |
| 2 | [truth] | ✗ FAILED | [what's wrong] |

## Required Artifacts

| Artifact | Exists | Substantive | Wired | Status |
|----------|--------|-------------|-------|--------|
| `path` | ✓ | ✓ | ✓ | VERIFIED |
| `path` | ✓ | ✗ | - | STUB |

## Key Links

| From | To | Status | Details |
|------|----|--------|---------|
| Component | API | WIRED | fetch call + response handling |
| API | DB | NOT_WIRED | query exists, result not returned |

## Gaps (if any)

[Structured description of what's missing]

## Human Verification Needed (if any)

[Things that need manual testing - visual, UX, real-time behavior]
```

## Quality Checklist

- [ ] All truths checked with evidence
- [ ] All artifacts verified at 3 levels
- [ ] All key links verified
- [ ] Stub patterns scanned
- [ ] Status determined accurately
- [ ] Gaps documented if found
