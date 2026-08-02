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

## The three checkpoints

### 1. Before design/implementation — recall, don't assume

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

### 2. After a bug is found and fixed — check recurrence BEFORE remembering

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

Skipping step 1 is how the same class of bug gets fixed twice under two
different names, two years apart, with nothing connecting them.

### 3. Before closing verification — cross-check the adversarial standard

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
