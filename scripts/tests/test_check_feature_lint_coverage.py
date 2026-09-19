"""The feature-lint-coverage guard refuses the state the repository was in.

The first case below is not invented: it is `ci.yml`'s own pair of clippy
invocations before #2348, against a manifest declaring `internal-bench`, with
`crates/velesdb-core/src/internal_bench.rs` gated behind it. A guard that
cannot refuse the defect it was written for proves nothing.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "check_feature_lint_coverage.py"

MANIFEST = """[package]
name = "velesdb-core"

[features]
default = ["persistence"]
gpu = ["wgpu", "pollster"]
internal-bench = []
persistence = ["dep:rayon"]
update-check = ["dep:reqwest"]

[dev-dependencies]
serde_json = "1"
"""

#: `ci.yml`'s two workspace clippy passes as they stood before #2348.
THE_REAL_PRE_FIX_PASSES = """jobs:
  lint:
    steps:
      - name: Run Clippy (strict)
        run: |
          cargo clippy --workspace --all-targets --features persistence,gpu,update-check \\
            -- -D warnings -D clippy::pedantic
      - name: Run Clippy (strict, no gpu)
        run: |
          cargo clippy --workspace --all-targets --features persistence,update-check \\
            -- -D warnings -D clippy::pedantic
  internal-bench-check:
    steps:
      - name: Library and benches under internal-bench
        run: cargo check -p velesdb-core --benches --features persistence,internal-bench
"""

THE_FIXED_PASSES = THE_REAL_PRE_FIX_PASSES.replace(
    "--features persistence,update-check \\",
    "--features persistence,update-check,internal-bench \\",
)

#: One `cfg` site per feature the manifest declares and the crate gates on.
SOURCES = {
    "src/lib.rs": (
        '#[cfg(feature = "persistence")]\npub mod storage;\n'
        '#[cfg(feature = "gpu")]\npub mod gpu;\n'
        '#[cfg(feature = "internal-bench")]\npub mod internal_bench;\n'
        '#[cfg(feature = "update-check")]\npub mod update;\n'
    ),
}


def run_guard(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def tree(
    workflow: str | None = THE_FIXED_PASSES,
    manifest: str | None = MANIFEST,
    sources: dict[str, str] | None = None,
) -> tempfile.TemporaryDirectory:
    holder = tempfile.TemporaryDirectory()
    root = Path(holder.name)
    (root / ".github" / "workflows").mkdir(parents=True)
    (root / "crates" / "velesdb-core" / "src").mkdir(parents=True)
    if workflow is not None:
        (root / ".github" / "workflows" / "ci.yml").write_text(workflow)
    if manifest is not None:
        (root / "crates" / "velesdb-core" / "Cargo.toml").write_text(manifest)
    for name, body in (SOURCES if sources is None else sources).items():
        path = root / "crates" / "velesdb-core" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
    return holder


class FeatureLintCoverage(unittest.TestCase):
    def test_the_state_the_repository_was_in_is_refused(self):
        holder = tree(THE_REAL_PRE_FIX_PASSES)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("internal-bench", result.stdout)
        self.assertIn("1 cfg site under", result.stdout)

    def test_naming_the_feature_in_a_pedantic_pass_is_accepted(self):
        holder = tree(THE_FIXED_PASSES)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("PASSED", result.stdout)

    def test_a_cargo_check_job_does_not_count_as_coverage(self):
        """The pre-fix workflow *did* build internal-bench -- with `cargo check`.

        That job is present in both fixtures above; only the clippy line
        differs. If `cargo check` counted, the refusal case would pass.
        """
        self.assertIn("internal-bench-check", THE_REAL_PRE_FIX_PASSES)
        holder = tree(THE_REAL_PRE_FIX_PASSES)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_a_clippy_pass_without_pedantic_does_not_count(self):
        workflow = THE_FIXED_PASSES.replace("-D warnings -D clippy::pedantic", "-D warnings")
        holder = tree(workflow)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("runs no `cargo clippy`", result.stdout)

    def test_all_features_covers_every_declared_feature(self):
        workflow = (
            "jobs:\n  lint:\n    steps:\n      - name: Run Clippy\n"
            "        run: cargo clippy --workspace --all-features"
            " -- -D warnings -D clippy::pedantic\n"
        )
        holder = tree(workflow)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_feature_named_only_in_a_comment_is_not_coverage(self):
        workflow = THE_REAL_PRE_FIX_PASSES.replace(
            "      - name: Run Clippy (strict)",
            "      # cargo clippy --features internal-bench -- -D clippy::pedantic\n"
            "      - name: Run Clippy (strict)",
        )
        holder = tree(workflow)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("internal-bench", result.stdout)

    def test_a_feature_that_gates_nothing_needs_no_coverage(self):
        """`bytemuck` gates no `cfg`; asking for it to be linted means nothing."""
        manifest = MANIFEST.replace("internal-bench = []", 'bytemuck = ["bytemuck/derive"]')
        sources = {
            "src/lib.rs": SOURCES["src/lib.rs"].replace(
                '#[cfg(feature = "internal-bench")]\npub mod internal_bench;\n', ""
            )
        }
        holder = tree(THE_REAL_PRE_FIX_PASSES, manifest, sources)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_cfg_site_under_tests_counts(self):
        """`--all-targets` lints tests and benches, so their `cfg`s count too."""
        sources = {
            "src/lib.rs": '#[cfg(feature = "persistence")]\npub mod storage;\n',
            "tests/fault.rs": '#[cfg(feature = "internal-bench")]\nfn t() {}\n',
        }
        holder = tree(THE_REAL_PRE_FIX_PASSES, MANIFEST, sources)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("internal-bench", result.stdout)

    def test_a_missing_manifest_fails_rather_than_passing_vacuously(self):
        holder = tree(THE_FIXED_PASSES, manifest=None)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("Cargo.toml", result.stdout)

    def test_a_missing_workflow_fails_rather_than_passing_vacuously(self):
        holder = tree(workflow=None)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("ci.yml", result.stdout)

    def test_a_crate_that_gates_on_nothing_fails_rather_than_passing_vacuously(self):
        holder = tree(THE_FIXED_PASSES, MANIFEST, {"src/lib.rs": "pub mod storage;\n"})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("found nothing to cover", result.stdout)

    def test_a_manifest_with_no_features_fails_rather_than_passing_vacuously(self):
        manifest = MANIFEST.split("[features]")[0] + "[dev-dependencies]\n"
        holder = tree(THE_FIXED_PASSES, manifest)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("declares no feature", result.stdout)

    def test_a_continuation_line_is_read_as_part_of_its_command(self):
        """The `--features` list sits on the line after `cargo clippy` here."""
        workflow = (
            "jobs:\n  lint:\n    steps:\n      - name: Run Clippy\n"
            "        run: |\n"
            "          cargo clippy --workspace --all-targets \\\n"
            "            --features persistence,gpu,internal-bench,update-check \\\n"
            "            -- -D warnings -D clippy::pedantic\n"
        )
        holder = tree(workflow)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_the_repository_itself_passes(self):
        result = run_guard(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
