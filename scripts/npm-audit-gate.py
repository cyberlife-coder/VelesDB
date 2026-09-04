#!/usr/bin/env python3
"""Fail on a vulnerable npm lockfile — and only on that.

`npm audit` exits 1 both when it finds an advisory and when it cannot reach
the registry's advisory endpoint, so the two are indistinguishable from the
exit code alone. On 2026-09-03 the npmjs.org bulk-advisory endpoint returned
503s and then timed out for eight hours; the `npm advisories` job went red
three times on `develop` and every failure read as "this lockfile is
vulnerable". None of them was: the lockfiles had not changed, and the same
content had been green at merge time hours earlier.

A transport failure is not a security verdict. It proves nothing either way,
so this gate keeps failing closed — an unaudited lockfile is not an audited
one — but it says which of the two happened, and it retries first so a
transient 503 costs seconds instead of a merge window:

  exit 0   the audit ran and found nothing at or above --audit-level
  exit 1   the audit ran and found an advisory  (a real refusal)
  exit 75  the registry stayed unreachable      (EX_TEMPFAIL, infrastructure)

The discriminator is the report itself, not the exit code or a message match:
a completed audit carries `metadata.vulnerabilities` (a per-severity count
map). npm's failure payload — `{"message": ..., "error": {...}}` — does not.
Matching on the shape rather than on error text keeps the classification from
drifting with npm's wording.

`--npm` exists so the refusal vectors in scripts/guards.json can inject a tool
double and prove both branches without a network.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

# npm's severity ladder, weakest first. `--audit-level=high` means "high and
# critical", so the threshold is an index into this list.
SEVERITIES = ["info", "low", "moderate", "high", "critical"]

EXIT_CLEAN = 0
EXIT_ADVISORY = 1
EXIT_UNREACHABLE = 75  # EX_TEMPFAIL


class Unreachable(Exception):
    """The audit did not run: no verdict was produced, only a transport error."""


def severities_at_or_above(level: str) -> list[str]:
    """The severities a given `--audit-level` is meant to catch."""
    if level not in SEVERITIES:
        raise ValueError(f"unknown audit level {level!r}; expected one of {SEVERITIES}")
    return SEVERITIES[SEVERITIES.index(level) :]


def classify(stdout: str) -> dict[str, int]:
    """Return the per-severity counts of a completed audit.

    Raises `Unreachable` for anything that is not a completed audit report —
    unparseable output, or npm's error payload, which has no `metadata`.
    """
    try:
        report = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise Unreachable(f"npm produced no JSON report ({exc})") from exc
    if not isinstance(report, dict):
        raise Unreachable("npm's JSON report is not an object")
    counts = report.get("metadata", {})
    counts = counts.get("vulnerabilities") if isinstance(counts, dict) else None
    if not isinstance(counts, dict):
        message = report.get("message") or "no metadata.vulnerabilities in the report"
        raise Unreachable(str(message))
    return {name: int(counts.get(name, 0) or 0) for name in SEVERITIES}


def run_audit(npm: str, root: Path, timeout: float) -> str:
    """One audit attempt. Returns stdout; the exit code is deliberately ignored.

    npm exits 1 for an advisory and 1 for a dead endpoint, so the code carries
    no information this gate can act on — `classify` reads the report instead.

    The timeout is load-bearing, not a belt-and-braces default. The 2026-09-03
    outage did not answer with a fast 503 for most of its length: the endpoint
    accepted the connection and then hung until npm's own network timeout, five
    minutes per call. Unbounded attempts would have turned a retry budget into
    a twenty-minute job. A hung attempt is exactly as informative as a refused
    one — no report — so it is raised as `Unreachable` like any other.
    """
    try:
        completed = subprocess.run(  # noqa: S603 - argv is built here, never shell-parsed
            [npm, "audit", "--json", "--package-lock-only"],
            cwd=str(root),
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        raise Unreachable(f"npm audit produced no report within {timeout:.0f}s") from exc
    if completed.stderr.strip():
        print(completed.stderr.rstrip(), file=sys.stderr)
    return completed.stdout


def audit_with_retries(
    npm: str,
    root: Path,
    attempts: int,
    backoff_seconds: float,
    attempt_timeout: float = 180.0,
    sleep=time.sleep,
) -> dict[str, int]:
    """Audit `root`, retrying only the attempts that produced no verdict.

    An advisory is a stable fact about the lockfile: it does not become false
    on a second try, so a verdict — clean or not — returns immediately. Only
    `Unreachable` is retried, with the delay doubling each time.
    """
    if attempts < 1:
        raise ValueError("attempts must be at least 1")
    delay = backoff_seconds
    last: Unreachable | None = None
    for attempt in range(1, attempts + 1):
        try:
            return classify(run_audit(npm, root, attempt_timeout))
        except Unreachable as exc:
            last = exc
            if attempt < attempts:
                print(
                    f"npm audit produced no report (attempt {attempt}/{attempts}): "
                    f"{exc} — retrying in {delay:.0f}s",
                    file=sys.stderr,
                )
                sleep(delay)
                delay *= 2
    assert last is not None  # noqa: S101 - the loop cannot exit without setting it
    raise last


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="directory holding package-lock.json")
    parser.add_argument("--audit-level", default="high", choices=SEVERITIES)
    parser.add_argument("--attempts", type=int, default=4)
    parser.add_argument("--backoff-seconds", type=float, default=5.0)
    parser.add_argument(
        "--attempt-timeout",
        type=float,
        default=180.0,
        help="seconds to let one npm audit run before treating it as unreachable",
    )
    parser.add_argument("--npm", default="npm", help="npm executable (injected by the refusal vectors)")
    args = parser.parse_args(argv)

    root = Path(args.root)
    blocking = severities_at_or_above(args.audit_level)

    try:
        counts = audit_with_retries(
            args.npm,
            root,
            args.attempts,
            args.backoff_seconds,
            args.attempt_timeout,
        )
    except Unreachable as exc:
        print(
            f"::error::npm advisories could not be checked for {args.root}: the "
            f"registry's advisory endpoint stayed unreachable across "
            f"{args.attempts} attempts ({exc}). This is an infrastructure "
            f"failure, NOT a vulnerability finding — the lockfile is unaudited, "
            f"not known-bad. Re-run the job once the registry answers.",
            file=sys.stderr,
        )
        return EXIT_UNREACHABLE
    except ValueError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return EXIT_ADVISORY

    found = {name: counts[name] for name in blocking if counts[name] > 0}
    if found:
        detail = ", ".join(f"{count} {name}" for name, count in found.items())
        print(
            f"::error::npm advisories in {args.root}: {detail}. "
            f"Run `npm audit` there and update the lockfile.",
            file=sys.stderr,
        )
        return EXIT_ADVISORY

    print(f"npm advisories clean for {args.root} (level {args.audit_level}).")
    return EXIT_CLEAN


if __name__ == "__main__":
    raise SystemExit(main())
