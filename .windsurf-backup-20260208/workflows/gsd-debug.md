---
description: Systematic debugging with persistent state across context resets
---

# /gsd-debug [issue description]

Systematic debugging with persistent state. Survives `/clear` — run `/gsd-debug` with no args to resume.

**Usage:** 
- `/gsd-debug "login button doesn't work"` — Start new session
- `/gsd-debug` — Resume active session

## Check for Active Session

```bash
ls .planning/debug/*.md 2>/dev/null | grep -v resolved | head -1
```

If active session exists and no new description provided → resume that session.

## Start New Session

### Step 1: Create Debug File

```bash
mkdir -p .planning/debug
SLUG=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-' | head -c 50)
DEBUG_FILE=".planning/debug/${SLUG}.md"
```

Create initial file:

```markdown
# Debug: [Issue Description]

**Started:** [timestamp]
**Status:** investigating

## Symptoms

[To be gathered]

## Evidence Collected

[To be gathered]

## Hypotheses

[To be formed]

## Timeline

| Time | Action | Result |
|------|--------|--------|

## Resolution

**Root cause:** TBD
**Fix:** TBD
```

### Step 2: Gather Symptoms

Use @gsd-debugger methodology. Ask adaptive questions:

**For UI issues:**
- What's expected vs actual?
- Consistent or intermittent?
- Any console errors?

**For API issues:**
- What endpoint/method?
- Request/response details?
- Error messages?

**For Build issues:**
- What command fails?
- Error message?
- Recent changes?

Update debug file with symptoms.

### Step 3: Collect Evidence

Gather concrete data:

```bash
# Recent changes
git log --oneline -10

# Error logs
cat [log file] | tail -50

# File state
cat [relevant file]
```

Update debug file with evidence.

### Step 4: Form Hypotheses

Based on evidence, form testable hypotheses:

```markdown
### Hypothesis 1: [Specific claim]

**Evidence supporting:** [what points to this]
**Evidence against:** [what contradicts]
**Test:** [how to verify/falsify]
**Status:** testing
```

### Step 5: Test Hypotheses

For each hypothesis:
1. Make ONE change
2. Observe result
3. Record outcome
4. Update hypothesis status

```markdown
**Result:** [confirmed | rejected]
**What happened:** [details]
```

### Step 6: Resolution

When fix found:

```markdown
## Resolution

**Root cause:** [what caused it]
**Fix:** [what was done]
**Commits:** [hashes]
**Prevention:** [how to prevent recurrence]
```

### Step 7: Archive

```bash
mkdir -p .planning/debug/resolved
mv ".planning/debug/${SLUG}.md" ".planning/debug/resolved/${SLUG}.md"
git add ".planning/debug/resolved/${SLUG}.md"
git commit -m "docs: resolve debug session - [slug]"
```

## Resume Session

When resuming (no description provided):

1. Read the active debug file
2. Show current status:

```
## Resuming Debug Session

**Issue:** [description]
**Status:** [status]
**Last action:** [from timeline]

### Current Hypothesis

[Active hypothesis details]

### Next Step

[What to test next]
```

3. Continue from where left off

## Context Reset Handling

The debug file is the persistent state. After `/clear`:

```
/gsd-debug
```

This will:
- Find the active debug file
- Load all context
- Show summary
- Continue investigation

## Success Criteria

- [ ] Symptoms documented
- [ ] Evidence collected
- [ ] Hypotheses formed and tested
- [ ] Root cause identified
- [ ] Fix verified
- [ ] Session archived
