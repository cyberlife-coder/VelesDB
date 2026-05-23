"""Tests for scripts/check-feature-claims.py.

The audit script cross-references capability keywords found in a crate's
source tree against capability claims in its README. A historical bug only
scanned `lib.rs` for the velesdb-python crate, while server / typescript /
integrations scanned the entire source tree — producing false MISSING gaps
for capabilities exposed by sub-modules (sparse, quantization, column_store).

These tests pin the contract: the Python crate audit must locate capability
keywords anywhere under `src/`, identically to how the server and TS audits
behave.
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-feature-claims.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> object:
    spec = importlib.util.spec_from_file_location("check_feature_claims", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cfc = _load_script()


class ScanRustSrcTests(unittest.TestCase):
    """Unit tests for the recursive Rust source scanner."""

    def test_finds_capability_keyword_in_submodule(self) -> None:
        """Capability keywords in any .rs file under src/ must be detected."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            (src / "lib.rs").write_text(
                "pub use collection::Collection;\npub use database::Database;\n",
                encoding="utf-8",
            )
            sub = src / "collection"
            sub.mkdir()
            (sub / "search.rs").write_text(
                "fn sparse_search() { /* bm25-backed sparse retrieval */ }\n",
                encoding="utf-8",
            )

            actual = cfc._scan_rust_src(src)
            self.assertIn("sparse", actual)

    def test_returns_empty_set_when_dir_missing(self) -> None:
        actual = cfc._scan_rust_src(Path("/nonexistent/path/that/does/not/exist"))
        self.assertEqual(actual, set())

    def test_aggregates_across_multiple_files(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            (src / "lib.rs").write_text("// empty\n", encoding="utf-8")
            (src / "database.rs").write_text(
                "fn create_metadata_collection() {}\nfn train_pq() {}\n",
                encoding="utf-8",
            )
            (src / "collection.rs").write_text(
                "fn sparse_search() {}\n",
                encoding="utf-8",
            )

            actual = cfc._scan_rust_src(src)
            self.assertIn("column_store", actual)
            self.assertIn("quantization", actual)
            self.assertIn("sparse", actual)


class AuditPythonOnRealRepoTests(unittest.TestCase):
    """Regression tests against the real velesdb-python source tree."""

    def test_no_missing_gaps_for_python_crate(self) -> None:
        """velesdb-python must not be reported as MISSING any documented capability.

        The Python crate exposes column_store via `create_metadata_collection`,
        quantization via `train_pq`, and sparse via `sparse_search`. Detection
        must locate these regardless of which sub-module hosts them.
        """
        result = cfc._audit_python(REPO_ROOT)
        missing = result.claimed - result.actual
        self.assertEqual(
            missing,
            set(),
            f"velesdb-python claims documented but missing in API: {sorted(missing)}",
        )


class FullAuditExitCodeTests(unittest.TestCase):
    """End-to-end: running the full audit must exit 0 on a clean tree."""

    def test_main_returns_zero(self) -> None:
        # Suppress stdout to keep test output clean.
        import contextlib
        import io

        with contextlib.redirect_stdout(io.StringIO()):
            exit_code = cfc.main()
        self.assertEqual(exit_code, 0, "check-feature-claims.py must exit 0")


if __name__ == "__main__":
    unittest.main()
