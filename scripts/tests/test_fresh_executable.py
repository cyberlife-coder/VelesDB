"""`fresh_executable`: a new fake is launched once, with no trace, before any deadline (#2284)."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts.tests import test_install_memory_daemon as installer_suite
from scripts.tests.fresh_executable import WARM_UP_ENV, guarded, warm_up, write_warm_executable

POSIX_ONLY = unittest.skipUnless(os.name == "posix", "the warm-up launches only on POSIX")


class FreshExecutableTests(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def test_guarded_refuses_a_script_without_a_shebang(self) -> None:
        with self.assertRaisesRegex(ValueError, "shebang"):
            guarded("exit 0\n")

    def test_guarded_keeps_the_shebang_first(self) -> None:
        text = guarded("#!/bin/sh\nexit 7\n")
        self.assertEqual(text.splitlines()[0], "#!/bin/sh")
        self.assertIn(WARM_UP_ENV, text.splitlines()[1])
        self.assertEqual(text.splitlines()[2], "exit 7")

    @POSIX_ONLY
    def test_the_warm_up_launch_leaves_no_trace_of_the_body(self) -> None:
        """The warm-up exits at the guard: the body's side effect never happens."""
        trace = self.dir / "trace"
        write_warm_executable(self.dir / "fake", f"#!/bin/sh\necho ran >> '{trace}'\nexit 3\n")
        self.assertFalse(trace.exists(), "the warm-up ran the fake's body")

    @POSIX_ONLY
    def test_the_fake_still_runs_its_body_outside_the_warm_up(self) -> None:
        """The guard is inert without the warm-up variable: the positive control."""
        fake = write_warm_executable(self.dir / "fake", "#!/bin/sh\nexit 3\n")
        env = {k: v for k, v in os.environ.items() if k != WARM_UP_ENV}
        self.assertEqual(subprocess.run([str(fake)], env=env, check=False).returncode, 3)

    @POSIX_ONLY
    def test_a_fake_that_fails_its_warm_up_is_reported(self) -> None:
        """An unguarded file that exits non-zero is a broken fake, named at once."""
        fake = self.dir / "fake"
        fake.write_text("#!/bin/sh\nexit 3\n", encoding="utf-8")
        fake.chmod(0o755)
        with self.assertRaisesRegex(RuntimeError, "exited 3"):
            warm_up(fake)

    def test_the_installer_suite_writes_its_fakes_through_the_helper(self) -> None:
        """The wiring: the suite whose 20 s deadlines #2284 names uses `write_warm_executable`."""
        case = installer_suite.InstallerHarness("setUp")
        case.fake_bin = self.dir
        fake = case._write_executable("probe", "#!/bin/sh\nexit 0\n")
        self.assertIn(WARM_UP_ENV, fake.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
