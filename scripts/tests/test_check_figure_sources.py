"""Tests for scripts/check-figure-sources.py, and above all its refusal.

Once the repository is clean it can no longer show the guard refusing anything,
so each test hands the guard a tree of its own (`--root`). A guard is worth
what it refuses.
"""

from __future__ import annotations

import contextlib
import importlib.metadata
import importlib.util
import io
import json
import re
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-figure-sources.py"
_SPEC = importlib.util.spec_from_file_location("check_figure_sources", SCRIPT_PATH)
guard = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(guard)


class FigureSourcesTest(unittest.TestCase):
    def tree(self, files: dict[str, str], claims: tuple[tuple[str, str], ...] = ()) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        contract = root / guard.CONTRACT
        contract.parent.mkdir(parents=True, exist_ok=True)
        contract.write_text(
            json.dumps(
                {
                    "claims": [
                        {"file": c[0], "must_contain": c[1], **({"covers_section": True} if len(c) > 2 and c[2] else {})}
                        for c in claims
                    ]
                }
            )
        )
        return root

    def flagged(self, root: Path) -> list[str]:
        return guard.violations(root)

    def test_an_unregistered_speed_ratio_in_a_guide_is_refused(self):
        root = self.tree({"docs/guides/G.md": "The fast builder is ~2-3x faster than `new`.\n"})
        self.assertEqual(len(self.flagged(root)), 1)
        self.assertIn("docs/guides/G.md:1: speed ratio", self.flagged(root)[0])
        self.assertEqual(guard.main(["--root", str(root)]), 1)

    def test_a_registered_figure_passes(self):
        root = self.tree(
            {"docs/guides/G.md": "The fast builder is ~2-3x faster than `new`.\n"},
            claims=(("docs/guides/G.md", "~2-3x faster"),),
        )
        self.assertEqual(self.flagged(root), [])
        self.assertEqual(guard.main(["--root", str(root)]), 0)

    def test_a_claim_covers_its_own_file_and_line_only(self):
        files = {
            "docs/guides/A.md": "Search answers in 450 µs at p50.\n",
            "docs/guides/B.md": "Search answers in 450 µs at p50.\nInserts land in 3 ms.\n",
        }
        root = self.tree(files, claims=(("docs/guides/A.md", "450 µs at p50"), ("docs/guides/B.md", "450 µs")))
        self.assertEqual(
            [v.split(": ")[0] for v in self.flagged(root)],
            ["docs/guides/B.md:2"],
        )

    def test_rustdoc_is_in_scope_but_not_plain_comments_or_code(self):
        source = (
            "/// Reaches ~90% recall at the default ef.\n"
            "// 2x faster than the scalar loop\n"
            'const NOTE: &str = "95% recall";\n'
            "//! The p99 latency is 2 ms on a laptop.\n"
        )
        root = self.tree({"crates/c/src/lib.rs": source})
        self.assertEqual(
            [v.split(": ")[0] for v in self.flagged(root)],
            ["crates/c/src/lib.rs:1", "crates/c/src/lib.rs:4"],
        )

    def test_history_archives_and_node_modules_are_out_of_scope(self):
        figure = "Search is 3x faster.\n"
        root = self.tree(
            {
                "CHANGELOG.md": figure,
                "docs/CHANGELOG.md": figure,
                "docs/archive/old.md": figure,
                "sdks/ts/node_modules/dep/README.md": figure,
            }
        )
        self.assertEqual(self.flagged(root), [])

    def test_readmes_under_crates_and_sdks_are_in_scope(self):
        root = self.tree({"crates/c/README.md": "12k QPS on one core.\n", "sdks/py/README.md": "95%+ recall.\n"})
        self.assertEqual(len(self.flagged(root)), 2)

    def test_a_configured_bound_is_not_a_latency_figure(self):
        root = self.tree(
            {"docs/guides/G.md": "Queries time out after 500 ms by default.\nThe p99 latency is 2 ms.\n"}
        )
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/guides/G.md:2"])

    def test_each_recall_and_throughput_form_is_caught(self):
        lines = ["95%+ recall.", "recall@10 of 0.95 on SIFT.", "Recall stays at 96% or more.", "12k QPS.", "3,000 inserts/s."]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(len(self.flagged(root)), len(lines))

    def test_a_section_claim_covers_its_section_and_no_other(self):
        doc = (
            "## 2. PQ\n"
            "### PQ Recall (pq_recall_benchmark)\n"
            "| Full precision | recall@10 of 0.99 |\n"
            "| PQ m=8 | 91% recall |\n"
            "### PQ Latency\n"
            "| PQ m=8 | p50 at 120 µs |\n"
        )
        root = self.tree(
            {"docs/BENCH.md": doc},
            claims=(("docs/BENCH.md", "### PQ Recall (pq_recall_benchmark)", True),),
        )
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/BENCH.md:6"])

    def test_a_line_claim_does_not_cover_its_section(self):
        doc = "### PQ Recall\n| Full precision | 99% recall |\n| PQ | 91% recall |\n"
        root = self.tree({"docs/BENCH.md": doc}, claims=(("docs/BENCH.md", "99% recall"),))
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/BENCH.md:3"])

    def test_a_user_story_tag_is_not_a_latency(self):
        root = self.tree({"crates/c/src/lib.rs": "/// Executes a MATCH query (EPIC-045 US-002).\n"})
        self.assertEqual(self.flagged(root), [])

    def test_emphasis_and_footnote_marks_do_not_hide_a_ratio(self):
        doc = "It is **130x** faster.\nBatch is 3x faster" + chr(0xB2) + " here.\nAnd 5% slower.\n"
        root = self.tree({"docs/G.md": doc})
        self.assertEqual(len(self.flagged(root)), 3)

    def test_each_throughput_unit_is_caught(self):
        lines = [
            "16,151 vec/s after the fix.",
            "1.3M queries/sec.",
            "21.5 Gelem/s at 768D.",
            "Import at 2,943 MB/s.",
            "25-30 Kvec/s on one thread.",
            "2,000 points/s.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(len(self.flagged(root)), len(lines))

    def test_a_table_cell_latency_is_caught_but_not_a_configured_one(self):
        root = self.tree({"docs/G.md": "| Search top-10 | 57.6 µs |\n| query timeout | 500 ms |\n"})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:1"])

    def test_default_before_the_figure_does_not_exempt_it(self):
        root = self.tree({"docs/G.md": "The default search answers in 2 ms.\n"})
        self.assertEqual(len(self.flagged(root)), 1)

    def test_a_verb_introduces_a_latency(self):
        root = self.tree({"docs/G.md": "A context compiles in 3 ms.\n"})
        self.assertEqual(len(self.flagged(root)), 1)

    def test_a_claim_covers_only_the_figure_it_overlaps(self):
        line = "Search answers in 450 µs at p50, and inserts in 3 ms.\n"
        root = self.tree({"docs/G.md": line}, claims=(("docs/G.md", "450 µs"),))
        self.assertEqual(len(self.flagged(root)), 1)
        root = self.tree({"docs/G.md": line}, claims=(("docs/G.md", "450 µs"), ("docs/G.md", "3 ms")))
        self.assertEqual(self.flagged(root), [])

    def test_binding_docs_are_in_scope_but_not_their_code(self):
        py = 'def f():\n    """Runs at 12k QPS."""\n    rate = "12k QPS"\n'
        ts = "/**\n * Answers 2x faster.\n */\nconst note = '2x faster';\n"
        root = self.tree(
            {"crates/velesdb-python/python/velesdb/__init__.py": py, "sdks/typescript/src/core.ts": ts}
        )
        self.assertEqual(
            sorted(v.split(": ")[0] for v in self.flagged(root)),
            ["crates/velesdb-python/python/velesdb/__init__.py:2", "sdks/typescript/src/core.ts:2"],
        )

    def test_a_plus_after_a_throughput_is_caught(self):
        root = self.tree({"docs/G.md": "It ingests 10,000+ points/s.\n"})
        self.assertEqual(len(self.flagged(root)), 1)

    def test_a_bare_speed_ratio_is_caught_but_not_a_size_ratio(self):
        root = self.tree({"docs/G.md": "Batching is ~50-105x here.\nSQ8 is 4x smaller in memory.\n"})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:1"])

    def test_a_ratio_that_is_no_measurement_is_left_alone(self):
        lines = [
            "Build with 0.5x ef on the middle layers.",
            "It falls back to a full scan above 50x the limit.",
            "Skip an outlier above 10x the threshold.",
            "RaBitQ saves 32x bandwidth.",
            "It stays near 22 ms at 1024 × 1 KB.",
            "The loop is aligned to `8 × lane`.",
            "Rerank retrieves 4x candidates, then keeps k.",
            "Then once more at 2× minEf, capped at maxEf.",
            "Binary quantization trades some recall for 32x less memory.",
            "Eight accumulators hold 8x norm_b.",
            "A timeout of 3x the default suits a remote server.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_measured_ratio_is_caught(self):
        lines = ["8 threads reach ~8x the throughput of one.", "Crossing 100k docs cost **43×** per query.", "| NEON | ~1.8x |"]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(len(self.flagged(root)), len(lines))

    def test_a_change_stated_as_a_percentage_is_caught(self):
        root = self.tree({"docs/G.md": "Build time rises 31 % at 10K.\nThe hook adds a 5% overhead.\n"})
        self.assertEqual(len(self.flagged(root)), 2)

    def test_a_time_in_a_table_row_is_caught_but_not_a_configured_one(self):
        doc = "| Kernel | Latency |\n|---|---|\n| Cosine 768D | 35.8 ns |\n| query timeout | 30 s |\n"
        root = self.tree({"docs/G.md": doc})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:3"])

    def test_a_hash_line_in_a_code_fence_does_not_end_a_section(self):
        doc = "### Results\n```bash\n# run it\ncargo bench\n```\n| Search | 57.6 µs |\n"
        root = self.tree({"docs/B.md": doc}, claims=(("docs/B.md", "### Results", True),))
        self.assertEqual(self.flagged(root), [])

    def test_a_section_claim_must_name_one_heading(self):
        doc = "### Results\n| Search | 57.6 µs |\n### Results\n| Insert | 3 ms |\n"
        root = self.tree({"docs/B.md": doc}, claims=(("docs/B.md", "### Results", True),))
        self.assertTrue(any("matches 2 headings" in v for v in self.flagged(root)))

    def test_memory_ratios_are_arithmetic_not_measurements(self):
        root = self.tree({"docs/G.md": "SQ8 stores each vector in 4x less memory.\nBinary quantization is 32x smaller.\n"})
        self.assertEqual(self.flagged(root), [])


    def test_a_configured_bound_named_by_its_parameter_is_not_a_latency(self):
        # `timeout` is one segment of `query_timeout_ms`: a table of ceilings
        # names the parameter, not the word.
        root = self.tree({"docs/G.md": "| `search.query_timeout_ms` | disabled | 24 h (86,400,000 ms) |\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_measured_time_beside_a_parameter_name_is_still_caught(self):
        # No latency keyword on these rows: only the table rule sees them, so
        # only a parameter segment (not `limit` inside `limited`) may exempt one.
        doc = "| Name | Cost |\n|---|---|\n| `export_elapsed_ms` | 2.1 ms |\n| Rate-limited export | 3 ms |\n"
        root = self.tree({"docs/G.md": doc})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:3", "docs/G.md:4"])

    def test_a_ratio_scaling_an_ef_parameter_is_left_alone(self):
        lines = [
            "- **Bulk** (middle 80%): 0.5x `ef_construction` -- leverages the existing",
            "Hard queries retry at 2\u00d7 ef_search.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_word_that_merely_ends_in_ef_does_not_exempt_a_ratio(self):
        root = self.tree({"docs/G.md": "In brief, batching is ~5x here.\n"})
        self.assertEqual(len(self.flagged(root)), 1)

    def test_the_plural_of_a_bound_exempts_a_ratio_like_its_singular(self):
        lines = [
            "//! - **stale penalty** \u2014 all factors inflated by 1.2\u00d7 when any histogram is stale",
            "//! Skip the outliers above 10x the thresholds.",
        ]
        root = self.tree({"crates/c/src/lib.rs": "\n".join(lines) + "\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_plural_is_a_word_not_a_suffix(self):
        root = self.tree({"docs/G.md": "The refactors brought search to 1.2\u00d7 the old throughput.\n"})
        self.assertEqual(len(self.flagged(root)), 1)

    def test_a_count_of_calls_or_simd_registers_is_not_a_ratio(self):
        rust = "/// This replaces the prior 3-pass approach (`dot_product_neon` called 3x).\n/// Invoked 2x per row.\n"
        root = self.tree({"crates/c/src/lib.rs": rust, "docs/G.md": "### 1. 32-Wide Unrolling (4x f32x8)\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_count_exempts_its_own_multiplier_and_no_other(self):
        cases = {
            "`dot_product_neon`, called 3x, made cosine ~2.5x the fused kernel's time.": ["~2.5x"],
            "The 32-wide loop (4x f32x8) runs at ~1.8x the scalar loop.": ["~1.8x"],
        }
        for line, expected in cases.items():
            self.assertEqual(self.figure_texts(line), expected, line)


    def test_a_time_below_the_millisecond_needs_no_keyword(self):
        # Nothing in these docs is configured in nanoseconds or microseconds,
        # so such a number is a measurement wherever it stands.
        lines = [
            "Foo 40 ns.",
            "- Cosine: ~32ns",
            "The rotation costs ~60 us for 768D.",
            "Warm-up is ~5\u03bcs at 4 GHz.",
            "A push holds the lock for 10 nanoseconds.",
            "Each probe is 3 microseconds.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(len(self.flagged(root)), len(lines))
        self.assertTrue(all(": time with no registered measurement" in v for v in self.flagged(root)))

    def test_a_millisecond_or_second_still_needs_a_keyword(self):
        # Configured values are stated in these units (a back-off, an idle
        # period): only a keyword, a verb or a table makes them a figure.
        root = self.tree({"docs/G.md": "Retries back off for 100 ms.\nThe pool idles 30 s before closing.\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_time_unit_in_a_name_a_code_span_a_spec_range_or_a_simd_type_is_no_figure(self):
        lines = [
            "Set `timeout_us` to bound the wait.",
            "The counter elapsed_ns and the group poll_10us are names.",
            "Pass `--poll 10us` to the bench.",
            "| `ef_search` | 16\u20134096 (or `auto` from mode) |",
            "The kernel keeps its sums in u8x16 and f32x8 registers.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(self.flagged(root), [])

    def test_a_size_word_exempts_only_the_ratio_it_qualifies(self):
        # "4x less memory" is arithmetic. A ratio in the next clause is not
        # about memory, whatever the line says elsewhere.
        cases = {
            "SQ8 stores each vector in 4x less memory.": [],
            "It cuts peak memory usage by ~2x.": [],
            "SQ8 needs 4x less memory and ~3x the throughput.": ["~3x"],
            "Binary is 32x smaller; batching gives ~8x the throughput.": ["~8x"],
        }
        for line, expected in cases.items():
            self.assertEqual(self.figure_texts(line), expected, line)

    def test_a_size_word_further_away_or_in_another_cell_exempts_nothing(self):
        # The size word sits within three words of its ratio, in its clause,
        # or labels the ratio's table column or row. Anywhere else it is no
        # licence: not four words away, not across a comma, not in the next
        # column.
        doc = (
            "The quantized index answers at ~3x the old rate.\n"
            "| Mode | Compression | Search |\n"
            "|---|---|---|\n"
            "| `sq8` | 4x | same |\n"
            "| `pq` | 16x | ~3x |\n"
            "| **Memory** | 3072 bytes | **4x** |\n"
            "The delta is 234 MiB, 4.8x.\n"
        )
        root = self.tree({"docs/G.md": doc})
        self.assertEqual(
            [v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:1", "docs/G.md:5", "docs/G.md:7"]
        )

    def test_a_bare_quantity_in_a_code_span_is_a_figure(self):
        # Backticks mark code, not a licence: a quantity that stands alone in
        # its span is a figure like any other.
        lines = [
            "At 100K rows (`JSON scan 3.84 ms \u2192 ColumnStore 29.5 us`).",
            "The blocker quoted `16.3 us/fact`.",
            "Batching gives `~3x`.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(
            [v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:1", "docs/G.md:2", "docs/G.md:3"]
        )

    def test_a_number_that_is_code_in_a_code_span_is_no_figure(self):
        # Part of a name, a path or an option, or an operand of an expression.
        lines = [
            "The group `poll_10us` is a name.",
            "Results land in `bench/10us/`.",
            "The call `sleep(10us)` yields.",
            "Pass `--poll 10us` to the bench.",
            "Set `wait = 10us`.",
            "The loop runs `4 \u00d7 dim` steps.",
            "The filter is a `4 x u64` array.",
        ]
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n"})
        self.assertEqual(self.flagged(root), [])

    def test_quantization_is_not_a_size_word(self):
        # "Quantized search ~3x" states a speed. A ratio is a size only when a
        # size word names it (memory, bytes, smaller, compression...): the
        # technique behind it says nothing of what was measured.
        doc = (
            "Quantized search ~3x.\n"
            "| Feature | VelesDB |\n"
            "|---|---|\n"
            "| **Quantization** | SQ8 (4x) |\n"
            "SQ8 quantization stores vectors in 4x less memory.\n"
        )
        root = self.tree({"docs/G.md": doc})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:1", "docs/G.md:4"])

    def test_a_bound_or_config_word_exempts_only_the_ratio_it_qualifies(self):
        # Like a size word, a threshold, an ef or a default is about the ratio
        # beside it: a ratio elsewhere on the line is still a figure.
        cases = {
            "Skip an outlier above 10x the threshold.": [],
            "A timeout of 3x the default suits a remote server.": [],
            "Skip an outlier above 10x the threshold; batching gives ~3x the rate.": ["~3x"],
            "Build at 0.5x ef, and the batch lands ~2x sooner.": ["~2x"],
            "The default is 30 s, which is 15x too long here.": ["15x"],
        }
        for line, expected in cases.items():
            self.assertEqual(self.figure_texts(line), expected, line)


    def test_a_config_word_elsewhere_in_a_table_row_exempts_no_time(self):
        # A time's config word must be about that time: in its cell, as its
        # column header or as its row label. In another cell of the row it
        # says nothing about it.
        doc = "| Operation | Time | Note |\n|---|---|---|\n| Bulk import | 3.2 ms | default mode |\n"
        root = self.tree({"docs/G.md": doc})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:3"])

    def test_every_time_of_a_table_row_is_judged_on_its_own(self):
        # The configured 250 ms is exempt by the word in its cell; the 3.2 ms
        # measured beside it is not. The row's first time no longer stands for
        # the row, and a config word still exempts the value it qualifies.
        header = "| Operation | Time | Note |"
        cases = {
            "| Export | 250 ms timeout | 3.2 ms |": ["3.2 ms"],
            "| query timeout | 500 ms |": [],
            "| Retry back-off | 100 ms by default |": [],
        }
        for line, expected in cases.items():
            self.assertEqual(self.figure_texts(line, header), expected, line)


    def test_readmes_under_examples_are_in_scope(self):
        # A demo's README is read like any other README: a figure there is a
        # promise. Its dependencies are not.
        figure = "Search answers in 3 ms.\n"
        root = self.tree({"examples/demo/README.md": figure, "examples/demo/node_modules/dep/README.md": figure})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["examples/demo/README.md:1"])


    def figure_spans(self, line: str, header: str | None = None) -> list[tuple[int, int]]:
        """Where, in `line`, the guard reads a figure: the line parsed alone,
        or as the one row of a table under `header`."""
        doc = [line]
        if header:
            columns = len(guard.cells(header))
            doc = [header, "|" + "---|" * columns, line]
        rendered = [e for e in guard.read_document(doc, "text") if isinstance(e, guard.Line) and e.number == len(doc)]
        return [guard.raw_span(rendered[0], s, e) for _, s, e in guard.figures(rendered[0])] if rendered else []

    def figure_texts(self, line: str, header: str | None = None) -> list[str]:
        return [line[s:e] for s, e in self.figure_spans(line, header)]

    def numbers_flagged(self, line: str, header: str | None = None) -> list[str]:
        spans = self.figure_spans(line, header)
        return sorted({m.group() for m in guard.NUMBER.finditer(line) if any(s <= m.start() < e for s, e in spans)})

    def test_a_table_cell_is_read_with_its_column_header(self):
        # A results table names its keyword once, in its header, and a unit in
        # parentheses there: every cell is read with it, whatever the kind.
        header = "| Profile | ef_search | Recall@10 | Throughput (QPS) | Latency (ms) | Compression |"
        line = "| Fast | 96 | 97.4% | 12,000 | 3.2 | 4x |"
        self.assertEqual(self.numbers_flagged(line, header), ["12,000", "3.2", "97.4"])
        root = self.tree({"docs/G.md": header + "\n|---|---|---|---|---|---|\n" + line + "\n"})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:3"])

    def test_a_keyword_on_the_line_above_reaches_the_figure(self):
        # A comment or a paragraph wraps: the keyword may end one line and the
        # figure begin the next.
        rust = "/// Accurate: 100% recall@10 in\n/// `recall_benchmark`, 0.98 on SIFT1M's 1M.\n"
        doc = "Search reaches a recall@10 of\n0.95 on SIFT1M.\n"
        root = self.tree(
            {"crates/c/src/lib.rs": rust, "docs/G.md": doc},
            claims=(("crates/c/src/lib.rs", "100% recall@10"),),
        )
        self.assertEqual(sorted(v.split(": ")[0] for v in self.flagged(root)), ["crates/c/src/lib.rs:2", "docs/G.md:2"])

    def test_a_time_in_a_doc_comment_table_is_read(self):
        rust = "/// | facts | elapsed |\n/// |---|---|\n/// | 250 | 10.3 ms |\n"
        root = self.tree({"crates/c/src/lib.rs": rust})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["crates/c/src/lib.rs:3"])

    def test_each_form_the_review_found_is_read(self):
        lines = [
            "It takes 42 s to rebuild.",
            "The server handles 12K req/s.",
            "It sustains 1.2K+ QPS.",
            "It is 3 times faster.",
            "It is 2.8-fold faster.",
            "It gives a \u00d73 speed-up.",
            "It has 40% lower latency.",
            "It has 35% higher throughput.",
            "It reaches a recall of 0.976.",
            "A probe costs 450 usec.",
            "- Bulk import: 50K+ vectors/sec at 768D",
            "- Maintain 50M+ items/sec filter throughput (vs 19M/s with JSON)",
            "with per-batch latency 66% higher than single-threaded.",
            "A search takes 3 seconds.",
            "The p50 is 2 msec.",
        ]
        table = "\n| Mode | Cost |\n|---|---|\n| Fast | 3 seconds |\n"
        root = self.tree({"docs/G.md": "\n".join(lines) + "\n" + table})
        self.assertEqual(len(self.flagged(root)), len(lines) + 1)

    def test_a_config_word_exempts_the_time_or_ratio_beside_it(self):
        # A poll interval, a retry count or a setting is configured; a measured
        # time or ratio beside it is still a figure.
        cases = {
            "The poll interval is 100 \u00b5s.": [],
            "Retry up to 3x before giving up.": [],
            "The poll interval is 100 \u00b5s; a search takes 57.6 \u00b5s.": ["57.6"],
            "Retries back off 2x; batching gives ~3x the throughput.": ["3"],
        }
        for line, expected in cases.items():
            self.assertEqual(self.numbers_flagged(line), expected, line)
        header = "| Setting | Value |"
        self.assertEqual(self.numbers_flagged("| `flush_interval_us` | 100 \u00b5s |", header), [])
        self.assertEqual(self.numbers_flagged("| Search | 57.6 \u00b5s |", header), ["57.6"])


    def test_a_unit_glued_to_a_name_is_no_time(self):
        # "f32s" is the plural of a type, not 32 seconds.
        for line in ("/// * `query` \u2014 query vector (dim f32s)", "Each query holds four f32s."):
            self.assertEqual(self.numbers_flagged(line), [], line)

    def test_a_config_word_exempts_the_rate_beside_it(self):
        # A rate limit is configured; a measured rate beside it is a figure.
        cases = {
            "The per-IP limiter (100 req/s by default) is saturated.": [],
            "Production: a 200 req/s limit per IP.": [],
            "The limiter allows 100 req/s by default; the server serves 12K req/s.": ["12"],
        }
        for line, expected in cases.items():
            self.assertEqual(self.numbers_flagged(line), expected, line)


    def test_an_architecture_name_is_no_speed_ratio(self):
        # "a \u00d73 speed-up" is a figure; "x86 FASTER" names an architecture.
        self.assertEqual(self.numbers_flagged("| \U0001F534 x86 FASTER | Investigate NEON codegen |"), [])
        self.assertEqual(self.numbers_flagged("It gives a \u00d73 speed-up."), ["3"])


    def test_a_time_unit_in_the_header_reads_a_bare_cell(self):
        # "| Build time (s) |" over "| 42 |" is 42 seconds, as "| 42 s |" is;
        # a config word in that header still makes it a setting.
        self.assertEqual(self.numbers_flagged("| Fast | 42 |", "| Mode | Build time (s) |"), ["42"])
        self.assertEqual(self.numbers_flagged("| Fast | 42 |", "| Mode | Timeout (s) |"), [])

    def test_an_escaped_pipe_stays_inside_its_cell(self):
        # GFM keeps `\\|` inside its cell: the cells after it keep their headers.
        self.assertEqual(self.numbers_flagged("| a \\| b | 97.4% |", "| Mode | Recall@10 |"), ["97.4"])
        self.assertEqual(self.numbers_flagged("| a \\| b | 30 s |", "| Setting | Timeout |"), [])


    def test_a_header_unit_reads_an_emphasized_cell(self):
        # "| **42** |" under "(s)" is 42 seconds, as "| 42 |" is.
        self.assertEqual(self.numbers_flagged("| Fast | **42** |", "| Mode | Build time (s) |"), ["42"])

    def test_a_header_unit_in_brackets_reads_a_bare_cell(self):
        # "[s]" and "[ms]" carry a unit as "(s)" does.
        self.assertEqual(self.numbers_flagged("| Fast | 42 |", "| Mode | Build time [s] |"), ["42"])
        self.assertEqual(self.numbers_flagged("| Fast | 42 |", "| Mode | Latency [ms] |"), ["42"])


    def unreadable(self, doc: str, claim: str | None = None) -> list[str]:
        """The unreadable-figure findings of a one-file tree; a claim over the
        line must not turn one into a pass."""
        root = self.tree({"docs/G.md": doc}, claims=(("docs/G.md", claim),) if claim else ())
        return [v.split(": ")[0] for v in self.flagged(root) if ": unreadable figure: " in v]

    def test_a_mark_between_a_number_and_its_unit_in_a_cell_is_unreadable(self):
        # A footnote or a literal mark, not markup a renderer takes out.
        doc = "| Mode | Latency (ms) |\n|---|---|\n| D | 3.2\u00b9 ms |\n| F | 1.2\\* \u00b5s |\n"
        self.assertEqual(self.unreadable(doc), ["docs/G.md:3", "docs/G.md:4"])

    def test_a_footnote_on_a_cell_under_a_header_unit_is_unreadable(self):
        doc = "| Mode | Build time (s) |\n|---|---|\n| Fast | 42\u00b9 |\n"
        self.assertEqual(self.unreadable(doc), ["docs/G.md:3"])

    def test_any_time_unit_standing_as_a_word_in_a_header_reads_its_column(self):
        # performance.rs states "| µs/fact |" over "| 41.1 |": a unit need not
        # stand alone in parentheses, and markup around the header hides none.
        for header in ("| n | \u00b5s/fact |", "| n | us/fact |", "| n | **Build time (s)** |", "| n | Build time (s)\u00b9 |"):
            self.assertEqual(self.numbers_flagged("| 250 | 41.1 |", header), ["41.1"], header)
            doc = f"{header}\n|---|---|\n| 250 | 41.1 |\n| 500 | ~48 |\n"
            self.assertEqual(self.unreadable(doc), ["docs/G.md:4"], header)
        # A word that merely contains a unit, at its end or at its start, names none.
        for header in ("| n | items |", "| n | msgs |", "| n | sections |"):
            self.assertEqual(self.numbers_flagged("| 250 | 41.1 |", header), [], header)

    def test_each_readable_cell_under_a_header_unit_is_read_not_refused(self):
        # "42", "42 s" and either inside one emphasis pair, "**" or "__".
        header = "| Mode | Build time (s) |"
        for cell in ("42", "42 s", "**42**", "__42__", "**42 s**", "__42 s__"):
            row = f"| Fast | {cell} |"
            self.assertEqual(self.unreadable(f"{header}\n|---|---|\n{row}\n"), [], cell)
            self.assertEqual(self.numbers_flagged(row, header), ["42"], cell)

    def test_a_rate_or_a_possessive_in_a_header_is_no_time_unit(self):
        # "Queries/s" is a rate: its cells are not times, so "12K" is no
        # unreadable time; nor is the "s" of "Rust's".
        for header in ("| Mode | Queries/s |", "| Mode | Rust's build |"):
            self.assertEqual(self.unreadable(f"{header}\n|---|---|\n| Fast | 12K |\n"), [], header)

    def test_a_number_with_digit_groups_is_one_number(self):
        # "2,592,000" is one number, read as a cell like "3,600".
        doc = "| Unit | Build time (s) |\n|---|---|\n| months | 2,592,000 |\n"
        self.assertEqual(self.unreadable(doc), [])
        self.assertEqual(self.numbers_flagged("| months | 2,592,000 |", "| Unit | Build time (s) |"), ["2,592,000"])

    def test_markup_a_renderer_takes_out_is_read_as_the_figure_it_renders(self):
        # Emphasis, links, HTML comments and tags, entities: the parser renders
        # them as a reader sees them, and the guard reads the rendered figure,
        # refused until registered and passed once it is.
        shapes = [
            "__42__ ms", "_42_ ms", "<b>42</b> ms", "42<br>ms", "42&nbsp;ms", "~~42~~ ms", "**42** ms",
            "[42](#run) ms", "42<!---->ms", "42 <!-- note --> ms", "[42][run] ms",
        ]
        for shape in shapes:
            doc = f"The p50 is {shape}.\n\n[run]: https://example.com\n"
            flagged = self.flagged(self.tree({"docs/G.md": doc}))
            self.assertEqual([v.split(": ", 2)[:2] for v in flagged], [["docs/G.md:1", "latency with no registered measurement"]], shape)
            self.assertEqual(self.flagged(self.tree({"docs/G.md": doc}, claims=(("docs/G.md", shape),))), [], shape)
        self.assertEqual(self.numbers_flagged("Cosine 42<!---->\u00b5s."), ["42"])
        self.assertEqual(self.numbers_flagged("| A | 42<!---->ms |", "| Case | p50 |"), ["42"])

    def test_a_mark_the_renderer_leaves_between_a_number_and_its_unit_is_unreadable(self):
        # What still keeps a number from its unit once rendered: code, a
        # superscript, an invisible character, a leading point, a literal mark.
        shapes = ["`42` ms", "42\u00b9 ms", "42 ms\u00b9", "42\u200bms", ".5 ms"]
        # Every character CommonMark or GFM uses as syntax, left literal.
        shapes += [f"42{mark} ms" for mark in "*_`~\\[]()<>|^#!"]
        for shape in shapes:
            line = f"The p50 is {shape}.\n"
            self.assertEqual(self.unreadable(line), ["docs/G.md:1"], shape)
            self.assertEqual(self.unreadable(line, claim=f"The p50 is {shape}"), ["docs/G.md:1"], shape)

    def test_a_time_inside_one_underscore_pair_is_read(self):
        # `__42 ms__` renders as `**42 ms**` does, and is read the same way.
        self.assertEqual(self.numbers_flagged("The median is __42 ms__."), ["42"])
        self.assertEqual(self.numbers_flagged("| Fast | __42 ms__ |", "| Mode | Latency |"), ["42"])
        self.assertEqual(self.unreadable("The median is __42 ms__.\n"), [])

    def test_a_time_glued_to_a_name_is_no_figure(self):
        # A number or a unit glued to a name on either side is part of it.
        # (No config word here: "poll_10us" would be exempt as a poll.)
        for row in ("| a | step_10us |", "| a | v1.5 ms |", "| a | 42 ms_total |", "| a | 3 s-curve |"):
            self.assertEqual(self.numbers_flagged(row, "| a | b |"), [], row)

    def test_a_name_an_operator_or_a_range_is_no_unreadable_figure(self):
        # Marks that are text, not markup, stay: a spaced `*` is no emphasis, an
        # underscore inside a name belongs to it, a dash or an apostrophe is text.
        for line in ("`poll_10us` and poll_10us", "four f32s", "4 * ms", "it took 1\u20133 s", "Rust's 3 s build",
                     "the 3 s-curve", "q_42_ms"):
            self.assertEqual(self.unreadable(line + "\n"), [], line)

    def test_code_is_left_to_the_other_rules(self):
        # A code span, a fenced block and a doc-test hold code, not prose.
        self.assertEqual(self.unreadable("Call `wait(**42** ms)` first.\n"), [])
        self.assertEqual(self.unreadable("```\nlet d = 42\u00b9 ms;\n```\nThe p50 is 42\u00b9 ms.\n"), ["docs/G.md:4"])
        source = (
            "/// ```\n"
            "/// let d = 42\u00b9 ms;\n"
            "/// ```\n"
            "/// The p50 is 42\u00b9 ms.\n"
            "fn f() {}\n"
            "/// ```\n"
            "fn g() {}\n"
            "/// The p99 is 42\u00b9 ms.\n"
        )
        root = self.tree({"crates/c/src/lib.rs": source})
        flagged = [v.split(": ")[0] for v in self.flagged(root) if ": unreadable figure: " in v]
        # A doc comment that a line of code interrupts closes its doc-test.
        self.assertEqual(flagged, ["crates/c/src/lib.rs:4", "crates/c/src/lib.rs:8"])

    def test_a_mark_between_a_number_and_its_unit_in_prose_is_unreadable_even_when_claimed(self):
        self.assertEqual(self.unreadable("The p50 is 42\u00b9 ms.\n"), ["docs/G.md:1"])
        self.assertEqual(self.unreadable("The p50 is 42\u00b9 ms.\n", claim="The p50 is 42\u00b9 ms"), ["docs/G.md:1"])

    def latency_lines(self, doc: str) -> list[str]:
        return [v.split(": ")[0] for v in self.flagged(self.tree({"docs/G.md": doc})) if ": latency " in v]

    def test_a_fence_ends_where_the_parser_ends_it(self):
        # A `~~~` inside a backtick fence does not close it; a line that opens
        # with backticks and closes them is inline code; a backtick fence whose
        # info string holds a backtick is no fence. None hides what follows.
        after = "\np50 42&nbsp;ms.\n"
        self.assertEqual(self.latency_lines("```\n~~~\np50 1 ms\n```\n" + after), ["docs/G.md:6"])
        self.assertEqual(self.latency_lines("```x``` is inline\n" + after), ["docs/G.md:3"])
        self.assertEqual(self.latency_lines("````x````\n" + after), ["docs/G.md:3"])

    def test_code_blocks_hold_no_figure(self):
        # A fenced or an indented code block is code, not a claim (#2313).
        self.assertEqual(self.latency_lines("```\nlet total = 1000 * ms; // p50 42 ms\n```\n"), [])
        self.assertEqual(self.latency_lines("Text.\n\n    p50 42 ms\n"), [])

    def test_a_table_without_outer_pipes_is_read(self):
        doc = "Case | Latency (ms)\n--- | ---\nA | ~42\nB | 42\n"
        root = self.tree({"docs/G.md": doc})
        self.assertEqual(
            [v.split(": ", 2)[:2] for v in self.flagged(root)],
            [
                ["docs/G.md:3", "unreadable figure"],
                ["docs/G.md:3", "latency with no registered measurement"],
                ["docs/G.md:4", "latency with no registered measurement"],
            ],
        )

    def test_a_heading_does_not_wrap_into_the_line_under_it(self):
        # Only two lines of one paragraph read as one sentence.
        self.assertEqual(self.flagged(self.tree({"docs/G.md": "## Recall@10 in\n0.98 on SIFT1M.\n"})), [])
        self.assertEqual(len(self.flagged(self.tree({"docs/G.md": "Recall@10 in\n0.98 on SIFT1M.\n"}))), 1)

    def test_a_docstring_is_read_once_its_indentation_is_removed(self):
        # A method's docstring is indented past a code block's four spaces: as
        # `inspect.cleandoc` does, the common indentation goes first.
        source = 'class C:\n    def f(self):\n        """Search.\n\n        The p50 is 42 ms.\n        """\n'
        root = self.tree({"crates/velesdb-python/python/velesdb/m.py": source})
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["crates/velesdb-python/python/velesdb/m.py:5"])

    def test_a_figure_keeps_its_place_in_the_file_line_after_an_entity_or_an_escaped_pipe(self):
        # A claim covers the figure its text overlaps in the file's own line.
        # An entity renders one character from six, an escaped pipe one from
        # two: the figures after them must still land where they are written,
        # or a claim beside a figure would cover it, or miss it.
        line = "p50&nbsp;is 3 ms; the p99 is 9 ms."
        root = self.tree({"docs/G.md": line + "\n"}, claims=(("docs/G.md", "3 ms"), ("docs/G.md", "9 ms")))
        self.assertEqual(self.flagged(root), [])
        doc = "| Case |\n|---|\n| x \\| 3 ms |\n"
        root = self.tree({"docs/G.md": doc}, claims=(("docs/G.md", "x \\| "),))
        self.assertEqual([v.split(": ")[0] for v in self.flagged(root)], ["docs/G.md:3"])

    def test_a_link_reference_definition_is_no_prose(self):
        self.assertEqual(self.latency_lines('See [run].\n\n[run]: https://example.com "p50 42 ms"\n'), [])

    def test_a_mermaid_diagram_is_read(self):
        # GitHub draws a mermaid block, labels and all: its figures are shown.
        self.assertEqual(
            [v.split(": ")[0] for v in self.flagged(self.tree({"docs/G.md": "```mermaid\nA -->|x| B[BFS 290ns]\n```\n"}))],
            ["docs/G.md:2"],
        )

    def test_a_raw_html_block_is_read_through_its_text(self):
        doc = "<table>\n<tr><td>p50 42<!-- x --> ms</td></tr>\n</table>\n"
        self.assertEqual(self.latency_lines(doc), ["docs/G.md:2"])

    def exit_without_the_pinned_parser(self, **patches) -> tuple[int, str]:
        root = self.tree({"docs/G.md": "Search answers in 3 ms.\n"})
        for name, value in patches.items():
            self.addCleanup(setattr, guard, name, getattr(guard, name))
            setattr(guard, name, value)
        with contextlib.redirect_stderr(io.StringIO()) as err, contextlib.redirect_stdout(io.StringIO()):
            code = guard.main(["--root", str(root)])
        return code, err.getvalue()

    def test_the_guard_says_it_could_not_run_without_its_parser(self):
        code, err = self.exit_without_the_pinned_parser(MarkdownIt=None)
        self.assertEqual(code, 2)
        self.assertIn("markdown-it-py is not importable", err)

    def test_the_guard_runs_with_no_other_parser_version(self):
        # markdown-it-py 3.0.0 renders differently and passed every test.
        for name in ("markdown-it-py", "mdurl"):
            with self.subTest(name=name):
                code, err = self.exit_without_the_pinned_parser(
                    installed_version=lambda package, stale=name: "0.0.1" if package == stale else guard.PARSER_PINS[package]
                )
                self.assertEqual(code, 2)
                self.assertIn(f"{name} 0.0.1 is installed; the guard needs {name}=={guard.PARSER_PINS[name]}", err)

    def test_the_guard_says_which_pinned_package_is_not_installed(self):
        for name in ("markdown-it-py", "mdurl"):
            with self.subTest(name=name):

                def version(package, absent=name):
                    if package == absent:
                        raise importlib.metadata.PackageNotFoundError(package)
                    return guard.PARSER_PINS[package]

                code, err = self.exit_without_the_pinned_parser(installed_version=version)
                self.assertEqual(code, 2)
                self.assertIn(f"{name} is not installed; the guard needs {name}=={guard.PARSER_PINS[name]}", err)

    def test_the_pinned_parser_is_the_one_installed_here(self):
        # The control: with the pins CI installs, the guard runs.
        self.assertIsNone(guard.parser_problem())

    def test_an_image_alt_text_is_read(self):
        self.assertEqual(self.latency_lines("See ![p50 42 ms](x.png).\n"), ["docs/G.md:1"])

    def test_a_number_with_its_unit_in_a_cell_is_read(self):
        doc = "| Mode | Latency |\n|---|---|\n| Fast | 42 ms |\n"
        self.assertEqual(self.unreadable(doc), [])
        self.assertEqual(self.numbers_flagged("| Fast | 42 ms |", "| Mode | Latency |"), ["42"])

    def test_a_number_with_its_unit_inside_one_emphasis_pair_is_read(self):
        doc = "| Mode | Latency |\n|---|---|\n| Fast | **42 ms** |\nThe median is **42 ms**.\n"
        self.assertEqual(self.unreadable(doc), [])
        self.assertEqual(self.numbers_flagged("| Fast | **42 ms** |", "| Mode | Latency |"), ["42"])
        self.assertEqual(self.numbers_flagged("The median is **42 ms**."), ["42"])


if __name__ == "__main__":
    unittest.main()
