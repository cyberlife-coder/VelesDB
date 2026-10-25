"""The semver gate must follow the version bump a release commit carries.

`ci.yml` declared `--release-type minor` for velesdb-core so that `develop`,
which holds the published version between releases, is not asked for a bump the
release commit performs. Hardcoded, the flag also hid a real major bump: the
7.0.0 release commit failed on the removals its own migration guide lists. The
step now drops the flag when the workspace version is ahead of crates.io.

The step's script is run for real under `bash`, with `cargo` and `curl`
replaced by stubs that report a workspace version and a crates.io index.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
STEP_NAME = "Check public API against the last published release"

CARGO_STUB = """#!/bin/sh
case "$1" in
  metadata) printf '{"packages":[{"name":"velesdb-core","version":"%s"}]}' "$WORKSPACE_VERSION" ;;
  semver-checks) echo "$@" >> "$CALLS" ;;
esac
"""
CURL_STUB = """#!/bin/sh
[ -n "$INDEX_FAILS" ] && exit 22
printf '%s' "$INDEX_BODY"
"""


def _step_script() -> str:
    workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))
    steps = workflow["jobs"]["semver-checks"]["steps"]
    return next(step["run"] for step in steps if step.get("name") == STEP_NAME)


def _index(*versions: str, yanked: tuple[str, ...] = ()) -> str:
    return "\n".join(json.dumps({"vers": v, "yanked": v in yanked}) for v in versions) + "\n"


def _calls_for(workspace: str, index_body: str = "", index_fails: bool = False) -> list[str]:
    with tempfile.TemporaryDirectory() as tmp:
        bin_dir = Path(tmp)
        for name, body in (("cargo", CARGO_STUB), ("curl", CURL_STUB)):
            path = bin_dir / name
            path.write_text(body, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        calls = bin_dir / "calls.log"
        env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "WORKSPACE_VERSION": workspace,
            "INDEX_BODY": index_body,
            "CALLS": str(calls),
        }
        if index_fails:
            env["INDEX_FAILS"] = "1"
        subprocess.run(["bash", "-e", "-c", _step_script()], env=env, check=True, capture_output=True)
        return calls.read_text(encoding="utf-8").splitlines()


def _core_call(calls: list[str]) -> str:
    return next(line for line in calls if "-p velesdb-core" in line)


class SemverReleaseTypeTests(unittest.TestCase):
    def test_develop_at_the_published_version_keeps_the_minor_declaration(self) -> None:
        calls = _calls_for("6.0.0", _index("5.2.0", "6.0.0"))
        self.assertIn("--release-type minor", _core_call(calls))

    def test_a_release_commit_ahead_of_crates_io_lets_the_tool_read_the_versions(self) -> None:
        calls = _calls_for("7.0.0", _index("5.2.0", "6.0.0"))
        self.assertNotIn("--release-type", _core_call(calls))

    def test_a_yanked_newer_release_is_not_the_baseline(self) -> None:
        calls = _calls_for("6.0.1", _index("6.0.0", "6.0.1", yanked=("6.0.1",)))
        self.assertNotIn("--release-type", _core_call(calls))

    def test_an_unreadable_index_keeps_the_declaration(self) -> None:
        calls = _calls_for("7.0.0", index_fails=True)
        self.assertIn("--release-type minor", _core_call(calls))

    def test_memory_is_always_checked_without_a_declared_type(self) -> None:
        calls = _calls_for("7.0.0", _index("6.0.0"))
        memory = next(line for line in calls if "-p velesdb-memory" in line)
        self.assertNotIn("--release-type", memory)


if __name__ == "__main__":
    unittest.main()
