"""`rust-toolchain.toml` decides which Rust builds this repository.

Its `channel` is the MSRV, and nothing else may state a different one:

* the workspace `rust-version`, which crates.io consumers read, equals it;
* a member crate inherits that field (`rust-version.workspace = true`) or
  leaves it out, and never declares its own;
* a workflow or composite action names no Rust version: CI installs from the
  toolchain file. The exception is a job that deliberately builds with
  nightly, and that job says why in a comment line of its own.

The third rule is there because the copies drifted. When it was written,
propagation-guard.yml installed 1.86 and perf-gate-e2e.yml 1.90 through
`toolchain:` inputs, and ci.yml and five other workflows restated 1.90 as
`RUST_VERSION`, while the MSRV was 1.90.

What the scan reads as naming a toolchain, on comment-stripped lines: the
value of a key whose name contains RUST, TOOLCHAIN or MSRV, `cargo +X`,
`rustc +X`, `rustdoc +X`, the argument of `rustup toolchain install`,
`install`, `default`, `override set` or `run`, `--toolchain X`, and a
`dtolnay/rust-toolchain` step with no `toolchain:` input, which installs the
action's own default rather than the file's. A `${{ ... }}` expression names
nothing. Workflows are read as text, not with PyYAML, so the suite needs
nothing beyond the standard library.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

#: A toolchain as rustup names it: a version or a channel, optionally dated or
#: followed by a host triple.
TOOLCHAIN_RE = re.compile(r"^(?:\d+\.\d+(?:\.\d+)?|stable|beta|nightly)(?:-[\w.-]+)?$", re.I)
NIGHTLY_RE = re.compile(r"^nightly(?:-[\w.-]+)?$", re.I)
KEY_RE = re.compile(r"^\s*(?:-\s+)?([\w-]+):\s*(\S.*)$")
TOOLCHAIN_KEY_RE = re.compile(r"rust|toolchain|msrv", re.I)
CLI_RE = re.compile(
    r"\b(?:cargo|rustc|rustdoc)\s+\+([\w.-]+)"
    r"|\brustup\s+(?:toolchain\s+install|install|default|override\s+set|run)\s+([\w.-]+)"
    r"|--toolchain[=\s]+([\w.-]+)"
)
ACTION_RE = re.compile(r"\buses:\s*dtolnay/rust-toolchain@([\w.-]+)")
COMMENT_RE = re.compile(r"^\s*#")
BANNER_RE = re.compile(r"^ {0,2}#")
STEP_RE = re.compile(r"^(\s*)- ")
JOB_RE = re.compile(r"^  ([\w-]+):\s*$")


def code(line: str) -> str:
    """The line without its comment."""
    return "" if COMMENT_RE.match(line) else re.sub(r"\s+#.*$", "", line)


def job_blocks(text: str) -> "list[tuple[str, list[tuple[int, str]]]]":
    """Each job's numbered lines, with the comment banner right above its key.

    Lines before the first job (a workflow's `env:`, or a whole composite
    action) form one more block, named `(top level)`.
    """
    blocks: "list[tuple[str, list[tuple[int, str]]]]" = [("(top level)", [])]
    in_jobs = False
    for number, line in enumerate(text.splitlines(), start=1):
        in_jobs = in_jobs or re.match(r"^jobs:\s*$", line) is not None
        match = JOB_RE.match(line) if in_jobs else None
        if match:
            previous, banner = blocks[-1][1], []
            while previous and (not previous[-1][1].strip() or BANNER_RE.match(previous[-1][1])):
                banner.insert(0, previous.pop())
            blocks.append((match.group(1), banner))
        blocks[-1][1].append((number, line))
    return blocks


def step_lines(lines: "list[str]", index: int) -> "list[str]":
    """The lines of the step that contains `lines[index]`."""
    start = index
    while start >= 0 and not STEP_RE.match(lines[start]):
        start -= 1
    if start < 0:
        return [lines[index]]
    indent = len(STEP_RE.match(lines[start]).group(1))
    end = start + 1
    while end < len(lines) and (not lines[end].strip() or len(lines[end]) - len(lines[end].lstrip()) > indent):
        end += 1
    return lines[start:end]


def names_in(block: "list[tuple[int, str]]") -> "list[tuple[int, str]]":
    """(line, toolchain) for each toolchain the block names."""
    lines = [line for _, line in block]
    found = []
    for index, (number, raw) in enumerate(block):
        line = code(raw)
        key = KEY_RE.match(line)
        if key and TOOLCHAIN_KEY_RE.search(key.group(1)):
            for value in key.group(2).strip("[]").split(","):
                if TOOLCHAIN_RE.match(value.strip().strip("'\"")):
                    found.append((number, value.strip().strip("'\"")))
        for match in CLI_RE.finditer(line):
            value = next(group for group in match.groups() if group)
            if TOOLCHAIN_RE.match(value):
                found.append((number, value))
        action = ACTION_RE.search(line)
        if action and not any(re.match(r"^\s*toolchain:", code(step)) for step in step_lines(lines, index)):
            ref = action.group(1)
            found.append((number, ref if TOOLCHAIN_RE.match(ref) else f"the action's default at @{ref[:12]}"))
    return found


def violations(text: str) -> "list[tuple[int, str, str]]":
    """(line, job, toolchain) for each name the job is not entitled to."""
    out = []
    for job, block in job_blocks(text):
        reasoned = any(COMMENT_RE.match(line) and "nightly" in line.lower() for _, line in block)
        out += [(n, job, name) for n, name in names_in(block) if not (NIGHTLY_RE.match(name) and reasoned)]
    return out


def declares_its_own_rust_version(manifest: dict) -> bool:
    return isinstance(manifest.get("package", {}).get("rust-version"), str)


def load(relative: str) -> dict:
    return tomllib.loads((REPO_ROOT / relative).read_text(encoding="utf-8"))


def workflow_files() -> "list[Path]":
    github = REPO_ROOT / ".github"
    return sorted(github.glob("workflows/*.y*ml")) + sorted(github.glob("actions/**/action.y*ml"))


def workflow(steps: str, top: str = "", banner: str = "") -> str:
    return f"on: push\n{top}jobs:\n{banner}  build:\n    runs-on: ubuntu-latest\n    steps:\n{steps}"


def named(text: str) -> "list[str]":
    return [name for _, _, name in violations(text)]


class ScannerTests(unittest.TestCase):
    """The scan, on synthetic workflows, before it judges the real ones."""

    def test_a_version_given_as_input_is_refused(self) -> None:
        steps = '      - uses: dtolnay/rust-toolchain@abc123\n        with:\n          toolchain: "1.86"\n'
        self.assertEqual(named(workflow(steps)), ["1.86"])

    def test_a_version_restated_in_env_is_refused_and_its_use_is_not(self) -> None:
        steps = "      - uses: dtolnay/rust-toolchain@abc123\n        with:\n          toolchain: ${{ env.RUST_VERSION }}\n"
        self.assertEqual(named(workflow(steps, top="env:\n  RUST_VERSION: '1.90'\n")), ["1.90"])

    def test_stable_is_refused_even_with_a_reason(self) -> None:
        steps = "      # stable, because the tool needs a newer compiler\n      - uses: dtolnay/rust-toolchain@abc123\n        with:\n          toolchain: stable\n"
        self.assertEqual(named(workflow(steps)), ["stable"])

    def test_nightly_with_a_reason_is_accepted(self) -> None:
        steps = "      # Nightly: miri runs nowhere else.\n      - uses: dtolnay/rust-toolchain@abc123 # nightly\n        with:\n          toolchain: nightly\n      - run: cargo +nightly miri test\n"
        self.assertEqual(named(workflow(steps)), [])

    def test_a_pin_label_is_not_a_reason(self) -> None:
        steps = "      - uses: dtolnay/rust-toolchain@abc123 # nightly\n        with:\n          toolchain: nightly\n      - run: cargo +nightly miri test\n"
        self.assertEqual(named(workflow(steps)), ["nightly", "nightly"])

    def test_the_reason_may_sit_in_the_banner_above_the_job(self) -> None:
        text = workflow("      - run: cargo +nightly test\n", banner="  # Loom needs nightly.\n")
        self.assertEqual(named(text), [])

    def test_the_action_without_an_input_installs_its_own_default(self) -> None:
        self.assertEqual(named(workflow("      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo test\n")), ["stable"])
        from_file = "      - uses: dtolnay/rust-toolchain@stable\n        with:\n          toolchain: ${{ steps.pin.outputs.channel }}\n"
        self.assertEqual(named(workflow(from_file)), [])

    def test_rustup_and_cargo_arguments_are_names(self) -> None:
        steps = "      - run: |\n          rustup toolchain install 1.86 --profile minimal\n          cargo +1.86 build\n"
        self.assertEqual(named(workflow(steps)), ["1.86", "1.86"])

    def test_a_commented_out_line_names_nothing(self) -> None:
        steps = '      # toolchain: "1.86"\n      - run: cargo test  # not cargo +1.86\n'
        self.assertEqual(named(workflow(steps)), [])


class MemberTests(unittest.TestCase):
    def test_a_declared_rust_version_is_its_own(self) -> None:
        self.assertTrue(declares_its_own_rust_version({"package": {"rust-version": "1.85"}}))

    def test_inheriting_or_omitting_the_field_is_not(self) -> None:
        self.assertFalse(declares_its_own_rust_version({"package": {"rust-version": {"workspace": True}}}))
        self.assertFalse(declares_its_own_rust_version({"package": {}}))


class RealTreeTests(unittest.TestCase):
    def test_the_toolchain_file_is_the_workspace_msrv(self) -> None:
        channel = load("rust-toolchain.toml")["toolchain"]["channel"]
        msrv = load("Cargo.toml")["workspace"]["package"]["rust-version"]
        self.assertEqual(channel, msrv, "rust-toolchain.toml and the workspace rust-version name different compilers")

    def test_no_member_declares_its_own_rust_version(self) -> None:
        members = load("Cargo.toml")["workspace"]["members"]
        self.assertTrue(members, "the workspace lists no member: this test would check nothing")
        own = [m for m in members if declares_its_own_rust_version(load(f"{m}/Cargo.toml"))]
        self.assertEqual(own, [], "these members declare a rust-version instead of inheriting the workspace's")

    def test_the_scan_reads_the_nightly_of_the_minimal_versions_job(self) -> None:
        # Positive control: a scan that read nothing would pass the sweep below.
        text = (REPO_ROOT / ".github" / "workflows" / "quality-deep.yml").read_text(encoding="utf-8")
        self.assertIn("nightly", [name for _, name in names_in(dict(job_blocks(text))["minimal-versions"])])
        self.assertNotIn("minimal-versions", [job for _, job, _ in violations(text)])

    def test_no_workflow_names_a_rust_version(self) -> None:
        found = [
            f"{path.relative_to(REPO_ROOT)}:{line}: `{job}` names `{name}`"
            for path in workflow_files()
            for line, job, name in violations(path.read_text(encoding="utf-8"))
        ]
        self.assertFalse(found, "install from rust-toolchain.toml instead (nightly needs a comment saying why):\n" + "\n".join(found))


if __name__ == "__main__":
    unittest.main()
