"""Tests for the SAFETY-template verifier.

The guard must refuse a genuine undocumented `unsafe` site and must not be
fooled by prose: a comment that merely *mentions* `unsafe impl` is not an
unsafe site. Both directions are asserted here — a verifier that only ever
passes is worth nothing, so every relaxation carries its positive control.
"""

from __future__ import annotations

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
GUARD = REPO_ROOT / "scripts" / "verify_unsafe_safety_template.py"


def _load_guard():
    """Import the guard module (its filename is not a valid identifier)."""
    spec = importlib.util.spec_from_file_location("verify_unsafe_safety_template", GUARD)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def _run_on(source: str) -> int:
    """Run the guard over `source` written to a temp .rs file; return exit code."""
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp) / "sample.rs"
        path.write_text(source, encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(GUARD), "--files", str(path), "--strict"],
            capture_output=True,
            text=True,
            check=False,
        )
        return result.returncode


DOCUMENTED = """\
// SAFETY: the caller guarantees the pointer is valid and aligned.
// - pointer is non-null and points to an initialised byte
// Reason: FFI boundary requires a raw read.
fn danger(p: *const u8) -> u8 {
    unsafe { *p }
}
"""

UNDOCUMENTED = """\
fn danger(p: *const u8) -> u8 {
    unsafe { *p }
}
"""

PROSE_ONLY = """\
// This type is auto-`Send`; it needs no `unsafe impl Send` of its own, and
// an unconditional `unsafe impl` here would mask a future non-Send field.
pub struct AutoSend;
"""

PROSE_THEN_REAL = """\
// A prose mention of `unsafe impl` must not satisfy the template for the
// real item that follows it.
unsafe impl Send for Foo {}
"""

BLOCK_COMMENT_PROSE = """\
/* Historical note: this used to carry an `unsafe impl Sync`. */
pub struct Plain;
"""

STRING_WITH_SLASHES = """\
// SAFETY: see https://example.com/docs for the aliasing argument.
// - buffer outlives the borrow
// Reason: zero-copy view over a mapped file.
fn view(p: *const u8) -> u8 {
    let _url = "https://example.com/a//b";
    unsafe { *p }
}
"""


class GuardRefuses(unittest.TestCase):
    """The positive control: the guard must fail on real violations."""

    def test_undocumented_unsafe_block_is_refused(self):
        self.assertEqual(_run_on(UNDOCUMENTED), 1, "guard passed an undocumented unsafe block")

    def test_prose_mention_does_not_document_a_following_unsafe_item(self):
        self.assertEqual(
            _run_on(PROSE_THEN_REAL), 1, "a comment mentioning unsafe satisfied the template"
        )


class GuardAccepts(unittest.TestCase):
    """Documented sites and prose-only mentions must pass."""

    def test_documented_unsafe_block_passes(self):
        self.assertEqual(_run_on(DOCUMENTED), 0, "guard refused a fully documented unsafe block")

    def test_comment_mentioning_unsafe_impl_is_not_a_site(self):
        # Regression: the matcher ran over raw file content, so the prose in
        # `native_inner.rs` explaining why the type deliberately has NO
        # `unsafe impl` was reported as four undocumented unsafe sites.
        self.assertEqual(_run_on(PROSE_ONLY), 0, "prose mention flagged as an unsafe site")

    def test_block_comment_mentioning_unsafe_is_not_a_site(self):
        self.assertEqual(_run_on(BLOCK_COMMENT_PROSE), 0, "block-comment prose flagged as a site")

    def test_double_slash_inside_a_string_does_not_truncate_code(self):
        self.assertEqual(_run_on(STRING_WITH_SLASHES), 0, "`//` inside a string literal hid code")


class CommentScrubbingKeepsOffsets(unittest.TestCase):
    """Line arithmetic must survive scrubbing, or reported lines drift."""

    def test_line_count_and_length_preserved(self):
        guard = _load_guard()
        source = 'let a = 1; // unsafe { }\n/* unsafe impl */\nunsafe { }\n'
        scrubbed = guard.strip_comments(source)
        self.assertEqual(scrubbed.count("\n"), source.count("\n"))
        for original, cleaned in zip(source.split("\n"), scrubbed.split("\n")):
            self.assertEqual(len(original), len(cleaned))

    def test_site_is_reported_on_its_real_line(self):
        guard = _load_guard()
        source = '// mentions unsafe impl in prose\n// second prose line\nunsafe { }\n'
        sites = guard.find_unsafe_sites(source)
        self.assertEqual([line for line, _ in sites], [3])


if __name__ == "__main__":
    unittest.main()
