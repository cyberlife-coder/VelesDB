---
description: Initialize a new GSD project with deep context gathering and PROJECT.md creation
---

# /gsd-new-project

Initialize a new project through comprehensive context gathering. This is the most leveraged moment in any project — deep questioning here means better plans, better execution, better outcomes.

## Pre-flight Checks

Before starting, verify:

1. **Check if project already exists:**
   ```bash
   [ -f .planning/PROJECT.md ] && echo "ERROR: Project already initialized. Use /gsd-progress" && exit 1
   ```

2. **Initialize git if needed:**
   ```bash
   if [ -d .git ] || [ -f .git ]; then
       echo "Git repo exists"
   else
       git init
       echo "Initialized new git repo"
   fi
   ```

3. **Detect existing code (brownfield):**
   ```bash
   find . -name "*.ts" -o -name "*.js" -o -name "*.py" -o -name "*.go" 2>/dev/null | grep -v node_modules | grep -v .git | head -20
   ```

## Brownfield Detection

If existing code detected and `.planning/codebase/` doesn't exist:

**Ask the user:**
> I detected existing code in this directory. Would you like to map the codebase first?
> - **Map codebase first** (Recommended) — Run `/gsd-map-codebase` to understand existing architecture
> - **Skip mapping** — Proceed with project initialization

If "Map codebase first" → Tell user to run `/gsd-map-codebase` first, then return.

## Context Gathering Process

### Step 1: Open Question

Ask:
> **What do you want to build?**

Wait for response. This gives context for intelligent follow-up questions.

### Step 2: Follow the Thread

Based on their response, ask follow-up questions that dig into what they said. Keep following threads until you understand:

**Core Understanding:**
- What problem does this solve?
- Who is this for?
- What does success look like?

**Technical Decisions:**
- What's the tech stack preference?
- Any existing constraints?
- Deployment target?

**Scope Clarity:**
- What's definitely in v1?
- What's explicitly out of scope?
- Any non-negotiable requirements?

**Questioning Techniques:**
- Challenge vague terms ("what do you mean by 'simple'?")
- Make abstract concrete ("can you give an example?")
- Surface assumptions ("are you assuming X?")
- Find edges ("what happens when Y?")

### Step 3: Decision Gate

When you could write a clear PROJECT.md, ask:

> I think I understand what you're building. Ready to create PROJECT.md?
> - **Create PROJECT.md** — Let's move forward
> - **Keep exploring** — I want to share more

Loop until they select "Create PROJECT.md".

## Create PROJECT.md

Create `.planning/PROJECT.md` with this structure:

```markdown
# [Project Name]

## Vision

[2-3 sentences capturing the core idea and why it matters]

## Problem Statement

[What problem this solves, for whom]

## Target User

[Who uses this, what they care about]

## Success Criteria

- [Observable outcome 1]
- [Observable outcome 2]
- [Observable outcome 3]

## Technical Approach

**Stack:** [Technologies chosen and why]

**Architecture:** [High-level approach]

**Constraints:** [Any limitations or requirements]

## Requirements

### Validated
(None yet — ship to validate)

### Active
- [ ] [Requirement 1]
- [ ] [Requirement 2]
- [ ] [Requirement 3]

### Out of Scope
- [Exclusion 1] — [why]
- [Exclusion 2] — [why]

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| [Choice] | [Why] | Pending |

---
*Last updated: [date] after initialization*
```

## Workflow Preferences

Ask the user:

**1. Mode:** How do you want to work?
- **YOLO** (Recommended) — Auto-approve, just execute
- **Interactive** — Confirm at each step

**2. Depth:** How thorough should planning be?
- **Quick** — Ship fast (3-5 phases, 1-3 plans each)
- **Standard** — Balanced (5-8 phases, 3-5 plans each)
- **Comprehensive** — Thorough (8-12 phases, 5-10 plans each)

## Create config.json

Create `.planning/config.json`:

```json
{
  "version": "1.0.0",
  "mode": "[yolo|interactive]",
  "depth": "[quick|standard|comprehensive]",
  "created": "[date]"
}
```

## Commit

```bash
mkdir -p .planning
git add .planning/PROJECT.md .planning/config.json
git commit -m "docs: initialize [project-name]

[One-liner from PROJECT.md vision]

Creates PROJECT.md with requirements and constraints."
```

## Completion

Present:

```
Project initialized:

- Project: .planning/PROJECT.md
- Config: .planning/config.json (mode: [chosen])

---

## ▶ Next Up

Choose your path:

**Option A: Research first** (recommended)
Research ecosystem → define requirements → create roadmap.

`/gsd-research-project`

**Option B: Define requirements directly** (familiar domains)
Skip research, define requirements from what you know.

`/gsd-define-requirements`

<sub>`/clear` first → fresh context window</sub>

---
```

## Success Criteria

- [ ] Deep questioning completed (threads followed, not rushed)
- [ ] PROJECT.md captures full context
- [ ] Requirements initialized as hypotheses
- [ ] config.json has mode and depth
- [ ] All committed to git
