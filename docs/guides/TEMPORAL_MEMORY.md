# Temporal Memory: Dated Recall + a Reasoning Scaffold

Teaches you to build "temporal memory" — recalling and reasoning about *when*
things happened — entirely on top of existing `velesdb-memory` capabilities.
There is no new API here: `remember` already accepts arbitrary metadata, and
`recall_where` now returns that metadata (including a stored date) alongside
each fact. Turning a dated timeline into a temporal *answer* ("how long ago
was X?", "what happened before Y?") is your own LLM's job — this guide gives
you a copy-paste prompt recipe for that part, not a library function.

## Store dated facts

Store a fact with a metadata key that holds a date. The key name and the date
format are entirely your choice — `velesdb-memory` treats metadata as opaque,
caller-defined key/value pairs (it imposes nothing beyond rejecting reserved
`_veles_*` keys and the `content` key). A `YYYYMMDD` integer is a convenient
convention because it sorts and range-filters correctly as a plain number:

```python
from velesdb import MemoryService

mem = MemoryService("./agent_mem")

mem.remember("Robert had knee surgery", metadata={"occurred_at": 20260615})
mem.remember("Robert started physical therapy", metadata={"occurred_at": 20260701})
mem.remember("Robert was cleared to run again", metadata={"occurred_at": 20260910})
```

The same call shape exists in Node (`@wiscale/velesdb-memory-node`), Rust
(`MemoryService::remember`), and over MCP (the `remember` tool's `metadata`
argument) — see the [main README](../../crates/velesdb-memory/README.md) for
each surface.

## Recall a dated timeline

`recall_where` is a fused vector + `ColumnStore` query: a semantic query plus
zero or more structured filters over metadata columns (ranges and
comparisons — something a pure vector search can't express). The real call
shape (Python):

```python
# recall_where(query, filters, k=10)
# filters: list of (field, op, value) tuples; op is one of eq/ne/lt/le/gt/ge
hits = mem.recall_where(
    "Robert's knee",
    [("occurred_at", "ge", 20260101), ("occurred_at", "le", 20261231)],
    k=10,
)
```

Each hit is now `{"id", "score", "content", "metadata"}` — the new
`metadata` field carries back whatever caller-supplied keys you stored (here,
`occurred_at`), so the dated fact round-trips out of recall. `metadata` is
absent/`None` for a fact stored with no metadata.

Format the timeline (doc snippet, not a library function — sort and render it
yourself):

```python
timeline = sorted(hits, key=lambda h: h["metadata"]["occurred_at"])
for h in timeline:
    d = str(h["metadata"]["occurred_at"])
    print(f"- [{d[:4]}-{d[4:6]}-{d[6:]}] {h['content']}")
```

That's the entire "temporal memory" surface `velesdb-memory` provides: dated
storage, and dated, chronologically-orderable recall. Everything below —
reasoning about the timeline — is a prompt you write and run against your own
model.

## The temporal-reasoning scaffold (a portable prompt recipe)

**This is a prompt recipe, not a `velesdb-memory` API.** `velesdb-memory`
never calls an LLM to answer a question — it only supplies the dated facts
above. The scaffold below is plain text you paste into your own LLM call
(local or cloud, any provider); adapt it freely.

Paste this template, filling in the timeline (from the previous section),
today's date, and the question:

```text
You answer a temporal question from a dated memory timeline (each line is
'- [YYYY-MM-DD] fact', in chronological order). Today's date is {today}.

Timeline:
{timeline}

Question: {question}

Reason step by step: pick the relevant dated fact(s), then compute the
interval, ordering, or date the question asks for. If the timeline does not
contain the answer, the final answer is NO_ANSWER. End with a line exactly of
the form:
FINAL: <answer in a few words>
```

The three things doing the work: the timeline is pre-sorted and pre-dated (no
date arithmetic left implicit), a "today's date is X" anchor gives the model a
reference point, and the `FINAL:` convention makes the answer trivial to
extract from a chain-of-thought reply. Parse it back out:

```python
def extract_final(reply: str) -> str:
    for line in reversed(reply.splitlines()):
        line = line.strip()
        if line[:6].lower() == "final:":
            return line[6:].strip()
    return reply.strip()

answer = extract_final(llm_call(prompt))
```

This is a direct adaptation of the scaffold prompt and `extract_final` used in
`velesdb-memory`'s own LoCoMo temporal benchmark harness
([`examples/locomo/judge.rs`](../../crates/velesdb-memory/examples/locomo/judge.rs)) —
translated here into a standalone recipe you can run with any model, not
something wired into the library.

## Honest results — read this before you quote a number

We ran the decomposition on the full 10-conversation LoCoMo benchmark
(2,393 extracted facts, 321 temporal / 1,540 answerable questions total;
answers generated fully locally with a local 35B model, graded by Claude
Opus 4.8 as a neutral judge — never in production, only for scoring this
benchmark):

| Configuration | Temporal accuracy | Answerable accuracy (all categories) |
|---|---|---|
| Baseline (no dates, no scaffold) | 17% | 42% |
| **+ dated recall** (this guide's step 2 — `recall_where` + metadata, chronologically ordered) | **53%** (+36pp) | **53%** (+11pp) |
| + temporal-reasoning scaffold (this guide's step 4, on top of dated recall) | 58% (+5pp more) | 51% (−2pp) |

**Dated recall alone — a VelesDB capability, no special prompting — accounts
for nearly all of the lift** (17% → 53% on temporal questions). The scaffold
prompt adds a further +5pp on temporal questions, but that isn't free: it
costs accuracy elsewhere (single-hop dropped from 60% to 54% in our run),
because routing a chain-of-thought prompt at every temporal-looking question
trades breadth for depth. Treat the scaffold as an optional, situational
technique to A/B test on your own workload — not a strict upgrade.

We deliberately never say "VelesDB temporal accuracy: X%" — the reasoning is
your LLM's, not the database's. What the numbers above show is that VelesDB's
contribution (dated, chronologically-ordered recall) is where nearly all of
the measured lift comes from; the scaffold is a portable prompt pattern you
can adapt to your own model, with a trade-off you should measure before
adopting it.

## See also

- [`crates/velesdb-memory/README.md`](../../crates/velesdb-memory/README.md) — full tool reference (`remember`, `recall`, `recall_where`, `relate`, `forget`, `why`, `remember_extracted`)
- [`docs/guides/AGENT_MEMORY.md`](AGENT_MEMORY.md) — the broader Agent Memory SDK guide (semantic/episodic/procedural memory, TTL, snapshots)
