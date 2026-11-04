---
name: velesdb-extract-outline
description: >
  Extract facts, entity edges and attributes into velesdb-memory WITHOUT a
  generative model, by writing the `outline` extractor's directive grammar
  yourself — one `fact:` / `edge:` / `attr:` directive per line, handed to
  `remember_extracted` with `extractor: "outline"`. This skill is the one the
  user picks for agent-side extraction: a no-LLM setup where
  `memory_status` reports `extraction.configured: false`, a local model too
  small to hold the generative extraction contract, a fully offline
  deployment, or a curated graph that must be exactly what was written down.
  It is never auto-selected — it is installed because the user chose it.
  Do NOT use it for prose nobody will reformat when a working `ollama` /
  `openai` extractor is configured — inferring structure is their job.
  Requires the velesdb-memory MCP server (tools: remember_extracted,
  extraction_status, entity, recall, recall_where); the velesdb-memory skill
  owns the async receipt/polling contract this one relies on.
---

# velesdb-extract-outline — you are the extractor

`remember_extracted` has a deterministic, network-free backend named
`outline`. It runs no model: each non-blank line of the text you send is one
directive, and the graph stored is **exactly** the one you wrote down. That
flips who does the extraction — instead of asking a model to infer structure
from prose, YOU state the structure and the backend stores it verbatim.

That is why this skill exists for small configurations: an agent writing
directives produces a better graph than a small local model failing on the
generative extraction prompt, and it needs nothing running at all.

## Installation

Install: `cp -r skills/velesdb-extract-outline ~/.claude/skills/` (repo
clone; the npm package bundles it at
`node_modules/@wiscale/velesdb-memory-node/skills/velesdb-extract-outline`,
and every GitHub Release attaches it in `velesdb-skills.tar.gz`).

## The directive grammar

One directive per line; blank lines are skipped. Fields are separated by
`|`, and each field is trimmed. The three prefixes are literal and
lowercase:

| line | yields |
|---|---|
| `edge: <subject> \| <predicate> \| <object>` | one typed, directed edge between two entity hubs |
| `attr: <entity> \| <key> \| <json value>` | one typed attribute carried by the entity hub |
| `fact: <text> \| <topic>, <topic>` | one stored fact, wired to each named topic's hub |
| any other non-blank line | one stored fact with **no topics** — recallable, linked to nothing |

- **`edge:` and `attr:` are strict.** Wrong field count, a blank subject /
  predicate / object / entity / key, or an `attr:` value that is not valid
  JSON each fail the whole call with a parse error — a graph that quietly
  lost half of what it was handed would be worse than a refusal. Count your
  `|` separators: a field may not itself contain `|`.
- **`fact:` and bare lines are lenient — so a typo is silent.** Only a line
  starting with `edge:` or `attr:` is parsed strictly. `edg:` or `attr ;`
  miss the prefix entirely and are stored as plain facts: no error, and no
  edge. Spell the prefixes exactly and re-check the lines that matter.
- **Entity names resolve to shared hubs.** Every name — `edge:` endpoints,
  `attr:` entities, `fact:` topics — is canonicalized (trimmed, lowercased)
  and content-addressed, so `Theo Durand` written today and `theo durand`
  written next month land on the SAME node, and they join the hubs a
  generative backend would have produced. Two backends write into one graph.
- **Numbers must stay numbers.** The `attr:` value is parsed as JSON, not
  stored as text: `15` is a number, `"15"` is a string, `true` is a boolean.
  `recall_where`'s range comparisons (`lt`/`le`/`gt`/`ge`) are type-strict
  with no coercion — an age written `"15"` never matches `age >= 15`, with
  no error, just silence. Write `attr: Theo Durand | age | 15`.
- **Direction is written, never inferred.** `edge: A | father of | B` stores
  `A -[father of]-> B`; the converse is not invented. Write the triple in
  the direction the predicate reads, and write both triples yourself when
  the passage states a relationship both ways. A predicate that is empty,
  over 512 bytes, or carries ASCII control characters is skipped at wiring
  time (the facts still land — the edge just never exists), and a self-loop
  whose endpoints canonicalize to the same entity is skipped too.
- **Kinship predicates may be re-pointed.** Like every backend's output,
  `edge:` triples pass through `orient_kinship`: when the text also states
  the kinship possessively, a triple whose predicate is a kinship noun can
  be flipped so the label sits on the person who carries it. It never
  invents or drops an edge.

## Facts carry the prose, edges carry the structure

The three directives do different jobs, and a usable graph needs the first
AND the others:

- `fact:` text is what gets embedded and what `recall` can return. Its
  comma-separated topics are not cosmetic: each one wires a `fact -[about]->
  entity` plus `entity -[mentions]-> fact` pair, which is how a `why` /
  `recall_fused` walk hops from one fact, through a shared hub, to a sibling
  fact that shares no vocabulary with the question. A fact with no topics
  is an island: recallable by its words, invisible to the walk.
- `edge:` triples are the entity→entity layer `entity(name)` reports. They
  store NO sentence — an outline made only of `edge:` lines builds a correct
  graph skeleton with nothing `recall` can find, because entity hubs are
  deliberately invisible to `recall`.
- `attr:` is the filterable layer: `recall_where` and `entity(name)` expose
  it; a value's JSON type is its contract.

So a stated relationship deserves BOTH lines — the `edge:` triple for the
graph and a `fact:` naming the same entities for the sentence a later
question can match.

## Workflow

1. **Check the setup once per session** — `memory_status` answers
   `extraction.configured`. `false` means the daemon has no generative
   extractor and `ollama`/`openai` calls will fail; `extractor: "outline"`
   on the call works regardless, because it is an explicit per-call choice.
2. **Read the passage and write the directives.** One `edge:` per stated
   relationship between named things, one `attr:` per stated property, one
   `fact:` per atomic fact worth recalling — with the entities it concerns
   as topics. Keep one directive per line.
3. **Call `remember_extracted`** with `extractor: "outline"`, the directives
   as `text`, a stable `idempotency_key` for the logical write, and any
   `metadata` (it applies to every extracted fact).
4. **Close the asynchronous loop as the velesdb-memory skill documents it**:
   keep the returned `request_id`, poll `extraction_status` until the job
   commits or fails, and read `skipped_over_cap` — non-zero means part of
   the passage was dropped. That contract (envelope shapes, id strings,
   terminal states) is owned by the velesdb-memory skill and deliberately
   not re-stated here.
5. **Read back through the right lens**: `entity(name)` for what the graph
   says ABOUT a thing — attributes plus `relations`/`relations_in` in both
   directions — and `recall` / `recall_where` for the fact sentences and
   attribute filters.

## When the user picks this — and when not

Choose `outline` (and this skill) when:

- `extraction.configured` is `false` — there is no generative backend at
  all, and `outline` is the only way a graph gets built.
- The configured model is too small for the generative extraction prompt.
  That prompt was written and measured against models that hold the
  contract; a small model's half-formed JSON is how a graph silently ends
  up empty. Writing the directives yourself removes the model from the
  loop entirely.
- The deployment is offline or air-gapped — `outline` needs no model, no
  network, no extra build feature.
- The graph must be exactly what was written: curated imports, fixtures,
  reproducible tests.

Do NOT reach for it when the input is prose nobody is going to reformat and
a generative backend is configured — reformatting prose into directives by
hand is exactly the work `ollama` / `openai` exist to do.

And it stays a choice: nothing auto-selects `outline` for a small model.
The user installs this skill; without it, the per-call `extractor` argument
still exists but nothing teaches you to write the grammar.

## Anti-patterns

- **Handing free prose to `outline`.** Every non-directive line becomes a
  topic-less fact — the store grows sentences but the graph stays flat, and
  `why` degrades to plain search.
- **`edge:` lines with no `fact:` alongside.** The skeleton is right but
  nothing is embedded for `recall` to return — pair each relationship with
  a fact naming its entities as topics.
- **Quoting numbers in `attr:` values.** `"15"` is a string and will never
  satisfy a numeric comparison; this is the most common way extraction
  looks like it works while `recall_where` returns nothing.
- **A `|` inside a field.** Fields are `|`-separated, so the separator
  cannot appear in a name, predicate, or fact text — reword instead.
- **A misspelled prefix.** `edge ;` or `Edge:` are not directives — the
  line silently becomes a fact. The grammar is case-sensitive.
- **Re-running `remember` on each fact afterward.** The call already stored
  them; `remember` only adds duplicates.

## Tool call example

```json
{"tool": "remember_extracted", "arguments": {
  "extractor": "outline",
  "idempotency_key": "family-notes-2026-10",
  "metadata": {"type": "fact", "project": "acme"},
  "text": "fact: Bruno Durand is the father of Theo Durand | Bruno Durand, Theo Durand\nfact: Theo Durand is 15 years old | Theo Durand\nedge: Bruno Durand | father of | Theo Durand\nattr: Theo Durand | age | 15"
}}
```

Stored from that text: two embedded facts each wired to their entity hubs,
a `bruno durand -[father of]-> theo durand` typed edge, and a numeric
`age: 15` attribute on `theo durand` — readable back through
`entity("Theo Durand")`.
