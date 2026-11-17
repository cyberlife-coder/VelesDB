#!/usr/bin/env python3
"""Fail on a vulnerable npm lockfile — and only on that.

`npm audit` exits 1 both when it finds an advisory and when it cannot reach
the registry's advisory endpoint, so the two are indistinguishable from the
exit code alone. On 2026-09-03 the npmjs.org bulk-advisory endpoint returned
503s and then timed out for eight hours; the `npm advisories` job went red
three times on `develop` and every failure read as "this lockfile is
vulnerable". None of them was: the lockfiles had not changed, and the same
content had been green at merge time hours earlier.

An advisory with no fixed version published can be ACCEPTED, by GHSA id,
in `scripts/npm-audit-accepted.json`, with a reason and a `review_by` date.
Acceptance is narrow and fails closed: a lockfile passes only when every
advisory at or above the level is attributable (npm's report names it) and
accepted, and an acceptance past its `review_by` date refuses again. A count
the report does not attribute to an advisory is never accepted.

A transport failure is not a security verdict. It proves nothing either way,
so this gate keeps failing closed — an unaudited lockfile is not an audited
one — but it says which of the two happened, and it retries first so a
transient 503 costs seconds instead of a merge window:

  exit 0   the audit ran and found nothing at or above --audit-level
  exit 1   the audit ran and found an advisory  (a real refusal)
  exit 75  no report: the registry stayed unreachable, or npm would not run
           (EX_TEMPFAIL, infrastructure)
  exit 2   the gate was called wrongly           (argparse's usage error)

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
import math
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import date, timedelta
from pathlib import Path

# npm's severity ladder, weakest first. `--audit-level=high` means "high and
# critical", so the threshold is an index into this list.
SEVERITIES = ["info", "low", "moderate", "high", "critical"]

EXIT_CLEAN = 0
EXIT_ADVISORY = 1
EXIT_UNREACHABLE = 75  # EX_TEMPFAIL
EXIT_USAGE = 2  # argparse's own code for a bad invocation


class Unreachable(Exception):
    """The audit did not run: no verdict was produced, only a transport error."""


DEFAULT_ACCEPTED = Path(__file__).resolve().parent / "npm-audit-accepted.json"
# How far ahead a `review_by` may be set. Past it a "dated" exception is a
# permanent one nobody is due to re-examine.
MAX_ACCEPTANCE_DAYS = 180


@dataclass(frozen=True)
class Acceptance:
    """One accepted advisory: why, and until when it is re-examined."""

    ghsa: str
    package: str
    reason: str
    review_by: date


def load_accepted(path: Path) -> list[Acceptance]:
    """The accepted advisories in `path` (a JSON list); none when it is absent.

    Raises `ValueError` for an entry without a GHSA id, a package, a reason or
    an ISO `review_by` date at most `MAX_ACCEPTANCE_DAYS` ahead: an acceptance nobody can re-examine is not one.
    """
    if not path.exists():
        return []
    entries = json.loads(path.read_text())
    if not isinstance(entries, list):
        raise ValueError(f"{path}: expected a JSON list of acceptances")
    accepted = []
    for entry in entries:
        try:
            ghsa, package, reason = (str(entry[key]) for key in ("ghsa", "package", "reason"))
            review_by = date.fromisoformat(str(entry["review_by"]))
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError(f"{path}: acceptance {entry!r} needs ghsa, package, reason and an ISO review_by ({exc})") from exc
        if not ghsa.startswith("GHSA-") or not reason.strip():
            raise ValueError(f"{path}: acceptance {entry!r} needs a GHSA id and a non-empty reason")
        horizon = date.today() + timedelta(days=MAX_ACCEPTANCE_DAYS)
        if review_by > horizon:
            raise ValueError(
                f"{path}: {ghsa} review_by {review_by.isoformat()} is past "
                f"{horizon.isoformat()}, the {MAX_ACCEPTANCE_DAYS}-day horizon"
            )
        accepted.append(Acceptance(ghsa, package, reason, review_by))
    return accepted


@dataclass(frozen=True)
class Report:
    """A completed audit: its per-severity counts and the raw JSON report."""

    counts: dict[str, int]
    text: str

    def advisories(self, blocking: list[str]) -> set[str] | None:
        """The GHSA ids of the advisories at a blocking severity, or `None`
        when the report does not attribute one of them to a GHSA advisory."""
        packages = json.loads(self.text).get("vulnerabilities")
        ids: set[str] = set()
        for package in (packages if isinstance(packages, dict) else {}).values():
            vias = package.get("via", []) if isinstance(package, dict) else []
            for via in vias:
                if isinstance(via, dict) and via.get("severity") in blocking:
                    ghsa = str(via.get("url", "")).rsplit("/", 1)[-1]
                    if not ghsa.startswith("GHSA-"):
                        return None
                    ids.add(ghsa)
        return ids or None


def severities_at_or_above(level: str) -> list[str]:
    """The severities a given `--audit-level` is meant to catch."""
    if level not in SEVERITIES:
        raise ValueError(f"unknown audit level {level!r}; expected one of {SEVERITIES}")
    return SEVERITIES[SEVERITIES.index(level) :]


def classify(stdout: str) -> dict[str, int]:
    """Return the per-severity counts of a completed audit.

    Raises `Unreachable` for anything that is not a completed audit report —
    unparseable output, npm's error payload, which has no `metadata`, or a
    count that is not a number.
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
    try:
        return {name: int(counts.get(name, 0) or 0) for name in SEVERITIES}
    except (TypeError, ValueError, OverflowError) as exc:
        raise Unreachable(f"npm's report carries a count that is not a number ({exc})") from exc


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
    except OSError as exc:
        raise Unreachable(f"npm could not be run ({exc})") from exc
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
) -> Report:
    """Audit `root`, retrying only the attempts that produced no verdict.

    An advisory is a stable fact about the lockfile: it does not become false
    on a second try, so a verdict — clean or not — returns immediately. Only
    `Unreachable` is retried, with the delay doubling each time up to
    `MAX_SECONDS`, which also bounds the first.
    """
    if attempts < 1:
        raise ValueError("attempts must be at least 1")
    if not math.isfinite(backoff_seconds) or backoff_seconds < 0:
        raise ValueError(f"backoff_seconds must be finite and at least 0, got {backoff_seconds}")
    delay = min(backoff_seconds, MAX_SECONDS)
    last: Unreachable | None = None
    for attempt in range(1, attempts + 1):
        try:
            text = run_audit(npm, root, attempt_timeout)
            return Report(classify(text), text)
        except Unreachable as exc:
            last = exc
            if attempt < attempts:
                print(
                    f"npm audit produced no report (attempt {attempt}/{attempts}): "
                    f"{exc} — retrying in {delay:.0f}s",
                    file=sys.stderr,
                )
                sleep(delay)
                delay = min(delay * 2, MAX_SECONDS)
    assert last is not None  # noqa: S101 - the loop cannot exit without setting it
    raise last


def _attempt_budget(text: str) -> int:
    """argparse type for `--attempts`: a budget of zero audits nothing.

    Refused here, as a usage error, rather than by `audit_with_retries`'s
    ValueError: `main` mapped that one to the advisory exit, so a mistyped
    flag read as "this lockfile is vulnerable".
    """
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError(f"must be at least 1, got {value}")
    return value


# A day. Past a clock-dependent bound `time.sleep` and `subprocess.run` raise
# OverflowError (1e10 already does); nothing here needs longer than this.
MAX_SECONDS = 86_400.0


def _seconds(allow_zero: bool):
    """argparse type for a duration: a negative one made `time.sleep` raise
    ValueError after the first unreachable attempt, exiting 1 — the advisory
    code. So did `nan` (ValueError too), `inf` and a finite value too large for
    the clock (OverflowError), which `time.sleep` and `subprocess.run` raise
    on."""
    def parse(text: str) -> float:
        value = float(text)
        if not math.isfinite(value) or value > MAX_SECONDS:
            raise argparse.ArgumentTypeError(
                f"must be a finite number of seconds up to {MAX_SECONDS:.0f}, got {text}"
            )
        if value < 0 or (value == 0 and not allow_zero):
            bound = "at least 0" if allow_zero else "above 0"
            raise argparse.ArgumentTypeError(f"must be {bound}, got {value}")
        return value
    return parse


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="directory holding package-lock.json")
    parser.add_argument("--audit-level", default="high", choices=SEVERITIES)
    parser.add_argument("--attempts", type=_attempt_budget, default=4)
    parser.add_argument("--backoff-seconds", type=_seconds(allow_zero=True), default=5.0)
    parser.add_argument(
        "--attempt-timeout",
        type=_seconds(allow_zero=False),
        default=180.0,
        help="seconds to let one npm audit run before treating it as unreachable",
    )
    parser.add_argument(
        "--accepted",
        type=Path,
        default=DEFAULT_ACCEPTED,
        help="JSON list of accepted advisories (ghsa, reason, review_by)",
    )
    parser.add_argument("--npm", default="npm", help="npm executable (injected by the refusal vectors)")
    args = parser.parse_args(argv)

    root = Path(args.root)
    blocking = severities_at_or_above(args.audit_level)
    try:
        accepted = load_accepted(args.accepted)
    except (OSError, ValueError) as exc:
        print(f"::error::unusable advisory acceptances: {exc}", file=sys.stderr)
        return EXIT_USAGE

    try:
        report = audit_with_retries(
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

    found = {name: report.counts[name] for name in blocking if report.counts[name] > 0}
    if not found:
        print(f"npm advisories clean for {args.root} (level {args.audit_level}).")
        return EXIT_CLEAN
    return judge_findings(args.root, found, report.advisories(blocking), accepted)


def judge_findings(
    root: str,
    found: dict[str, int],
    advisories: set[str] | None,
    accepted: list[Acceptance],
) -> int:
    """The exit for a report with blocking findings: clean only when every one
    is an attributed advisory under an acceptance that is still in date."""
    detail = ", ".join(f"{count} {name}" for name, count in found.items())
    if advisories is None:
        print(
            f"::error::npm advisories in {root}: {detail}, not all attributed to a "
            f"GHSA advisory. Run `npm audit` there and update the lockfile.",
            file=sys.stderr,
        )
        return EXIT_ADVISORY
    by_id = {entry.ghsa: entry for entry in accepted}
    unaccepted = sorted(advisories - by_id.keys())
    expired = sorted(ghsa for ghsa in advisories & by_id.keys() if by_id[ghsa].review_by < date.today())
    if unaccepted or expired:
        reasons = []
        if unaccepted:
            reasons.append(f"not accepted: {', '.join(unaccepted)}")
        if expired:
            reasons.append(f"acceptance past its review_by date: {', '.join(expired)}")
        print(
            f"::error::npm advisories in {root}: {detail} ({'; '.join(reasons)}). "
            f"Run `npm audit` there and update the lockfile, or re-examine the "
            f"acceptance in scripts/npm-audit-accepted.json.",
            file=sys.stderr,
        )
        return EXIT_ADVISORY
    for ghsa in sorted(advisories):
        entry = by_id[ghsa]
        print(f"accepted {ghsa} ({entry.package}) until {entry.review_by.isoformat()}: {entry.reason}")
    print(f"npm advisories in {root}: {detail}, all accepted (level-blocking advisories: {len(advisories)}).")
    return EXIT_CLEAN


if __name__ == "__main__":
    raise SystemExit(main())
