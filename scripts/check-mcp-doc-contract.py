#!/usr/bin/env python3
"""MCP return-contract guard: every surface that DESCRIBES what an MCP tool
returns must announce the same root keys as the published ``outputSchema``.

Source of truth is ``docs/reference/mcp-tools.json`` — the raw JSON-RPC
``tools/list`` capture of what the ``velesdb-memory`` server actually
publishes, itself kept honest by ``crates/velesdb-memory/tests/mcp_tools_drift.rs``.
Nothing here re-derives the contract from the Rust source: it reads the same
artifact a client reads.

This is the mechanism of ``crates/velesdb-memory/tests/binding_parity_bdd.rs``
(root keys of ``output_schema`` vs what each binding relays) applied one level
out, to prose: guides, reference pages, skills, integration docstrings and SDK
comments. The failure it exists to catch is the one #1694 had to fix by hand —
``load_working_context`` grew a ``{found, working, other_sessions}`` envelope
while a dozen surfaces still told the model it got the bare working context
back, so an agent read a resumable session as a fresh start.

**Written limit, same class as binding_parity_bdd.rs's.** This is a text
search. It proves what a surface *declares*, never what any code *returns*.
A page can declare the right keys and the implementation still be wrong; the
runtime side is `mcp_tools_drift.rs` + `binding_parity_bdd.rs`, not this.
It also only sees *literal* shape declarations (``Returns `{a, b, c}` ``) —
a TypeScript ``interface`` or a Rust ``struct`` is a type declaration, left to
the binding-parity gate.

**Second written limit: the registry is FIFTEEN tools of the twenty-one
published, and the six still held out are ``compile_context``, ``entity``,
``recall``, ``remember``, ``remember_extracted`` and ``why`` — each is waiting
on the nested-shape treatment (#1695, lots 2-3) before its literals can be
read without false drift.** The structure-aware attribution described below
is what made widening possible at all — under proximity alone, 13 of
``docs/reference/MCP_TOOLS.md``'s 19 declarations were charged to a
neighbouring section, which is the "68 non-conformances" this header used to
record. It is not enough for the rest: each tool still out carries literals
this guard would report as drift that are really a sibling's shape, an input
schema, an MCP client config file or another API's dict. ``entity`` alone has
twelve, six of them LangChain dictionaries in files saturated with the word.
They go in by batches, and #1695 says how: each batch verified by a mutation
that must make the guard refuse. Say "this guard polices fourteen of the
twenty published tools", never "every MCP tool".

**Third: what the sweep reads is a measured list, not every file.** Adding
``crates/velesdb-memory/src/**/*.rs``, ``crates/velesdb-wasm/src`` or the
Python ``.pyi`` was tried and reintroduced false positives (a doc comment
about ``list_working_contexts`` that names ``load_working_context`` closer to
the literal than its own tool; ``use`` blocks; test fixtures). The Rust
sources that ARE swept are listed one by one in ``SURFACE_GLOBS`` with the
reason.

What counts as a declaration, deliberately narrow (a wider rule was measured
first and produced ~110 false positives on this tree — input schemas, sibling
tools, unrelated JSON):

  1. a brace literal whose top-level tokens are ALL identifier-shaped — the
     syntactic shape of an object's root keys. This is what separates
     `{found, working, other_sessions}` from the Rust function body
     `{let svc = Arc…}` that a `-> T {` signature would otherwise anchor,
  2. introduced *as a return*: a return anchor (`returns`, `resolves`, `->`,
     `ts_return_type`, or a shape noun `envelope`/`shape`/`contract`) within
     140 characters before it, or a shape noun within 40 characters after it
     — the tree uses both phrasings ("returns `{a, b}`" and "the `{a, b}`
     envelope"). No other brace may sit between the anchor and the literal,
  3. ATTRIBUTED by ``owning_tool`` to the tool most SPECIFICALLY named for
     it: by the line the literal sits on, failing that by the section heading
     above it, failing that by the nearest alias within 500 characters —
     among the aliases of EVERY tool the capture publishes.

Root keys are compared with their CASE folded, because ``datedContext`` and
``dated_context`` are one key rendered for two bindings and the rendered
spelling is ``binding_parity_bdd.rs``'s question, not this one's. Only the
case: a renamed key is still a different key, and still fails.

Then the rule: for a policed tool, the declared key set must EQUAL the
published root key set.

**Attribution is nearest-alias over all published tools, and that is
load-bearing twice.** An earlier rule attributed by mere proximity to the
policed tool's alias and then required the declared keys to INTERSECT the
published ones. Both halves were wrong, in opposite directions:

  * the intersection was a RECOGNITION filter, so renaming *every* key made
    a declaration cease to exist instead of failing. A surface carrying a
    second, correct declaration also stayed "covered", so nothing fired: a
    page could lie from end to end and this guard stayed green — the very
    defect it exists to catch;
  * proximity alone stole a SIBLING's correct sentence. `unrelate` publishes
    `{found, removed}` and `forget` `{found, id, id_str}`; documented within
    500 characters of `loadWorkingContext` — which is where they are
    documented — each was read as `load_working_context` drifting, turning a
    required check red on a correct tree.

Nearest-alias attribution fixes both: a total rename is still the policed
tool's declaration (and fails), and a sibling's sentence belongs to the
sibling. The price is that attribution is only as good as the prose: a
sentence saying "It returns …" 100 characters after a *different* tool's
name is attributed to that other tool. Measured on this tree the residue is
zero, but a surface must name the tool it is describing near the literal —
which is also what makes the sentence readable to the model.

A literal whose only key is `error` is the documented error payload
(`integrations/langgraph` returns one instead of raising), not a return
shape, and is skipped. That exemption is exactly one key wide on purpose: it
cannot hide a rename the way the intersection filter did.

Anti-disarm, the failure mode ``scripts/check-doc-contract.sh`` was written
against (an extraction that breaks and then passes vacuously): an empty tool
registry, an empty file sweep, a tool absent from the capture, a tool with no
pinned surfaces, a pinned surface the sweep cannot even reach, or a pinned
surface that stopped declaring anything — every one of those FAILS. A guard
that finds nothing is broken, never green.

Adding a tool: append a ``PolicedTool`` below, run with ``--verbose`` to see
every declaration the sweep attributes to it, and pin the surfaces it found.

Exit codes: 0 = passed (or ran in ``--mode warn``), 1 = failed in strict mode,
2 = usage/IO error.
"""

from __future__ import annotations

import argparse
import bisect
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# The published capture. Not the Rust source: this is the artifact a client
# sees, and mcp_tools_drift.rs is what keeps it equal to the live server.
TOOLS_CAPTURE = "docs/reference/mcp-tools.json"


class PolicedTool:
    """One MCP tool whose return contract the documentation must restate."""

    def __init__(
        self,
        name: str,
        aliases: "tuple[str, ...]",
        pinned_surfaces: "tuple[str, ...]",
    ) -> None:
        self.name = name
        # Names a surface may use for the same tool: the MCP tool id, the
        # camelCase binding method, the SDK type. A declaration must sit near
        # one of them, otherwise it is somebody else's shape.
        self.aliases = (name,) + aliases
        # Surfaces that MUST carry a declaration. This is the half of the
        # guard that survives deletion: the sweep alone would go green the
        # moment a page stopped describing the contract at all.
        self.pinned_surfaces = pinned_surfaces


POLICED_TOOLS: "tuple[PolicedTool, ...]" = (
    # --- #1695 batch 1: the low-noise five --------------------------------
    PolicedTool(
        "feedback",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    # `recall` joins batch 2: its two remaining literals need the
    # nested-shape treatment (WASM item fields, the SDK's dated envelope),
    # not a shortcut at the registry level.
    PolicedTool(
        "recall_where",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "relate",
        ("RelateResult",),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "unrelate",
        ("UnrelateOutcome",),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "compile_transcript",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/reference/MCP_TOOLS.md",
            "sdks/typescript/src/memory.ts",
        ),
    ),
    PolicedTool(
        "context_savings",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "explain_compilation",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/reference/MCP_TOOLS.md",
        ),
    ),
    PolicedTool(
        "forget",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "memory_status",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "list_working_contexts",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/guides/NODE_ADDON.md",
            "docs/reference/MCP_TOOLS.md",
        ),
    ),
    PolicedTool(
        "load_working_context",
        ("loadWorkingContext", "LoadedWorkingContext"),
        (
            "crates/velesdb-node/README.md",
            # The skill that OWNS the working-context tools, and its bundled
            # npm copy. The pin moved here from the two
            # `velesdb-context-optimizer` copies when resumption moved with
            # it: the compression skill now points at this one instead of
            # restating the envelope, so it no longer declares a shape and
            # cannot be pinned for one. Coverage did not shrink — the
            # declaration and its pin travelled together, which is the only
            # way a moved contract stays policed.
            "crates/velesdb-memory/skill/velesdb-memory/SKILL.md",
            "crates/velesdb-node/skills/velesdb-memory/SKILL.md",
            # The `ts_return_type` string is the .d.ts every npm consumer
            # compiles against. binding_parity_bdd.rs reads this region but
            # treats that string as EVIDENCE of a relay, never checks its
            # keys — its own header says so. Nothing else policed it.
            "crates/velesdb-node/src/lib.rs",
            "docs/guides/NODE_ADDON.md",
            "docs/guides/PYTHON_CONTEXT_COMPILER.md",
            "docs/guides/WASM_API.md",
            "docs/reference/ECOSYSTEM_PARITY.md",
            "docs/reference/MCP_TOOLS.md",
            # The three agent hooks. Injected into the context of EVERY
            # Claude Code / Codex / Windsurf session, so they are the surface
            # a model reads most often, and they were outside the sweep.
            "integrations/agent-hooks/claude-code/hooks/session-start.sh",
            "integrations/agent-hooks/codex/README.md",
            "integrations/agent-hooks/codex/hooks/session-start.sh",
            "integrations/agent-hooks/windsurf/hooks/pre-user-prompt.sh",
            "integrations/langgraph/README.md",
            "integrations/langgraph/src/langgraph_velesdb/tools.py",
            "sdks/typescript/src/memory.ts",
        ),
    ),
    PolicedTool(
        "recall_fused",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/reference/MCP_TOOLS.md",
            "integrations/langgraph/README.md",
        ),
    ),
    PolicedTool(
        "retrieve_context_source",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/guides/CONTEXT_COMPILER.md",
            "docs/guides/NODE_ADDON.md",
            "docs/guides/PYTHON_CONTEXT_COMPILER.md",
            "docs/reference/MCP_TOOLS.md",
        ),
    ),
    PolicedTool(
        "save_working_context",
        (),
        ("docs/reference/MCP_TOOLS.md",),
    ),
    PolicedTool(
        "suggest_budget",
        (),
        (
            "crates/velesdb-node/src/lib.rs",
            "docs/reference/MCP_TOOLS.md",
        ),
    ),
)

# Where a return contract can be described. Widened from the doc-contract
# workflow's original blind spots: sdks/, integrations/ and skills/ describe
# the same contract to the same model and were policed by nothing.
#
# The two non-prose entries are named FILE BY FILE, not by directory, and the
# module header says why: sweeping `crates/**/src/**/*.rs` was measured and
# put back false positives. These two carry a real declaration and produce
# none.
SURFACE_GLOBS: "tuple[str, ...]" = (
    "README.md",
    "docs/**/*.md",
    "skills/**/*.md",
    "crates/velesdb-memory/README.md",
    # The velesdb-memory skill's SOURCE. Its bundled npm copy under
    # `crates/velesdb-node/skills/` was already swept, which had the polarity
    # backwards: the copy is generated from this file by
    # `sync-skills.py --bundle`, so policing only the artefact means the thing
    # a contributor edits is the one nothing reads.
    "crates/velesdb-memory/skill/**/*.md",
    "crates/velesdb-node/README.md",
    "crates/velesdb-node/skills/**/*.md",
    # The shipped TypeScript return type of the napi addon.
    "crates/velesdb-node/src/lib.rs",
    "integrations/**/*.md",
    "integrations/**/*.py",
    # The session-start hooks: prompt text handed to a model, every session.
    "integrations/agent-hooks/*/hooks/*.sh",
    "sdks/typescript/src/**/*.ts",
)

# Historical records legitimately quote the OLD shape; test files assert it
# directly rather than describing it.
SURFACE_EXCLUDED_PARTS: "tuple[str, ...]" = (
    "/archive/",
    "/tests/",
    "/node_modules/",
    "/target/",
    "/__pycache__/",
)
SURFACE_EXCLUDED_NAMES = re.compile(r"^(CHANGELOG|MIGRATION_)", re.IGNORECASE)

# Surfaces that document ANOTHER API sharing the MCP tools' verbs — the core
# `AgentMemory` Rust API, the SDK's core-collection backends, the VelesQL
# grammar. Their `recall`/`relate`/`feedback` legitimately return OTHER
# shapes, and attributing those literals to the MCP tools is the
# false-positive class the widening of #1695 predicted. Declared here WITH a
# reason, never silently skipped — and each entry must keep matching a swept
# file, or the guard reports it stale (the SHAPE_DIVERGENCES rule).
NON_MCP_SURFACES: "dict[str, str]" = {
    "docs/guides/AGENT_MEMORY.md": (
        "documents velesdb-core's AgentMemory Rust API — same verbs "
        "(recall, relate), different shapes by design"
    ),
    "docs/VELESQL_SPEC.md": (
        "the VelesQL grammar; its `feedback`/`{name}` literals belong to the "
        "query language, not to MCP tools"
    ),
    "sdks/typescript/src/backends/": (
        "SDK backends wrap velesdb-core collections (graph edges, agent "
        "memory records) — their relate/unrelate/recall shapes are the "
        "core's, not the MCP tools'"
    ),
    "integrations/common/src/velesdb_common/memory.py": (
        "the Python integrations' shared core-API helper — its recall "
        "returns core recollections, not the MCP envelope"
    ),
}


def non_mcp_reason(surface: str) -> "str | None":
    """The registered reason ``surface`` is out of MCP scope, if any."""
    for prefix, reason in NON_MCP_SURFACES.items():
        if surface == prefix or surface.startswith(prefix):
            return reason
    return None

# A brace literal counts as a return declaration only when one of these
# anchors it. Measured against this tree: dropping the anchor re-admits input
# schemas and unrelated JSON examples as "declarations" (~110 of them).
#
# The word boundaries are load-bearing. Without the trailing `\b`, `resolves?`
# also matches inside `resolved`, and "the build predates the {found, ...}"
# was being read as a return declaration — right answer, wrong reason. That
# literal is now anchored by the `envelope` that follows it, which is what
# actually makes it one.
RETURN_VERB_RE = re.compile(
    r"(\b(?:returns?|resolves?|answers?\s+with|renvoie"
    r"|envelope|shape|contract)\b|->|=>|ts_return_type)[^{}]{0,140}$",
    re.IGNORECASE,
)
SHAPE_NOUN_RE = re.compile(r"^[^{}]{0,40}?\b(envelope|shape|contract)\b", re.IGNORECASE)
VERB_LOOKBACK = 140
NOUN_LOOKAHEAD = 60
ANCHOR_WINDOW = 500
MAX_LITERAL = 3000

# A root key is an identifier. Requiring EVERY top-level token to look like
# one is what keeps `-> AsyncTask<…> {` from turning a Rust function body
# into a "declaration" — the `->` anchor alone cannot tell them apart. This
# is a SYNTACTIC test, independent of the expected keys: unlike the
# intersection filter it replaced, a rename is still identifiers, so a rename
# is still seen and still fails.
IDENTIFIER_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

# …and it is not enough on its own: `AsyncTask` IS identifier-shaped, so
# `-> AsyncTask<…> {` survived the test above and was read as a declaration of
# `{AsyncTask}` (measured at crates/velesdb-node/src/lib.rs:646). A literal
# whose tokens are ALL PascalCase names a TYPE, never a shape: every root key
# the capture publishes is snake_case, and every binding renders them
# camelCase, so neither form can be lost to this rule.
TYPE_NAME_RE = re.compile(r"^[A-Z][A-Za-z0-9_]*$")

# `datedContext` and `dated_context` are the SAME root key, rendered for two
# bindings: napi and wasm publish camelCase because that is their contract.
# Comparing the rendered spelling made this guard call a correct binding a
# liar (measured at crates/velesdb-node/src/lib.rs:229, docs/guides/WASM_API.md:271).
# Only the CASING is folded — a key that was renamed is still a different key,
# so a rename still fails. Which spelling each binding must actually ship is
# binding_parity_bdd.rs's question, not this one's.
CAMEL_BOUNDARY_RE = re.compile(r"(?<!^)(?=[A-Z])")

# `{"error": "…"}` is the documented error payload integrations/langgraph
# returns instead of raising. Exactly one key wide.
ERROR_PAYLOAD_KEYS = frozenset({"error"})

# `{"tool": …, "arguments": …}` is the JSON-RPC CALL envelope — an input, the
# mirror of the payload above, and no published tool has those root keys. Every
# skill page shows one, and prose about what a tool `returns` routinely sits a
# few dozen characters above the example of how to call it, which is enough for
# the return anchor to claim it (measured at
# skills/velesdb-context-optimizer/SKILL.md:317, both copies). Exactly two keys
# wide, for the same reason the one above is exactly one: an exemption that can
# grow can hide a rename.
CALL_ENVELOPE_KEYS = frozenset({"tool", "arguments"})

# `{@link Foo}` is a JSDoc cross-reference, not a shape. Masked (length
# preserved, so every offset below stays valid against the raw text) before
# anything else looks at braces.
JSDOC_LINK_RE = re.compile(r"\{@link[^}]*\}")
BRACE_RE = re.compile(r"\{")

_BRACE_DEPTH = {"{": 1, "}": -1}
_NESTING_DEPTH = {"{": 1, "}": -1, "[": 1, "]": -1, "(": 1, ")": -1}

# Decoration a key may wear in prose or in a comment: backticks, quotes, the
# `*` of a JSDoc continuation line, an optional marker, a `+` left by a
# JavaScript string concatenation, an ellipsis.
TOKEN_TRIM = " \t\r\n`'\"*?+[]|#…"
COMMENT_PREFIX_RE = re.compile(r"^\s*[*/#>]+")


# --------------------------------------------------------------------------
# The published capture
# --------------------------------------------------------------------------


def load_output_schema_keys(root: Path) -> "dict[str, list[str]]":
    """Root keys of every tool's ``outputSchema``, from the capture."""
    payload = json.loads((root / TOOLS_CAPTURE).read_text(encoding="utf-8"))
    tools = payload.get("tools")
    if not tools:
        raise RuntimeError(f"{TOOLS_CAPTURE} declares no tool at all")
    return {
        tool["name"]: sorted((tool.get("outputSchema") or {}).get("properties", {}))
        for tool in tools
    }


# --------------------------------------------------------------------------
# Surfaces
# --------------------------------------------------------------------------


def _is_scanned(root: Path, path: Path) -> bool:
    posix = "/" + path.relative_to(root).as_posix()
    if any(part in posix for part in SURFACE_EXCLUDED_PARTS):
        return False
    return not SURFACE_EXCLUDED_NAMES.match(path.name)


def _tracked_files(root: Path) -> "set[Path] | None":
    """The set of git-tracked files under `root`, or `None` outside a repo.

    The sweep's perimeter is what git TRACKS, not what the working tree
    happens to contain (#1730): the same commit used to sweep 218 files from
    the main repository and 214 from a clean worktree, the four extras being
    untracked artifacts (`integrations/**/.pytest_cache/README.md`). An
    exclusion list enumerates incidents; the tracked set states the rule.

    `None` (not a git repository, or no git binary) falls back to the raw
    globs — the sandboxed unit tests of this guard run in plain temp dirs,
    and a tarball consumer still gets the historical behavior. Inside a
    repository the filter always applies, which is the case CI and every
    developer checkout are in.
    """
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return {
        root / name
        for name in out.stdout.decode("utf-8", "surrogateescape").split("\0")
        if name
    }


def surface_files(root: Path) -> "list[Path]":
    """Every file the sweep reads, deduplicated and sorted."""
    tracked = _tracked_files(root)
    matched: "set[Path]" = set()
    for glob in SURFACE_GLOBS:
        matched.update(
            p
            for p in root.glob(glob)
            if p.is_file()
            and _is_scanned(root, p)
            and (tracked is None or p in tracked)
        )
    return sorted(matched)


# --------------------------------------------------------------------------
# Extraction
# --------------------------------------------------------------------------


def mask_jsdoc_links(text: str) -> str:
    return JSDOC_LINK_RE.sub(lambda m: " " * len(m.group(0)), text)


def brace_span(text: str, start: int) -> "int | None":
    """End offset (exclusive) of the balanced brace literal opening at ``start``."""
    depth = 0
    for index in range(start, min(len(text), start + MAX_LITERAL)):
        depth += _BRACE_DEPTH.get(text[index], 0)
        if depth == 0:
            return index + 1
    return None


def _split_top_level(body: str) -> "list[str]":
    """Split on separators that are not inside a nested brace/bracket/paren.

    `;` as well as `,`: a TypeScript type literal — which is what
    `#[napi(ts_return_type = "Promise<{ found: boolean; … }>")]` ships to
    every npm consumer — separates its members with semicolons.
    """
    out: "list[str]" = []
    depth = 0
    current: "list[str]" = []
    for char in body:
        depth += _NESTING_DEPTH.get(char, 0)
        if char in (",", ";") and depth == 0:
            out.append("".join(current))
            current = []
            continue
        current.append(char)
    out.append("".join(current))
    return out


def _normalize_token(token: str) -> str:
    token = token.split(":", 1)[0].replace("\n", " ")
    return COMMENT_PREFIX_RE.sub("", token).strip(TOKEN_TRIM)


def declared_keys(body: str) -> "list[str]":
    """Top-level keys of a brace literal body (braces excluded)."""
    keys: "list[str]" = []
    for token in _split_top_level(body):
        key = _normalize_token(token)
        if key:
            keys.append(key)
    return keys


# --------------------------------------------------------------------------
# Attribution: which tool is this literal about?
# --------------------------------------------------------------------------


def camel_case(name: str) -> str:
    """``load_working_context`` -> ``loadWorkingContext``, the binding name."""
    head, *rest = name.split("_")
    return head + "".join(word.capitalize() for word in rest)


def snake_case(key: str) -> str:
    """``datedContext`` -> ``dated_context``: one root key, two renderings."""
    return CAMEL_BOUNDARY_RE.sub("_", key).lower()


def build_alias_index(
    tool_names: "list[str]",
    policed: "tuple[PolicedTool, ...]" = (),
) -> "dict[str, str]":
    """``alias -> tool name`` for EVERY tool the capture publishes.

    Attribution needs the whole set, not just the policed tools: knowing that
    ``unrelate`` is a real tool is what stops its correct sentence from being
    charged to ``load_working_context``.
    """
    index: "dict[str, str]" = {}
    for name in tool_names:
        index[name] = name
        index.setdefault(camel_case(name), name)
    for tool in policed:
        for alias in tool.aliases:
            index[alias] = tool.name
    return index


def alias_positions(text: str, index: "dict[str, str]") -> "list[tuple[int, str]]":
    """Every alias occurrence in ``text``, as sorted ``(offset, tool)`` pairs.

    Word-bounded: without it ``recall`` matches inside ``recall_fused`` and
    would win every attribution contest at distance zero.
    """
    found: "list[tuple[int, str]]" = []
    for alias, tool in index.items():
        pattern = rf"(?<![A-Za-z0-9_]){re.escape(alias)}(?![A-Za-z0-9_])"
        found.extend((m.start(), tool) for m in re.finditer(pattern, text))
    found.sort()
    return found


# A page that documents twenty tools in a row is the case proximity cannot
# handle: every section sits within ANCHOR_WINDOW of its neighbours, so a
# sentence lands on whichever alias happens to be nearer rather than on the
# tool the section is about. Measured on docs/reference/MCP_TOOLS.md: 13 of its
# 19 declarations were charged to a sibling — `recall`'s `{memories}` to
# `feedback`, `suggest_budget`'s `{window, suggested_budget, source}` to
# `save_working_context`.
#
# A heading is the document's own statement of what it is describing, so it
# answers wherever the literal's own line does not. It does NOT replace
# proximity either: guides, READMEs and source comments have no per-tool
# sections, and a section-only rule was measured to lose 17 attributions that
# are correct today. See ``owning_tool`` for the three tiers and their order.
#
# Only `#` and `##` delimit a tool section — a `###` inside one is a
# subsection of it, not a new owner — and a heading that names no tool closes
# the previous one instead of extending it, which is what stops a trailing
# "Error model" section from inheriting the last tool documented above it.
SECTION_HEADING_RE = re.compile(r"^#{1,2}[ \t]+(.*)$", re.MULTILINE)
SECTION_TOOL_RE = re.compile(r"^`([A-Za-z_][A-Za-z0-9_]*)`")


def section_positions(
    text: str,
    tool_names: "set[str]",
) -> "list[tuple[int, str | None]]":
    """``(offset, tool)`` per top-level heading; ``None`` when it names none."""
    found: "list[tuple[int, str | None]]" = []
    for match in SECTION_HEADING_RE.finditer(text):
        named = SECTION_TOOL_RE.match(match.group(1).strip())
        tool = named.group(1) if named else None
        found.append((match.start(), tool if tool in tool_names else None))
    return found


def section_tool(
    sections: "list[tuple[int, str | None]]",
    offset: int,
) -> "str | None":
    """The tool named by the section ``offset`` falls in, if it names one."""
    pivot = bisect.bisect_right([position for position, _tool in sections], offset)
    return sections[pivot - 1][1] if pivot else None


def _candidate(
    positions: "list[tuple[int, str]]",
    index: int,
    offset: int,
) -> "tuple[int, str] | None":
    """``(distance, tool)`` if ``index`` is in range and inside the window."""
    if not 0 <= index < len(positions):
        return None
    distance = abs(positions[index][0] - offset)
    return (distance, positions[index][1]) if distance <= ANCHOR_WINDOW else None


def nearest_tool(positions: "list[tuple[int, str]]", offset: int) -> "str | None":
    """The tool whose alias sits closest to ``offset``, within ANCHOR_WINDOW.

    ``positions`` is sorted, so only the two neighbours of the insertion
    point can win; ties break on the tool name, which keeps the verdict
    deterministic rather than dependent on dict ordering.
    """
    pivot = bisect.bisect_left([position for position, _tool in positions], offset)
    candidates = [
        candidate
        for index in (pivot - 1, pivot, pivot + 1)
        if (candidate := _candidate(positions, index, offset)) is not None
    ]
    return min(candidates)[1] if candidates else None


def _closer(
    best: "tuple[int, str] | None",
    candidate: "tuple[int, str]",
) -> "tuple[int, str]":
    return candidate if best is None or candidate < best else best


def naming_tool(
    text: str,
    positions: "list[tuple[int, str]]",
    offset: int,
) -> "str | None":
    """The tool named ON THE SAME LINE as the literal — its subject, if any.

    The line is the whole discriminator, and direction is deliberately NOT.
    Prose puts its subject first (``sibling_tool` resolves `{found,
    removed}``) while a destructuring assignment puts it last (``const {
    sessions } = await store.listWorkingContexts(…)``); either way the subject
    shares the literal's line. What does NOT share it is the cross-reference,
    which is where reading one line further was measured to go wrong four
    times in ``docs/reference/MCP_TOOLS.md`` alone — "Returns `{id, id_str}` …
    relay `id_str` if you intend to `forget` it" names `forget` on the next
    line and means nothing of the sort.
    """
    offsets = [position for position, _tool in positions]
    pivot = bisect.bisect_left(offsets, offset)
    best: "tuple[int, str] | None" = None
    for index in range(pivot - 1, -1, -1):
        position, tool = positions[index]
        if "\n" in text[position:offset]:
            break
        best = _closer(best, (offset - position, tool))
    for index in range(pivot, len(positions)):
        position, tool = positions[index]
        if "\n" in text[offset:position]:
            break
        best = _closer(best, (position - offset, tool))
    return best[1] if best else None


def owning_tool(
    text: str,
    sections: "list[tuple[int, str | None]]",
    positions: "list[tuple[int, str]]",
    offset: int,
) -> "str | None":
    """The tool a literal is ABOUT: the one most SPECIFICALLY named for it.

    One question, read off three structures in order of how narrowly each
    speaks for this literal, stopping at the first that answers:

      1. the STATEMENT it sits in — a sentence that names a tool right beside
         the literal is describing that tool, even in the middle of another
         tool's section. This is what keeps `unrelate`'s correct
         ``{found, removed}``, documented under a neighbour's heading, from
         being read as the neighbour drifting;
      2. the SECTION it sits in — a heading is the document's own statement of
         what the part below it describes, and it is the only signal available
         on a page that documents twenty tools in a row;
      3. the PAGE around it — the nearest alias within ANCHOR_WINDOW, which is
         all there is in a guide, a README or a source comment, none of which
         carry per-tool headings.

    Each tier is narrower than the next, so a more precise statement always
    overrides a vaguer one; that ordering is the whole rule.
    """
    return (
        naming_tool(text, positions, offset)
        or section_tool(sections, offset)
        or nearest_tool(positions, offset)
    )


# --------------------------------------------------------------------------
# Extraction
# --------------------------------------------------------------------------


def _is_return_declaration(text: str, start: int, end: int) -> bool:
    """A return verb just before the literal, or a shape noun just after it."""
    if RETURN_VERB_RE.search(text[max(0, start - VERB_LOOKBACK):start]):
        return True
    return bool(SHAPE_NOUN_RE.match(text[end:end + NOUN_LOOKAHEAD]))


def _is_shape_literal(keys: "list[str]") -> bool:
    """Non-empty, all identifier-shaped, not a type name, not a known envelope."""
    if not keys or set(keys) in (ERROR_PAYLOAD_KEYS, CALL_ENVELOPE_KEYS):
        return False
    if all(TYPE_NAME_RE.match(key) for key in keys):
        return False
    return all(IDENTIFIER_RE.match(key) for key in keys)


def _declaration_at(text: str, start: int) -> "list[str] | None":
    """The declared keys of the literal opening at ``start``, if it is one."""
    end = brace_span(text, start)
    if end is None:
        return None
    if not _is_return_declaration(text, start, end):
        return None
    keys = declared_keys(text[start + 1:end - 1])
    return keys if _is_shape_literal(keys) else None


def find_declarations(
    text: str,
    tool_name: str,
    positions: "list[tuple[int, str]]",
    sections: "list[tuple[int, str | None]]",
    schema: "dict[str, list[str]] | None" = None,
) -> "list[tuple[int, list[str]]]":
    """Every return-shape declaration ATTRIBUTED to ``tool_name`` in ``text``."""
    found: "list[tuple[int, list[str]]]" = []
    for match in BRACE_RE.finditer(text):
        start = match.start()
        if owning_tool(text, sections, positions, start) != tool_name:
            continue
        keys = _declaration_at(text, start)
        if keys is None:
            continue
        if schema and _is_another_tools_exact_shape(keys, tool_name, schema):
            # The literal IS another published tool's root shape, verbatim.
            # A sentence like "unrelate — relate's exact undo — answers
            # {found, removed}" names both tools, and proximity alone charged
            # the neighbour; an exact match of a sibling's schema is that
            # sibling's correct declaration, not this tool lying.
            continue
        found.append((start, keys))
    return found


def _is_another_tools_exact_shape(
    keys: "list[str]",
    tool_name: str,
    schema: "dict[str, list[str]]",
) -> bool:
    """Whether ``keys`` equals the FULL root shape of a DIFFERENT tool."""
    published = set(keys)
    if published == set(schema.get(tool_name, ())):
        return False
    return any(
        other != tool_name and published == set(other_keys) and other_keys
        for other, other_keys in schema.items()
    )


# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------


def rel(root: Path, path: Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:  # pragma: no cover - defensive
        return str(path)


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _drift_message(
    where: str,
    tool: PolicedTool,
    keys: "list[str]",
    schema_keys: "list[str]",
) -> str:
    declared = sorted(set(keys))
    # The diff is computed on the case-folded form, so a camelCase binding is
    # not accused of a rename it did not make; the message still shows both
    # sides as they are actually written.
    folded = {snake_case(key) for key in declared}
    published = {snake_case(key) for key in schema_keys}
    missing = sorted(published - folded)
    extra = sorted(folded - published)
    parts = []
    if missing:
        parts.append(f"missing {', '.join(missing)}")
    if extra:
        parts.append(f"unknown {', '.join(extra)}")
    return (
        f"{where}: `{tool.name}` is described as returning "
        f"{{{', '.join(declared)}}} but the published outputSchema in "
        f"{TOOLS_CAPTURE} has {{{', '.join(schema_keys)}}} ({'; '.join(parts)}). "
        f"Fix the surface, or the server and re-capture the tools. [return-shape]"
    )


# --------------------------------------------------------------------------
# Anti-disarm invariants
# --------------------------------------------------------------------------


def _tool_registry_problems(
    tool: PolicedTool,
    schema: "dict[str, list[str]]",
    swept: "set[str]",
) -> "list[str]":
    if tool.name not in schema:
        return [f"{TOOLS_CAPTURE}: no tool named `{tool.name}` — the registry is stale."]
    if not schema[tool.name]:
        return [
            f"{TOOLS_CAPTURE}: `{tool.name}` publishes an EMPTY outputSchema, so this "
            "guard would compare against nothing and pass vacuously."
        ]
    if not tool.pinned_surfaces:
        return [
            f"`{tool.name}` pins no surface: the sweep alone goes green the moment "
            "every page stops describing the contract."
        ]
    unreachable = sorted(set(tool.pinned_surfaces) - swept)
    return [
        f"{surface}: pinned for `{tool.name}` but the sweep never reads it "
        "(missing file, or outside SURFACE_GLOBS) — the pin enforces nothing."
        for surface in unreachable
    ]


def _structural_failures(
    schema: "dict[str, list[str]]",
    files: "list[Path]",
    swept: "set[str]",
) -> "list[str]":
    problems: "list[str]" = []
    if not POLICED_TOOLS:
        problems.append("POLICED_TOOLS is EMPTY — this guard would verify nothing.")
    if len(build_alias_index(sorted(schema), POLICED_TOOLS)) < len(schema):
        problems.append(
            "the alias index is smaller than the capture: attribution would charge "
            "a sibling tool's declaration to a policed one."
        )
    if not files:
        problems.append(
            f"the surface sweep matched ZERO file under {len(SURFACE_GLOBS)} glob(s): "
            "the extraction is broken, not the tree."
        )
    for tool in POLICED_TOOLS:
        problems.extend(_tool_registry_problems(tool, schema, swept))
    return problems


def _coverage_failures(tool: PolicedTool, seen: "set[str]") -> "list[str]":
    return [
        f"{surface}: pinned as a surface describing `{tool.name}`'s return contract, "
        f"but no shape declaration was found in it. Restore a sentence naming the "
        f"tool and the literal it returns, or drop the pin from POLICED_TOOLS "
        f"on purpose. [return-shape]"
        for surface in tool.pinned_surfaces
        if surface not in seen
    ]


# --------------------------------------------------------------------------
# Guard
# --------------------------------------------------------------------------


def _check_tool(
    tool: PolicedTool,
    schema_keys: "list[str]",
    texts: "list[tuple[str, str]]",
    index: "dict[str, str]",
    schema: "dict[str, list[str]] | None" = None,
) -> "tuple[list[str], list[str]]":
    failures: "list[str]" = []
    info: "list[str]" = []
    seen: "set[str]" = set()
    published = sorted(snake_case(key) for key in schema_keys)
    for name, raw in texts:
        reason = non_mcp_reason(name)
        if reason is not None:
            # Declared out of MCP scope: this surface documents another API
            # that shares the tool's verb. Skipped WITH its reason on file,
            # and the registry itself is policed for staleness below.
            continue
        text = mask_jsdoc_links(raw)
        positions = alias_positions(text, index)
        sections = section_positions(text, set(index.values()))
        for offset, keys in find_declarations(text, tool.name, positions, sections, schema):
            seen.add(name)
            where = f"{name}:{line_of(raw, offset)}"
            if sorted({snake_case(key) for key in keys}) == published:
                info.append(f"  ok  {where} [{tool.name}] {{{', '.join(keys)}}}")
                continue
            failures.append(_drift_message(where, tool, keys, schema_keys))
    failures.extend(_coverage_failures(tool, seen))
    info.insert(
        0,
        f"{tool.name}: published root keys {{{', '.join(schema_keys)}}}; "
        f"{len(seen)} surface(s) declare them, {len(tool.pinned_surfaces)} pinned.",
    )
    return failures, info


def guard_return_shape(root: Path) -> "tuple[list[str], list[str]]":
    schema = load_output_schema_keys(root)
    files = surface_files(root)
    swept = {rel(root, path) for path in files}
    info = [f"Swept {len(files)} surface file(s) against {TOOLS_CAPTURE}."]
    structural = _structural_failures(schema, files, swept)
    if structural:
        return structural, info
    index = build_alias_index(sorted(schema), POLICED_TOOLS)
    texts = [
        (rel(root, path), path.read_text(encoding="utf-8", errors="replace"))
        for path in files
    ]
    failures: "list[str]" = []
    for prefix in NON_MCP_SURFACES:
        if not any(name == prefix or name.startswith(prefix) for name in swept):
            failures.append(
                f"NON_MCP_SURFACES entry {prefix!r} matches no swept surface — "
                "a stale exemption hides nothing today and everything tomorrow; "
                "drop it or fix the path."
            )
    for tool in POLICED_TOOLS:
        tool_failures, tool_info = _check_tool(tool, schema[tool.name], texts, index, schema)
        failures.extend(tool_failures)
        info.extend(tool_info)
    return failures, info


GUARDS = {
    "return-shape": (
        guard_return_shape,
        "every surface describing an MCP tool's return announces the published root keys",
    ),
}


def _report(name: str, failures: "list[str]", mode: str, annotate: bool) -> None:
    """Print the problems; emit GitHub annotations only for the REAL tree.

    ``::error file=…`` pins a message to a path in the PR's "Files changed"
    tab. The self-test runs this guard against a synthetic repository under a
    temp dir, and those paths do not mean anything outside it — emitting them
    put two red annotations and one warning on a real, innocent file every
    time the suite ran, about a tool that exists nowhere in the tree. A
    reviewer who learns to ignore this guard's annotations is a reviewer who
    will ignore the true one.
    """
    label = "FAIL" if mode == "strict" else "WARN"
    annotation = "error" if mode == "strict" else "warning"
    print(f"   {label} ({name}): {len(failures)} problem(s)")
    for line in failures:
        print(f"     - {line}")
        if annotate:
            print(f"::{annotation} file={line.split(':', 1)[0]}::{line}")
    print()


def run(root: Path, names: "list[str]", mode: str, verbose: bool) -> int:
    annotate = root.resolve() == REPO_ROOT.resolve()
    failed_strict = False
    for name in names:
        func, blurb = GUARDS[name]
        print(f"== guard '{name}': {blurb}")
        failures, info = func(root)
        for line in info if verbose else info[:1]:
            print(f"   {line}")
        if not failures:
            print(f"   PASS ({name})\n")
            continue
        _report(name, failures, mode, annotate)
        failed_strict = failed_strict or mode == "strict"
    if failed_strict:
        print("MCP doc-contract guards FAILED.")
        return 1
    print("MCP doc-contract guards passed.")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--guard",
        action="append",
        choices=sorted(GUARDS) + ["all"],
        help="guard to run (repeatable). Default: all.",
    )
    parser.add_argument(
        "--mode",
        choices=("strict", "warn"),
        default="strict",
        help="strict (default) exits 1 on any problem; warn reports and exits 0.",
    )
    parser.add_argument("--root", default=str(REPO_ROOT), help="repository root to scan")
    parser.add_argument(
        "-v", "--verbose", action="store_true", help="list every declaration found"
    )
    args = parser.parse_args(argv)

    selected = args.guard or ["all"]
    names = sorted(GUARDS) if "all" in selected else sorted(set(selected))

    root = Path(args.root).resolve()
    if not (root / TOOLS_CAPTURE).is_file():
        print(f"ERROR: {root}/{TOOLS_CAPTURE} not found", file=sys.stderr)
        return 2
    try:
        return run(root, names, args.mode, args.verbose)
    except (OSError, RuntimeError, ValueError, KeyError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
