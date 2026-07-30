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

**Second written limit: the registry is ONE tool, and that is not an
oversight.** ``load_working_context`` is policed; the other nineteen tools
the capture publishes are NOT. Policing them was measured, not assumed:
attribution by proximity collapses inside ``docs/reference/MCP_TOOLS.md``,
where every tool's section sits within 500 characters of its neighbours', and
the sweep produced 68 "non-conformances" of which almost all were code
braces, error payloads and neighbouring sections. Widening the registry needs
a section-aware extractor, not a bigger constant. Say "this guard polices
``load_working_context``", never "every MCP tool".

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
  3. ATTRIBUTED to the tool whose alias is NEAREST, among the aliases of
     EVERY tool the capture publishes, within 500 characters.

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
    PolicedTool(
        "load_working_context",
        ("loadWorkingContext", "LoadedWorkingContext"),
        (
            "crates/velesdb-node/README.md",
            "crates/velesdb-node/skills/velesdb-context-optimizer/SKILL.md",
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
            "skills/velesdb-context-optimizer/SKILL.md",
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

# `{"error": "…"}` is the documented error payload integrations/langgraph
# returns instead of raising. Exactly one key wide.
ERROR_PAYLOAD_KEYS = frozenset({"error"})

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


def surface_files(root: Path) -> "list[Path]":
    """Every file the sweep reads, deduplicated and sorted."""
    matched: "set[Path]" = set()
    for glob in SURFACE_GLOBS:
        matched.update(p for p in root.glob(glob) if p.is_file() and _is_scanned(root, p))
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


# --------------------------------------------------------------------------
# Extraction
# --------------------------------------------------------------------------


def _is_return_declaration(text: str, start: int, end: int) -> bool:
    """A return verb just before the literal, or a shape noun just after it."""
    if RETURN_VERB_RE.search(text[max(0, start - VERB_LOOKBACK):start]):
        return True
    return bool(SHAPE_NOUN_RE.match(text[end:end + NOUN_LOOKAHEAD]))


def _is_shape_literal(keys: "list[str]") -> bool:
    """Non-empty, all identifier-shaped, and not the `{error}` payload."""
    if not keys or set(keys) == ERROR_PAYLOAD_KEYS:
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
) -> "list[tuple[int, list[str]]]":
    """Every return-shape declaration ATTRIBUTED to ``tool_name`` in ``text``."""
    found: "list[tuple[int, list[str]]]" = []
    for match in BRACE_RE.finditer(text):
        start = match.start()
        if nearest_tool(positions, start) != tool_name:
            continue
        keys = _declaration_at(text, start)
        if keys is not None:
            found.append((start, keys))
    return found


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
    missing = sorted(set(schema_keys) - set(declared))
    extra = sorted(set(declared) - set(schema_keys))
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
) -> "tuple[list[str], list[str]]":
    failures: "list[str]" = []
    info: "list[str]" = []
    seen: "set[str]" = set()
    for name, raw in texts:
        text = mask_jsdoc_links(raw)
        positions = alias_positions(text, index)
        for offset, keys in find_declarations(text, tool.name, positions):
            seen.add(name)
            where = f"{name}:{line_of(raw, offset)}"
            if sorted(set(keys)) == schema_keys:
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
    for tool in POLICED_TOOLS:
        tool_failures, tool_info = _check_tool(tool, schema[tool.name], texts, index)
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
