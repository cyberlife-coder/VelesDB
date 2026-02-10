---
description: Gather context for next milestone - figure out what to build next
---

# /gsd-discuss-milestone

Figure out what you want to build in the next milestone. Reviews previous work and helps identify what's next.

## Process

### Step 1: Review Previous Milestone

```bash
cat .planning/MILESTONES.md 2>/dev/null
cat .planning/REQUIREMENTS.md 2>/dev/null | grep -A 100 "## v2"
```

Present:
- What was shipped in previous milestone
- What's in the v2 backlog
- Any user feedback or learnings

### Step 2: Open Discussion

> Looking at what you've built, what would you like to focus on next?
> 
> **Current v2 backlog:**
> [List from REQUIREMENTS.md]
> 
> **Options:**
> - Build from v2 backlog
> - Add new features
> - Improve existing features
> - Technical improvements

### Step 3: Explore Ideas

For each direction:
- What problem does this solve?
- Who benefits?
- What's the scope?
- What's the priority?

### Step 4: Scope the Milestone

Help user identify:
- Core features for this milestone
- Nice-to-haves
- Out of scope

### Step 5: Ready Gate

> Ready to create the milestone?
> - **Create milestone** — Let's build it
> - **Keep exploring** — More to discuss

### Step 6: Route to Creation

```
Ready to create milestone:

**Focus:** [summary]
**Key features:** [list]

---

## ▶ Create Milestone

`/gsd-new-milestone "[name]"`

---
```
