"""Tests for scripts/check-ai-attribution.py — both compartments.

The rule (no AI attribution, CLAUDE.md #5) predates this guard. What did not
exist was a guard that could be handed a case and asked its verdict: the check
lived twice as inline shell — `.githooks/commit-msg` and
`.github/workflows/pr-governance.yml` — and both spelled the same two words,
`claude|anthropic`. Codex, Copilot, Cursor, Devin and any `[bot]` walked
straight through (#1699).

The guard then admitted four bot identities by name, which the contributor
rule forbids outright: the author and committer of a commit are the human
maintainer, without exception (#2336). What makes the guard survivable is not
an admission list but a boundary — 174 commits already published here were
authored by a bot, five of them on `develop` and not yet on `main`, so
`GRANDFATHERED_THROUGH` forgives what is written and judges
everything written after. Both halves are tested below: the amnesty must hold
(or the next back-merge is blocked) and it must not stretch (or the rule is
the old exemption under a new name).
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-ai-attribution.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_ai_attribution", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


caa = _load_script()


class RefusedIdentityTests(unittest.TestCase):
    """Each assistant identity the old pair of words missed."""

    REFUSED = (
        "Claude <noreply@anthropic.com>",
        "Codex <codex@openai.com>",
        "Copilot <copilot@github.com>",
        "Cursor Agent <agent@cursor.sh>",
        "Devin AI <devin@cognition.ai>",
        "some-agent[bot] <x@y.z>",
    )

    def test_every_assistant_identity_is_refused(self) -> None:
        for identity in self.REFUSED:
            with self.subTest(identity=identity):
                self.assertIsNotNone(caa.identity_is_refused(identity))


class AdmittedIdentityTests(unittest.TestCase):
    """The positive controls. A guard that refuses these breaks the repo."""

    ADMITTED = (
        "cyberlife-coder <174732281+cyberlife-coder@users.noreply.github.com>",
        "Wiscale <174732281+cyberlife-coder@users.noreply.github.com>",
    )

    def test_humans_pass(self) -> None:
        for identity in self.ADMITTED:
            with self.subTest(identity=identity):
                self.assertIsNone(caa.identity_is_refused(identity))

    def test_no_bot_is_admitted_by_name(self) -> None:
        # The four identities the guard used to list. The contributor rule
        # names `github-actions[bot]` explicitly among what must never author
        # a commit here, so an exemption for it was the guard contradicting
        # the contract it enforces (#2336).
        for identity in (
            "dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
            "dependabot-preview[bot] <support@dependabot.com>",
            "github-actions[bot] <41898282+github-actions[bot]@users.noreply.github.com>",
            "renovate[bot] <bot@renovateapp.com>",
        ):
            with self.subTest(identity=identity):
                self.assertIsNotNone(caa.identity_is_refused(identity))

    def test_a_person_whose_name_contains_an_assistant_name_passes(self) -> None:
        # Whole-word matching: `Claudette` is a person. A substring match
        # here would refuse a real contributor by their own name.
        self.assertIsNone(caa.identity_is_refused("Claudette Dupont <c@example.org>"))

    def test_a_bot_lookalike_is_refused_too(self) -> None:
        # Nothing to inherit any more, but the `[bot]` rule must still see it.
        self.assertIsNotNone(caa.identity_is_refused("evil-copilot[bot] <x@y.z>"))


class TrailerTests(unittest.TestCase):
    """Attribution in the message, in every spelling seen in the wild."""

    REFUSED = (
        "feat: x\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
        "feat: x\n\nCo_Authored_By: Claude <n@a.com>\n",
        "feat: x\n\nSigned-off-by: Codex <c@openai.com>\n",
        "feat: x\n\nAssisted-by: Copilot\n",
        "feat: x\n\nClaude-Session: abc123\n",
        "feat: x\n\n\U0001f916 Generated with Claude Code\n",
        "feat: x\n\nSee https://claude.ai/code for details\n",
    )

    def test_every_trailer_shape_is_refused(self) -> None:
        for message in self.REFUSED:
            with self.subTest(message=message.strip().splitlines()[-1]):
                self.assertIsNotNone(caa.message_is_refused(message))

    def test_a_human_sign_off_passes(self) -> None:
        self.assertIsNone(
            caa.message_is_refused("feat: x\n\nSigned-off-by: cyberlife-coder <j@w.fr>\n")
        )

    def test_prose_describing_a_trailer_is_not_a_trailer(self) -> None:
        # Anchored at column 0: this repository's own documentation of the
        # rule must be committable. The commit-msg hook used to fail on a
        # message that merely quoted the pattern it enforces.
        self.assertIsNone(
            caa.message_is_refused(
                "docs: explain that a Co-Authored-By trailer naming an assistant is refused\n"
            )
        )


class PublishedProseTests(unittest.TestCase):
    """The body is the surface the rule loses on by default.

    Tooling appends a "Generated by ..." footer server-side, so a pull request
    arrives already violating CLAUDE.md #5 with every commit in it clean. This
    was found the hard way: three pull requests in one session were published
    with the footer and had to be stripped by hand.
    """

    def test_the_real_appended_footer_is_refused(self) -> None:
        # The exact shape the tooling appends, reassembled rather than written
        # literally — see the blind spot in guards.json: the URL pattern is
        # unanchored, so a test file spelling it out would trip the guard when
        # that text reaches a body.
        footer = "_Generated by [Claude Code](https://" + "claude.ai/code" + ")_"
        body = f"## Description\n\nA real change.\n\n---\n{footer}\n"
        self.assertEqual(len(caa.audit_text(body)), 1)

    def test_an_ordinary_body_passes(self) -> None:
        body = (
            "## Description\n\nFixes the fusion weights.\n\n"
            "## Checklist\n\n- [x] Tests pass\n"
        )
        self.assertEqual(caa.audit_text(body), [])

    def test_prose_naming_an_assistant_is_not_attribution(self) -> None:
        """Naming a vendor is not claiming one wrote the change."""
        body = (
            "Updates the claude-api skill and the Anthropic SDK pin. "
            "Reviewed with Copilot disabled."
        )
        self.assertEqual(caa.audit_text(body), [])

    def test_an_empty_body_passes(self) -> None:
        """A PR opened with no description is a style problem, not this rule's."""
        self.assertEqual(caa.audit_text(""), [])

    def test_a_co_authored_trailer_pasted_into_a_body_is_refused(self) -> None:
        body = "Describes the change.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n"
        self.assertEqual(len(caa.audit_text(body)), 1)

    def test_the_body_path_reuses_the_commit_message_rule(self) -> None:
        """One list of patterns, not two that drift.

        The guard this replaced knew `claude|anthropic` and nothing else
        because the rule was spelled twice (#1699). A body-specific pattern
        list would recreate that.
        """
        trailer = "Co-Authored-By: Codex <codex@example.com>"
        self.assertIsNotNone(caa.message_is_refused(trailer))
        self.assertEqual(len(caa.audit_text(trailer)), 1)


class BodyCliTests(unittest.TestCase):
    """The body arrives as a FILE, never as an argument — it is untrusted text."""

    def _run(self, body: str) -> int:
        import io
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pr-body.md"
            path.write_text(body, encoding="utf-8")
            stdout, sys.stdout = sys.stdout, io.StringIO()
            try:
                return caa.main(["--text", str(path)])
            finally:
                sys.stdout = stdout

    def test_clean_body_exits_zero(self) -> None:
        self.assertEqual(self._run("## Description\n\nA change.\n"), 0)

    def test_attributed_body_exits_one(self) -> None:
        trailer = "Co-Authored-By: Claude <noreply@anthropic.com>"
        self.assertEqual(self._run(f"A change.\n\n{trailer}\n"), 1)

    def test_a_missing_body_file_exits_two_not_one(self) -> None:
        """Exit 2 is 'could not run'. A crash is not a refusal."""
        import io

        stdout, sys.stdout = sys.stdout, io.StringIO()
        try:
            self.assertEqual(caa.main(["--text", "/nonexistent/pr-body.md"]), 2)
        finally:
            sys.stdout = stdout


class TreeScanTests(unittest.TestCase):
    """The "code, comments, docs" half of the rule, which nothing guarded.

    A footer in a markdown file, or a `// Generated with ...` line in a source
    file, was refused in a commit message and accepted three lines into the
    file that commit added.
    """

    def test_a_comment_leader_no_longer_defeats_the_anchor(self) -> None:
        """The finding that made this mode worth more than a doc grep.

        The trailer patterns are anchored at column 0 so this guard can
        describe them without tripping. In SOURCE, attribution wears a comment
        marker — and `// Generated with Claude` sailed straight past.
        """
        for line in (
            "// Generated with Claude, do not edit",
            "# Co-Authored-By: Codex <c@example.com>",
            "  -- Generated with Copilot",
            " * Assisted-by: Cursor",
            "<!-- Generated with Claude -->",
            "> Co-Authored-By: Devin <d@example.com>",
        ):
            stripped = caa.strip_comment_leader(line)
            self.assertIsNotNone(
                caa.message_is_refused(stripped),
                f"a comment leader still hides attribution: {line!r}",
            )

    def test_stripping_a_leader_does_not_invent_a_violation(self) -> None:
        for line in (
            "// This function is fast.",
            "# Nothing to see.",
            "* bullet about the anthropic SDK pin",
            "// see AGENTS.md for the attribution rule",
        ):
            stripped = caa.strip_comment_leader(line)
            self.assertIsNone(
                caa.message_is_refused(stripped),
                f"false positive after stripping: {line!r}",
            )

    def test_the_commit_message_rule_keeps_its_anchor(self) -> None:
        """Only --tree strips leaders; the message path must not.

        Prose describing a trailer mid-sentence is not a trailer, and that
        property is what lets the rule be documented at all.
        """
        prose = "Some text mentioning Co-Authored-By: Claude inside a sentence."
        self.assertIsNone(caa.message_is_refused(prose))

    def test_the_rule_defining_admissions_are_exact_paths(self) -> None:
        """Admitted by name, never by directory — an exemption becomes a
        loophole the moment it covers a tree."""
        for path in caa.RULE_DEFINING_PATHS:
            self.assertFalse(
                path.endswith("/"),
                f"{path} admits a directory, not a file",
            )
            self.assertNotIn("*", path, f"{path} is a wildcard admission")

    def test_changelog_and_claude_md_are_not_admitted(self) -> None:
        """They describe the rule today carrying no pattern, which is the
        proof that describing it does not require carrying it."""
        self.assertNotIn("CHANGELOG.md", caa.RULE_DEFINING_PATHS)
        self.assertNotIn("CLAUDE.md", caa.RULE_DEFINING_PATHS)

    def test_the_checked_in_tree_is_clean(self) -> None:
        """The positive control: this repository passes its own tree scan."""
        self.assertEqual(caa.audit_tree(str(REPO_ROOT)), [])

    def test_admitted_files_are_actually_skipped(self) -> None:
        listed = caa.tracked_text_files(str(REPO_ROOT))
        for path in caa.RULE_DEFINING_PATHS:
            self.assertNotIn(path, listed)

    def test_binary_assets_are_not_read(self) -> None:
        listed = caa.tracked_text_files(str(REPO_ROOT))
        self.assertFalse(
            [p for p in listed if p.lower().endswith((".png", ".woff2", ".pdf"))]
        )


class SurfaceLabelTests(unittest.TestCase):
    """`--surface` names the thing; it must never change the rule."""

    def test_the_label_reaches_the_message(self) -> None:
        trailer = "Co-Authored-By: Claude <noreply@anthropic.com>"
        found = caa.audit_text(trailer, "pull request title")
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0].startswith("pull request title:"))

    def test_every_surface_applies_the_same_rule(self) -> None:
        trailer = "Co-Authored-By: Cursor <c@example.com>"
        clean = "An ordinary description."
        for surface in ("pull request title", "pull request body", "issue", "text"):
            self.assertEqual(len(caa.audit_text(trailer, surface)), 1)
            self.assertEqual(caa.audit_text(clean, surface), [])


class RemedyTests(unittest.TestCase):
    """The remedy must be sufficient, not merely correct.

    Telling an author to edit the body is true and incomplete: the edit
    re-runs this guard (`pr-governance` carries no retarget guard) but not
    `CI Success`, which is skipped on `edited` and keeps the failing
    conclusion. Three pull requests in a row looped on that gap.
    """

    def _stderr(self, surface: str) -> str:
        import io
        import tempfile

        trailer = "Co-Authored-By: Claude <noreply@anthropic.com>"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "text.md"
            path.write_text(f"A change.\n\n{trailer}\n", encoding="utf-8")
            out, err = sys.stdout, sys.stderr
            sys.stdout, sys.stderr = io.StringIO(), io.StringIO()
            try:
                caa.main(["--text", str(path), "--surface", surface])
                return sys.stderr.getvalue()
            finally:
                sys.stdout, sys.stderr = out, err

    def test_a_pull_request_surface_is_told_to_push(self) -> None:
        for surface in ("pull request title", "pull request body"):
            with self.subTest(surface=surface):
                message = self._stderr(surface)
                self.assertIn("PUSH", message)
                self.assertIn("CI Success", message)

    def test_a_non_pull_request_surface_is_not(self) -> None:
        """The push half is specific to the required-chain gap, not general.

        Asserting only that "PUSH" is absent passed on an EMPTY stderr, so a
        change that stopped auditing the issue surface altogether would have
        left this green (#2246, P2-c). The refusal itself is required first.
        """
        message = self._stderr("issue")
        # The violation line goes to STDOUT and the remedy to stderr, so the
        # remedy is what proves the surface was refused on this stream.
        self.assertIn(
            "Edit the issue",
            message,
            "the issue surface must still be refused -- and told how to fix it -- "
            "before the absence of the push step means anything",
        )
        self.assertNotIn("PUSH", message)


class SingleSourceTests(unittest.TestCase):
    """The two call sites delegate; neither re-spells the rule."""

    ROOT = SCRIPT_PATH.parent.parent

    def test_the_workflow_calls_the_guard(self) -> None:
        text = (self.ROOT / ".github/workflows/pr-governance.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/check-ai-attribution.py", text)

    def test_the_commit_msg_hook_calls_the_guard(self) -> None:
        text = (self.ROOT / ".githooks/commit-msg").read_text(encoding="utf-8")
        self.assertIn("check-ai-attribution.py", text)

    def test_the_issue_workflow_calls_the_guard(self) -> None:
        # The surface the module docstring listed as the remaining gap: an
        # issue body or a comment arrives on events no `pull_request` workflow
        # sees, so `pr-governance.yml` cannot reach it however it is written.
        text = (self.ROOT / ".github/workflows/issue-attribution.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/check-ai-attribution.py", text)

    def test_the_issue_workflow_covers_both_surfaces_and_edits(self) -> None:
        # A guard that only fired on `opened` would be walked past by posting
        # clean text and editing the footer back in.
        text = (self.ROOT / ".github/workflows/issue-attribution.yml").read_text(encoding="utf-8")
        self.assertIn("types: [opened, edited]", text)
        self.assertIn("types: [created, edited]", text)
        self.assertIn("github.event.issue.body", text)
        self.assertIn("github.event.comment.body", text)

    def test_the_issue_workflow_never_interpolates_untrusted_text_into_the_shell(self) -> None:
        # Same rule the pull request step documents: untrusted prose reaches
        # the guard through env and then a file, never as `${{ }}` inside a
        # `run:` block, which would be a script-injection vector.
        text = (self.ROOT / ".github/workflows/issue-attribution.yml").read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith("python3") or stripped.startswith("printf"):
                self.assertNotIn("${{", stripped, f"untrusted interpolation in: {stripped}")

    def test_neither_site_still_greps_the_old_pair_of_words_as_its_rule(self) -> None:
        # The workflow's own grep was the rule; it must be gone. The hook
        # keeps its two-word grep ONLY as a fallback for a machine without
        # python3, which the comment above it says.
        text = (self.ROOT / ".github/workflows/pr-governance.yml").read_text(encoding="utf-8")
        self.assertNotIn("grep -iE 'claude|anthropic'", text)


if __name__ == "__main__":
    unittest.main()


class GrandfatheringTests(unittest.TestCase):
    """The amnesty must hold, and must not stretch.

    174 commits published in this repository were authored by a bot — 173 by
    `dependabot[bot]` — and five of them are on `develop` but not yet on
    `main`: a guard that refused every `[bot]` with no boundary would fail
    the next back-merge pull request and block the release. So the boundary
    is a positive control in its own right — but one that a single commit
    bounds, or it is the old exemption wearing a date.

    The fixture is a repository of its own, so the verdicts do not depend on
    which commits this clone happens to carry.
    """

    def setUp(self) -> None:
        import subprocess
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._git("init", "-q", "-b", "main")
        # Background maintenance in a scratch repository can still be running
        # when the temp dir is removed, and the test then dies while tidying
        # up rather than on its subject (#2356).
        self._git("config", "gc.auto", "0")
        self._git("config", "maintenance.auto", "false")
        self._git("config", "commit.gpgsign", "false")

        self.published = self._commit("published.txt", "dependabot[bot] <b@b.io>")
        self.boundary = self._commit("boundary.txt", "Wiscale <w@example.org>")
        self.fresh = self._commit("fresh.txt", "dependabot[bot] <b@b.io>")

        self._real_boundary = caa.GRANDFATHERED_THROUGH
        caa.GRANDFATHERED_THROUGH = self.boundary

    def tearDown(self) -> None:
        caa.GRANDFATHERED_THROUGH = self._real_boundary
        self._tmp.cleanup()

    def _git(self, *args: str) -> str:
        import subprocess

        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()

    def _commit(self, name: str, identity: str) -> str:
        who, email = identity.split(" <")
        email = email.rstrip(">")
        (self.root / name).write_text("x\n")
        self._git("add", name)
        self._git(
            "-c",
            f"user.name={who}",
            "-c",
            f"user.email={email}",
            "commit",
            "-q",
            "-m",
            f"chore: {name}",
        )
        return self._git("rev-parse", "HEAD")

    def test_a_bot_commit_at_or_before_the_boundary_is_forgiven(self) -> None:
        self.assertTrue(caa.commit_is_grandfathered(self.published, str(self.root)))
        self.assertEqual([], caa.audit(self.boundary, str(self.root)))

    def test_a_bot_commit_after_the_boundary_is_refused(self) -> None:
        self.assertFalse(caa.commit_is_grandfathered(self.fresh, str(self.root)))
        violations = caa.audit(f"{self.boundary}..{self.fresh}", str(self.root))
        self.assertEqual(1 * 2, len(violations), violations)  # author and committer
        self.assertIn("`[bot]` identity", violations[0])

    def test_an_absent_boundary_forgives_nothing(self) -> None:
        # A clone too shallow to carry the boundary, and any repository that
        # is not this one: the guard must judge, not admit. `audit` would
        # otherwise return [] here and the refusal vectors would read exit 0.
        caa.GRANDFATHERED_THROUGH = "0" * 40
        self.assertFalse(caa.commit_is_grandfathered(self.published, str(self.root)))
        self.assertNotEqual([], caa.audit(self.published, str(self.root)))

    def test_the_pinned_boundary_is_a_commit_in_this_repository(self) -> None:
        # A boundary nothing resolves forgives nothing, silently: the amnesty
        # would be gone and the next back-merge would fail with no trace of
        # why. Asserted against the real repository, not the fixture.
        caa.GRANDFATHERED_THROUGH = self._real_boundary
        self.assertRegex(self._real_boundary, r"^[0-9a-f]{40}$")
        self.assertTrue(
            caa.commit_is_grandfathered(self._real_boundary, str(REPO_ROOT)),
            f"{self._real_boundary[:12]} is not a commit in this clone",
        )


class ShallowGraftTests(unittest.TestCase):
    """A depth fetch grafts a complete clone, and ancestry then lies.

    `commit_is_grandfathered` asks the graph a question only a complete graph
    can answer. `pr-governance.yml` checks out with `fetch-depth: 0` and then
    fetched the base branch again with `--depth`, which reads like a harmless
    optimisation and is not: the flag writes `.git/shallow`, and git stops
    following parent links past the graft even though the older objects are
    still in the clone. The mechanism test below is the control — the day git
    stops behaving this way, the YAML assertion that follows can be relaxed
    rather than obeyed out of superstition.
    """

    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _git(self, cwd: Path, *args: str) -> str:
        import subprocess

        return subprocess.run(
            ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
        ).stdout.strip()

    def test_a_depth_fetch_makes_a_complete_clone_answer_ancestry_wrongly(self) -> None:
        import subprocess

        src = self.root / "src"
        src.mkdir()
        self._git(src, "init", "-q", "-b", "main")
        self._git(src, "config", "gc.auto", "0")
        self._git(src, "config", "maintenance.auto", "false")
        for i in range(5):
            (src / f"f{i}").write_text("x\n")
            self._git(src, "add", f"f{i}")
            self._git(
                src, "-c", "user.name=U", "-c", "user.email=u@e", "commit", "-q", "-m", f"c{i}"
            )
        old = self._git(src, "rev-parse", "HEAD~4")

        clone = self.root / "clone"
        self._git(self.root, "clone", "-q", str(src), str(clone))
        self.assertEqual(
            0,
            subprocess.run(
                ["git", "merge-base", "--is-ancestor", old, "HEAD"], cwd=clone
            ).returncode,
            "a complete clone must answer ancestry correctly",
        )

        self._git(clone, "fetch", "-q", "origin", "main", "--depth=3")
        self.assertEqual(
            0,
            subprocess.run(
                ["git", "cat-file", "-e", f"{old}^{{commit}}"],
                cwd=clone,
                capture_output=True,
            ).returncode,
            "the object is still there -- which is what makes this silent",
        )
        self.assertNotEqual(
            0,
            subprocess.run(
                ["git", "merge-base", "--is-ancestor", old, "HEAD"],
                cwd=clone,
                capture_output=True,
            ).returncode,
            "git no longer grafts on a depth fetch: the YAML gate below may be relaxed",
        )

    def test_pr_governance_never_depth_fetches(self) -> None:
        import re

        text = (REPO_ROOT / ".github" / "workflows" / "pr-governance.yml").read_text()
        offenders = [
            line.strip()
            for line in text.splitlines()
            if re.search(r"^\s*git fetch\b.*--depth", line)
        ]
        self.assertEqual(
            [],
            offenders,
            "a depth fetch grafts the complete checkout, so every guard in this "
            "workflow reads a graph whose ancestry is cut",
        )

    def test_the_guard_suites_are_run_on_a_complete_checkout(self) -> None:
        text = (REPO_ROOT / ".github" / "workflows" / "gate-contracts.yml").read_text()
        head = text.split("npm-audit:")[0]
        self.assertIn(
            "fetch-depth: 0",
            head,
            "the attribution suite asserts the amnesty boundary is really in "
            "the clone, which a default depth-1 checkout does not carry",
        )
