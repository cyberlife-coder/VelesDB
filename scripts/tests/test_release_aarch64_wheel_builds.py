#!/usr/bin/env python3
"""The aarch64 glibc wheel must be built, and built with the flag that lets it link.

Two separate ways to lose that wheel, both of which 5.2.0 demonstrated are
survivable by a green release run:

1. **The build fails.** ring's pregenerated ARM assembly refuses to assemble
   unless `__ARM_ARCH` is defined (`ring-core/asm_base.h:73`), and the
   manylinux2014 cross image's gcc does not define it. The job failed twice,
   identically, while the other twelve publish jobs succeeded — so the release
   reported success and PyPI simply had no `manylinux_2_17_aarch64` file (#2107).

2. **The entry is deleted.** Dropping the matrix row would also turn the run
   green, and would be the more tempting "fix" of the two. A wheel that is never
   built cannot fail to build.

So this pins both: the matrix row exists, *and* it carries the flag. Asserting
only the flag would let someone delete the row and still pass.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"

# The cc-rs env var that carries flags to an aarch64-unknown-linux-gnu build.
# Target-scoped by name: no other matrix entry reads it.
CFLAGS_VAR = "CFLAGS_aarch64_unknown_linux_gnu"


def wheels_job() -> str:
    """The text of the `publish-pypi-wheels:` job, up to the next job key."""
    text = WORKFLOW.read_text(encoding="utf-8")
    after = text.split("\n  publish-pypi-wheels:\n", 1)
    if len(after) != 2:
        raise AssertionError(
            "the `publish-pypi-wheels` job is gone from release.yml — if it was "
            "renamed, retarget this test rather than deleting it"
        )
    # Jobs are two-space indented; the next one ends this block.
    return re.split(r"\n  [a-z0-9-]+:\n", after[1], maxsplit=1)[0]


class ReleaseAarch64Wheel(unittest.TestCase):
    def test_the_glibc_aarch64_entry_is_still_in_the_matrix(self) -> None:
        entries = re.findall(r"^\s+- \{([^}]*)\}", wheels_job(), re.M)
        glibc_aarch64 = [
            e
            for e in entries
            if re.search(r"target:\s*aarch64\s*,", e + ",")
            and "musllinux" not in e
            and "manylinux" in e
        ]
        self.assertTrue(
            glibc_aarch64,
            "no aarch64 glibc wheel is built any more: `pip install velesdb` on "
            "Graviton and other glibc ARM64 hosts falls back to the sdist. A "
            "release with this row removed is green and still ships nothing.",
        )

    def test_the_aarch64_build_defines_arm_arch_for_ring(self) -> None:
        job = wheels_job()
        self.assertIn(
            CFLAGS_VAR,
            job,
            f"{CFLAGS_VAR} is unset, so ring's ARM assembly hits "
            f'`#error "ARM assembler must define __ARM_ARCH"` under the '
            f"manylinux2014 cross gcc and the aarch64 wheel never publishes (#2107)",
        )
        value = re.search(rf"^\s+{CFLAGS_VAR}:\s*(.+)$", job, re.M)
        assert value is not None  # implied by the assertIn above
        self.assertRegex(
            value.group(1).strip(),
            r"-D__ARM_ARCH=\d+",
            "the variable no longer defines __ARM_ARCH, which is the only reason "
            "it exists",
        )

    def test_the_flag_reaches_the_container(self) -> None:
        """maturin-action only forwards a documented set of env prefixes.

        The variable is set on the step, but the compile runs inside the
        manylinux docker container. `CFLAGS` is one of the action's
        ALLOWED_ENV_PREFIXES, which is what carries it across; a rename to
        something outside those prefixes would leave the build unfixed while
        this file still looked correct.
        """
        self.assertTrue(
            CFLAGS_VAR.startswith("CFLAGS"),
            "the variable must keep a CFLAGS prefix or maturin-action drops it "
            "at the docker boundary",
        )


if __name__ == "__main__":
    unittest.main()
