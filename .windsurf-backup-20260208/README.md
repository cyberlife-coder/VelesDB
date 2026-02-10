# GSD (Claude) → Windsurf Translation Guide

This repo is a Windsurf-friendly adaptation of the original **GSD (Get Shit Done)** workflow.

In “original GSD”, a lot of the structure was driven by a chat/agent that could spawn parallel subagents and keep long-lived context. In **Windsurf**, the workflow is implemented as **slash commands** and a **file-based planning system** under `.planning/`.

## What changed vs “original GSD”

### 1) “Agent memory” → Project files in `.planning/`

Instead of relying on a single long chat thread, the source of truth becomes:

- `.planning/PROJECT.md` — vision, user, success criteria, constraints
- `.planning/REQUIREMENTS.md` — scoped requirements (v1/v2/out-of-scope) + traceability
- `.planning/ROADMAP.md` — phases, goals, success criteria
- `.planning/STATE.md` — where you are right now (phase/plan/progress)
- `.planning/phases/.../*-PLAN.md` — actionable plan(s) per phase
- `.planning/phases/.../*-SUMMARY.md` — execution summary per plan

This makes the workflow resilient to context resets and easy to resume.

### 2) “Parallel subagents” → Sequential plan execution

Windsurf doesn’t spawn parallel subagents (the workflow docs explicitly assume sequential execution).

So you typically:

- plan a phase
- execute one plan
- create a summary
- optionally run `/clear` to reset context
- repeat for the next plan

### 3) “Big-bang conversation” → Repeatable slash commands

Instead of manually remembering prompts, you use:

- `/gsd-...` commands (defined in `.windsurf/workflows/`)
- skills (methodologies) referenced by those workflows (in `.windsurf/skills/`)

## How to use (recommended)

### Option A: Fast setup (new project)

Use this when you want to initialize vision + requirements + roadmap in one go.

- Run: `/gsd-quick-start [project-name]`
- Next: `/gsd-plan-phase 1`
- Then: `/gsd-execute-plan`

### Option B: Thorough setup (new project)

- Run: `/gsd-new-project`
- Then: `/gsd-define-requirements`
- Then: `/gsd-create-roadmap`
- Then: `/gsd-plan-phase 1`
- Then: `/gsd-execute-plan`

### Brownfield (existing codebase)

If you already have code in the repo:

- Run: `/gsd-map-codebase`
- Then proceed with:
  - `/gsd-define-requirements`
  - `/gsd-create-roadmap`
  - `/gsd-plan-phase N`

## Core slash commands you can run

You can always run:

- `/gsd-help`

### Initialization

- `/gsd-new-project` — create `.planning/PROJECT.md` and `.planning/config.json`
- `/gsd-quick-start [project-name]` — create PROJECT/REQUIREMENTS/ROADMAP/STATE in one session
- `/gsd-define-requirements` — scope v1/v2/out-of-scope
- `/gsd-create-roadmap` — create phased roadmap mapped to requirements
- `/gsd-map-codebase` — map an existing codebase into `.planning/codebase/`

### Research (optional)

- `/gsd-research-project` — research the domain ecosystem before roadmap
- `/gsd-research-phase N` — research for a specific phase

### Planning & execution

- `/gsd-discuss-phase N` — clarify constraints and success criteria before planning
- `/gsd-plan-phase N` — generate detailed plan(s) for phase N
- `/gsd-execute-plan` — execute a single plan and produce summary/checkpoints
- `/gsd-execute-phase N` — execute all plans in a phase sequentially
- `/gsd-verify-work N` — verify phase goal achievement (manual acceptance)

### Progress & session management

- `/gsd-progress` — status + recommended next action
- `/gsd-pause-work` — create handoff info when stopping mid-phase
- `/gsd-resume-work` — restore context from `.planning/STATE.md` and `.planning/.continue-here`

### Roadmap changes

- `/gsd-add-phase` — append a new phase
- `/gsd-insert-phase N` — insert a phase between existing ones
- `/gsd-remove-phase N` — remove a future phase

### Milestones

- `/gsd-discuss-milestone` — plan the next milestone
- `/gsd-new-milestone` — create new milestone structure
- `/gsd-audit-milestone` — verify milestone completion
- `/gsd-complete-milestone` — archive/close milestone

### Utilities

- `/gsd-add-todo` — capture an idea/task
- `/gsd-check-todos` — review and pick a todo to work on
- `/gsd-debug` — systematic debugging flow

## Windsurf-specific tips

### Context resets are normal

The workflows recommend using `/clear` between major steps/plans when the context window gets full.

Typical pattern:

- `/gsd-plan-phase 1`
- `/clear`
- `/gsd-execute-plan` (for the next plan file)
- `/clear`
- `/gsd-progress`

### “Source of truth” rule

If there’s ever disagreement between chat and files:

- treat `.planning/*` as authoritative
- update those docs as part of work (plans, summaries, state)

## Where the commands live

All slash command definitions are in:

- `.windsurf/workflows/*.md`

Skills/methodologies referenced by workflows are in:

- `.windsurf/skills/`
