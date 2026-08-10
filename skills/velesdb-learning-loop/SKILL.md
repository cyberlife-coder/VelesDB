---
name: velesdb-learning-loop
description: >
  Turn every velesdb design / implement / verify cycle into durable, connected
  memory so the SAME mistake is never repeated and later decisions build on
  what was already learned, instead of re-deriving it or silently colliding
  with it. Trigger BEFORE starting a design or implementation on
  velesdb / velesdb-core / velesdb-memory (recall prior decisions and known
  anti-patterns for this area first — don't propose an approach blind).
  Trigger AFTER a bug is found and fixed, a review finding is confirmed, or a
  non-trivial decision is made (check whether this failure mode was already
  recorded before storing it — a match means RECURRENCE, not a fresh fact).
  Trigger BEFORE declaring a change verified (cross-check the four adversarial
  axes — OOM, performance, data loss, vulnerabilities — instead of closing on a
  green suite or a coverage number). Trigger when a stored memory turns out to
  be WRONG (correct it with forget plus a replacement; a stale fact outranks no
  fact and keeps being recalled). Complements the velesdb-memory skill's
  generic recall / recall_fused / remember / relate / entity / why / feedback
  loop with a stricter, project-specific discipline: no design starts blind, no
  fix ships without a recurrence check, no verification closes on coverage
  alone, and writing to memory is never automatic.
---

# velesdb-learning-loop — don't repeat a mistake twice

This skill is not a new set of tools — it's a **discipline on top of**
`velesdb-memory`'s `remember`/`recall`/`recall_fused`/`relate`/`entity`/`why`/
`feedback`, aimed specifically at making design → implementation →
verification on velesdb *compound*: each cycle should make the next one faster
and safer, not just add another isolated fact.

## This policy is binding

For an opted-in repository, the learning loop has four mandatory steps:

1. **Recall** relevant decisions and anti-patterns before the first code edit.
2. **Decision**: store every non-trivial approach, trade-off, or exception that
   a later session must preserve.
3. **Causality**: link the decision to its evidence and an incident to its root
   cause with an outgoing relation.
4. **Feedback**: reinforce or demote every recalled memory that affected the
   work.

The repository opts in with `enforce_learning_loop: true` in
`.velesdb-hooks.json`. The `PreToolUse` guard refuses the first `Edit`/`Write`
or `apply_patch` until `PostToolUse` has observed a successful VelesDB recall
in the same host session and repository. A timeout, an MCP error, or
`compile_context` without a `memory_scope` never creates the sentinel. The
`Stop` guard blocks once on the first Stop so design, diagnosis, and review
sessions without edits still close the loop. It blocks again after each later
covered edit batch. Before consuming those edit records it stores the complete
batch in an atomic pending/delivered manifest, so an interrupted Stop re-emits
every repository identity; the continuation then passes, while a later edit
creates a fresh checkpoint.

The edit target, not just the host `cwd`, selects the repository policy. When
they differ, a refusal queues the target. A successful recall promotes it only
when the target is unambiguous from cwd, an explicit project filter, or a sole
pending record from an unconfigured cwd. An accepted multi-repository patch
records every opted-in target independently for the next `Stop`, which must
save each listed project/session.

Policy discovery canonicalizes the nearest existing parent directory. A final
symlink that crosses an opted-in repository boundary is refused even after
recall; invoke the edit through its physical path instead.

The versioned implementation lives in `pre-tool-use.sh`, `post-tool-use.sh`,
and `stop.sh` under each supported harness integration. Installing the skill
alone teaches the policy; installing and trusting the hooks activates the
mechanical guard.

Be exact about the enforcement boundary: only recall-before-edit is refused
mechanically. Decision, causality, and feedback are policy plus a blocking
continuation reminder; shell commands that mutate files and specialized tool
paths can bypass the edit hook. Never describe this guardrail as a complete
security boundary.

The continuation has a real context cost: at most one extra model turn for a
session with no covered edit, plus one after each later covered edit batch.
Claude can offset large tool-result costs through deterministic replacement;
Codex exposes no equivalent replacement channel. Do not claim that the guard
itself saves tokens or that its overhead is neutral.

The installed hooks require `jq`. The installer and drift check refuse a
missing dependency. If it disappears while a host is running, the edit guard
uses the host's blocking exit code before every covered edit; other lifecycle
reminders may still fail. “Binding” therefore also assumes the hooks are
installed and trusted.

## Writing is never systematic — sort first

Most of what happens in a session must NOT be written. A memory store that
accumulates everything stops being a memory and becomes a log: recall returns
the noise ahead of the fact, and the discipline below quietly stops working
because nothing useful surfaces.

Before storing anything, sort it into one of five:

| It is | What to do |
|---|---|
| **Durable** — a decision, an incident, a measured fact, an anti-pattern that will still be true next month | `remember`, with the metadata convention below, and `relate` it to what it touches |
| **Temporary working state** — where you are in a task, what is next | not `remember`. It belongs to the working-context tools, which are the resumption mechanism, not durable memory |
| **Ephemeral** — a command output, a path you just read, a number you are about to use once | nothing. Re-derive it if you need it again |
| **A secret** — token, credential, private URL, personal data | never, under any framing. Not in a fact, not in metadata, not "temporarily" |
| **An existing memory that is now wrong** | correct it (below). Adding the new version beside the old one leaves both in recall, and the reader cannot tell which is current |

When in doubt, ask whether a *later* session reaching this fact would act on
it. If not, it is noise.

## The four steps and their checkpoints

### 1. Recall

Before proposing an approach or writing code for a feature/module/area, run:

```
recall_fused("<area> design decisions and known pitfalls", filter={"project":"velesdb"})
```

Read what comes back for:

- **decisions already taken** in this area (don't re-litigate or silently
  contradict one — if you must deviate, say so explicitly and `relate` the
  new decision to the old one with `supersedes`).
- **anti-patterns / incidents** tied to this area (a past bug, a rejected
  design, a race condition class). Treat these as constraints, not trivia.

When the area is a **named thing** rather than a topic — a crate, a module, a
tool, a workflow — ask about the thing itself instead of about the sentences
that mention it:

```
entity("velesdb-core")        # what is known ABOUT it, and what it connects to
```

`recall` ranks passages; `entity` returns the node and its edges. Asking
`recall("velesdb-core")` for "what do we know about this crate" gets you the
paragraphs where the name appears, which is a different and usually worse
answer.

If recall returns nothing, say so and proceed — never invent a memory to
fill the gap.

### 2. Decision

When the work selects an approach, accepts a compromise, rejects a plausible
alternative, or creates an exception to an existing rule, remember the
decision while its reason is still known. Use `type: "decision"`, the project
and area metadata below, and wording that states both **what** was chosen and
**why**.

Then create an outgoing relation from the decision to the evidence or cause:

```
relate(decision_id, cause_id, "caused_by")
```

Direction is part of the contract. `why(decision)` follows outgoing edges; a
cause pointing into the decision produces a graph that looks connected from
the other side but cannot explain the decision being queried. If a decision
replaces an earlier one, preserve the history with `supersedes` instead of
silently contradicting it.

### 3. Causality

This is the step most likely to be skipped, and the one that matters most.
When you fix a bug, a race, a silent data-loss path, or a review finding:

1. **Recall first**, using the *failure signature* (symptom + mechanism, not
   just the file name) — e.g. `recall("write path deadlocks under concurrent
   compaction")`, not `recall("bug in wal.rs")`.
2. **Score the match.** A close semantic match (same mechanism, same class of
   trigger — concurrency, boundary, resource exhaustion) is a candidate
   recurrence even if the surface symptom or the file differs.
3. **If it's a likely recurrence:**
   - Say so explicitly to the user — *"this looks like the same root cause as
     <prior incident>, not a new bug"* — don't bury it in a routine commit
     message.
   - `relate(new_incident_id, prior_incident_id, "same_root_cause_as")`
     instead of leaving two disconnected facts that `why` can't join.
   - `remember` a **generalized** anti-pattern fact (the class of mistake,
     not just this instance) if one doesn't already exist, and link both
     incidents to it (`caused_by` → the anti-pattern).
4. **If it's genuinely new:**
   - `remember("<what broke, what the fix was, why>", metadata={"type":
     "incident", "project": "velesdb", "area": "<area>", "date": <YYYYMMDD
     numeric>})`.
   - `relate` it to the fix (PR/commit description as a fact if durable
     enough), the module it concerns, and any design decision it invalidates.

Before calling a cause "root", write the complete chain. A durable incident
must distinguish these seven points:

1. observed symptom;
2. failing boundary or component;
3. trigger and required preconditions;
4. faulty mechanism;
5. invariant that mechanism violated;
6. why existing guards or tests did not catch it;
7. corrective control and recurrence-prevention guard.

If one of those is unknown, label it unknown. A plausible mechanism is a
hypothesis, not a cause. Recall the chain before remembering: a match on the
mechanism and violated invariant is recurrence even when the symptom differs.

Skipping step 1 is how the same class of bug gets fixed twice under two
different names, two years apart, with nothing connecting them.

### 4. Feedback

For every memory surfaced in Step 1 or the recurrence check that influenced
the work, call `feedback(id_str, true)` when it helped and
`feedback(id_str, false)` when it misled. Do this before closing, not only when
repairing a false memory. A recalled item that was ignored because it was
irrelevant still needs a negative outcome if it materially distracted the
reasoning; otherwise ranking cannot learn from use.

Feedback is not a substitute for correction. If the fact itself is false,
follow the forget-and-replace procedure below as well.

### Verification checkpoint — cross-check the adversarial standard

Before marking a change "tested" or a PR "ready", recall the project's own
test standard rather than trusting a green suite or a coverage number:

```
recall_fused("test standard adversarial axes", filter={"project":"velesdb","topic":"test-standard"})
```

Confirm, for the change at hand, that the four axes were exercised — not
merely that lines were covered:

- **OOM** — unbounded allocations, giant payloads, index and cache growth,
  leaks under a long-running process.
- **Performance** — regressions, lock contention, cost under concurrency.
- **Data loss** — WAL, snapshots, compaction, retention and purge, restore
  that is not atomic, anything that rewrites a file in place.
- **Vulnerabilities** — input reaching a query or a path without validation,
  an unauthenticated route, isolation between callers.

A green suite is evidence about the cases someone thought to write. It is not
evidence about these four, and it never becomes evidence about them by being
greener. If a change ships without one of the axes exercised, say which one
and why, before declaring it done — the point of saying it out loud is that a
reader six months later can tell an axis was *considered and skipped* from an
axis that was never looked at.

## Correcting a memory that turned out to be wrong

A stale fact is worse than a missing one: it outranks nothing, it keeps
surfacing, and the reader has no way to know it has been superseded. When a
stored memory is contradicted by something you have just proven:

1. `why(<id>)` first — read what it was based on. Sometimes the fact is right
   and the *conclusion* drawn from it was wrong, which is a different repair.
2. If it is genuinely wrong, `forget(<id>)`, then `remember` the corrected
   version. Two calls, in that order. Leaving the old one and adding the new
   one beside it is the failure this step exists to prevent.
3. If it is not wrong but no longer *current* — a decision that has since been
   superseded — keep both and `relate(new_id, old_id, "supersedes")`. The
   history is worth having; the ambiguity is not.
4. Say which you did. "I corrected a memory" and "I recorded that a decision
   changed" mean different things to whoever reads the trail.

Use `feedback(id, false)` on a memory that surfaced and misled you, and
`feedback(id, true)` on one that actually helped. Ranking only improves if the
outcome comes back; skipping it leaves confidence flat forever.

## Metadata convention (so recall/recurrence-check actually work)

Use these consistently or the recurrence check in step 2 has nothing to match
against:

- `type`: `"decision" | "fact" | "incident" | "anti-pattern" | "milestone" |
  "measurement"`
- `project`: `"velesdb"`
- `area`: the subsystem (`"raft"`, `"wal"`, `"hnsw"`, `"context-compiler"`,
  `"velesql"`, …) — this is what makes an area-scoped `recall_fused` filter
  useful before a design.
- `date`: numeric `YYYYMMDD` (`recall_where`'s comparisons are type-strict —
  see the velesdb-memory skill's date-format note).

## Worked example

A crash-recovery test finds that a compaction mid-write can leave a torn WAL
tail (already fixed once, PR #1011).

- Before writing a NEW compaction-adjacent fix, `recall_fused("compaction
  torn WAL tail write path")` — this should surface PR #1011's fix and its
  reasoning *before* re-deriving the same analysis or, worse, reintroducing
  the same gap in a different code path.
- If a second, distinct torn-tail bug shows up in a different subsystem later,
  the recurrence check surfaces #1011 as `same_root_cause_as`, and a
  generalized anti-pattern ("compaction-adjacent writes need X invariant")
  gets remembered once instead of two unlinked incident facts nobody connects
  six months later.

## Anti-patterns

- **Remembering the fix without checking for its predecessor first.** This is
  the single most valuable step and the easiest to skip under time pressure.
- **Storing the incident but not the generalized anti-pattern.** A recall hit
  on the exact same bug is useful; a recall hit on the *same class* of bug in
  a new file is what actually prevents repeats — that only works if the
  anti-pattern fact exists and is linked.
- **Writing every turn to memory.** The store fills with session chatter, and
  the facts that matter stop being the ones that come back.
- **Adding a corrected fact without removing the wrong one.** Now recall
  returns both and the reader has to guess which is current.
- **Treating coverage % as the verification checkpoint.** It's a floor, not a
  substitute for the 4-axis check.
- **Skipping this loop under time pressure "just this once."** The failure
  mode this skill exists to prevent happens under exactly that pressure —
  a suite that was green while three deterministic blockers sat behind it.

### A green signal can lie

Treat a green result as evidence only after excluding these observed modes:

1. a query-language limit was mistaken for an architectural limit;
2. a property measured below a threshold was generalized above it;
3. asymmetric operations were compared as a ratio;
4. debug-profile performance was presented as product performance;
5. a benchmark changed two paths at once, so it attributed neither;
6. a test called the setup API but never asserted the intended state existed;
7. `0 passed, N filtered out` was read as a passing test;
8. concurrent benchmarks contaminated each other's latency;
9. top-k excluded the target and was mistaken for data loss;
10. a job log was searched for tests that the job never runs.

For every guard, assert the state or effect it protects, run latency measures
alone, and verify what the job actually executes. A guard counts only after
its refusal and the corresponding positive control have both been observed.

### A red signal can lie too

A red exit code can send the work toward code that is healthy. Run the guard
alone and read its message before blaming the change. Exit status `2` may be a
missing file or invocation error rather than the guard's intended refusal.
In zsh, a scalar containing `script.py --flag` is not split into a command and
argument as bash users may expect; use an argument array or spell the command
out. Record the discriminating message, not only the color or status code.

## Where the tool contract lives

This file holds the discipline. The tools' own contract — argument shapes,
return envelopes, embedder and extractor setup, the date-format rule, and the
working-context calls that resume a session — belongs to the **velesdb-memory
skill**, which is the single source of truth for them. Read it there rather
than trusting a paraphrase here: a duplicated contract is a contract that
drifts.

At the start of a session, load the previous working context before
re-deriving anything; at the end, save it. If you do not know the session
name, list the project's sessions before concluding that no earlier work
exists — an empty result for a mistyped name looks exactly like a fresh start.

## A machine-local layer, if one exists

Everything above is generic and ships with the repository. A machine may add
a personal layer beside this file, in `LOCAL.md`, in the same directory as
this `SKILL.md`. It is never versioned and never a copy of what is above.

If that file exists, **read it after** these rules and treat it as a
complement, never a replacement — the generic rules resolve first, the local
layer refines them. The harness does not load it for you.

It must never contain a token, a credential, or personal data.
