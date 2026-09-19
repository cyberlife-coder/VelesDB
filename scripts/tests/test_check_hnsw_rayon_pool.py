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

    def test_a_writer_added_elsewhere_in_the_hnsw_module_is_refused(self):
        """The field is `pub(crate)`: its own file is not the only risk.

        An earlier version scanned `native_index.rs` alone, so a writer added
        from any other file of the module would have voided
        `brute_force_search_parallel`'s allowance in silence.
        """
        holder = tree({str(HNSW / "elsewhere.rs"): NATIVE_WRITER})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_a_known_hnsw_index_writer_is_not_reported(self):
        """`HnswIndex` has a writer by design -- that is what graph_pool is for."""
        holder = tree({str(HNSW / "index" / "vacuum.rs"): NATIVE_WRITER})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_mutable_parallel_iterator_is_not_a_blind_spot(self):
        """`par_iter` was matched, `par_iter_mut` was not.

        A review probe appended `v.par_iter_mut().for_each(...)` to the HNSW
        module and the guard answered PASSED -- it let through the exact shape
        it promises to refuse. Every submission form the pattern now covers is
        asserted here, because a pattern narrower than its promise is worse
        than no pattern.
        """
        for call in (
            "v.par_iter_mut().for_each(|x| *x += 1);",
            "v.par_chunks_mut(8).for_each(|c| c[0] = 1.0);",
            "v.par_drain(..).count();",
            "v.into_par_iter().count();",
            "rayon::scope_fifo(|s| s.spawn_fifo(|_| ()));",
        ):
            with self.subTest(call=call):
                body = (
                    "impl HnswIndex {\n    fn sneaky(&self) {\n"
                    "        let inner = self.inner.read();\n"
                    f"        {call}\n    }}\n}}\n"
                )
                holder = tree({str(HNSW / "probe.rs"): body})
                with holder:
                    result = run_guard(Path(holder.name))
                self.assertEqual(result.returncode, 1, f"{call} slipped past: {result.stdout}")

    def test_a_closed_install_does_not_vouch_for_what_follows(self):
        """Indentation is not containment.

        The first version walked up for any less-indented `.install(` and
        accepted the submission, so a closure that had already closed still
        vouched for code below it. A review probe of this exact shape got
        PASSED — the guard asserting a property it did not check.
        """
        closed = (
            "impl HnswIndex {\n    fn f(&self) {\n"
            "        graph_pool()?.install(|| self.items.par_iter().count());\n"
            "        if big {\n"
            "            self.items.par_iter().map(|x| inner.score(x)).count()\n"
            "        }\n    }\n}\n"
        )
        holder = tree({str(HNSW / "newcomer.rs"): closed})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_an_open_install_still_vouches_for_what_it_contains(self):
        """The fix must not refuse the legitimate shape it exists to allow."""
        for body in (
            # multi-line closure
            "impl HnswIndex {\n    fn f(&self) {\n"
            "        graph_pool()?.install(|| {\n"
            "            self.items.par_iter().count()\n"
            "        })\n    }\n}\n",
            # the whole call on one line
            "impl HnswIndex {\n    fn f(&self) {\n"
            "        graph_pool()?.install(|| self.items.par_iter().count())\n"
            "    }\n}\n",
        ):
            with self.subTest(body=body.splitlines()[2].strip()):
                holder = tree({str(HNSW / "newcomer.rs"): body})
                with holder:
                    result = run_guard(Path(holder.name))
                self.assertEqual(result.returncode, 0, result.stdout)

    def test_an_install_in_a_previous_function_does_not_carry_over(self):
        body = (
            "impl HnswIndex {\n"
            "    fn a(&self) {\n"
            "        graph_pool()?.install(|| { self.items.par_iter().count() })\n"
            "    }\n"
            "    fn b(&self) {\n"
            "        self.items.par_iter().count()\n"
            "    }\n}\n"
        )
        holder = tree({str(HNSW / "newcomer.rs"): body})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_an_exemption_does_not_travel_by_function_name(self):
        """`ALLOWED` is keyed by `path::fn`, and that is not cosmetic.

        `search_batch_parallel` and `brute_force_search_parallel` are each
        defined twice under `index/hnsw/`: on `HnswIndex` (`index/batch.rs`,
        whose lock HAS a writer -- `vacuum`) and on `NativeHnswIndex`
        (`native_index.rs`, whose lock has none). Keyed by bare name, one
        reason written about the second type exempted the first, so a new
        `par_iter` under a held guard in `index/batch.rs` -- #2343's exact
        shape -- would have passed with nobody asked to justify it.
        """
        body = (
            "impl HnswIndex {\n"
            "    pub fn brute_force_search_parallel(&self) {\n"
            "        let inner = self.inner.read();\n"
            "        self.v.par_iter().map(|x| inner.score(x)).count()\n"
            "    }\n}\n"
        )
        holder = tree({str(HNSW / "index" / "newfile.rs"): body})
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        # The message must name the key the author has to add, not the bare
        # name, or it sends them to write an entry that would not match.
        self.assertIn("index/newfile.rs::brute_force_search_parallel", result.stdout)

    def test_the_real_repository_passes(self):
        result = run_guard(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
