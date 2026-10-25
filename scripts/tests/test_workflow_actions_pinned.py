"""Every third-party action a workflow runs is pinned by full commit SHA.

A tag is a pointer its owner can move: `uses: actions/cache@v6` runs whatever
`v6` names on the day the job starts, so a compromised or force-moved tag
reaches every workflow that cites it, with the job's token and secrets in
reach. A 40-hex commit is content-addressed. The repository pins its actions
that way (`uses: owner/repo@<sha> # vX.Y.Z`) and Dependabot's `github-actions`
ecosystem moves the SHA and its comment together -- but nothing stopped the
next `uses:` line from arriving as a bare tag, and twenty did: the Docker
publish workflow, three `actions/cache` steps in ci.yml, the setup-python and
setup-node steps of gate-contracts.yml, and several `actions/checkout` steps
in the release workflows.

This suite is discovered by `gate-contracts.yml` (`unittest discover`), which
`ci.yml` calls as a required job, so a tag-pinned `uses:` fails `CI Success`.

What it allows: a local action or reusable workflow (`./...`), a remote action
or reusable workflow at a lower-case 40-hex commit, and a `docker://` image
pinned by `@sha256:` digest. What it refuses: everything else -- a tag, a
branch, a short SHA, a `docker://` tag. It does not require the trailing
`# vX.Y.Z` comment: that is what Dependabot reads, not what makes the pin safe.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
ACTIONS = REPO_ROOT / ".github" / "actions"
DEPENDABOT = REPO_ROOT / ".github" / "dependabot.yml"

#: A step's (or a job's) `uses:` key, with or without the list dash.
USES_RE = re.compile(r"^\s*(?:-\s+)?uses:\s*(?P<value>.*?)\s*$")
#: A full commit: 40 lower-case hex digits and nothing else.
SHA_RE = re.compile(r"[0-9a-f]{40}")
#: A container image pinned by content digest.
DIGEST_RE = re.compile(r"sha256:[0-9a-f]{64}")


def reference(value: str) -> str:
    """The `uses:` operand, without quotes and without a trailing comment."""
    value = value.strip()
    if value[:1] in {"'", '"'}:
        end = value.find(value[0], 1)
        return value[1:end] if end != -1 else value[1:]
    return value.split("#", 1)[0].strip()


def is_pinned(ref: str) -> bool:
    if ref.startswith("./"):
        return True  # in-repo: pinned by the commit that contains it
    if ref.startswith("docker://"):
        _, _, digest = ref.partition("@")
        return DIGEST_RE.fullmatch(digest) is not None
    _, at, rev = ref.rpartition("@")
    return bool(at) and SHA_RE.fullmatch(rev) is not None


def unpinned_uses(text: str) -> list[tuple[int, str]]:
    """`(line number, reference)` for every `uses:` that is not pinned."""
    found: list[tuple[int, str]] = []
    for number, line in enumerate(text.splitlines(), start=1):
        match = USES_RE.match(line)
        if match is None:
            continue
        ref = reference(match["value"])
        if not is_pinned(ref):
            found.append((number, ref))
    return found


def workflow_files() -> list[Path]:
    files = sorted(WORKFLOWS.glob("*.y*ml"))
    if ACTIONS.is_dir():  # composite actions run third-party steps too
        files += sorted(ACTIONS.rglob("*.y*ml"))
    return files


SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1"


class CheckerRefusals(unittest.TestCase):
    """The checker itself: it must refuse what it claims to refuse."""

    def refused(self, line: str) -> list[tuple[int, str]]:
        return unpinned_uses(line + "\n")

    def test_refuses_a_moving_major_tag(self) -> None:
        self.assertEqual(self.refused("      - uses: actions/cache@v6"), [(1, "actions/cache@v6")])

    def test_refuses_a_full_version_tag(self) -> None:
        self.assertTrue(self.refused("        uses: actions/setup-node@v7.1.0"))

    def test_refuses_a_branch(self) -> None:
        self.assertTrue(self.refused("      - uses: dtolnay/rust-toolchain@stable"))
        self.assertTrue(self.refused("      - uses: owner/repo@main"))

    def test_refuses_a_short_sha(self) -> None:
        self.assertTrue(self.refused("      - uses: owner/repo@3d3c42e"))

    def test_refuses_a_sha_one_digit_short_or_long(self) -> None:
        self.assertTrue(self.refused(f"      - uses: owner/repo@{SHA[:-1]}"))
        self.assertTrue(self.refused(f"      - uses: owner/repo@{SHA}0"))

    def test_refuses_an_upper_case_sha(self) -> None:
        self.assertTrue(self.refused(f"      - uses: owner/repo@{SHA.upper()}"))

    def test_refuses_a_missing_ref(self) -> None:
        self.assertTrue(self.refused("      - uses: owner/repo"))

    def test_a_trailing_comment_cannot_smuggle_a_sha_past_it(self) -> None:
        self.assertTrue(self.refused(f"      - uses: owner/repo@v1 # was {SHA}"))

    def test_refuses_a_quoted_tag(self) -> None:
        self.assertTrue(self.refused("      - uses: 'owner/repo@v1'"))
        self.assertTrue(self.refused('      - uses: "owner/repo@v1"'))

    def test_refuses_a_remote_reusable_workflow_at_a_tag(self) -> None:
        self.assertTrue(self.refused("    uses: owner/repo/.github/workflows/x.yml@v1"))

    def test_refuses_a_docker_image_by_tag(self) -> None:
        self.assertTrue(self.refused("      - uses: docker://alpine:3.20"))

    def test_reports_the_line_number(self) -> None:
        text = f"steps:\n  - uses: a/b@{SHA}\n  - uses: c/d@v2\n"
        self.assertEqual(unpinned_uses(text), [(3, "c/d@v2")])


class CheckerAcceptances(unittest.TestCase):
    def accepted(self, line: str) -> bool:
        return unpinned_uses(line + "\n") == []

    def test_accepts_a_full_sha_with_a_version_comment(self) -> None:
        self.assertTrue(self.accepted(f"      - uses: actions/checkout@{SHA} # v7.0.1"))

    def test_accepts_a_full_sha_without_a_comment(self) -> None:
        self.assertTrue(self.accepted(f"        uses: owner/repo@{SHA}"))

    def test_accepts_a_quoted_full_sha(self) -> None:
        self.assertTrue(self.accepted(f"      - uses: 'owner/repo@{SHA}'"))

    def test_accepts_a_remote_reusable_workflow_at_a_sha(self) -> None:
        self.assertTrue(self.accepted(f"    uses: owner/repo/.github/workflows/x.yml@{SHA}"))

    def test_accepts_a_local_action_and_a_local_reusable_workflow(self) -> None:
        self.assertTrue(self.accepted("      - uses: ./.github/actions/setup"))
        self.assertTrue(self.accepted("    uses: ./.github/workflows/doc-contract.yml"))

    def test_accepts_a_docker_image_pinned_by_digest(self) -> None:
        self.assertTrue(self.accepted(f"      - uses: docker://alpine@sha256:{'a' * 64}"))

    def test_ignores_commented_out_lines_and_other_keys(self) -> None:
        self.assertTrue(self.accepted("      # - uses: actions/cache@v6"))
        self.assertTrue(self.accepted("        name: uses: is only a word here"))
        self.assertTrue(self.accepted("        run: echo uses owner/repo@v1"))


class RepositoryWorkflows(unittest.TestCase):
    def test_there_are_workflows_to_scan(self) -> None:
        # A glob that matches nothing makes the test below vacuously green.
        self.assertGreater(len(workflow_files()), 10)

    def test_every_remote_action_is_pinned_by_full_commit_sha(self) -> None:
        offenders = [
            f"{path.relative_to(REPO_ROOT)}:{number}: {ref}"
            for path in workflow_files()
            for number, ref in unpinned_uses(path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(
            offenders,
            [],
            "pin each of these as `uses: owner/repo@<40-hex sha> # vX.Y.Z`, the "
            "commit the tag resolves to (`git ls-remote https://github.com/"
            "owner/repo.git refs/tags/<tag> refs/tags/<tag>^{}`; take the peeled "
            "commit for an annotated tag)",
        )

    def test_dependabot_keeps_the_pins_current(self) -> None:
        # A SHA pin never moves by itself: without the ecosystem below the
        # pins would rot instead of being bumped with their comment.
        text = DEPENDABOT.read_text(encoding="utf-8")
        self.assertRegex(text, r'package-ecosystem:\s*["\']?github-actions["\']?')


if __name__ == "__main__":
    unittest.main()
