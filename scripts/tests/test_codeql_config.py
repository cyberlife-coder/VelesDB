"""The CodeQL exclusion list stays small, real and justified.

`.github/codeql/codeql-config.yml` keeps one generated-code false positive
(`rust/access-invalid-pointer` on a `#[napi]` class) out of the analysis by
excluding the whole file. A whole-file exclusion hides every other query too, so
it is allowed only for a Rust file that forbids `unsafe` outright: with
`#![deny(unsafe_code)]` the rule has nothing of the file's own to find. This pins
that the workflow reads the config, that every excluded path exists, and that an
excluded Rust file still carries the attribute.
"""

from __future__ import annotations

import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
CONFIG = ROOT / ".github" / "codeql" / "codeql-config.yml"
WORKFLOW = ROOT / ".github" / "workflows" / "codeql.yml"
CONFIG_REF = "./.github/codeql/codeql-config.yml"
DENY_UNSAFE = "#![deny(unsafe_code)]"


def _excluded() -> list[str]:
    return list(yaml.safe_load(CONFIG.read_text(encoding="utf-8")).get("paths-ignore", []))


def _init_inputs() -> dict:
    workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    steps = workflow["jobs"]["analyze"]["steps"]
    init = next(s for s in steps if str(s.get("uses", "")).startswith("github/codeql-action/init@"))
    return init.get("with", {})


class CodeqlConfigTests(unittest.TestCase):
    def test_the_workflow_reads_the_config(self) -> None:
        self.assertEqual(_init_inputs().get("config-file"), CONFIG_REF)

    def test_every_excluded_path_exists(self) -> None:
        for path in _excluded():
            with self.subTest(path=path):
                self.assertTrue((ROOT / path).is_file(), f"{path} is excluded but is not a file")

    def test_an_excluded_rust_file_still_forbids_unsafe(self) -> None:
        rust = [p for p in _excluded() if p.endswith(".rs")]
        self.assertTrue(rust, "the config no longer excludes a Rust file: drop this test with it")
        for path in rust:
            with self.subTest(path=path):
                text = (ROOT / path).read_text(encoding="utf-8")
                self.assertIn(DENY_UNSAFE, text, f"{path} is hidden from CodeQL and no longer forbids unsafe")

    def test_only_rust_files_are_excluded(self) -> None:
        for path in _excluded():
            with self.subTest(path=path):
                self.assertTrue(path.endswith(".rs"), f"{path}: the exclusion is justified for Rust glue only")


if __name__ == "__main__":
    unittest.main()
