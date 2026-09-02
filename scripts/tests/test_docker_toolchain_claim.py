#!/usr/bin/env python3
"""The server README states the Dockerfile's builder image; keep them equal.

`crates/velesdb-server/README.md` tells a reader which Rust image the
repository's `Dockerfile` builds with. Nothing was checking that claim, and it
drifted the moment it could: Dependabot's Docker bumps rewrite the `FROM` line
alone, so PR #2120 (1.97 -> 1.98) would have gone green leaving the README
naming a toolchain the build no longer uses.

`check-version-sync.py` does not cover this. It polices the *product* version
-- including the Dockerfile's `LABEL version="X.Y.Z"` -- against the Cargo
workspace. The builder image is a different axis with a different source of
truth (the `Dockerfile` itself), which is why this lives here rather than
being bolted onto that script's version-triple machinery.

Deliberately narrow: it asserts the two agree, not that either names a
particular version. A toolchain bump stays a one-line Dependabot diff plus the
README line it invalidates -- never a test edit.
"""

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DOCKERFILE = REPO_ROOT / "Dockerfile"
SERVER_README = REPO_ROOT / "crates" / "velesdb-server" / "README.md"

# The builder stage only. A multi-stage Dockerfile has several `FROM` lines and
# the runtime one (`debian:bookworm-slim`) is not a Rust image at all.
BUILDER_RE = re.compile(r"^FROM\s+(rust:[\w.-]+)\s+AS\s+builder", re.MULTILINE)
# The README names it inside backticks, in a prose table cell.
README_CLAIM_RE = re.compile(r"`(rust:[\w.-]+)`\s+builder")


def builder_image() -> str:
    """The Rust image the repository's Dockerfile actually builds with."""
    match = BUILDER_RE.search(DOCKERFILE.read_text(encoding="utf-8"))
    if match is None:
        raise AssertionError("no `FROM rust:... AS builder` line in the Dockerfile")
    return match.group(1)


def readme_claim() -> str:
    """The Rust image the server README tells a reader is used."""
    match = README_CLAIM_RE.search(SERVER_README.read_text(encoding="utf-8"))
    if match is None:
        raise AssertionError(
            "crates/velesdb-server/README.md no longer names a `rust:...` builder "
            "image -- drop this guard, or restore the claim it protects"
        )
    return match.group(1)


class DockerToolchainClaimTests(unittest.TestCase):
    def test_the_readme_names_the_image_the_dockerfile_builds_with(self) -> None:
        self.assertEqual(
            readme_claim(),
            builder_image(),
            "crates/velesdb-server/README.md names a different Rust builder image "
            "than the Dockerfile. A Dependabot Docker bump rewrites the `FROM` line "
            "and nothing else, so update the README line in the same PR.",
        )


class ClaimParserTests(unittest.TestCase):
    """RED-then-GREEN on synthetic text, per this suite's parser contract."""

    def test_builder_parser_ignores_the_runtime_stage(self) -> None:
        text = (
            "FROM rust:1.98-bookworm AS builder\n"
            "FROM debian:bookworm-slim AS runtime\n"
        )
        self.assertEqual(BUILDER_RE.search(text).group(1), "rust:1.98-bookworm")

    def test_readme_parser_reads_the_backticked_claim(self) -> None:
        cell = "| Docker | Supported | `Dockerfile`: `rust:1.98-bookworm` builder, ... |"
        self.assertEqual(README_CLAIM_RE.search(cell).group(1), "rust:1.98-bookworm")


if __name__ == "__main__":
    unittest.main()
