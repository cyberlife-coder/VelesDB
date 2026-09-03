#!/usr/bin/env python3
"""Extract one version's section from a Keep-a-Changelog file.

Release notes that say nothing are the default outcome of the velesdb-memory
train: `release-memory.yml` and `release-mcpb.yml` both create the GitHub
Release for the tag, each with its own one-line blurb, and neither ever wrote
the CHANGELOG section. Whichever won the race decided what the release said.

This is what `release.yml` does inline for the workspace train, minus two
things its `sed` cannot do: it drops the last line of a section that has no
successor (`head -n -1` with nothing to trim), and it says nothing when the
version is absent, emitting an empty file instead of failing.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# `## [0.14.2] - 2026-09-03`, `## [0.14.2]`, and nothing else. The version sits
# in brackets so `0.1` can never match its way into `## [0.14.2]`.
SECTION_RE_TEMPLATE = r"^##\s+\[{version}\]"
ANY_SECTION_RE = re.compile(r"^##\s+\[", re.MULTILINE)


def extract(changelog: str, version: str) -> str:
    """Return the body of `version`'s section, without its own heading."""
    start = re.compile(
        SECTION_RE_TEMPLATE.format(version=re.escape(version)), re.MULTILINE
    ).search(changelog)
    if start is None:
        raise LookupError(f"no '## [{version}]' section in the changelog")

    body_start = changelog.find("\n", start.end())
    if body_start == -1:
        raise LookupError(f"section '## [{version}]' has no body")

    following = ANY_SECTION_RE.search(changelog, body_start)
    body_end = following.start() if following else len(changelog)

    body = changelog[body_start:body_end].strip("\n")
    if not body.strip():
        raise LookupError(f"section '## [{version}]' is empty")
    return body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--changelog", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, help="default: stdout")
    args = parser.parse_args()

    try:
        text = args.changelog.read_text(encoding="utf-8")
    except OSError as error:
        print(f"changelog-release-notes: {error}", file=sys.stderr)
        return 1

    try:
        notes = extract(text, args.version)
    except LookupError as error:
        print(f"changelog-release-notes: {error}", file=sys.stderr)
        return 1

    if args.output is None:
        print(notes)
    else:
        args.output.write_text(notes + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
