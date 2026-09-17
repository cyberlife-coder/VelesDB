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

A figure is a number with a unit, read with the words around it: a keyword or
a verb before it ("p50 450 µs", "answers in 2 ms", "takes 42 s"), the end of
the line above when a comment or a paragraph wraps ("recall@10 in" / "0.98 on
SIFT1M"), or the column header of its table cell ("| Recall@10 |" over
"| 97.4% |", "| Latency (ms) |" over "| 3.2 |"). Below the millisecond the
unit alone is enough ("Cosine 32 ns"), and every time in a table row counts.
Not a figure: a memory or compression ratio (f32 to u8 is 4x less memory by
arithmetic, not by measurement), a configured value (a timeout, a poll
interval, a retry count, a rate limit), a scale applied to a parameter (0.5x
`ef_construction`) and a count (a kernel called 3x, 4x f32x8 registers). The
word that makes a value one of these must stand beside it, in its clause or as
its table cell's label, and exempts that value alone. A number in a code span
is exempt only as code (`sleep(10us)`, `4 × dim`): a quantity standing alone
in one (`29.5 us`) is a figure.

It fails closed on time figures, by construction rather than by a list of
shapes. It reads a time in exactly three forms: a number with its unit
("42 ms"), a bare number under a header where a time unit stands as a word
("| Build time (s) |", "| µs/fact |" over "| 42 |"), and either inside one
emphasis pair ("**42**", "__42 ms__"). It takes the markup out of every line
(Markdown and GFM syntax, HTML tags and entities, link targets, superscripts,
invisible characters) and reports any number with a time unit left there that
it does not read on the line itself, at the same place, as an unreadable
figure, whatever the register holds: "**42** ms", "42¹ ms", "<b>42</b> ms",
"42&nbsp;ms", "`42` ms", "~~42~~ ms". So is any other cell under a time-unit
header ("~42", "42¹"). A code span, a fenced block and a doc-test hold code:
the unreadable rule leaves them alone.

What it cannot see: this is a heuristic. It does not read
- a number written with no unit and no such word ("4,000 of them a second");
- a duration whose keyword or verb is not next to it ("14.19 s cold against
  0.22 s warm", "held every call for 46-52 s", "roughly 16 seconds"), or one
  given in minutes ("1 min 59 s", "56 minutes");
- a time in a table row whose label is a config word ("| Budget pressure |
  ... | 1.07 ms |");
- a figure drawn in an image or a chart.
What binds a figure to its run is the register, not this guard: a figure it
misses is a promise all the same, and belongs in the contract.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

CONTRACT = Path("docs/reference/promise-contract.json")

# The one number grammar every rule reads.
_NUM = r"\d+(?:[.,]\d+)*"
NUMBER = re.compile(_NUM)
# Markdown emphasis or code may wrap a figure: `**130x** faster`.
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
# is a user story, not a microsecond). An emphasis underscore is no part of a
# name: `__42 ms__` is 42 ms.
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
# A time in a table row, in a Markdown file or a doc comment, is a measurement
# whatever its column says: a benchmark table names its keyword once, in the
# header. Each time of the row is judged on its own (`qualified`): a config
# word exempts the value it qualifies, in its cell, as its column header or as
# its row label ("| query timeout | 30 s |"), and no other value of the row.
TABLE_TIME = re.compile(_TIME, re.I)
# A column header may carry the unit of its cells: "| Latency (ms) |",
# "| Build time [s] |". Emphasis around a cell's number is no part of it.
HEADER_UNIT = re.compile(r"^(.*?)\s*(?:\(([^()]{1,12})\)|\[([^\[\]]{1,12})\])\s*$")
# One emphasis pair around a whole cell ("**42**", "**42 ms**") is no part of it.
ONE_PAIR = re.compile(r"^(\*\*|__|\*|_)(\S(?:.*\S)?)\1$")

# Fail closed, by construction. The guard reads a time in exactly three forms:
# a number with its unit ("42 ms", `TABLE_TIME`), a bare number under a header that
# names a time unit ("| µs/fact |" over "| 42 |"), and either inside one
# emphasis pair ("**42**", "__42 ms__"). It does not list the shapes it cannot
# read: it takes the markup out of each line (`markup_free`) and reports every
# number with a time unit left there that `TABLE_TIME` does not read on the raw
# line, at the same place, as an unreadable figure, whatever the register
# holds: "**42** ms", "42¹ ms", "42 ms¹", "<b>42</b> ms", "42&nbsp;ms",
# "`42` ms", "~~42~~ ms".
#
# The markup taken out: every character CommonMark and GFM use as syntax
# (emphasis, code, strikethrough, escapes, links and images, raw HTML, tables,
# footnotes, headings), an HTML tag or entity, a link target,
# a superscript and an invisible format character. A `*` or `_` with blank
# space on both sides is no emphasis ("4 * ms"), and an underscore between two
# letters or digits belongs to a name (`poll_10us`): both stay. So do dashes,
# quotes and every other mark of the text itself.
#
# Code is not prose: a time wholly inside a code span, a fenced block or a
# doc-test is left to the rules above.
MARKDOWN_SYNTAX = frozenset("*_`~\\[]()<>|^#!")
HIDDEN_CATEGORIES = ("No", "Cf")
MARKUP = re.compile(r"</?[A-Za-z][^<>]*>|&(?:[A-Za-z][A-Za-z0-9]*|#\d+|#[xX][0-9A-Fa-f]+);")
LINK_TARGET = re.compile(r"\]\([^()\s]*\)")
TEXT_MARK = re.compile(r"(?<!\S)([*_])\1*(?!\S)|(?<=[^\W_])_(?=[^\W_])")
BARE_TIME = re.compile(rf"{_NUM}\s*{_TIME_UNIT}", re.I)
# What glues a number or a unit to a name, as `_BEFORE` and `_AFTER` read it
# on the raw line. A point before a number is no name: ".5 ms" is a time the
# guard does not read, so it is reported.
GLUED_BEFORE = re.compile(r"\w")
GLUED_AFTER = re.compile(r"[\w-]")
# A column header names a time unit when one stands as a word in it: "(ms)",
# "[s]", "µs/fact", "**Build time (s)**". A unit after a slash is a rate's
# denominator ("Queries/s"), after an apostrophe a possessive ("Rust's").
HEADER_TIME_UNIT = re.compile(rf"(?<![\w/'’]){_TIME_UNIT}(?![^\W_])", re.I)
READABLE_CELL = re.compile(rf"^{_NUM}(?:\s*{_TIME_UNIT})?$", re.I)
# What leads a doc comment's text, so two wrapped lines join on their words.
LEAD = re.compile(r"^\s*(?:(?:///|//!|\*)\s*)?")


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
CODE_SPAN = re.compile(r"`[^`]*`")
# In a code span a number is code when it belongs to a name, a path or an
# option, or is an operand: glued to what precedes it (`poll_10us`,
# `bench/10us/`, `sleep(10us)`), the value of an option (`--poll 10us`), after
# an operator (`wait = 10us`), or multiplying what follows it (`4 × dim`). A
# quantity standing alone in its span (`29.5 us`, `16.3 us/fact`, `~3x`) is a
# figure like any other.
CODE_BEFORE = re.compile(r"(?:[\w./(\[]|[=<>+*×%]\s*|(?:^|\s)--?[A-Za-z][\w-]*\s+)$")
CODE_OPERAND = re.compile(r"\s*[\w(]")
FENCE = re.compile(r"^\s*(?:```|~~~)")
# A Markdown table row, also inside a doc comment, and the rule under its header.
TABLE_ROW = re.compile(r"^\s*(?:(?:///|//!|\*)\s*)?\|")
TABLE_RULE = re.compile(r"^\s*(?:(?:///|//!|\*)\s*)?\|[\s:|-]*-[\s:|-]*$")

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


HEADING = re.compile(r"^#{1,6}\s")


def contract_claims(root: Path) -> list[tuple[str, str, bool]]:
    """(file, must_contain, covers_section) for every claim of the contract."""
    data = json.loads((root / CONTRACT).read_text(encoding="utf-8"))
    return [(c["file"], c["must_contain"], c.get("covers_section") is True) for c in data["claims"]]


def doc_lines(lines: list[str], kind: str):
    """Yield (number, line) for the lines of a file that are documentation:
    all of a Markdown file, a Rust doc comment, a Python docstring, a
    TypeScript comment."""
    in_docstring = False
    for number, line in enumerate(lines, 1):
        stripped = line.lstrip()
        if kind == "text":
            yield number, line
        elif kind == "rust":
            if stripped.startswith(("///", "//!")):
                yield number, line
        elif kind == "typescript":
            if stripped.startswith(("*", "/*", "//")):
                yield number, line
        else:
            quotes = line.count('"""') + line.count("\'\'\'")
            if in_docstring or quotes:
                yield number, line
            if quotes % 2:
                in_docstring = not in_docstring


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


def figures(line: str, header: str | None = None):
    """Each (kind, start, end) of a performance figure the line states.
    `header` is the header row of the table the line belongs to, if any."""
    yield from line_figures(line, header)
    yield from table_times(line, header)
    yield from header_figures(line, header)


def line_figures(line: str, header: str | None = None):
    """The figures the line's own words state, by the rules of KINDS."""
    for kind, pattern in KINDS.items():
        for match in pattern.finditer(line):
            start, end = match.span()
            if kind == "latency" and CONFIG_WORDS.search(line[start : end + CONFIG_LOOKAHEAD]):
                continue
            if kind == "ratio" and (
                is_code(line, start, end)
                or counted(line, start, end)
                or qualified(line, start, end, header, RATIO_QUALIFIERS)
            ):
                continue
            if kind == "time" and (is_code(line, start, end) or qualified(line, start, end, header, CONFIG_QUALIFIERS)):
                continue
            if kind == "throughput" and qualified(line, start, end, header, CONFIG_QUALIFIERS):
                continue
            yield kind, start, end


def table_times(line: str, header: str | None):
    """Every time of a table row, each judged on its own."""
    if TABLE_ROW.match(line):
        for match in TABLE_TIME.finditer(line):
            start, end = match.span()
            if not qualified(line, start, end, header, CONFIG_QUALIFIERS):
                yield "latency", start, end


# A cell boundary: GFM keeps an escaped `\|` inside its cell.
CELL_BAR = re.compile(r"(?<!\\)\|")


def cells(row: str) -> list[tuple[int, int]]:
    """(start, end) of the text of each cell of a table row."""
    bars = [match.start() for match in CELL_BAR.finditer(row)]
    return [(left + 1, right) for left, right in zip(bars, bars[1:])]


def header_figures(line: str, header: str | None):
    """A results table names its keyword, and often its unit, once, in its
    header: each cell is read with its column's header ("Recall@10" over
    "97.4%", "Latency (ms)" over "3.2"), and yields the cell when that reads
    as a figure."""
    if header is None or not TABLE_ROW.match(line):
        return
    names = [header[start:end].strip() for start, end in cells(header)]
    for column, (start, end) in enumerate(cells(line)):
        text, name = line[start:end].strip(), names[column] if column < len(names) else ""
        if not text or not name:
            continue
        probes = [f"{name} {text}", f"{text} {name}"]
        unit = HEADER_UNIT.match(name)
        bare = inner(text)
        sign = unit and (unit.group(2) or unit.group(3))
        if unit:
            probes.append(f"{unit.group(1)} {bare} {sign}")
        kind = next((kind for probe in probes for kind, _, _ in line_figures(probe)), None)
        # A time unit in the header reads a bare cell as a time, as a unit in
        # the cell does ("| Build time (s) |", "[s]" or "µs/fact" over "| 42 |",
        # "| **42** |").
        if not kind and HEADER_TIME_UNIT.search(name) and READABLE_CELL.match(bare):
            kind = "latency"
        # The cell's own labels still qualify it: its row label and column
        # header, as for any figure of the row ("| query timeout | 30 s |").
        if kind and not qualified(line, start, end, header, QUALIFIERS.get(kind, ())):
            yield kind, start, end


def inner(text: str) -> str:
    """A cell's text inside its one emphasis pair, if it has one."""
    pair = ONE_PAIR.match(text)
    return pair.group(2) if pair else text


def markup_free(line: str) -> tuple[str, list[int]]:
    """The line with its markup taken out (see MARKDOWN_SYNTAX), and, for each
    character left, its index in the line."""
    dropped = set()
    for match in MARKUP.finditer(line):
        dropped.update(range(*match.span()))
    for match in LINK_TARGET.finditer(line):
        dropped.update(range(match.start() + 1, match.end()))
    kept = {i for match in TEXT_MARK.finditer(line) for i in range(*match.span())}
    text, where = [], []
    for at, char in enumerate(line):
        if at in dropped or (
            at not in kept and (char in MARKDOWN_SYNTAX or unicodedata.category(char) in HIDDEN_CATEGORIES)
        ):
            continue
        text.append(char)
        where.append(at)
    return "".join(text), where


def glued(text: str, where: list[int], start: int, end: int) -> bool:
    """Whether text[start:end] touches a name on either side in the raw line
    too: markup taken out between them is a boundary, not glue."""
    before = start > 0 and where[start - 1] == where[start] - 1 and GLUED_BEFORE.match(text[start - 1])
    after = end < len(text) and where[end] == where[end - 1] + 1 and GLUED_AFTER.match(text[end])
    return bool(before or after)


def in_code_span(line: str, start: int, end: int) -> bool:
    """Whether line[start:end] lies wholly inside one code span."""
    return any(span.start() < start and end < span.end() for span in CODE_SPAN.finditer(line))


def unreadable_figures(line: str, header: str | None):
    """Each (start, end, form) of a figure written in a shape the guard does not
    read exactly, with the form to write instead. Fail closed: such a figure is
    a finding, never a pass."""
    text, where = markup_free(line)
    read = {match.span() for match in TABLE_TIME.finditer(line)}
    for match in BARE_TIME.finditer(text):
        if glued(text, where, *match.span()):
            continue
        start, end = where[match.start()], where[match.end() - 1] + 1
        if (start, end) not in read and not in_code_span(line, start, end):
            yield start, end, "`42 ms` or `**42 ms**`"
    if header is None or not TABLE_ROW.match(line):
        return
    names = [header[start:end].strip() for start, end in cells(header)]
    for column, (start, end) in enumerate(cells(line)):
        text, name = line[start:end].strip(), names[column] if column < len(names) else ""
        if NUMBER.search(text) and HEADER_TIME_UNIT.search(name) and not READABLE_CELL.match(inner(text)):
            yield start, end, "`42`, `42 s` or `**42**`"


def wrapped_figures(previous: str, line: str):
    """A comment or a paragraph wraps: a figure whose keyword ends the previous
    line and whose number begins this one ("recall@10 in" / "0.98 on SIFT1M"),
    as (kind, start, end) in this line."""
    head, body = LEAD.match(previous).end(), LEAD.match(line).end()
    joined = previous[head:].rstrip() + " "
    seam = len(joined)
    for kind, start, end in line_figures(joined + line[body:]):
        if start < seam < end:
            yield kind, body, body + end - seam


def is_code(line: str, start: int, end: int) -> bool:
    """Whether the number at line[start:end] sits in a code span as code: part
    of a name, a path or an option there, or an operand (CODE_BEFORE)."""
    for span in CODE_SPAN.finditer(line):
        if span.start() < start < span.end():
            before, after = line[span.start() + 1 : start], line[end : span.end() - 1]
            return bool(CODE_BEFORE.search(before) or (line[end - 1] in "x×" and CODE_OPERAND.match(after)))
    return False


def qualified(line: str, start: int, end: int, header: str | None, words: tuple[re.Pattern[str], ...]) -> bool:
    """Whether a word of `words` qualifies the figure at line[start:end]: a
    near word of its clause, or a label of its table cell."""
    before = CLAUSE_END.split(line[:start])[-1].split()[-QUALIFIER_REACH:]
    after = CLAUSE_END.split(line[end:])[0].split()[:QUALIFIER_REACH]
    near = (*before, *after, *table_labels(line, start, header))
    return any(pattern.search(text) for pattern in words for text in near)


def table_labels(line: str, at: int, header: str | None) -> list[str]:
    """The header of the table column holding position `at`, when the header is
    known, and the label (first cell) of its row; nothing outside a table."""
    if not TABLE_ROW.match(line):
        return []
    row = cells(line)
    column = next((i for i, (start, end) in enumerate(row) if start <= at <= end), None)
    if column is None:
        return []
    heads = cells(header) if header else []
    labels = [header[slice(*heads[column])]] if column < len(heads) else []
    if column > 0:
        labels.append(line[slice(*row[0])])
    return labels


def is_prose(line: str) -> bool:
    """A line of running text: not blank, not a table row, a heading or a fence."""
    text = line[LEAD.match(line).end() :]
    return bool(text.strip()) and not (TABLE_ROW.match(line) or HEADING.match(text) or FENCE.match(text))


def counted(line: str, start: int, end: int) -> bool:
    """Whether the multiplier at line[start:end] counts calls or registers."""
    return bool(COUNTED_CALLS.search(line[:start]) or COUNTED_REGISTERS.match(line, end))


def covered(line: str, start: int, end: int, texts: list[str]) -> bool:
    """A line claim covers the figures its `must_contain` overlaps, not the
    whole line: one registered figure must not silence another beside it."""
    for text in texts:
        at = line.find(text)
        while at >= 0:
            if at < end and at + len(text) > start:
                return True
            at = line.find(text, at + 1)
    return False


def headings(lines: list[str]) -> list[str]:
    """The Markdown headings of a file, outside fenced code."""
    out, in_fence = [], False
    for line in lines:
        if FENCE.match(line):
            in_fence = not in_fence
        elif not in_fence and HEADING.match(line):
            out.append(line.strip())
    return out


def ambiguous_sections(rel: str, lines: list[str], sections: set[str]) -> list[str]:
    """A section claim must name one heading: one that repeats would cover every
    section of that name."""
    seen = headings(lines)
    return [
        f"{rel}: the section claim for {text!r} matches {seen.count(text)} headings, not one"
        for text in sorted(sections)
        if seen.count(text) != 1
    ]


def violations(root: Path) -> list[str]:
    claims = contract_claims(root)
    found = []
    for path, kind in documents(root):
        rel = path.relative_to(root).as_posix()
        if path.name == "CHANGELOG.md":
            continue
        lines_claimed = [text for file, text, section in claims if file == rel and not section]
        sections_claimed = {text.strip() for file, text, section in claims if file == rel and section}
        heading = None
        in_fence = False
        header = previous = None
        prose = None  # (number, line) of the previous line, when it is prose
        last = 0  # the number of the previous documentation line
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        if kind == "text":
            found.extend(ambiguous_sections(rel, lines, sections_claimed))
        for number, line in doc_lines(lines, kind):
            # A doc comment's fence (a doc-test) opens after its `///`, and the
            # comment ends where a line of code interrupts it.
            fence = FENCE.match(line if kind == "text" else line[LEAD.match(line).end() :])
            if kind != "text" and number != last + 1:
                in_fence = False
            last = number
            if fence:
                in_fence = not in_fence
            elif kind == "text" and not in_fence and HEADING.match(line):
                heading = line.strip()
            if TABLE_RULE.match(line):
                header = previous
            elif not TABLE_ROW.match(line):
                header = None
            previous = line
            wrapped = ()
            if prose and prose[0] == number - 1 and is_prose(line):
                wrapped = wrapped_figures(prose[1], line)
            prose = (number, line) if is_prose(line) else None
            for _, _, form in () if fence or in_fence else unreadable_figures(line, header):
                found.append(f"{rel}:{number}: unreadable figure: rewrite as {form}: {line.strip()[:140]}")
                break
            if heading in sections_claimed:
                continue
            for figure, start, end in (*figures(line, header), *wrapped):
                if not covered(line, start, end, lines_claimed):
                    found.append(
                        f"{rel}:{number}: {figure} with no registered measurement: {line.strip()[:140]}"
                    )
                    break
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args(argv)
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
