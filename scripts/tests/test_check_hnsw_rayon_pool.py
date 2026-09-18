"""The HNSW rayon-pool guard refuses what #2343 made possible.

A guard that only ever passes proves nothing: every test here builds a tree the
guard must REFUSE, and each is paired with the repaired tree it must accept, so
a guard degraded into a no-op fails these rather than going quiet.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "check_hnsw_rayon_pool.py"
HNSW = Path("crates/velesdb-core/src/index/hnsw")


def run_guard(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def tree(files: dict[str, str]) -> tempfile.TemporaryDirectory:
    holder = tempfile.TemporaryDirectory()
    root = Path(holder.name)
    for relative, body in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
    return holder


GLOBAL_SUBMISSION = """impl HnswIndex {
    fn a_new_holder(&self) -> usize {
        let inner = self.inner.read();
        self.items.par_iter().map(|x| inner.score(x)).count()
    }
}
"""

DEDICATED_SUBMISSION = """impl HnswIndex {
    fn a_new_holder(&self) -> usize {
        let inner = self.inner.read();
        graph_pool()?.install(|| {
            self.items.par_iter().map(|x| inner.score(x)).count()
        })
    }
}
"""

NATIVE_WRITER = """impl NativeHnswIndex {
    pub fn compact(&self) {
        let mut inner = self.inner.write();
        inner.compact();
    }
}
"""

NATIVE_NO_WRITER = """impl NativeHnswIndex {
    pub fn peek(&self) {
        let inner = self.inner.read();
        inner.len();
    }
}
"""


class HnswRayonPoolGuard(unittest.TestCase):
    def test_a_new_global_submission_is_refused(self):
        holder = tree({str(HNSW / "newcomer.rs"): GLOBAL_SUBMISSION})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("a_new_holder", result.stdout)

    def test_the_same_submission_on_the_dedicated_pool_is_accepted(self):
        """The pool decides, not the function's name."""
        holder = tree({str(HNSW / "newcomer.rs"): DEDICATED_SUBMISSION})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_writer_on_native_index_inner_is_refused(self):
        """It voids the allowance `brute_force_search_parallel` rests on."""
        holder = tree({str(HNSW / "native_index.rs"): NATIVE_WRITER})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("NativeHnswIndex::inner", result.stdout)

    def test_native_index_without_a_writer_is_accepted(self):
        holder = tree({str(HNSW / "native_index.rs"): NATIVE_NO_WRITER})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_submission_quoted_in_a_comment_does_not_trip_the_guard(self):
        holder = tree(
            {str(HNSW / "newcomer.rs"): "// self.items.par_iter() would deadlock\n"}
        )
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_the_real_repository_passes(self):
        result = run_guard(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
