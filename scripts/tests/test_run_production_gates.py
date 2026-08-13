"""CLI contracts for scripts/run-production-gates.sh."""

from __future__ import annotations

import subprocess
import shutil
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "run-production-gates.sh"


class ProductionGateCliTests(unittest.TestCase):
    def _fixture(self, documented: bool) -> Path:
        root = Path(tempfile.mkdtemp(prefix="production-gates-test-"))
        self.addCleanup(shutil.rmtree, root, True)
        source = root / "crates" / "velesdb-server" / "src"
        source.mkdir(parents=True)
        (source / "routes.rs").write_text(
            '.route("/query/explain", get(explain))\n.route("/aggregate", post(aggregate))\n',
            encoding="utf-8",
        )
        readme = "`/query/explain`\n"
        if documented:
            readme += "`/aggregate`\n"
        (root / "README.md").write_text(readme, encoding="utf-8")
        return root

    def _run(self, root: Path, guard: str = "doc-contract") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(SCRIPT), "--root", str(root), "--guard", guard],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_selected_member_refuses_and_accepts(self) -> None:
        self.assertEqual(self._run(self._fixture(False)).returncode, 1)
        self.assertEqual(self._run(self._fixture(True)).returncode, 0)

    def test_unknown_selector_is_a_usage_error(self) -> None:
        self.assertEqual(self._run(self._fixture(True), "unknown").returncode, 2)


if __name__ == "__main__":
    unittest.main()
