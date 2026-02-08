# Ralph Iteration Checklist

Follow this script every time you run a Ralph-style loop. One pass == one story.

## 0. Preconditions
- [ ] `prd.json` exists and follows the Ralph schema
- [ ] Branch `ralph/<feature>` created (or will be created on first pass)
- [ ] `progress.txt` appended with context from previous runs
- [ ] Workspace clean (`git status` empty)

## 1. Select Next Story
- [ ] Parse `prd.json`
- [ ] Filter to `userStories` where `passes: false`
- [ ] Pick the lowest `priority` value (ties → lowest `id`)
- [ ] Confirm story scope fits in one context window (see story-sizing guide)

## 2. Prepare Workspace
- [ ] Checkout/create `ralph/<feature>` branch from `develop`
- [ ] Pull latest from remote base branch (rebase if required)
- [ ] Run fast tests (lint/format) to ensure baseline green

## 3. Plan Implementation
- [ ] Re-read story description + acceptance criteria
- [ ] Identify touched modules/files
- [ ] Note required scripts/commands (tests, build, browser runs)
- [ ] Document plan snippet in `progress.txt` ("Iteration N plan")

## 4. Implement Story
- [ ] Apply smallest possible change set to satisfy criteria
- [ ] Keep changes scoped to this story only
- [ ] Avoid TODOs unless captured in `progress.txt` with owner

## 5. Quality Gates
- [ ] `cargo fmt --all`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Additional project scripts (e.g., `./scripts/local-ci.ps1`)
- [ ] UI stories: run dev server + `dev-browser` skill to verify UX

## 6. Commit & Update PRD
- [ ] If all checks pass, commit with `feat: <story title> [US-XXX]`
- [ ] Update the story in `prd.json` (`passes: true`, optional notes)
- [ ] Re-run formatter on `prd.json` for stable ordering

## 7. Capture Learnings
- [ ] Append iteration summary to `progress.txt`:
  - What changed
  - Tests run
  - Issues encountered & mitigations
- [ ] Update relevant `AGENTS.md` with patterns/gotchas discovered

## 8. Loop Decision
- [ ] If stories remain (`passes: false`), repeat from Step 1
- [ ] If all stories pass, emit `<promise>COMPLETE</promise>` and open PR
