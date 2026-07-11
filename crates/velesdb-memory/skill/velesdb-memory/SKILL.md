---
name: velesdb-memory
description: >
  Use durable, explainable, self-improving memory across a coding session via the
  velesdb-memory MCP server. Trigger whenever the velesdb-memory MCP tools
  (remember/recall/recall_fused/relate/why/feedback/forget) are available and the
  work would benefit from remembering decisions, recalling prior context, or
  answering "why did we do X". Use it at the START of a task (recall what's known),
  when a decision or durable fact emerges (remember + relate it), when asked why
  something is the way it is (why), and after using a recalled memory (feedback).
  Especially relevant for: multi-session projects, architecture/config decisions,
  incident postmortems, "why is this value/setting like this", onboarding to an
  unfamiliar codebase, and any place a fact learned now must survive to a later
  session. Do NOT use for transient chatter, secrets, or one-off scratch notes.
---

# velesdb-memory — the agent memory flow

velesdb-memory gives you **durable local memory** with three properties no plain
vector store has: it **explains** its recalls (`why` returns the evidence trail,
not just look-alike text), it **connects** facts (a typed graph you build as you
work), and it **learns** which memories are worth surfacing (`feedback`). Using it
well is a *loop you run throughout a task*, not a one-shot lookup.

## The loop (run it every task)

1. **Recall before you act.** At the start of a task, retrieve what's already
   known before doing anything else.
   - `recall(query)` for semantic look-up of facts.
   - `recall_fused(query)` when the answer may depend on a *connected* fact the
     query doesn't name directly (multi-hop) — it also walks the graph.
   - `why(question)` when the user asks *why* something is the way it is: it
     returns the seed fact **plus the subgraph that explains it**.
   - If recall returns nothing useful, say so and proceed — don't invent memory.

2. **Remember durable facts and decisions — with metadata AND links.** When a
   decision is made or a durable fact is established, store it. Two things make it
   valuable later, so never skip them:
   - **metadata** (the `ColumnStore` facet): `{ "type": "decision"|"fact"|"incident",
     "area": "payments", "project": "acme", "date": "2026-07-11", "status": "active" }`
     — this is what lets you filter/scope recall later.
   - **links** (the graph facet): connect the new fact to the artifacts it concerns
     — the PR, the ticket, the file, the prior decision it supersedes. **The graph
     is what makes `why` work.** A fact with no edges is invisible to `why`.

3. **Connect facts as relationships appear (`relate`).** Whenever a new fact
   relates to an existing memory, create a typed, directional edge. Good relation
   labels: `caused_by`, `decided_in`, `supersedes`, `references`, `depends_on`,
   `fixes`, `concerns`. This is the differentiator's fuel — build the graph
   incrementally, don't batch it up "later" (later never comes).

4. **Explain with `why`, not `recall`.** When asked to justify a value, a config,
   or a design choice, use `why`: recall alone finds text that *looks* similar;
   `why` follows the links to the decision/incident/ticket that shares **no words**
   with the code but is the actual reason.

5. **Reinforce after use (`feedback`).** After you act on a recalled memory, tell
   the memory whether it helped: `feedback(id, success=true)` if it was useful,
   `success=false` if it was noise. Recall re-ranks by this learned confidence, so
   over time useful facts rise and noise sinks — the memory improves without any
   retraining. Give feedback on the memory you actually used, not on everything.

## Concrete scenarios

**Incident → decision → later "why?"** — the flagship case.
An incident postmortem finds the payment provider's 30 s timeout let a stalled
request pile up and take down checkout. The team drops it to 8 s.
- `remember("Payment provider timeout set to 8s", metadata={type:"decision",
  area:"payments", date:"2026-07-11"})` → returns id `D`.
- `remember("Incident 2026-07-10: 30s payment timeout stalled checkout under load",
  metadata={type:"incident", area:"payments"})` → id `I`.
- `relate(D, I, "caused_by")` and `relate(D, <config-PR fact>, "decided_in")`.
- Six weeks later a new dev asks *"why is the payment timeout only 8 seconds?"* The
  config file just says `timeout = 8`. `why("why is the payment timeout 8s")`
  surfaces the **incident** — the real reason — through the graph, which a vector
  search over the code would never find.

**Onboarding to an unfamiliar codebase.**
You learn that `orders.status` is driven by a state machine, not free-form text.
`remember("orders.status is a strict state machine: created→paid→shipped→closed",
metadata={type:"fact", area:"orders"})` and `relate` it to the ADR and the module.
Next session, `recall("orders status")` restores that context instantly.

**Cross-session continuity.**
At the start of each session on a project, `recall("open decisions <project>")` and
`recall_fused("current architecture <project>")` to rebuild context before touching
code — memory that survived the process restart is exactly the point.

## Anti-patterns

- **Storing everything.** Remember decisions and durable facts, not transient
  conversation, not secrets/tokens, not scratch output.
- **Facts with no edges.** An unlinked fact can be recalled but never *explained*.
  If it relates to something, `relate` it.
- **Recall-and-forget.** Not giving `feedback` leaves the memory unable to learn —
  a quick `feedback` on the memory you used is what makes tomorrow's recall better.
- **Trusting recall quality blindly with the default embedder.** See below.

## Setup notes (know your embedder)

Recall quality depends entirely on the embedding backend the server was built and
launched with:

- **`hash` (default in the prebuilt binary): lexical, NOT semantic.** It matches on
  shared words, so recall of paraphrases is weak. Good enough to demo the *graph*
  (`why` still works — it follows links, not similarity), but for real semantic
  recall configure a semantic embedder.
- **`ollama`:** real on-device semantic recall. Requires a build with
  `--features ollama`, a running Ollama, and `ollama pull all-minilm`; set
  `VELESDB_MEMORY_EMBEDDER=ollama`.

Set a stable store location with `VELESDB_MEMORY_PATH` (e.g. `~/.velesdb-memory`) so
memory persists in one place across sessions. Optionally set
`VELESDB_MEMORY_DEFAULT_TTL` (seconds) to auto-expire facts you only want short-term.

The store never leaves the machine — memory is local by design.
