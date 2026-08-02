"""Tests for scripts/check-mcp-doc-contract.py.

Same discipline as ``test_check_doc_freshness.py``: every rule is pinned
RED-first on a synthetic repository built under a temp dir, then repaired and
re-asserted green. A guard nobody has seen refuse protects nothing.

Three families here, and the last two are the ones that matter most:

  * the RULE — a surface that declares the wrong root keys is refused;
  * the ANTI-DISARM invariants — an empty registry, an empty sweep, a tool
    missing from the capture, a tool with an empty ``outputSchema``, a tool
    with no pinned surface, a pinned surface the sweep cannot reach: each of
    those makes the guard FAIL rather than pass vacuously. This is the exact
    failure mode ``scripts/check-doc-contract.sh`` documents in its header
    (an extraction that broke and then iterated over an empty array);
  * the EXTRACTION — the narrow definition of "declaration" is asserted on
    the real shapes found in this tree (a JSDoc ``*`` continuation, a
    JavaScript string concatenation, a nested dict, a reStructuredText line
    wrap) AND on the near-misses it must keep refusing to see (an input
    schema sitting next to the tool with no return verb in front of it).

The last test runs the guard against the REAL repository. Without it the
synthetic suite would keep passing while the registry rotted.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import shutil
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-mcp-doc-contract.py"
REPO_ROOT = SCRIPT_PATH.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_mcp_doc_contract", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


cmdc = _load_script()

ENVELOPE = ["found", "other_sessions", "working"]

SIBLING = ["found", "removed"]

CAPTURE = {
    "tools": [
        {
            "name": "demo_tool",
            "description": "A tool.",
            "inputSchema": {"properties": {"project": {"type": "string"}}},
            "outputSchema": {
                "properties": {key: {"type": "string"} for key in ENVELOPE},
                "required": ["found"],
            },
        },
        {
            # A SIBLING tool sharing the `found` key, published by the same
            # server. Its correct declaration must never be read as
            # `demo_tool` drifting.
            "name": "sibling_tool",
            "description": "Another tool.",
            "inputSchema": {"properties": {"id": {"type": "string"}}},
            "outputSchema": {
                "properties": {key: {"type": "string"} for key in SIBLING},
                "required": ["found"],
            },
        },
    ]
}

REFERENCE_CLEAN = """# Tools

## `demo_tool`

Resume a session.

Returns `{ found, working, other_sessions }`. `found: false` means nothing was
saved under that exact pair.
"""

SKILL_CLEAN = """# Skill

Call `demo_tool` with the project and a stable session id. It returns
`{found, working, other_sessions}`. When `found` is true, adopt the state.
"""


def _tool(pinned: "tuple[str, ...]" = ("docs/reference/DEMO_TOOLS.md",)) -> "cmdc.PolicedTool":
    return cmdc.PolicedTool("demo_tool", ("demoTool",), pinned)


class DocContractTestCase(unittest.TestCase):
    """A minimal, self-consistent fake repository under a temp dir."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="mcp-doc-contract-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.write("docs/reference/mcp-tools.json", json.dumps(CAPTURE, indent=2))
        self.write("docs/reference/DEMO_TOOLS.md", REFERENCE_CLEAN)
        self.write("skills/demo/SKILL.md", SKILL_CLEAN)
        self.policed(_tool())

    def write(self, rel_path: str, content: str) -> None:
        path = self.tmp / rel_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def policed(self, *tools: "cmdc.PolicedTool") -> None:
        """Swap the module-level registry for this test only."""
        previous = cmdc.POLICED_TOOLS
        cmdc.POLICED_TOOLS = tuple(tools)
        self.addCleanup(setattr, cmdc, "POLICED_TOOLS", previous)

    def assertGuardPasses(self) -> None:
        failures, _info = cmdc.guard_return_shape(self.tmp)
        self.assertEqual(failures, [], f"guard should pass but reported {failures}")

    def assertGuardFails(self, *expected_substrings: str) -> "list[str]":
        failures, _info = cmdc.guard_return_shape(self.tmp)
        self.assertTrue(failures, "guard should have failed but reported nothing")
        joined = "\n".join(failures)
        for needle in expected_substrings:
            self.assertIn(needle, joined)
        return failures


class ReturnShapeRuleTests(DocContractTestCase):
    def test_baseline_repository_passes(self) -> None:
        self.assertGuardPasses()

    def test_bare_shape_fails_then_passes_once_the_envelope_is_restored(self) -> None:
        # The exact drift #1694 had to fix by hand across ten surfaces.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN.replace("{ found, working, other_sessions }", "{ working }"),
        )
        self.assertGuardFails(
            "described as returning {working}",
            "missing found, other_sessions",
        )
        self.write("docs/reference/DEMO_TOOLS.md", REFERENCE_CLEAN)
        self.assertGuardPasses()

    def test_invented_extra_key_is_refused(self) -> None:
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN.replace(
                "{ found, working, other_sessions }",
                "{ found, working, other_sessions, resumed_at }",
            ),
        )
        self.assertGuardFails("unknown resumed_at")

    def test_an_unpinned_surface_is_policed_too(self) -> None:
        # The sweep is what catches a NEW surface nobody registered.
        self.write("skills/demo/SKILL.md", SKILL_CLEAN.replace(", other_sessions", ""))
        self.assertGuardFails("skills/demo/SKILL.md", "missing other_sessions")

    def test_a_TOTAL_rename_is_refused_and_not_silently_unseen(self) -> None:
        # The hole a wider rule closes: when recognition depended on the
        # literal INTERSECTING the published keys, renaming EVERY key made
        # the declaration stop existing instead of failing. On a surface
        # carrying a second, correct declaration the file also stayed in
        # `seen`, so the coverage check never fired either: a page could lie
        # from end to end and the guard stayed green.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN
            + "\nAnd again: `demo_tool` returns `{ context, sessions }`.\n",
        )
        self.assertGuardFails("described as returning {context, sessions}")

    def test_a_sibling_tools_CORRECT_declaration_is_not_read_as_drift(self) -> None:
        # `sibling_tool` publishes {found, removed} and shares `found`.
        # Documented next to demo_tool — inside the 500-character anchor
        # window — a correct sentence about it must not turn a required
        # check red and accuse demo_tool.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN + "\n`sibling_tool` resolves `{found, removed}`.\n",
        )
        self.assertGuardPasses()

    def test_declaration_removed_from_a_pinned_surface_fails(self) -> None:
        # Deleting the sentence must not be a way to go green: this is the
        # pre-#1694 state of WASM_API.md, which said nothing at all.
        self.write("docs/reference/DEMO_TOOLS.md", "# Tools\n\n## `demo_tool`\n\nResume.\n")
        self.assertGuardFails(
            "docs/reference/DEMO_TOOLS.md",
            "no shape declaration was found in it",
        )


class AntiDisarmTests(DocContractTestCase):
    """Every way this guard could quietly verify nothing must be red."""

    def test_empty_registry_fails(self) -> None:
        self.policed()
        self.assertGuardFails("POLICED_TOOLS is EMPTY")

    def test_tool_absent_from_the_capture_fails(self) -> None:
        self.policed(cmdc.PolicedTool("ghost_tool", (), ("docs/reference/DEMO_TOOLS.md",)))
        self.assertGuardFails("no tool named `ghost_tool`")

    def test_tool_with_an_empty_output_schema_fails(self) -> None:
        empty = json.loads(json.dumps(CAPTURE))
        empty["tools"][0]["outputSchema"] = {"properties": {}}
        self.write("docs/reference/mcp-tools.json", json.dumps(empty))
        self.assertGuardFails("publishes an EMPTY outputSchema")

    def test_tool_with_no_pinned_surface_fails(self) -> None:
        self.policed(_tool(pinned=()))
        self.assertGuardFails("pins no surface")

    def test_pinned_surface_the_sweep_cannot_reach_fails(self) -> None:
        self.policed(_tool(pinned=("docs/reference/DEMO_TOOLS.md", "no/such/file.md")))
        self.assertGuardFails("no/such/file.md", "the sweep never reads it")

    def test_pinned_surface_outside_the_globs_fails(self) -> None:
        # A real file, but in a directory the sweep does not look at: the pin
        # would be decoration.
        self.write("private/NOTES.md", SKILL_CLEAN)
        self.policed(_tool(pinned=("docs/reference/DEMO_TOOLS.md", "private/NOTES.md")))
        self.assertGuardFails("private/NOTES.md", "outside SURFACE_GLOBS")

    def test_empty_sweep_fails(self) -> None:
        previous = cmdc.SURFACE_GLOBS
        cmdc.SURFACE_GLOBS = ("nothing/**/*.md",)
        self.addCleanup(setattr, cmdc, "SURFACE_GLOBS", previous)
        self.assertGuardFails("matched ZERO file")

    def test_capture_with_no_tool_is_an_error_not_a_pass(self) -> None:
        self.write("docs/reference/mcp-tools.json", json.dumps({"tools": []}))
        with self.assertRaises(RuntimeError):
            cmdc.guard_return_shape(self.tmp)


class OwnershipTests(DocContractTestCase):
    """Which tool a literal is ABOUT — the three tiers of ``owning_tool``,
    each pinned by the case that made it necessary. Every fixture here is
    reduced from a site measured on the real tree while widening the registry
    for #1695. Distinct from ``AttributionTests`` below, which pins the alias
    index those tiers read."""

    def test_a_neighbours_name_in_the_prose_does_not_steal_a_sections_declaration(
        self,
    ) -> None:
        # `sibling_tool`'s own section, whose prose mentions `demo_tool`
        # CLOSER to the literal than its own heading sits. Proximity alone
        # read this as `demo_tool` drifting: on docs/reference/MCP_TOOLS.md
        # that mistake accounted for 13 of 19 declarations.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN
            + "\n## `sibling_tool`\n\nThe exact undo of `demo_tool`'s edge.\n\n"
            + "Returns `{ found, removed }`.\n",
        )
        self.assertGuardPasses()

    def test_a_subject_on_the_literals_own_line_overrides_the_section(self) -> None:
        # The reverse mistake, and why the section cannot simply win: a
        # sibling documented INSIDE another tool's section names itself right
        # beside its literal. The narrower statement has to prevail, or the
        # section rule turns a correct tree red the way proximity did.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN + "\n`sibling_tool` resolves `{found, removed}`.\n",
        )
        self.assertGuardPasses()

    def test_a_heading_that_names_no_tool_closes_the_section_above_it(self) -> None:
        # A trailing "Error model" section must not inherit the last tool
        # documented above it. The padding puts the literal out of proximity
        # range, so the section is the ONLY thing that could attribute it.
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN
            + "\n## Error model\n\n"
            + "Filler with no alias in it at all. " * 20
            + "\n\nEvery call resolves `{ code, message }` on failure.\n",
        )
        self.assertGuardPasses()

    def test_a_camel_case_rendering_of_the_same_keys_is_not_drift(self) -> None:
        # napi and wasm publish camelCase because that IS their contract.
        # Comparing the rendered spelling accused a correct binding of lying
        # (crates/velesdb-node/src/lib.rs:229, docs/guides/WASM_API.md:271).
        self.write(
            "skills/demo/SKILL.md",
            SKILL_CLEAN.replace("other_sessions", "otherSessions"),
        )
        self.assertGuardPasses()

    def test_a_renamed_key_is_still_refused_under_the_casing_fold(self) -> None:
        # The control that keeps the fold honest: only the CASE is folded, so
        # a key that was renamed is still a different key and still fails.
        self.write(
            "skills/demo/SKILL.md",
            SKILL_CLEAN.replace("other_sessions", "sessions"),
        )
        self.assertGuardFails("missing other_sessions", "unknown sessions")


class ExtractionTests(unittest.TestCase):
    """The narrow definition of "declaration", on real shapes from this tree."""

    def _declarations(self, text: str) -> "list[list[str]]":
        masked = cmdc.mask_jsdoc_links(text)
        index = cmdc.build_alias_index(["demo_tool", "sibling_tool"])
        positions = cmdc.alias_positions(masked, index)
        sections = cmdc.section_positions(masked, set(index.values()))
        return [
            keys
            for _offset, keys in cmdc.find_declarations(
                masked, "demo_tool", positions, sections
            )
        ]

    def test_a_type_name_literal_is_not_a_shape(self) -> None:
        # `-> AsyncTask<…> {` cleared IDENTIFIER_RE because `AsyncTask` IS an
        # identifier, and was read as a declaration of `{AsyncTask}`
        # (crates/velesdb-node/src/lib.rs:646). Every published root key is
        # snake_case and every binding renders them camelCase, so refusing
        # all-PascalCase literals cannot lose a real one.
        self.assertEqual(self._declarations("`demo_tool` -> {AsyncTask}"), [])

    def test_the_mcp_call_envelope_is_not_a_return_shape(self) -> None:
        # A skill page shows how to CALL a tool a few dozen characters after
        # prose about what it `returns`, which is enough for the return anchor
        # to claim the example (skills/velesdb-context-optimizer/SKILL.md:317).
        self.assertEqual(
            self._declarations(
                "`demo_tool` returns the state.\n\n"
                '{"tool": "demo_tool", "arguments": {"project": "veles"}}'
            ),
            [],
        )

    def test_a_destructuring_belongs_to_the_call_on_its_own_line(self) -> None:
        # A destructuring names its subject AFTER the literal, and a nearer
        # mention of a different tool sits on the line above — the shape of
        # docs/guides/NODE_ADDON.md:226-228. Proximity picks the neighbour
        # because it is closer in characters, and so does the backwards-only
        # reading that prose alone suggests ("X returns Y" names X first),
        # since it finds no alias before the literal and falls through. Only
        # the line the literal sits on gets this right.
        self.assertEqual(
            self._declarations(
                "// The envelope `demoTool` resolves.\n"
                "const { sessions } = await store.backend().siblingTool('veles')\n"
            ),
            [],
        )

    def test_plain_prose_literal(self) -> None:
        self.assertEqual(
            self._declarations("`demo_tool` returns `{found, working, other_sessions}`."),
            [["found", "working", "other_sessions"]],
        )

    def test_jsdoc_continuation_star_is_not_part_of_the_key(self) -> None:
        # sdks/typescript/src/memory.ts wraps the literal across a `*` line.
        text = (
            "/**\n * The resumption envelope {@link saveWorkingContext} mirrors:"
            " `{found,\n * working, other_sessions}`, the shape `demo_tool` serves.\n */"
        )
        self.assertEqual(self._declarations(text), [["found", "working", "other_sessions"]])

    def test_javascript_string_concatenation_is_not_part_of_the_key(self) -> None:
        # sdks/typescript/src/memory.ts:994 splits the literal across a `' +` join.
        text = (
            "throw new Error(\n  'build predates the {found, working, ' +\n"
            "    'other_sessions} envelope that demo_tool returns'\n)"
        )
        self.assertEqual(self._declarations(text), [["found", "working", "other_sessions"]])

    def test_nested_value_does_not_leak_into_the_top_level_keys(self) -> None:
        # docs/guides/PYTHON_CONTEXT_COMPILER.md documents it as a Python repr.
        text = (
            '# -> {"found": True, "working": {"goal": "x", "decisions": []},'
            ' "other_sessions": [...]}\nmem.demo_tool("veles", "s1")'
        )
        self.assertEqual(self._declarations(text), [["found", "working", "other_sessions"]])

    def test_restructuredtext_line_wrap_inside_the_literal(self) -> None:
        # integrations/langgraph/src/langgraph_velesdb/tools.py wraps mid-literal.
        text = (
            'def demo_tool(self):\n    """Returns ``{"found": bool, "working": dict | None,'
            ' "other_sessions":\n        [str]}``."""'
        )
        self.assertEqual(self._declarations(text), [["found", "working", "other_sessions"]])

    def test_an_input_schema_next_to_the_tool_is_not_a_return_declaration(self) -> None:
        # No return verb in front. A wider rule was measured on this tree
        # first and turned ~110 of these into false positives.
        text = "`demo_tool` takes `{ project, session, working }` as arguments."
        self.assertEqual(self._declarations(text), [])

    def test_a_literal_too_far_from_any_mention_of_the_tool_is_ignored(self) -> None:
        text = "`demo_tool` is a tool.\n\n" + ("filler. " * 90) + "It returns `{found, working}`."
        self.assertEqual(self._declarations(text), [])

    def test_the_error_payload_is_not_a_return_shape(self) -> None:
        # The error payload integrations/langgraph documents next to the tool.
        # Exempted by NAME (`{error}`, one key), not by "shares no key with
        # the schema" — the latter also swallowed total renames.
        text = '`demo_tool` returns `{"error": "upgrade the wheel"}` instead of raising.'
        self.assertEqual(self._declarations(text), [])

    def test_a_TOTAL_rename_is_still_seen_as_a_declaration(self) -> None:
        # Recognition must not depend on the expected keys, or renaming all
        # of them is a way to disappear instead of a way to fail.
        text = "`demo_tool` returns `{context, sessions}`."
        self.assertEqual(self._declarations(text), [["context", "sessions"]])

    def test_a_sibling_tools_literal_is_attributed_to_the_sibling(self) -> None:
        text = "`demo_tool` resumes. Nearby, `sibling_tool` returns `{found, removed}`."
        self.assertEqual(self._declarations(text), [])

    def test_a_rust_function_body_anchored_by_its_return_arrow_is_not_a_shape(self) -> None:
        # `fn demo_tool(…) -> AsyncTask<Job<JsonOut>> {` puts a `->` within
        # the lookback of a body brace. Only the identifier-shape rule tells
        # the two apart.
        text = "pub fn demo_tool(&self) -> AsyncTask<Job<JsonOut>> {\n    let svc = Arc::clone(&self.0);\n}"
        self.assertEqual(self._declarations(text), [])

    def test_a_typescript_return_type_literal_is_read(self) -> None:
        # crates/velesdb-node/src/lib.rs ships this string as the .d.ts every
        # npm consumer compiles against. Semicolons, not commas.
        text = (
            '#[napi(js_name = "demoTool", ts_return_type = "Promise<{ found: boolean;'
            ' working: object | null; other_sessions: Array<string> }>")]'
        )
        self.assertEqual(self._declarations(text), [["found", "working", "other_sessions"]])

    def test_jsdoc_link_is_masked_without_shifting_offsets(self) -> None:
        text = "a {@link Foo} b"
        masked = cmdc.mask_jsdoc_links(text)
        self.assertEqual(len(masked), len(text))
        self.assertNotIn("{", masked)


class AttributionTests(unittest.TestCase):
    """Nearest-alias attribution, over every tool the capture publishes."""

    def setUp(self) -> None:
        self.index = cmdc.build_alias_index(["load_working_context", "recall", "recall_fused"])

    def test_camel_case_alias_is_derived(self) -> None:
        self.assertEqual(self.index["loadWorkingContext"], "load_working_context")

    def test_a_registry_alias_wins_over_the_derived_one(self) -> None:
        index = cmdc.build_alias_index(
            ["load_working_context"],
            (cmdc.PolicedTool("load_working_context", ("LoadedWorkingContext",), ()),),
        )
        self.assertEqual(index["LoadedWorkingContext"], "load_working_context")

    def test_a_shorter_alias_does_not_match_inside_a_longer_tool_name(self) -> None:
        # Without word boundaries `recall` matches inside `recall_fused` and
        # wins every attribution contest at distance zero.
        positions = cmdc.alias_positions("see recall_fused for more", self.index)
        self.assertEqual([tool for _offset, tool in positions], ["recall_fused"])

    def test_the_nearest_alias_wins(self) -> None:
        text = "recall ......................................... load_working_context X"
        offset = text.index("X")
        self.assertEqual(cmdc.nearest_tool(cmdc.alias_positions(text, self.index), offset),
                         "load_working_context")

    def test_nothing_is_attributed_beyond_the_anchor_window(self) -> None:
        text = "load_working_context" + ("." * (cmdc.ANCHOR_WINDOW + 10)) + "X"
        offset = text.index("X")
        self.assertIsNone(cmdc.nearest_tool(cmdc.alias_positions(text, self.index), offset))


class ModeTests(DocContractTestCase):
    """`--mode warn` reports the very same problems without failing."""

    def _run(self, mode: str) -> int:
        return cmdc.main(["--root", str(self.tmp), "--mode", mode])

    def test_strict_exits_1_and_warn_exits_0_on_identical_input(self) -> None:
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN.replace("{ found, working, other_sessions }", "{ working }"),
        )
        self.assertEqual(self._run("strict"), 1)
        self.assertEqual(self._run("warn"), 0)

    def test_strict_exits_0_once_the_problem_is_fixed(self) -> None:
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN.replace("{ found, working, other_sessions }", "{ working }"),
        )
        self.assertEqual(self._run("strict"), 1)
        self.write("docs/reference/DEMO_TOOLS.md", REFERENCE_CLEAN)
        self.assertEqual(self._run("strict"), 0)

    def test_a_missing_capture_is_a_usage_error_not_a_pass(self) -> None:
        (self.tmp / "docs" / "reference" / "mcp-tools.json").unlink()
        self.assertEqual(self._run("strict"), 2)


class AnnotationTests(DocContractTestCase):
    """``::error file=…`` must never be emitted for a synthetic tree.

    It pins a message to a path in the PR's "Files changed" tab. This suite
    scans a temp-dir repository whose paths mean nothing outside it; before
    this was gated on the root, every run of this very file printed two
    errors and one warning against a REAL repository file, about a tool named
    nowhere in the tree. Reviewers learn to ignore a guard that cries wolf.
    """

    def _stdout_of_a_failing_run(self, root: Path) -> str:
        self.write(
            "docs/reference/DEMO_TOOLS.md",
            REFERENCE_CLEAN.replace("{ found, working, other_sessions }", "{ working }"),
        )
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            cmdc.main(["--root", str(root)])
        return buffer.getvalue()

    def test_a_synthetic_root_emits_no_github_annotation(self) -> None:
        output = self._stdout_of_a_failing_run(self.tmp)
        self.assertIn("FAIL (return-shape)", output, "the run must still report the problem")
        self.assertNotIn("::error", output)
        self.assertNotIn("::warning", output)

    def test_the_real_root_still_annotates(self) -> None:
        # The suppression is scoped to a foreign root, not a way to make the
        # guard quiet in CI.
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            cmdc._report("return-shape", ["docs/x.md:1: bad"], "strict", annotate=True)
        self.assertIn("::error file=docs/x.md::", buffer.getvalue())


class RealRepositoryTests(unittest.TestCase):
    """The synthetic suite above would stay green on a rotted registry."""

    def test_the_guard_passes_on_this_repository(self) -> None:
        failures, _info = cmdc.guard_return_shape(REPO_ROOT)
        self.assertEqual(failures, [], "\n".join(failures))

    def test_every_policed_tool_is_published_by_the_capture(self) -> None:
        schema = cmdc.load_output_schema_keys(REPO_ROOT)
        for tool in cmdc.POLICED_TOOLS:
            with self.subTest(tool=tool.name):
                self.assertIn(tool.name, schema)
                self.assertTrue(schema[tool.name], "empty outputSchema")

    def test_load_working_context_is_policed(self) -> None:
        # The tool whose drift #1694 fixed by hand. Dropping it from the
        # registry must break a test, not silently shrink the gate.
        self.assertIn("load_working_context", {tool.name for tool in cmdc.POLICED_TOOLS})

    def test_the_pinned_surfaces_are_EXACTLY_these(self) -> None:
        # The guard refuses a tool with ZERO pins, and refused nothing in
        # between: sixteen pins could be walked down to one, one deletion at
        # a time, each looking like a small cleanup — and the guard's own
        # failure message offers exactly that ("or drop the pin from
        # POLICED_TOOLS on purpose"). The sweep cannot cover for a lost pin,
        # because the sweep only ever sees declarations that are PRESENT;
        # deleting the sentence is what the pin exists to catch.
        #
        # Adding a surface is a one-line edit here. Removing one has to be
        # argued for in a diff that says so.
        #
        # Pinned for EVERY policed tool, not just the first one: a registry
        # that grows by batches (#1695) needs the counter to grow with it, or
        # nine of the ten are back to being deletable one sentence at a time.
        expected = {
            "compile_transcript": {
                "crates/velesdb-node/src/lib.rs",
                "docs/reference/MCP_TOOLS.md",
                "sdks/typescript/src/memory.ts",
            },
            "context_savings": {"docs/reference/MCP_TOOLS.md"},
            "explain_compilation": {
                "crates/velesdb-node/src/lib.rs",
                "docs/reference/MCP_TOOLS.md",
            },
            "forget": {"docs/reference/MCP_TOOLS.md"},
            "list_working_contexts": {
                "crates/velesdb-node/src/lib.rs",
                "docs/guides/NODE_ADDON.md",
                "docs/reference/MCP_TOOLS.md",
            },
            "load_working_context": {
                # The two `velesdb-context-optimizer` copies used to be here.
                # Resumption moved to the memory skill, and the pin moved with
                # the declaration rather than being dropped — same count, same
                # coverage, different owner.
                "crates/velesdb-memory/skill/velesdb-memory/SKILL.md",
                "crates/velesdb-node/README.md",
                "crates/velesdb-node/skills/velesdb-memory/SKILL.md",
                "crates/velesdb-node/src/lib.rs",
                "docs/guides/NODE_ADDON.md",
                "docs/guides/PYTHON_CONTEXT_COMPILER.md",
                "docs/guides/WASM_API.md",
                "docs/reference/ECOSYSTEM_PARITY.md",
                "docs/reference/MCP_TOOLS.md",
                "integrations/agent-hooks/claude-code/hooks/session-start.sh",
                "integrations/agent-hooks/codex/README.md",
                "integrations/agent-hooks/codex/hooks/session-start.sh",
                "integrations/agent-hooks/windsurf/hooks/pre-user-prompt.sh",
                "integrations/langgraph/README.md",
                "integrations/langgraph/src/langgraph_velesdb/tools.py",
                "sdks/typescript/src/memory.ts",
            },
            "recall_fused": {
                "crates/velesdb-node/src/lib.rs",
                "docs/reference/MCP_TOOLS.md",
                "integrations/langgraph/README.md",
            },
            "retrieve_context_source": {
                "crates/velesdb-node/src/lib.rs",
                "docs/guides/CONTEXT_COMPILER.md",
                "docs/guides/NODE_ADDON.md",
                "docs/guides/PYTHON_CONTEXT_COMPILER.md",
                "docs/reference/MCP_TOOLS.md",
            },
            "save_working_context": {"docs/reference/MCP_TOOLS.md"},
            "suggest_budget": {
                "crates/velesdb-node/src/lib.rs",
                "docs/reference/MCP_TOOLS.md",
            },
        }
        pinned = {
            tool.name: set(tool.pinned_surfaces) for tool in cmdc.POLICED_TOOLS
        }
        self.assertEqual(pinned, expected)

    def test_the_session_hooks_are_swept(self) -> None:
        # The prompt text injected into EVERY Claude Code / Codex / Windsurf
        # session — the surface a model reads most often, and the one the
        # original globs (*.md, *.py, *.ts) could not see at all.
        swept = {cmdc.rel(REPO_ROOT, path) for path in cmdc.surface_files(REPO_ROOT)}
        for hook in (
            "integrations/agent-hooks/claude-code/hooks/session-start.sh",
            "integrations/agent-hooks/codex/hooks/session-start.sh",
            "integrations/agent-hooks/windsurf/hooks/pre-user-prompt.sh",
        ):
            with self.subTest(hook=hook):
                self.assertIn(hook, swept)

    def test_the_node_ts_return_type_declaration_is_read(self) -> None:
        # `#[napi(ts_return_type = "Promise<{ … }>")]` IS the .d.ts every npm
        # consumer compiles against. binding_parity_bdd.rs reads this region
        # but treats the string as evidence of a relay and never checks its
        # keys — its own header says so.
        path = REPO_ROOT / "crates/velesdb-node/src/lib.rs"
        raw = path.read_text(encoding="utf-8")
        index = cmdc.build_alias_index(
            sorted(cmdc.load_output_schema_keys(REPO_ROOT)), cmdc.POLICED_TOOLS
        )
        text = cmdc.mask_jsdoc_links(raw)
        declared = [
            sorted(set(keys))
            for _offset, keys in cmdc.find_declarations(
                text,
                "load_working_context",
                cmdc.alias_positions(text, index),
                cmdc.section_positions(text, set(index.values())),
            )
        ]
        self.assertIn(["found", "other_sessions", "working"], declared)
        self.assertIn("ts_return_type", raw)


if __name__ == "__main__":
    unittest.main()
