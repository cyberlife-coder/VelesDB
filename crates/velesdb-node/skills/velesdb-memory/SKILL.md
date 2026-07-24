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

## Installation

Install: `cp -r crates/velesdb-memory/skill/velesdb-memory ~/.claude/skills/`
(repo clone). No repo clone? Every
[GitHub Release](https://github.com/cyberlife-coder/VelesDB/releases/latest)
attaches `velesdb-skills.tar.gz` (both bundled skills, one folder per skill):
`curl -L https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-skills.tar.gz | tar -xz -C ~/.claude/skills/`.
The npm package bundles it too, at
`node_modules/@wiscale/velesdb-memory-node/skills/velesdb-memory`.
Server setup: [velesdb-memory README](https://github.com/cyberlife-coder/VelesDB/blob/main/crates/velesdb-memory/README.md#configure-your-client).

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
     "area": "payments", "project": "acme", "status": "active" }` — this is what
     lets you filter/scope recall later. **You do not need to manage a date field
     yourself.** Every `remember`/`remember_extracted` call auto-stamps
     `_veles_date` — today's date as a `YYYYMMDD` integer — unless you already set
     it, so `recall_fused(date_field="_veles_date")` gives you a chronological
     `dated_context` timeline (plus a `now` anchor) with zero setup on your part
     (Node binding: the dated variant is its own method,
     `recallFusedDated(query, k, filter, opts, "_veles_date")`).
     Set `_veles_date` explicitly only to override the default — e.g. to date a
     fact by when an incident actually happened, not when you recorded it; once
     set, the server never overwrites it. Store any OTHER comparable value
     NUMERICALLY too (`20260711`, not `"2026-07-11"`): `recall_where`'s
     range/comparison filters (`lt`/`le`/`gt`/`ge`) are type-strict with no
     coercion (issue #1473) — a numeric filter value never matches a
     string-stored one, silently returning nothing, no error. Plain equality
     filters on `recall`/`recall_fused` are unaffected either way.
   - **links** (the graph facet): connect the new fact to the artifacts it concerns
     — the PR, the ticket, the file, the prior decision it supersedes. **The graph
     is what makes `why` work.** A fact with no edges is invisible to `why`.

3. **Connect facts as relationships appear (`relate`).** Whenever a new fact
   relates to an existing memory, create a typed, directional edge. **Direction
   rule**: `why` walks *outgoing* edges only — always point `from` at the thing
   you will later ask about and `to` at its evidence (decision → cause,
   fact → source). An edge pointing *into* a memory is invisible to
   `why(that memory)`. Good relation
   labels: `caused_by`, `decided_in`, `supersedes`, `references`, `depends_on`,
   `fixes`, `concerns`. This is the differentiator's fuel — build the graph
   incrementally, don't batch it up "later" (later never comes).
   **Use `id_str`, not `id`, for `from`/`to`** (and for `feedback`/`forget`'s id
   too): every id in a response also comes back as a decimal-string `id_str`
   twin specifically because a raw JSON-number id can exceed 2^53 and get
   rounded by a float-lossy client on the way back in, silently pointing
   `relate` at the wrong memory — relay `id_str` verbatim instead of retyping
   the numeric `id`.
   **Harness caveat**: some MCP harnesses coerce any all-digit scalar (even a
   JSON string) back into a JSON number before it reaches the server, which
   defeats `id_str` and reintroduces the precision loss it exists to avoid. If
   that happens, prefix the id with `+` (e.g. `"+12732540571541475285"`) — not
   a valid JSON number, so the harness leaves it as a string, and the id
   parser accepts the leading `+`. Surrounding whitespace in an id string is
   also tolerated (trimmed before parsing).

4. **Explain with `why`, not `recall`.** When asked to justify a value, a config,
   or a design choice, use `why`: recall alone finds text that *looks* similar;
   `why` follows the links to the decision/incident/ticket that shares **no words**
   with the code but is the actual reason.

5. **Reinforce after use (`feedback`).** After you act on a recalled memory, tell
   the memory whether it helped: `feedback(id_str, success=true)` if it was useful,
   `success=false` if it was noise — pass the recalled memory's `id_str`, not its
   numeric `id` (see the `id_str` note above). Recall re-ranks by this learned
   confidence, so over time useful facts rise and noise sinks — the memory
   improves without any retraining. Give feedback on the memory you actually
   used, not on everything.

## Concrete scenarios

**Incident → decision → later "why?"** — the flagship case.
An incident postmortem finds the payment provider's 30 s timeout let a stalled
request pile up and take down checkout. The team drops it to 8 s.
- `remember("Payment provider timeout set to 8s", metadata={type:"decision",
  area:"payments"})` → returns id `D`. No need to set a date — `_veles_date`
  auto-stamps today's date as a numeric `YYYYMMDD`, as covered above.
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
