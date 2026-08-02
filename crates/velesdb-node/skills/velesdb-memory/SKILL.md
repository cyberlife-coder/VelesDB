---
name: velesdb-memory
description: >
  Use durable, explainable, self-improving memory across a coding session via the
  velesdb-memory MCP server. Trigger whenever the velesdb-memory MCP tools
  (remember/recall/recall_fused/relate/why/feedback/forget/remember_extracted/entity)
  are available and the
  work would benefit from remembering decisions, recalling prior context, or
  answering "why did we do X". Use it at the START of a task (recall what's known),
  when a decision or durable fact emerges (remember + relate it), when asked why
  something is the way it is (why), and after using a recalled memory (feedback).
  Use remember_extracted when a passage states relationships or properties of
  named people/places/things, and entity(name) to answer a question ABOUT such a
  thing rather than about the sentences that mention it.
  Use it ACROSS sessions too: list_working_contexts to discover what a project
  already has, load_working_context to pick up a previous session's hand-off,
  and save_working_context to leave one. When a load returns found:false, or
  when you do not know the exact session name, list the project's contexts
  before concluding that no earlier work exists — a mistyped id looks exactly
  like a fresh start.
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
     (Node binding: the dated variant is its own method, and it takes the date
     field SECOND, not last:
     `recallFusedDated(query, "_veles_date", k, filter, opts)`).
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
   - **Before storing a new incident/bug/anti-pattern, check for recurrence.**
     `recall`/`recall_fused` using the *failure signature* — symptom + mechanism,
     not the file name (`"write path deadlocks under concurrent compaction"`, not
     `"bug in wal.rs"`) — a close semantic match (same trigger class: concurrency,
     boundary, resource exhaustion) is a candidate recurrence even when the surface
     symptom or file differs. If one turns up: say so explicitly rather than
     storing a disconnected duplicate, `relate(new, prior, "same_root_cause_as")`,
     and — if this is the second time the same *class* of mistake shows up under a
     different name — `remember` one generalized `metadata: {"type":
     "anti-pattern"}` fact describing the class itself (not just this instance),
     linked to both incidents. A recall hit on the exact same bug is useful; a
     recall hit on the *same class* of bug in a new file is what actually prevents
     repeats, and that only works once the anti-pattern fact exists and is linked.
     Skipping this check is how the same mistake gets "discovered" and fixed twice,
     two names apart, with nothing connecting them.

3. **Connect facts as relationships appear (`relate`).** Whenever a new fact
   relates to an existing memory, create a typed, directional edge. **Direction
   rule**: `why` walks *outgoing* edges only — always point `from` at the thing
   you will later ask about and `to` at its evidence (decision → cause,
   fact → source). An edge pointing *into* a memory is invisible to
   `why(that memory)`. Good relation
   labels: `caused_by`, `decided_in`, `supersedes`, `references`, `depends_on`,
   `fixes`, `concerns`. This is the differentiator's fuel — build the graph
   incrementally, don't batch it up "later" (later never comes).
   **Pass the response's `id_str` STRING into the `id`/`from`/`to` parameter**
   (same for `feedback`/`forget`) — the parameter is still named `id` (there is
   no separate `id_str` *input* field anywhere; `id_str` only exists in
   *responses*), but what you put in it matters: every id in a response also
   comes back as a decimal-string `id_str` twin specifically because a raw
   JSON-number id can exceed 2^53 and get rounded by a float-lossy client on
   the way back in, silently pointing `relate` at the wrong memory — relay
   the `id_str` value verbatim into `id` instead of retyping the numeric
   `id`.
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

## Resuming a session: list → load → work → save

The loop above runs *inside* one session. A **working context** is what crosses
between them: a distilled hand-off — goal, active constraints, verified facts,
open hypotheses, decisions, evidence handles, pending actions — stored under a
`project` and a `session` id and read back by whoever comes next, which is
usually you, tomorrow, with none of today's context.

This is memory, not compression. `save_working_context` embeds and stores; it
returns a fact id. The compression skill (`velesdb-context-optimizer`) shrinks
one prompt with a pure function and keeps nothing — a different mechanism for a
different problem. It points here when a distilled context deserves to survive.

**1. `list_working_contexts` — discover before you assume.**
Run it when you do not know the exact session name, and whenever a load comes
back empty. Session ids are chosen by hand (`"rolling"`, a task id, a
conversation id), which means they are mistyped by hand too.

**2. `load_working_context(project, session)` — read the hand-off.**
It returns `{found, working, other_sessions}`.

- `found: true` — adopt `working.goal`, re-assert `active_constraints`, trust
  `verified_facts`, and continue from `pending_actions` instead of re-deriving
  them. Fetch `exact_evidence` handles with `retrieve_context_source` when you
  need the actual bytes.
- `found: false` (with `working: null`) — **this is not proof that no earlier
  work exists.** It says nothing was ever saved under *that exact pair*. Read
  `other_sessions`, or call `list_working_contexts`, before you say "fresh
  start": a similarly-named session in that list means the id was a typo and
  the work is sitting one character away from where you looked.
- `other_sessions` is filled in **on a hit too**. If one of them looks more
  like the session you meant, you may have just resumed the *wrong* one — the
  failure that reads as success.

**3. Work**, running the loop above: recall, remember, relate, feedback.

**4. `save_working_context(project, session, working)` — leave a hand-off.**
Near the end of a session, or whenever the state changes meaningfully. Keep it
distilled: this is the note, not the transcript. Saving again under the same
project and session **replaces** the previous state, so a resumed session
should re-save rather than accumulate. An entirely empty `working` is refused
instead of being allowed to wipe what the last save stored.

```json
{"tool": "list_working_contexts", "arguments": {"project": "veles"}}
```

```json
{"tool": "load_working_context", "arguments": {"project": "veles", "session": "task-1234"}}
```

```json
{"tool": "save_working_context", "arguments": {
  "project": "veles", "session": "task-1234",
  "working": {
    "goal": "fix the failing canary deploy",
    "active_constraints": [{"text": "never restart the primary during a rebalance"}],
    "verified_facts": [{"text": "the canary fails only on arm64 runners"}],
    "pending_actions": ["bisect the arm64-only failure", "re-run the canary"]
  }
}}
```

**Field shapes are not uniform, and guessing costs a confusing error.**
`active_constraints`, `verified_facts` and `open_hypotheses` are lists of
`{text, source?}` objects; `decisions` and `exact_evidence` are typed structs
where `fragment_id` is required; **`pending_actions` is a plain list of
strings.** Sending `pending_actions: [{"text": "..."}]` fails with a `missing
field fragment_id` that never names the field actually at fault. Follow the
example above literally rather than inventing one shape for every field.

## Entities: let the graph build itself

Steps 2 and 3 build the graph by hand, one `relate` at a time. That is the right
tool for a *decision* graph, where you choose the edges deliberately. It is the
wrong tool for facts about **people, places, organisations and things**, where
the edges are simply what the sentence already says.

`remember_extracted(text)` reads a passage and stores three things at once: the
atomic facts, the typed edges **between named entities**, and the **attributes**
those entities carry. Say it in plain language and the graph assembles itself:

> "Bruno Durand est le père d'Theo Durand. Theo Durand a 15 ans.
>  Theo Durand a une sœur, Camille Durand."

produces `bruno durand -[père de]-> theo durand`, an `age: 15` attribute on Theo,
and a brand-new `camille durand` node wired by `sœur de`. No `relate` calls.

**Entity names resolve across calls and across sessions.** An entity's id is
content-addressed from its (lowercased) name, so "Theo Durand" in today's
sentence and "theo durand" in next month's land on the *same* node. Attributes
accumulate onto it — learning the sister does not erase the age.

**Read entities with `entity(name)`, not `recall`.** This matters and is easy to
get wrong. Entity nodes are deliberately invisible to `recall` and
`recall_where`: a node called `Entity: theo durand` would rank for its own name
and evict a real fact from your results. So:

- *"What does the memory say about Theo?"* → `entity("Theo Durand")` — returns his
  attributes and every typed edge touching him, in BOTH directions:
  `relations` leave him, `relations_in` point AT him. Ask only the first and
  the graph looks half empty — it holds `camille --sister of--> theo`, so
  Theo's outgoing edges never mention Camille.
- *"Which notes mention Theo?"* → `recall("Theo Durand")` — returns sentences.

Use `entity` for questions **about a thing**, `recall` for questions **about what
was written**. `found: false` means nothing has ever mentioned that name.

**Numbers must stay numbers.** Attributes keep the JSON type the extractor
produced, and `recall_where`'s comparisons are type-strict with no coercion. An
age stored as `"15"` will never match `age >= 15` — no error, just silence. This
is the same trap as the date field in step 2, and it is the single most common
way a memory system looks like it is working while returning nothing.

`remember_extracted` needs an extraction backend; without one the tool reports
itself as unconfigured rather than silently storing less. Two exist, and they
are **not** interchangeable:

- `"ollama"` runs a local generative model that **infers** the facts, entity
  edges and attributes a passage states. It needs that model running, and a
  binary built with `--features extract`.
- `"outline"` is deterministic and fully offline — no model, no network, and no
  extra build feature. But it only reads structure you write out **explicitly**,
  one directive per line (`fact:`, `edge:`, `attr:`). Hand it free prose and you
  get plain facts with no graph around them.

Pick `"outline"` when you control the input format, or to get a graph at all
without running a model. Pick `"ollama"` when the input is prose nobody is
going to reformat.

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
- **`openai`:** any OpenAI-compatible server — oMLX, llama.cpp, LM Studio,
  vLLM, or a hosted provider. Same `--features ollama` build (that feature
  carries the HTTP dependency for the embedding role, and its name predates
  the protocol split). `openai` names a **protocol, not a vendor**: reaching a
  different server is a different URL, never a new backend name. It therefore
  has **no default URL and no default model** — set
  `VELESDB_MEMORY_EMBEDDER_URL` and `_MODEL` yourself.

The extraction role takes the same three-way choice — `outline`, `ollama`,
`openai` — under `VELESDB_MEMORY_EXTRACTOR`, and **the two roles are
configured independently**: nothing requires them to share a backend, a server
or a token. Embedding on a local Ollama while extracting on an
OpenAI-compatible server is a supported combination.

The base URL may be written **with or without** the `/v1` suffix. Server
consoles advertise the version-prefixed form (`http://127.0.0.1:8019/v1`)
beside a copy button, so pasting it works instead of producing
`/v1/v1/embeddings` and a `404`.

**A store is fixed to one embedding MODEL, not to one backend.** The store
records the model that filled it (`embedding-provenance.json`) and refuses to
open under a different one — including a different model of the same width,
which the dimension check alone cannot see. Changing only the *transport* is
safe: the same model over Ollama or over an OpenAI-compatible API produces the
same vectors, so the backend is deliberately not part of the record. A store
created before this recording existed stays unrecorded and is checked on the
dimension alone, and says so.

### Configure it in a file, not in a plist

Every setting can live in **`velesdb-memory.toml`**, looked for next to the store
(`~/.velesdb-memory/velesdb-memory.toml`), or named explicitly with `--config` /
`VELESDB_MEMORY_CONFIG`:

```toml
path = "~/.velesdb-memory"     # where memory lives; keep it stable across sessions
default_ttl = 0                # seconds; 0 = permanent

[embedder]
backend = "ollama"             # `hash` is lexical, not semantic — see above
model = "bge-m3"
# url = "http://127.0.0.1:8019"  # required by "openai"; defaulted by "ollama"

[extractor]
backend = "ollama"             # or "outline"/"openai" — see above
model = "qwen3.6:35b-mlx"      # required by "ollama" and "openai"; "outline" needs none
# url = "http://127.0.0.1:8019"  # required by "openai"; defaulted by "ollama"

# There is deliberately NO api_token field, in either section. A token is read
# from VELESDB_MEMORY_EMBEDDER_API_TOKEN / VELESDB_MEMORY_EXTRACTOR_API_TOKEN
# and from nowhere else — a credential at rest in a versionable file is one
# `git add .` away from a public history. Writing one here is refused at
# startup, and the refusal does not echo the line back.

[graph]
autograph = false              # true = every `remember` also wires entities
```

With `autograph = true` you stop having to choose the right tool: a plain
`remember("Bruno Durand est le père d'Theo Durand")` stores the fact verbatim
**and** wires the graph around it. It costs one generation per `remember`, so
it is opt-in — and if the model is down the write still succeeds, losing only
the enrichment, never the fact.

Precedence is **command line > environment > file > default**, so a value pinned
in the file can still be overridden for one run
(`VELESDB_MEMORY_EMBEDDER=hash velesdb-memory`). Every setting also remains
available as a `VELESDB_MEMORY_*` variable — the file changes nothing about how
they are read.

An unknown key is a startup error, not a warning: a typo that was silently
ignored would leave you convinced you had configured something you had not.

The store never leaves the machine — memory is local by design.
