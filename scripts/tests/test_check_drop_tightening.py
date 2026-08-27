"""Tests for scripts/check-drop-tightening.py.

Pins the guard's shape without invoking cargo — every case drives
`--from-json` with synthetic `--message-format=json` records, so the suite
runs in milliseconds and does not need a Rust toolchain:

* only `clippy::significant_drop_tightening` is counted, and only at a
  diagnostic's PRIMARY span — a secondary span pointing into another file
  must not charge that file;
* a file that gained findings is refused, with the before/after counts;
* a file with findings that the baseline does not list is refused;
* a shrink is refused too, asking for the baseline to be lowered — a stale
  entry above the true count would let a later regrowth hide under it;
* a baselined file with no findings left is refused, asking for the entry to
  be deleted;
* the checked-in baseline parses, and its shape (`path<TAB>count`) round-trips
  through `write_baseline`.

The end-to-end path — that clippy under `--force-warn` actually reports these
findings, and that the gate sees a newly introduced one — is verified by
running the script against the real tree; that is not repeated here because
it costs a full clippy build.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-drop-tightening.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_drop_tightening", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_drop_tightening"] = module
    spec.loader.exec_module(module)
    return module


cdt = _load_script()


def diagnostic(code: str, primary_file: str, *, secondary_file: str = None) -> str:
    """One cargo `compiler-message` JSON line."""
    spans = [{"file_name": primary_file, "is_primary": True}]
    if secondary_file is not None:
        spans.append({"file_name": secondary_file, "is_primary": False})
    return json.dumps(
        {
            "reason": "compiler-message",
            "message": {"code": {"code": code}, "level": "warning", "spans": spans},
        }
    )


class CountFromJsonTests(unittest.TestCase):
    def test_counts_only_the_lint_under_guard(self):
        stream = [
            diagnostic(cdt.LINT, "a.rs"),
            diagnostic("clippy::pedantic", "a.rs"),
            diagnostic("clippy::significant_drop_in_scrutinee", "a.rs"),
            diagnostic(cdt.LINT, "b.rs"),
            diagnostic(cdt.LINT, "b.rs"),
        ]
        self.assertEqual(cdt.count_from_json(stream), {"a.rs": 1, "b.rs": 2})

    def test_charges_only_the_primary_span(self):
        """A guard flagged in a.rs whose note points at b.rs must not charge b.rs."""
        stream = [diagnostic(cdt.LINT, "a.rs", secondary_file="b.rs")]
        self.assertEqual(cdt.count_from_json(stream), {"a.rs": 1})

    def test_ignores_non_message_records_and_noise(self):
        stream = [
            json.dumps({"reason": "compiler-artifact", "target": {"name": "x"}}),
            "   Compiling velesdb-core v5.2.0",
            "",
            "{not json at all",
            diagnostic(cdt.LINT, "a.rs"),
        ]
        self.assertEqual(cdt.count_from_json(stream), {"a.rs": 1})

    def test_a_diagnostic_without_a_primary_span_is_skipped(self):
        stream = [
            json.dumps(
                {
                    "reason": "compiler-message",
                    "message": {
                        "code": {"code": cdt.LINT},
                        "spans": [{"file_name": "a.rs", "is_primary": False}],
                    },
                }
            )
        ]
        self.assertEqual(cdt.count_from_json(stream), {})


class CompareTests(unittest.TestCase):
    def test_identical_counts_pass(self):
        self.assertEqual(cdt.compare({"a.rs": 3}, {"a.rs": 3}), [])

    def test_a_grown_file_is_refused_with_both_counts(self):
        problems = cdt.compare({"a.rs": 5}, {"a.rs": 3})
        self.assertEqual(len(problems), 1)
        self.assertIn("grew from 3 to 5", problems[0])
        self.assertIn("only ever shrinks", problems[0])

    def test_a_file_absent_from_the_baseline_is_refused(self):
        problems = cdt.compare({"a.rs": 1, "new.rs": 2}, {"a.rs": 1})
        self.assertEqual(len(problems), 1)
        self.assertIn("new.rs", problems[0])
        self.assertIn("baseline does not list", problems[0])

    def test_a_shrink_is_refused_so_the_baseline_cannot_go_stale(self):
        problems = cdt.compare({"a.rs": 1}, {"a.rs": 3})
        self.assertEqual(len(problems), 1)
        self.assertIn("shrank from 3 to 1", problems[0])
        self.assertIn("Lower its entry to 1", problems[0])

    def test_a_drained_file_must_leave_the_baseline(self):
        problems = cdt.compare({}, {"a.rs": 3})
        self.assertEqual(len(problems), 1)
        self.assertIn("none left", problems[0])
        self.assertIn("delete this line", problems[0])

    def test_every_drift_is_reported_not_just_the_first(self):
        problems = cdt.compare(
            {"grew.rs": 4, "new.rs": 1, "shrank.rs": 1},
            {"grew.rs": 2, "shrank.rs": 3, "gone.rs": 1},
        )
        self.assertEqual(len(problems), 4)


class BaselineFileTests(unittest.TestCase):
    def test_the_checked_in_baseline_parses(self):
        baseline = cdt.load_baseline(
            REPO_ROOT / "scripts" / "drop-tightening-baseline.txt"
        )
        self.assertGreater(len(baseline), 0, "the frozen baseline must not be empty")
        for path, count in baseline.items():
            self.assertTrue(
                path.startswith("crates/"),
                f"baseline path is not repo-relative: {path!r}",
            )
            self.assertGreater(count, 0, f"{path} is listed with a non-positive count")

    def test_a_malformed_line_is_a_clear_error(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "baseline.txt"
            bad.write_text("crates/a.rs 3\n", encoding="utf-8")  # space, not tab
            with self.assertRaises(ValueError) as caught:
                cdt.load_baseline(bad)
            self.assertIn("expected 'path<TAB>count'", str(caught.exception))

    def test_a_missing_baseline_is_a_clear_error(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError) as caught:
                cdt.load_baseline(Path(tmp) / "nope.txt")
            self.assertIn("missing", str(caught.exception))

    def test_write_then_load_round_trips(self):
        import tempfile

        counts = {"crates/b.rs": 2, "crates/a.rs": 1}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "baseline.txt"
            cdt.write_baseline(path, counts)
            self.assertEqual(cdt.load_baseline(path), counts)
            # Sorted on disk, so a regeneration produces a reviewable diff
            # rather than a reshuffle.
            self.assertEqual(
                path.read_text(encoding="utf-8").splitlines(),
                ["crates/a.rs\t1", "crates/b.rs\t2"],
            )


class ClippyCommandTests(unittest.TestCase):
    """The invocation is the guard's contract with CI; pin its load-bearing parts."""

    def test_force_warn_re_enables_the_workspace_allowed_lint(self):
        command = cdt.clippy_command()
        self.assertIn("--force-warn", command)
        self.assertEqual(
            command[command.index("--force-warn") + 1],
            cdt.LINT,
            "--force-warn must name the lint being counted",
        )
        self.assertLess(
            command.index("--"),
            command.index("--force-warn"),
            "--force-warn is a rustc flag and must follow the cargo/rustc separator",
        )

    def test_json_output_and_all_targets_are_requested(self):
        command = cdt.clippy_command()
        self.assertIn("--message-format=json", command)
        self.assertIn(
            "--all-targets",
            command,
            "CI lints --all-targets; a lib-only count would freeze a smaller "
            "backlog than the one CI would enforce",
        )

    def test_every_covered_package_is_named(self):
        command = cdt.clippy_command()
        for package in cdt.PACKAGES:
            self.assertIn(package, command)


class MainExitCodeTests(unittest.TestCase):
    def _run(self, argv, stdin_lines):
        stdin, sys.stdin = sys.stdin, io.StringIO("\n".join(stdin_lines))
        stdout, sys.stdout = sys.stdout, io.StringIO()
        try:
            return cdt.main(argv), sys.stdout.getvalue()
        finally:
            sys.stdin, sys.stdout = stdin, stdout

    def test_matching_counts_exit_zero(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            cdt.write_baseline(baseline, {"crates/a.rs": 1})
            code, out = self._run(
                ["--from-json", "-", "--baseline", str(baseline)],
                [diagnostic(cdt.LINT, "crates/a.rs")],
            )
            self.assertEqual(code, 0)
            self.assertIn("PASSED", out)

    def test_drift_exits_one(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            cdt.write_baseline(baseline, {"crates/a.rs": 1})
            code, out = self._run(
                ["--from-json", "-", "--baseline", str(baseline)],
                [diagnostic(cdt.LINT, "crates/a.rs"), diagnostic(cdt.LINT, "crates/a.rs")],
            )
            self.assertEqual(code, 1)
            self.assertIn("FAILED", out)
            self.assertIn("grew from 1 to 2", out)


if __name__ == "__main__":
    unittest.main()
