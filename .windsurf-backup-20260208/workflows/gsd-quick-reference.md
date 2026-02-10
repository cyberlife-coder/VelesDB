---
description: Quick reference guide for building with GSD - use this as your go-to checklist
---

# GSD Quick Reference

Your step-by-step guide for building projects with GSD.

---

## ⚡ Lean Mode (Credit-Efficient)

**Best for:** Small-medium projects, familiar domains, moving fast.

```
1. /gsd-quick-start       ← Vision + requirements + roadmap (one session)
2. /gsd-plan-phase 1      ← Plan first phase
3. /gsd-execute-phase 1   ← Execute + verify (built-in)
4. Repeat 2-3 for each phase
```

**When to `/clear`:** After each phase execution, before planning next phase.

---

## 🔧 Standard Mode (More Control)

**Best for:** Complex projects, unfamiliar domains, large teams.

### Setup (one-time)
```
/gsd-new-project          ← Define vision
/gsd-define-requirements  ← Scope features
/gsd-research-project     ← (if unfamiliar domain)
/gsd-create-roadmap       ← Break into phases
```

### Per-Phase Loop
```
/gsd-discuss-phase [N]    ← (optional) Capture vision
/gsd-plan-phase [N]       ← Create task plans
/gsd-execute-phase [N]    ← Execute + verify
```

**When to `/clear`:** Between each step above for fresh context.

---

## 🧠 Context Management

GSD uses `/clear` + STATE.md to prevent context rot:

| Situation | Action |
|-----------|--------|
| Planning complete, starting execution | `/clear` first |
| Phase complete, planning next | `/clear` first |
| Session > 30 messages | Consider `/clear` |
| Cascade seems confused | `/clear` + `/gsd-resume-work` |

**Why it works:** STATE.md and PLAN.md files contain all context needed. Fresh session reads these files = fresh start with full context.

---

## 📍 During Development

### Check Progress
```
/gsd-progress
```
Shows current status and recommends next action.

### Pause Work (stopping mid-session)
```
/gsd-pause-work
```
Creates handoff file for later resumption.

### Resume Work (new session)
```
/gsd-resume-work
```
Restores context and shows where you left off.

### Capture Ideas
```
/gsd-add-todo [description]
```
Saves ideas without derailing current work.

### Debug Issues
```
/gsd-debug
```
Systematic debugging with hypothesis testing.

---

## 🔄 The Core Loop

```
PLAN → EXECUTE (includes verify) → NEXT PHASE
```

`/gsd-execute-phase` automatically verifies at the end — no separate verify step needed.

---

## 📋 Existing Codebase Flow

```
1. /gsd-map-codebase    ← Understand what exists
2. /gsd-quick-start     ← Define what you're adding (or use standard mode)
3. /gsd-plan-phase 1    ← Plan first phase
4. /gsd-execute-phase 1 ← Execute + verify
```

---

## 🎯 Key Principles

- **One thing at a time** — Focus on current task
- **Atomic commits** — Commit after each task
- **Verify goals, not tasks** — Did the feature actually work?
- **Fresh context** — `/clear` between phases prevents context rot
- **STATE.md is your memory** — It survives `/clear` and restores context

---

## 📁 File Structure Reference

```
.planning/
├── PROJECT.md          ← Vision and goals
├── REQUIREMENTS.md     ← What to build
├── ROADMAP.md          ← Phase breakdown
├── STATE.md            ← Current progress
├── phases/
│   ├── 01-setup/
│   │   ├── 01-PLAN.md
│   │   ├── 01-SUMMARY.md
│   │   └── VERIFICATION.md
│   └── 02-feature/
│       └── ...
└── todos/
    └── pending/
```

---

## ❓ Need Help?

```
/gsd-help
```
Shows all available commands with descriptions.
