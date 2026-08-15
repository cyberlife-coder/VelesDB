#!/usr/bin/env python3
"""
Measure where velesdb-memory's write path spends its time, and which local
model to run its graph extraction on.

Four subcommands, cheapest first:

  from-log   Digest the daemon's own `mcp tool call` events into a per-tool
             latency map. Costs nothing, needs no model, and is the only view of
             what real sessions actually paid.
  screen     Phase A. Send each scenario straight to the model's API and score
             the JSON it returns. Eliminates a candidate on a broken schema, a
             truncated answer or an invented relation before anything more
             expensive runs.
  endtoend   Phase B. Drive a DISPOSABLE daemon through its real MCP tools over
             its real transport, and assert on the graph that comes back out.
  report     Render the frozen markdown report from a result file. The report is
             generated, never typed: that is what keeps the published table and
             the measurement behind it from drifting apart.

Three rules the rest of the file exists to enforce.

**The bench must not drift from the product.** The prompt and the generation cap
are read out of `crates/velesdb-memory/src/extract.rs`, and the request body is
the one `openai::chat_body` builds — `temperature: 0` and `max_tokens` included.
A bench that hand-copies the prompt measures a fossil: the 2026-08-07 campaign
sent no cap at all, so it never once exercised the truncation the cap introduced.

**One set of expectations, two levels.** Phase B does not restate what phase A
checks. The same declarative `relation_present` / `attribute_on` specs are
applied to the stored graph instead of the raw JSON, so the two levels cannot
disagree about what a passage means.

**A number is unreadable without its conditions.** Every scored run carries the
cold-load time, the warm-up trace and the residency proof that surround it. A
p95 with no idea whether it was taken cold, warm, or under the memory guard is
not a measurement.

Usage:

    python3 scripts/bench-memory-extraction.py from-log [--log PATH] [--since ISO]
    python3 scripts/bench-memory-extraction.py screen --config MODEL [--runs N]
    python3 scripts/bench-memory-extraction.py endtoend --config MODEL --binary PATH
    python3 scripts/bench-memory-extraction.py report --results PATH
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import time
import unicodedata
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXTRACT_RS = ROOT / "crates" / "velesdb-memory" / "src" / "extract.rs"
CASES_FILE = Path(__file__).resolve().parent / "memory-extraction-cases.json"
DEFAULT_LOG = Path.home() / "Library" / "Logs" / "velesdb-memory" / "daemon.err.log"

# The daemon's tool-level trace event (mcp.rs, issue #1780). Pinned as a whole
# line shape rather than a loose `elapsed_ms` grep: the same key appears on the
# HTTP-layer event, and mixing the two averages a 200-second `remember` with a
# 0-millisecond POST.
TOOL_EVENT_RE = re.compile(
    r"tool=(?P<tool>\S+) session=(?P<session>\S+) "
    r"verdict=(?P<verdict>\S+) elapsed_ms=(?P<ms>\d+)"
)
HTTP_EVENT_RE = re.compile(
    r"mcp http request method=(?P<method>\S+) session=\S+ "
    r"status=(?P<status>\d+) elapsed_ms=(?P<ms>\d+)"
)
LOG_TIMESTAMP_RE = re.compile(r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})")

SEVERITIES = ("fatal", "major", "minor")


# --------------------------------------------------------------------- text --


def fold(value: object) -> str:
    """Case- and accent-fold for matching.

    The ligatures are mapped by hand because NFKD does not decompose them:
    U+0153 has no decomposition at all, so the ASCII pass drops it outright and
    `soeur` folds to `sur`. Every kinship check in the suite would then stop
    matching without failing — the worst kind of broken oracle, one that reports
    success.
    """
    text = str(value).lower()
    for ligature, expansion in (("œ", "oe"), ("æ", "ae")):
        text = text.replace(ligature, expansion)
    return unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()


def strip_code_fence(content: str) -> str:
    """Drop a markdown fence some models wrap their JSON in.

    Tolerated rather than scored: the contract says "no markdown fence", but a
    fence is recoverable and uninteresting, while what the JSON says is not.
    Models that fence are visible in the raw answers, not disqualified for it.
    """
    text = content.strip()
    if not text.startswith("```"):
        return text
    parts = text.split("```")
    return parts[1].removeprefix("json").strip() if len(parts) > 1 else text


# ------------------------------------------------------- prompt, from source --


def _unescape_rust_literal(raw: str) -> str:
    """Turn one Rust string literal's body into the text it denotes."""
    # Line continuations first: a trailing backslash eats the newline and the
    # indentation that follows it.
    text = re.sub(r"\\\n\s*", "", raw)
    for escape, literal in (("\\n", "\n"), ("\\t", "\t"), ('\\"', '"'), ("\\\\", "\\")):
        text = text.replace(escape, literal)
    return text


def read_graph_prompt_template(source: Path = EXTRACT_RS) -> str:
    """Read `build_graph_prompt`'s literal out of the crate.

    Sourced rather than copied so the bench cannot measure a prompt the product
    no longer sends. A parse failure is raised, never defaulted: quietly falling
    back to a bundled copy is exactly the drift this guards against.
    """
    text = source.read_text(encoding="utf-8")
    start = text.find("fn build_graph_prompt(")
    if start < 0:
        raise RuntimeError(f"build_graph_prompt not found in {source}")
    # The literal runs to its first UNESCAPED quote. A lazy `"(.*?)"\s*\)` looks
    # equivalent and is not: the prompt contains `(e.g. \"bruno durand\")`, whose
    # `")` ends the match two thirds of the way in, and the bench then measures a
    # prompt that stops mid-sentence.
    literal = re.search(r'format!\(\s*"((?:[^"\\]|\\.)*)"', text[start:], re.DOTALL)
    if not literal:
        raise RuntimeError(f"no format! literal under build_graph_prompt in {source}")
    return _unescape_rust_literal(literal.group(1))


def read_generation_cap(source: Path = EXTRACT_RS) -> int:
    """Read `MAX_GENERATION_TOKENS` out of the crate, for the same reason."""
    text = source.read_text(encoding="utf-8")
    match = re.search(r"const MAX_GENERATION_TOKENS: u32 = (\d+);", text)
    if not match:
        raise RuntimeError(f"MAX_GENERATION_TOKENS not found in {source}")
    return int(match.group(1))


def build_graph_prompt(passage: str, template: str) -> str:
    """Apply the crate's template to one passage.

    The template is a Rust `format!` string: `{text}` is the passage and every
    literal brace is doubled. The passage goes in BEFORE the braces are
    collapsed, so a brace inside the passage stays a brace.
    """
    filled = template.replace("{text}", passage)
    prompt = filled.replace("{{", "{").replace("}}", "}")
    # The LAST marker is the one that matters. A truncated parse still carries
    # the passage and the opening sections, so a guard that only looks at the
    # head passes on a prompt cut two thirds of the way through — which is
    # exactly what the first version of this function shipped.
    for marker in (passage, "STEP 0", '"relations"', '"attributes"', '"entity": string'):
        if marker not in prompt:
            raise RuntimeError(f"prompt built from source lacks {marker!r}; the parse is wrong")
    if not prompt.rstrip().endswith("}"):
        raise RuntimeError("prompt built from source does not end on the JSON contract")
    return prompt


# -------------------------------------------------------------- the scorer ----


def relation_triples(payload: dict) -> "list[tuple[str, str, str]] | None":
    """Fold `relations` into comparable triples, or None if the schema broke.

    None is the case that matters: a model answering with arrays instead of
    objects produces JSON that parses and enrichment that silently vanishes,
    because `RawRelation` refuses it and autograph is deliberately infallible.
    """
    triples = []
    for relation in payload.get("relations") or []:
        if not isinstance(relation, dict):
            return None
        triples.append((
            fold(relation.get("subject", "")),
            fold(relation.get("predicate", "")),
            fold(relation.get("object", "")),
        ))
    return triples


def _endpoints_match(triple: "tuple[str, str, str]", spec: dict) -> bool:
    """Do the triple's two ends satisfy the spec? Absent keys constrain nothing."""
    subject, _, obj = triple
    if "subject" in spec and subject != fold(spec["subject"]):
        return False
    if "object" in spec and obj != fold(spec["object"]):
        return False
    return "object_contains" not in spec or fold(spec["object_contains"]) in obj


def _predicate_matches(predicate: str, spec: dict) -> bool:
    """Any listed term appearing in the predicate is a match; no list matches all."""
    wanted = spec.get("predicate_any")
    return not wanted or any(fold(term) in predicate for term in wanted)


def _matches(triple: "tuple[str, str, str]", spec: dict) -> bool:
    """Does one triple satisfy a relation spec?"""
    return _endpoints_match(triple, spec) and _predicate_matches(triple[1], spec)


def _check_relation_present(ctx: dict, spec: dict) -> bool:
    return any(_matches(triple, spec) for triple in ctx["triples"])


def _check_relation_absent(ctx: dict, spec: dict) -> bool:
    return not any(_matches(triple, spec) for triple in ctx["triples"])


def _check_relations_empty(ctx: dict, _spec: dict) -> bool:
    return not ctx["triples"]


def _check_min_relations(ctx: dict, spec: dict) -> bool:
    return len(ctx["triples"]) >= spec["n"]


def _check_single_edge_between(ctx: dict, spec: dict) -> bool:
    pair = {fold(name) for name in spec["entities"]}
    return len([t for t in ctx["triples"] if {t[0], t[2]} == pair]) <= 1


def _check_predicate_forbids(ctx: dict, spec: dict) -> bool:
    banned = [fold(term) for term in spec["substrings"]]
    return not any(term in predicate for _, predicate, _ in ctx["triples"] for term in banned)


def _check_predicate_max_words(ctx: dict, spec: dict) -> bool:
    return all(len(predicate.split()) <= spec["n"] for _, predicate, _ in ctx["triples"])


def _attributes(ctx: dict) -> "list[dict]":
    return [a for a in ctx["payload"].get("attributes") or [] if isinstance(a, dict)]


def _attribute_selected(attribute: dict, spec: dict) -> bool:
    """Is this the attribute the spec is about?"""
    keys = [fold(key) for key in spec["key_any"]]
    if not any(key in fold(attribute.get("key", "")) for key in keys):
        return False
    return "entity" not in spec or fold(attribute.get("entity", "")) == fold(spec["entity"])


def _is_number(value: object) -> bool:
    """A JSON number, and not a bool.

    `isinstance(True, int)` is True in Python, so an age of `true` would sail
    through a naive numeric check and be stored as one.
    """
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _check_attribute_number(ctx: dict, spec: dict) -> bool:
    for attribute in _attributes(ctx):
        if not _attribute_selected(attribute, spec):
            continue
        value = attribute.get("value")
        return _is_number(value) and ("value" not in spec or value == spec["value"])
    return False


def _attribute_holders(ctx: dict, value: str) -> "list[str]":
    wanted = fold(value)
    return [
        fold(a.get("entity", ""))
        for a in _attributes(ctx)
        if wanted in fold(a.get("value", ""))
    ]


def _check_attribute_on(ctx: dict, spec: dict) -> bool:
    return fold(spec["entity"]) in _attribute_holders(ctx, spec["value"])


def _check_attribute_not_on(ctx: dict, spec: dict) -> bool:
    return fold(spec["entity"]) not in _attribute_holders(ctx, spec["value"])


def _facts_text(ctx: dict) -> str:
    facts = ctx["payload"].get("facts") or []
    joined = " ".join(fold(f.get("fact", "")) for f in facts if isinstance(f, dict))
    # Padded so a check for " il habite" also matches a fact that opens with it.
    return f" {joined} "


def _check_facts_mention(ctx: dict, spec: dict) -> bool:
    return all(fold(term) in _facts_text(ctx) for term in spec["substrings"])


def _check_facts_forbid(ctx: dict, spec: dict) -> bool:
    return not any(fold(term) in _facts_text(ctx) for term in spec["substrings"])


def _check_not_truncated(ctx: dict, _spec: dict) -> bool:
    return not ctx.get("truncated", False)


CHECKS = {
    "relation_present": _check_relation_present,
    "relation_absent": _check_relation_absent,
    "relations_empty": _check_relations_empty,
    "min_relations": _check_min_relations,
    "single_edge_between": _check_single_edge_between,
    "predicate_forbids": _check_predicate_forbids,
    "predicate_max_words": _check_predicate_max_words,
    "attribute_number": _check_attribute_number,
    "attribute_on": _check_attribute_on,
    "attribute_not_on": _check_attribute_not_on,
    "facts_mention": _check_facts_mention,
    "facts_forbid": _check_facts_forbid,
    "not_truncated": _check_not_truncated,
}


def score_passage(payload: "dict | None", checks: "list[dict]",
                  truncated: bool = False) -> "list[dict]":
    """Run one passage's checks, returning the failures.

    An unusable payload short-circuits to a single fatal finding: running the
    other checks against nothing reports a dozen derived failures and buries the
    one that caused them.
    """
    if payload is None:
        return [{"severity": "fatal", "label": "response is not valid JSON", "type": "parse"}]
    triples = relation_triples(payload)
    if triples is None:
        return [{"severity": "fatal", "label": "relations are not objects", "type": "schema"}]
    ctx = {"payload": payload, "triples": triples, "truncated": truncated}
    failures = []
    for spec in checks:
        check = CHECKS.get(spec["type"])
        if check is None:
            raise RuntimeError(f"unknown check type {spec['type']!r}")
        if not check(ctx, spec):
            failures.append(
                {"severity": spec["severity"], "label": spec["label"], "type": spec["type"]}
            )
    return failures


def _predicate_set(payload: "dict | None") -> "set[str]":
    triples = relation_triples(payload) if payload else None
    return {predicate for _, predicate, _ in triples or []}


def score_cross_checks(payloads: "list[dict | None]", checks: "list[dict]") -> "list[dict]":
    """Run a paired case's cross-passage checks.

    The point of a close pair: the two passages differ by one word, so identical
    predicate sets mean the model collapsed a distinction the passage draws, and
    the graph inherits the collapse.
    """
    failures = []
    for spec in checks:
        if spec["type"] != "predicates_differ":
            raise RuntimeError(f"unknown cross-check type {spec['type']!r}")
        sets = [_predicate_set(payload) for payload in payloads]
        if len(sets) == 2 and sets[0] and sets[0] == sets[1]:
            failures.append(
                {"severity": spec["severity"], "label": spec["label"], "type": spec["type"]}
            )
    return failures


def tally(failures: "list[dict]") -> "dict[str, int]":
    counts = {severity: 0 for severity in SEVERITIES}
    for failure in failures:
        counts[failure["severity"]] += 1
    return counts


# ---------------------------------------------- the same checks, on the graph --


def entity_as_payload(name: str, profile: dict) -> dict:
    """Reshape an `entity` response into what the phase-A checks already read.

    Phase B asks the same questions of the stored graph rather than a second set
    of expectations: the outgoing edges become triples, the attribute map becomes
    entity/key/value rows. `relations_in` is deliberately left out — an incoming
    edge belongs to its SOURCE entity, and folding it in here would credit this
    entity with an edge it does not carry.
    """
    subject = fold(profile.get("name") or name)
    relations = [
        {
            "subject": subject,
            "predicate": edge.get("predicate", ""),
            # `target` reads `Entity: <name>`; the prefix is a storage detail.
            "object": re.sub(r"^entity:\s*", "", fold(edge.get("target", ""))),
        }
        for edge in profile.get("relations") or []
    ]
    attributes = [
        {"entity": subject, "key": key, "value": value}
        for key, value in (profile.get("attributes") or {}).items()
    ]
    return {"facts": [], "relations": relations, "attributes": attributes}


# Checks that mean the same thing about a stored entity as about raw JSON. The
# others are about the ANSWER (its language, its verbosity, whether it was cut),
# not about the graph, and applying them here would score the same defect twice.
GRAPH_CHECK_TYPES = frozenset({
    "relation_present", "relation_absent", "attribute_on", "attribute_not_on", "attribute_number",
})


def graph_checks_for(passage: dict) -> "list[dict]":
    """The subset of a passage's checks that a stored entity can answer."""
    return [
        spec for spec in passage["checks"]
        if spec["type"] in GRAPH_CHECK_TYPES and (spec.get("subject") or spec.get("entity"))
    ]


def check_subject(spec: dict) -> str:
    """Which entity a graph check must be run against."""
    return spec.get("subject") or spec["entity"]


# ------------------------------------------------------------- log digest ----


def percentile(values: "list[float]", quantile: float) -> float:
    """Nearest-rank percentile.

    Spelled out because a bench that does not say which definition it uses
    invites two readers to compare incomparable p95s.
    """
    if not values:
        return float("nan")
    ordered = sorted(values)
    rank = max(1, min(len(ordered), math.ceil(quantile * len(ordered))))
    return ordered[rank - 1]


def _digest_line(line: str, tools: dict, https: list, verdicts: dict,
                 since: "str | None") -> None:
    if since:
        stamp = LOG_TIMESTAMP_RE.match(line)
        if not stamp or stamp.group("ts") < since:
            return
    tool_event = TOOL_EVENT_RE.search(line)
    if tool_event:
        tools.setdefault(tool_event.group("tool"), []).append(int(tool_event.group("ms")))
        key = (tool_event.group("tool"), tool_event.group("verdict"))
        verdicts[key] = verdicts.get(key, 0) + 1
        return
    http_event = HTTP_EVENT_RE.search(line)
    if http_event and http_event.group("method") == "POST":
        https.append(int(http_event.group("ms")))


def _summary(values: "list[int]") -> dict:
    return {
        "n": len(values),
        "p50_ms": percentile(values, 0.50),
        "p95_ms": percentile(values, 0.95),
        "max_ms": max(values) if values else None,
    }


def digest_log(path: Path, since: "str | None" = None) -> dict:
    """Turn the daemon's trace events into a per-tool latency map."""
    tools: "dict[str, list[int]]" = {}
    https: "list[int]" = []
    verdicts: "dict[tuple[str, str], int]" = {}
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            _digest_line(line, tools, https, verdicts, since)
    return {
        "source": str(path),
        "since": since,
        "tools": {name: _summary(values) for name, values in tools.items()},
        "http_post": _summary(https),
        "verdicts": {
            f"{tool}:{verdict}": count for (tool, verdict), count in sorted(verdicts.items())
        },
    }


def print_digest(digest: dict) -> None:
    rows = sorted(digest["tools"].items(), key=lambda kv: -(kv[1]["p95_ms"] or 0))
    print(f"{'tool':30} {'n':>5} {'p50':>10} {'p95':>10} {'max':>10}   (ms)")
    for name, stats in rows:
        print(f"{name:30} {stats['n']:>5} {stats['p50_ms']:>10.0f} "
              f"{stats['p95_ms']:>10.0f} {stats['max_ms']:>10}")
    http = digest["http_post"]
    print(f"\n{'MCP HTTP POST':30} {http['n']:>5} {http['p50_ms']:>10.0f} "
          f"{http['p95_ms']:>10.0f} {http['max_ms']:>10}")
    bad = {key: n for key, n in digest["verdicts"].items() if not key.endswith(":ok")}
    print(f"\nnon-ok verdicts: {bad or 'none'}")


# ---------------------------------------------------------------- backends ----


def insecure_context(url: str) -> "ssl.SSLContext | None":
    """Skip verification for the daemon's own loopback HTTPS.

    velesdb-memory terminates HTTPS with a certificate it generates locally, so
    there is no chain to verify and nothing gained by pretending otherwise.
    Restricted to loopback `https://`; anything else gets the default context and
    fails honestly.
    """
    if not url.startswith(("https://127.0.0.1", "https://localhost")):
        return None
    return ssl._create_unverified_context()  # noqa: S323 - loopback, self-signed


def http_json(url: str, payload: "dict | None" = None, token: "str | None" = None,
              timeout: int = 400) -> dict:
    """One JSON request. Loopback only; see `insecure_context` for the TLS note."""
    data = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(url, data=data, method="POST" if data is not None else "GET")
    request.add_header("Content-Type", "application/json")
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=timeout, context=insecure_context(url)) as response:
        body = response.read().decode("utf-8", errors="replace")
    return json.loads(body) if body.strip() else {}


def extractor_token() -> str:
    """Read the extractor token from the installed LaunchAgent, as the daemon does."""
    plist = Path.home() / "Library" / "LaunchAgents" / "com.velesdb.memory.plist"
    output = subprocess.run(["plutil", "-p", str(plist)],
                            capture_output=True, text=True, check=True).stdout
    match = re.search(r'EXTRACTOR_API_TOKEN"\s*=>\s*"([^"]*)"', output)
    if not match:
        raise RuntimeError(f"no EXTRACTOR_API_TOKEN in {plist}")
    return match.group(1)


class OpenAiBackend:
    """The `openai` extractor path: an OpenAI-compatible server such as omlx.

    The body is what `openai::chat_body` builds — same temperature, same cap.
    Anything else measures a request velesdb-memory never sends.
    """

    def __init__(self, base_url: str, model: str, token: str, cap: int) -> None:
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.token = token
        self.cap = cap

    def generate(self, prompt: str) -> dict:
        body = {
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0,
            "max_tokens": self.cap,
        }
        started = time.monotonic()
        response = http_json(f"{self.base_url}/v1/chat/completions", body, self.token)
        elapsed = time.monotonic() - started
        choice = (response.get("choices") or [{}])[0]
        return {
            "seconds": elapsed,
            "content": (choice.get("message") or {}).get("content", ""),
            "completion_tokens": (response.get("usage") or {}).get("completion_tokens"),
            # `length` is the server saying it stopped AT the cap: the truncation
            # this bench exists to catch, reported by the server rather than
            # inferred from a JSON parse error.
            "truncated": choice.get("finish_reason") == "length",
        }

    def residency(self) -> dict:
        return http_json(f"{self.base_url}/v1/models/status", token=self.token, timeout=30)

    def set_loaded(self, model: str, loaded: bool) -> None:
        action = "load" if loaded else "unload"
        http_json(f"{self.base_url}/v1/models/{model}/{action}",
                  payload={}, token=self.token, timeout=900)


class OllamaBackend:
    """The `ollama` extractor path, body-for-body with `OllamaExtractor`."""

    def __init__(self, base_url: str, model: str, cap: int, keep_alive: object = -1) -> None:
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.cap = cap
        self.keep_alive = keep_alive

    def generate(self, prompt: str) -> dict:
        body = {
            "model": self.model,
            "prompt": prompt,
            "stream": False,
            "think": False,
            "keep_alive": self.keep_alive,
            "options": {"temperature": 0, "num_predict": self.cap},
        }
        started = time.monotonic()
        response = http_json(f"{self.base_url}/api/generate", body)
        elapsed = time.monotonic() - started
        return {
            "seconds": elapsed,
            "content": response.get("response", ""),
            "completion_tokens": response.get("eval_count"),
            "truncated": response.get("done_reason") == "length",
        }

    def residency(self) -> dict:
        # `/api/ps` is what is LOADED. `/api/tags` is what is INSTALLED, and
        # reading it instead is how a cold model gets mistaken for a warm one.
        return http_json(f"{self.base_url}/api/ps", timeout=30)

    def set_loaded(self, model: str, loaded: bool) -> None:
        http_json(f"{self.base_url}/api/generate", {
            "model": model, "prompt": "ok",
            "keep_alive": -1 if loaded else 0,
            "options": {"num_predict": 1},
        }, timeout=900)


# ------------------------------------------------- step 0: isolate and warm --


WARMUP_TOLERANCE = 0.10
WARMUP_WINDOW = 3
WARMUP_MAX_ROUNDS = 6


def is_stable(latencies: "list[float]", window: int = WARMUP_WINDOW,
              tolerance: float = WARMUP_TOLERANCE) -> bool:
    """Have the last `window` runs settled within `tolerance` of their median?

    A stabilisation criterion rather than a fixed number of warm-up calls: how
    many rounds a model needs is exactly what is unknown, and guessing either
    wastes minutes or scores a model that had not settled.
    """
    if len(latencies) < window:
        return False
    recent = latencies[-window:]
    middle = sorted(recent)[window // 2]
    if middle <= 0:
        return False
    return all(abs(value - middle) / middle <= tolerance for value in recent)


def warm_up(backend: object, prompt: str) -> dict:
    """Run a real extraction until latency settles, and report the trace.

    A real prompt, never a one-token ping: the ~600-token prefill and the KV
    cache are part of what must be warm, and a ping warms neither. Failing to
    stabilise is a finding about the model, not a bench error, so it comes back
    in the result rather than raising.
    """
    latencies: "list[float]" = []
    for _ in range(WARMUP_MAX_ROUNDS):
        latencies.append(backend.generate(prompt)["seconds"])
        if is_stable(latencies):
            return {"rounds": len(latencies), "latencies": latencies, "stabilised": True}
    return {"rounds": len(latencies), "latencies": latencies, "stabilised": False}


def cold_load(backend: object, model: str, prompt: str) -> dict:
    """Load the model and time the first real answer.

    A metric, not discarded warm-up: this is what a developer pays on the first
    `remember` after a restart, and it decides whether a model is usable at all
    on a smaller machine.
    """
    started = time.monotonic()
    backend.set_loaded(model, True)
    load_seconds = time.monotonic() - started
    first = backend.generate(prompt)
    return {
        "load_seconds": load_seconds,
        "first_answer_seconds": first["seconds"],
        "cold_total_seconds": load_seconds + first["seconds"],
    }


# ------------------------------------------------------ phase A: screening ----


def load_cases(path: Path = CASES_FILE) -> "list[dict]":
    return json.loads(path.read_text(encoding="utf-8"))["cases"]


def parse_payload(content: str) -> "dict | None":
    try:
        parsed = json.loads(strip_code_fence(content))
    except (json.JSONDecodeError, ValueError):
        return None
    return parsed if isinstance(parsed, dict) else None


def screen_passage(backend: object, template: str, passage: dict) -> dict:
    """One scored call: generate, parse, score, and keep the raw answer.

    The raw answer is kept because it is the only thing that can later feed the
    scorer's own tests: a fixture written by hand proves the scorer reacts, a
    captured one proves it reacts to what models actually produce.
    """
    prompt = build_graph_prompt(passage["text"], template)
    result = backend.generate(prompt)
    payload = parse_payload(result["content"])
    return {
        "text": passage["text"],
        "seconds": result["seconds"],
        "completion_tokens": result["completion_tokens"],
        "truncated": result["truncated"],
        "parse_ok": payload is not None,
        "raw": result["content"],
        "failures": score_passage(payload, passage["checks"], result["truncated"]),
        "payload": payload,
    }


def screen_case(backend: object, template: str, case: dict) -> dict:
    """Score one case, cross-checks included."""
    passages = [screen_passage(backend, template, passage) for passage in case["passages"]]
    cross = score_cross_checks([p["payload"] for p in passages], case.get("cross_checks") or [])
    failures = [f for p in passages for f in p["failures"]] + cross
    for passage in passages:
        passage.pop("payload", None)
    return {
        "id": case["id"],
        "family": case["family"],
        "lang": case["lang"],
        "passages": passages,
        "cross_failures": cross,
        "counts": tally(failures),
    }


def _print_case_line(scored: dict) -> None:
    counts = scored["counts"]
    verdict = "OK " if not any(counts.values()) else "ERR"
    seconds = sum(p["seconds"] for p in scored["passages"])
    labels = "; ".join(
        f["label"] for p in scored["passages"] for f in p["failures"]
    ) or "; ".join(f["label"] for f in scored["cross_failures"])
    print(f"{verdict} [{scored['id']}] run{scored['run']} {seconds:.1f}s "
          f"fatal={counts['fatal']} major={counts['major']} {labels}")


def _scored_passages(results: "list[dict]") -> "list[dict]":
    return [passage for case in results for passage in case["passages"]]


def totals_of(results: "list[dict]") -> dict:
    passages = _scored_passages(results)
    latencies = [passage["seconds"] for passage in passages]
    parses = [passage["parse_ok"] for passage in passages]
    counts = {severity: sum(case["counts"][severity] for case in results) for severity in SEVERITIES}
    return {
        **counts,
        "parse_rate": (sum(parses) / len(parses)) if parses else 0.0,
        "truncated": sum(passage["truncated"] for passage in passages),
        "p50_seconds": percentile(latencies, 0.50),
        "p95_seconds": percentile(latencies, 0.95),
    }


def totals_by_language(results: "list[dict]") -> dict:
    """Totals per passage language.

    Reported, but NOT comparable across languages on its own: this suite is
    deliberately unbalanced (the edge and close-pair families are French), so a
    raw fr-vs-en total compares 15 cases against 4. Use `mirror_gap` for the
    comparison that means something.
    """
    languages = sorted({case["lang"] for case in results})
    return {
        language: totals_of([case for case in results if case["lang"] == language])
        for language in languages
    }


def totals_by_family(results: "list[dict]") -> dict:
    families = sorted({case["family"] for case in results})
    return {
        family: totals_of([case for case in results if case["family"] == family])
        for family in families
    }


def mirror_gap(results: "list[dict]") -> dict:
    """Compare the two nominal families, which are case-for-case mirrors.

    This is the language verdict. `nominal-fr` and `nominal-en` state the same
    four situations in two languages, so their scores ARE comparable, and a
    model that passes one and fails the other is visible here and nowhere else.
    A good global score hides exactly this: it is what killed prompt v2, which
    over-corrected towards French and looked fine on average.
    """
    french = totals_of([case for case in results if case["family"] == "nominal-fr"])
    english = totals_of([case for case in results if case["family"] == "nominal-en"])
    weighted = {
        language: totals["fatal"] * 10 + totals["major"]
        for language, totals in (("fr", french), ("en", english))
    }
    return {
        "fr": french,
        "en": english,
        "weighted_fr": weighted["fr"],
        "weighted_en": weighted["en"],
        "gap": abs(weighted["fr"] - weighted["en"]),
        "weaker": None if weighted["fr"] == weighted["en"] else
                  ("fr" if weighted["fr"] > weighted["en"] else "en"),
    }


def screen(backend: object, template: str, cases: "list[dict]", runs: int = 1) -> dict:
    """Phase A over every case, `runs` times each, strictly sequentially."""
    results = []
    for index in range(runs):
        for case in cases:
            scored = screen_case(backend, template, case)
            scored["run"] = index + 1
            results.append(scored)
            _print_case_line(scored)
    return {
        "phase": "screen",
        "runs": runs,
        "cases": results,
        "totals": totals_of(results),
        "by_language": totals_by_language(results),
        "by_family": totals_by_family(results),
        "mirror_gap": mirror_gap(results),
    }


# ------------------------------------------------- phase B: the real server ----


class McpClient:
    """A minimal streamable-HTTP MCP client, talking to the daemon as an agent does.

    Phase B exists because phase A proves nothing about storage: it measures a
    model, not velesdb-memory. Here the calls cross the real transport, the real
    handlers and the real single writer, and the assertions are about the graph
    that comes back out.
    """

    def __init__(self, base_url: str) -> None:
        self.url = base_url.rstrip("/") + "/mcp"
        self.session: "str | None" = None
        self._next_id = 0

    def _post(self, payload: dict, timeout: int = 400) -> "dict | None":
        request = urllib.request.Request(
            self.url, data=json.dumps(payload).encode(), method="POST"
        )
        request.add_header("Content-Type", "application/json")
        request.add_header("Accept", "application/json, text/event-stream")
        if self.session:
            request.add_header("mcp-session-id", self.session)
        context = insecure_context(self.url)
        with urllib.request.urlopen(request, timeout=timeout, context=context) as response:
            self.session = self.session or response.headers.get("mcp-session-id")
            body = response.read().decode("utf-8", errors="replace")
        return decode_mcp_body(body)

    def initialize(self) -> dict:
        self._next_id += 1
        result = self._post({
            "jsonrpc": "2.0", "id": self._next_id, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "bench-memory-extraction", "version": "1"},
            },
        })
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return result or {}

    def call(self, tool: str, arguments: dict, timeout: int = 400) -> dict:
        self._next_id += 1
        started = time.monotonic()
        response = self._post({
            "jsonrpc": "2.0", "id": self._next_id, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }, timeout=timeout)
        return {"seconds": time.monotonic() - started, "response": response or {}}

    def tool_names(self) -> "list[str]":
        """What this server actually exposes, asked rather than assumed."""
        self._next_id += 1
        listing = self._post(
            {"jsonrpc": "2.0", "id": self._next_id, "method": "tools/list", "params": {}}
        ) or {}
        return sorted(
            tool.get("name", "") for tool in (listing.get("result") or {}).get("tools", [])
        )


def decode_mcp_body(body: str) -> "dict | None":
    """Read a JSON-RPC reply out of either a plain body or an SSE stream.

    The stream opens with a priming frame — `data: ` with an empty payload, then
    `id:` and `retry:` — before the reply. Taking the FIRST `data:` line reads
    that empty frame and dies on it, so every frame is tried and the first one
    that parses wins.
    """
    text = body.strip()
    if not text:
        return None
    if not text.startswith(("event:", "data:", "id:", "retry:")):
        return json.loads(text)
    for line in text.splitlines():
        if not line.startswith("data:"):
            continue
        payload = line[len("data:"):].strip()
        if not payload:
            continue
        try:
            return json.loads(payload)
        except (json.JSONDecodeError, ValueError):
            continue
    return None


def tool_payload(result: dict) -> "dict | None":
    """Pull a tool's structured content out of its MCP envelope.

    A REFUSED tool call comes back as a valid result carrying `isError`, not as a
    protocol error. Reading only the outer reply would score every refusal as a
    success — the exact misreading the daemon's own trace event was built to
    prevent.
    """
    payload = (result.get("response") or {}).get("result") or {}
    if payload.get("isError"):
        return None
    for item in payload.get("content") or []:
        if item.get("type") == "text":
            try:
                return json.loads(item["text"])
            except (json.JSONDecodeError, ValueError):
                return {"text": item["text"]}
    return payload.get("structuredContent")


def await_edges(client: McpClient, entity: str, timeout_s: float = 180.0) -> dict:
    """Poll `entity` until the autograph worker's edges land, or give up.

    Edges derived from a `remember` are asynchronous by design (#1846), so the
    delay is measured rather than slept away: it is the drain time that decides
    how many writes a session can sustain before the queue starts dropping
    enrichment.
    """
    started = time.monotonic()
    while time.monotonic() - started < timeout_s:
        profile = tool_payload(client.call("entity", {"name": entity})) or {}
        if profile.get("relations") or profile.get("attributes"):
            return {"drained": True, "seconds": time.monotonic() - started, "profile": profile}
        time.sleep(1.0)
    return {"drained": False, "seconds": time.monotonic() - started, "profile": {}}


class DisposableDaemon:
    """A velesdb-memory server on a scratch store, for one configuration.

    A scratch store rather than the installed one, because phase B writes: it
    must never touch the memory an actual session depends on, and a fresh store
    also keeps recall ranking free of an unrelated corpus.
    """

    def __init__(self, binary: Path, port: int, env: "dict[str, str]") -> None:
        self.binary = binary
        self.port = port
        self.env = env
        self.store = Path(tempfile.mkdtemp(prefix="velesdb-bench-"))
        self.process: "subprocess.Popen | None" = None

    @property
    def url(self) -> str:
        return f"https://127.0.0.1:{self.port}"

    def start(self, timeout_s: float = 60.0) -> None:
        (self.store / "velesdb-memory.toml").write_text(
            "[graph]\nautograph = true\n", encoding="utf-8"
        )
        environment = {**os.environ, **self.env, "VELESDB_MEMORY_PATH": str(self.store)}
        self.process = subprocess.Popen(  # noqa: S603 - fixed binary, no shell
            [str(self.binary), "--http", "--http-port", str(self.port)],
            env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self._await_health(timeout_s)

    def _await_health(self, timeout_s: float) -> None:
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            try:
                http_json(f"{self.url}/health", timeout=5)
                return
            except OSError:
                time.sleep(0.5)
        self.stop()
        raise RuntimeError(f"daemon on port {self.port} never became healthy")

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            # Terminate, not kill: shutdown is what drains the autograph queue
            # and joins the worker, and killing it would report drops that only
            # the bench caused.
            self.process.terminate()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()
        shutil.rmtree(self.store, ignore_errors=True)


def remember_passage(client: McpClient, text: str) -> dict:
    """Store one passage and report what the caller waited for."""
    result = client.call("remember", {"fact": text})
    payload = tool_payload(result)
    return {"seconds": result["seconds"], "stored": payload is not None, "payload": payload}


def score_stored_case(client: McpClient, case: dict) -> dict:
    """Check the graph a case's passages actually produced."""
    entities: "dict[str, dict]" = {}
    drains = []
    failures = []
    for passage in case["passages"]:
        for spec in graph_checks_for(passage):
            name = check_subject(spec)
            if name not in entities:
                drain = await_edges(client, name)
                drains.append(drain["seconds"])
                entities[name] = drain["profile"]
            payload = entity_as_payload(name, entities[name])
            failures += score_passage(payload, [spec])
    return {
        "id": case["id"],
        "failures": failures,
        "counts": tally(failures),
        "drain_seconds": max(drains) if drains else 0.0,
        "entities": entities,
    }


def burst(client: McpClient, texts: "list[str]") -> dict:
    """Write a burst, then read what the bounded queue had to drop.

    The queue holds 64 and one worker drains it; a session that writes faster
    than the model generates loses enrichment. Counted and visible by design —
    this measures how close a given model puts a user to that edge.
    """
    latencies = [remember_passage(client, text)["seconds"] for text in texts]
    status = tool_payload(client.call("memory_status", {})) or {}
    return {
        "count": len(texts),
        "p50_seconds": percentile(latencies, 0.50),
        "p95_seconds": percentile(latencies, 0.95),
        "autograph_dropped": (status.get("extraction") or status).get("autograph_dropped"),
    }


def endtoend(daemon: DisposableDaemon, cases: "list[dict]") -> dict:
    """Phase B: store every passage, then assert on the stored graph."""
    client = McpClient(daemon.url)
    client.initialize()
    writes = []
    for case in cases:
        for passage in case["passages"]:
            writes.append(remember_passage(client, passage["text"])["seconds"])
    stored = [score_stored_case(client, case) for case in cases]
    burst_texts = [f"Note de charge {index} sur le projet Ardoise." for index in range(20)]
    return {
        "phase": "endtoend",
        "write_p50_seconds": percentile(writes, 0.50),
        "write_p95_seconds": percentile(writes, 0.95),
        "cases": stored,
        "burst": burst(client, burst_texts),
        "totals": {
            "fatal": sum(case["counts"]["fatal"] for case in stored),
            "major": sum(case["counts"]["major"] for case in stored),
            "max_drain_seconds": max((case["drain_seconds"] for case in stored), default=0.0),
        },
    }


# ------------------------------------------------------------------ report ----


def _report_row(name: str, entry: dict) -> str:
    totals = entry.get("totals", {})
    cold = entry.get("cold", {}).get("cold_total_seconds")
    warm = entry.get("warmup", {})
    return " | ".join([
        f"| `{name}`",
        str(totals.get("fatal", 0)),
        str(totals.get("major", 0)),
        str(totals.get("minor", 0)),
        f"{totals.get('parse_rate', 0) * 100:.0f}%",
        str(totals.get("truncated", 0)),
        f"{totals.get('p50_seconds', float('nan')):.1f}s",
        f"{totals.get('p95_seconds', float('nan')):.1f}s",
        "n/a" if cold is None else f"{cold:.1f}s",
        f"{warm.get('rounds', '?')}{'' if warm.get('stabilised', True) else ' (unstable)'} |",
    ])


def render_report(results: dict) -> str:
    """Render the frozen markdown report from a result file.

    Generated, never typed: the class of error where a published table and the
    measurement behind it drift apart cannot exist if the table is derived.
    """
    lines = [
        f"# velesdb-memory extraction bench — {results.get('campaign', 'undated')}",
        "",
        "Generated by `scripts/bench-memory-extraction.py report`. Do not edit:",
        "re-run the generator instead.",
        "",
        "## Environment",
        "",
    ]
    lines += [f"- **{key}**: {value}" for key, value in sorted(results.get("environment", {}).items())]
    lines += [
        "",
        "## Configurations",
        "",
        "| configuration | fatal | major | minor | parse | cut | p50 | p95 | cold | warm-up |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    configurations = results.get("configurations", {})
    lines += [_report_row(name, entry) for name, entry in configurations.items()]
    lines += _language_section(configurations)
    return "\n".join(lines) + "\n"


def _language_section(configurations: dict) -> "list[str]":
    """The French/English verdict, on the mirrored families only.

    Kept as its own table because the overall score cannot answer it: the suite
    is deliberately unbalanced towards French, and only `nominal-fr` and
    `nominal-en` state the same four situations twice.
    """
    lines = [
        "",
        "## Language symmetry (mirrored families only)",
        "",
        "`nominal-fr` and `nominal-en` state the same four situations in two",
        "languages. A model that passes one and fails the other is disqualified",
        "on that alone: `works at` and `travaille chez` are two graph predicates",
        "for one relation, and the graph fragments accordingly.",
        "",
        "| configuration | fatal fr | major fr | fatal en | major en | gap | weaker |",
        "|---|---|---|---|---|---|---|",
    ]
    for name, entry in configurations.items():
        gap = entry.get("mirror_gap")
        if not gap:
            continue
        lines.append(
            f"| `{name}` | {gap['fr']['fatal']} | {gap['fr']['major']} | "
            f"{gap['en']['fatal']} | {gap['en']['major']} | {gap['gap']} | "
            f"{gap['weaker'] or 'balanced'} |"
        )
    return lines


# --------------------------------------------------------------------- cli ----


def cmd_from_log(args: argparse.Namespace) -> int:
    digest = digest_log(Path(args.log), args.since)
    print_digest(digest)
    if args.out:
        Path(args.out).write_text(json.dumps(digest, indent=2), encoding="utf-8")
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    results = json.loads(Path(args.results).read_text(encoding="utf-8"))
    rendered = render_report(results)
    if args.out:
        Path(args.out).write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 0


def build_backend(args: argparse.Namespace, cap: int) -> object:
    if args.backend == "ollama":
        return OllamaBackend(args.url or "http://localhost:11434", args.config, cap)
    return OpenAiBackend(args.url or "http://127.0.0.1:8019", args.config, extractor_token(), cap)


def cmd_screen(args: argparse.Namespace) -> int:
    cap = read_generation_cap()
    template = read_graph_prompt_template()
    backend = build_backend(args, cap)
    cases = load_cases(Path(args.cases))
    prompt = build_graph_prompt(cases[0]["passages"][0]["text"], template)
    outcome = {
        "config": args.config,
        "generation_cap": cap,
        "residency_before": backend.residency(),
        "cold": cold_load(backend, args.config, prompt),
        "warmup": warm_up(backend, prompt),
    }
    print(f"# {args.config}: cold {outcome['cold']['cold_total_seconds']:.1f}s, "
          f"warm-up {outcome['warmup']['rounds']} rounds, "
          f"stabilised={outcome['warmup']['stabilised']}")
    outcome.update(screen(backend, template, cases, args.runs))
    totals = outcome["totals"]
    gap = outcome["mirror_gap"]
    print(f"## {args.config}: fatal={totals['fatal']} major={totals['major']} "
          f"parse={totals['parse_rate'] * 100:.0f}% p95={totals['p95_seconds']:.1f}s")
    print(f"## language: fr fatal={gap['fr']['fatal']} major={gap['fr']['major']} | "
          f"en fatal={gap['en']['fatal']} major={gap['en']['major']} | "
          f"gap={gap['gap']} weaker={gap['weaker'] or 'balanced'}")
    if args.out:
        Path(args.out).write_text(
            json.dumps(outcome, indent=2, ensure_ascii=False), encoding="utf-8"
        )
    return 1 if totals["fatal"] else 0


def endtoend_env(args: argparse.Namespace) -> "dict[str, str]":
    """The extractor and embedder the disposable daemon runs with.

    The embedder stays bge-m3 through Ollama: it is the product's choice and the
    bench has no business varying it.
    """
    env = {
        "VELESDB_MEMORY_EMBEDDER": "ollama",
        "VELESDB_MEMORY_OLLAMA_MODEL": args.embedder,
        "VELESDB_MEMORY_EXTRACTOR": args.backend,
        "VELESDB_MEMORY_EXTRACTOR_MODEL": args.config or "",
    }
    if args.backend == "openai":
        env["VELESDB_MEMORY_EXTRACTOR_URL"] = args.url or "http://127.0.0.1:8019"
        env["VELESDB_MEMORY_EXTRACTOR_API_TOKEN"] = extractor_token()
    elif args.backend == "ollama":
        env["VELESDB_MEMORY_EXTRACTOR_URL"] = args.url or "http://localhost:11434"
    return env


def cmd_endtoend(args: argparse.Namespace) -> int:
    daemon = DisposableDaemon(Path(args.binary), args.port, endtoend_env(args))
    daemon.start()
    try:
        outcome = endtoend(daemon, load_cases(Path(args.cases)))
    finally:
        daemon.stop()
    outcome["config"] = args.config
    totals = outcome["totals"]
    print(f"## {args.config}: fatal={totals['fatal']} major={totals['major']} "
          f"drain_max={totals['max_drain_seconds']:.1f}s "
          f"dropped={outcome['burst']['autograph_dropped']}")
    if args.out:
        Path(args.out).write_text(
            json.dumps(outcome, indent=2, ensure_ascii=False), encoding="utf-8"
        )
    return 1 if totals["fatal"] else 0


# What phase B calls. Checked against the server rather than assumed: the
# daemon installed on this machine on 2026-08-15 exposes 20 tools and NOT
# `memory_status`, because its binary predates that tool. Phase B run against
# it would have reported "tool not found" as a measurement.
PHASE_B_TOOLS = ("remember", "entity", "recall", "memory_status")


def cmd_probe(args: argparse.Namespace) -> int:
    """Read-only handshake against a running daemon, to prove the client works.

    Reads nothing but the tool list and one millisecond-scale read, so it is
    safe against a daemon another session is using.
    """
    client = McpClient(args.url or "https://127.0.0.1:18090")
    client.initialize()
    names = client.tool_names()
    missing = [tool for tool in PHASE_B_TOOLS if tool not in names]
    contexts = tool_payload(client.call("list_working_contexts", {"project": "velesdb"}, timeout=60))
    print(f"tools: {len(names)}")
    print(f"read-only call ok: {contexts is not None}")
    print(f"missing for phase B: {missing or 'none'}")
    return 1 if missing or contexts is None else 0


def _add_model_arguments(parser: argparse.ArgumentParser, config_required: bool) -> None:
    parser.add_argument("--config", required=config_required, help="model id")
    parser.add_argument("--backend", choices=["openai", "ollama", "outline"], default="openai")
    parser.add_argument("--url")
    parser.add_argument("--out")
    # velesdb-memory is used in whatever language its user writes in, and the
    # extractor model is THEIR choice (`VELESDB_MEMORY_EXTRACTOR_MODEL`). The
    # shipped suite measures French and English because those are the two this
    # campaign answers for; anyone else points this at their own cases and gets
    # the same verdict for their own language.
    parser.add_argument("--cases", default=str(CASES_FILE),
                        help="scenario file; replace it to bench another language")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    from_log = subparsers.add_parser("from-log", help="digest the daemon's tool-call events")
    from_log.add_argument("--log", default=str(DEFAULT_LOG))
    from_log.add_argument("--since", help="ISO timestamp prefix, e.g. 2026-08-15T00")
    from_log.add_argument("--out")
    from_log.set_defaults(func=cmd_from_log)

    screen_parser = subparsers.add_parser("screen", help="phase A: score the model's JSON")
    _add_model_arguments(screen_parser, config_required=True)
    screen_parser.add_argument("--runs", type=int, default=1)
    screen_parser.set_defaults(func=cmd_screen)

    e2e = subparsers.add_parser("endtoend", help="phase B: drive a disposable MCP server")
    _add_model_arguments(e2e, config_required=False)
    e2e.add_argument("--binary", required=True, help="velesdb-memory binary built from develop")
    e2e.add_argument("--port", type=int, default=18099)
    e2e.add_argument("--embedder", default="bge-m3")
    e2e.set_defaults(func=cmd_endtoend)

    probe = subparsers.add_parser("probe", help="read-only MCP handshake against a running daemon")
    probe.add_argument("--url")
    probe.set_defaults(func=cmd_probe)

    report = subparsers.add_parser("report", help="render the markdown report from a result file")
    report.add_argument("--results", required=True)
    report.add_argument("--out")
    report.set_defaults(func=cmd_report)
    return parser


def main(argv: "list[str] | None" = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
