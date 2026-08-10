"""Tests for scripts/check-version-sync.py's velesdb-node README reader.

The README of `crates/velesdb-node/` is listed in that package's `files`, so
npm publishes it: it is the page npmjs.com renders for whichever version is
being released. Its footer was nonetheless outside every gate — the doc
freshness checker sweeps `docs/**` plus the root README, and this path had no
entry in the version-sync targets — and it drifted to announcing
`velesdb-node v0.11.2` / `@wiscale/velesdb-memory-node@0.11.1` in a tree
already bumped to 0.12.0. A published page telling readers to install a
version older than the one they are reading about.

Both halves are pinned RED-first below: the reader must reject the exact
stale footer that shipped, and must reject the two names disagreeing with
each other even when neither is compared to the manifest yet.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-version-sync.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_version_sync", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


cvs = _load_script()

FOOTER = (
    "`velesdb-node v{crate}` (npm `@wiscale/velesdb-memory-node@{npm}`) "
    "· Last updated: 2026-07-30 · Applies to: velesdb-core 4.2.0 "
    "· [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)\n"
)


def _readme(body: str) -> Path:
    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".md", delete=False, encoding="utf-8"
    )
    handle.write(body)
    handle.close()
    return Path(handle.name)


class NodeReadmeStampTests(unittest.TestCase):
    """`node_readme_stamp` reads BOTH names the footer announces."""

    def test_agreeing_footer_yields_the_single_version(self):
        path = _readme("# velesdb-node\n\n---\n\n" + FOOTER.format(crate="0.12.0", npm="0.12.0"))
        self.addCleanup(path.unlink)

        self.assertEqual(cvs._read_node_readme_stamp(path), "0.12.0")

    def test_the_exact_footer_that_shipped_stale_is_rejected(self):
        # Verbatim the state found on the branch: the crate name one patch
        # ahead of the npm name, both far behind the 0.12.0 manifest. Reading
        # either one alone would have returned a version and let the other rot.
        path = _readme("# velesdb-node\n\n---\n\n" + FOOTER.format(crate="0.11.2", npm="0.11.1"))
        self.addCleanup(path.unlink)

        with self.assertRaises(RuntimeError) as caught:
            cvs._read_node_readme_stamp(path)
        self.assertIn("one artifact, two versions", str(caught.exception))

    def test_a_missing_footer_is_an_error_not_a_silent_skip(self):
        path = _readme("# velesdb-node\n\nNo footer at all.\n")
        self.addCleanup(path.unlink)

        with self.assertRaises(RuntimeError):
            cvs._read_node_readme_stamp(path)


class LastUpdatedMemoryVersionTests(unittest.TestCase):
    """The parity matrix's header names two versions; both must be read.

    `doc_last_updated_version` captures the workspace half and stops, so the
    memory half went unread and sat at 0.11.0 while the body of the very same
    file documented a 0.12.0 change.
    """

    HEADER = "# Matrix\n\nLast updated: 2026-07-30 (v4.2.0; velesdb-memory {mem})\n"

    def test_reads_the_memory_half_not_the_workspace_half(self):
        path = _readme(self.HEADER.format(mem="0.12.0"))
        self.addCleanup(path.unlink)

        self.assertEqual(cvs._read_doc_last_updated_memory_version(path), "0.12.0")

    def test_the_stale_half_is_visible_to_the_reader(self):
        path = _readme(self.HEADER.format(mem="0.11.0"))
        self.addCleanup(path.unlink)

        # The value is returned, not raised: main() is what compares it to the
        # manifest. What matters here is that it is 0.11.0 and not 4.2.0 —
        # reading the workspace half by accident is how this drifted unseen.
        self.assertEqual(cvs._read_doc_last_updated_memory_version(path), "0.11.0")

    def test_a_header_without_the_memory_half_is_an_error(self):
        path = _readme("# Matrix\n\nLast updated: 2026-07-30 (v4.2.0)\n")
        self.addCleanup(path.unlink)

        with self.assertRaises(RuntimeError):
            cvs._read_doc_last_updated_memory_version(path)

    def test_the_parity_matrix_is_actually_policed(self):
        self.assertIn(
            ("docs/reference/ECOSYSTEM_PARITY.md", "doc_last_updated_memory_version"),
            cvs.MEMORY_TARGETS,
        )


class NodeReadmeIsActuallyPolicedTests(unittest.TestCase):
    """A reader nothing invokes protects nothing.

    These assert the WIRING, not the parsing: the README must appear in both
    target lists, so deleting an entry — the cheapest way to make a red gate
    green — turns this suite red instead.
    """

    README = "crates/velesdb-node/README.md"

    def test_the_node_readme_is_a_memory_version_target(self):
        self.assertIn(
            (self.README, "node_readme_stamp"),
            cvs.MEMORY_TARGETS,
            "the npm-published README must track the velesdb-memory version",
        )

    def test_the_node_readme_core_stamp_is_a_workspace_target(self):
        self.assertIn(
            (self.README, "applies_to_stamp"),
            cvs.TARGETS,
            "its `Applies to: velesdb-core X.Y.Z` stamp must track the workspace",
        )

    def test_the_reader_is_registered(self):
        self.assertIs(cvs._READERS["node_readme_stamp"], cvs._read_node_readme_stamp)


class CrateFooterStampTests(unittest.TestCase):
    """The `` `<crate> vX.Y.Z` `` footer every crate README ships.

    The 2026-08 audit found eight of these announcing 4.0/4.1/0.11-era
    versions in a 4.3.0/0.12.0 tree — hand-maintained, published by the
    registries, policed by nothing. The reader keys the crate name on the
    README's parent directory, so both failure modes below are pinned:
    a stale version AND a footer copy-pasted from another crate.
    """

    def _crate_readme(self, crate: str, body: str) -> Path:
        directory = Path(tempfile.mkdtemp()) / crate
        directory.mkdir()
        path = directory / "README.md"
        path.write_text(body, encoding="utf-8")
        return path

    def test_reads_the_version_of_the_named_crate(self):
        path = self._crate_readme(
            "velesdb-cli",
            "# CLI\n\n---\n\n`velesdb-cli v4.3.0` · Last updated: 2026-08-08\n",
        )
        self.assertEqual(cvs._read_crate_footer_stamp(path), "4.3.0")

    def test_a_footer_naming_another_crate_is_an_error(self):
        # Copy-pasted footer: the file lives in velesdb-server/ but announces
        # the CLI. Matching on the directory name makes this loud instead of
        # silently reading whichever version the wrong crate carries.
        path = self._crate_readme(
            "velesdb-server",
            "# Server\n\n---\n\n`velesdb-cli v4.3.0` · Last updated: 2026-08-08\n",
        )
        with self.assertRaises(RuntimeError):
            cvs._read_crate_footer_stamp(path)

    def test_a_missing_footer_is_an_error_not_a_silent_skip(self):
        path = self._crate_readme("velesdb-cli", "# CLI\n\nNo footer.\n")
        with self.assertRaises(RuntimeError):
            cvs._read_crate_footer_stamp(path)


class CrateFootersAreActuallyPolicedTests(unittest.TestCase):
    """Wiring assertions, same rationale as the node-README class above."""

    WORKSPACE_READMES = (
        "crates/velesdb-cli/README.md",
        "crates/velesdb-migrate/README.md",
        "crates/velesdb-mobile/README.md",
        "crates/velesdb-python/README.md",
        "crates/velesdb-server/README.md",
        "crates/velesdb-wasm/README.md",
        "crates/tauri-plugin-velesdb/README.md",
    )

    def test_every_workspace_crate_readme_footer_is_a_target(self):
        for readme in self.WORKSPACE_READMES:
            self.assertIn(
                (readme, "crate_footer_stamp"),
                cvs.TARGETS,
                f"{readme}'s footer must track the workspace version",
            )

    def test_the_core_readme_stamp_is_a_target(self):
        # velesdb-core's footer carries no crate-version half; its
        # `Applies to:` stamp is the pinned form instead.
        self.assertIn(("crates/velesdb-core/README.md", "applies_to_stamp"), cvs.TARGETS)

    def test_the_memory_readme_footer_is_a_memory_target(self):
        self.assertIn(
            ("crates/velesdb-memory/README.md", "crate_footer_stamp"),
            cvs.MEMORY_TARGETS,
            "the memory crate versions independently; its footer tracks 0.x",
        )

    def test_the_reader_is_registered(self):
        self.assertIs(cvs._READERS["crate_footer_stamp"], cvs._read_crate_footer_stamp)


if __name__ == "__main__":
    unittest.main()
