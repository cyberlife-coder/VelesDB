"""Tests for scripts/bench-memory-extraction.py — above all, its refusals.

A bench reports a verdict on models. If its scorer cannot fail, every model
passes and the campaign publishes a number that means nothing. So each severity
has a vector here that MUST produce it, next to a positive control that must
produce nothing.

Three of these tests exist because the code they cover was already wrong, and
only running it showed that:

  * **The prompt parse stopped two thirds of the way in.** A lazy
    `format!\\(\\s*"(.*?)"\\s*\\)` ends on the `")` inside the prompt's own
    example, `(e.g. \\"bruno durand\\")`. It produced 904 characters of a
    2,696-character prompt, and the sanity guard missed it because every marker
    it checked — the passage, `STEP 0`, `"relations"` — appears in the surviving
    head. Guards now check the TAIL, and `test_prompt_is_whole` pins the closing
    JSON contract.
  * **The SSE decoder read the priming frame.** The daemon opens its stream with
    `data: ` carrying an empty payload before the reply, so taking the first
    `data:` line dies on empty input. Every frame is tried now.
  * **`memory_status` is not everywhere.** The daemon installed on this machine
    on 2026-08-15 exposes 20 tools and not that one — its binary predates it.
    Phase B calls it, so `probe` checks the tool list against the server instead
    of assuming, and reports "tool not found" as a missing capability rather
    than as a measurement.

The fixtures below are written by hand, which proves the scorer REACTS but not
that it reacts to what models produce. The campaign's raw answers are kept for
exactly that reason: once one has run, these fixtures get replaced by captured
ones.
"""

from __future__ import annotations

import importlib.util
import json
import math
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "bench-memory-extraction.py"
CASES_PATH = Path(__file__).resolve().parent.parent / "memory-extraction-cases.json"


def load_module():
    spec = importlib.util.spec_from_file_location("bench_memory_extraction", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


bench = load_module()


def case_by_id(case_id: str) -> dict:
    for case in bench.load_cases(CASES_PATH):
        if case["id"] == case_id:
            return case
    raise AssertionError(f"no case {case_id!r}")


def severities(failures: "list[dict]") -> "list[str]":
    return [failure["severity"] for failure in failures]


# ------------------------------------------------------------------- folding --


class FoldTest(unittest.TestCase):
    def test_ligature_expands_instead_of_vanishing(self):
        """NFKD does not decompose U+0153: the ASCII pass would drop it entirely.

        Without the explicit mapping `soeur` folds to `sur`, and every kinship
        check stops matching while still reporting success.
        """
        self.assertEqual(bench.fold("sœur"), "soeur")
        self.assertEqual(bench.fold("belle-sœur"), "belle-soeur")
        self.assertEqual(bench.fold("œuvre"), "oeuvre")
        self.assertEqual(bench.fold("Ex æquo"), "ex aequo")

    def test_folds_case_and_accents(self):
        self.assertEqual(bench.fold("Éco-Kérité"), "eco-kerite")
        self.assertEqual(bench.fold("Marie DUPONT"), "marie dupont")

    def test_folds_non_strings(self):
        self.assertEqual(bench.fold(15), "15")
        self.assertEqual(bench.fold(None), "none")


# ------------------------------------------------- the prompt, read from Rust --


class PromptSourcingTest(unittest.TestCase):
    def test_prompt_is_whole(self):
        """The parse must reach the closing JSON contract, not stop at an escaped quote."""
        template = bench.read_graph_prompt_template()
        prompt = bench.build_graph_prompt("Marie Dupont a une soeur, Camille Dupont.", template)
        self.assertIn("Marie Dupont a une soeur", prompt)
        self.assertIn('"attributes"', prompt)
        self.assertIn('"entity": string', prompt)
        self.assertTrue(prompt.rstrip().endswith("}"), prompt[-80:])
        # The truncated parse produced 904 characters; the whole prompt is ~2.7k.
        self.assertGreater(len(prompt), 2000)

    def test_braces_collapse_but_passage_braces_survive(self):
        template = bench.read_graph_prompt_template()
        prompt = bench.build_graph_prompt('Projet "Ardoise {beta}" chez Wiscale.', template)
        self.assertIn("Ardoise {beta}", prompt)
        self.assertIn('{"facts"', prompt)

    def test_refuses_a_truncated_template(self):
        """A template cut before the contract must raise, never be sent."""
        with self.assertRaises(RuntimeError):
            bench.build_graph_prompt("passage", "STEP 0 and \"relations\" but cut here")

    def test_refuses_a_source_without_the_function(self):
        with tempfile.TemporaryDirectory() as tmp:
            empty = Path(tmp) / "extract.rs"
            empty.write_text("fn other() {}\n", encoding="utf-8")
            with self.assertRaises(RuntimeError):
                bench.read_graph_prompt_template(empty)
            with self.assertRaises(RuntimeError):
                bench.read_generation_cap(empty)

    def test_generation_cap_matches_the_crate(self):
        self.assertEqual(bench.read_generation_cap(), 512)


# ------------------------------------------------------- scorer: the control --


GOOD_POSSESSIVE = {
    "facts": [{"fact": "Camille Dupont a 15 ans.", "entities": ["camille dupont"]}],
    "relations": [{"subject": "camille dupont", "predicate": "soeur de", "object": "marie dupont"}],
    "attributes": [
        {"entity": "camille dupont", "key": "age", "value": 15},
        {"entity": "camille dupont", "key": "employeur", "value": "Wiscale"},
    ],
}


class PositiveControlTest(unittest.TestCase):
    def test_a_correct_answer_scores_nothing(self):
        """Without this, a scorer that never fires would look like a strict one."""
        case = case_by_id("fr-possessive")
        failures = bench.score_passage(GOOD_POSSESSIVE, case["passages"][0]["checks"])
        self.assertEqual(failures, [], failures)

    def test_a_correct_empty_answer_scores_nothing(self):
        case = case_by_id("edge-no-relation")
        payload = {"facts": [{"fact": "Il pleuvait hier soir.", "entities": []}],
                   "relations": [], "attributes": []}
        self.assertEqual(bench.score_passage(payload, case["passages"][0]["checks"]), [])


# --------------------------------------------------- scorer: refusal vectors --


class RefusalVectorTest(unittest.TestCase):
    def test_relations_as_arrays_are_fatal(self):
        """The silent one: this JSON parses, RawRelation refuses it, enrichment vanishes."""
        payload = {"relations": [["camille dupont", "soeur de", "marie dupont"]]}
        failures = bench.score_passage(payload, case_by_id("fr-possessive")["passages"][0]["checks"])
        self.assertEqual(severities(failures), ["fatal"])
        self.assertEqual(failures[0]["type"], "schema")

    def test_unparsable_response_is_one_fatal_not_a_cascade(self):
        failures = bench.score_passage(None, case_by_id("fr-possessive")["passages"][0]["checks"])
        self.assertEqual(len(failures), 1)
        self.assertEqual(failures[0]["type"], "parse")

    def test_reversed_orientation_is_major(self):
        payload = {**GOOD_POSSESSIVE, "relations": [
            {"subject": "marie dupont", "predicate": "soeur de", "object": "camille dupont"},
        ]}
        failures = bench.score_passage(payload, case_by_id("fr-possessive")["passages"][0]["checks"])
        labels = [failure["label"] for failure in failures]
        self.assertIn("sibling edge missing or reversed", labels)
        self.assertIn("parasitic converse: both directions of one predicate", labels)

    def test_wrong_language_predicate_is_major(self):
        payload = {**GOOD_POSSESSIVE, "relations": [
            {"subject": "camille dupont", "predicate": "sister of", "object": "marie dupont"},
        ]}
        failures = bench.score_passage(payload, case_by_id("fr-possessive")["passages"][0]["checks"])
        self.assertIn("English predicate on a French passage",
                      [failure["label"] for failure in failures])

    def test_number_as_string_is_major(self):
        payload = {**GOOD_POSSESSIVE,
                   "attributes": [{"entity": "camille dupont", "key": "age", "value": "15"}]}
        failures = bench.score_passage(payload, case_by_id("fr-possessive")["passages"][0]["checks"])
        self.assertIn("age missing, misattributed, or emitted as a string",
                      [failure["label"] for failure in failures])

    def test_boolean_is_not_a_number(self):
        """`True` is an int in Python; an age of `true` must not pass as 15."""
        payload = {**GOOD_POSSESSIVE,
                   "attributes": [{"entity": "camille dupont", "key": "age", "value": True}]}
        failures = bench.score_passage(payload, case_by_id("fr-possessive")["passages"][0]["checks"])
        self.assertTrue(failures)

    def test_invented_relation_is_fatal(self):
        payload = {"facts": [], "relations": [
            {"subject": "la ville", "predicate": "a eu", "object": "pluie"},
        ], "attributes": []}
        failures = bench.score_passage(payload, case_by_id("edge-no-relation")["passages"][0]["checks"])
        self.assertEqual(severities(failures), ["fatal"])

    def test_truncation_is_fatal_and_comes_from_the_server(self):
        case = case_by_id("edge-verbose-truncation")
        payload = {"facts": [], "relations": [
            {"subject": "a", "predicate": "p", "object": "b"},
            {"subject": "c", "predicate": "p", "object": "d"},
            {"subject": "e", "predicate": "p", "object": "f"},
        ], "attributes": []}
        clean = bench.score_passage(payload, case["passages"][0]["checks"], truncated=False)
        cut = bench.score_passage(payload, case["passages"][0]["checks"], truncated=True)
        self.assertEqual(clean, [])
        self.assertEqual(severities(cut), ["fatal"])

    def test_cross_contamination_is_fatal(self):
        case = case_by_id("close-homonyms")
        payload = {"facts": [], "relations": [], "attributes": [
            {"entity": "marie dupont", "key": "ville", "value": "Lyon"},
            {"entity": "marie dupont", "key": "ville", "value": "Nantes"},
            {"entity": "marie dupond", "key": "ville", "value": "Nantes"},
        ]}
        failures = bench.score_passage(payload, case["passages"][0]["checks"])
        self.assertIn("fatal", severities(failures))

    def test_pronoun_left_unresolved_is_major(self):
        case = case_by_id("fr-pronoun")
        payload = {"facts": [{"fact": "Il habite a Lyon.", "entities": []}],
                   "relations": [], "attributes": [
                       {"entity": "bruno durand", "key": "ville", "value": "Lyon"}]}
        failures = bench.score_passage(payload, case["passages"][0]["checks"])
        self.assertIn("pronoun left unresolved in a fact meant to stand alone",
                      [failure["label"] for failure in failures])

    def test_both_directions_of_one_predicate_is_major(self):
        case = case_by_id("en-possessive")
        payload = {"facts": [], "relations": [
            {"subject": "tom miller", "predicate": "brother of", "object": "sarah miller"},
            {"subject": "sarah miller", "predicate": "brother of", "object": "tom miller"},
        ], "attributes": [{"entity": "tom miller", "key": "age", "value": 22}]}
        failures = bench.score_passage(payload, case["passages"][0]["checks"])
        self.assertIn("both directions over the same pair",
                      [failure["label"] for failure in failures])

    def test_unknown_check_type_raises(self):
        """A typo in the cases file must stop the campaign, not skip a check."""
        with self.assertRaises(RuntimeError):
            bench.score_passage({"relations": []}, [{"type": "nope", "severity": "major", "label": "x"}])


class CrossCheckTest(unittest.TestCase):
    def test_identical_predicates_across_a_close_pair_is_major(self):
        case = case_by_id("close-role")
        collapsed = {"relations": [
            {"subject": "alice martin", "predicate": "travaille chez", "object": "wiscale"}]}
        failures = bench.score_cross_checks([collapsed, collapsed], case["cross_checks"])
        self.assertEqual(severities(failures), ["major"])

    def test_distinct_predicates_pass(self):
        case = case_by_id("close-role")
        works = {"relations": [
            {"subject": "alice martin", "predicate": "travaille chez", "object": "wiscale"}]}
        leads = {"relations": [
            {"subject": "alice martin", "predicate": "dirige", "object": "wiscale"}]}
        self.assertEqual(bench.score_cross_checks([works, leads], case["cross_checks"]), [])

    def test_two_empty_answers_do_not_count_as_a_collapse(self):
        """Both empty is a different defect, already scored by the per-passage checks."""
        case = case_by_id("close-role")
        empty = {"relations": []}
        self.assertEqual(bench.score_cross_checks([empty, empty], case["cross_checks"]), [])


# ------------------------------------------------------ the same checks, graph --


class GraphViewTest(unittest.TestCase):
    def test_entity_response_becomes_scorable(self):
        profile = {
            "found": True, "name": "camille dupont",
            "attributes": {"age": 15},
            "relations": [{"predicate": "soeur de", "target": "Entity: marie dupont"}],
            "relations_in": [{"predicate": "employe", "target": "Entity: wiscale"}],
        }
        payload = bench.entity_as_payload("camille dupont", profile)
        triples = bench.relation_triples(payload)
        self.assertEqual(triples, [("camille dupont", "soeur de", "marie dupont")])
        self.assertEqual(payload["attributes"],
                         [{"entity": "camille dupont", "key": "age", "value": 15}])

    def test_incoming_edges_are_not_credited_to_this_entity(self):
        """An incoming edge belongs to its SOURCE; folding it in invents an edge."""
        profile = {"name": "wiscale", "attributes": {}, "relations": [],
                   "relations_in": [{"predicate": "travaille chez", "target": "Entity: alice martin"}]}
        payload = bench.entity_as_payload("wiscale", profile)
        self.assertEqual(payload["relations"], [])

    def test_graph_checks_keep_only_what_a_stored_entity_can_answer(self):
        case = case_by_id("fr-possessive")
        kept = {spec["type"] for spec in bench.graph_checks_for(case["passages"][0])}
        self.assertIn("relation_present", kept)
        self.assertIn("attribute_number", kept)
        # About the ANSWER, not the graph: scoring it here would double-count.
        self.assertNotIn("predicate_forbids", kept)

    def test_a_missing_entity_fails_its_graph_check(self):
        case = case_by_id("fr-possessive")
        empty = bench.entity_as_payload("camille dupont", {"found": False, "name": "camille dupont"})
        specs = bench.graph_checks_for(case["passages"][0])
        self.assertTrue(bench.score_passage(empty, specs))


# ---------------------------------------------------------------- transport --


class McpBodyTest(unittest.TestCase):
    def test_sse_priming_frame_is_skipped(self):
        """The exact body the daemon returned on 2026-08-15."""
        body = ('data: \nid: 0\nretry: 3000\n\n'
                'data: {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}\n\n')
        decoded = bench.decode_mcp_body(body)
        self.assertEqual(decoded["result"]["protocolVersion"], "2025-06-18")

    def test_plain_json_body(self):
        self.assertEqual(bench.decode_mcp_body('{"jsonrpc":"2.0","id":1}')["id"], 1)

    def test_empty_body_is_none(self):
        self.assertIsNone(bench.decode_mcp_body("   "))

    def test_sse_without_any_payload_is_none(self):
        self.assertIsNone(bench.decode_mcp_body("data: \nid: 0\nretry: 3000\n\n"))


class ToolPayloadTest(unittest.TestCase):
    def test_refusal_inside_a_valid_result_is_not_a_success(self):
        """`isError` rides INSIDE a well-formed result; reading the outer reply lies."""
        refused = {"response": {"result": {"isError": True, "content": [
            {"type": "text", "text": "refused"}]}}}
        self.assertIsNone(bench.tool_payload(refused))

    def test_text_content_is_parsed_as_json(self):
        ok = {"response": {"result": {"content": [{"type": "text", "text": '{"found": true}'}]}}}
        self.assertEqual(bench.tool_payload(ok), {"found": True})

    def test_non_json_text_is_kept_verbatim(self):
        ok = {"response": {"result": {"content": [{"type": "text", "text": "plain"}]}}}
        self.assertEqual(bench.tool_payload(ok), {"text": "plain"})


# ------------------------------------------------------------ log digestion --


SAMPLE_LOG = """\
2026-08-15T09:23:21.000000Z  INFO velesdb_memory::mcp: tool=remember session=abc verdict=ok elapsed_ms=132033 "mcp tool call"
2026-08-15T09:24:00.000000Z  INFO velesdb_memory::mcp: tool=recall session=abc verdict=ok elapsed_ms=128 "mcp tool call"
2026-08-15T09:24:01.000000Z  INFO velesdb_memory::mcp: tool=recall session=abc verdict=tool_error elapsed_ms=5 "mcp tool call"
2026-08-15T09:24:02.000000Z  INFO velesdb_memory::http: mcp http request method=POST session=abc status=200 elapsed_ms=2
2026-08-14T23:40:41.000000Z  INFO velesdb_memory::mcp: tool=remember session=abc verdict=ok elapsed_ms=300403 "mcp tool call"
"""


class DigestTest(unittest.TestCase):
    def digest(self, since=None):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "daemon.err.log"
            path.write_text(SAMPLE_LOG, encoding="utf-8")
            return bench.digest_log(path, since)

    def test_tool_and_http_events_are_not_mixed(self):
        """Both carry `elapsed_ms`; averaging them buries a 132-second write."""
        digest = self.digest()
        self.assertEqual(digest["tools"]["remember"]["n"], 2)
        self.assertEqual(digest["http_post"]["n"], 1)
        self.assertNotIn("mcp", digest["tools"])

    def test_verdicts_separate_refusals_from_successes(self):
        digest = self.digest()
        self.assertEqual(digest["verdicts"]["recall:tool_error"], 1)
        self.assertEqual(digest["verdicts"]["recall:ok"], 1)

    def test_since_filters_by_timestamp(self):
        digest = self.digest(since="2026-08-15T00")
        self.assertEqual(digest["tools"]["remember"]["n"], 1)
        self.assertEqual(digest["tools"]["remember"]["max_ms"], 132033)


class PercentileTest(unittest.TestCase):
    def test_nearest_rank(self):
        values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        self.assertEqual(bench.percentile(values, 0.50), 5)
        self.assertEqual(bench.percentile(values, 0.95), 10)
        self.assertEqual(bench.percentile([7], 0.95), 7)

    def test_empty_is_nan_not_a_crash(self):
        self.assertTrue(math.isnan(bench.percentile([], 0.5)))


# ------------------------------------------------------- the cases file itself --


class CasesFileTest(unittest.TestCase):
    """The cases are data, so their integrity is a test, not a runtime surprise.

    A mistyped check type raises mid-campaign otherwise — after the model has
    been loaded, warmed and half-scored.
    """

    def setUp(self):
        self.cases = bench.load_cases(CASES_PATH)

    def test_every_check_type_is_implemented(self):
        for case in self.cases:
            for passage in case["passages"]:
                for spec in passage["checks"]:
                    self.assertIn(spec["type"], bench.CHECKS, f"{case['id']}: {spec['type']}")
            for spec in case.get("cross_checks") or []:
                self.assertEqual(spec["type"], "predicates_differ", case["id"])

    def test_every_check_declares_a_known_severity_and_a_label(self):
        for case in self.cases:
            specs = [s for p in case["passages"] for s in p["checks"]]
            specs += case.get("cross_checks") or []
            for spec in specs:
                self.assertIn(spec["severity"], bench.SEVERITIES, case["id"])
                self.assertTrue(spec["label"].strip(), case["id"])

    def test_the_suite_covers_every_family_the_plan_names(self):
        families = {case["family"] for case in self.cases}
        self.assertEqual(families, {"nominal-fr", "nominal-en", "edge", "close-pair"})

    def test_french_and_english_nominals_mirror_each_other(self):
        """A model passing one language and failing the other must be visible."""
        fr = sum(1 for case in self.cases if case["family"] == "nominal-fr")
        en = sum(1 for case in self.cases if case["family"] == "nominal-en")
        self.assertEqual(fr, en)

    def test_close_pairs_carry_two_passages_and_a_cross_check(self):
        pairs = [case for case in self.cases
                 if case["family"] == "close-pair" and len(case["passages"]) == 2]
        self.assertTrue(pairs)
        for case in pairs:
            self.assertTrue(case.get("cross_checks"), case["id"])

    def test_case_ids_are_unique(self):
        ids = [case["id"] for case in self.cases]
        self.assertEqual(len(ids), len(set(ids)))

    def test_every_passage_builds_a_valid_prompt(self):
        """Cheap, and it proves no passage breaks the format! substitution."""
        template = bench.read_graph_prompt_template()
        for case in self.cases:
            for passage in case["passages"]:
                prompt = bench.build_graph_prompt(passage["text"], template)
                self.assertIn(passage["text"], prompt)


# ---------------------------------------------------------------- language ----


def scored_case(case_id, family, lang, fatal=0, major=0, seconds=1.0):
    """A minimal scored case, shaped as `screen_case` returns one."""
    failures = ([{"severity": "fatal", "label": "f", "type": "t"}] * fatal
                + [{"severity": "major", "label": "m", "type": "t"}] * major)
    return {
        "id": case_id, "family": family, "lang": lang,
        "passages": [{"seconds": seconds, "parse_ok": True, "truncated": False,
                      "failures": failures}],
        "cross_failures": [], "counts": bench.tally(failures),
    }


class LanguageVerdictTest(unittest.TestCase):
    """The verdict a global score cannot give.

    velesdb-memory is used in whatever language its user writes in, and the
    extractor model is theirs to choose. A model strong in English and weak in
    French does not merely score slightly lower — `works at` and `travaille
    chez` become two graph predicates for one relation, and the graph fragments.
    """

    def test_an_english_only_failure_names_english_as_weaker(self):
        results = [
            scored_case("fr1", "nominal-fr", "fr"),
            scored_case("fr2", "nominal-fr", "fr"),
            scored_case("en1", "nominal-en", "en", major=3),
            scored_case("en2", "nominal-en", "en"),
        ]
        gap = bench.mirror_gap(results)
        self.assertEqual(gap["weaker"], "en")
        self.assertEqual(gap["gap"], 3)

    def test_a_fatal_outweighs_majors_in_the_gap(self):
        """One fatal is not three majors: it means the graph is wrong, not poorer."""
        results = [
            scored_case("fr1", "nominal-fr", "fr", fatal=1),
            scored_case("en1", "nominal-en", "en", major=5),
        ]
        gap = bench.mirror_gap(results)
        self.assertEqual(gap["weaker"], "fr")

    def test_a_balanced_model_names_no_weaker_side(self):
        results = [
            scored_case("fr1", "nominal-fr", "fr", major=1),
            scored_case("en1", "nominal-en", "en", major=1),
        ]
        self.assertIsNone(bench.mirror_gap(results)["weaker"])

    def test_the_gap_ignores_the_french_only_families(self):
        """The suite is unbalanced on purpose; only the mirrors are comparable.

        Edge and close-pair cases are French, so counting them would report
        every model as 'weaker in French' regardless of what it did.
        """
        results = [
            scored_case("fr1", "nominal-fr", "fr"),
            scored_case("en1", "nominal-en", "en"),
            scored_case("edge1", "edge", "fr", fatal=2),
            scored_case("close1", "close-pair", "fr", major=4),
        ]
        gap = bench.mirror_gap(results)
        self.assertEqual(gap["gap"], 0)
        self.assertIsNone(gap["weaker"])

    def test_by_language_still_reports_both_sides(self):
        results = [scored_case("fr1", "nominal-fr", "fr", major=2),
                   scored_case("en1", "nominal-en", "en")]
        by_language = bench.totals_by_language(results)
        self.assertEqual(by_language["fr"]["major"], 2)
        self.assertEqual(by_language["en"]["major"], 0)

    def test_the_report_carries_the_language_table(self):
        results = {"campaign": "x", "environment": {}, "configurations": {
            "m": {"totals": {"fatal": 0, "major": 3, "minor": 0, "parse_rate": 1.0,
                             "truncated": 0, "p50_seconds": 1.0, "p95_seconds": 1.0},
                  "mirror_gap": bench.mirror_gap([
                      scored_case("fr1", "nominal-fr", "fr"),
                      scored_case("en1", "nominal-en", "en", major=3)])}}}
        rendered = bench.render_report(results)
        self.assertIn("Language symmetry", rendered)
        self.assertIn("| `m` |", rendered.split("Language symmetry")[1])
        self.assertIn("en", rendered.split("Language symmetry")[1])


class OtherLanguageTest(unittest.TestCase):
    """A user writing in neither French nor English must be able to decide too.

    The model is their choice (`VELESDB_MEMORY_EXTRACTOR_MODEL`); this suite
    answers for French and English, and `--cases` is what makes the same verdict
    reachable for any other language.
    """

    def test_an_alternate_cases_file_is_accepted(self):
        cases = {"version": 1, "cases": [{
            "id": "de-possessive", "family": "nominal-de", "lang": "de",
            "passages": [{"text": "Marie Dupont hat eine Schwester, Camille Dupont.",
                          "checks": [{"type": "relation_present",
                                      "subject": "camille dupont",
                                      "predicate_any": ["schwester"],
                                      "object": "marie dupont",
                                      "severity": "major", "label": "sibling edge missing"}]}],
        }]}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "de.json"
            path.write_text(json.dumps(cases), encoding="utf-8")
            loaded = bench.load_cases(path)
        self.assertEqual(loaded[0]["lang"], "de")
        payload = {"relations": [{"subject": "camille dupont", "predicate": "schwester von",
                                  "object": "marie dupont"}]}
        self.assertEqual(bench.score_passage(payload, loaded[0]["passages"][0]["checks"]), [])

    def test_a_suite_without_mirrors_reports_no_language_verdict(self):
        """No mirrored families means no comparison — and it must say so, not invent one."""
        gap = bench.mirror_gap([scored_case("de1", "nominal-de", "de", major=2)])
        self.assertIsNone(gap["weaker"])
        self.assertEqual(gap["gap"], 0)


# --------------------------------------------------------------- warm-up ----


class WarmUpTest(unittest.TestCase):
    def test_stability_needs_a_full_window(self):
        self.assertFalse(bench.is_stable([10.0, 10.0]))

    def test_settled_latencies_are_stable(self):
        self.assertTrue(bench.is_stable([50.0, 10.1, 10.0, 9.9]))

    def test_a_drifting_model_is_not_stable(self):
        self.assertFalse(bench.is_stable([10.0, 13.0, 17.0]))

    def test_warm_up_reports_failure_to_settle_instead_of_raising(self):
        """Not stabilising is a finding about the model, not a bench error."""

        class Drifting:
            def __init__(self):
                self.calls = 0

            def generate(self, _prompt):
                self.calls += 1
                return {"seconds": float(self.calls) * 3}

        trace = bench.warm_up(Drifting(), "prompt")
        self.assertFalse(trace["stabilised"])
        self.assertEqual(trace["rounds"], bench.WARMUP_MAX_ROUNDS)


# ----------------------------------------------------------------- report ----


class ReportTest(unittest.TestCase):
    def test_report_is_derived_from_the_results(self):
        results = {
            "campaign": "2026-08-15",
            "environment": {"machine": "M5 Pro", "daemon_mtime": "2026-08-15T10:00"},
            "configurations": {
                "default:fast": {
                    "totals": {"fatal": 0, "major": 1, "minor": 0, "parse_rate": 1.0,
                               "truncated": 0, "p50_seconds": 10.4, "p95_seconds": 11.2},
                    "cold": {"cold_total_seconds": 92.3},
                    "warmup": {"rounds": 3, "stabilised": True},
                },
            },
        }
        rendered = bench.render_report(results)
        self.assertIn("`default:fast`", rendered)
        self.assertIn("10.4s", rendered)
        self.assertIn("92.3s", rendered)
        self.assertIn("M5 Pro", rendered)

    def test_a_missing_cold_load_is_reported_not_faked(self):
        results = {"campaign": "x", "environment": {}, "configurations": {
            "m": {"totals": {"fatal": 0, "major": 0, "minor": 0, "parse_rate": 1.0,
                             "truncated": 0, "p50_seconds": 1.0, "p95_seconds": 1.0}}}}
        self.assertIn("n/a", bench.render_report(results))

    def test_an_unstable_warm_up_is_flagged_in_the_table(self):
        results = {"campaign": "x", "environment": {}, "configurations": {
            "m": {"totals": {"fatal": 0, "major": 0, "minor": 0, "parse_rate": 1.0,
                             "truncated": 0, "p50_seconds": 1.0, "p95_seconds": 1.0},
                  "warmup": {"rounds": 6, "stabilised": False}}}}
        self.assertIn("unstable", bench.render_report(results))

    def test_report_is_reproducible(self):
        """Same input, same bytes — the check that the table was not hand-edited."""
        results = json.loads(json.dumps({"campaign": "x", "environment": {"a": "b"},
                                         "configurations": {}}))
        self.assertEqual(bench.render_report(results), bench.render_report(results))


if __name__ == "__main__":
    unittest.main(verbosity=2)
