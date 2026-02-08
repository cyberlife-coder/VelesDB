---
description: Show all available GSD commands and usage guide for Windsurf
---

# GSD Command Reference (Windsurf Edition)

**GSD** (Get Shit Done) creates hierarchical project plans optimized for solo agentic development.

## Quick Start

1. `/gsd-new-project` - Initialize project with deep context gathering
2. `/gsd-define-requirements` - Scope v1/v2/out-of-scope requirements
3. `/gsd-create-roadmap` - Create roadmap with phases
4. `/gsd-plan-phase 1` - Create detailed plan for phase 1
5. `/gsd-execute-plan` - Execute the plan

## Core Workflow

```
Initialization → Planning → Execution → Milestone Completion
```

### Project Initialization

| Command | Description |
|---------|-------------|
| `/gsd-new-project` | Initialize project with brief, creates PROJECT.md |
| `/gsd-define-requirements` | Scope requirements into v1/v2/out-of-scope |
| `/gsd-create-roadmap` | Create roadmap with phases mapped to requirements |
| `/gsd-map-codebase` | Map existing codebase (brownfield projects) |

### Research (Optional but Recommended)

| Command | Description |
|---------|-------------|
| `/gsd-research-project` | Research domain ecosystem before roadmap |
| `/gsd-research-phase N` | Deep research for specific phase |

### Phase Planning & Execution

| Command | Description |
|---------|-------------|
| `/gsd-discuss-phase N` | Gather context before planning |
| `/gsd-plan-phase N` | Create detailed task plans for phase |
| `/gsd-execute-plan` | Execute single plan with checkpoints |
| `/gsd-execute-phase N` | Execute all plans in phase sequentially |
| `/gsd-verify-work N` | Verify phase goal achievement |

### Progress & Session

| Command | Description |
|---------|-------------|
| `/gsd-progress` | Check status and get next action |
| `/gsd-pause-work` | Create handoff when stopping mid-phase |
| `/gsd-resume-work` | Restore from last session |

### Roadmap Management

| Command | Description |
|---------|-------------|
| `/gsd-add-phase` | Append phase to roadmap |
| `/gsd-insert-phase N` | Insert urgent work between phases |
| `/gsd-remove-phase N` | Remove future phase |

### Milestones

| Command | Description |
|---------|-------------|
| `/gsd-discuss-milestone` | Plan next milestone |
| `/gsd-new-milestone` | Create new milestone |
| `/gsd-audit-milestone` | Verify milestone completion |
| `/gsd-complete-milestone` | Archive and tag milestone |

### Utilities

| Command | Description |
|---------|-------------|
| `/gsd-add-todo` | Capture idea or task |
| `/gsd-check-todos` | List and work on todos |
| `/gsd-debug` | Systematic debugging with state |
| `/gsd-help` | Show this reference |

## Files & Structure

```
.planning/
├── PROJECT.md            # Project vision
├── REQUIREMENTS.md       # Scoped requirements
├── ROADMAP.md            # Phase breakdown
├── STATE.md              # Project memory
├── config.json           # Workflow settings
├── todos/                # Captured tasks
├── debug/                # Debug sessions
├── codebase/             # Codebase map (brownfield)
└── phases/
    ├── 01-foundation/
    │   ├── 01-01-PLAN.md
    │   └── 01-01-SUMMARY.md
    └── 02-core-features/
        └── ...
```

## Skills Available

Use `@skill-name` to invoke specialized capabilities:

- `@gsd-executor` - Plan execution methodology
- `@gsd-researcher` - Research methodology
- `@gsd-verifier` - Verification patterns
- `@gsd-debugger` - Debugging methodology
- `@gsd-codebase-mapper` - Codebase analysis
- `@gsd-templates` - Output templates

## Context Management

**Important:** Windsurf doesn't spawn subagents like Claude Code. Use `/clear` between major phases to reset context when it fills up.

Recommended workflow:
```
/gsd-plan-phase 1
/clear
/gsd-execute-plan (plan 1)
/clear
/gsd-execute-plan (plan 2)
/clear
/gsd-progress
```

## Getting Help

- Read `.planning/PROJECT.md` for project vision
- Read `.planning/STATE.md` for current context
- Check `.planning/ROADMAP.md` for phase status
- Run `/gsd-progress` to check where you're at
