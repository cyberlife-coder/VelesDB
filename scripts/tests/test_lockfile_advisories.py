"""Every tracked lockfile is audited, and the list is derived, not written down.

`ci.yml`'s security job ran `cargo deny` once, at the repository root, so it
read the root `Cargo.lock` and nothing else. Six other lockfiles are tracked
and none was audited. Measured when #2321 bumped rustls for RUSTSEC-2026-0285:
the gate failed on the root lock while the Tauri demo's lock pinned a
vulnerable rustls and nothing reported it (#2322).

The point of this module is the DERIVATION. A step that names the six files it
audits is one commit away from being wrong again -- the seventh lockfile
escapes it exactly as the six escaped the root-only audit. So the step must
read `git ls-files`, and a literal path list here is a defect this refuses.
"""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"

#: A tracked lockfile path written out in the workflow, e.g. `fuzz/Cargo.lock`.
#: Deliberately matches the root one too: `Cargo.lock` alone is what the
#: original step audited implicitly, and naming it here would be the same
#: hard-coding one level down.
LOCK_PATH_RE = re.compile(r"[\w./-]*Cargo\.lock")


def security_job(text: str) -> str:
    """The `security:` job's lines, and only those."""
    marker = "\n  security:\n"
    if marker not in text:
        raise AssertionError("no `security:` job in ci.yml")
    rest = text[text.index(marker) + 1 :]
    following = re.search(r"\n  [A-Za-z0-9_-]+:\n", rest)
    return rest if following is None else rest[: following.start()]


def tracked_lockfiles() -> list[str]:
    """What git says is tracked -- the same question the workflow asks."""
    out = subprocess.run(  # noqa: S603 - fixed argv, no shell
        ["git", "ls-files", "*Cargo.lock"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [line for line in out.splitlines() if line.strip()]


class EveryTrackedLockfileIsAuditedTests(unittest.TestCase):
    def setUp(self) -> None:
        self.job = security_job(CI_WORKFLOW.read_text(encoding="utf-8"))

    def test_there_is_more_than_one_lockfile_to_audit(self) -> None:
        """The premise. With one lockfile the whole subject is moot."""
        locks = tracked_lockfiles()
        self.assertIn("Cargo.lock", locks, "git ls-files found no root lockfile -- parser broke")
        self.assertGreater(
            len(locks), 1, "only the root lockfile is tracked; this gate has nothing to add"
        )

    def test_the_job_derives_the_list_from_git(self) -> None:
        self.assertIn(
            "git ls-files",
            self.job,
            "the security job does not ask git which lockfiles exist, so a newly tracked "
            "one is audited by nothing (#2322)",
        )

    def test_the_job_names_no_lockfile_path(self) -> None:
        """A literal path is the hard-coding this replaces.

        Comments are stripped first: the rationale above the step cites
        `Cargo.lock` by name on purpose, and prose is not the thing that runs.
        """
        code = "\n".join(
            line for line in self.job.splitlines() if not line.lstrip().startswith("#")
        )
        # Two literals in the step are PATTERNS, not paths: the glob handed
        # to `git ls-files` and the anchored exclusion of the already-audited
        # root. Stripping them is what keeps this check about hard-coded
        # paths rather than about the derivation's own spelling.
        for pattern in ("'*Cargo.lock'", "grep -v '^Cargo.lock$'"):
            code = code.replace(pattern, "")
        named = sorted(set(LOCK_PATH_RE.findall(code)))
        self.assertEqual(
            [],
            named,
            f"the security job names lockfile paths instead of deriving them: {named}",
        )

    def test_an_empty_list_is_refused(self) -> None:
        """A loop over nothing exits 0 and reads exactly like a clean audit."""
        self.assertRegex(
            self.job,
            r'if \[ -z "\$locks" \]',
            "the step does not refuse an empty lockfile list, so it would pass by "
            "auditing nothing",
        )

    def test_one_failure_does_not_hide_the_next(self) -> None:
        """`set -e` plus a loop reports the first lockfile and stops."""
        self.assertIn(
            'failed="$failed $lock"',
            self.job,
            "the step stops at the first failing lockfile instead of auditing all of them",
        )

    def test_the_audit_uses_the_repository_policy(self) -> None:
        """Without `--config`, cargo-deny falls back to its own defaults.

        The exceptions this repository has reviewed and dated live in
        `deny.toml`; auditing an example against stock policy would report
        things the root is already excused for, and the gate would be noise.
        """
        self.assertRegex(
            self.job,
            r"--config \"?\$GITHUB_WORKSPACE/deny\.toml",
            "the per-lockfile audit does not pass the repository's deny.toml",
        )


class ABrokenGateDoesNotReadAsAFindingTests(unittest.TestCase):
    """cargo-deny exits 1 when it refuses and 2 when it could not run.

    The first CI run of this very step proved the distinction is not
    academic: CI installs a newer cargo-deny than a developer machine is
    likely to have, `--config` is a top-level flag there and a subcommand
    flag in 0.19, and each version rejects the other's spelling. The step
    reported six `advisories found in: ...` for what was a usage error, so a
    completely broken gate read exactly like a failing one.
    """

    def setUp(self) -> None:
        self.job = security_job(CI_WORKFLOW.read_text(encoding="utf-8"))

    def test_the_step_tells_a_refusal_from_a_breakage(self) -> None:
        self.assertIn(
            "could not run on:",
            self.job,
            "the step reports every non-zero exit as an advisory, so a broken "
            "cargo-deny invocation reads as a finding",
        )
        self.assertRegex(
            self.job,
            r"\n\s+1\) failed=",
            "only exit 1 may be counted as advisories found; every other code means "
            "the guard did not run",
        )

    def test_the_call_survives_bash_e(self) -> None:
        """GitHub runs `run:` blocks under `bash -e`.

        An unguarded failing call aborts the step before the first verdict is
        recorded, and the remaining lockfiles go unaudited while the log looks
        like a normal failure.
        """
        self.assertIn(
            "|| rc=$?",
            self.job,
            "the cargo-deny call is unguarded, so `bash -e` aborts the loop at the "
            "first non-clean lockfile",
        )

    def test_the_tool_version_is_pinned(self) -> None:
        """A floating install moves the argument grammar under the gate."""
        self.assertRegex(
            self.job,
            r"cargo install cargo-deny@\d+\.\d+\.\d+ --locked",
            "cargo-deny is installed unpinned; 0.19 and 0.20 disagree on where "
            "`--config` goes, so the next release can break this step with no "
            "change to the repository",
        )


class TheCheckRefusesTheShapeThisReplacedTests(unittest.TestCase):
    """The positive control, on the two shapes that would reopen the gap."""

    def test_a_hard_coded_list_is_refused(self) -> None:
        hard_coded = (
            "\n  security:\n"
            "    steps:\n"
            "      - run: |\n"
            "          for lock in fuzz/Cargo.lock examples/rust/Cargo.lock; do\n"
            "            cargo deny --manifest-path $(dirname $lock)/Cargo.toml check advisories\n"
            "          done\n"
            "\n  next-job:\n"
        )
        job = security_job(hard_coded)
        self.assertNotIn("git ls-files", job)
        code = "\n".join(line for line in job.splitlines() if not line.lstrip().startswith("#"))
        self.assertNotEqual([], sorted(set(LOCK_PATH_RE.findall(code))))

    def test_the_shipped_job_passes_the_same_check(self) -> None:
        job = security_job(CI_WORKFLOW.read_text(encoding="utf-8"))
        self.assertIn("git ls-files", job)
