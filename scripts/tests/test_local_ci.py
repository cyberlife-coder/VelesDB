"""Tests for scripts/local-ci.sh.

The script exists because a hand-copied list of gate commands drifts from the
workflow, and the drift is silent: it shows up as a green local gate followed by
a red CI. So the property worth testing is not that it runs — it is that the
list it runs is **derived** from the workflow rather than baked in.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "local-ci.sh"


def fake_rust_toolchain(bin_dir: Path) -> dict[str, str]:
    """A cargo and a rustup on PATH. The default toolchain lacks the rustfmt
    component and the toolchain `full` has it: `cargo --list` names `fmt`
    either way (rustup's `cargo-fmt` proxy is always there), and only
    `rustup which` tells them apart. A listed subcommand that runs exits 0; an
    unknown one exits 101, as cargo does. No download, no real toolchain."""
    bin_dir.mkdir()
    rustup = bin_dir / "rustup"
    rustup.write_text(
        "#!/usr/bin/env bash\n"
        "[ \"$1\" = which ] || exit 0\n"
        "[ \"$2 $3\" = \"--toolchain full\" ] && { echo /toolchain/bin/\"${@: -1}\"; exit 0; }\n"
        "[ \"${@: -1}\" = cargo-fmt ] && { echo \"error: 'cargo-fmt' is not installed\" >&2; exit 1; }\n"
        "echo /toolchain/bin/\"${@: -1}\"\n",
        encoding="utf-8",
    )
    cargo = bin_dir / "cargo"
    cargo.write_text(
        "#!/usr/bin/env bash\n"
        "for a in \"$@\"; do [ \"$a\" = --list ] && { printf 'Installed Commands:\\n    build\\n    fmt\\n'; exit 0; }; done\n"
        "for a in \"$@\"; do [ \"$a\" = definitely-not-a-subcommand-xyz ] && { echo \"error: no such command\" >&2; exit 101; }; done\n"
        # rustup reads a toolchain only in first place; later, cargo sees a command.
        "for a in \"${@:2}\"; do case \"$a\" in +*) echo \"error: no such command: $a\" >&2; exit 101;; esac; done\n"
        "exit 0\n",
        encoding="utf-8",
    )
    for tool in (rustup, cargo):
        tool.chmod(0o755)
    (bin_dir / "cargo-fmt").symlink_to(rustup)
    return {"PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"}


def run(workflow: Path | None, *args: str, env_extra: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    # The synthetic workflows below declare `lint` only. The script's default
    # covers every gate job, and asking for an absent one is an error — see
    # `test_an_unknown_job_answers_2`. Scoping here keeps these tests about
    # derivation rather than about job coverage.
    env = dict(os.environ, JOBS="lint", **(env_extra or {}))
    if workflow is not None:
        env["WORKFLOW"] = str(workflow)
    return subprocess.run(
        [str(SCRIPT), *args], capture_output=True, text=True, cwd=REPO_ROOT, env=env
    )


def workflow_with(step_name: str, run_line: str, extra: str = "") -> str:
    return textwrap.dedent(f"""\
        name: synthetic
        jobs:
          lint:
            steps:
              - name: {step_name}
                run: {run_line}
        {extra}""")


class DerivationTests(unittest.TestCase):
    """A gate added to the workflow appears without touching the script."""

    def test_a_gate_only_the_workflow_knows_about_is_listed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Gate nobody hard-coded", "'echo ok'"), encoding="utf-8")
            result = run(wf, "--list")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Gate nobody hard-coded", result.stdout)

    def test_a_runner_only_step_is_reported_rather_than_omitted(self) -> None:
        # A gate skipped in silence is worth less than a gate that is absent:
        # the operator believes it ran.
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Needs the runner", "echo $GITHUB_SHA"), encoding="utf-8")
            result = run(wf, "--list")
            self.assertIn("SKIPPED", result.stdout)
            self.assertIn("Needs the runner", result.stdout)


class RefusalTests(unittest.TestCase):
    """It fails loudly rather than reporting a pass it never established."""

    def test_a_failing_gate_makes_the_script_fail(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Gate that fails", "'exit 1'"), encoding="utf-8")
            result = run(wf)
            self.assertEqual(result.returncode, 1)
            self.assertIn("Gate that fails", result.stderr)

    def test_a_missing_workflow_answers_2_not_1(self) -> None:
        # 2 is "cannot read", 1 is "refused". A tree this cannot read must not
        # wear the exit code of a refusal it never made.
        with tempfile.TemporaryDirectory() as tmp:
            result = run(Path(tmp) / "absent.yml")
            self.assertEqual(result.returncode, 2)

    def test_an_unknown_job_answers_2(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("x", "'echo ok'"), encoding="utf-8")
            env = dict(os.environ, WORKFLOW=str(wf), JOBS="does-not-exist")
            result = subprocess.run(
                [str(SCRIPT), "--list"], capture_output=True, text=True,
                cwd=REPO_ROOT, env=env,
            )
            self.assertEqual(result.returncode, 2)


class EmptyRunTests(unittest.TestCase):
    """Zero steps executed must never look like a pass."""

    def test_a_workflow_with_no_replayable_steps_answers_2(self) -> None:
        # Found by this very suite: an unexpected value crashed the reader, the
        # loop processed zero steps, and the script reported "all gates pass".
        # An operator would have believed the gates ran. Zero is now an error.
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(
                "name: synthetic\njobs:\n  lint:\n    steps:\n      - uses: actions/checkout@v4\n",
                encoding="utf-8",
            )
            result = run(wf)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("no steps read", result.stderr)

    def test_a_boolean_run_value_is_refused_rather_than_coerced(self) -> None:
        # `run: true` is a boolean to YAML. Coercing it produced "True", which
        # is not a command, so the gate reported a failure it had invented — a
        # verdict about nothing. A non-string `run:` is a malformed workflow.
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Boolean run", "true"), encoding="utf-8")
            result = run(wf)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("non-string", result.stderr)


class MissingToolTests(unittest.TestCase):
    """A tool this machine lacks is not a gate that refused."""

    def test_a_shell_style_missing_command_is_not_a_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Needs a tool", "'definitely-not-installed-xyz'"), encoding="utf-8")
            result = run(wf)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("TOOL MISSING", result.stdout)

    def test_a_missing_cargo_subcommand_is_not_a_failure(self) -> None:
        # `cargo machete` without cargo-machete exits 101, the code of any
        # failing cargo command. The replay tells them apart before running:
        # a subcommand cargo does not list exits 127, as a missing command does.
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Cargo subcommand", "cargo definitely-not-a-subcommand-xyz"), encoding="utf-8")
            result = run(wf)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("TOOL MISSING", result.stdout)

    def test_a_rustup_component_that_is_absent_is_a_missing_tool(self) -> None:
        # `cargo --list` names `fmt` whether or not rustfmt is installed: the
        # proxy is rustup's. Only `rustup which cargo-fmt` knows.
        with tempfile.TemporaryDirectory() as tmp:
            env = fake_rust_toolchain(Path(tmp) / "bin")
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Format", "cargo fmt --version"), encoding="utf-8")
            result = run(wf, env_extra=env)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("TOOL MISSING", result.stdout)
            # A toolchain that has the component runs it: the check asks that
            # toolchain, not the default one.
            wf.write_text(workflow_with("Format", "cargo +full fmt --version"), encoding="utf-8")
            result = run(wf, env_extra=env)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("ok", result.stdout)
            self.assertNotIn("TOOL MISSING", result.stdout)

    def replay(self, env: dict[str, str], tmp: str, command: str) -> subprocess.CompletedProcess:
        wf = Path(tmp) / "ci.yml"
        wf.write_text(workflow_with("Cargo", command), encoding="utf-8")
        return run(wf, env_extra=env)

    def test_a_toolchain_or_a_known_flag_before_a_missing_subcommand_is_seen_through(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = fake_rust_toolchain(Path(tmp) / "bin")
            for command in (
                "cargo +stable definitely-not-a-subcommand-xyz",
                "cargo --locked definitely-not-a-subcommand-xyz",
                "cargo -q --offline --frozen -v definitely-not-a-subcommand-xyz",
                "cargo +stable --quiet fmt",
                "cargo +stable -q fmt",
            ):
                with self.subTest(command=command):
                    result = self.replay(env, tmp, command)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn("TOOL MISSING", result.stdout)
            # A known prefix before a subcommand cargo has: it runs.
            for command in ("cargo +full build", "cargo --locked build"):
                with self.subTest(command=command):
                    result = self.replay(env, tmp, command)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertNotIn("TOOL MISSING", result.stdout)

    def test_any_other_option_runs_cargo_rather_than_being_guessed_at(self) -> None:
        # The replay keeps no table of the options that take a value: a value
        # read as the subcommand would report a gate missing and skip it
        # (`cargo --explain E0308`, `cargo -qZ unstable-options fmt`). Cargo
        # runs instead, and its own exit code is the verdict.
        with tempfile.TemporaryDirectory() as tmp:
            env = fake_rust_toolchain(Path(tmp) / "bin")
            for command in ("cargo --explain E0308", "cargo -qZ unstable-options fmt --version", "cargo -C crates build"):
                with self.subTest(command=command):
                    result = self.replay(env, tmp, command)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn("ok", result.stdout)
                    self.assertNotIn("TOOL MISSING", result.stdout)
            # A `+toolchain` counts only as the first argument, as rustup reads
            # it: anywhere later cargo refuses it as a command, and the gate fails.
            for command in (
                "cargo --config x=1 definitely-not-a-subcommand-xyz",
                "cargo -q +nightly fmt --version",
                "cargo --quiet +nightly definitely-not-a-subcommand-xyz",
            ):
                with self.subTest(command=command):
                    result = self.replay(env, tmp, command)
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn("FAILED", result.stdout)
                    self.assertNotIn("TOOL MISSING", result.stdout)

    def test_a_red_gate_whose_output_names_a_missing_tool_stays_red(self) -> None:
        # "Missing tool" is decided by the exit code, never by the output: a
        # guard quoting a doc line such as "... when the plugin is not
        # installed." failed, and was reported TOOL MISSING with exit 0.
        phrases = (
            "Search answers in 42 ms even when the plugin is not installed.",
            "bash: foo: command not found",
            "error: no such command: machete",
            "open: No such file or directory: missing.txt",
        )
        for phrase in phrases:
            for code in (1, 2, 101):
                with self.subTest(phrase=phrase, code=code), tempfile.TemporaryDirectory() as tmp:
                    wf = Path(tmp) / "ci.yml"
                    wf.write_text(
                        workflow_with("Red gate", f"'echo \"{phrase}\"; exit {code}'"), encoding="utf-8"
                    )
                    result = run(wf)
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn("FAILED", result.stdout)
                    self.assertNotIn("TOOL MISSING", result.stdout)


class DependencyTests(unittest.TestCase):
    """A missing PyYAML is announced, not surfaced as a traceback."""

    def test_a_missing_pyyaml_names_the_install_command(self) -> None:
        # Found in CI: setup-python ships a bare interpreter, so every
        # invocation died on ModuleNotFoundError. The exit code was already
        # right; the message was not, and an operator cannot act on a stack
        # trace. Shadowing the module reproduces the absence on a machine that
        # HAS it -- `test_a_gate_only_the_workflow_knows_about_is_listed` is
        # the control: same invocation, no shadow, exit 0.
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "yaml.py").write_text(
                "raise ImportError('shadowed by the test')", encoding="utf-8"
            )
            wf = Path(tmp) / "ci.yml"
            wf.write_text(workflow_with("Any gate", "'echo ok'"), encoding="utf-8")
            env = dict(os.environ, WORKFLOW=str(wf), JOBS="lint", PYTHONPATH=tmp)
            result = subprocess.run(
                [str(SCRIPT), "--list"], capture_output=True, text=True,
                cwd=REPO_ROOT, env=env,
            )
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("pip install pyyaml", result.stderr)
            self.assertNotIn("Traceback", result.stderr)


class EscapingTests(unittest.TestCase):
    """The bug this script shipped with, pinned so it cannot return."""

    def test_a_line_continuation_survives_the_round_trip(self) -> None:
        # Encoding the command through escape sequences turned a trailing `\`
        # into a literal `\` + `n`, and the shell received an argument `n`.
        # That produced a FALSE RED on clippy — the surest way to get a gate
        # switched off. The command is base64-carried now.
        with tempfile.TemporaryDirectory() as tmp:
            wf = Path(tmp) / "ci.yml"
            wf.write_text(textwrap.dedent("""\
                name: synthetic
                jobs:
                  lint:
                    steps:
                      - name: Continued command
                        run: |
                          test 1 -eq 1 \\
                            -a 2 -eq 2
                """), encoding="utf-8")
            result = run(wf)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
