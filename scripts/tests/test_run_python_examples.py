"""Self-test for scripts/run-python-examples.py: it fails when one example fails."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("run_python_examples", SCRIPTS / "run-python-examples.py")
runner = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = runner
_spec.loader.exec_module(runner)


def _tree(root: Path, failing: "str | None") -> None:
    for example in runner.EXAMPLES:
        path = root / example
        path.parent.mkdir(parents=True, exist_ok=True)
        body = "raise SystemExit(3)\n" if example == failing else "print('ok')\n"
        path.write_text(body, encoding="utf-8")


class RunPythonExamplesTests(unittest.TestCase):
    def test_every_example_passing_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _tree(Path(tmp), failing=None)
            self.assertEqual(runner.main(["--root", tmp]), 0)

    def test_one_failing_example_fails_the_run(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _tree(Path(tmp), failing="examples/python/hello_velesdb.py")
            self.assertEqual(runner.main(["--root", tmp]), 1)

    @unittest.skipUnless(importlib.util.find_spec("numpy"), "the stand-in encoder needs numpy")
    def test_the_stand_in_encoder_is_importable_from_its_fixture(self) -> None:
        sys.path.insert(0, str(runner.STUB_ENCODER))
        try:
            from sentence_transformers import SentenceTransformer  # noqa: PLC0415
        finally:
            sys.path.remove(str(runner.STUB_ENCODER))
        self.assertEqual(SentenceTransformer("any").encode("text").shape, (384,))


if __name__ == "__main__":
    unittest.main()
