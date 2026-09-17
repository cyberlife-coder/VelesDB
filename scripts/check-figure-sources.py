#!/usr/bin/env python3
"""Every performance figure in the docs and the rustdoc names its measurement.

A figure that says how fast or how accurate VelesDB is ("2-3x faster", "~90%
recall", "450 µs p50", "10k QPS") is a promise. The README's figures are pinned
in docs/reference/promise-contract.json, each with the source, date, machine
and version of the run behind it, and scripts/check-promise-contract.py keeps
those pins true. Nothing tied the guides, the reference docs or the rustdoc to
a measurement (#2266): the review of #2250 found figures there that no run
produced, and some that contradict the benchmark meant to back them.

A line of those documents that states such a figure must be registered in the
promise contract: a claim whose `file` is that document and whose
`must_contain` is part of that line, or a claim with `covers_section: true`
whose `must_contain` is the heading of the section the line sits in (a table
of results measured in one run is registered once, with that run's source,
date, machine and version). A figure nobody measured is removed, or stated
without a number.

Scope: README.md, docs/ (except docs/archive/), every README under crates/,
sdks/ and examples/, the doc comments (`///`, `//!`) under crates/*/src, and
the Python and TypeScript bindings' docstrings and doc comments. A CHANGELOG
records history, not promises, and benchmark result files are sources: both
are out of scope. So is bench code: its figures are the bounds it asserts and
the results it prints, the measurement itself rather than a promise about it.

It reads what a reader sees. Every document, and every doc comment or
docstring once its comment markers are taken off, is parsed as Markdown by
markdown-it-py (CommonMark with GFM tables and strikethrough), and the guard
reads the text the parser renders: emphasis, links, HTML comments and tags,
entities and escapes are resolved the way a renderer resolves them, so
"p50 **42** ms", "p50 42<!---->ms" and "p50 [42][run] ms" all read 42 ms.
Fenced and indented code blocks hold code and are not read, except a mermaid
block, which GitHub draws with its labels shown; a table is read by
the parser's cells and header cells, with or without its outer pipes; a raw
HTML block is read through the standard library's HTML parser.

A figure is a number with a unit, read with the words around it: a keyword or
a verb before it ("p50 450 µs", "answers in 2 ms", "takes 42 s"), the end of
the line above when a paragraph wraps ("recall@10 in" / "0.98 on SIFT1M"), or
the column header of its table cell ("| Recall@10 |" over "| 97.4% |",
"| Latency (ms) |" over "| 3.2 |"). Below the millisecond the unit alone is
enough ("Cosine 32 ns"), and every time in a table row counts. Not a figure: a
memory or compression ratio (f32 to u8 is 4x less memory by arithmetic, not by
measurement), a configured value (a timeout, a poll interval, a retry count, a
rate limit), a scale applied to a parameter (0.5x `ef_construction`) and a
count (a kernel called 3x, 4x f32x8 registers). The word that makes a value one
of these must stand beside it, in its clause or as its table cell's label, and
exempts that value alone. A number in a code span is exempt only as code
(`sleep(10us)`, `4 × dim`): a quantity standing alone in one (`29.5 us`) is a
figure.

It fails closed on time figures. In the rendered text it reads a time in two
forms: a number with its unit ("42 ms"), and a bare number, or a number with
its unit, alone in a cell under a header where a time unit stands as a word
("| Build time (s) |", "| µs/fact |" over "| 42 |"). Any other cell under such
a header ("~42", "42¹") is an unreadable figure, and so is any number and time
unit that only a mark keeps apart in the rendered text: the guard takes out of
it every character Markdown uses as syntax, left there as a literal mark, every
superscript and every invisible format character, and reports each number
with a time unit it then finds that it did not read at the same place ("42¹ ms",
"42* ms", "`42` ms"), whatever the register holds.

What it cannot see: this is a heuristic. It does not read
- a number written with no unit and no such word ("4,000 of them a second");
- a duration whose keyword or verb is not next to it ("14.19 s cold against
  0.22 s warm", "held every call for 46-52 s", "roughly 16 seconds"), or one
  given in minutes ("1 min 59 s", "56 minutes");
- a time in a table row whose label is a config word ("| Budget pressure |
  ... | 1.07 ms |");
- a figure drawn in an image or a chart;
- a Python doctest (`>>>`), which Markdown renders as a quotation and the guard
  therefore reads as prose.
What binds a figure to its run is the register, not this guard: a figure it
misses is a promise all the same, and belongs in the contract.
"""

from __future__ import annotations

import argparse
import html
import json
import re
import sys
import unicodedata
from html.parser import HTMLParser
from pathlib import Path
from typing import NamedTuple

try:
    from markdown_it import MarkdownIt
except ModuleNotFoundError:  # reported by main(): the guard could not run
    MarkdownIt = None

CONTRACT = Path("docs/reference/promise-contract.json")
# The parser the guard reads Markdown with. CI installs exactly this version.
PARSER_REQUIREMENT = "markdown-it-py==4.2.0"

# The one number grammar every rule reads.
_NUM = r"\d+(?:[.,]\d+)*"
NUMBER = re.compile(_NUM)
# Emphasis or code may wrap a figure: `130x` faster.
_MARK = r"(?:\*\*|__|\*|_|`)?"
# A footnote marker may follow the word: `faster²`.
_SUP = "[" + "".join(chr(c) for c in (0xB9, 0xB2, 0xB3, *range(0x2070, 0x207A))) + "]*"
# A time unit written out or abbreviated: "450 usec", "3 seconds", "2 msec".
# The abbreviations are matched in lower case: "MS" names no unit.
_LONG_TIME = r"(?:[nuµμm]?secs?|(?:nano|micro|milli)?seconds?)"
_SUB_MS_UNIT = r"(?:(?-i:ns|[µμ]s|us)|[nuµμ]secs?|nanoseconds?|microseconds?)"
_TIME_UNIT = rf"(?:(?-i:ns|[µμ]s|us|ms|s)|{_LONG_TIME})"
# Where a time starts and ends. A number glued to a name is part of it
# (`poll_10us`, `f32s`, `v1.5 ms`), and so is a unit before a hyphen ("US-012"
# is a user story, not a microsecond).
_BEFORE = r"(?<![^\W_])(?<!\.)(?<![^\W_]_)"
_AFTER = r"(?![^\W_]|-)(?!_[^\W_])"
# A time as every rule reads it: a number with its unit.
_TIME = rf"{_BEFORE}{_NUM}\s*{_TIME_UNIT}{_AFTER}"
KINDS: dict[str, re.Pattern[str]] = {
    "speed ratio": re.compile(
        rf"{_NUM}\s*(?:[-–]\s*{_NUM}\s*)?[x×]{_MARK}\s+{_MARK}(?:faster|slower|speed-?ups?|quicker){_SUP}\b"
        rf"|{_NUM}\s*%{_MARK}\s+{_MARK}(?:faster|slower|speed-?ups?|quicker){_SUP}\b"
        # "3 times faster", "2.8-fold faster", "a ×3 speed-up" (the sign, not
        # the letter: "x86 faster" names an architecture).
        rf"|{_NUM}\s*(?:times|-?\s*fold){_MARK}\s+{_MARK}(?:faster|slower|speed-?ups?|quicker){_SUP}\b"
        rf"|×\s*{_NUM}{_MARK}\s+{_MARK}(?:faster|slower|speed-?ups?|quicker){_SUP}\b",
        re.I,
    ),
    # A bare ratio (`~50–105x`) is a speed claim unless a size, bound or
    # config word qualifies it (`qualified`).
    # A `×` followed by a number is a product of dimensions (`1024 × 1 KB`).
    "ratio": re.compile(rf"~?{_NUM}\s*(?:[-–]\s*{_NUM}\s*)?[x×](?![-\w])(?!\s*\d)"),
    # A change stated as a percentage: "rises 31 %", "a 5% overhead".
    "change": re.compile(
        rf"\b(?:rises?|rose|falls?|fell|drops?|grows?|grew|improves?|regress(?:es)?|increases?|decreases?"
        rf"|gains?|cuts?|reduces?|slows?|overhead)\b[^.\n]{{0,20}}?{_NUM}\s*%"
        rf"|{_NUM}\s*%\s+(?:overhead|improvement|regression|speed-?up|reduction|slowdown|higher|lower)\b",
        re.I,
    ),
    "recall": re.compile(
        rf"recall(?:@\d+)?[^.|\n]{{0,40}}?{_NUM}\s*%|{_NUM}\s*%\+?\s*recall|recall(?:@\d+)?[^.|\n]{{0,40}}?\b0\.\d{{2,}}",
        re.I,
    ),
    "throughput": re.compile(
        # A rate: "12k QPS", "1.2K+ QPS", "50K+ vectors/sec", "12K req/s", "19M/s".
        rf"{_NUM}\+?\s*[kKMG]?\+?\s*(?:QPS|qps|queries per second|[A-Za-z]+\s*/\s*s(?:ec)?|/\s*s(?:ec)?)\b"
    ),
    "latency": re.compile(
        rf"(?:\b(?:p50|p9\d|p99\.9|median|mean|latenc(?:y|ies)|search(?:es)?|quer(?:y|ies)|inserts?|round[- ]trip)\b"
        rf"[^.\n]{{0,40}}?"
        rf"|\b(?:takes?|took|runs?|completes?|answers?|responds?|compiles?|finishes?|returns?)\s+(?:in\s+)?"
        rf"(?:about\s+|under\s+|~|<\s*)?)"
        rf"{_TIME}",
        re.I,
    ),
    # Below the millisecond a number needs no keyword ("Cosine 32 ns"): it is a
    # measurement unless a config word qualifies it ("the poll interval is 100
    # µs"). Milliseconds and seconds need a keyword, a verb or a table: timeouts
    # are set in them. A number glued to a name (`poll_10us`) is part of it.
    "time": re.compile(rf"{_BEFORE}{_NUM}\s*{_SUB_MS_UNIT}{_AFTER}", re.I),
}
# A time in a table row is a measurement whatever its column says: a benchmark
# table names its keyword once, in the header. Each time of the row is judged
# on its own (`qualified`): a config word exempts the value it qualifies, in
# its cell, as its column header or as its row label ("| query timeout | 30 s
# |"), and no other value of the row.
TABLE_TIME = re.compile(_TIME, re.I)
# A column header may carry the unit of its cells: "| Latency (ms) |",
# "| Build time [s] |".
HEADER_UNIT = re.compile(r"^(.*?)\s*(?:\(([^()]{1,12})\)|\[([^\[\]]{1,12})\])\s*$")
# A column header names a time unit when one stands as a word in it: "(ms)",
# "[s]", "µs/fact", "Build time (s)¹". A unit after a slash is a rate's
# denominator ("Queries/s"), after an apostrophe a possessive ("Rust's").
HEADER_TIME_UNIT = re.compile(rf"(?<![\w/'’]){_TIME_UNIT}(?![^\W_])", re.I)
READABLE_CELL = re.compile(rf"^{_NUM}(?:\s*{_TIME_UNIT})?$", re.I)

# Fail closed. After parsing, a character Markdown uses as syntax that is still
# in the rendered text is a literal mark ("42* ms", "42\\_ ms"), and a
# superscript or an invisible format character is no part of a number or a
# unit ("42¹ ms", "42​ms"). The guard takes them out of the rendered text
# and reports every number with a time unit it then finds that it did not read
# at the same place. A `*` or `_` with blank space on both sides is an operator
# ("4 * ms"), and an underscore between two letters or digits belongs to a name
# (`poll_10us`): both stay.
MARKDOWN_SYNTAX = frozenset("*_`~\\[]()<>|^#!")
HIDDEN_CATEGORIES = ("No", "Cf")
TEXT_MARK = re.compile(r"(?<!\S)([*_])\1*(?!\S)|(?<=[^\W_])_(?=[^\W_])")
BARE_TIME = re.compile(rf"{_NUM}\s*{_TIME_UNIT}", re.I)
# What glues a number or a unit to a name, as `_BEFORE` and `_AFTER` read it.
# A point before a number is no name: ".5 ms" is a time the guard does not
# read, so it is reported.
GLUED_BEFORE = re.compile(r"\w")
GLUED_AFTER = re.compile(r"[\w-]")


def _segment(words: str) -> str:
    """`words` as a whole word or as one segment of a snake_case identifier:
    `timeout` in `query_timeout_ms`, `ef` in `ef_construction`. A word that
    merely ends in one (`brief`) is neither."""
    return rf"(?<![^\W_])(?:{words})(?![^\W_])"


# A ratio of memory or size is arithmetic, not a measurement: f32 to u8 is 4x.
# A size word names it (memory, bytes, smaller, compression...). Quantization
# does not: it names a technique, and "Quantized search ~3x" is a speed.
SIZE_WORDS = re.compile(
    r"\b(?:memory|compress\w*|smaller|larger|bytes?|bits?|size|storage|RAM|footprint|disk|space|bandwidth"
    r"|[KMG]i?B)\b",
    re.I,
)
# Nor is a ratio a measurement when it scales a parameter or states a bound
# ("0.5x `ef_construction`", "2× minEf", "above 50× the limit", "all factors
# inflated by 1.2×"), or relates configured values ("3x the default",
# CONFIG_WORDS below).
BOUND_WORDS = re.compile(
    r"\b(?:candidates?|registers?|accumulators?|lanes?|limits?|thresholds?|floors?|caps?|factors?|oversampl\w*)\b"
    rf"|{_segment('ef')}|(?-i:[a-z]Ef\b)",
    re.I,
)
# A size, bound or config word must be about the ratio it exempts: one of the
# three nearest words on either side within its clause ("4x less memory",
# "above 10x the threshold"), or the header of its table column or the label
# of its row. Elsewhere on the line it says nothing about that ratio ("4x less
# memory and ~3x the rate", "above 10x the threshold; ~3x the rate": each ~3x
# is a measurement).
QUALIFIER_REACH = 3
CLAUSE_END = re.compile(r"[,;:.!?|]|\s[—–]\s|\b(?:and|but|while|whereas)\b", re.I)
# Nor is a multiplier that counts: calls ("`dot_product_neon` called 3x") or
# SIMD registers ("4x f32x8"). Only that multiplier is exempt, not its line.
COUNTED_CALLS = re.compile(r"\b(?:called|invoked)\s+$", re.I)
COUNTED_REGISTERS = re.compile(r"\s*`?[fiu](?:8|16|32|64)x\d+\b")
# In a code span a number is code when it belongs to a name, a path or an
# option, or is an operand: glued to what precedes it (`poll_10us`,
# `bench/10us/`, `sleep(10us)`), the value of an option (`--poll 10us`), after
# an operator (`wait = 10us`), or multiplying what follows it (`4 × dim`). A
# quantity standing alone in its span (`29.5 us`, `16.3 us/fact`, `~3x`) is a
# figure like any other.
CODE_BEFORE = re.compile(r"(?:[\w./(\[]|[=<>+*×%]\s*|(?:^|\s)--?[A-Za-z][\w-]*\s+)$")
CODE_OPERAND = re.compile(r"\s*[\w(]")

# A configured bound is not a measurement: a latency is left alone when a word
# like these sits between its keyword and its number, or just after the number
# ("query timeout 500 ms", "500 ms by default"), and so does a parameter named
# after one (`query_timeout_ms`). One before the keyword ("the default search
# answers in 2 ms") does not exempt it. Beside a ratio, such a word exempts
# that ratio (`qualified`).
CONFIG_WORDS = re.compile(
    _segment(r"time[sd]?[ -]?outs?|deadline|interval|poll\w*|retr(?:y|ies)|ttl|budget|max_\w+|limit|default"), re.I
)
CONFIG_LOOKAHEAD = 20
# What makes a figure no measurement when it stands beside it: a size, bound or
# config word for a ratio; a config word alone for a time or a rate.
RATIO_QUALIFIERS = (SIZE_WORDS, BOUND_WORDS, CONFIG_WORDS)
CONFIG_QUALIFIERS = (CONFIG_WORDS,)
QUALIFIERS = {"ratio": RATIO_QUALIFIERS, "time": CONFIG_QUALIFIERS, "latency": CONFIG_QUALIFIERS, "throughput": CONFIG_QUALIFIERS}

# The comment markers a doc comment's Markdown sits behind.
RUST_DOC = re.compile(r"^\s*(///|//!)")
TS_DOC = re.compile(r"^\s*(?:/\*+|\*+(?!/)|//+)")
TS_DOC_END = re.compile(r"\*+/\s*$")
FENCE_CLOSE = re.compile(r"^\s*(?:`{3,}|~{3,})\s*$")
DOCSTRING_QUOTES = re.compile(r"\"\"\"|'''")
# A cell boundary in a rendered row: an escaped `\|` stays inside its cell.
CELL_BAR = re.compile(r"(?<!\\)\|")


class Line(NamedTuple):
    """One line of rendered text: what a reader sees on line `number` of a file.

    `where` gives, for each character of `text`, its column in `raw`, the
    file's own line. `code` holds the (start, end) of each code span in `text`,
    backticks included. A table row is rendered `| cell | cell |`, whatever
    its source, and carries its table's rendered header row. `group` is the
    block the line belongs to: two lines of one paragraph share it."""

    number: int
    raw: str
    text: str
    where: tuple[int, ...] = ()
    code: tuple[tuple[int, int], ...] = ()
    row: bool = False
    header: str | None = None
    group: int | None = None


class Heading(NamedTuple):
    number: int
    text: str


class Segment(NamedTuple):
    """Markdown source cut out of a file: its lines, and for each the file line
    number, the file line itself and the column the source line starts at."""

    lines: list[str]
    numbers: list[int]
    raws: list[str]
    offsets: list[int]


def contract_claims(root: Path) -> list[tuple[str, str, bool]]:
    """(file, must_contain, covers_section) for every claim of the contract."""
    data = json.loads((root / CONTRACT).read_text(encoding="utf-8"))
    return [(c["file"], c["must_contain"], c.get("covers_section") is True) for c in data["claims"]]


def documents(root: Path):
    """Yield (path, kind) for every file in scope."""
    readme = root / "README.md"
    if readme.is_file():
        yield readme, "text"
    for path in sorted((root / "docs").rglob("*.md")):
        if "archive" not in path.relative_to(root / "docs").parts:
            yield path, "text"
    for top in ("crates", "sdks", "examples"):
        for path in sorted((root / top).rglob("README.md")):
            if not {"node_modules", "target"} & set(path.parts):
                yield path, "text"
    for path in sorted((root / "crates").glob("*/src/**/*.rs")):
        yield path, "rust"
    # The bindings' own docs mirror the rustdoc: Python docstrings and stubs,
    # and the TypeScript SDK's doc comments.
    for path in sorted((root / "crates").glob("*/python/**/*.py*")):
        if path.suffix in (".py", ".pyi"):
            yield path, "python"
    for path in sorted((root / "sdks").glob("*/src/**/*.ts")):
        if "node_modules" not in path.parts:
            yield path, "typescript"


def comment_pieces(lines: list[str], kind: str):
    """Yield (block, number, start, end): the part line[start:end] of a line
    that holds documentation, and the block of consecutive documentation it
    belongs to (a doc comment, a docstring)."""
    block = 0
    if kind == "python":
        in_docstring = False
        for number, line in enumerate(lines, 1):
            quotes = [match.span() for match in DOCSTRING_QUOTES.finditer(line)]
            if not (in_docstring or quotes):
                continue
            if in_docstring:
                start, rest = 0, quotes
            else:
                block += 1
                start, rest = quotes[0][1], quotes[1:]
            end = rest[0][0] if rest else len(line)
            if len(quotes) % 2:
                in_docstring = not in_docstring
            yield block, number, start, max(start, end)
        return
    previous = None
    for number, line in enumerate(lines, 1):
        if kind == "rust":
            marker = RUST_DOC.match(line)
            if not marker:
                continue
            key, start, end = marker.group(1), marker.end(), len(line)
        else:
            text = line.lstrip()
            if not text.startswith(("*", "/*", "//")):
                continue
            marker = TS_DOC.match(line)
            start = marker.end() if marker else len(line) - len(text)
            closing = TS_DOC_END.search(line)
            key, end = None, closing.start() if closing and closing.start() >= start else len(line)
        if previous != (number - 1, key):
            block += 1
        previous = (number, key)
        yield block, number, start, max(start, end)


def segments(lines: list[str], kind: str):
    """The Markdown sources of a file: the whole of a Markdown file, and each
    doc comment or docstring with its markers and common indentation removed,
    as rustdoc and `inspect.cleandoc` remove them."""
    if kind == "text":
        yield Segment(lines, list(range(1, len(lines) + 1)), lines, [0] * len(lines))
        return
    groups: dict[int, list[tuple[int, int, int]]] = {}
    for block, number, start, end in comment_pieces(lines, kind):
        groups.setdefault(block, []).append((number, start, end))
    for pieces in groups.values():
        bodies = [lines[number - 1][start:end] for number, start, end in pieces]
        rest = bodies[1:] if kind == "python" else bodies
        indents = [len(body) - len(body.lstrip()) for body in rest if body.strip()]
        indent = min(indents, default=0)
        out, offsets = [], []
        for index, ((number, start, _), body) in enumerate(zip(pieces, bodies)):
            cut = len(body) - len(body.lstrip()) if index == 0 and kind == "python" else min(indent, len(body) - len(body.lstrip()))
            out.append(body[cut:])
            offsets.append(start + cut)
        yield Segment(out, [n for n, _, _ in pieces], [lines[n - 1] for n, _, _ in pieces], offsets)


def markdown():
    """The parser: CommonMark, with GFM tables and strikethrough. Entities and
    escapes stay tokens of their own, so each maps back to its source."""
    return MarkdownIt("commonmark").enable(["table", "strikethrough"]).disable("text_join")


def columns(content: str, source: str, start: int) -> tuple[int, list[int]]:
    """Where `content`, one line of a block's inline source, sits in `source`
    from column `start`: its first column and the column of each character. A
    table cell's content has its `\\|` unescaped: the backslash is skipped."""
    at = source.find(content, start)
    if at < 0:
        at = source.find(content.replace("|", "\\|"), start)
    if at < 0:
        at = source.find(content.strip(), start)
    at = max(at, start)
    out, col = [], at
    for char in content:
        if char == "|" and source.startswith("\\|", col):
            col += 1
        out.append(min(col, max(len(source) - 1, 0)))
        col += 1
    return col, out


def render(inline, segment: Segment, cell_from: int | None = None):
    """The rendered lines of an inline token: [(segment line, text, where,
    code)], `where` in segment columns. `cell_from` places a table cell, whose
    source shares its line with the other cells, from that column on."""
    content, first = inline.content, inline.map[0]
    offsets: list[tuple[int, int]] = []  # (segment line, column) per content char
    for k, piece in enumerate(content.split("\n")):
        index = min(first + k, len(segment.lines) - 1)
        _, cols = columns(piece, segment.lines[index], cell_from if cell_from is not None and k == 0 else 0)
        offsets += [(index, col) for col in cols] + [(index, cols[-1] + 1 if cols else 0)]
    chars: list[tuple[str, int]] = []  # (character, content offset); "\n" breaks a line
    code: list[tuple[int, int]] = []
    cursor = 0

    def seek(text: str) -> int:
        at = content.find(text, cursor) if text else -1
        return at if at >= 0 else cursor

    for child in inline.children or ():
        kind = child.type
        if kind == "text" and child.content:
            at = seek(child.content)
            chars += [(char, at + i) for i, char in enumerate(child.content)]
            cursor = at + len(child.content)
        elif kind == "text_special":
            at = seek(child.markup)
            chars += [(char, at) for char in child.content]
            cursor = at + len(child.markup)
        elif kind in ("softbreak", "hardbreak"):
            at = content.find("\n", cursor)
            chars.append(("\n", at if at >= 0 else cursor))
            cursor = at + 1 if at >= 0 else cursor
        elif kind == "code_inline":
            opening = seek(child.markup)
            cursor = opening + len(child.markup)
            body = seek(child.content)
            closing = content.find(child.markup, body + len(child.content))
            begin = len(chars)
            chars.append(("`", opening))
            chars += [(char.replace("`", "'"), body + i) for i, char in enumerate(child.content)]
            chars.append(("`", closing if closing >= 0 else opening))
            code.append((begin, len(chars)))
            cursor = closing + len(child.markup) if closing >= 0 else body + len(child.content)
        elif kind == "html_inline":
            cursor = seek(child.content) + len(child.content)
        elif child.markup and kind.endswith(("_open", "_close")) and not kind.startswith("link"):
            cursor = seek(child.markup) + len(child.markup)
    lines, text, where, start = [], [], [], 0
    for position, (char, at) in enumerate(chars + [("\n", len(content))]):
        if char == "\n":
            if text:
                line = offsets[min(where[0], len(offsets) - 1)][0]
                lines.append(
                    (
                        line,
                        "".join(text),
                        [offsets[min(o, len(offsets) - 1)][1] for o in where],
                        [(s - start, e - start) for s, e in code if start <= s and e <= position],
                    )
                )
            text, where, start = [], [], position + 1
            continue
        text.append(char)
        where.append(at)
    return lines


class _HtmlText(HTMLParser):
    """The text of a raw HTML block, with the (line, column) of each character.
    Tags and comments give no text; an entity gives its character."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=False)
        self.chars: list[tuple[str, int, int]] = []

    def handle_data(self, data: str) -> None:
        line, col = self.getpos()
        for char in data:
            self.chars.append((char, line, col))
            line, col = (line + 1, 0) if char == "\n" else (line, col + 1)

    def handle_entityref(self, name: str) -> None:
        self._reference(html.unescape(f"&{name};"))

    def handle_charref(self, name: str) -> None:
        self._reference(html.unescape(f"&#{name};"))

    def _reference(self, text: str) -> None:
        line, col = self.getpos()
        self.chars += [(char, line, col) for char in text]


def html_lines(token, segment: Segment):
    """The rendered lines of a raw HTML block: its text, tags and comments out."""
    first, last = token.map
    reader = _HtmlText()
    reader.feed("\n".join(segment.lines[first:last]))
    reader.close()
    by_line: dict[int, list[tuple[str, int]]] = {}
    for char, line, col in reader.chars:
        if char != "\n":
            by_line.setdefault(first + line - 1, []).append((char, col))
    for index, chars in sorted(by_line.items()):
        text = "".join(char for char, _ in chars)
        if text.strip():
            yield index, text, [col for _, col in chars]


def row_text(cells: list[tuple[str, list[int], list[tuple[int, int]]]]):
    """A rendered table row, `| cell | cell |`, with its columns and code spans.
    A `|` inside a cell is written `\\|`, as GFM keeps it inside its cell."""
    text, where, code = "|", [cells[0][1][0] if cells and cells[0][1] else 0], []
    for cell, cols, spans in cells:
        text += " "
        where.append(where[-1])
        shift = []
        for char, col in zip(cell, cols):
            if char == "|":
                text += "\\"
                where.append(col)
            shift.append(len(text))
            text += char
            where.append(col)
        code += [(shift[s], shift[e - 1] + 1) for s, e in spans if e > s]
        text += " |"
        where += [where[-1], where[-1]]
    return text, where, code


def read_segment(segment: Segment, parser):
    """Yield the Headings and rendered Lines of a Markdown segment, in order."""
    tokens = parser.parse("\n".join(segment.lines))
    group = 0

    def line_of(index: int, text: str, where: list[int], code, **extra) -> Line:
        offset = segment.offsets[index]
        return Line(
            segment.numbers[index], segment.raws[index], text, tuple(offset + col for col in where), tuple(code), **extra
        )

    i = 0
    while i < len(tokens):
        token = tokens[i]
        if token.type == "heading_open":
            yield Heading(segment.numbers[token.map[0]], segment.raws[token.map[0]].strip())
        if token.type == "inline":
            group += 1
            for index, text, where, code in render(token, segment):
                yield line_of(index, text, where, code, group=group)
        elif token.type == "fence" and token.info.split()[:1] == ["mermaid"]:
            # GitHub draws a mermaid block: its labels are shown, not code.
            for index in range(token.map[0] + 1, token.map[1]):
                text = segment.lines[index]
                if text.strip() and not FENCE_CLOSE.match(text):
                    group += 1
                    yield line_of(index, text, list(range(len(text))), (), group=group)
        elif token.type == "html_block":
            group += 1
            for index, text, where in html_lines(token, segment):
                yield line_of(index, text, where, (), group=group)
        elif token.type == "table_open":
            i, rows = table_rows(tokens, i, segment)
            header = None
            for index, text, where, code, is_header in rows:
                yield line_of(index, text, where, code, row=True, header=None if is_header else header)
                if is_header:
                    header = text
            continue
        i += 1


def table_rows(tokens, i: int, segment: Segment):
    """The rows of the table opening at tokens[i], rendered, and the index after
    it: [(segment line, text, where, code, is_header)]."""
    rows, cells, index, is_header, cell_from = [], [], 0, False, 0
    while tokens[i].type != "table_close":
        token = tokens[i]
        if token.type == "tr_open":
            cells, index, cell_from = [], token.map[0], 0
            is_header = tokens[i - 1].type == "thead_open"
        elif token.type == "inline":
            rendered = render(token, segment, cell_from)
            text, where, code = (rendered[0][1], rendered[0][2], rendered[0][3]) if rendered else ("", [], [])
            if where:
                cell_from = where[-1] + 1
            cells.append((text, where or [cell_from], code))
        elif token.type == "tr_close":
            text, where, code = row_text(cells)
            rows.append((index, text, where, code, is_header))
        i += 1
    return i + 1, rows


def read_document(lines: list[str], kind: str, parser=None):
    """Yield the Headings and rendered Lines of a file, in order."""
    parser = parser or markdown()
    for segment in segments(lines, kind):
        yield from read_segment(segment, parser)


def figures(line: Line):
    """Each (kind, start, end) of a performance figure the line states, as
    positions in `line.text`."""
    yield from line_figures(line)
    yield from table_times(line)
    yield from header_figures(line)


def line_figures(line: Line):
    """The figures the line's own words state, by the rules of KINDS."""
    text = line.text
    for kind, pattern in KINDS.items():
        for match in pattern.finditer(text):
            start, end = match.span()
            if kind == "latency" and CONFIG_WORDS.search(text[start : end + CONFIG_LOOKAHEAD]):
                continue
            if kind == "ratio" and (
                is_code(line, start, end) or counted(text, start, end) or qualified(line, start, end, RATIO_QUALIFIERS)
            ):
                continue
            if kind == "time" and (is_code(line, start, end) or qualified(line, start, end, CONFIG_QUALIFIERS)):
                continue
            if kind == "throughput" and qualified(line, start, end, CONFIG_QUALIFIERS):
                continue
            yield kind, start, end


def table_times(line: Line):
    """Every time of a table row, each judged on its own."""
    if line.row:
        for match in TABLE_TIME.finditer(line.text):
            start, end = match.span()
            if not qualified(line, start, end, CONFIG_QUALIFIERS):
                yield "latency", start, end


def cells(row: str) -> list[tuple[int, int]]:
    """(start, end) of the text of each cell of a rendered table row."""
    bars = [match.start() for match in CELL_BAR.finditer(row)]
    return [(left + 1, right) for left, right in zip(bars, bars[1:])]


def header_figures(line: Line):
    """A results table names its keyword, and often its unit, once, in its
    header: each cell is read with its column's header ("Recall@10" over
    "97.4%", "Latency (ms)" over "3.2"), and yields the cell when that reads
    as a figure."""
    if line.header is None or not line.row:
        return
    names = [line.header[start:end].strip() for start, end in cells(line.header)]
    for column, (start, end) in enumerate(cells(line.text)):
        text, name = line.text[start:end].strip(), names[column] if column < len(names) else ""
        if not text or not name:
            continue
        probes = [f"{name} {text}", f"{text} {name}"]
        unit = HEADER_UNIT.match(name)
        if unit:
            probes.append(f"{unit.group(1)} {text} {unit.group(2) or unit.group(3)}")
        kind = next((kind for probe in probes for kind, _, _ in line_figures(Line(0, probe, probe))), None)
        # A time unit in the header reads a bare cell as a time, as a unit in
        # the cell does ("| Build time (s) |", "[s]" or "µs/fact" over "| 42 |").
        if not kind and HEADER_TIME_UNIT.search(name) and READABLE_CELL.match(text):
            kind = "latency"
        # The cell's own labels still qualify it: its row label and column
        # header, as for any figure of the row ("| query timeout | 30 s |").
        if kind and not qualified(line, start, end, QUALIFIERS.get(kind, ())):
            yield kind, start, end


def hidden_view(text: str) -> tuple[str, list[int]]:
    """`text` with its literal marks, superscripts and format characters taken
    out (see MARKDOWN_SYNTAX), and, for each character left, its index."""
    kept = {i for match in TEXT_MARK.finditer(text) for i in range(*match.span())}
    out, where = [], []
    for at, char in enumerate(text):
        if at not in kept and (char in MARKDOWN_SYNTAX or unicodedata.category(char) in HIDDEN_CATEGORIES):
            continue
        out.append(char)
        where.append(at)
    return "".join(out), where


def glued(text: str, where: list[int], start: int, end: int) -> bool:
    """Whether text[start:end] touches a name on either side in the rendered
    text too: a mark taken out between them is a boundary, not glue."""
    before = start > 0 and where[start - 1] == where[start] - 1 and GLUED_BEFORE.match(text[start - 1])
    after = end < len(text) and where[end] == where[end - 1] + 1 and GLUED_AFTER.match(text[end])
    return bool(before or after)


def in_code_span(line: Line, start: int, end: int) -> bool:
    """Whether line.text[start:end] lies wholly inside one code span."""
    return any(span_start < start and end < span_end for span_start, span_end in line.code)


def unreadable_figures(line: Line):
    """Each (start, end, form) of a figure written in a shape the guard does not
    read exactly, with the form to write instead. Fail closed: such a figure is
    a finding, never a pass."""
    read = {match.span() for match in TABLE_TIME.finditer(line.text)}
    view, where = hidden_view(line.text)
    for match in BARE_TIME.finditer(view):
        if glued(view, where, *match.span()):
            continue
        start, end = where[match.start()], where[match.end() - 1] + 1
        if (start, end) not in read and not in_code_span(line, start, end):
            yield start, end, "`42 ms`"
    if line.header is None or not line.row:
        return
    names = [line.header[start:end].strip() for start, end in cells(line.header)]
    for column, (start, end) in enumerate(cells(line.text)):
        text, name = line.text[start:end].strip(), names[column] if column < len(names) else ""
        if NUMBER.search(text) and HEADER_TIME_UNIT.search(name) and not READABLE_CELL.match(text):
            yield start, end, "`42` or `42 s`"


def wrapped_figures(previous: Line, line: Line):
    """A paragraph wraps: a figure whose keyword ends the previous line and
    whose number begins this one ("recall@10 in" / "0.98 on SIFT1M"), as
    (kind, start, end) in this line."""
    joined = previous.text.rstrip() + " "
    seam = len(joined)
    code = (*previous.code, *((start + seam, end + seam) for start, end in line.code))
    for kind, start, end in line_figures(Line(line.number, line.raw, joined + line.text, code=code)):
        if start < seam < end:
            yield kind, 0, end - seam


def is_code(line: Line, start: int, end: int) -> bool:
    """Whether the number at line.text[start:end] sits in a code span as code:
    part of a name, a path or an option there, or an operand (CODE_BEFORE)."""
    text = line.text
    for span_start, span_end in line.code:
        if span_start < start < span_end:
            before, after = text[span_start + 1 : start], text[end : span_end - 1]
            return bool(CODE_BEFORE.search(before) or (text[end - 1] in "x×" and CODE_OPERAND.match(after)))
    return False


def qualified(line: Line, start: int, end: int, words: tuple[re.Pattern[str], ...]) -> bool:
    """Whether a word of `words` qualifies the figure at line.text[start:end]: a
    near word of its clause, or a label of its table cell."""
    before = CLAUSE_END.split(line.text[:start])[-1].split()[-QUALIFIER_REACH:]
    after = CLAUSE_END.split(line.text[end:])[0].split()[:QUALIFIER_REACH]
    near = (*before, *after, *table_labels(line, start))
    return any(pattern.search(text) for pattern in words for text in near)


def table_labels(line: Line, at: int) -> list[str]:
    """The header of the table column holding position `at`, when the header is
    known, and the label (first cell) of its row; nothing outside a table."""
    if not line.row:
        return []
    row = cells(line.text)
    column = next((i for i, (start, end) in enumerate(row) if start <= at <= end), None)
    if column is None:
        return []
    heads = cells(line.header) if line.header else []
    labels = [line.header[slice(*heads[column])]] if column < len(heads) else []
    if column > 0:
        labels.append(line.text[slice(*row[0])])
    return labels


def counted(text: str, start: int, end: int) -> bool:
    """Whether the multiplier at text[start:end] counts calls or registers."""
    return bool(COUNTED_CALLS.search(text[:start]) or COUNTED_REGISTERS.match(text, end))


def raw_span(line: Line, start: int, end: int) -> tuple[int, int]:
    """Where line.text[start:end] sits in the file's own line."""
    if not line.where:
        return start, end
    cols = line.where[start:end] or (line.where[min(start, len(line.where) - 1)],)
    return min(cols), max(cols) + 1


def covered(raw: str, start: int, end: int, texts: list[str]) -> bool:
    """A line claim covers the figures its `must_contain` overlaps, not the
    whole line: one registered figure must not silence another beside it."""
    for text in texts:
        at = raw.find(text)
        while at >= 0:
            if at < end and at + len(text) > start:
                return True
            at = raw.find(text, at + 1)
    return False


def ambiguous_sections(rel: str, seen: list[str], sections: set[str]) -> list[str]:
    """A section claim must name one heading: one that repeats would cover every
    section of that name."""
    return [
        f"{rel}: the section claim for {text!r} matches {seen.count(text)} headings, not one"
        for text in sorted(sections)
        if seen.count(text) != 1
    ]


def violations(root: Path) -> list[str]:
    if MarkdownIt is None:
        raise RuntimeError(f"{PARSER_REQUIREMENT} is not installed")
    claims = contract_claims(root)
    parser = markdown()
    found = []
    for path, kind in documents(root):
        rel = path.relative_to(root).as_posix()
        if path.name == "CHANGELOG.md":
            continue
        lines_claimed = [text for file, text, section in claims if file == rel and not section]
        sections_claimed = {text.strip() for file, text, section in claims if file == rel and section}
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        events = list(read_document(lines, kind, parser))
        seen_headings = [event.text for event in events if isinstance(event, Heading)]
        if kind == "text":
            found.extend(ambiguous_sections(rel, seen_headings, sections_claimed))
        heading = previous = None
        reported: set[tuple[str, int]] = set()
        for event in events:
            if isinstance(event, Heading):
                if kind == "text":
                    heading = event.text
                continue
            line = event
            wrapped = ()
            if previous and previous.group == line.group and previous.number == line.number - 1:
                wrapped = tuple(wrapped_figures(previous, line))
            previous = line
            for _, _, form in unreadable_figures(line):
                if ("unreadable", line.number) not in reported:
                    reported.add(("unreadable", line.number))
                    found.append(f"{rel}:{line.number}: unreadable figure: rewrite as {form}: {line.raw.strip()[:140]}")
                break
            if heading in sections_claimed:
                continue
            for figure, start, end in (*figures(line), *wrapped):
                if not covered(line.raw, *raw_span(line, start, end), lines_claimed):
                    if ("figure", line.number) not in reported:
                        reported.add(("figure", line.number))
                        found.append(
                            f"{rel}:{line.number}: {figure} with no registered measurement: {line.raw.strip()[:140]}"
                        )
                    break
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args(argv)
    if MarkdownIt is None:
        print(
            f"ERROR: {PARSER_REQUIREMENT} is not installed, so the figure guard could not run "
            f"(python3 -m pip install {PARSER_REQUIREMENT}).",
            file=sys.stderr,
        )
        return 2
    found = violations(args.root)
    for item in found:
        print(item)
    if found:
        print(
            f"\n{len(found)} performance figure(s) name no measurement. Register each in "
            f"{CONTRACT} (source, measured_on, measured_machine, measured_version), "
            "or remove the number.",
            file=sys.stderr,
        )
        return 1
    print("every performance figure is registered in the promise contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
