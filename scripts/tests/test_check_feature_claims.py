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
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-feature-claims.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_feature_claims", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    # Register before exec so the module is reachable by name from any code
    # inside the script that introspects sys.modules (NamedTuple pickling,
    # dataclasses, importlib.reload, etc.).
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


cfc = _load_script()


class ScanRustSrcTests(unittest.TestCase):
    """Unit tests for the recursive Rust source scanner."""

    def test_scans_both_lib_rs_and_submodules(self) -> None:
        """Recursion must reach lib.rs AND sub-module files in the same pass.

        A regression that narrowed the glob to `*/**/*.rs` (skipping top-level
        lib.rs) would not be caught if the test only verified sub-module reach.
        Pin both anchors with distinct capability keywords.
        """
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            (src / "lib.rs").write_text(
                "use wgpu::Device;\npub use collection::Collection;\n",
                encoding="utf-8",
            )
            sub = src / "collection"
            sub.mkdir()
            (sub / "search.rs").write_text(
                "fn sparse_search() {}\n",
                encoding="utf-8",
            )

            actual = cfc._scan_rust_src(src)
            self.assertIn("gpu", actual, "lib.rs keyword (wgpu) must be detected")
            self.assertIn("sparse", actual, "submodule keyword (sparse_search) must be detected")

    def test_returns_empty_set_when_dir_missing(self) -> None:
        actual = cfc._scan_rust_src(Path("/nonexistent/path/that/does/not/exist"))
        self.assertEqual(actual, set())

    def test_returns_empty_set_when_path_is_a_file(self) -> None:
        """Defensive: a file path passed as src_dir must not crash with NotADirectoryError.

        Path.glob('**/*.rs') raises NotADirectoryError on a regular file, which
        would crash the audit instead of yielding an empty set. Real trigger
        scenarios: a crate src layout change leaves `src` as a stale file
        during a refactor, or a symlink points to a file.
        """
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=".not_a_dir") as f:
            actual = cfc._scan_rust_src(Path(f.name))
            self.assertEqual(actual, set())

    def test_does_not_follow_symlinks_out_of_src_tree(self) -> None:
        """`_scan_rust_src` must not follow symlinks — cycles cause infinite
        recursion, and external symlinks pull in code that isn't part of the
        crate's API surface.
        """
        import os
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            external = Path(tmp) / "external"
            external.mkdir()
            (external / "sparse.rs").write_text(
                "fn sparse_search() {}\n",
                encoding="utf-8",
            )

            src = Path(tmp) / "src"
            src.mkdir()
            (src / "lib.rs").write_text("// real source\n", encoding="utf-8")
            try:
                os.symlink(external, src / "linked")
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unsupported on this filesystem")

            actual = cfc._scan_rust_src(src)
            self.assertNotIn(
                "sparse",
                actual,
                "symlinked .rs files outside src tree must not contribute",
            )

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

    def test_ignores_capability_keywords_inside_cfg_test_blocks(self) -> None:
        """`#[cfg(test)]` mod blocks must not contribute to the API capability set.

        The audit's purpose is to verify documented capabilities are exposed by
        the public API. Test fixtures (variant names in error-mapping tests,
        helper assertions, mock data) are not API and must not register as
        capabilities. Otherwise a removed production feature can stay
        "documented" purely on the strength of a leftover test name.
        """
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            (src / "lib.rs").write_text(
                "pub fn real_thing() {}\n"
                "\n"
                "#[cfg(test)]\n"
                "mod tests {\n"
                "    fn exercise_gpu_error_mapping() {\n"
                "        // mentions GpuError + wgpu purely for mock coverage\n"
                "        let _wgpu = ();\n"
                "    }\n"
                "}\n",
                encoding="utf-8",
            )

            actual = cfc._scan_rust_src(src)
            self.assertNotIn(
                "gpu",
                actual,
                "gpu keyword inside #[cfg(test)] mod must not count as API",
            )

    def test_keeps_production_keywords_when_cfg_test_present(self) -> None:
        """Stripping `#[cfg(test)]` must not also strip production code above/below it.

        Production-side keywords in the same file as a test mod must still
        register. Otherwise the filter trades a false positive for a false
        negative — silently dropping real public-API signal.
        """
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            (src / "lib.rs").write_text(
                "pub fn sparse_search() {}\n"
                "\n"
                "#[cfg(test)]\n"
                "mod tests {\n"
                "    #[test]\n"
                "    fn anything() { let _x = 1; }\n"
                "}\n"
                "\n"
                "pub fn train_pq() {}\n",
                encoding="utf-8",
            )

            actual = cfc._scan_rust_src(src)
            self.assertIn("sparse", actual, "production fn before test mod must still register")
            self.assertIn("quantization", actual, "production fn after test mod must still register")


class AuditCrateBuilderTests(unittest.TestCase):
    """`_audit_crate` is the single AuditResult builder for every per-crate audit.

    All four Rust audit helpers (core, python, server, wasm) must route their
    result construction through it so that a future field added to AuditResult,
    or a future cross-cutting concern (logging, telemetry), is wired once and
    inherited by every crate. The previous design rebuilt the result inline in
    _audit_python and _audit_server, locking in drift.
    """

    def test_audit_crate_accepts_precomputed_actual_set(self) -> None:
        """The builder must accept a precomputed `actual` set so the scanner is
        decoupled from result construction — single-file (lib.rs) and recursive
        scanners both feed in via the same parameter."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            readme = Path(tmp) / "README.md"
            # Use phrases that match DOC_CLAIM_KEYWORDS: "knowledge graph" → graph,
            # "vector search" → search.
            readme.write_text("Supports knowledge graph traversal and vector search.\n", encoding="utf-8")

            result = cfc._audit_crate("test-crate", {"graph", "search"}, readme)

            self.assertEqual(result.name, "test-crate")
            self.assertEqual(result.actual, {"graph", "search"})
            self.assertIn("graph", result.claimed)
            self.assertIn("search", result.claimed)
            self.assertEqual(result.notes, [])

    def test_audit_crate_preserves_extra_notes(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            readme = Path(tmp) / "README.md"
            readme.write_text("test\n", encoding="utf-8")

            result = cfc._audit_crate(
                "x",
                set(),
                readme,
                extra_notes=["Note: feature X intentionally excluded"],
            )
            self.assertEqual(result.notes, ["Note: feature X intentionally excluded"])


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

    def test_gpu_not_reported_for_velesdb_python(self) -> None:
        """velesdb-python exposes no GPU API — the audit must not falsely detect it.

        Regression guard: previously `CoreError::GpuError("x")` inside a
        `#[cfg(test)] mod tests` block in exceptions.rs produced a spurious
        [UNDOC] gpu entry. The fix must keep test-only mentions out of the
        capability set.
        """
        result = cfc._audit_python(REPO_ROOT)
        self.assertNotIn(
            "gpu",
            result.actual,
            "gpu should not be detected for velesdb-python — it has no GPU API",
        )


if __name__ == "__main__":
    unittest.main()
