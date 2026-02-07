---
name: ralph-loop
description: Emulates the Ralph autonomous loop (PRD → JSON → iterative implementation with progress tracking and AGENTS.md updates)
---

> Use this skill when you need Cascade to behave like Ralph/Claude-Ralph: break work into tiny user stories, run isolated iterations, and keep state in files instead of long-lived context.

## Purpose
Ralph is an autonomous loop that repeatedly spawns a fresh coding session (Amp, Claude Code, Cascade) to implement **one** story at a time until every PRD item passes. This skill captures the guardrails, required artifacts, and iteration rituals so Cascade can reproduce the same behavior inside Windsurf.

## Mandatory Artifacts
| File | Role | Notes |
| ---- | ---- | ----- |
| `tasks/prd-<feature>.md` | Narrative PRD captured via the PRD skill | Answer every clarifying question before saving |
| `prd.json` | Machine-readable task list in Ralph schema | Convert with the Ralph converter skill or the template below |
| `progress.txt` | Append-only learnings between iterations | Each iteration adds context for future loops |
| `AGENTS.md` (per scope) | Codifies discoveries & gotchas | Update after every iteration |
| Branch `ralph/<feature>` | Isolated working branch | Created automatically by `ralph.sh` or manually if running steps by hand |

See [`prd-json-template.md`](prd-json-template.md) for the schema and acceptance criteria guidance.

## High-Level Workflow
1. **Create PRD** – invoke the PRD skill: `Load the prd skill and create a PRD for <feature>`.
2. **Convert PRD** – `Load the ralph skill and convert tasks/prd-<feature>.md to prd.json`.
3. **Run loop** – execute `./scripts/ralph/ralph.sh [--tool amp|claude] [max_iterations]` or follow the manual loop in [`iteration-checklist.md`](iteration-checklist.md).
4. **Stop condition** – halt when every `userStories[*].passes` is `true` and emit `<promise>COMPLETE</promise>`.

### Working directly from an EPIC folder
If your project already stores detailed user stories inside `.epics/EPIC-XYZ/`, you can skip the classic *PRD → Markdown* step:

1. Point Cascade at the EPIC path when invoking the skill, e.g.
   ```text
   @ralph-loop develop feature from @[.epics/EPIC-045-match-graph-execution]
   ```
2. The skill reads all `US-*.md` files inside the EPIC, converts them to `prd.json` (keeping `id`, `title`, `acceptanceCriteria`, `priority`).
3. It sets `branchName` to `ralph/<epic-folder-name>` and starts the normal iteration loop.

This way the **single source of truth remains in `.epics`** while Ralph/Cascade still benefit from `prd.json` and `progress.txt` for autonomous iterations.


## Iteration Protocol (One Story Per Loop)
Follow the detailed checklist in `iteration-checklist.md`. Summary:
1. Parse `prd.json` and pick the highest-priority story with `passes: false`.
2. Create/checkout `ralph/<feature>` branch and ensure `git status` is clean.
3. Plan & implement **only that story**.
4. Run tests/typecheck/linters relevant to the change.
5. If green, commit with message `feat: <story title> [US-XXX]` and set `passes: true` for that story.
6. Append insights to `progress.txt` (what worked, blockers, TODOs).
7. Update relevant `AGENTS.md` sections with:
   - Patterns ("This repo prefers X over Y")
   - Traps ("Changing Z requires updating W")
   - UI locations ("Settings panel lives in component Foo")
8. Rerun the loop until all work is complete or iteration budget exhausted.

## Story Sizing Rules
- **One iteration ≈ one LLM context window.** If a story cannot be completed in a single context, split it before running.
- Prefer CRUD-scale tasks (add column, tweak UI component, adjust service function) over broad initiatives ("Build dashboard", "Add auth").
- Use the heuristics and examples in [`story-sizing-guide.md`](story-sizing-guide.md) to validate scope.

## Acceptance Criteria Guidance
Every story must include verifiable acceptance criteria:
1. Functional expectations (inputs/outputs, UI behavior).
2. Quality gates ("Typecheck passes", "All existing tests stay green").
3. UI stories add: "Verify in browser using dev-browser skill".

See `prd-json-template.md` for canonical criteria samples and red flags.

## Feedback Loops & Quality Gates
- Treat failing tests or lint as **hard stops**—fix before continuing.
- Never skip `cargo test`, `cargo clippy`, or project-specific scripts noted in `progress.txt`.
- Browser verification is mandatory for UI-facing stories via `dev-browser` skill.

## Archiving & Resetting
- When the loop completes, capture a summary in `progress.txt` and optionally archive artifacts under `archive/<date>-<feature>/`.
- To restart, duplicate the PRD, reset `passes` flags, and prune the branch.

## References
- Original Ralph repo: https://github.com/snarktank/ralph
- Claude Code variant: https://github.com/frankbria/ralph-claude-code
- Flowchart (interactive): https://snarktank.github.io/ralph/
