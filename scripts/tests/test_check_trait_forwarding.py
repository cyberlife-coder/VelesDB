"""Tests for scripts/check-trait-forwarding.py.

Pins the guard's shape:

* the positive control — the checked-in tree — stays exit 0, and actually
  reaches the five adapters it is supposed to police, `DynReranker` named
  among them (a guard that silently scanned nothing would also "pass");
* the historical bug is refused: `Extractor for Arc<T>` forwarding only
  `extract` and dropping the defaulted `extract_graph` (#1690-#1692), with
  the missing method named in the message;
* forwarding the whole trait is accepted, so the refusal is about
  completeness and not about the shape of the impl;
* the erased-alias shape (`type DynFoo = Box<dyn Foo>` + `impl Foo for
  DynFoo`) is policed too — it carries no generic parameter, so the wrapper
  pattern alone never sees it, and it is what the bindings actually hold;
* an ordinary `impl Foo for Backend` is NOT held to completeness: a concrete
  backend is entitled to the trait's default, which is what a default is for;
* a supertrait's methods are NOT demanded — the storage facets (#1959) are
  separate traits and the doctrine's unit is the facet;
* a `fn` named only inside a doc comment, a line comment or a block comment
  is not mistaken for a trait method, which would make the guard demand a
  method that does not exist;
* a `fn` nested inside a method body belongs to neither set;
* a forwarding whose trait lives outside the scanned tree is reported as
  SKIPPED rather than passed, so a blind spot never reads as a clean run;
* an impl's generic parameter list is closed by matching angles, so a nested
  bound (`Store<K>`) or an arrow (`Fn(&str) -> u8`) does not silently drop the
  adapter from the scan;
* a lifetime tick is not read as a char literal. Rust spends `'` on lifetimes
  far more often than on char literals, and a `'static` bound leaves an
  unmatched tick: a lexer that opens a literal there runs past every comment
  after it, so a `fn` merely *named* in a comment becomes a trait method the
  guard then demands. That false finding is how a guard gets switched off.
"""

from __future__ import annotations

import importlib.util
import io
import sys
import tempfile
import types
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-trait-forwarding.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_trait_forwarding", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_trait_forwarding"] = module
    spec.loader.exec_module(module)
    return module


ctf = _load_script()


def run_on(source: str, *, filename: str = "adapter.rs") -> tuple[int, str]:
    """Materialise `source` as one production file and run the guard on it."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src_dir = root / "crates" / "velesdb-memory" / "src"
        src_dir.mkdir(parents=True)
        (src_dir / filename).write_text(source, encoding="utf-8")
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            code = ctf.main(["--root", str(root)])
        return code, buffer.getvalue()


PARTIAL = """
pub trait Extractor {
    fn extract(&self, text: &str) -> u8;
    fn extract_graph(&self, text: &str) -> u8 {
        self.extract(text)
    }
}

impl<T: Extractor + ?Sized> Extractor for std::sync::Arc<T> {
    fn extract(&self, text: &str) -> u8 {
        (**self).extract(text)
    }
}
"""

WHOLE = """
pub trait Extractor {
    fn extract(&self, text: &str) -> u8;
    fn extract_graph(&self, text: &str) -> u8 {
        self.extract(text)
    }
}

impl<T: Extractor + ?Sized> Extractor for std::sync::Arc<T> {
    fn extract(&self, text: &str) -> u8 {
        (**self).extract(text)
    }
    fn extract_graph(&self, text: &str) -> u8 {
        (**self).extract_graph(text)
    }
}
"""


class TestRealTree(unittest.TestCase):
    def test_checked_in_tree_passes_and_actually_scans(self) -> None:
        failures, _skipped, checked = ctf.audit(REPO_ROOT)
        self.assertEqual(failures, [], "\n".join(failures))
        self.assertGreaterEqual(
            checked,
            5,
            "the guard must reach the crate's adapter forwardings — the four "
            "generic wrappers and the DynReranker alias; scanning zero of them "
            "would pass vacuously",
        )

    def test_the_erased_alias_shape_is_among_them(self) -> None:
        """`impl Reranker for DynReranker` is a non-generic impl on a type
        alias, so the wrapper regex alone never sees it — and it is the shape
        the Node binding holds. Losing it would make the guard blind exactly
        where the #1690-#1692 bugs landed."""
        rerank = REPO_ROOT / "crates" / "velesdb-memory" / "src" / "rerank.rs"
        source = ctf.strip_comments(rerank.read_text(encoding="utf-8"))
        aliases = ctf.collect_aliases(source)
        self.assertEqual(aliases.get("DynReranker"), ("Box", "Reranker"))
        targets = [t for _, t, _, _ in ctf.collect_forwardings(source, aliases)]
        self.assertIn("DynReranker", targets)


class TestRefusal(unittest.TestCase):
    def test_partial_forwarding_is_refused(self) -> None:
        code, out = run_on(PARTIAL)
        self.assertEqual(code, 1, out)
        self.assertIn("extract_graph", out)
        self.assertIn("1/2 methods", out)

    def test_whole_forwarding_is_accepted(self) -> None:
        code, out = run_on(WHOLE)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)


class TestErasedAliasForwardings(unittest.TestCase):
    """`type DynFoo = Box<dyn Foo>` plus `impl Foo for DynFoo` is a forwarding
    written without a generic parameter. It is what a binding holds, so it has
    to be held to the same completeness — while an ordinary
    `impl Foo for Backend`, which merely *implements* the trait, must not be:
    taking a default is what a default is for."""

    ALIAS_PARTIAL = """
pub trait Speaker {
    fn say(&self) -> u8;

    fn shout(&self) -> u8 {
        self.say()
    }
}

pub type DynSpeaker = Box<dyn Speaker + Send + Sync>;

impl Speaker for DynSpeaker {
    fn say(&self) -> u8 {
        (**self).say()
    }
}
"""

    ALIAS_WHOLE = ALIAS_PARTIAL.replace(
        """    fn say(&self) -> u8 {
        (**self).say()
    }
}
""",
        """    fn say(&self) -> u8 {
        (**self).say()
    }

    fn shout(&self) -> u8 {
        (**self).shout()
    }
}
""",
    )

    CONCRETE_BACKEND = """
pub trait Speaker {
    fn say(&self) -> u8;

    fn shout(&self) -> u8 {
        self.say()
    }
}

pub struct Quiet;

impl Speaker for Quiet {
    fn say(&self) -> u8 {
        1
    }
}
"""

    def test_a_partial_alias_forwarding_is_refused(self) -> None:
        code, out = run_on(self.ALIAS_PARTIAL)
        self.assertEqual(code, 1, out)
        self.assertIn("DynSpeaker", out)
        self.assertIn("shout", out)

    def test_a_whole_alias_forwarding_is_accepted(self) -> None:
        code, out = run_on(self.ALIAS_WHOLE)
        self.assertEqual(code, 0, out)

    def test_a_concrete_backend_may_keep_the_default(self) -> None:
        code, out = run_on(self.CONCRETE_BACKEND)
        self.assertEqual(code, 0, out)
        self.assertIn("0 adapter forwarding(s)", out)

    def test_an_alias_declared_in_another_file_still_resolves(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src_dir = root / "crates" / "velesdb-memory" / "src"
            src_dir.mkdir(parents=True)
            (src_dir / "speaker.rs").write_text(
                """
pub trait Speaker {
    fn say(&self) -> u8;

    fn shout(&self) -> u8 {
        self.say()
    }
}

pub type DynSpeaker = Box<dyn Speaker + Send + Sync>;
""",
                encoding="utf-8",
            )
            (src_dir / "binding.rs").write_text(
                """
use super::speaker::{DynSpeaker, Speaker};

impl Speaker for DynSpeaker {
    fn say(&self) -> u8 {
        (**self).say()
    }
}
""",
                encoding="utf-8",
            )
            failures, _skipped, checked = ctf.audit(root)
            self.assertEqual(checked, 1)
            self.assertEqual(len(failures), 1, failures)
            self.assertIn("shout", failures[0])


class TestScopeOfTheDemand(unittest.TestCase):
    def test_supertrait_methods_are_not_demanded(self) -> None:
        # `RecallStore: FactStore` — a forwarding for RecallStore forwards its
        # OWN methods; FactStore travels through its own impl (#1959 facets).
        code, out = run_on(
            """
pub trait FactStore {
    fn put(&self) -> u8;
}
pub trait RecallStore: FactStore {
    fn recall(&self) -> u8;
}
impl<T: RecallStore + ?Sized> RecallStore for std::sync::Arc<T> {
    fn recall(&self) -> u8 {
        (**self).recall()
    }
}
"""
        )
        self.assertEqual(code, 0, out)

    def test_fn_named_in_comments_is_not_a_method(self) -> None:
        code, out = run_on(
            """
pub trait Small {
    /// See `fn ghost_doc` for the rationale.
    // fn ghost_line(&self);
    /* fn ghost_block(&self); */
    fn only(&self) -> u8;
}
impl<T: Small + ?Sized> Small for Box<T> {
    fn only(&self) -> u8 {
        (**self).only()
    }
}
"""
        )
        self.assertEqual(code, 0, out)

    def test_nested_fn_belongs_to_neither_set(self) -> None:
        code, out = run_on(
            """
pub trait Small {
    fn only(&self) -> u8 {
        fn helper_in_trait() -> u8 { 1 }
        helper_in_trait()
    }
}
impl<T: Small + ?Sized> Small for Box<T> {
    fn only(&self) -> u8 {
        fn helper_in_impl() -> u8 { 2 }
        let _ = helper_in_impl();
        (**self).only()
    }
}
"""
        )
        self.assertEqual(code, 0, out)


class TestGenericHeadersAreParsedByBalance(unittest.TestCase):
    """An impl's parameter list is closed by matching angles, not by a regex.
    `impl<K, T: Store<K> + ?Sized>` nests, and `impl<F: Fn(&str) -> u8, ...>`
    puts a `>` in it that closes nothing. A pattern that stops at the first
    `>` matches neither — so the guard would police fewer adapters every time
    a bound grew a generic argument, and say nothing about it. Silent loss of
    reach is the failure mode a guard must not have."""

    HEADERS = {
        "plain": "impl<T: Ext + ?Sized> Ext for std::sync::Arc<T>",
        "nested generic argument": "impl<K, T: Store<K> + ?Sized> Store<K> for Arc<T>",
        "Fn bound with an arrow": "impl<F: Fn(&str) -> u8, T: Hook<F> + ?Sized> Hook<F> for Box<T>",
        "where clause": "impl<T> Speak for Box<T> where T: Speak + ?Sized",
    }

    def test_every_header_shape_is_recognised(self) -> None:
        for name, header in self.HEADERS.items():
            with self.subTest(header=name):
                found = ctf.collect_forwardings(header + " { fn a(&self) {} }")
                self.assertEqual(len(found), 1, f"{name}: {found}")
                self.assertEqual(found[0][2], ["a"])

    def test_a_non_wrapper_target_is_not_a_forwarding(self) -> None:
        self.assertEqual(
            ctf.collect_forwardings("impl<T> Speak for MyType<T> { fn say(&self) {} }"),
            [],
        )

    def test_an_unclosed_parameter_list_does_not_swallow_the_file(self) -> None:
        self.assertIsNone(ctf.angles_close("impl<T: Ext { fn a() {} }", 4))


class TestLexingIsRustAware(unittest.TestCase):
    """Rust spends `'` on lifetimes far more often than on char literals, and
    a `'static` bound leaves the tick unmatched. Treating it as an opening
    quote makes the scan run past every comment that follows, so a method
    named only in prose is counted as declared — the guard then demands a
    forwarding for a method that does not exist. A guard that invents work is
    a guard someone deletes."""

    STATIC_BOUND_THEN_PROSE = """pub trait Ext: Send + Sync + 'static {
    fn extract(&self);
    // This trait used to declare fn extract_relations; it was removed in #1949.
    fn extract_graph(&self) {}
}

impl<T: Ext + ?Sized> Ext for std::sync::Arc<T> {
    fn extract(&self) {
        (**self).extract()
    }

    fn extract_graph(&self) {
        (**self).extract_graph()
    }
}
"""

    def test_a_static_bound_does_not_leak_prose_into_the_method_set(self) -> None:
        methods = ctf.collect_traits(ctf.strip_comments(self.STATIC_BOUND_THEN_PROSE))
        self.assertEqual(methods["Ext"], ["extract", "extract_graph"])

    def test_that_whole_forwarding_is_accepted(self) -> None:
        code, output = run_on(self.STATIC_BOUND_THEN_PROSE)
        self.assertEqual(code, 0, output)

    def test_char_literals_are_still_recognised(self) -> None:
        for literal in ("'x'", "'\\n'", "'\\\''", "'\\u{1F600}'"):
            source = f'fn real() {{ let c = {literal}; let s = "fn ghost() {{}}"; }}'
            self.assertEqual(
                ctf.top_level_fns(ctf.strip_comments(source)), ["real"], literal
            )

    def test_offsets_survive_stripping(self) -> None:
        for source in (
            "impl<'a, T> Foo for &'a T {}",
            "pub trait T: Send + 'static { fn a(&self); }",
            'fn a() {{ let s = "it\'s // not a comment"; }}',
            'fn a() {{ let s = r#"fn ghost() // "#; }}',
        ):
            self.assertEqual(len(ctf.strip_comments(source)), len(source), source)


class TestBlindSpotIsVisible(unittest.TestCase):
    def test_foreign_trait_is_skipped_not_passed(self) -> None:
        code, out = run_on(
            """
impl<T: ForeignTrait + ?Sized> ForeignTrait for std::sync::Arc<T> {
    fn whatever(&self) -> u8 {
        (**self).whatever()
    }
}
"""
        )
        self.assertEqual(code, 0, out)
        self.assertIn("SKIPPED", out)
        self.assertIn("ForeignTrait", out)


if __name__ == "__main__":
    unittest.main()
