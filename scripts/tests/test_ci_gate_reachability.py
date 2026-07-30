"""`CI Success` must actually depend on the jobs it lists.

The final ``ci-success`` job in ``.github/workflows/ci.yml`` gates merges
twice over, and the two halves are independent:

  * ``needs: [...]`` — makes the job RUN before ci-success and makes its
    status visible;
  * the ``[[ "${{ needs.X.result }}" == "success" ]]`` chain — makes a
    FAILURE of that job fail ci-success.

A job in ``needs`` but absent from the chain reports its red status on the PR
page and blocks nothing: ``if: always()`` means ci-success still runs, and an
unchecked result is simply never read. That is a gate you can watch fail while
the merge button stays green, and it is the same class of defect as
``doc-contract.yml`` never being in ``needs`` at all.

So the invariant is mechanical, not a checklist: every job in ``needs`` is
either read by the chain or listed in ``CHAIN_EXEMPT`` with its reason.

Deliberately regex-based, not PyYAML: ``gate-contracts.yml`` runs these
suites with a bare ``actions/setup-python`` and installs nothing, so a
third-party import here would be an ImportError in CI — a guard that cannot
run. The parsers are unit-tested RED-then-GREEN on synthetic workflow text
below before being pointed at the real file.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"

# Jobs allowed in `needs` without being read by the chain, and why. Keep this
# as short as it is: each entry is a job whose failure cannot block a merge.
CHAIN_EXEMPT = {
    # External service, skipped on forks and token-less runs. The comment
    # above the chain in ci.yml says the same thing.
    "sonarcloud",
}

JOB_KEY_RE = re.compile(r"^  ([a-z0-9][a-z0-9-]*):$", re.MULTILINE)
CI_SUCCESS_NEEDS_RE = re.compile(r"^  ci-success:$.*?^    needs:\s*\[([^\]]*)\]", re.MULTILINE | re.DOTALL)
CHAIN_RESULT_RE = re.compile(r"needs\.([a-z0-9][a-z0-9-]*)\.result")
# The whole comparison, not just the job name: `== "success"` weakened to
# `!= "cancelled"` reads every result and blocks nothing.
CHAIN_TEST_RE = re.compile(
    r"needs\.([a-z0-9][a-z0-9-]*)\.result\s*\}\}\"\s*(==|!=)\s*\"([a-z]+)\""
)
COMMENT_LINE_RE = re.compile(r"^\s*#", re.MULTILINE)
GUARD_INVOCATION_RE = re.compile(r"^\s*run:\s*(python3?\s+scripts/[^\n]*)$", re.MULTILINE)


def strip_comments(text: str) -> str:
    """Drop whole-line YAML comments, preserving line count.

    Load-bearing: a regex over the raw text finds `needs.X.result` inside a
    COMMENTED-OUT chain line just as happily as in a live one, so commenting
    the line out was a way to disarm the gate with every test still green.
    """
    return "\n".join(
        "" if COMMENT_LINE_RE.match(line) else line for line in text.split("\n")
    )


def job_names(text: str) -> "set[str]":
    """Every top-level job key declared in the workflow."""
    return set(JOB_KEY_RE.findall(text))


def ci_success_needs(text: str) -> "list[str]":
    """The `needs:` list of the final ci-success job, in declaration order."""
    match = CI_SUCCESS_NEEDS_RE.search(strip_comments(text))
    if match is None:
        raise AssertionError("no `ci-success:` job with an inline `needs: [...]` list found")
    return [name.strip() for name in match.group(1).split(",") if name.strip()]


def _chain_text(text: str) -> str:
    stripped = strip_comments(text)
    match = CI_SUCCESS_NEEDS_RE.search(stripped)
    if match is None:  # pragma: no cover - covered by ci_success_needs
        raise AssertionError("no `ci-success:` job found")
    return stripped[match.end():]


def chain_checked_jobs(text: str) -> "set[str]":
    """Jobs whose `.result` the LIVE `[[ ... ]]` chain reads."""
    return set(CHAIN_RESULT_RE.findall(_chain_text(text)))


def chain_comparisons(text: str) -> "list[tuple[str, str, str]]":
    """Every `(job, operator, expected)` triple the chain tests."""
    return CHAIN_TEST_RE.findall(_chain_text(text))


def chain_failure_branch(text: str) -> str:
    """The `|| { … }` tail of the chain — what runs when a job is not green."""
    chain = _chain_text(text)
    marker = chain.find("||")
    if marker == -1:
        raise AssertionError("the ci-success chain has no `|| …` failure branch")
    return chain[marker:chain.find("\n", marker)]


def job_block(text: str, job: str) -> str:
    """The YAML block of one job, up to the next top-level job key."""
    start = text.find(f"\n  {job}:\n")
    if start == -1:
        raise AssertionError(f"no `{job}:` job found")
    rest = text[start + 1:]
    following = JOB_KEY_RE.search(rest[len(job) + 4:])
    return rest if following is None else rest[: len(job) + 4 + following.start()]


def guard_invocations(text: str, script: str) -> "list[str]":
    """Every `run:` line invoking ``script``."""
    return [line for line in GUARD_INVOCATION_RE.findall(text) if script in line]


SYNTHETIC = """\
jobs:
  lint:
    name: Lint
  openapi-drift:
    name: OpenAPI Drift Check
  mcp-doc-contract:
    name: MCP Doc Contract
  ci-success:
    name: CI Success
    needs: [lint, openapi-drift, mcp-doc-contract]
    if: always()
    steps:
      - name: Check results
        run: |
          [[ "${{ needs.lint.result }}" == "success" ]] && \\
          [[ "${{ needs.openapi-drift.result }}" == "success" ]] && \\
          [[ "${{ needs.mcp-doc-contract.result }}" == "success" ]] && \\
          echo "ok" || { echo "ko"; exit 1; }
"""


class ParserTests(unittest.TestCase):
    """The parsers, pinned on synthetic workflow text."""

    def test_job_names(self) -> None:
        self.assertEqual(
            job_names(SYNTHETIC),
            {"lint", "openapi-drift", "mcp-doc-contract", "ci-success"},
        )

    def test_needs_and_chain_agree_on_a_well_wired_workflow(self) -> None:
        self.assertEqual(
            ci_success_needs(SYNTHETIC), ["lint", "openapi-drift", "mcp-doc-contract"]
        )
        self.assertEqual(
            chain_checked_jobs(SYNTHETIC), {"lint", "openapi-drift", "mcp-doc-contract"}
        )

    def test_a_job_in_needs_but_not_in_the_chain_is_detected(self) -> None:
        # The exact half-wiring this suite exists to refuse.
        broken = SYNTHETIC.replace(
            '          [[ "${{ needs.mcp-doc-contract.result }}" == "success" ]] && \\\n', ""
        )
        self.assertIn("mcp-doc-contract", ci_success_needs(broken))
        self.assertNotIn("mcp-doc-contract", chain_checked_jobs(broken))

    def test_a_job_dropped_from_needs_is_detected(self) -> None:
        broken = SYNTHETIC.replace("needs: [lint, openapi-drift, mcp-doc-contract]", "needs: [lint]")
        self.assertNotIn("mcp-doc-contract", ci_success_needs(broken))

    def test_a_chain_entry_before_the_needs_list_is_not_counted(self) -> None:
        # chain_checked_jobs reads only what FOLLOWS the needs list, so an
        # earlier job mentioning `needs.X.result` cannot fake coverage.
        polluted = SYNTHETIC.replace(
            "  ci-success:", '  decoy:\n    run: echo "${{ needs.ghost.result }}"\n  ci-success:'
        )
        self.assertNotIn("ghost", chain_checked_jobs(polluted))

    def test_a_workflow_without_ci_success_raises(self) -> None:
        with self.assertRaises(AssertionError):
            ci_success_needs("jobs:\n  lint:\n    name: Lint\n")


class RealWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.text = CI_WORKFLOW.read_text(encoding="utf-8")
        self.needs = ci_success_needs(self.text)
        self.chain = chain_checked_jobs(self.text)

    def test_every_needed_job_is_checked_by_the_chain(self) -> None:
        unchecked = sorted(set(self.needs) - self.chain - CHAIN_EXEMPT)
        self.assertEqual(
            unchecked,
            [],
            f"job(s) in `CI Success`'s needs whose result nothing reads: {unchecked}. "
            "Add `[[ \"${{ needs.<job>.result }}\" == \"success\" ]] && \\` to the chain, "
            "or document the job in CHAIN_EXEMPT.",
        )

    def test_the_chain_never_reads_a_job_that_is_not_needed(self) -> None:
        # `needs.X.result` for an X outside `needs` evaluates to empty, so the
        # chain would fail forever.
        phantom = sorted(self.chain - set(self.needs))
        self.assertEqual(phantom, [], f"chain reads job(s) absent from needs: {phantom}")

    def test_every_needed_job_is_actually_declared(self) -> None:
        declared = job_names(self.text)
        missing = sorted(set(self.needs) - declared)
        self.assertEqual(missing, [], f"needs names undeclared job(s): {missing}")

    def test_the_mcp_doc_contract_gate_is_required(self) -> None:
        # Named explicitly: this suite's generic invariant would stay green if
        # the job were removed from `needs` AND from the chain together.
        self.assertIn("mcp-doc-contract", job_names(self.text))
        self.assertIn("mcp-doc-contract", self.needs)
        self.assertIn("mcp-doc-contract", self.chain)

    def test_the_needs_list_is_not_empty(self) -> None:
        self.assertTrue(self.needs, "`CI Success` needs nothing — it gates nothing")


class BlockingBehaviourTests(unittest.TestCase):
    """Presence is not blocking, and only blocking blocks.

    Everything above proves the job is WIRED. None of it proved that a
    failure of the job fails anything — and five one-word edits were measured
    to leave the whole suite green: commenting the chain line out, weakening
    `== "success"` to `!= "cancelled"`, replacing the `|| { …; exit 1; }` tail
    with `|| true`, adding `continue-on-error: true` to the job, and passing
    `--mode warn` to the guard so it reports and exits 0.

    That last one is not hypothetical: `scripts/check-doc-contract.sh` says in
    its own header that four routes entered this repository undocumented
    "while the sweep was disarmed" under `DOC_CONTRACT_MODE=warn`. The mode
    was never taken back out. So the invocation line is pinned too.
    """

    def setUp(self) -> None:
        self.text = CI_WORKFLOW.read_text(encoding="utf-8")

    def test_every_chain_entry_demands_success_exactly(self) -> None:
        weak = [
            (job, operator, expected)
            for job, operator, expected in chain_comparisons(self.text)
            if (operator, expected) != ("==", "success")
        ]
        self.assertEqual(weak, [], f"chain entries that do not demand success: {weak}")

    def test_the_chain_covers_every_entry_it_reads(self) -> None:
        # chain_comparisons is stricter than chain_checked_jobs; if one sees a
        # job the other does not, the comparison regex has drifted.
        self.assertEqual(
            {job for job, _op, _expected in chain_comparisons(self.text)},
            chain_checked_jobs(self.text),
        )

    def test_the_failure_branch_exits_non_zero(self) -> None:
        branch = chain_failure_branch(self.text)
        self.assertIn("exit 1", branch, f"the chain's failure branch cannot fail: {branch!r}")

    def test_the_gate_job_cannot_opt_out_of_blocking(self) -> None:
        # Comment-stripped: the job's own comment NAMES `continue-on-error`
        # to explain why it must not be there, and a raw substring search
        # read that prose as the setting. Caught by this very test.
        block = strip_comments(job_block(self.text, "mcp-doc-contract"))
        self.assertNotIn("continue-on-error", block, "the gate job can be made non-blocking")
        self.assertNotRegex(block, r"(?m)^    if:", "a job-level `if:` can skip the gate")

    def test_every_python_suite_of_this_change_runs_in_the_required_job(self) -> None:
        # gate-contracts.yml's `unittest discover` picks these up too, but it
        # is NOT in `CI Success`'s needs, and `CI Success` is the only
        # required check on develop — so a suite reached only from there is a
        # suite whose red does not block. Shipping one inside a change whose
        # thesis is "an unrequired gate protects nothing" would be the same
        # mistake, one level down.
        block = job_block(self.text, "mcp-doc-contract")
        for suite in (
            "scripts.tests.test_check_mcp_doc_contract",
            "scripts.tests.test_ci_gate_reachability",
            "scripts.tests.test_skill_copies_are_identical",
        ):
            with self.subTest(suite=suite):
                self.assertIn(suite, block)

    def test_the_guard_is_invoked_in_strict_mode(self) -> None:
        invocations = guard_invocations(self.text, "check-mcp-doc-contract.py")
        self.assertTrue(invocations, "ci.yml never runs the MCP doc-contract guard")
        for line in invocations:
            with self.subTest(line=line):
                self.assertNotIn("--mode warn", line)
                self.assertNotIn("|| true", line)
                self.assertNotIn("continue-on-error", line)


class DisarmTests(unittest.TestCase):
    """Each disarm above, replayed on the REAL workflow text, must be RED."""

    def setUp(self) -> None:
        self.text = CI_WORKFLOW.read_text(encoding="utf-8")

    def _chain_line(self) -> str:
        line = '          [[ "${{ needs.mcp-doc-contract.result }}" == "success" ]] && \\'
        self.assertIn(line, self.text, "the chain line changed shape — update this test")
        return line

    def test_commenting_the_chain_line_out_is_detected(self) -> None:
        line = self._chain_line()
        broken = self.text.replace(line, "          # " + line.strip())
        self.assertIn("mcp-doc-contract", ci_success_needs(broken))
        self.assertNotIn("mcp-doc-contract", chain_checked_jobs(broken))

    def test_weakening_the_comparison_is_detected(self) -> None:
        line = self._chain_line()
        broken = self.text.replace(line, line.replace('== "success"', '!= "cancelled"'))
        weak = [c for c in chain_comparisons(broken) if c[1:] != ("==", "success")]
        self.assertEqual(weak, [("mcp-doc-contract", "!=", "cancelled")])

    def test_neutering_the_failure_branch_is_detected(self) -> None:
        broken = self.text.replace('|| { echo "❌ CI failed"; exit 1; }', "|| true")
        self.assertNotIn("exit 1", chain_failure_branch(broken))

    def test_continue_on_error_on_the_gate_job_is_detected(self) -> None:
        broken = self.text.replace(
            "  mcp-doc-contract:\n    name: MCP Doc Contract\n",
            "  mcp-doc-contract:\n    name: MCP Doc Contract\n    continue-on-error: true\n",
        )
        self.assertIn("continue-on-error", job_block(broken, "mcp-doc-contract"))

    def test_warn_mode_on_the_invocation_is_detected(self) -> None:
        invocation = guard_invocations(self.text, "check-mcp-doc-contract.py")[0]
        broken = self.text.replace(invocation, invocation + " --mode warn")
        self.assertIn(
            "--mode warn", guard_invocations(broken, "check-mcp-doc-contract.py")[0]
        )


if __name__ == "__main__":
    unittest.main()
