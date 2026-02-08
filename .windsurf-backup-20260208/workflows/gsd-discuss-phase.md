---
description: Gather context before planning a phase - capture user's vision
---

# /gsd-discuss-phase [N]

Help articulate your vision for a phase before planning. Creates CONTEXT.md with your vision, essentials, and boundaries.

**Usage:** `/gsd-discuss-phase 2`

## Process

### Step 1: Load Phase Context

```bash
PHASE_NUM=$1
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }
```

Read phase goal from ROADMAP.md.

### Step 2: Open Discussion

Present phase goal, then ask:

> Looking at Phase [N]: [Name]
> 
> Goal: [from ROADMAP.md]
> 
> **How do you imagine this working?** Tell me about:
> - What the user experience should feel like
> - Any specific implementation ideas you have
> - Things that are absolutely essential
> - Things you definitely DON'T want

### Step 3: Follow the Thread

Based on response, dig deeper:
- "What do you mean by [vague term]?"
- "Can you give me an example of [concept]?"
- "What would make this feel 'right' to you?"
- "What would be a dealbreaker?"

### Step 4: Capture Decision Gate

When you have enough context:

> I think I understand your vision. Ready to capture this in CONTEXT.md?
> - **Create CONTEXT.md** — Save this vision
> - **Keep discussing** — There's more to explore

### Step 5: Create CONTEXT.md

Create `$PHASE_DIR/CONTEXT.md`:

```markdown
# Phase [N]: [Name] — Context

**Captured:** [date]

## Vision

[User's description of how they imagine this working]

## User Experience

[What the UX should feel like]

## Essentials

Things that MUST be true:
- [Essential 1]
- [Essential 2]

## Boundaries

Things to explicitly AVOID:
- [Anti-pattern 1]
- [Anti-pattern 2]

## Implementation Notes

Specific technical preferences mentioned:
- [Preference 1]
- [Preference 2]

## Open Questions

Things to decide during planning:
- [Question 1]

---
*This context informs planning. The planner will honor these preferences.*
```

### Step 6: Commit

```bash
git add "$PHASE_DIR/CONTEXT.md"
git commit -m "docs(phase-$PHASE_NUM): capture phase context"
```

### Step 7: Completion

```
Context captured:

- File: [path to CONTEXT.md]
- Phase: [N] - [Name]

---

## ▶ Next Up

**Plan the phase** — Using your captured vision

`/gsd-plan-phase [N]`

<sub>`/clear` first → fresh context window</sub>

---
```
