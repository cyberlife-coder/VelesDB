"""A freshly written executable, launched once before any test deadline starts (#2284).

macOS assesses a new executable on its first exec: syspolicyd's Gatekeeper scan runs
after `exec` has returned, so the delay lands inside whatever deadline the caller
already started. It measured about 0.25 s on an idle machine and 11 s for the last of
60 new files launched together, because the scans run one at a time; #2280's mutation
batches saw 40.5 s. The same file launched again answers in milliseconds. So a test
that writes an executable and then runs something under a deadline launches that
file once first, here, with a patience no first launch reaches. From then on the
test's own deadline measures only what the test is about.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

# Set only in the environment of the warm-up launch. A fake whose body has side effects
# answers it with `exit 0` before doing anything (`guarded`), so the warm-up leaves no trace.
WARM_UP_ENV = "VELES_TEST_WARM_UP"

# Far above any first launch measured (40.5 s), so a slow assessment is waited for; a
# file that hangs still fails the test, only later.
WARM_UP_PATIENCE_S = 600


def guarded(script: str) -> str:
    """`script` with a first line that exits 0 at once when launched for warm-up."""
    shebang, newline, body = script.partition("\n")
    if not shebang.startswith("#!"):
        raise ValueError(f"a fake executable starts with a shebang, not {shebang!r}")
    return f'{shebang}{newline}[ -z "${{{WARM_UP_ENV}:-}}" ] || exit 0\n{body}'


def warm_up(path: Path) -> None:
    """Launch `path` once, so its first-exec assessment is paid before any test deadline.

    POSIX only: Windows runs no such assessment, and cannot exec a `#!/bin/sh` file. The
    launch must exit 0, the proof that the file ran through its interpreter; a file that
    does not is a broken fake, reported here rather than as a later, stranger failure.
    """
    if os.name != "posix":
        return
    done = subprocess.run(
        [str(path)],
        env={**os.environ, WARM_UP_ENV: "1"},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        timeout=WARM_UP_PATIENCE_S,
        check=False,
    )
    if done.returncode != 0:
        raise RuntimeError(
            f"warm-up launch of {path} exited {done.returncode}: {done.stderr[-500:]}"
        )


def write_warm_executable(path: Path, script: str) -> Path:
    """Write `script` to `path` as an executable, `guarded`, and launch it once (`warm_up`)."""
    path.write_text(guarded(script), encoding="utf-8")
    path.chmod(path.stat().st_mode | 0o111)
    warm_up(path)
    return path
