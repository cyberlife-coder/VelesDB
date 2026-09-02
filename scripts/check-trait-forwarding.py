#!/usr/bin/env python3
"""
Every delegating adapter forwards its trait WHOLE.

`velesdb-memory`'s doctrine (`src/lib.rs`, "Generics at the core, `dyn` at the
edges") ends on a rule that until now only review enforced:

    an adapter over one of these traits forwards the **whole** trait —
    partial forwarding is how a binding silently loses a capability the
    server already publishes (the #1690-#1692 gap family)

This guard makes that mechanical. It is narrow on purpose, because the bug it
prevents is narrow: rustc already refuses an `impl` that omits a *required*
method, so a partial forwarding can only ever drop a method that has a
**default body**. That is precisely what makes it silent — `Arc<Concrete>`
keeps compiling and quietly runs the trait's default instead of `Concrete`'s
override. `Extractor::extract_graph` is the shape that already cost the repo
a bug: forward only `extract`, and every `Arc`-held backend loses the
relations and attributes it actually produced.

What is checked
---------------
Two shapes of adapter, both under `crates/*/src/**/*.rs` (test files
excluded). For each, the guard reads the trait's own method names and demands
the impl define every one of them:

1. the generic wrapper — `impl<T: Trait + ?Sized> Trait for Box<T> | Arc<T> |
   Rc<T>`;
2. the erased alias — `impl Trait for DynTrait` where `DynTrait` is declared
   as `type DynTrait = Box<dyn Trait + ...>` (or `Arc`/`Rc`). This is the
   shape the bindings actually hold: `DynReranker` is what the Node binding
   wraps a JS callback in, and a default-bodied method dropped there is lost
   for every JS caller while the Rust tests, which use the concrete type,
   stay green.

A plain `impl Trait for ConcreteType` is NOT a forwarding and is never
demanded: a concrete backend is entitled to take the trait's default — that
is what defaults are for. Only an adapter that exists solely to delegate is
held to completeness.

Supertrait methods are NOT demanded: the storage facets (#1959) are separate
traits, and the doctrine's unit is the facet — "an adapter picks which facets
it serves, but each facet it implements is forwarded whole".

A trait the guard cannot see (defined in another crate) is reported as
skipped rather than passed, so a silent miss never reads as a clean run.

Exit code: 0 = every forwarding is whole, 1 = at least one is partial.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SCAN_GLOB = "crates/*/src/**/*.rs"

#: Wrapper types whose `impl Trait for Wrapper<T>` — or whose `type Dyn… =
#: Wrapper<dyn Trait>` alias — is a forwarding adapter.
#: `&T`/`&mut T` are deliberately absent: std already blanket-forwards through
#: references for the shapes this crate uses, and a hand-written one would be
#: the exception worth reading rather than a pattern to police.
WRAPPERS = ("Box", "Arc", "Rc")

_TRAIT_RE = re.compile(r"\btrait\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b")
#: Start of a generic impl. Its parameter list is closed by matching angles
#: rather than by a regex: `impl<K, T: Store<K> + ?Sized>` nests, and a
#: `[^>]*` stops at the first inner `>` and quietly matches nothing — the
#: guard would then police fewer adapters every time a bound grew a generic
#: argument, without saying so.
_IMPL_HEAD_RE = re.compile(r"\bimpl\s*<")

#: What follows that parameter list for a wrapper forwarding: the trait (with
#: its own generic arguments, if any), `for`, and the wrapper.
_IMPL_TAIL_RE = re.compile(
    r"\s*(?P<trait>[A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^{;]*?>)?\s+for\s+"
    r"(?:std::sync::|std::rc::|alloc::\w+::)?(?P<wrapper>" + "|".join(WRAPPERS) + r")\s*<"
)
#: `type DynFoo = Box<dyn Foo + Send + Sync>;` — the erased alias a binding
#: holds. Captured tree-wide so the `impl Foo for DynFoo` below can be told
#: apart from an ordinary concrete impl, which must NOT be held to
#: completeness.
_ALIAS_RE = re.compile(
    r"\btype\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*"
    r"(?:std::sync::|std::rc::|alloc::\w+::)?(?P<wrapper>" + "|".join(WRAPPERS) + r")\s*<\s*"
    r"dyn\s+(?P<trait>[A-Za-z_][A-Za-z0-9_]*)"
)

#: A non-generic `impl Trait for Name`. Only a `Name` that `_ALIAS_RE` proved
#: to be a wrapper of `dyn Trait` counts as a forwarding.
_ALIAS_IMPL_RE = re.compile(
    r"\bimpl\s+(?P<trait>[A-Za-z_][A-Za-z0-9_]*)\s+for\s+"
    r"(?P<alias>[A-Za-z_][A-Za-z0-9_]*)\s*\{"
)


def is_scanned_file(path: Path) -> bool:
    """Production Rust only — test volume has its own program (#1918)."""
    if path.name.endswith("_tests.rs") or path.name == "tests.rs":
        return False
    norm = str(path).replace("\\", "/")
    return "/tests/" not in norm and "/benches/" not in norm


def _char_literal_end(src: str, start: int) -> int | None:
    """Index just past the char literal opening at `start`, or `None` when the
    `'` is a lifetime tick rather than a quote.

    A char literal is at most a handful of bytes and always closes; a lifetime
    never does. Bounding the search is what makes the two distinguishable
    without a real lexer.
    """
    i = start + 1
    if i < len(src) and src[i] == "\\":
        i += 1
        if i < len(src) and src[i] == "u" and src[i + 1 : i + 2] == "{":
            close = src.find("}", i)
            if close == -1 or close - i > 10:
                return None
            i = close
        i += 1
    else:
        i += 1
    return i + 1 if src[i : i + 1] == "'" else None


def strip_comments(src: str) -> str:
    """Blank out comments while preserving offsets, so brace matching and the
    reported line numbers both stay true to the original text.

    String and char literals are tracked so a `//` or `/*` inside one does not
    open a comment. Raw strings (`r#"..."#`) are handled at any hash depth.

    The `'` is the delicate one: Rust spends it on lifetimes far more often
    than on char literals, and a `'static` bound leaves the tick unmatched.
    Read as an opening quote it runs to the next `'` — or to EOF — and every
    comment in between goes unblanked, so a method named only in prose enters
    the trait's method set and the guard demands a forwarding for something
    that does not exist. So a `'` opens a literal only when it actually closes
    as one (`'x'`, `'\\n'`, `'\\u{1F600}'`); otherwise it is a lifetime and only
    the tick itself is stepped over.
    """
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == "'":
            end = _char_literal_end(src, i)
            i = end if end is not None else i + 1
            continue
        if c == '"':
            i += 1
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if (src.startswith('r"', i) or src.startswith('r#"', i)) and not (
            i and (src[i - 1].isalnum() or src[i - 1] == "_")
        ):
            hashes = 0
            j = i + 1
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            terminator = '"' + "#" * hashes
            end = src.find(terminator, j + 1)
            i = n if end == -1 else end + len(terminator)
            continue
        if src.startswith("//", i):
            end = src.find("\n", i)
            end = n if end == -1 else end
            for k in range(i, end):
                out[k] = " "
            i = end
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue
        i += 1
    return "".join(out)


def block_after(src: str, start: int) -> tuple[int, int] | None:
    """Span of the `{...}` block that opens at or after `start`.

    Returns `(body_start, body_end)` exclusive of the braces, or `None` when
    the item has no block (`trait Foo;` never appears, but a `where` clause
    spanning to EOF in a truncated file would).
    """
    open_at = src.find("{", start)
    if open_at == -1:
        return None
    depth, i, n = 0, open_at, len(src)
    while i < n:
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                return (open_at + 1, i)
        i += 1
    return None


def angles_close(src: str, open_at: int) -> int | None:
    """Index just past the `>` matching the `<` at `open_at`.

    `->` is stepped over whole: a bound like `F: Fn(&str) -> u8` puts a `>`
    inside the parameter list that closes nothing.
    """
    depth, i, n = 0, open_at, len(src)
    while i < n:
        if src.startswith("->", i):
            i += 2
            continue
        if src[i] == "<":
            depth += 1
        elif src[i] == ">":
            depth -= 1
            if depth == 0:
                return i + 1
        elif src[i] in "{};":
            # An impl header never reaches a body or a statement end with its
            # parameter list still open; bailing keeps a malformed file from
            # swallowing the rest of the tree.
            return None
        i += 1
    return None


def top_level_fns(body: str) -> list[str]:
    """Names of the `fn`s declared directly in `body`, ignoring nested ones.

    A closure or a helper `fn` inside a method body sits at depth > 0 and must
    not be mistaken for a member of the trait or impl.
    """
    names: list[str] = []
    depth = 0
    for match in re.finditer(r"[{}]|\bfn\s+[A-Za-z_][A-Za-z0-9_]*", body):
        token = match.group(0)
        if token == "{":
            depth += 1
        elif token == "}":
            depth -= 1
        elif depth == 0:
            names.append(token.split()[-1])
    return names


def collect_traits(src: str) -> dict[str, list[str]]:
    """Trait name -> its own method names (supertrait methods excluded)."""
    traits: dict[str, list[str]] = {}
    for match in _TRAIT_RE.finditer(src):
        span = block_after(src, match.end())
        if span is None:
            continue
        traits[match.group("name")] = top_level_fns(src[span[0] : span[1]])
    return traits


def collect_aliases(src: str) -> dict[str, tuple[str, str]]:
    """`DynFoo -> (wrapper, trait)` for each `type DynFoo = Box<dyn Foo…>`."""
    return {
        match.group("alias"): (match.group("wrapper"), match.group("trait"))
        for match in _ALIAS_RE.finditer(src)
    }


def collect_forwardings(
    src: str, aliases: dict[str, tuple[str, str]] | None = None
) -> list[tuple[str, str, list[str], int]]:
    """`(trait, target, methods, line)` for each forwarding impl in `src`.

    `target` is what the impl is written for, as it should read in a message:
    `Arc<T>` for a generic wrapper, `DynReranker` for an erased alias.

    `aliases` is the tree-wide alias table; an alias impl is recognised only
    when the alias is known to wrap `dyn` of the very trait being implemented.
    Passing `None` collects the generic wrappers alone, which is what a
    single-file caller wants.
    """
    aliases = aliases or {}
    found = []

    def record(trait: str, target: str, body: tuple[int, int], at: int) -> None:
        found.append(
            (trait, target, top_level_fns(src[body[0] : body[1]]),
             src.count("\n", 0, at) + 1)
        )

    for head in _IMPL_HEAD_RE.finditer(src):
        after = angles_close(src, head.end() - 1)
        if after is None:
            continue
        tail = _IMPL_TAIL_RE.match(src, after)
        if tail is None:
            continue
        span = block_after(src, tail.end())
        if span is not None:
            record(tail.group("trait"), f"{tail.group('wrapper')}<T>", span, head.start())

    for match in _ALIAS_IMPL_RE.finditer(src):
        trait, alias = match.group("trait"), match.group("alias")
        # A concrete `impl Trait for Backend` is not a forwarding: a backend
        # is entitled to the trait's defaults. Only an alias that wraps
        # `dyn Trait` exists solely to delegate.
        if aliases.get(alias, (None, None))[1] != trait:
            continue
        span = block_after(src, match.end() - 1)
        if span is not None:
            record(trait, alias, span, match.start())

    return found


def audit(root: Path) -> tuple[list[str], list[str], int]:
    """`(failures, skipped, checked_count)` over the whole tree."""
    traits: dict[str, list[str]] = {}
    forwardings: list[tuple[Path, str, str, list[str], int]] = []

    # Two passes: an alias declared in one file is implemented in another
    # (`DynExtractor` lives beside its trait, its impl need not), so the alias
    # table has to be complete before any impl is judged.
    sources: list[tuple[Path, str]] = []
    aliases: dict[str, tuple[str, str]] = {}
    for path in sorted(root.glob(SCAN_GLOB)):
        if not is_scanned_file(path):
            continue
        try:
            src = strip_comments(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError):
            continue
        sources.append((path, src))
        traits.update(collect_traits(src))
        aliases.update(collect_aliases(src))

    for path, src in sources:
        for trait, target, methods, line in collect_forwardings(src, aliases):
            forwardings.append((path.relative_to(root), trait, target, methods, line))

    failures: list[str] = []
    skipped: list[str] = []
    for rel, trait, target, methods, line in forwardings:
        if trait not in traits:
            skipped.append(
                f"{rel}:{line}: `impl {trait} for {target}` — trait not found in "
                f"crates/*/src; defined outside the scanned tree, so its method "
                f"set could not be read."
            )
            continue
        missing = [m for m in traits[trait] if m not in methods]
        if missing:
            # `Arc<T>` reads as `Arc<Concrete>` at a call site; an alias reads
            # as itself. Naming the shape the caller holds is what makes the
            # message actionable.
            holder = target.replace("<T>", "<Concrete>") if target.endswith("<T>") else target
            failures.append(
                f"{rel}:{line}: `impl {trait} for {target}` forwards "
                f"{len(methods)}/{len(traits[trait])} methods — missing "
                f"{', '.join('`' + m + '`' for m in missing)}.\n"
                f"    rustc cannot catch this: a method missing here has a default "
                f"body, so {holder} silently runs the trait default "
                f"instead of the concrete override.\n"
                f"    Forward it verbatim — `fn {missing[0]}(...) {{ (**self)."
                f"{missing[0]}(...) }}` — or delete the default from the trait so "
                f"rustc demands it."
            )
    return failures, skipped, len(forwardings)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--root", default=".", help="repository root to scan")
    args = parser.parse_args(argv)

    failures, skipped, checked = audit(Path(args.root))

    for note in skipped:
        print(f"SKIPPED: {note}")

    if failures:
        print(f"\nFAILED: {len(failures)} partial trait forwarding(s):\n")
        for failure in failures:
            print(f"  - {failure}\n")
        print(
            "velesdb-memory's doctrine (src/lib.rs) requires an adapter to forward "
            "the WHOLE trait; partial forwarding is the #1690-#1692 gap family."
        )
        return 1

    print(
        f"PASSED: {checked} adapter forwarding(s) checked, every trait method "
        f"forwarded."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
