"""The public trait surface guard, and the bug that nearly shipped inside it.

`cargo-semver-checks` 0.46.0 has no lint for a trait method's return type
changing between two NON-UNIT types, which is how #2293's
`io::Result<()>` -> `io::Result<usize>` passed a green gate (#2329). This
guard closes that by recording the public trait method surface and refusing
a change to it.

The tests feed synthetic listings through ``--api-file`` rather than running
cargo-public-api: the tool needs a nightly rustdoc build of the whole crate,
and a suite that costs a minute per case is a suite people stop running.
The wiring test at the bottom is what keeps the real tool in CI.

One of these cases exists because the first version of the script was WRONG
in a way that reported success. ``[A-Za-z0-9_:]+`` is greedy over the colon,
so ``pub trait Foo: Send`` yielded the path ``Foo:`` and none of Foo's
methods matched. Traits without bounds still worked, so the snapshot came
out at 14 methods of 83 and printed PASSED. A guard that silently covers a
sixth of its surface is worse than none, because it is believed.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-public-trait-api.py"
REPO_ROOT = SCRIPT_PATH.parent.parent
SNAPSHOT = REPO_ROOT / "scripts" / "public-trait-api.txt"
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"


def _load() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_public_trait_api", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


guard = _load()

BOUNDED = "pub trait velesdb_core::Widget: core::marker::Send + core::marker::Sync"
UNBOUNDED = "pub trait velesdb_core::Plain"
SURFACE = [
    BOUNDED,
    "pub fn velesdb_core::Widget::dump(&self, &str) -> core::io::error::Result<()>",
    "pub fn velesdb_core::Widget::len(&self) -> usize",
    UNBOUNDED,
    "pub fn velesdb_core::Plain::ping(&self)",
    "pub struct velesdb_core::NotATrait",
    "pub fn velesdb_core::NotATrait::inherent(&self) -> usize",
]


class ParserTests(unittest.TestCase):
    def test_a_bounded_trait_keeps_its_path(self) -> None:
        """The greedy-colon bug, pinned.

        `Foo: Send` must yield `velesdb_core::Widget`, never
        `velesdb_core::Widget:` — the second matches no method and passes.
        """
        self.assertEqual(
            ["velesdb_core::Widget", "velesdb_core::Plain"], guard.trait_paths(SURFACE)
        )

    def test_a_bounded_trait_s_methods_are_collected(self) -> None:
        found = guard.trait_methods(SURFACE)
        self.assertIn(
            "pub fn velesdb_core::Widget::dump(&self, &str) -> core::io::error::Result<()>", found
        )
        self.assertIn("pub fn velesdb_core::Plain::ping(&self)", found)

    def test_an_inherent_method_is_not_a_trait_method(self) -> None:
        """Scope: this guard is about implementors, not callers."""
        self.assertNotIn(
            "pub fn velesdb_core::NotATrait::inherent(&self) -> usize", guard.trait_methods(SURFACE)
        )

    def test_a_neighbouring_trait_does_not_adopt_methods(self) -> None:
        """`Foo` and `FooBar` are different traits.

        Matching on the path plus `::` is what keeps them apart; a bare
        prefix match would file `FooBar::baz` under `Foo`.
        """
        surface = [
            "pub trait velesdb_core::Foo",
            "pub fn velesdb_core::FooBar::baz(&self)",
        ]
        self.assertEqual([], guard.trait_methods(surface))


class CliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="trait-api-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))
        self.api = self.tmp / "api.txt"
        self.api.write_text("\n".join(SURFACE) + "\n", encoding="utf-8")
        self.snapshot = self.tmp / "surface.txt"

    def run_guard(self, *extra: str) -> "subprocess.CompletedProcess[str]":
        return subprocess.run(  # noqa: S603 - fixed argv, no shell
            [
                sys.executable,
                str(SCRIPT_PATH),
                "--root",
                str(self.tmp),
                "--snapshot",
                "surface.txt",
                "--api-file",
                str(self.api),
                *extra,
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_write_then_check_passes(self) -> None:
        self.assertEqual(0, self.run_guard("--write").returncode)
        done = self.run_guard()
        self.assertEqual(0, done.returncode, done.stderr)
        self.assertIn("PASSED", done.stdout)

    def test_a_changed_return_type_is_refused(self) -> None:
        """The #2329 shape: non-unit to non-unit, which semver-checks misses."""
        self.run_guard("--write")
        self.api.write_text(
            "\n".join(SURFACE).replace(
                "dump(&self, &str) -> core::io::error::Result<()>",
                "dump(&self, &str) -> core::io::error::Result<usize>",
            )
            + "\n",
            encoding="utf-8",
        )
        done = self.run_guard()
        self.assertEqual(1, done.returncode)
        self.assertIn("-pub fn velesdb_core::Widget::dump", done.stdout)
        self.assertIn("+pub fn velesdb_core::Widget::dump", done.stdout)

    def test_a_surface_with_no_trait_cannot_pass(self) -> None:
        """Empty compared to empty is not a clean check, it is no check.

        Exit 2, not 1: the guard could not run, which this repository keeps
        distinct from a refusal everywhere else.
        """
        self.api.write_text("pub struct velesdb_core::Alone\n", encoding="utf-8")
        done = self.run_guard()
        self.assertEqual(2, done.returncode)
        self.assertIn("no `pub trait`", done.stderr)

    def test_a_missing_snapshot_is_not_a_refusal(self) -> None:
        self.assertEqual(2, self.run_guard().returncode)


class ShippedSurfaceTests(unittest.TestCase):
    """The recorded surface must be real, and the job must run the guard."""

    def test_the_snapshot_is_recorded_and_plausible(self) -> None:
        self.assertTrue(SNAPSHOT.is_file(), "scripts/public-trait-api.txt is missing")
        lines = SNAPSHOT.read_text(encoding="utf-8").splitlines()
        self.assertGreater(len(lines), 50, f"only {len(lines)} methods recorded — the greedy-colon "
                           "bug produced exactly this shape, 14 of 83, and printed PASSED")
        self.assertTrue(all(line.startswith("pub fn ") for line in lines))

    def test_the_method_that_motivated_this_is_covered(self) -> None:
        recorded = SNAPSHOT.read_text(encoding="utf-8")
        self.assertIn(
            "NativeHnswBackend::file_dump",
            recorded,
            "the signature #2293 changed and the semver gate missed is not in the surface",
        )

    def test_a_job_runs_the_guard_and_its_verdict_is_required(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(
            "check-public-trait-api.py", ci, "no CI job runs the public trait surface guard"
        )
        self.assertIn(
            "needs.public-trait-api.result",
            ci,
            "`CI Success` never reads the guard's result, so it gates nothing",
        )
