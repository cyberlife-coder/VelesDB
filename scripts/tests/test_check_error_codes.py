"""Tests for scripts/check-error-codes.py.

The guard exists because `docs/reference/ERROR_CODES.md` is a second source
of truth for `VELES-XXX` and `E0XX` codes that nothing compared — a code
added to `error.rs` could ship undocumented, or stay documented after being
removed, with no red anywhere. The tests that matter are the refusals in
BOTH directions, plus the pass on a tree that agrees.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-error-codes.py"

_spec = importlib.util.spec_from_file_location("check_error_codes", SCRIPT_PATH)
assert _spec is not None and _spec.loader is not None
guard = importlib.util.module_from_spec(_spec)
sys.modules["check_error_codes"] = guard
_spec.loader.exec_module(guard)

ENGINE = Path("crates/velesdb-core/src/error.rs")
PARSER = Path("crates/velesdb-core/src/velesql/error.rs")
DOC = Path("docs/reference/ERROR_CODES.md")

ERROR_RS = '#[error("[VELES-001] a")]\n#[error("[VELES-002] b")]\n'
PARSER_RS = 'const A: &str = "E001";\nconst B: &str = "E002";\n'
DOC_MD = "codes: VELES-001 VELES-002 and E001 E002\n"


def build_tree(root: Path, error_rs: str, parser_rs: str, doc: str) -> None:
    """Writes the minimal tree the guard reads."""
    for rel, text in ((ENGINE, error_rs), (PARSER, parser_rs), (DOC, doc)):
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


class CheckErrorCodesTests(unittest.TestCase):
    def test_passes_when_both_schemes_agree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, ERROR_RS, PARSER_RS, DOC_MD)
            self.assertEqual(guard.check(root), 0)

    def test_refuses_a_code_declared_but_undocumented(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(
                root,
                ERROR_RS + '#[error("[VELES-099] new")]\n',
                PARSER_RS,
                DOC_MD,
            )
            self.assertEqual(guard.check(root), 1)

    def test_refuses_a_code_documented_but_undeclared(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, ERROR_RS, PARSER_RS, DOC_MD + "VELES-050\n")
            self.assertEqual(guard.check(root), 1)

    def test_refuses_parser_drift_either_direction(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(
                root, ERROR_RS, PARSER_RS + 'const C: &str = "E099";\n', DOC_MD
            )
            self.assertEqual(guard.check(root), 1)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, ERROR_RS, PARSER_RS, DOC_MD + "E042\n")
            self.assertEqual(guard.check(root), 1)

    def test_real_tree_is_in_sync(self) -> None:
        # The guard must be green on the repository it ships in — otherwise
        # the CI step it is wired into is born red.
        self.assertEqual(guard.check(Path(__file__).resolve().parent.parent.parent), 0)


if __name__ == "__main__":
    unittest.main()
