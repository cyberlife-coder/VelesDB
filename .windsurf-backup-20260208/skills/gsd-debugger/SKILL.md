---
name: gsd-debugger
description: Systematic debugging methodology with hypothesis testing and persistent state. Invoke when debugging issues.
---

# GSD Debugger

Systematic debugging using scientific method: gather evidence → form hypothesis → test → iterate.

## Debugging Mindset

**NEVER guess.** Debugging is investigation, not trial-and-error.

- Gather evidence before forming hypotheses
- Test one thing at a time
- Document what you learn
- Follow the data, not assumptions

## Process

### Phase 1: Symptom Gathering

Ask adaptive questions based on issue type:

**For UI issues:**
- What's the expected vs actual behavior?
- When did it start happening?
- Does it happen consistently?
- Any console errors?

**For API issues:**
- What endpoint? What method?
- What's the request/response?
- Any error messages?
- Works in other environments?

**For Build issues:**
- What command fails?
- What's the error message?
- When did it start?
- Any recent changes?

### Phase 2: Evidence Collection

Gather concrete data:

```bash
# Check logs
cat logs/error.log | tail -50

# Check recent changes
git log --oneline -10
git diff HEAD~5

# Check process status
ps aux | grep [process]

# Check file state
ls -la [path]
cat [file]
```

### Phase 3: Hypothesis Formation

Based on evidence, form testable hypotheses:

**Format:**
```
Hypothesis: [specific claim]
Evidence supporting: [what points to this]
Evidence against: [what contradicts]
Test: [how to verify/falsify]
```

**Rules:**
- One hypothesis at a time
- Must be falsifiable
- Test should be quick

### Phase 4: Hypothesis Testing

**Before testing:**
- Document current state
- Identify what will change
- Know how to revert

**Testing:**
- Make ONE change
- Observe result
- Record outcome

**After testing:**
- Did it work? → Document and verify fix
- Didn't work? → Revert, update hypothesis, iterate

### Phase 5: Root Cause Analysis

When fix found, identify root cause:

- Why did this happen?
- How can we prevent recurrence?
- Are there similar issues elsewhere?

## Investigation Techniques

### Binary Search
When: Large space of possibilities
How: Eliminate half the possibilities each step

```bash
# Find which commit introduced bug
git bisect start
git bisect bad HEAD
git bisect good [known-good-commit]
# Test at each step
```

### Isolation
When: Complex interactions suspected
How: Remove components until problem disappears

### Tracing
When: Need to understand flow
How: Add logging at key points

```typescript
console.log('[DEBUG] Function called with:', args);
console.log('[DEBUG] State after operation:', state);
```

### Comparison
When: Works somewhere, not elsewhere
How: Identify differences

```bash
# Compare configs
diff production.env development.env

# Compare versions
npm ls [package]
```

## Debug State File

For persistent debugging across sessions, create `.planning/debug/[slug].md`:

```markdown
# Debug: [Issue Description]

**Started:** [date]
**Status:** [investigating | testing | resolved]

## Symptoms

[What's happening]

## Evidence Collected

- [Finding 1]
- [Finding 2]

## Hypotheses

### Hypothesis 1: [Description]
**Status:** [testing | rejected | confirmed]
**Evidence:** [what supports/contradicts]
**Test:** [how to verify]
**Result:** [outcome]

## Timeline

| Time | Action | Result |
|------|--------|--------|
| [time] | [what] | [outcome] |

## Resolution

**Root cause:** [if found]
**Fix:** [what was done]
**Prevention:** [how to prevent recurrence]
```

## Common Patterns

### The Recent Change
Most bugs come from recent changes.
```bash
git log --oneline -20
git diff HEAD~5
```

### The Environment Difference
Works locally, fails elsewhere.
- Check env vars
- Check versions
- Check configs

### The Race Condition
Happens intermittently.
- Add logging with timestamps
- Check async handling
- Look for shared state

### The Null Reference
Crashes on property access.
- Trace data flow backwards
- Check all sources of the value
- Add null checks

## Anti-Patterns

**Don't:**
- Make multiple changes at once
- Assume without testing
- Ignore error messages
- Skip documentation
- Give up on root cause

**Do:**
- Change one thing at a time
- Test hypotheses
- Read errors carefully
- Document findings
- Find and fix root cause
