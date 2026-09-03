"""`check_binary_size.py` refuses an over-ceiling binary, and only that.

The registry used to declare this half untestable: "the smallest ceiling is
9 MiB, so a fixture exceeding one would mean carrying 9 MiB of literal bytes",
and concluded it was "exercised for real on every release, against the binaries
the release build produced". Both halves of that were wrong.

It was wrong that nothing could test it. The guard calls ``Path.stat().st_size``
— apparent size — so ``truncate`` produces a 13 MiB fixture that occupies no
blocks. Every fixture below is sparse; the whole module costs kilobytes.

And it was wrong that reality covered it. The release build did produce
over-ceiling binaries, the guard did print ``Binary size gate FAILED``, and
``binary-size.yml`` piped its exit status into ``tee`` and reported success
(#2193). The half declared "exercised for real" was the only half that could
never turn a build red, and nothing looked at it because the registry said
something already had.

The declared refusal vector in ``guards.json`` covers the MISSING half, where
literal fixture bytes are enough. This module covers the OVER half, plus the
boundary the ``<=`` comparison turns on.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
GUARD = REPO_ROOT / "scripts" / "check_binary_size.py"

MIB = 1024 * 1024

#: Mirrors `BINARIES` in the guard. Duplicated on purpose: a test that imports
#: the ceilings it checks cannot notice one being raised.
CEILINGS = {
    "velesdb-server": int(14.25 * MIB),
    "velesdb": int(11.5 * MIB),
    "velesdb-migrate": 9 * MIB,
}

REFUSED, ACCEPTED = 1, 0


def sparse(path: Path, size: int) -> None:
    """A file of apparent size ``size`` that occupies (almost) no blocks."""
    with open(path, "wb") as handle:
        handle.truncate(size)


class BinarySizeGateTests(unittest.TestCase):
    def run_guard(self, sizes: "dict[str, int]") -> "subprocess.CompletedProcess[str]":
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, size in sizes.items():
                sparse(root / name, size)
            return subprocess.run(
                [sys.executable, str(GUARD), "--target-dir", str(root)],
                capture_output=True,
                text=True,
                check=False,
            )

    def under(self) -> "dict[str, int]":
        """Every binary a megabyte inside its ceiling."""
        return {name: ceiling - MIB for name, ceiling in CEILINGS.items()}

    # -- the positive control -------------------------------------------------

    def test_binaries_within_their_ceilings_are_accepted(self) -> None:
        # Without this, a guard that refuses everything would pass every
        # refusal test below and break the build on every release.
        result = self.run_guard(self.under())
        self.assertEqual(result.returncode, ACCEPTED, result.stdout + result.stderr)
        self.assertIn("passed", result.stdout)

    # -- the half the registry called untestable ------------------------------

    def test_one_over_ceiling_binary_is_refused(self) -> None:
        sizes = self.under()
        sizes["velesdb-server"] = CEILINGS["velesdb-server"] + MIB
        result = self.run_guard(sizes)
        self.assertEqual(result.returncode, REFUSED, result.stdout + result.stderr)
        self.assertIn("FAILED", result.stdout)
        self.assertIn("velesdb-server", result.stdout)

    def test_what_6_0_0_actually_measures_is_accepted(self) -> None:
        """The release's own bytes, so a ceiling cannot drift under the build.

        These are the sizes `cargo build --release` produced for 6.0.0 on
        x86_64-unknown-linux-gnu. If a future edit lowers a ceiling below what
        the project ships, this fails before CI spends 3 minutes building to
        find out.
        """
        result = self.run_guard(
            {"velesdb-server": 14345984, "velesdb": 11579160, "velesdb-migrate": 9037128}
        )
        self.assertEqual(result.returncode, ACCEPTED, result.stdout + result.stderr)

    def test_two_over_one_under_names_only_the_two(self) -> None:
        sizes = self.under()
        sizes["velesdb-server"] = CEILINGS["velesdb-server"] + MIB
        sizes["velesdb"] = CEILINGS["velesdb"] + MIB
        result = self.run_guard(sizes)
        self.assertEqual(result.returncode, REFUSED, result.stdout + result.stderr)
        verdict = result.stdout.split("FAILED")[1]
        self.assertIn("velesdb-server:", verdict)
        # The one inside its ceiling must not be dragged into the refusal.
        self.assertNotIn("velesdb-migrate:", verdict)

    def test_every_binary_is_weighed_not_just_the_first(self) -> None:
        # A loop that returned on its first finding would report one name and
        # leave the operator to rediscover the rest on the next run.
        sizes = {name: ceiling + MIB for name, ceiling in CEILINGS.items()}
        result = self.run_guard(sizes)
        self.assertEqual(result.returncode, REFUSED, result.stdout)
        for name in CEILINGS:
            self.assertIn(f"{name}: ", result.stdout.split("FAILED")[1])

    # -- the boundary the `<=` turns on ---------------------------------------

    def test_a_binary_exactly_at_its_ceiling_is_accepted(self) -> None:
        sizes = self.under()
        sizes["velesdb"] = CEILINGS["velesdb"]
        self.assertEqual(self.run_guard(sizes).returncode, ACCEPTED)

    def test_a_single_byte_over_the_ceiling_is_refused(self) -> None:
        sizes = self.under()
        sizes["velesdb"] = CEILINGS["velesdb"] + 1
        self.assertEqual(self.run_guard(sizes).returncode, REFUSED)

    # -- the half the declared vector covers, pinned to exit 1 ----------------

    def test_a_missing_binary_is_refused_with_one_not_a_crash(self) -> None:
        # `guards.json` declares this vector, and the refusal harness reads
        # exit 1 specifically: a guard crashing on a missing path exits 2, and
        # a crash is not a refusal.
        sizes = self.under()
        del sizes["velesdb-migrate"]
        result = self.run_guard(sizes)
        self.assertEqual(result.returncode, REFUSED, result.stdout + result.stderr)
        self.assertIn("[MISS]", result.stdout)


if __name__ == "__main__":
    unittest.main()
