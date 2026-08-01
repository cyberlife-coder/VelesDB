"""A test target that no CI command can reach is documentation, not a gate.

`ci.yml` already states that doctrine, in the comment above its
`Test the velesdb-memory extract feature` step: `extract` was *checked* in
isolation but never *tested*, so its unit tests could not fail a build. That
step fixed one feature by hand. Nothing stopped the same hole from existing
elsewhere — and it did, for the whole `http` transport: six `[[test]]` targets
declare `required-features = ["http"]`, and no `cargo test` in any workflow
passed that feature, so fourteen tests covering TLS termination, graceful
SIGTERM drain and store-lock contention had never executed on CI.

That is the transport the daemon actually serves (`velesdb-memory --http`),
and the one every multi-client MCP setup connects through.

This suite closes the class rather than the instance: it reads the declared
targets out of `Cargo.toml` and the `cargo test` invocations out of the
workflows, and fails when a target's features are never enabled by any of
them. Adding a `required-features` target with no matching command turns it
red on the spot.

## Scope, and why it stops where it does

`velesdb-memory` only. `velesdb-core` also has `required-features` targets, and
its `gpu` ones are deliberately unreachable on CI — no runner has the hardware
— so pulling that crate in would demand an exemption list, and an exemption
list is a place for a genuine gap to hide behind a plausible reason. Widening
this is its own change, with its own arbitration.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CARGO = REPO / "crates" / "velesdb-memory" / "Cargo.toml"
WORKFLOWS = REPO / ".github" / "workflows"

#: The package whose targets must be reachable. See the module docstring.
PACKAGE = "velesdb-memory"

#: A `cargo test` line, with whatever follows it on the same line. Continuations
#: (`\` at end of line) are joined before matching, since the workspace test
#: command is written across several lines.
CARGO_TEST = re.compile(r"cargo\s+test\b([^\n]*)")

#: `--features a,b,c` or `--features "a,b"`, in either order relative to `-p`.
FEATURES = re.compile(r"--features[= ]+[\"']?([A-Za-z0-9_,\-/]+)[\"']?")


def declared_targets() -> list[tuple[str, frozenset[str]]]:
    """Every `[[test]]` of the package that declares `required-features`."""
    manifest = tomllib.loads(CARGO.read_text(encoding="utf-8"))
    return [
        (target["name"], frozenset(target["required-features"]))
        for target in manifest.get("test", [])
        if target.get("required-features")
    ]


def test_invocations() -> list[tuple[str, str, frozenset[str]]]:
    """Every `cargo test` in the workflows, as `(file, line, features)`.

    Only invocations that reach this package count: `-p velesdb-memory`, or a
    `--workspace` run. A `-p some-other-crate` cannot build these targets
    however many features it names.
    """
    found: list[tuple[str, str, frozenset[str]]] = []
    for workflow in sorted(WORKFLOWS.glob("*.yml")):
        # Join backslash continuations first: the workspace command spans
        # several lines, and matching line-by-line would read its `--features`
        # as belonging to no command at all.
        text = workflow.read_text(encoding="utf-8").replace("\\\n", " ")
        for match in CARGO_TEST.finditer(text):
            command = match.group(1)
            reaches_package = f"-p {PACKAGE}" in command or "--workspace" in command
            if not reaches_package:
                continue
            features = FEATURES.search(command)
            enabled = frozenset(features.group(1).split(",")) if features else frozenset()
            found.append((workflow.name, command.strip(), enabled))
    return found


class TestTargetReachability(unittest.TestCase):
    def test_the_manifest_still_declares_gated_targets(self) -> None:
        """Guard the guard: if the manifest stops declaring any, this suite
        would pass vacuously and report nothing forever."""
        self.assertTrue(
            declared_targets(),
            f"{CARGO} declares no `required-features` test target — either the "
            "manifest changed shape or this suite is reading the wrong file, "
            "and either way it is no longer checking anything",
        )

    def test_the_workflows_still_contain_cargo_test_commands(self) -> None:
        """The other half of the same worry: a parser that finds no command at
        all would report every target as unreachable, which reads like a real
        finding and is not one."""
        self.assertTrue(
            test_invocations(),
            "no `cargo test` reaching velesdb-memory was found in any "
            "workflow — the parser is broken, not the CI",
        )

    def test_every_gated_test_target_is_reached_by_some_cargo_test(self) -> None:
        invocations = test_invocations()
        unreachable = []
        for name, required in declared_targets():
            if not any(required <= enabled for _, _, enabled in invocations):
                unreachable.append(f"{name} (needs {', '.join(sorted(required))})")
        self.assertFalse(
            unreachable,
            "these test targets are compiled by nothing CI runs, so their "
            "tests cannot fail a build:\n  "
            + "\n  ".join(sorted(unreachable))
            + "\n\nAdd a `cargo test -p velesdb-memory --features <…>` step "
            "covering them, or delete the targets. A test that cannot fail "
            "the build is documentation, not a gate.",
        )


if __name__ == "__main__":
    unittest.main()
