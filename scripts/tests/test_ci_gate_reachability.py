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

import json
import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
GUARDS_REGISTRY = REPO_ROOT / "scripts" / "guards.json"

# Which scripts count as guards for the exhaustiveness check below. Deliberately
# a name pattern over the filesystem and not a list: a list is the thing that
# was missing, so bootstrapping the fix from another list would reproduce the
# defect one level up. Adding `scripts/check-foo.py` makes this suite red until
# the registry accounts for it.
GUARD_SCRIPT_RE = re.compile(r"^scripts/[^/]*(?:check|verify|gate)[^/]*\.(?:py|sh)$")

# Tokens that turn a finding into a report. Only ever applied to entries whose
# declared mode is `strict` — a `warn` entry declares its own leniency and
# carries an `exit_condition` instead.
DISARM_TOKENS = ("--mode warn", "|| true", "continue-on-error")

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
    # Narrowed to the `run:` body first. Everything after ci-success's
    # `needs:` is in scope here, its `if:` included, and a job condition may
    # legitimately contain `||` — the retarget guard does. Scanning from the
    # shell block keeps this on the chain's own tail while still seeing a
    # branch neutered to something brace-less like `|| true`.
    run_at = chain.find("run:")
    if run_at == -1:
        raise AssertionError("the ci-success job has no `run:` chain")
    body = chain[run_at:]
    marker = body.find("||")
    if marker == -1:
        raise AssertionError("the ci-success chain has no `|| …` failure branch")
    return body[marker:body.find("\n", marker)]


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


# A gate scoped to "the files this PR changed" resolves that list with
# `git diff <base>..<head>`. On a pull request `actions/checkout` fetches only
# the merge commit unless told otherwise, so the base commit is simply absent
# and the diff errors out — inside `mapfile < <(...)`, where the failure is
# invisible and the list comes back empty. The gate then reports "no changed
# files to validate" and exits 0 on every pull request. That is the shape this
# module exists to refuse: a guard that runs, reports success, and reads
# nothing.
BASE_SHA_DIFF_RE = re.compile(r"git diff [^\n|]*\$(?:\{)?BASE_SHA")
CHECKOUT_RE = re.compile(r"uses:\s*actions/checkout@")
FETCH_DEPTH_RE = re.compile(r"^\s*fetch-depth:\s*0\s*$", re.MULTILINE)


BASE_REACHABLE_RE = re.compile(r"git cat-file -e \"\$BASE_SHA\^\{commit\}\"")


def asserts_base_is_reachable(block: str) -> bool:
    """Whether a job proves the base commit exists before diffing against it.

    `fetch-depth: 0` is the fix; this is the alarm for every other way the
    base can go missing (a force-push that orphans it, a rewritten base
    branch). Without it the diff fails into an empty list and the step
    reports "nothing changed".
    """
    return bool(BASE_REACHABLE_RE.search(block))


def jobs_diffing_against_base(text: str) -> "set[str]":
    """Jobs whose steps resolve a file list from a base-vs-head `git diff`."""
    stripped = strip_comments(text)
    return {
        job
        for job in job_names(stripped)
        if BASE_SHA_DIFF_RE.search(job_block(stripped, job))
    }


MAPFILE_OPEN_RE = re.compile(r"^\s*mapfile -t \w+ < <\(\s*$")


def guard_input_listings(block: str) -> "list[str]":
    """The `git diff --name-only` lines that FEED a guard a list of paths.

    Only these must exclude deletions: their paths are handed to a script
    that opens them. A `git diff | grep -q` path FILTER must NOT exclude
    deletions — removing a file is a change the filtered job still needs to
    react to, so filtering deletions out there would make a deletion-only
    pull request skip the job.
    """
    lines = block.split("\n")
    listings, inside = [], False
    for line in lines:
        if MAPFILE_OPEN_RE.match(line):
            inside = True
            continue
        if inside:
            if line.strip().startswith(")"):
                inside = False
            elif "git diff --name-only" in line:
                listings.append(line)
    return listings


def checkout_is_full_history(block: str) -> bool:
    """Whether a job's `actions/checkout` asks for the full history.

    `fetch-depth: 0` is the only value that guarantees the base commit is
    present; any positive depth is a gamble on how far the branch has moved.
    """
    if not CHECKOUT_RE.search(block):
        # No checkout at all: nothing to diff against either way.
        return False
    return bool(FETCH_DEPTH_RE.search(block))


def load_guard_registry() -> dict:
    """``scripts/guards.json`` — the declared set of repository guards."""
    return json.loads(GUARDS_REGISTRY.read_text(encoding="utf-8"))


def discovered_guard_scripts() -> "set[str]":
    """Guard-shaped scripts present on disk, repo-relative.

    The filesystem, not the registry, answers "what guards exist". That is what
    makes the registry unable to shrink quietly: dropping an entry leaves the
    script on disk, and the script still has to be accounted for.
    """
    found = set()
    for path in sorted((REPO_ROOT / "scripts").glob("*")):
        rel = f"scripts/{path.name}"
        if path.is_file() and GUARD_SCRIPT_RE.match(rel):
            found.add(rel)
    return found


#: A repo-relative script path as a workflow spells it.
WORKFLOW_SCRIPT_RE = re.compile(r"(?<![\w./-])(scripts/[\w./-]+\.(?:py|sh|ps1))")


def scripts_invoked_by_workflows() -> "dict[str, list[str]]":
    """Every `scripts/…` path any workflow runs, to `file:line` sites.

    The mirror of :func:`discovered_guard_scripts`, and the half that closes
    the class. That one asks the filesystem "which files LOOK like guards?",
    through a name pattern (:44) — and a name pattern is forever guessable.
    Measured: `scripts/compare_perf.py` exits 1 on a performance regression,
    runs in the required `perf-smoke` job, and matched no pattern, so the
    registry never knew it existed.

    Lines inside the `on:` block are skipped: a `paths:` filter NAMES scripts
    it never runs, and reading those would demand registry entries for files
    the workflow only watches.
    """
    found: "dict[str, list[str]]" = {}
    for workflow in sorted(WORKFLOW_DIR.glob("*.yml")):
        inside_triggers = False
        for number, line in enumerate(
            workflow.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if COMMENT_LINE_RE.match(line):
                continue
            if re.match(r"^on:", line):
                inside_triggers = True
                continue
            if inside_triggers:
                if re.match(r"^[a-z]", line):
                    inside_triggers = False
                else:
                    continue
            for match in WORKFLOW_SCRIPT_RE.finditer(line):
                found.setdefault(match.group(1), []).append(
                    f"{workflow.name}:{number}"
                )
    return found


def script_mentions_in_job(workflow_text: str, job: str, script: str) -> "list[str]":
    """Live (comment-stripped) lines of ``job`` that name ``script``.

    Not GUARD_INVOCATION_RE: that one only matches a single-line
    ``run: python3 scripts/…``, and two required guards are invoked from inside
    a multi-line ``run: |`` block (ci.yml:271 and :296). A guard whose
    invocation shape the regex cannot see would have looked unwired.
    """
    block = strip_comments(job_block(workflow_text, job))
    return [line.strip() for line in block.split("\n") if script in line]


def inline_step_blocks(workflow_text: str, job: str, steps: "list[str]") -> "dict[str, str]":
    """The live YAML of each named step of ``job``, keyed by step name.

    A guard need not be a script: pr-governance.yml writes Git Flow, branch
    freshness and the AI-attribution check as steps. They were real guards that
    the registry could not even name, because every wiring test looked for a
    `run:` line mentioning a path under scripts/. Missing steps are simply
    absent from the mapping, which is what makes their removal detectable.
    """
    block = strip_comments(job_block(workflow_text, job))
    found = {}
    for step in steps:
        start = block.find(f"- name: {step}\n")
        if start == -1:
            continue
        rest = block[start:]
        following = re.search(r"^      - name: ", rest[1:], re.MULTILINE)
        found[step] = rest if following is None else rest[: 1 + following.start()]
    return found


def step_attributes(step_block: str) -> str:
    """A step's own YAML keys, without the shell body of its `run:`.

    Where a step can be disarmed. Scanning the whole block instead would read
    the shell too, and `ident=$(git log … | grep -iE … || true)` is a `grep`
    that found nothing, not a guard that gave up — DISARM_TOKENS applied to the
    body flagged it, which is a false positive on a line doing its job.
    """
    return "\n".join(
        line
        for line in step_block.split("\n")
        if re.match(r"^ {8}[A-Za-z][\w-]*:", line)
    )


def trigger_block(text: str, trigger: str) -> str:
    """The `on:` sub-block of one trigger, up to the next trigger or top key."""
    on_match = re.search(r"^on:$", strip_comments(text), re.MULTILINE)
    if on_match is None:
        raise AssertionError("no top-level `on:` block found")
    rest = strip_comments(text)[on_match.end():]
    end = re.search(r"^[a-z]", rest, re.MULTILINE)
    on_block = rest[: end.start()] if end else rest
    start = re.search(rf"^  {re.escape(trigger)}:", on_block, re.MULTILINE)
    if start is None:
        raise AssertionError(f"ci.yml has no `{trigger}:` trigger")
    tail = on_block[start.end():]
    following = re.search(r"^  [a-z_]+:", tail, re.MULTILINE)
    return tail[: following.start()] if following else tail


def called_workflow(ci_text: str, job: str) -> "str | None":
    """The workflow a ci.yml job delegates to via ``uses: ./…``, if any."""
    match = re.search(
        r"^\s*uses:\s*\./(\.github/workflows/[\w.-]+)\s*$",
        strip_comments(job_block(ci_text, job)),
        re.MULTILINE,
    )
    return match.group(1) if match else None


def required_ci_jobs() -> "set[str]":
    """ci.yml jobs whose failure actually fails the one required check.

    Both halves are needed: `needs` makes the job run, the chain makes its
    failure count. CHAIN_EXEMPT jobs are in `needs` and deliberately unread,
    so they are NOT required.
    """
    text = CI_WORKFLOW.read_text(encoding="utf-8")
    return set(ci_success_needs(text)) & chain_checked_jobs(text)


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

    def test_ci_runs_on_every_pull_request(self) -> None:
        # Load-bearing since the gate fold: six guards gave up their own
        # `pull_request` triggers to report inside `CI Success`. A `paths:`
        # filter here would silently stop ALL of them — plus the thirteen jobs
        # that were already required — on any PR touching only unlisted files.
        # The `push` trigger keeps its filter; each folded workflow kept its own.
        block = trigger_block(CI_WORKFLOW.read_text(encoding="utf-8"), "pull_request")
        for key in ("paths:", "paths-ignore:"):
            with self.subTest(key=key):
                self.assertNotIn(
                    key,
                    block,
                    "ci.yml's pull_request trigger must stay unfiltered: every "
                    "required gate now reports through it. Filter individual "
                    "jobs instead, never the workflow.",
                )

    def test_ci_still_runs_when_a_draft_becomes_ready(self) -> None:
        # pr-governance.yml declared `ready_for_review` before it was folded
        # into ci.yml. The default type set is [opened, synchronize, reopened],
        # so folding without spelling the types out here would have narrowed
        # that guard's reach without a single line saying so — exactly the
        # class of silent coverage loss this campaign exists to close.
        block = trigger_block(CI_WORKFLOW.read_text(encoding="utf-8"), "pull_request")
        self.assertIn(
            "ready_for_review",
            block,
            "ci.yml's pull_request types must keep `ready_for_review`: it is not "
            "in the default set, and pr-governance now runs only from here.",
        )


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


class GuardRegistryShapeTests(unittest.TestCase):
    """``scripts/guards.json`` is well-formed and exhaustive.

    Everything below this class checks that each declared guard is wired. This
    class checks the prior question the repository had no answer to: *which
    guards are we even claiming to have?* The set used to live only in the YAML
    text and in GitHub's branch-protection settings, so removing an invocation
    was invisible (#1702) and a guard nobody required was decorative (#1698).
    """

    def setUp(self) -> None:
        self.registry = load_guard_registry()
        self.guards = self.registry["guards"]
        self.unwired = self.registry["unwired"]

    def test_every_guard_declares_the_fields_the_wiring_tests_read(self) -> None:
        required_fields = ("script", "purpose", "workflow", "job", "required", "mode")
        for entry in self.guards:
            with self.subTest(script=entry.get("script")):
                for field in required_fields:
                    self.assertIn(field, entry, f"missing `{field}`")
                self.assertIn(entry["mode"], ("strict", "warn"))
                self.assertIsInstance(entry["required"], bool)

    def test_a_warn_guard_states_what_would_make_it_strict(self) -> None:
        # A `warn` without a way out is not a guard, it is a log. This is the
        # rule check-doc-contract.sh needed and did not have: its own header
        # prescribed the flip to strict, and nothing carried the obligation.
        for entry in self.guards:
            if entry["mode"] != "warn":
                continue
            with self.subTest(script=entry["script"]):
                self.assertTrue(
                    (entry.get("exit_condition") or "").strip(),
                    "a `warn` guard must declare its exit_condition",
                )

    def test_an_inline_guard_names_the_steps_that_are_the_guard(self) -> None:
        # `inline_steps: []` would satisfy the wiring test vacuously — an entry
        # claiming a guard while pointing at nothing.
        for entry in self.guards:
            if "inline_steps" not in entry:
                continue
            with self.subTest(script=entry["script"]):
                self.assertIsInstance(entry["inline_steps"], list)
                self.assertTrue(
                    entry["inline_steps"],
                    "an inline guard must name at least one step, else it "
                    "declares a guard the wiring test cannot read",
                )

    def test_a_strict_required_guard_has_been_seen_refusing_or_says_why_not(self) -> None:
        # The question under this whole registry. `guards.json` answered
        # "which guards exist" (#1702), and the fold of #1698 made them block a
        # merge; neither answers "can this one refuse at all?".
        # `check-perf-claims.py` satisfies both today and is structurally
        # unable to reach its own `exit 1` (#1701). So a strict, required guard
        # either carries executable vectors — run by
        # scripts/tests/test_guard_refusal_vectors.py — or states in writing
        # why it has none yet, naming the issue that will give it one.
        for entry in self.guards:
            if entry["mode"] != "strict" or not entry["required"]:
                continue
            with self.subTest(script=entry["script"]):
                if entry.get("must_refuse"):
                    continue
                reason = (entry.get("refusal_untested") or "").strip()
                self.assertTrue(
                    reason,
                    "declares no `must_refuse` vector and no `refusal_untested` "
                    "reason: nothing has ever seen this guard refuse, and nothing "
                    "says why not",
                )
                self.assertRegex(
                    reason,
                    r"#\d+",
                    "an untested refusal must name the issue that will close it, "
                    "else the gap has no owner",
                )

    def test_every_script_a_workflow_runs_is_accounted_for(self) -> None:
        # The sweep in the other direction, and the one that closes the class.
        # `test_every_guard_shaped_script_on_disk_is_accounted_for` asks the
        # filesystem which files LOOK like guards, through a name pattern —
        # forever guessable. This one starts from what CI actually RUNS, which
        # cannot be guessed wrong.
        #
        # It found `scripts/compare_perf.py`: exits 1 on a performance
        # regression, runs in the required `perf-smoke` job, matched no
        # pattern, and was therefore a required guard the registry did not
        # know existed.
        declared = {entry["script"] for entry in self.guards}
        declared |= {entry["script"] for entry in self.unwired}
        declared |= {entry["script"] for entry in self.registry.get("not_a_gate", [])}
        invoked = scripts_invoked_by_workflows()
        unaccounted = sorted(set(invoked) - declared)
        self.assertEqual(
            unaccounted,
            [],
            "script(s) a workflow runs and the registry never mentions: "
            + ", ".join(f"{name} ({', '.join(invoked[name][:2])})" for name in unaccounted)
            + ". Add each under `guards`, `unwired` or `not_a_gate`.",
        )

    def test_a_not_a_gate_entry_says_why_it_cannot_refuse(self) -> None:
        # The escape hatch has to cost something, or every awkward script ends
        # up in it. A script declared here must have no failure path at all —
        # the claim is checkable, so it is checked.
        for entry in self.registry.get("not_a_gate", []):
            with self.subTest(script=entry["script"]):
                self.assertTrue(
                    (entry.get("reason") or "").strip(),
                    "a `not_a_gate` entry must say why it cannot refuse",
                )
                source = (REPO_ROOT / entry["script"]).read_text(
                    encoding="utf-8", errors="replace"
                )
                self.assertNotRegex(
                    source,
                    r"sys\.exit\(\s*[1-9]|raise SystemExit\(\s*[1-9]|^\s*exit 1\b",
                    f"{entry['script']} is declared as unable to refuse, but it "
                    "carries a non-zero exit path — it is a guard, and it belongs "
                    "under `guards`.",
                )

    def test_no_guard_is_declared_twice(self) -> None:
        # For a script guard the path IS the identity. An inline guard's
        # `script` is its workflow file, which every inline guard of that
        # workflow shares (ci.yml carries three), so its identity is
        # (workflow, job). Script paths keep the stricter rule: one entry
        # per script, whatever the job.
        keys = [
            (entry["script"], entry["job"])
            if "inline_steps" in entry
            else entry["script"]
            for entry in self.guards
        ]
        duplicates = sorted({str(key) for key in keys if keys.count(key) > 1})
        self.assertEqual(duplicates, [], f"duplicate registry entries: {duplicates}")

    def test_every_declared_script_exists_on_disk(self) -> None:
        for entry in self.guards + self.unwired:
            with self.subTest(script=entry["script"]):
                self.assertTrue(
                    (REPO_ROOT / entry["script"]).is_file(),
                    "registry names a script that is not there",
                )

    def test_every_guard_shaped_script_on_disk_is_accounted_for(self) -> None:
        # The anti-shrink half. A registry that only had to agree with itself
        # could be emptied one entry at a time; this makes the filesystem the
        # source of truth for the SET, and the registry answerable to it.
        declared = {entry["script"] for entry in self.guards}
        declared |= {entry["script"] for entry in self.unwired}
        unaccounted = sorted(discovered_guard_scripts() - declared)
        self.assertEqual(
            unaccounted,
            [],
            f"guard-shaped script(s) missing from scripts/guards.json: {unaccounted}. "
            "Add each one under `guards` with its workflow and job, or under "
            "`unwired` with a written reason.",
        )

    def test_an_unwired_guard_states_why(self) -> None:
        for entry in self.unwired:
            with self.subTest(script=entry["script"]):
                self.assertTrue(
                    (entry.get("reason") or "").strip(),
                    "an unwired guard must say why it is not wired",
                )

    def test_at_least_one_guard_is_required(self) -> None:
        self.assertTrue(
            [entry for entry in self.guards if entry["required"]],
            "no guard in the registry is required at merge — the registry gates nothing",
        )


class GuardWiringTests(unittest.TestCase):
    """Each declared guard is invoked where the registry says it is."""

    def setUp(self) -> None:
        self.guards = load_guard_registry()["guards"]
        self.required_jobs = required_ci_jobs()

    def _workflow_text(self, entry: dict) -> str:
        path = REPO_ROOT / entry["workflow"]
        self.assertTrue(path.is_file(), f"no such workflow: {entry['workflow']}")
        return path.read_text(encoding="utf-8")

    def test_every_guard_is_invoked_in_the_job_the_registry_names(self) -> None:
        for entry in self.guards:
            with self.subTest(script=entry["script"]):
                steps = entry.get("inline_steps")
                if steps:
                    present = inline_step_blocks(
                        self._workflow_text(entry), entry["job"], steps
                    )
                    self.assertEqual(
                        sorted(present),
                        sorted(steps),
                        f"{entry['workflow']} job `{entry['job']}` no longer has "
                        f"step(s) {sorted(set(steps) - set(present))} — the check "
                        "was removed or renamed, or the registry entry is stale.",
                    )
                    continue
                mentions = script_mentions_in_job(
                    self._workflow_text(entry), entry["job"], entry["script"]
                )
                self.assertTrue(
                    mentions,
                    f"{entry['workflow']} job `{entry['job']}` never runs "
                    f"{entry['script']} — the invocation was removed, or the "
                    "registry entry is stale.",
                )

    def test_a_required_guard_lives_in_a_job_that_blocks_the_merge(self) -> None:
        ci_text = CI_WORKFLOW.read_text(encoding="utf-8")
        for entry in self.guards:
            if not entry["required"]:
                continue
            caller = entry.get("required_via")
            with self.subTest(script=entry["script"]):
                if caller is None:
                    # Invoked directly by a ci.yml job.
                    self.assertEqual(
                        entry["workflow"],
                        ".github/workflows/ci.yml",
                        "a guard required without `required_via` must live in ci.yml",
                    )
                    self.assertIn(
                        entry["job"],
                        self.required_jobs,
                        f"`{entry['job']}` is not both in CI Success's needs and "
                        "read by its chain, so this guard cannot block a merge",
                    )
                    continue
                # Invoked in its own workflow, made required by a caller job.
                # Both links are checked: pointing `required_via` at some
                # unrelated required job would otherwise pass.
                self.assertIn(
                    caller,
                    self.required_jobs,
                    f"caller job `{caller}` is not required by CI Success",
                )
                self.assertEqual(
                    called_workflow(ci_text, caller),
                    entry["workflow"],
                    f"ci.yml job `{caller}` does not call {entry['workflow']}",
                )

    def test_every_declared_subguard_is_actually_selected(self) -> None:
        # One level below the invocation. `check-doc-freshness.py` is named
        # four times in its job, once per `--guard <name>`; the wiring test
        # above only asks whether the SCRIPT is mentioned, so deleting the
        # `--guard tracked` line left it mentioned three times and green.
        # A sub-guard that can vanish while the registry announces it is the
        # registry lying about its own coverage, one level down.
        for entry in self.guards:
            selectors = entry.get("subguards")
            if not selectors:
                continue
            lines = script_mentions_in_job(
                self._workflow_text(entry), entry["job"], entry["script"]
            )
            joined = "\n".join(lines)
            # An invocation carrying no `--guard` at all runs every sub-guard
            # (both scripts default to `all`), so it satisfies each selector.
            # Only a job that names selectors explicitly can silently lose one.
            runs_all = any("--guard" not in line for line in lines)
            for selector in selectors:
                with self.subTest(script=entry["script"], subguard=selector):
                    if runs_all:
                        continue
                    self.assertRegex(
                        joined,
                        rf"--guard\s+{re.escape(selector)}\b",
                        f"`--guard {selector}` is declared in the registry but "
                        f"never selected in {entry['workflow']} job "
                        f"`{entry['job']}` — the sub-guard does not run.",
                    )

    def test_a_strict_guard_is_not_disarmed_at_its_invocation(self) -> None:
        for entry in self.guards:
            if entry["mode"] != "strict":
                continue
            text = self._workflow_text(entry)
            steps = entry.get("inline_steps")
            invocations = (
                [
                    step_attributes(block)
                    for block in inline_step_blocks(text, entry["job"], steps).values()
                ]
                if steps
                else script_mentions_in_job(text, entry["job"], entry["script"])
            )
            for line in invocations:
                for token in DISARM_TOKENS:
                    with self.subTest(script=entry["script"], token=token):
                        self.assertNotIn(
                            token,
                            line,
                            f"strict guard disarmed at its invocation: {line!r}",
                        )

    def _required_test_surface(self) -> str:
        """Everything a required job can run, including through a called workflow.

        A caller job's own block is three lines of `uses:`; the suites it runs
        live in the callee. Reading only ci.yml would report every folded gate's
        self-test as unreached.
        """
        ci_text = CI_WORKFLOW.read_text(encoding="utf-8")
        parts = []
        for job in sorted(self.required_jobs):
            parts.append(job_block(ci_text, job))
            callee = called_workflow(ci_text, job)
            if callee:
                parts.append((REPO_ROOT / callee).read_text(encoding="utf-8"))
        return "\n".join(parts)

    def test_the_self_test_declaration_is_true(self) -> None:
        # Both directions: a `true` that is not backed by a required job
        # overstates coverage, and a `false` that IS backed understates it and
        # goes stale the day the wiring improves.
        surface = self._required_test_surface()
        # `unittest discover -s scripts/tests` runs every module under that
        # directory, so it covers each one by name without naming any.
        discovers_all = bool(
            re.search(r"unittest\s+discover\s+-s\s+scripts/tests", surface)
        )
        for entry in self.guards:
            module = entry.get("self_test")
            if not module:
                continue
            actually_required = discovers_all or module in surface
            with self.subTest(script=entry["script"], module=module):
                self.assertEqual(
                    entry.get("self_test_required"),
                    actually_required,
                    f"`self_test_required` for {module} says "
                    f"{entry.get('self_test_required')}, the workflow says "
                    f"{actually_required}",
                )


class GuardRegistryDisarmTests(unittest.TestCase):
    """Every way to quietly widen the hole, replayed — each must be RED.

    Same discipline as DisarmTests above: the registry is worth exactly what it
    refuses. These mutate in-memory copies, so nothing on disk changes.
    """

    def setUp(self) -> None:
        self.registry = load_guard_registry()
        self.ci_text = CI_WORKFLOW.read_text(encoding="utf-8")

    def _required_entry(self) -> dict:
        for entry in self.registry["guards"]:
            if entry["required"] and entry["workflow"] == ".github/workflows/ci.yml":
                return entry
        raise AssertionError("no required ci.yml guard to mutate")

    def test_removing_an_invocation_from_ci_yml_is_detected(self) -> None:
        entry = self._required_entry()
        before = script_mentions_in_job(self.ci_text, entry["job"], entry["script"])
        self.assertTrue(before, "fixture precondition: the guard is invoked today")
        broken = self.ci_text.replace(entry["script"], "scripts/removed-guard.py")
        self.assertEqual(
            script_mentions_in_job(broken, entry["job"], entry["script"]),
            [],
            "dropping the invocation must leave nothing for the wiring test to find",
        )

    def test_deleting_a_registry_entry_is_detected_by_the_disk_sweep(self) -> None:
        entry = self._required_entry()
        declared = {
            e["script"] for e in self.registry["guards"] if e["script"] != entry["script"]
        }
        declared |= {e["script"] for e in self.registry["unwired"]}
        self.assertIn(
            entry["script"],
            discovered_guard_scripts() - declared,
            "a deleted entry must resurface as an unaccounted script on disk",
        )

    def test_an_unregistered_new_guard_script_is_detected(self) -> None:
        declared = {e["script"] for e in self.registry["guards"]}
        declared |= {e["script"] for e in self.registry["unwired"]}
        self.assertNotIn("scripts/check-newcomer.py", declared)
        self.assertRegex(
            "scripts/check-newcomer.py",
            GUARD_SCRIPT_RE,
            "a check-*.py file must be seen as guard-shaped, else the sweep is blind",
        )

    def test_warn_mode_on_a_strict_guard_is_detected(self) -> None:
        entry = self._required_entry()
        line = script_mentions_in_job(self.ci_text, entry["job"], entry["script"])[0]
        broken = self.ci_text.replace(line, line + " --mode warn")
        disarmed = [
            candidate
            for candidate in script_mentions_in_job(broken, entry["job"], entry["script"])
            if "--mode warn" in candidate
        ]
        self.assertTrue(disarmed, "the disarm must be visible on the invocation line")

    def _inline_entry(self) -> dict:
        for entry in self.registry["guards"]:
            if entry.get("inline_steps"):
                return entry
        # #1715 extracted the final inline guards into executable scripts. Keep
        # the parser/disarm contract alive against a real remaining workflow
        # step so reintroducing an inline entry cannot make this capability
        # silently rot merely because the registry currently needs none.
        return {
            "script": ".github/workflows/ci.yml",
            "workflow": ".github/workflows/ci.yml",
            "job": "wasm-check",
            "inline_steps": ["Run wasm-bindgen tests (Node)"],
        }

    def test_deleting_an_inline_step_from_its_workflow_is_detected(self) -> None:
        entry = self._inline_entry()
        text = (REPO_ROOT / entry["workflow"]).read_text(encoding="utf-8")
        steps = entry["inline_steps"]
        self.assertEqual(
            sorted(inline_step_blocks(text, entry["job"], steps)),
            sorted(steps),
            "fixture precondition: every declared step is there today",
        )
        victim = steps[-1]
        broken = text.replace(f"- name: {victim}\n", "- name: Something else\n")
        self.assertNotIn(
            victim,
            inline_step_blocks(broken, entry["job"], steps),
            "renaming a declared step away must leave the wiring test nothing to find",
        )

    def test_continue_on_error_on_an_inline_step_is_detected(self) -> None:
        # The inline equivalent of `--mode warn`: the step still runs, still
        # exits 1, and the job stays green.
        entry = self._inline_entry()
        text = (REPO_ROOT / entry["workflow"]).read_text(encoding="utf-8")
        victim = entry["inline_steps"][-1]
        today = [
            step_attributes(block)
            for block in inline_step_blocks(
                text, entry["job"], entry["inline_steps"]
            ).values()
        ]
        self.assertEqual(
            [token for attrs in today for token in DISARM_TOKENS if token in attrs],
            [],
            "fixture precondition: no declared step is disarmed today",
        )
        broken = text.replace(
            f"- name: {victim}\n", f"- name: {victim}\n        continue-on-error: true\n"
        )
        attributes = step_attributes(
            inline_step_blocks(broken, entry["job"], [victim])[victim]
        )
        self.assertTrue(
            any(token in attributes for token in DISARM_TOKENS),
            "a disarmed inline step must be visible among the step's own YAML keys",
        )

    def test_dropping_ready_for_review_from_ci_is_detected(self) -> None:
        text = CI_WORKFLOW.read_text(encoding="utf-8")
        broken = text.replace(
            "    types: [opened, synchronize, reopened, ready_for_review, edited]\n", "", 1
        )
        self.assertIn("ready_for_review", trigger_block(text, "pull_request"))
        self.assertNotIn("ready_for_review", trigger_block(broken, "pull_request"))

    def test_moving_a_required_guard_to_an_unrequired_job_is_detected(self) -> None:
        # The subtler disarm: the guard still runs, still strict, still in
        # ci.yml — just in a job whose result the chain does not read.
        exempt = sorted(CHAIN_EXEMPT)[0]
        self.assertNotIn(
            exempt,
            required_ci_jobs(),
            f"`{exempt}` is chain-exempt, so a guard parked there blocks nothing",
        )


class DiffScopedGateReachabilityTests(unittest.TestCase):
    """A diff-scoped gate needs the history its diff reads.

    `lint` carried two of them — the SAFETY-template verifier and the
    TODO/FIXME governance check — against a default depth-1 checkout, so both
    validated zero files on every pull request while reporting success. The
    three other jobs that diff against the base already asked for
    `fetch-depth: 0` and even carry the comment explaining why, which is what
    makes this a mechanical invariant rather than a matter of taste.
    """

    def setUp(self) -> None:
        self.text = CI_WORKFLOW.read_text(encoding="utf-8")

    def test_every_job_diffing_against_the_base_checks_out_full_history(self) -> None:
        diffing = jobs_diffing_against_base(self.text)
        self.assertTrue(
            diffing,
            "no job diffs against a base SHA any more — drop this suite or fix the parser",
        )
        shallow = sorted(
            job
            for job in diffing
            if not checkout_is_full_history(job_block(strip_comments(self.text), job))
        )
        self.assertEqual(
            shallow,
            [],
            f"job(s) resolving a changed-file list from `git diff $BASE_SHA` "
            f"without `fetch-depth: 0`: {shallow}. On a pull request the base "
            "commit is not in a depth-1 checkout, so the diff fails, the file "
            "list comes back empty and the gate passes having read nothing. "
            "Add `with: {fetch-depth: 0}` to that job's actions/checkout step.",
        )

    def test_the_lint_job_is_one_of_them(self) -> None:
        # Pins the regression specifically: `lint` is where both diff-scoped
        # guards in scripts/guards.json declare they run.
        self.assertIn("lint", jobs_diffing_against_base(self.text))

    def test_a_shallow_checkout_on_a_diffing_job_is_detected(self) -> None:
        # RED-then-GREEN on the real file: removing the fetch-depth from
        # `lint` must make the invariant fail, or the test proves nothing.
        block = job_block(strip_comments(self.text), "lint")
        self.assertTrue(checkout_is_full_history(block))
        self.assertFalse(checkout_is_full_history(block.replace("fetch-depth: 0", "", 1)))

    def test_every_diffing_job_refuses_an_unreachable_base(self) -> None:
        # Belt and braces to fetch-depth: the depth fix makes the base
        # present, this makes its ABSENCE loud instead of silent, for the
        # failure modes fetch-depth cannot cover.
        missing = sorted(
            job
            for job in jobs_diffing_against_base(self.text)
            if not asserts_base_is_reachable(job_block(strip_comments(self.text), job))
        )
        self.assertEqual(
            missing,
            [],
            f"job(s) diffing against a base they never prove is present: {missing}. "
            "A missing base makes `git diff` fail into an empty file list, which "
            "reads exactly like 'nothing changed'. Add the `git cat-file -e "
            "\"$BASE_SHA^{commit}\"` check before the diff.",
        )

    def test_removing_the_reachability_check_is_detected(self) -> None:
        # `lint` carries one check per diff-scoped gate, so the negative
        # control has to remove EVERY occurrence — dropping just the first
        # leaves the second matching and proves nothing.
        block = job_block(strip_comments(self.text), "lint")
        self.assertTrue(asserts_base_is_reachable(block))
        self.assertFalse(
            asserts_base_is_reachable(block.replace("git cat-file -e", "true #"))
        )

    def test_this_changes_own_suite_runs_in_the_required_job(self) -> None:
        # Same reasoning as the mcp-doc-contract suites above: the verifier's
        # self-test must run in `lint`, which `CI Success` reads, and not only
        # in gate-contracts.yml, whose result blocks nothing.
        self.assertIn(
            "scripts.tests.test_verify_unsafe_safety_template",
            job_block(self.text, "lint"),
        )

    def test_a_job_that_does_not_diff_is_not_required_to_be_deep(self) -> None:
        # The invariant must stay scoped to gates that need the history:
        # a synthetic job with a checkout and no base diff is not flagged.
        synthetic = (
            "jobs:\n"
            "  shallow-ok:\n"
            "    steps:\n"
            "      - uses: actions/checkout@v7\n"
            "      - run: cargo fmt --all -- --check\n"
            "  ci-success:\n"
            "    needs: [shallow-ok]\n"
        )
        self.assertEqual(jobs_diffing_against_base(synthetic), set())

    def test_the_diff_gates_skip_deleted_paths(self) -> None:
        # A rename or a removal leaves a path in `git diff --name-only` that
        # is no longer on disk; the guards then warn per missing file. The
        # file list must be filtered at the source instead.
        checked = 0
        for job in sorted(jobs_diffing_against_base(self.text)):
            block = job_block(strip_comments(self.text), job)
            for line in guard_input_listings(block):
                checked += 1
                with self.subTest(job=job, line=line.strip()):
                    self.assertIn(
                        "--diff-filter=d",
                        line,
                        f"{job}: a path list handed to a guard must exclude "
                        "deletions (`--diff-filter=d`) — a removed path is still "
                        "named by `--name-only` but is gone from disk, so the "
                        "guard warns on a file it cannot open",
                    )
        self.assertTrue(checked, "no guard-input listing found — fix the parser")

    def test_a_path_filter_must_keep_deletions(self) -> None:
        # The mirror of the rule above, and the reason it is scoped to
        # `mapfile` rather than to every base diff: `node-windows-changed`
        # and `perf-path-changed` only `grep -q` the names to decide whether
        # to run. Deleting a file under crates/velesdb-node/ is a change that
        # job must still react to, so excluding deletions there would make a
        # deletion-only PR skip it.
        for job in ("node-windows-changed", "perf-path-changed"):
            block = job_block(strip_comments(self.text), job)
            with self.subTest(job=job):
                self.assertEqual(
                    guard_input_listings(block),
                    [],
                    f"{job} is a path filter, not a guard input",
                )
                filters = [
                    line
                    for line in block.split("\n")
                    if BASE_SHA_DIFF_RE.search(line) and "grep -q" in line
                ]
                self.assertTrue(filters, f"{job} no longer greps a base diff")
                for line in filters:
                    self.assertNotIn(
                        "--diff-filter=d",
                        line,
                        f"{job}: a path filter must SEE deletions",
                    )


if __name__ == "__main__":
    unittest.main()


# ---------------------------------------------------------------------------
# Retargeting a PR must re-run the gates
# ---------------------------------------------------------------------------

PR_TYPES_RE = re.compile(r"^\s*types:\s*\[([^\]]*)\]", re.MULTILINE)
JOB_ID_RE = re.compile(r"^  ([a-z][a-z0-9_-]*):$", re.MULTILINE)
# The clause that admits `edited` only for a base change. Whitespace-tolerant
# so reformatting the expression does not read as removing it.
# The one job that must NOT carry the guard, with its reason — same shape as
# CHAIN_EXEMPT above. `mcp-doc-contract` runs the guard suites' own self-tests,
# and `test_the_gate_job_cannot_opt_out_of_blocking` forbids it any job-level
# `if:` at all: a condition that can skip it can disarm them. It pays for that
# by running on title-only edits, which is one cheap Python job.
RETARGET_GUARD_EXEMPT = {"mcp-doc-contract": "runs the guard self-tests; must never be skippable"}

RETARGET_GUARD_RE = re.compile(
    r"github\.event\.action\s*!=\s*'edited'\s*\|\|\s*github\.event\.changes\.base\s*!=\s*null"
)


def pull_request_types(text: str) -> list[str]:
    """Event types the workflow's `pull_request:` trigger subscribes to."""
    match = PR_TYPES_RE.search(text)
    if match is None:
        return []
    return [t.strip() for t in match.group(1).split(",") if t.strip()]


def jobs_with_guard(text: str) -> tuple[list[str], list[str]]:
    """Splits ci.yml's top-level jobs into (guarded, unguarded)."""
    starts = [(m.group(1), m.start()) for m in JOB_ID_RE.finditer(text)]
    # `on:` keys (push/pull_request/workflow_dispatch) match the job shape;
    # everything from `jobs:` onward is a real job.
    jobs_at = text.index("\njobs:\n")
    starts = [(name, at) for name, at in starts if at > jobs_at]
    guarded, unguarded = [], []
    for index, (name, at) in enumerate(starts):
        end = starts[index + 1][1] if index + 1 < len(starts) else len(text)
        block = text[at:end]
        head = block.split("\n    steps:", 1)[0]
        (guarded if RETARGET_GUARD_RE.search(head) else unguarded).append(name)
    return guarded, unguarded


class RetargetRerunTests(unittest.TestCase):
    """Changing a PR's base must not leave a stale verdict or absent checks.

    Retargeting emits `pull_request.edited` and nothing else — no
    `synchronize`, no `reopened`. Before this, neither workflow listened to
    it, with two consequences seen in production on PR #2118:

      * `PR Governance` kept reporting its refusal against the *old* base
        forever. Re-running cannot clear that: GitHub replays the original
        event payload, stale `BASE_REF` included.
      * `ci.yml` (filtered to `branches: [main, develop]`) had never run at
        all for a PR opened against a feature branch, so after retargeting
        onto develop the required `CI Success` check was *absent* — the
        #1465 failure mode, which blocks a merge with no way out.
    """

    def setUp(self) -> None:
        self.ci = CI_WORKFLOW.read_text(encoding="utf-8")
        self.governance = (WORKFLOW_DIR / "pr-governance.yml").read_text(encoding="utf-8")

    def test_both_workflows_listen_for_a_retarget(self) -> None:
        for label, text in (("ci.yml", self.ci), ("pr-governance.yml", self.governance)):
            with self.subTest(workflow=label):
                self.assertIn(
                    "edited",
                    pull_request_types(text),
                    f"{label} must subscribe to `edited`, the only event a base change emits; "
                    "without it a retargeted PR keeps a stale verdict and never re-runs",
                )

    def test_every_ci_job_guards_against_a_title_only_edit(self) -> None:
        guarded, unguarded = jobs_with_guard(self.ci)
        unguarded = [j for j in unguarded if j not in RETARGET_GUARD_EXEMPT]
        self.assertEqual(
            unguarded,
            [],
            "`edited` also fires on every title/body edit. Each job must admit it only "
            "when `github.event.changes.base` is set, or routine description edits cost a "
            f"full CI run. Unguarded: {unguarded}",
        )
        self.assertGreater(len(guarded), 20, "job discovery looks broken, not the guard")

    def test_the_exempt_job_really_is_unguarded(self) -> None:
        """An exemption nobody exercises is an exemption that has rotted."""
        _guarded, unguarded = jobs_with_guard(self.ci)
        for job in RETARGET_GUARD_EXEMPT:
            with self.subTest(job=job):
                self.assertIn(
                    job,
                    unguarded,
                    f"`{job}` is listed exempt but now carries the guard — drop the "
                    "exemption, or the list is documenting a rule that no longer holds",
                )

    def test_ci_success_carries_the_guard_too(self) -> None:
        block = self.ci[self.ci.index("\n  ci-success:") :]
        self.assertRegex(
            block.split("\n    steps:", 1)[0],
            RETARGET_GUARD_RE,
            "ci-success asserts `result == 'success'` for each of its needs, so leaving it "
            "unguarded while its needs skip on a title edit turns every such edit RED. It "
            "must skip with them — a skipped check-run is accepted by branch protection.",
        )


class RetargetGuardParserTests(unittest.TestCase):
    """RED-then-GREEN on synthetic text, per this module's parser contract."""

    GUARD = "github.event.action != 'edited' || github.event.changes.base != null"

    def test_types_parser_reads_the_list(self) -> None:
        self.assertEqual(
            pull_request_types("on:\n  pull_request:\n    types: [opened, edited]\n"),
            ["opened", "edited"],
        )
        self.assertEqual(pull_request_types("on:\n  push:\n"), [])

    def test_guard_parser_separates_guarded_from_unguarded(self) -> None:
        text = (
            "on:\n  pull_request:\n    types: [opened]\n"
            "\njobs:\n"
            f"  alpha:\n    name: A\n    if: {self.GUARD}\n    steps:\n      - run: true\n"
            "  beta:\n    name: B\n    steps:\n      - run: true\n"
        )
        guarded, unguarded = jobs_with_guard(text)
        self.assertEqual((guarded, unguarded), (["alpha"], ["beta"]))

    def test_guard_parser_accepts_a_composed_condition(self) -> None:
        text = (
            "on:\n  pull_request:\n    types: [opened]\n"
            "\njobs:\n"
            f"  alpha:\n    name: A\n    if: (always()) && ({self.GUARD})\n    steps:\n      - run: true\n"
        )
        self.assertEqual(jobs_with_guard(text), (["alpha"], []))

    def test_guard_parser_ignores_a_match_inside_steps(self) -> None:
        """A guard in a step is not a guard on the job."""
        text = (
            "on:\n  pull_request:\n    types: [opened]\n"
            "\njobs:\n"
            f"  alpha:\n    name: A\n    steps:\n      - if: {self.GUARD}\n        run: true\n"
        )
        self.assertEqual(jobs_with_guard(text), ([], ["alpha"]))
