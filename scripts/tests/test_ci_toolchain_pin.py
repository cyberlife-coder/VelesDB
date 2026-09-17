"""Every build of this repository installs rust-toolchain.toml's toolchain, from that file, first.

rustup resolves ``rust-toolchain.toml`` on every ``cargo`` or ``rustc`` call made
inside the repository, whatever toolchain a step installed. A version written in
a workflow is therefore dead, and worse than dead: when it is not the file's, or
lacks the file's components, the first cargo call makes rustup install the
file's toolchain in the middle of a build step. That implicit install is where
``Python Integrations`` and ``Python SDK Tests`` died, intermittently, inside
``pip install ./crates/velesdb-python`` (runs 34509954412, 34597461423,
34656653039, 34760724019)::

    info: syncing channel updates for 1.90-x86_64-unknown-linux-gnu
    info: downloading 6 components
    info: rolling back changes
    error: failed to install component: 'clippy-preview-x86_64-unknown-linux-gnu',
    detected conflict: 'bin/cargo-clippy'

So the file is the only pin, and only an install step may install it. Two layers hold that.

At runtime, every workflow sets ``RUSTUP_AUTO_INSTALL: "0"`` in its top-level
``env`` (rustup 1.28.1 and later honour it). A cargo, rustc or rustup call that
reaches a toolchain no earlier step installed then fails at once with rustup's
``error: toolchain '<name>' is not installed`` instead of installing it
mid-build, whatever shell form reached it. That setting, not the static scan
below, is what catches every call: shell is not parsed here, and a scan of it
never converges. The setting does not reach inside a Docker build or container,
which is why Dockerfiles are checked on their own.

Statically, over every workflow (``*.yml``, ``*.yaml``), every composite action
and every Dockerfile, this suite holds that:

* every workflow sets ``RUSTUP_AUTO_INSTALL`` to ``"0"`` in its top-level
  ``env`` (a composite action, on each step), and the name appears nowhere else
  in the document: not in another ``env``, a ``run``, a ``shell`` or
  ``defaults``, since a mention can set, unset or re-enable it in forms no scan
  enumerates. Nothing runs ``rustup set auto-install`` either;
* nothing names a toolchain but ``nightly``: no ``RUST_VERSION``, no
  ``*toolchain`` key anywhere in a workflow, no ``RUSTUP_TOOLCHAIN``
  assignment (key, inline, ``export``, ``$GITHUB_ENV`` or Dockerfile ``ENV``),
  no ``rustup toolchain install|add``, ``install``, ``update``, ``default``,
  ``override set|add`` or ``run`` naming one, no ``cargo +<toolchain>`` (by
  path too), no ``FROM rust:<version>`` once ``ARG`` defaults are expanded,
  and the file's channel nowhere outside a comment;
* a call the scan reads finds its toolchain installed by an earlier step of the
  same job. It reads a ``Swatinem/rust-cache`` step (its ``cargo metadata``)
  and, in each ``run`` line outside a heredoc body, split at ``&&``, ``||``,
  ``;``, pipes and parentheses, a command whose text holds ``cargo``, ``rustc``
  or ``rustdoc``, ``cargo +X``, ``maturin build|develop``, ``wasm-pack``,
  ``napi build`` or ``pip install ... ./crates/``. The toolchain it needs is
  ``X`` for ``cargo +X``; else the ``RUSTUP_TOOLCHAIN`` or rustup override in
  force; else the file of the checkout of this repository the call runs in,
  following ``working-directory``, ``cd``, ``pushd`` and ``popd`` to a literal
  directory, installed by ``rustup toolchain install`` with no toolchain name.
  Outside every checkout of this repository there is no such file: only the
  runner's default toolchain, or another repository's;
* the scan also reads a command whose first word is ``bash``, ``sh``, ``zsh``,
  ``python``, ``python3``, ``node`` or ``pwsh`` followed by a script path, or a
  path run as the command: that command reaches cargo when the script's text
  names cargo, rustc, rustdoc, rustup, maturin, wasm-pack or cross, or names,
  by a literal path that resolves, a script that does. A mention counts as a
  call: a script that only mentions one is listed in
  ``SCRIPTS_THAT_ONLY_NAME_A_TOOL`` with its reason. A script path it cannot
  resolve or read is a finding; a nested name holding ``$`` is not followed.
  Not read: heredoc bodies, wrapper words (``source``, ``.``, ``exec``,
  ``sudo``, ``timeout``, ``env <options>``, ``xargs``), ``python -m`` and what
  an installed module loads;
* an install under an ``if:`` covers only calls under the identical ``if:``
  (``${{ }}`` and spacing aside), and ``success()`` is no condition. An install
  under ``always()`` covers every later call. An install with no ``if:`` covers
  every later call except one that can run after an earlier step failed: an
  ``if:`` holding ``always()``, ``failure()`` or ``cancelled()``;
* each ``nightly`` pin carries a ``# nightly: <why>`` comment of its own step or
  key, naming what the job runs on nightly -- a ``-Z`` flag, miri, cargo fuzz or
  cargo careful -- as its ``run:`` values and ``*FLAGS`` variables show, never
  its keys or step names;
* no ``actions/cache`` step saves ``~/.rustup`` or ``~/.cargo/bin``;
* no tracked file claims loom runs on nightly, and every loom command a doc
  gives is one CI runs.

Workflows are read by PyYAML, so flow mappings, anchors, aliases, merge keys
and quoted keys mean what YAML says; a document PyYAML rejects is a finding.
Some forms the static scan cannot place are findings rather than passes: an
``env`` or ``with`` that is not a mapping, a shell line it cannot tokenize, a
``cd`` target holding ``$`` or a command substitution, a directory it cannot
resolve, a Dockerfile heredoc, a COPY or RUN in exec form (options dropped
first), a COPY of rust-toolchain.toml whose landing it cannot tell, or a
``FROM`` still holding ``$`` once ``ARG`` defaults are expanded. Every other
shell form is left to ``RUSTUP_AUTO_INSTALL``.

PyYAML is the one dependency: every job that runs this suite installs it at one
pin, and without it the import below fails loudly instead of skipping.
"""

from __future__ import annotations

import json
import posixpath
import re
import shlex
import subprocess
import tempfile
import tomllib
import unittest
from dataclasses import dataclass, field
from pathlib import Path
from unittest import mock

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"

ALLOWED = "nightly"
TOOLCHAIN_ACTION = "dtolnay/rust-toolchain@"
CACHE_ACTION = "actions/cache@"
RUST_CACHE_ACTION = "Swatinem/rust-cache@"
CHECKOUT_ACTION = "actions/checkout@"
RUST_ACTIONS = (TOOLCHAIN_ACTION, RUST_CACHE_ACTION, "PyO3/maturin-action@")
# `repository:` values that check out this repository.
THIS_REPOSITORY = ("", "${{ github.repository }}")
# What a call resolves to with no rust-toolchain.toml of this repository to read,
# or when the guard cannot tell which one governs it; a directory it cannot resolve.
OUTSIDE = "<outside every checkout>"
UNREADABLE = "<unreadable>"
UNKNOWN_DIR = "<unknown directory>"
FOREIGN = "foreign:"
MERGE_TAG = "tag:yaml.org,2002:merge"
PINS_UNEXPLAINED = "pins nightly without a comment `# nightly: <why>`"

# A call that resolves its toolchain through rust-toolchain.toml or RUSTUP_TOOLCHAIN:
# cargo, rustc or rustdoc, bare or by path, but not `~/.cargo/...`, `cargo-foo`
# or `cargo +<toolchain>`...
PLAIN_CALL = r"(?<![\w.+-])(?:cargo|rustc|rustdoc)(?:\.exe)?(?![\w.-])(?!\s+\+)"
# ...or a build that spawns one.
BUILD_RE = re.compile(
    PLAIN_CALL
    + r"|\bmaturin\s+(?:build|develop)\b"
    + r"|\bwasm-pack\b"
    + r"|\bnapi\s+build\b"
    + r"|\bpip\s+install\b[^\n]*\./crates/"
)
PLUS_RE = re.compile(r"(?<![\w.-])(?:cargo|rustc|rustdoc)(?:\.exe)?\s+\+([^\s\"'`;&|)]+)")
RUSTUP_RE = re.compile(
    r"\brustup(?:\.exe)?\s+(toolchain\s+(?:install|add)|install|update|default|override\s+(?:set|add)|run)\b([^\n;&|]*)"
)
ASSIGN_RE = re.compile(r"\bRUSTUP_TOOLCHAIN\s*=\s*[\"']?([^\s\"'`;&|)]+)")
ASSIGNMENT_WORD_RE = re.compile(r"^[A-Za-z_]\w*=")
DOCKER_ENV_RE = re.compile(r"^\s*ENV\s+RUSTUP_TOOLCHAIN\s+([^\s=]+)")
TOOLCHAIN_KEY_RE = re.compile(r"^(?:[\w-]*toolchain|RUSTUP_TOOLCHAIN)$")
# A Dockerfile heredoc opener; `<<<` is a here-string, not one.
HEREDOC_RE = re.compile(r"(?<!<)<<(?!<)-?\s*(['\"]?)([A-Za-z_]\w*)\1")
ARG_REF_RE = re.compile(r"\$\{(\w+)(?::?-([^}]*))?\}|\$(\w+)")
SEPARATORS = frozenset({"&&", "||", ";", ";;", "|", "&", "(", ")", "{", "}"})
SHELL_KEYWORDS = frozenset({"then", "else", "do", "if", "elif", "while", "until", "!", "time"})
DIRECTORY_COMMANDS = frozenset({"cd", "pushd", "popd"})
INSTALL_VERBS = frozenset({"toolchain install", "toolchain add", "install"})
OVERRIDE_VERBS = frozenset({"override set", "override add"})
# RUST_VERSION as a reference, never inside a longer name such as
# CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS (a cargo resolver setting).
RUST_VERSION_RE = re.compile(r"(?<![\w.$])RUST_VERSION\s*[:=]|\benv\.RUST_VERSION\b|\$\{?RUST_VERSION\b")
TOOLCHAIN_PATHS_RE = re.compile(r"\.rustup\b|\.cargo/bin\b")
VALUE_FLAGS = frozenset({"--profile", "-c", "--component", "-t", "--target"})
NIGHTLY_REASON_RE = re.compile(r"^\s*#\s*nightly:\s*\S")
# What needs nightly, as a reason names it and a job runs it.
NIGHTLY_NEED_RE = re.compile(r"-Z\s*([A-Za-z][\w-]*)|\b(miri|fuzz|careful)\b", re.IGNORECASE)
FROM_VERSION_RE = re.compile(r"^(?:[\w.-]+/)*rust:(\d[\w.-]*)$", re.IGNORECASE)
DOCKERFILE_NAME_RE = re.compile(r"(?:^|/)(?:Dockerfile(?:\.[\w.-]+)?|[\w.-]+\.Dockerfile)$")
DOC_SUFFIXES = frozenset({".md", ".rs", ".toml", ".yml", ".yaml", ".txt", ".py", ".sh", ".ps1"})
LOOM_CLAIM_EXEMPT = {
    "CHANGELOG.md": "history is not rewritten",
    "scripts/tests/test_ci_toolchain_pin.py": "the checker spells the words it looks for",
}
COMMENT_RE = re.compile(r"^\s*#")
REACH = {
    None: "builds the repository with the base image's toolchain, not rust-toolchain.toml's",
    OUTSIDE: "reaches cargo outside every checkout, on the runner's default toolchain",
    UNREADABLE: "reaches cargo where the guard cannot tell which rust-toolchain.toml governs",
}
INSTALL_ELSEWHERE = {
    OUTSIDE: "installs outside every checkout, so not rust-toolchain.toml's toolchain",
    UNREADABLE: "installs where the guard cannot tell which rust-toolchain.toml governs",
}
# `if:` values: `success()` is what a step without one runs under; an install under
# `always()` runs whenever any later step of the job can. A call whose `if:` holds a
# status function other than `success()` can run after an earlier step failed, and so
# after an install with no `if:` was skipped.
UNCONDITIONAL = ""
ALWAYS = "always()"
RUNS_AFTER_FAILURE_RE = re.compile(r"\b(?:always|failure|cancelled)\(\)")
# The runtime backstop: rustup refuses to install a toolchain a call needs.
AUTO_INSTALL = "RUSTUP_AUTO_INSTALL"
AUTO_INSTALL_OFF = "0"
AUTO_INSTALL_MENTION_RE = re.compile(re.escape(AUTO_INSTALL) + r"|\brustup(?:\.exe)?\s+set\s+auto-install\b")
AUTO_INSTALL_PAST_TOP_LEVEL = f"names {AUTO_INSTALL} past the top-level env key"
EXPRESSION_RE = re.compile(r"^\$\{\{\s*(.*?)\s*\}\}$")
# A repository script a step runs: `<interpreter> [options] <path>`, or `<path>` itself.
INTERPRETER_RE = re.compile(r"^(?:bash|sh|zsh|pwsh|node|python(?:3(?:\.\d+)?)?)(?:\.exe)?$")
INLINE_CODE_FLAGS = frozenset({"-c", "-e", "--eval", "-p", "--print", "-Command", "-"})
MODULE_FLAG = "-m"
SCRIPT_SUFFIX = r"\.(?:sh|bash|py|ps1|mjs|cjs|js)"
SCRIPT_SUFFIX_RE = re.compile(SCRIPT_SUFFIX + "$")
# What a script's text must not name for it to count as no build: every tool that
# resolves rust-toolchain.toml or installs a toolchain. Read over the whole text,
# comments and strings included, so a mention counts as a call until a reason below
# says otherwise.
SCRIPT_TOOL_RE = re.compile(
    r"(?<![\w./-])(?:cargo|rustc|rustdoc|rustup|maturin|wasm-pack|cross)(?:\.exe)?(?![\w/-])(?!\.(?:toml|lock)\b)"
)
# Another script a script's text names, followed whether it calls it or only mentions it.
SCRIPT_NAME_RE = re.compile(r"[\w./-]*[\w-]" + SCRIPT_SUFFIX + r"(?![\w.-])")
# Scripts whose text names a tool (or a script that does) without running it. Each
# entry is checked to still name one, so it cannot outlive its reason.
SCRIPTS_THAT_ONLY_NAME_A_TOOL = {
    "scripts/check-version-sync.py": "runs no command: it names the cargo registry in a docstring and reads "
                                     "version pins out of the dx-timing scenarios it names",
    "scripts/check-promise-contract.py": "names `cargo bench` as a documentary claim it skips; the executable "
                                         "claims it runs name no tool, which a test holds",
    "scripts/bench-memory-extraction.py": "calls `rustc --version` only in `report --from-dir`, which no workflow "
                                          "runs, and `cross` is a variable",
    "scripts/check-ai-attribution.py": "runs git only; it names the refusal-vector test in its help text",
    "scripts/check-doc-contract.sh": "runs grep only; it names run-production-gates.sh in its header comment",
    "scripts/check-mcp-doc-contract.py": "runs `git ls-files` only; the scripts it names are files it reads",
    "integrations/agent-hooks/test/hooks.test.sh": "names cargo in comments and in a hook payload it feeds as data",
    "integrations/agent-hooks/claude-code/hooks/lib/freshness.sh": "names `cargo install` in a comment saying the hook "
                                                                   "never runs it",
}


def _unquote(value: str) -> str:
    value = re.sub(r"\s+#.*$", "", value).strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        value = value[1:-1]
    return value


def toolchain_file() -> tuple[str, frozenset[str]]:
    table = tomllib.loads(TOOLCHAIN_FILE.read_text(encoding="utf-8"))["toolchain"]
    return str(table["channel"]), frozenset(table.get("components", ()))


# --- YAML, read by PyYAML -----------------------------------------------------


def _load(text: str) -> tuple[yaml.Node | None, list[str]]:
    try:
        return yaml.compose(text, Loader=yaml.SafeLoader), []
    except yaml.YAMLError as error:
        return None, [f"the workflow is not valid YAML: {' '.join(str(error).split())}"]


def _pairs(node: yaml.Node | None) -> dict[str, tuple[yaml.Node, yaml.Node]]:
    """A mapping node's scalar keys to (key, value) nodes, merge keys resolved; {} for anything else."""
    if not isinstance(node, yaml.MappingNode):
        return {}
    merged: dict[str, tuple[yaml.Node, yaml.Node]] = {}
    own: dict[str, tuple[yaml.Node, yaml.Node]] = {}
    for key, value in node.value:
        if key.tag == MERGE_TAG:
            for source in value.value if isinstance(value, yaml.SequenceNode) else [value]:
                for name, pair in _pairs(source).items():
                    merged.setdefault(name, pair)
        elif isinstance(key, yaml.ScalarNode):
            own[key.value] = (key, value)
    return {**merged, **own}


def _value(pairs: dict[str, tuple[yaml.Node, yaml.Node]], name: str) -> yaml.Node | None:
    pair = pairs.get(name)
    return pair[1] if pair else None


def _scalar(node: yaml.Node | None) -> str | None:
    return node.value if isinstance(node, yaml.ScalarNode) else None


def _string(pairs: dict, name: str, where: str, problems: list[str]) -> str:
    node = _value(pairs, name)
    if node is None:
        return ""
    text = _scalar(node)
    if text is None:
        problems.append(f"{where} writes `{name}` as something other than a string, which the guard cannot read")
        return ""
    return text


def _strings(pairs: dict, name: str, where: str, problems: list[str]) -> dict[str, str]:
    """`env` or `with`: a mapping of strings; anything else is a problem."""
    node = _value(pairs, name)
    if node is None:
        return {}
    if not isinstance(node, yaml.MappingNode):
        problems.append(f"{where} writes `{name}` as something other than a mapping, which the guard cannot read")
        return {}
    return {key: text for key, (_, value) in _pairs(node).items() if (text := _scalar(value)) is not None}


def _directory(pairs: dict, where: str, problems: list[str]) -> str:
    """`defaults.run.working-directory` of a workflow or a job."""
    return _string(_pairs(_value(_pairs(_value(pairs, "defaults")), "run")), "working-directory", where, problems)


@dataclass
class Step:
    uses: str = ""
    run: str = ""
    run_line: int = 0  # 0-based line where the run text starts
    inputs: dict[str, str] = field(default_factory=dict)
    env: dict[str, str] = field(default_factory=dict)
    directory: str = ""
    condition: str = UNCONDITIONAL  # the step's `if:`, normalised
    first: int = 0  # 0-based line of the step's first code line
    last: int = 0  # bound: the next step's first line, or the job's end


@dataclass
class Job:
    env: dict[str, str]
    steps: list[Step]
    directory: str = ""
    first: int = 0
    last: int = 0
    problems: list[str] = field(default_factory=list)


@dataclass
class Workflow:
    env: dict[str, str]
    jobs: dict[str, Job]
    directory: str = ""
    problems: list[str] = field(default_factory=list)
    raw: list[str] = field(default_factory=list)
    root: yaml.Node | None = None


def _step(node: yaml.Node, where: str, problems: list[str]) -> Step:
    pairs = _pairs(node)
    if not isinstance(node, yaml.MappingNode):
        problems.append(f"{where} is not a mapping, which the guard cannot read")
    run = _value(pairs, "run")
    return Step(
        uses=_string(pairs, "uses", where, problems),
        run=_string(pairs, "run", where, problems),
        run_line=run.start_mark.line + (run.style in ("|", ">")) if run is not None else node.start_mark.line,
        inputs=_strings(pairs, "with", where, problems),
        env=_strings(pairs, "env", where, problems),
        directory=_string(pairs, "working-directory", where, problems),
        condition=_condition(_string(pairs, "if", where, problems)),
        first=node.start_mark.line,
    )


def _condition(text: str) -> str:
    """An `if:` as written, `${{ }}` and spacing dropped; `success()` is no condition."""
    text = " ".join(text.split())
    text = match.group(1) if (match := EXPRESSION_RE.match(text)) else text
    return UNCONDITIONAL if text == "success()" else text


def _job(name: str, key: yaml.Node, node: yaml.Node, end: int) -> Job:
    problems: list[str] = []
    pairs = _pairs(node)
    steps_node = _value(pairs, "steps")
    if steps_node is not None and not isinstance(steps_node, yaml.SequenceNode):
        problems.append(f"{name}: `steps` is not a list, which the guard cannot read")
    items = steps_node.value if isinstance(steps_node, yaml.SequenceNode) else []
    steps = [_step(item, f"{name}: step {index}", problems) for index, item in enumerate(items)]
    for step, bound in zip(steps, [step.first for step in steps[1:]] + [end]):
        step.last = bound
    return Job(env=_strings(pairs, "env", f"{name}: job", problems), steps=steps,
               directory=_directory(pairs, f"{name}: job", problems), first=key.start_mark.line, last=end, problems=problems)


def parse(text: str) -> Workflow:
    root, problems = _load(text)
    raw = text.splitlines()
    top = _pairs(root)
    if root is not None and not isinstance(root, yaml.MappingNode):
        problems.append("the workflow is not a mapping, which the guard cannot read")
    workflow = Workflow(env=_strings(top, "env", "workflow", problems), jobs={},
                        directory=_directory(top, "workflow", problems), problems=problems, raw=raw, root=root)
    jobs = list(_pairs(_value(top, "jobs")).items())
    if not jobs and _value(top, "runs") is not None:  # a composite action: one job, `runs.steps`, no checkout of its own
        jobs = [("runs", top["runs"])]
    heads = [key.start_mark.line for _, (key, _) in jobs] + [len(raw)]
    for index, (name, (key, value)) in enumerate(jobs):
        workflow.jobs[name] = _job(name, key, value, heads[index + 1])
    return workflow


def _children(node: yaml.Node, owner: yaml.Node | None) -> list[tuple[yaml.Node, yaml.Node | None]]:
    if isinstance(node, yaml.MappingNode):
        return [(value, key) for key, value in node.value]
    if isinstance(node, yaml.SequenceNode):
        return [(item, owner) for item in node.value]
    return []


def _nodes(root: yaml.Node | None) -> list[tuple[yaml.Node, yaml.Node | None]]:
    """Every value node of the document with the key it sits under, each once (an alias shares its node)."""
    seen, stack, out = set(), [(root, None)], []
    while stack:
        node, owner = stack.pop()
        if node is None or id(node) in seen:
            continue
        seen.add(id(node))
        out.append((node, owner))
        stack += _children(node, owner)
    return out


def _mapping_pairs(root: yaml.Node | None) -> list[tuple[yaml.Node, yaml.Node, yaml.Node | None]]:
    """Every (key, value, parent key) of the document, in order."""
    pairs = [(key, value, owner) for node, owner in _nodes(root) if isinstance(node, yaml.MappingNode) for key, value in node.value]
    return sorted(pairs, key=lambda triple: triple[0].start_mark.index)


def _value_scalars(root: yaml.Node | None) -> list[tuple[yaml.ScalarNode, yaml.Node | None]]:
    """Every scalar value or list item of the document in order, with the key it sits under."""
    return sorted(((node, owner) for node, owner in _nodes(root) if isinstance(node, yaml.ScalarNode)),
                  key=lambda pair: pair[0].start_mark.index)


def _scalar_lines(node: yaml.ScalarNode) -> list[tuple[int, str]]:
    """(0-based line, text) of a scalar's lines, shell comment lines left out."""
    first = node.start_mark.line + (node.style in ("|", ">"))
    return [(first + i, text) for i, text in enumerate(node.value.splitlines()) if not text.lstrip().startswith("#")]


# --- nightly reasons ------------------------------------------------------------


def _needs(text: str) -> set[str]:
    """What a text says needs nightly: `-Z` flags, miri, cargo fuzz, cargo careful."""
    return {f"-z{m.group(1).lower()}" if m.group(1) else m.group(2).lower() for m in NIGHTLY_NEED_RE.finditer(text)}


def _job_needs(job: Job, workflow: Workflow) -> set[str]:
    """What a job runs on nightly, read from its `run:` values and `*FLAGS` variables only."""
    runs = [line for step in job.steps for line in step.run.splitlines() if not line.lstrip().startswith("#")]
    envs = (workflow.env, job.env, *(step.env for step in job.steps))
    flags = [value for env in envs for key, value in env.items() if key.endswith("FLAGS")]
    return _needs("\n".join(runs + flags))


class _Reasons:
    """The comment block right above each code line of a text."""

    def __init__(self, raw: list[str]) -> None:
        self.above: dict[int, list[str]] = {}
        pending: list[str] = []
        for index, line in enumerate(raw):
            if COMMENT_RE.match(line):
                pending.append(line)
                continue
            if line.strip() and pending:
                self.above[index] = pending
            pending = []

    def given(self, lines, needs: set[str]) -> bool:
        """A block above one of `lines` holds a `# nightly:` line and names something in `needs`."""
        blocks = [self.above.get(line, []) for line in lines]
        return any(any(NIGHTLY_REASON_RE.match(c) for c in block) and bool(_needs("\n".join(block)) & needs) for block in blocks)


def _pin_context(workflow: Workflow, line: int, parent: yaml.Node | None) -> tuple[list[int], set[str]]:
    """The code lines whose comments may explain a pin at `line`, and what its job runs on nightly."""
    for job in workflow.jobs.values():
        for step in job.steps:
            if step.first <= line < step.last:
                return list(range(step.first, step.last)), _job_needs(job, workflow)
        if job.first <= line < job.last:
            return _own_lines(line, parent), _job_needs(job, workflow)
    every = [_job_needs(job, workflow) for job in workflow.jobs.values()]
    return _own_lines(line, parent), set().union(*every)


def _own_lines(line: int, parent: yaml.Node | None) -> list[int]:
    return [line] + ([parent.start_mark.line] if parent is not None else [])


def _pins(text: str) -> list[tuple[str, bool]]:
    """(toolchain, is a pin site that must say why) for each toolchain a shell text names."""
    pins = [(match.group(1), False) for match in PLUS_RE.finditer(text)]
    pins += [(name, True) for match in RUSTUP_RE.finditer(text) if (name := _named(match.group(2)))]
    return pins + [(_unquote(match.group(1)), True) for match in ASSIGN_RE.finditer(text)]


def _pin_findings(line: int, pins: list[tuple[str, bool]], explained) -> list[str]:
    found = [f"line {line + 1} names toolchain {name!r}" for name, _ in pins if name != ALLOWED]
    if any(site for name, site in pins if name == ALLOWED) and not explained():
        found.append(f"line {line + 1} {PINS_UNEXPLAINED}")
    return found


def _key_findings(workflow: Workflow, reasons: _Reasons) -> list[str]:
    """Every `*toolchain` key and RUST_VERSION key of the document, at any depth."""
    found = []
    for key, value, parent in _mapping_pairs(workflow.root):
        line = key.start_mark.line
        if not isinstance(key, yaml.ScalarNode):
            found.append(f"line {line + 1} has a key the guard cannot read")
        elif key.value == "RUST_VERSION":
            found.append(f"line {line + 1} copies the toolchain version: 'RUST_VERSION'")
        elif TOOLCHAIN_KEY_RE.match(key.value):
            found += _toolchain_value(workflow, reasons, key, value, parent)
    return found


def _toolchain_value(workflow: Workflow, reasons: _Reasons, key: yaml.Node, value: yaml.Node, parent) -> list[str]:
    line, name = key.start_mark.line, _scalar(value)
    if name is None:
        return [f"line {line + 1} names a toolchain the guard cannot read"]
    lines, needs = _pin_context(workflow, line, parent)
    return _pin_findings(line, [(name, True)], lambda: reasons.given(lines, needs))


def _scalar_findings(workflow: Workflow, reasons: _Reasons, channel: str) -> list[str]:
    """Copies of the version in any scalar; toolchains named in any scalar but a `run:` (the walk reads those)."""
    channel_re = re.compile(r"(?<![\w.])" + re.escape(channel) + r"(?![\w.])")
    found = []
    for node, owner in _value_scalars(workflow.root):
        in_run = owner is not None and _scalar(owner) == "run"
        for line, text in _scalar_lines(node):
            if RUST_VERSION_RE.search(text) or channel_re.search(text):
                found.append(f"line {line + 1} copies the toolchain version: {text.strip()!r}")
            if not in_run:
                lines, needs = _pin_context(workflow, line, owner)
                found += _pin_findings(line, _pins(text), lambda: reasons.given(lines, needs))
    return found


# --- shell ------------------------------------------------------------------------


def _split(line: str) -> list[str]:
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return list(lexer)


def _read_line(lines: list[str], index: int) -> tuple[int, list[str] | None]:
    """One shell line from `index`: `\\` continuations and open quotes joined; its last line and words."""
    text = lines[index]
    while True:
        if text.rstrip().endswith("\\") and index + 1 < len(lines):
            index += 1
            text = text.rstrip()[:-1] + " " + lines[index]
            continue
        try:
            return index, _split(text)
        except ValueError:
            if index + 1 >= len(lines):
                return index, None
            index += 1
            text += "\n" + lines[index]


def _heredoc(words: list[str]) -> str | None:
    """The terminator of a heredoc the words open, if any."""
    for index, word in enumerate(words[:-1]):
        if word == "<<":
            return words[index + 1].lstrip("-")
    return None


def _past(lines: list[str], index: int, terminator: str) -> int:
    while index < len(lines) and lines[index].strip() != terminator:
        index += 1
    return index


def _logical_lines(script: str) -> tuple[list[tuple[int, list[str]]], list[str]]:
    """(offset, words) of each shell line, heredoc bodies skipped as data; what could not be read."""
    lines, out, problems, index = script.splitlines(), [], [], 0
    while index < len(lines):
        start = index
        index, words = _read_line(lines, index)
        if words is None:
            return out, [f"has a `run` the guard cannot tokenize from its line {start + 1}"]
        out.append((start, words))
        terminator = _heredoc(words)
        if terminator is not None:
            index = _past(lines, index + 1, terminator)
            problems += [f"has a heredoc without its `{terminator}` line"] if index >= len(lines) else []
        index += 1
    return out, problems


def _segments(words: list[str]) -> list[list[str]]:
    """The commands of a shell line: words split at `&&`, `||`, `;`, `|`, `&` and parentheses."""
    out, current = [], []
    for word in words:
        if word in SEPARATORS:
            out += [current] if current else []
            current = []
        else:
            current.append(word)
    return out + ([current] if current else [])


def _command(segment: list[str]) -> list[str]:
    index = 0
    while index < len(segment) and segment[index] in SHELL_KEYWORDS:
        index += 1
    return segment[index:]


def _prefix_toolchain(command: list[str]) -> str | None:
    """RUSTUP_TOOLCHAIN set for this command only, by a leading assignment word."""
    for word in command:
        if not ASSIGNMENT_WORD_RE.match(word):
            return None
        if word.startswith("RUSTUP_TOOLCHAIN="):
            return word.partition("=")[2]
    return None


def _assigned(words: list[str]) -> str | None:
    return next((word.partition("=")[2] for word in words if word.startswith("RUSTUP_TOOLCHAIN=")), None)


@dataclass
class _Shell:
    """Where a shell is, and what it exported, within one step or RUN."""

    directory: str
    stack: list[str] = field(default_factory=list)
    exported: str | None = None


def _target(command: list[str]) -> str:
    """Where `cd` or `pushd` goes: its first word that is not an option, else home."""
    targets = [word for word in command[1:] if not word.startswith("-")]
    return targets[0] if targets else "~"


def _change_directory(command: list[str], shell: _Shell, join) -> str | None:
    """Apply `cd`, `pushd` or `popd`; return a target the guard cannot resolve, if any."""
    if command[0] == "popd":
        shell.directory = shell.stack.pop() if shell.stack else UNKNOWN_DIR
        return None
    target = _target(command)
    if command[0] == "pushd":
        shell.stack.append(shell.directory)
    unresolvable = "$" in target or "`" in target
    shell.directory = UNKNOWN_DIR if unresolvable else join(shell.directory, target)
    return target if unresolvable else None


def _named(args: str) -> str | None:
    """The toolchain a rustup command line names, or None when it names none."""
    skip = False
    for token in args.split():
        if skip:
            skip = False
        elif token in VALUE_FLAGS:
            skip = True
        elif not token.startswith("-"):
            return token
    return None


def _verb(match: re.Match[str]) -> str:
    return " ".join(match.group(1).split())


def _installs(verb: str, named: str | None) -> bool:
    """`rustup toolchain install|add`, `rustup install`, or `rustup update <toolchain>`."""
    return verb in INSTALL_VERBS or (verb == "update" and named is not None)


def _installs_from_the_file(run: str) -> bool:
    return any(_verb(m) in INSTALL_VERBS and _named(m.group(2)) is None for m in RUSTUP_RE.finditer(run))


def _reach(needed: str | None) -> str:
    """What is wrong with a call that needs `needed` and finds it not installed."""
    if needed in REACH:
        return REACH[needed]
    if needed.startswith(FOREIGN):
        return f"reaches cargo in a checkout of {needed.removeprefix(FOREIGN)}, not of this repository"
    if needed.startswith("file:"):
        copy = needed.removeprefix("file:")
        prefix = "" if copy == "." or copy.startswith("/") else f"{copy}/"
        return f"reaches cargo before {prefix}rust-toolchain.toml's toolchain is installed"
    return f"reaches cargo on {needed!r} before installing it"


def _norm(directory: str) -> str:
    return posixpath.normpath(directory.strip() or ".")


def _resolve(directory: str) -> str:
    """A workspace-relative directory, or UNKNOWN_DIR when the guard cannot tell where it is."""
    normal = _norm(directory)
    if "${{" in directory or normal.startswith(("/", "$", "~", "..")) or normal == "-":
        return UNKNOWN_DIR
    return normal


def _within(directory: str, copy: str) -> bool:
    return copy == "." or directory == copy or directory.startswith(copy + "/")


def _cd(directory: str, target: str) -> str:
    return UNKNOWN_DIR if directory == UNKNOWN_DIR else _resolve(posixpath.join(directory, target))


# --- scripts: a step that runs one reaches cargo when the script does ------------------


def _script(command: list[str]) -> tuple[str, bool] | None:
    """The file a command runs, and whether the guard must read it; None when it runs no file.

    `bash x.sh`, `python3 -B x.py`, `node x.mjs` or `pwsh -File x.ps1` run a script
    that must be read. So does a path run as the command when it carries a script
    suffix (`./scripts/x.sh`); a suffixless one (`./mcp-publisher`, a binary an
    earlier step fetched) is read only when it is a file of the repository, and so
    is `python -m <module>`, an installed module otherwise. Inline code (`-c`, `-e`,
    `-p`, `-`) and a bare interpreter run no file.
    """
    words = command[:]
    while words and (ASSIGNMENT_WORD_RE.match(words[0]) or words[0] == "env"):
        words.pop(0)
    if not words:
        return None
    if not INTERPRETER_RE.match(posixpath.basename(words[0])):
        return (words[0], SCRIPT_SUFFIX_RE.search(words[0]) is not None) if "/" in words[0] else None
    arguments = iter(words[1:])
    for word in arguments:
        if word in INLINE_CODE_FLAGS:
            return None
        if word == MODULE_FLAG:
            module = next(arguments, "")
            return (module.replace(".", "/") + ".py", False) if module else None
        if not word.startswith("-"):
            return word, True
    return None


def script_builds(root: Path, path: str, exempt: dict[str, str] = SCRIPTS_THAT_ONLY_NAME_A_TOOL, seen: frozenset[str] = frozenset()) -> bool:
    """Whether the repository script at `path` (relative to `root`) names a tool, or a script that does.

    Every script name its text holds is followed where it resolves, against the
    script's directory or the root, whether the script calls it or only mentions
    it; a name that resolves nowhere is no file of the repository.
    """
    if path in exempt or path in seen:
        return False
    text = (root / path).read_text(encoding="utf-8", errors="replace")
    if SCRIPT_TOOL_RE.search(text):
        return True
    for name in dict.fromkeys(SCRIPT_NAME_RE.findall(text)):
        for base in (posixpath.dirname(path), ""):
            candidate = posixpath.normpath(posixpath.join(base, name.lstrip("/")))
            if not candidate.startswith(("..", "/")) and (root / candidate).is_file():
                if script_builds(root, candidate, exempt, seen | {path}):
                    return True
                break
    return False


# --- the walk: what each job installs, and what each call needs ------------------------


class _Walk:
    """One job, step by step: its checkouts, the toolchains it installed, and what each call needs."""

    def __init__(self, name: str, job: Job, workflow: Workflow, reasons: _Reasons, root: Path) -> None:
        self.name, self.job, self.workflow, self.reasons, self.root = name, job, workflow, reasons, root
        self.needs = _job_needs(job, workflow)
        self.copies: dict[str, str] = {}  # checkout path -> repository ("" for this one)
        self.installed: dict[str, set[str]] = {}  # toolchain -> the `if:` of each step that installed it
        self.override: str | None = None  # `rustup override set <toolchain>`
        self.exported: str | None = None  # RUSTUP_TOOLCHAIN written to $GITHUB_ENV
        self.found: list[str] = []

    def file(self, directory: str) -> str:
        """The rust-toolchain.toml a call in `directory` resolves through."""
        if directory == UNKNOWN_DIR:
            return UNREADABLE
        owners = [copy for copy in self.copies if _within(directory, copy)]
        if not owners:
            return OUTSIDE
        copy = max(owners, key=len)
        return f"{FOREIGN}{self.copies[copy]}" if self.copies[copy] else f"file:{copy}"

    def toolchain(self, step: Step, directory: str, line_env: str | None = None) -> str:
        chosen = (
            line_env
            or step.env.get("RUSTUP_TOOLCHAIN")
            or self.exported
            or self.job.env.get("RUSTUP_TOOLCHAIN")
            or self.workflow.env.get("RUSTUP_TOOLCHAIN")
        )
        return chosen or self.override or self.file(directory)

    def require(self, where: str, step: Step, needed: str) -> None:
        """A call of `step` needs `needed`, installed under `always()` or the call's own `if:`.

        An install with no `if:` also covers a call that runs only while every earlier
        step succeeded.
        """
        conditions = self.installed.get(needed, set())
        covering = {ALWAYS, step.condition} | (set() if RUNS_AFTER_FAILURE_RE.search(step.condition) else {UNCONDITIONAL})
        if conditions & covering:
            return
        only = "".join(f" (installed only under `if: {c}`)" for c in sorted(conditions))
        self.found.append(f"{where} {_reach(needed)}{only}")

    def script(self, where: str, directory: str, command: list[str]) -> bool:
        """Whether the command runs a repository script that reaches cargo; a script it cannot read is a finding."""
        named = _script(command)
        if named is None:
            return False
        script, must_read = named
        path, problem = self._script_path(directory, script)
        if path is not None and (self.root / path).is_file():
            return script_builds(self.root, path)
        if must_read:
            self.found.append(f"{where} runs {script!r}, which the guard cannot read: {problem or 'no such file in the repository'}")
        return False

    def _script_path(self, directory: str, script: str) -> tuple[str | None, str | None]:
        """The script's path inside this repository, or why the guard cannot tell it."""
        if "$" in script or "`" in script or directory == UNKNOWN_DIR:
            return None, "its path is not a literal the guard can resolve"
        target = _cd(directory, script)
        if target == UNKNOWN_DIR:
            return None, "its path leaves the workspace"
        owner = self.file(target)
        if owner.startswith(FOREIGN):
            return None, f"it is a file of {owner.removeprefix(FOREIGN)}, not of this repository"
        if not owner.startswith("file:"):
            return None, "it is outside every checkout of this repository"
        return posixpath.relpath(target, owner.removeprefix("file:")), None

    def visit(self, index: int, step: Step) -> None:
        where = f"{self.name}: step {index}"
        self._visit_uses(where, step)
        shell = _Shell(directory=_resolve(step.directory or self.job.directory or self.workflow.directory))
        lines, problems = _logical_lines(step.run)
        self.found += [f"{where} {problem}" for problem in problems]
        for offset, words in lines:
            for segment in _segments(words):
                self._visit_segment(where, step, step.run_line + offset, _command(segment), shell)

    def _visit_uses(self, where: str, step: Step) -> None:
        if step.uses.startswith(CHECKOUT_ACTION):
            self._checkout(step)
        elif step.uses.startswith(TOOLCHAIN_ACTION):
            self._toolchain_action(where, step)
        elif step.uses.startswith(RUST_CACHE_ACTION):
            self.require(where, step, self.toolchain(step, "."))
        elif step.uses.startswith(CACHE_ACTION) and TOOLCHAIN_PATHS_RE.search(step.inputs.get("path", "")):
            self.found.append(f"{where} caches a toolchain path: {step.inputs['path']!r}")

    def _checkout(self, step: Step) -> None:
        """A checkout of this repository carries its rust-toolchain.toml; one of another repository does not."""
        repository = step.inputs.get("repository", "")
        self.copies[_resolve(step.inputs.get("path", ""))] = "" if repository in THIS_REPOSITORY else repository

    def _toolchain_action(self, where: str, step: Step) -> None:
        if "toolchain" in step.inputs:
            self.installed.setdefault(step.inputs["toolchain"], set()).add(step.condition)
        else:
            self.found.append(f"{where} installs the action's ref, not rust-toolchain.toml's toolchain")

    def _visit_segment(self, where: str, step: Step, line: int, command: list[str], shell: _Shell) -> None:
        if command and command[0] in DIRECTORY_COMMANDS:
            target = _change_directory(command, shell, _cd)
            self.found += [f"{where} changes to a directory the guard cannot resolve: {target!r}"] if target else []
            return
        text = " ".join(command)
        explained = range(step.first, step.last)
        self.found += _pin_findings(line, _pins(text), lambda: self.reasons.given(explained, self.needs))
        line_env = self._rustup(where, step, text, shell.directory) or self._environment(command, text, shell) or shell.exported
        for match in PLUS_RE.finditer(text):
            self.require(where, step, match.group(1))
        if self.script(where, shell.directory, command) or BUILD_RE.search(text):
            self.require(where, step, self.toolchain(step, shell.directory, line_env))

    def _environment(self, command: list[str], text: str, shell: _Shell) -> str | None:
        """RUSTUP_TOOLCHAIN set for this command; record what `export` and $GITHUB_ENV set for later."""
        if "GITHUB_ENV" in text:
            self.exported = _assigned(command) or self.exported
        elif command[:1] == ["export"]:
            shell.exported = _assigned(command) or shell.exported
        return _prefix_toolchain(command)

    def _rustup(self, where: str, step: Step, text: str, directory: str) -> str | None:
        """Record what the command installs or overrides; return the toolchain `rustup run` names."""
        ran = None
        for match in RUSTUP_RE.finditer(text):
            verb, named = _verb(match), _named(match.group(2))
            if _installs(verb, named):
                self._install(where, step, named or self.file(directory))
            elif verb in OVERRIDE_VERBS and named:
                self.override = named
            elif verb == "run":
                ran = named
        return ran

    def _install(self, where: str, step: Step, target: str) -> None:
        if target in INSTALL_ELSEWHERE:
            self.found.append(f"{where} {INSTALL_ELSEWHERE[target]}")
        elif target.startswith(FOREIGN):
            self.found.append(f"{where} installs in a checkout of {target.removeprefix(FOREIGN)}, not from this repository's rust-toolchain.toml")
        else:
            self.installed.setdefault(target, set()).add(step.condition)


def _job_findings(name: str, job: Job, workflow: Workflow, reasons: _Reasons, root: Path) -> list[str]:
    walk = _Walk(name, job, workflow, reasons, root)
    for index, step in enumerate(job.steps):
        walk.visit(index, step)
    return walk.found


def findings(text: str, channel: str, root: Path = REPO_ROOT) -> list[str]:
    """What is wrong with a workflow whose repository scripts live under `root`."""
    workflow = parse(text)
    reasons = _Reasons(workflow.raw)
    found = workflow.problems + _key_findings(workflow, reasons) + _scalar_findings(workflow, reasons, channel)
    for name, job in workflow.jobs.items():
        found += job.problems + _job_findings(name, job, workflow, reasons, root)
    return found


def auto_install_findings(text: str) -> list[str]:
    """Where a workflow or composite action lets rustup install a toolchain on its own."""
    workflow = parse(text)
    found = list(workflow.problems)
    composite = "jobs" not in _pairs(workflow.root)
    if not composite and workflow.env.get(AUTO_INSTALL) != AUTO_INSTALL_OFF:
        found.append(f'the workflow does not set {AUTO_INSTALL}: "{AUTO_INSTALL_OFF}" in its top-level env')
    allowed = {id(key) for key, _ in [_pairs(_value(_pairs(workflow.root), "env")).get(AUTO_INSTALL, (None, None))]}
    if composite:
        for name, job in workflow.jobs.items():
            for index, step in enumerate(job.steps):
                if step.env.get(AUTO_INSTALL) != AUTO_INSTALL_OFF:
                    found.append(f'{name}: step {index} of a composite action does not set {AUTO_INSTALL}: "{AUTO_INSTALL_OFF}"')
        steps = _value(_pairs(_value(_pairs(workflow.root), "runs")), "steps")
        items = steps.value if isinstance(steps, yaml.SequenceNode) else []
        allowed = {id(pair[0]) for item in items if (pair := _pairs(_value(_pairs(item), "env")).get(AUTO_INSTALL))}
    # Past the one key that turns it off, any mention may set, unset or re-enable it
    # (`env -u`, `unset`, `Remove-Item Env:`, a `shell:` wrapper, `$GITHUB_ENV`): no form is parsed.
    scalars = [key for key, _, _ in _mapping_pairs(workflow.root)] + [node for node, _ in _value_scalars(workflow.root)]
    for node in scalars:
        if isinstance(node, yaml.ScalarNode) and id(node) not in allowed and AUTO_INSTALL_MENTION_RE.search(node.value):
            found.append(f"line {node.start_mark.line + 1} {AUTO_INSTALL_PAST_TOP_LEVEL}: {node.value.strip()[:80]!r}")
    return found


# --- Dockerfiles, which have no parser at hand: fail closed -------------------------


@dataclass
class _Stage:
    """One Dockerfile stage: where it builds, and what it copied and installed so far."""

    workdir: str = "/"
    toolchain_dirs: set[str] = field(default_factory=set)  # where rust-toolchain.toml landed
    repository: bool = False  # the repository's sources are in the stage
    default: str | None = None  # `rustup default <toolchain>`
    env: str | None = None  # ENV RUSTUP_TOOLCHAIN
    installed: set[str] = field(default_factory=set)


@dataclass
class _Dockerfile:
    """A Dockerfile being read: its build arguments, and the stage so far."""

    args: dict[str, str | None] = field(default_factory=dict)
    stage: _Stage = field(default_factory=_Stage)


def _instruction(lines: list[str], index: int) -> tuple[int, str]:
    """The instruction starting at `index`, `\\` continuations joined, and its last line."""
    body = ""
    while True:
        body += " " + lines[index].strip().removesuffix("\\")
        if not lines[index].rstrip().endswith("\\") or index + 1 >= len(lines):
            return index, body.strip()
        index += 1


def _instructions(text: str) -> list[tuple[int, str, str, bool]]:
    """(0-based line, INSTRUCTION, arguments, opens a heredoc), heredoc bodies skipped."""
    lines, out, index = text.splitlines(), [], 0
    while index < len(lines):
        if lines[index].strip() and not COMMENT_RE.match(lines[index]):
            start = index
            index, body = _instruction(lines, index)
            word, _, args = body.partition(" ")
            heredoc = HEREDOC_RE.search(args)
            if heredoc:
                index = _past(lines, index + 1, heredoc.group(2))
            out.append((start, word.upper(), args, heredoc is not None))
        index += 1
    return out


def _options(args: str) -> tuple[list[str], str]:
    """An instruction's leading `--opt` words, and the rest."""
    options, rest = [], args.lstrip()
    while rest.startswith("--"):
        option, _, rest = rest.partition(" ")
        options.append(option)
        rest = rest.lstrip()
    return options, rest


def _docker_path(base: str, path: str) -> str:
    """`path` resolved against `base` inside the image, or UNKNOWN_DIR when it cannot be."""
    if base == UNKNOWN_DIR or "$" in path or path.startswith("~"):
        return UNKNOWN_DIR
    return posixpath.normpath(posixpath.join(base, path))


def _expand(text: str, args: dict[str, str | None]) -> str:
    """`${NAME}`, `${NAME:-default}` and `$NAME` replaced by build-argument defaults; others left as written."""

    def value(match: re.Match[str]) -> str:
        known = args.get(match.group(1) or match.group(3))
        if known is not None:
            return known
        return match.group(2) if match.group(2) is not None else match.group(0)

    return ARG_REF_RE.sub(value, text)


def _docker_from(where: str, args: str, build_args: dict[str, str | None]) -> list[str]:
    words = _options(args)[1].split()
    image = words[0] if words else ""
    expanded = _expand(image, build_args)
    if "$" in expanded:
        return [f"{where} FROM the guard cannot resolve: {image!r}"]
    match = FROM_VERSION_RE.match(expanded)
    return [f"{where} names a Rust image by version (rust:{match.group(1)}); rust-toolchain.toml owns the version"] if match else []


def _docker_arg(dockerfile: _Dockerfile, where: str, word: str, args: str, heredoc: bool) -> list[str]:
    try:
        words = shlex.split(args)
    except ValueError:
        return [f"{where} ARG the guard cannot tokenize"]
    for arg in words:
        name, sep, default = arg.partition("=")
        dockerfile.args[name] = default if sep else dockerfile.args.get(name)
    return []


def _copy_landing(stage: _Stage, sources: list[str], dest: str) -> str:
    """The directory rust-toolchain.toml lands in, or UNKNOWN_DIR."""
    target = _docker_path(stage.workdir, dest)
    if dest.endswith("/") or dest in (".", "./") or len(sources) > 1 or sources == ["."]:
        return target
    if posixpath.basename(dest) == "rust-toolchain.toml" and target != UNKNOWN_DIR:
        return posixpath.dirname(target)
    return UNKNOWN_DIR  # one file copied to a path that may be a directory or a new name


def _docker_copy(dockerfile: _Dockerfile, where: str, word: str, args: str, heredoc: bool) -> list[str]:
    options, rest = _options(args)
    if any(option.startswith("--from") for option in options):
        return []
    if heredoc:
        return [f"{where} {word} uses a heredoc, which the guard cannot read"]
    if rest.startswith("["):
        return [f"{where} {word} in exec (JSON) form, which the guard cannot read"]
    parts = rest.split()
    sources, dest = parts[:-1], (parts[-1] if parts else ".")
    names = {posixpath.basename(posixpath.normpath(source)) for source in sources}
    stage = dockerfile.stage
    if names & {"rust-toolchain.toml", "."}:
        stage.toolchain_dirs.add(_copy_landing(stage, sources, dest))
    stage.repository |= bool(names & {".", "Cargo.toml", "Cargo.lock", "crates"})
    return []


def _docker_workdir(dockerfile: _Dockerfile, where: str, word: str, args: str, heredoc: bool) -> list[str]:
    dockerfile.stage.workdir = _docker_path(dockerfile.stage.workdir, _unquote(args))
    return []


def _docker_env(dockerfile: _Dockerfile, where: str, word: str, args: str, heredoc: bool) -> list[str]:
    match = ASSIGN_RE.search(args) or re.search(r"\bRUSTUP_TOOLCHAIN\s+([^\s=]+)", args)
    dockerfile.stage.env = _unquote(match.group(1)) if match else dockerfile.stage.env
    return []


def _governing(stage: _Stage, directory: str) -> str | None:
    """The directory whose rust-toolchain.toml governs `directory`: None when none does."""
    if directory == UNKNOWN_DIR or UNKNOWN_DIR in stage.toolchain_dirs:
        return UNKNOWN_DIR
    owners = [owner for owner in stage.toolchain_dirs if directory == owner or directory.startswith(owner.rstrip("/") + "/")]
    return max(owners, key=len, default=None)


def _docker_file_install(stage: _Stage, where: str, owner: str | None) -> list[str]:
    if owner in (None, UNKNOWN_DIR):
        return [f"{where} installs where no rust-toolchain.toml the guard can see governs, so not the file's toolchain"]
    stage.installed.add(f"file:{owner}")
    return []


def _docker_rustup(stage: _Stage, where: str, text: str, owner: str | None) -> list[str]:
    found: list[str] = []
    for match in RUSTUP_RE.finditer(text):
        verb, named = _verb(match), _named(match.group(2))
        if verb == "default" and named:
            stage.default = named
        if named and (verb in INSTALL_VERBS or verb == "default"):
            stage.installed.add(named)
        elif verb in INSTALL_VERBS:
            found += _docker_file_install(stage, where, owner)
    return found


def _docker_plus(stage: _Stage, where: str, text: str) -> list[str]:
    return [f"{where} reaches cargo on {m.group(1)!r} before installing it"
            for m in PLUS_RE.finditer(text) if m.group(1) not in stage.installed]


def _docker_needed(stage: _Stage, owner: str | None) -> str | None:
    if stage.env:
        return stage.env
    if owner == UNKNOWN_DIR:
        return UNREADABLE
    return f"file:{owner}" if owner else stage.default


def _docker_build(stage: _Stage, where: str, text: str, owner: str | None) -> list[str]:
    if not (stage.repository and BUILD_RE.search(text)):
        return []
    needed = _docker_needed(stage, owner)
    return [] if needed in stage.installed else [f"{where} {_reach(needed)}"]


def _touches_rust(text: str) -> bool:
    return any(regex.search(text) for regex in (BUILD_RE, PLUS_RE, RUSTUP_RE))


def _docker_segment(stage: _Stage, where: str, command: list[str], shell: _Shell) -> list[str]:
    if command and command[0] in DIRECTORY_COMMANDS:
        target = _change_directory(command, shell, _docker_path)
        return [f"{where} changes to a directory the guard cannot resolve: {target!r}"] if target else []
    text, owner = " ".join(command), _governing(stage, shell.directory)
    found = _docker_rustup(stage, where, text, owner)
    return found + _docker_plus(stage, where, text) + _docker_build(stage, where, text, owner)


def _docker_run(dockerfile: _Dockerfile, where: str, word: str, args: str, heredoc: bool) -> list[str]:
    rest = _options(args)[1]
    if heredoc:
        return [f"{where} RUN uses a heredoc, which the guard cannot read"]
    if rest.startswith("["):
        return [f"{where} RUN in exec (JSON) form, which the guard cannot read"] if _touches_rust(rest) else []
    try:
        words = _split(rest)
    except ValueError:
        return [f"{where} RUN the guard cannot tokenize"]
    stage, shell, found = dockerfile.stage, _Shell(directory=dockerfile.stage.workdir), []
    for segment in _segments(words):
        found += _docker_segment(stage, where, _command(segment), shell)
    return found


DOCKER_INSTRUCTIONS = {
    "ARG": _docker_arg,
    "COPY": _docker_copy,
    "ADD": _docker_copy,
    "WORKDIR": _docker_workdir,
    "ENV": _docker_env,
    "RUN": _docker_run,
}


def _dockerfile_literals(text: str, channel: str) -> list[str]:
    """Toolchains named, and version copies, on any line but a comment."""
    raw, reasons = text.splitlines(), _Reasons(text.splitlines())
    needs = _needs("\n".join(args for _, word, args, _ in _instructions(text) if word == "RUN" or "FLAGS" in args))
    channel_re = re.compile(r"(?<![\w.])" + re.escape(channel) + r"(?![\w.])")
    found = []
    for index, line in enumerate(raw):
        if not COMMENT_RE.match(line):
            found += _dockerfile_line(index, line, lambda i=index: reasons.given([i], needs), channel_re)
    return found


def _dockerfile_line(index: int, line: str, explained, channel_re: re.Pattern[str]) -> list[str]:
    keyed = DOCKER_ENV_RE.match(line)
    found = _pin_findings(index, _pins(line) + ([(_unquote(keyed.group(1)), True)] if keyed else []), explained)
    if RUST_VERSION_RE.search(line) or channel_re.search(line):
        found.append(f"line {index + 1} copies the toolchain version: {line.strip()!r}")
    return found


def dockerfile_findings(text: str, channel: str) -> list[str]:
    found = _dockerfile_literals(text, channel)
    dockerfile = _Dockerfile()
    for number, word, args, heredoc in _instructions(text):
        where = f"line {number + 1}"
        if word == "FROM":
            dockerfile.stage = _Stage()
            found += _docker_from(where, args, dockerfile.args)
        elif word in DOCKER_INSTRUCTIONS:
            found += DOCKER_INSTRUCTIONS[word](dockerfile, where, word, args, heredoc)
    return found


# --- the tree ----------------------------------------------------------------------


def workflow_files(root: Path = REPO_ROOT) -> list[Path]:
    workflows, actions = root / ".github" / "workflows", root / ".github" / "actions"
    found = [*workflows.glob("*.yml"), *workflows.glob("*.yaml"), *actions.glob("**/action.yml"), *actions.glob("**/action.yaml")]
    return sorted(set(found))


def _tracked(root: Path) -> list[str]:
    listed = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True, check=True).stdout
    return [name for name in listed.decode("utf-8").split("\0") if name]


def dockerfiles(root: Path = REPO_ROOT) -> list[Path]:
    return [root / name for name in _tracked(root) if DOCKERFILE_NAME_RE.search(name)]


def loom_nightly_claims(root: Path = REPO_ROOT) -> list[str]:
    claims = []
    for name in _tracked(root):
        if Path(name).suffix not in DOC_SUFFIXES or name in LOOM_CLAIM_EXEMPT:
            continue
        try:
            text = (root / name).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        claims += [f"{name}:{n}: {line.strip()}" for n, line in enumerate(text.splitlines(), 1)
                   if "loom" in line.lower() and "nightly" in line.lower()]
    return claims


def real_findings() -> dict[str, list[str]]:
    channel, _ = toolchain_file()
    report = {}
    for path in workflow_files():
        text = path.read_text(encoding="utf-8")
        found = findings(text, channel) + auto_install_findings(text)
        if found:
            report[path.relative_to(REPO_ROOT).as_posix()] = found
    return report


INSTALL = "        run: rustup toolchain install --no-self-update --profile minimal\n"
GOOD = """\
jobs:
  py:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      # a comment between two steps
      - name: Install the toolchain rust-toolchain.toml pins
        if: always()
""" + INSTALL + """\
      - name: Cache Cargo registry
        uses: actions/cache@v6
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
          key: k
      - uses: Swatinem/rust-cache@v2
      - name: Build
        run: |
          # PEP 517 build
          pip install ./crates/velesdb-python --force-reinstall
  fuzz:
    runs-on: ubuntu-latest
    env:
      # nightly: cargo-fuzz instruments with -Zsanitizer.
      RUSTUP_TOOLCHAIN: nightly
    steps:
      # nightly: cargo fuzz needs it; installed here, so rustup never installs it mid-step.
      - uses: dtolnay/rust-toolchain@abc # nightly
        with:
          toolchain: nightly
      - uses: Swatinem/rust-cache@v2
      - run: cargo install cargo-fuzz
      - run: cargo +nightly fuzz run target
  miri:
    runs-on: ubuntu-latest
    steps:
      # nightly: Miri ships with nightly only; the override keeps every call on it.
      - run: |
          rustup toolchain install nightly --component miri
          rustup override set nightly
          cargo miri test
"""
FUZZ_ENV = "    env:\n      # nightly: cargo-fuzz instruments with -Zsanitizer.\n      RUSTUP_TOOLCHAIN: nightly\n"
FUZZ_INSTALL = (
    "      # nightly: cargo fuzz needs it; installed here, so rustup never installs it mid-step.\n"
    "      - uses: dtolnay/rust-toolchain@abc # nightly\n        with:\n          toolchain: nightly\n"
)
FUZZ_CACHE = "      - uses: Swatinem/rust-cache@v2\n      - run: cargo install cargo-fuzz\n"
AFTER_INSTALL = "      - name: Cache Cargo registry\n"

# The review's job whose steps sit at 4 spaces, where a 6-space parser sees none.
EARLY = """\
jobs:
  early:
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v7
    - run: cargo build
"""
COMPOSITE = """\
name: setup
runs:
  using: composite
  steps:
    - uses: dtolnay/rust-toolchain@stable
    - run: cargo build
      shell: bash
"""
BASE = """\
jobs:
  regression:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: actions/checkout@v7
        with:
          ref: develop
          path: base
      - name: Install the toolchain rust-toolchain.toml pins
""" + INSTALL + """\
      - name: Baseline
        working-directory: base
        run: cargo bench
"""
BASE_INSTALL = (
    "      - name: Install the toolchain base/rust-toolchain.toml pins\n        working-directory: base\n" + INSTALL
)
DOCKERFILE_WITHOUT_THE_FILE = """\
FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --bin velesdb-server
FROM debian:bookworm-slim
COPY --from=builder /app/target/release/velesdb-server /usr/local/bin/
"""
DOCKERFILE_FROM_THE_FILE = """\
FROM rust:bookworm AS builder
WORKDIR /app
COPY rust-toolchain.toml ./
RUN rustup toolchain install --no-self-update --profile minimal
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --bin velesdb-server
FROM debian:bookworm-slim
COPY --from=builder /app/target/release/velesdb-server /usr/local/bin/
"""
DOCKERFILE_NIGHTLY = """\
FROM rust:slim-bookworm AS builder
WORKDIR /app
# nightly: -Zbuild-std rebuilds std for this benchmark build.
RUN rustup default nightly && \\
    apt-get update
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build -Zbuild-std --release --bin velesdb-server
"""
# Six ways of choosing a toolchain the round-1 guard let through, each as a step.
PIN_SITES = (
    ("      - run: rustup override add stable\n", "stable"),
    ("      - run: rustup toolchain add 1.86\n", "1.86"),
    ("      - run: RUSTUP_TOOLCHAIN=1.86 cargo build\n", "1.86"),
    ("      - run: echo RUSTUP_TOOLCHAIN=stable >> $GITHUB_ENV\n", "stable"),
    ("      - run: ~/.cargo/bin/cargo +1.86 build\n", "1.86"),
    ("      - uses: PyO3/maturin-action@v1\n        with:\n          rust-toolchain: stable\n", "stable"),
)


# Round 3: every way of setting RUSTUP_TOOLCHAIN, each reasoned and never installed.
CHANNELS = {
    "github-env": ("      # nightly: -Zbuild-std rebuilds std\n      - run: echo RUSTUP_TOOLCHAIN=nightly >> $GITHUB_ENV\n"
                   "      - run: cargo build -Zbuild-std\n", 2),
    "export": ("      # nightly: -Zbuild-std rebuilds std\n      - run: |\n          export RUSTUP_TOOLCHAIN=nightly\n"
               "          cargo build -Zbuild-std\n", 1),
    "inline": ("      # nightly: -Zbuild-std rebuilds std\n      - run: RUSTUP_TOOLCHAIN=nightly cargo build -Zbuild-std\n", 1),
    "step-env": ("      # nightly: -Zbuild-std rebuilds std\n      - env:\n          RUSTUP_TOOLCHAIN: nightly\n"
                 "        run: cargo build -Zbuild-std\n", 1),
    "rustup-run": ("      # nightly: -Zbuild-std rebuilds std\n      - run: rustup run nightly cargo build -Zbuild-std\n", 1),
}
DOCKERFILE_ENV_NIGHTLY = """\
FROM rust:slim-bookworm AS builder
WORKDIR /app
# nightly: -Zbuild-std rebuilds std
ENV RUSTUP_TOOLCHAIN=nightly
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build -Zbuild-std --release
"""
ONE_JOB = "jobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n"


def _channel(steps: str) -> list[str]:
    return findings(ONE_JOB + "      - uses: actions/checkout@v7\n" + steps, "1.90")


def _ci_loom_commands() -> list[list[str]]:
    """Each loom `cargo test` quality-deep.yml runs, as tokens, its env first."""
    job = parse((WORKFLOW_DIR / "quality-deep.yml").read_text(encoding="utf-8")).jobs["loom"]
    return [[f"{key}={value}" for key, value in sorted(step.env.items())] + shlex.split(step.run.replace("\\\n", " "))
            for step in job.steps if "--features loom" in step.run]


def _documented_loom_command(path: str) -> list[str]:
    """The loom command a doc tells a reader to run."""
    for line in (REPO_ROOT / path).read_text(encoding="utf-8").splitlines():
        if "cargo test" in line and "--cfg loom" in line:
            return shlex.split(line.split("`")[1] if "`" in line else line.split("//!", 1)[-1])
    raise AssertionError(f"{path} documents no loom command")


CHECKOUT = "      - uses: actions/checkout@v7\n"
INSTALL_STEP = "      - run: rustup toolchain install --no-self-update --profile minimal\n"
DOCKER_HEAD = "FROM rust:bookworm AS builder\nWORKDIR /app\n"
# Round 4, finding 1: a reasoned nightly, set through a form the hand parser read as empty.
YAML_FORMS = {
    "flow mapping on the next line": ("      # nightly: -Zbuild-std rebuilds std\n      - env:\n          { RUSTUP_TOOLCHAIN: nightly }\n"
                                      "        run: cargo build -Zbuild-std\n"),
    "alias": ("        env: &nightly\n          # nightly: -Zbuild-std rebuilds std\n          RUSTUP_TOOLCHAIN: nightly\n"
              "      - env: *nightly\n        run: cargo build -Zbuild-std\n"),
    "quoted key": ("      # nightly: -Zbuild-std rebuilds std\n      - env:\n          \"RUSTUP_TOOLCHAIN\": nightly\n"
                   "        run: cargo build -Zbuild-std\n"),
}
UNEXPLAINED = "pins nightly without a comment `# nightly: <why>`"
# Round 4, finding 4: a reason naming what only a key or a step name says.
NEEDS_CASES = {
    "job keyed miri": ("jobs:\n  miri:\n    runs-on: ubuntu-latest\n    env:\n      # nightly: miri\n      RUSTUP_TOOLCHAIN: nightly\n"
                       "    steps:\n      - uses: dtolnay/rust-toolchain@abc\n        with:\n          # nightly: miri\n"
                       "          toolchain: nightly\n      - run: cargo build\n",
                       [f"line 6 {UNEXPLAINED}", f"line 11 {UNEXPLAINED}"]),
    "step named miri": ("jobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      # nightly: miri\n      - name: Run miri\n"
                        "        uses: dtolnay/rust-toolchain@abc\n        with:\n          toolchain: nightly\n"
                        "      - run: cargo +nightly test\n",
                        [f"line 9 {UNEXPLAINED}"]),
}
# Round 4, finding 5: after a `cd` the guard cannot resolve, no file governs the call.
CD_FORMS = {
    'cd "/tmp"': ('      - run: |\n          cd "/tmp"\n          cargo build\n', []),
    "&& cd /tmp": ("      - run: true && cd /tmp && cargo build\n", []),
    "pushd /tmp": ("      - run: |\n          pushd /tmp\n          cargo build\n", []),
    "cd /tmp;cargo build": ("      - run: cd /tmp;cargo build\n", []),
    'cd "$HOME"': ('      - run: |\n          cd "$HOME"\n          cargo build\n',
                   ["j: step 2 changes to a directory the guard cannot resolve: '$HOME'"]),
}
# A documented loom command: a line that starts, past a comment marker or a
# `Run with:` label, with optional VAR=value words and then `cargo test`.
DOC_COMMAND_RE = re.compile(r"^\s*(?://[!/]?\s*|##\s+Run with:\s*`)?((?:[A-Z_]+=(?:\"[^\"]*\"|'[^']*'|\S+)\s+)*cargo test\b.*)$")
PYYAML_RE = re.compile(r"pip\s+install\b[^\n]*\bpyyaml==([\w.]+)", re.IGNORECASE)
RUNS_THIS_GUARD_RE = re.compile(r"test_ci_toolchain_pin|unittest\s+discover\s+-s\s+scripts/tests")


def _documented_loom_commands() -> list[tuple[str, int, list[str]]]:
    """Every loom `cargo test` a tracked doc tells a reader to run, as tokens."""
    found = []
    for name in _tracked(REPO_ROOT):
        if Path(name).suffix not in (".md", ".rs", ".toml") or name.startswith("scripts/tests/"):
            continue
        for number, line in enumerate((REPO_ROOT / name).read_text(encoding="utf-8").splitlines(), 1):
            match = DOC_COMMAND_RE.match(line)
            if match and "loom" in match.group(1):
                found.append((name, number, shlex.split(match.group(1).split("`")[0])))
    return found


def _variant(old: str, new: str, text: str = GOOD) -> list[str]:
    if old not in text:
        raise ValueError(f"not in the reference workflow: {old!r}")
    return findings(text.replace(old, new), "1.90")


def _add_step(snippet: str) -> list[str]:
    return _variant(AFTER_INSTALL, snippet + AFTER_INSTALL)


def _has(found: list[str], fragment: str) -> bool:
    return any(fragment in finding for finding in found)


def _items_under_steps(lines: list[str]) -> int:
    """The `- ` items of the first `steps:` block in `lines`, at whatever indentation."""
    count, item_depth, inside = 0, None, False
    for line in lines:
        depth = len(line) - len(line.lstrip(" "))
        if not inside:
            inside = re.match(r"^\s*steps:\s*$", line) is not None
            continue
        if item_depth is None:
            item_depth = depth
        if depth < item_depth or (depth == item_depth and not line.lstrip().startswith("- ")):
            break
        count += depth == item_depth
    return count


def _job_heads(lines: list[str]) -> list[int]:
    """Where each job's key sits, two spaces in, after `jobs:`."""
    start = lines.index("jobs:")
    return [i for i, line in enumerate(lines) if i > start and re.match(r"^  [A-Za-z0-9_-]+:\s*$", line)]


def _written_steps(text: str) -> dict[str, int]:
    """Each job's `- ` items under `steps:`, counted without the guard's parser."""
    lines = [line for line in text.splitlines() if line.strip() and not COMMENT_RE.match(line)]
    if "jobs:" not in lines:
        return {"runs": _items_under_steps(lines)}
    heads = _job_heads(lines)
    ends = heads[1:] + [len(lines)]
    return {lines[h].strip()[:-1]: _items_under_steps(lines[h:e]) for h, e in zip(heads, ends)}


class FindingsTests(unittest.TestCase):
    """The checker itself: silent on the right shape, loud on each wrong one."""

    def test_the_right_shape_is_silent(self) -> None:
        self.assertEqual([], findings(GOOD, "1.90"))

    def test_the_parser_sees_every_step_and_the_job_env(self) -> None:
        workflow = parse(GOOD)
        self.assertEqual(["py", "fuzz", "miri"], list(workflow.jobs))
        self.assertEqual(5, len(workflow.jobs["py"].steps))
        self.assertEqual("nightly", workflow.jobs["fuzz"].env["RUSTUP_TOOLCHAIN"])
        self.assertIn("pip install ./crates/velesdb-python", workflow.jobs["py"].steps[4].run)

    def test_the_failing_shape_is_refused(self) -> None:
        # What Python Integrations shipped: `stable` from the action's ref, then pip.
        found = _variant(
            "      - name: Install the toolchain rust-toolchain.toml pins\n        if: always()\n" + INSTALL,
            "      - uses: dtolnay/rust-toolchain@stable\n",
        )
        self.assertTrue(_has(found, "py: step 1 installs the action's ref"), found)
        self.assertTrue(_has(found, "py: step 3 reaches cargo"), found)
        self.assertTrue(_has(found, "py: step 4 reaches cargo"), found)

    def test_a_version_literal_is_refused(self) -> None:
        found = _variant(INSTALL, '        uses: dtolnay/rust-toolchain@stable\n        with:\n          toolchain: "1.90"\n')
        self.assertTrue(_has(found, "names toolchain '1.90'"), found)
        self.assertTrue(_has(found, "copies the toolchain version"), found)

    def test_a_rust_version_copy_is_refused(self) -> None:
        found = findings('env:\n  RUST_VERSION: "1.90"\n' + GOOD, "1.90")
        self.assertTrue(_has(found, "copies the toolchain version"), found)

    # Independent check (a): a cargo resolver setting is not a RUST_VERSION copy.
    def test_only_a_rust_version_key_or_reference_is_a_copy(self) -> None:
        resolver = "env:\n  CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS: fallback\n"
        self.assertEqual([], findings(resolver + GOOD, "1.90"))
        for copy in ('  RUST_VERSION: "1.89"', "  X: ${{ env.RUST_VERSION }}", "  X: $RUST_VERSION", "  X: ${RUST_VERSION}"):
            with self.subTest(copy=copy):
                self.assertTrue(_has(findings("env:\n" + copy + "\n" + GOOD, "1.90"), "copies the toolchain version"))

    def test_a_named_rustup_install_is_refused(self) -> None:
        found = _variant("install --no-self-update", "install 1.86 --no-self-update")
        self.assertTrue(_has(found, "names toolchain '1.86'"), found)
        self.assertTrue(_has(found, "py: step 3 reaches cargo"), found)

    def test_a_cargo_plus_version_is_refused(self) -> None:
        found = _variant("cargo +nightly fuzz", "cargo +1.86 fuzz")
        self.assertTrue(_has(found, "names toolchain '1.86'"), found)

    def test_a_build_before_the_install_is_refused(self) -> None:
        text = GOOD.replace(
            "      - uses: actions/checkout@v7\n",
            "      - uses: actions/checkout@v7\n      - run: cargo machete\n",
        )
        self.assertTrue(_has(findings(text, "1.90"), "py: step 1 reaches cargo"))

    def test_rust_cache_before_the_install_is_refused(self) -> None:
        text = GOOD.replace(
            "      - uses: actions/checkout@v7\n",
            "      - uses: actions/checkout@v7\n      - uses: Swatinem/rust-cache@v2\n",
        )
        self.assertTrue(_has(findings(text, "1.90"), "py: step 1 reaches cargo"))

    def test_a_nightly_job_that_leaves_the_file_in_charge_is_refused(self) -> None:
        self.assertEqual(
            [
                "fuzz: step 1 reaches cargo outside every checkout, on the runner's default toolchain",
                "fuzz: step 2 reaches cargo outside every checkout, on the runner's default toolchain",
            ],
            _variant(FUZZ_ENV, ""),
        )

    def test_a_nightly_pin_without_a_reason_is_refused(self) -> None:
        for comment in (
            "      # nightly: cargo-fuzz instruments with -Zsanitizer.\n",
            "      # nightly: Miri ships with nightly only; the override keeps every call on it.\n",
        ):
            with self.subTest(comment=comment):
                self.assertTrue(_has(_variant(comment, ""), "pins nightly without a comment"))

    def test_a_cache_holding_a_toolchain_is_refused(self) -> None:
        for path in ("~/.rustup", "~/.cargo/bin"):
            with self.subTest(path=path):
                self.assertTrue(_has(_variant("~/.cargo/git", path), "caches a toolchain path"))

    # Round 2, finding 1: every way of choosing a toolchain is a pin site.
    def test_every_way_of_naming_a_toolchain_is_refused(self) -> None:
        for snippet, name in PIN_SITES:
            with self.subTest(snippet=snippet):
                found = _add_step(snippet)
                self.assertTrue(_has(found, f"names toolchain {name!r}"), found)

    # Round 2, finding 2: a nightly exception holds only if nightly is installed first and said why.
    def test_the_nightly_exceptions_hold_to_what_they_claim(self) -> None:
        with self.subTest(case="rust-cache above the nightly install"):
            found = _variant(FUZZ_INSTALL + FUZZ_CACHE, "      - uses: Swatinem/rust-cache@v2\n" + FUZZ_INSTALL
                             + "      - run: cargo install cargo-fuzz\n")
            self.assertTrue(_has(found, "fuzz: step 0 reaches cargo on 'nightly' before installing it"), found)
        with self.subTest(case="cargo +nightly in a job that never installs nightly"):
            found = _add_step("      - run: cargo +nightly build\n")
            self.assertTrue(_has(found, "py: step 2 reaches cargo on 'nightly' before installing it"), found)
        with self.subTest(case="# TODO instead of the reason"):
            found = _variant("# nightly: cargo-fuzz instruments with -Zsanitizer.", "# TODO")
            self.assertTrue(_has(found, "pins nightly without a comment"), found)

    # Round 2, finding 4: steps at any indentation, and composite actions, are read.
    def test_steps_at_any_indentation_are_seen(self) -> None:
        self.assertEqual({"early": 2}, _written_steps(EARLY))
        self.assertEqual({"early": 2}, {name: len(job.steps) for name, job in parse(EARLY).jobs.items()})
        self.assertTrue(_has(findings(EARLY, "1.90"), "early: step 1 reaches cargo"))

    def test_a_composite_action_is_checked_like_a_job(self) -> None:
        found = findings(COMPOSITE, "1.90")
        self.assertTrue(_has(found, "installs the action's ref"), found)
        self.assertTrue(_has(found, "reaches cargo"), found)

    def test_workflow_files_reads_yaml_and_composite_actions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wanted = [".github/actions/setup/action.yml", ".github/actions/x/y/action.yaml",
                      ".github/workflows/a.yml", ".github/workflows/b.yaml"]
            for name in wanted + [".github/actions/setup/README.md", ".github/workflows/c.txt"]:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text("x\n", encoding="utf-8")
            self.assertEqual(wanted, sorted(p.relative_to(root).as_posix() for p in workflow_files(root)))

    # Round 2, finding 5: a Dockerfile that builds the repository installs from the file too.
    def test_a_dockerfile_building_the_repository_without_the_file_is_refused(self) -> None:
        found = dockerfile_findings(DOCKERFILE_WITHOUT_THE_FILE, "1.90")
        self.assertTrue(_has(found, "rust-toolchain.toml"), found)

    def test_a_dockerfile_that_installs_from_the_file_is_silent(self) -> None:
        self.assertEqual([], dockerfile_findings(DOCKERFILE_FROM_THE_FILE, "1.90"))

    def test_a_nightly_dockerfile_must_say_why_and_name_nothing_else(self) -> None:
        self.assertEqual([], dockerfile_findings(DOCKERFILE_NIGHTLY, "1.90"))
        no_reason = DOCKERFILE_NIGHTLY.replace("# nightly: -Zbuild-std rebuilds std for this benchmark build.\n", "")
        self.assertTrue(_has(dockerfile_findings(no_reason, "1.90"), "pins nightly without a comment"))
        pinned = DOCKERFILE_NIGHTLY.replace("rustup default nightly", "rustup default 1.86")
        self.assertTrue(_has(dockerfile_findings(pinned, "1.90"), "names toolchain '1.86'"))

    # Round 2, finding 6: a build in another checkout needs that checkout's toolchain.
    def test_a_build_in_another_checkout_needs_that_checkouts_toolchain(self) -> None:
        wanted = "regression: step 3 reaches cargo before base/rust-toolchain.toml's toolchain is installed"
        self.assertTrue(_has(findings(BASE, "1.90"), wanted), findings(BASE, "1.90"))
        cd_base = _variant("        working-directory: base\n        run: cargo bench\n",
                           "        run: cd base && cargo bench\n", BASE)
        self.assertTrue(_has(cd_base, wanted), cd_base)
        self.assertEqual([], _variant("      - name: Baseline\n", BASE_INSTALL + "      - name: Baseline\n", BASE))


    # Round 3, finding 1: every channel that sets RUSTUP_TOOLCHAIN reaches the call it governs.
    def test_every_rustup_toolchain_channel_reaches_the_call_it_governs(self) -> None:
        for channel, (steps, index) in CHANNELS.items():
            with self.subTest(channel=channel):
                self.assertEqual([f"j: step {index} reaches cargo on 'nightly' before installing it"], _channel(steps))
        with self.subTest(channel="dockerfile-env"):
            self.assertEqual(["line 7 reaches cargo on 'nightly' before installing it"],
                             dockerfile_findings(DOCKERFILE_ENV_NIGHTLY, "1.90"))

    # Round 3, finding 2: outside every checkout there is no rust-toolchain.toml.
    def test_nothing_outside_a_checkout_counts_as_the_file(self) -> None:
        outside = "j: step 0 installs outside every checkout, so not rust-toolchain.toml's toolchain"
        before = ONE_JOB + "      - run: rustup toolchain install\n      - uses: actions/checkout@v7\n      - run: cargo build\n"
        self.assertEqual([outside, "j: step 2 reaches cargo before rust-toolchain.toml's toolchain is installed"],
                         findings(before, "1.90"))
        without = ONE_JOB + "      - run: rustup toolchain install\n      - run: cargo build\n"
        self.assertEqual([outside, "j: step 1 reaches cargo outside every checkout, on the runner's default toolchain"],
                         findings(without, "1.90"))

    # Round 3, finding 3: the reason names what the job runs on nightly, and belongs to its own step.
    def test_a_nightly_reason_names_what_the_job_runs_and_is_not_borrowed(self) -> None:
        cases = {
            "todo": _variant("# nightly: cargo-fuzz instruments with -Zsanitizer.", "# nightly: TODO"),
            "not run": _variant("# nightly: Miri ships with nightly only; the override keeps every call on it.",
                                "# nightly: -Zsanitizer is unstable."),
            "borrowed": _variant(FUZZ_INSTALL + FUZZ_CACHE, "      # nightly: cargo fuzz needs it\n      - uses: Swatinem/rust-cache@v2\n"
                                 + FUZZ_INSTALL.split("\n", 1)[1] + "      - run: cargo install cargo-fuzz\n"),
        }
        for case, found in cases.items():
            with self.subTest(case=case):
                self.assertTrue(_has(found, "pins nightly without a comment"), found)

    # Round 4: PyYAML reads a flow mapping like a block one, so it is judged, not refused.
    def test_flow_style_env_or_with_is_read_like_block_style(self) -> None:
        found = _add_step("      - env: { RUSTUP_TOOLCHAIN: stable }\n        run: cargo build\n")
        self.assertTrue(_has(found, "names toolchain 'stable'"), found)
        self.assertTrue(_has(found, "py: step 2 reaches cargo on 'stable' before installing it"), found)
        self.assertFalse(_has(found, "flow style"), found)
        found = _add_step("      - uses: PyO3/maturin-action@v1\n        with: { rust-toolchain: stable }\n")
        self.assertTrue(_has(found, "names toolchain 'stable'"), found)
        self.assertFalse(_has(found, "flow style"), found)

    # Round 4, finding 1: YAML forms the hand parser read as empty.
    def test_yaml_forms_the_hand_parser_missed_are_read(self) -> None:
        for form, steps in YAML_FORMS.items():
            with self.subTest(form=form):
                self.assertEqual(["j: step 1 reaches cargo on 'nightly' before installing it"], _channel(steps))

    def test_a_workflow_the_yaml_loader_rejects_is_a_finding(self) -> None:
        found = findings("jobs: [\n", "1.90")
        self.assertTrue(found and found[0].startswith("the workflow is not valid YAML: "), found)

    def test_an_env_that_is_not_a_mapping_is_a_finding(self) -> None:
        text = ONE_JOB + CHECKOUT + INSTALL_STEP + "      - env: ${{ fromJSON(inputs.env) }}\n        run: cargo build\n"
        self.assertEqual(["j: step 2 writes `env` as something other than a mapping, which the guard cannot read"],
                         findings(text, "1.90"))

    # Round 4, finding 2: exec form behind options, and heredocs.
    def test_dockerfile_exec_form_behind_options_and_heredocs_are_refused(self) -> None:
        linked = DOCKER_HEAD + 'COPY --link ["rust-toolchain.toml", "./"]\n'
        self.assertEqual(["line 3 COPY in exec (JSON) form, which the guard cannot read"], dockerfile_findings(linked, "1.90"))
        heredoc = DOCKER_HEAD + "RUN <<EOF\ncargo build\nEOF\n"
        self.assertEqual(["line 3 RUN uses a heredoc, which the guard cannot read"], dockerfile_findings(heredoc, "1.90"))

    # Round 4, finding 3: each fail-closed rule, by its exact message.
    def test_an_unresolvable_working_directory_is_named(self) -> None:
        text = ONE_JOB + CHECKOUT + "      - working-directory: ${{ matrix.dir }}\n        run: cargo build\n"
        self.assertEqual(["j: step 1 reaches cargo where the guard cannot tell which rust-toolchain.toml governs"],
                         findings(text, "1.90"))

    def test_an_exec_form_copy_is_named(self) -> None:
        text = DOCKER_HEAD + 'COPY ["rust-toolchain.toml", "./"]\n'
        self.assertEqual(["line 3 COPY in exec (JSON) form, which the guard cannot read"], dockerfile_findings(text, "1.90"))

    def test_an_exec_form_run_is_named(self) -> None:
        text = DOCKER_HEAD + 'RUN ["cargo", "build"]\n'
        self.assertEqual(["line 3 RUN in exec (JSON) form, which the guard cannot read"], dockerfile_findings(text, "1.90"))

    # Round 4, finding 4: what a job runs comes from its `run:` values and flags, never its keys or names.
    def test_a_nightly_reason_is_read_against_run_values_only(self) -> None:
        for case, (text, expected) in NEEDS_CASES.items():
            with self.subTest(case=case):
                self.assertEqual(expected, findings(text, "1.90"))

    # Round 4, finding 5: `cd` and `pushd`, tokenized, in workflows and in Dockerfiles.
    def test_every_cd_form_moves_the_call(self) -> None:
        reach = "j: step 2 reaches cargo where the guard cannot tell which rust-toolchain.toml governs"
        for form, (snippet, before) in CD_FORMS.items():
            with self.subTest(form=form):
                self.assertEqual(before + [reach], findings(ONE_JOB + CHECKOUT + INSTALL_STEP + snippet, "1.90"))
        docker = DOCKERFILE_FROM_THE_FILE.replace("RUN cargo build --release", 'RUN cd "/opt" && cargo build --release')
        self.assertEqual(["line 7 builds the repository with the base image's toolchain, not rust-toolchain.toml's"],
                         dockerfile_findings(docker, "1.90"))

    # Round 4, finding 6: FROM through ARG is resolved, or refused.
    def test_a_from_behind_an_arg_is_resolved_or_refused(self) -> None:
        by_version = "line 2 names a Rust image by version (rust:1.89); rust-toolchain.toml owns the version"
        self.assertEqual([by_version], dockerfile_findings("ARG TAG=1.89\nFROM rust:${TAG} AS builder\n", "1.90"))
        self.assertEqual([by_version], dockerfile_findings("ARG BASE=rust:1.89\nFROM ${BASE} AS builder\n", "1.90"))
        self.assertEqual(["line 1 FROM the guard cannot resolve: 'rust:${TAG}'"],
                         dockerfile_findings("FROM rust:${TAG} AS builder\n", "1.90"))

    # Round 4: a checkout of another repository carries its toolchain file, not this one's.
    def test_a_foreign_checkout_does_not_carry_this_toolchain_file(self) -> None:
        text = (ONE_JOB + "      - uses: actions/checkout@v7\n        with:\n          repository: someone/other-repo\n"
                + INSTALL_STEP + "      - run: cargo build\n")
        self.assertEqual(["j: step 1 installs in a checkout of someone/other-repo, not from this repository's rust-toolchain.toml",
                          "j: step 2 reaches cargo in a checkout of someone/other-repo, not of this repository"],
                         findings(text, "1.90"))

    def test_a_dockerfile_copy_counts_only_where_the_build_runs(self) -> None:
        cases = {
            "copied elsewhere": DOCKERFILE_FROM_THE_FILE.replace("COPY rust-toolchain.toml ./\n", "COPY rust-toolchain.toml /opt/\n"),
            "built elsewhere": DOCKERFILE_FROM_THE_FILE.replace("COPY Cargo.toml Cargo.lock ./\n", "WORKDIR /build\nCOPY Cargo.toml Cargo.lock ./\n"),
            "exec form": DOCKERFILE_FROM_THE_FILE.replace("COPY rust-toolchain.toml ./\n", 'COPY ["rust-toolchain.toml", "./"]\n'),
        }
        for case, text in cases.items():
            with self.subTest(case=case):
                found = dockerfile_findings(text, "1.90")
                self.assertTrue(found, f"{case}: no finding")

    # Round 3, finding 7: a Rust image named by version is a toolchain name outside the file.
    def test_a_rust_image_named_by_version_is_refused(self) -> None:
        for base in ("rust:1.98-bookworm", "rust:1-slim", "docker.io/library/rust:1.86"):
            with self.subTest(base=base):
                found = dockerfile_findings(DOCKERFILE_FROM_THE_FILE.replace("rust:bookworm", base), "1.90")
                self.assertTrue(_has(found, "names a Rust image by version"), found)


REACHES_THE_FILE = "reaches cargo before rust-toolchain.toml's toolchain is installed"
CANNOT_READ = "which the guard cannot read"
# Round 6: a synthetic repository whose scripts reach cargo directly, through another
# script, or not at all.
SCRIPTS = {
    "scripts/gates.sh": "#!/usr/bin/env bash\ncargo test -p velesdb-core\n",
    "scripts/outer.py": "import subprocess\nsubprocess.run(['bash', 'scripts/gates.sh'], check=True)\n",
    "scripts/lint.py": "print('no build here')\n",
}
SCRIPT_FORMS = ("bash scripts/gates.sh", "sh scripts/gates.sh", "./scripts/gates.sh", "python3 -B scripts/outer.py",
                "cd scripts && ./gates.sh", "FOO=1 bash scripts/gates.sh")


def _in_repository(steps: str, files: dict[str, str] | None = None) -> list[str]:
    """Findings for one job over a repository holding `files`."""
    with tempfile.TemporaryDirectory() as tmp:
        for path, text in (SCRIPTS if files is None else files).items():
            (Path(tmp) / path).parent.mkdir(parents=True, exist_ok=True)
            (Path(tmp) / path).write_text(text, encoding="utf-8")
        return findings(ONE_JOB + CHECKOUT + steps, "1.90", Path(tmp))


def _real_variant(workflow: str, old: str, new: str) -> list[str]:
    text = (WORKFLOW_DIR / workflow).read_text(encoding="utf-8")
    if old not in text:
        raise ValueError(f"not in {workflow}: {old!r}")
    return findings(text.replace(old, new), toolchain_file()[0])


class ScriptAndConditionTests(unittest.TestCase):
    """Round 6: a script that reaches cargo is a build; a conditional install covers only its condition."""

    def test_a_step_running_a_script_that_reaches_cargo_is_a_build(self) -> None:
        for form in SCRIPT_FORMS:
            with self.subTest(form=form):
                self.assertTrue(_has(_in_repository(f"      - run: {form}\n"), f"j: step 1 {REACHES_THE_FILE}"))
                self.assertEqual([], _in_repository(INSTALL_STEP + f"      - run: {form}\n"))

    def test_a_script_that_reaches_no_tool_is_not_a_build(self) -> None:
        self.assertEqual([], _in_repository("      - run: python3 scripts/lint.py\n"))

    def test_a_script_that_cannot_be_read_is_a_finding(self) -> None:
        for form in ("bash scripts/missing.sh", "bash \"$DIR/gates.sh\"", "./scripts/missing.py",
                     "bash /tmp/gates.sh", "bash ../gates.sh"):
            with self.subTest(form=form):
                self.assertTrue(_has(_in_repository(INSTALL_STEP + f"      - run: {form}\n"), CANNOT_READ))
        foreign = "      - uses: actions/checkout@v7\n        with:\n          repository: o/r\n          path: other\n"
        self.assertTrue(_has(_in_repository(foreign + INSTALL_STEP + "      - run: bash other/gates.sh\n"), "a file of o/r"))

    def test_a_fetched_binary_an_installed_module_or_inline_code_is_no_script(self) -> None:
        for form in ("./mcp-publisher publish", "python -m pip install pyyaml", "node -p \"require('./package.json')\"",
                     "python3 -c 'print(1)'", "/tmp/venv/bin/pip install x"):
            with self.subTest(form=form):
                self.assertEqual([], _in_repository(f"      - run: {form}\n"))

    def test_an_exempt_script_is_not_followed(self) -> None:
        with mock.patch.dict(SCRIPTS_THAT_ONLY_NAME_A_TOOL, {"scripts/gates.sh": "test"}):
            self.assertEqual([], _in_repository("      - run: bash scripts/gates.sh\n"))

    def test_a_conditional_install_covers_only_a_build_under_the_same_condition(self) -> None:
        install = "      - if: ${{ matrix.rust }}\n        run: rustup toolchain install --no-self-update --profile minimal\n"
        cases = {
            "": True,
            "        if: ${{ github.event_name == 'push' }}\n": True,
            "        if: matrix.rust\n": False,
            "        if: ${{  matrix.rust }}\n": False,
        }
        for condition, refused in cases.items():
            with self.subTest(condition=condition):
                found = _in_repository(install + "      - run: cargo build\n" + condition)
                self.assertEqual(refused, _has(found, "installed only under `if: matrix.rust`"), found)

    def test_an_install_under_always_covers_every_build_and_success_is_no_condition(self) -> None:
        for condition, build in (("always()", "failure()"), ("${{ always() }}", "cancelled()"), ("success()", "success()")):
            with self.subTest(condition=condition):
                install = f"      - if: {condition}\n        run: rustup toolchain install --no-self-update --profile minimal\n"
                self.assertEqual([], _in_repository(install + f"      - if: {build}\n        run: cargo build\n"))
        # `success()` on the build is the default: an install with no `if:` covers it...
        self.assertEqual([], _in_repository(INSTALL_STEP + "      - if: success()\n        run: cargo build\n"))
        # ...and an install under `success()` covers a build with no `if:`.
        success = "      - if: success()\n        run: rustup toolchain install --no-self-update --profile minimal\n"
        self.assertEqual([], _in_repository(success + "      - run: cargo build\n"))

    # The review's reproduction (a): production-gates.yml reaches cargo only inside its script.
    def test_production_gates_without_its_install_step_is_refused(self) -> None:
        found = _real_variant("production-gates.yml",
                              "      - name: Install the toolchain rust-toolchain.toml pins\n"
                              "        run: rustup toolchain install --no-self-update --profile minimal\n", "")
        self.assertTrue(_has(found, f"gates: step 1 {REACHES_THE_FILE}"), found)

    # The review's reproduction (b): binary-size.yml's install step made conditional.
    def test_binary_size_with_a_disabled_install_step_is_refused(self) -> None:
        found = _real_variant("binary-size.yml", "      - name: Install the toolchain rust-toolchain.toml pins\n",
                              "      - name: Install the toolchain rust-toolchain.toml pins\n        if: false\n")
        self.assertTrue(_has(found, "installed only under `if: false`"), found)

    def test_every_exempt_script_still_names_a_tool(self) -> None:
        for path in SCRIPTS_THAT_ONLY_NAME_A_TOOL:
            others = {other: why for other, why in SCRIPTS_THAT_ONLY_NAME_A_TOOL.items() if other != path}
            with self.subTest(script=path):
                self.assertTrue(script_builds(REPO_ROOT, path, others), "the exemption outlived its reason")

    # The reasons two exemptions give, held.
    def test_no_workflow_runs_the_extraction_bench_report(self) -> None:
        for path in workflow_files():
            with self.subTest(workflow=path.name):
                self.assertNotRegex(path.read_text(encoding="utf-8"), r"bench-memory-extraction\.py\s+report\b")

    def test_no_executable_promise_claim_names_a_tool(self) -> None:
        registry = json.loads((REPO_ROOT / "docs" / "reference" / "promise-contract.json").read_text(encoding="utf-8"))
        commands = [claim.get("validation_command") or "" for claim in registry["claims"] if claim.get("executable")]
        self.assertTrue(commands)
        self.assertEqual([], [command for command in commands if SCRIPT_TOOL_RE.search(command)])


AUTO_INSTALL_ENV = 'env:\n  RUSTUP_AUTO_INSTALL: "0"\n'
AUTO_INSTALL_JOB = AUTO_INSTALL_ENV + ONE_JOB + CHECKOUT
OUTSIDE_TOP_LEVEL = AUTO_INSTALL_PAST_TOP_LEVEL


class RuntimeBackstopTests(unittest.TestCase):
    """Round 8: every workflow turns rustup's automatic install off, and nothing turns it back on."""

    def test_a_workflow_that_turns_auto_install_off_is_silent(self) -> None:
        self.assertEqual([], auto_install_findings(AUTO_INSTALL_JOB + INSTALL_STEP))
        self.assertEqual([], auto_install_findings("env:\n  RUSTUP_AUTO_INSTALL: 0\n" + ONE_JOB + CHECKOUT))

    def test_a_workflow_that_leaves_auto_install_on_is_refused(self) -> None:
        cases = {
            "no top-level env": ONE_JOB + CHECKOUT,
            "another value": 'env:\n  RUSTUP_AUTO_INSTALL: "1"\n' + ONE_JOB + CHECKOUT,
            "a job env only": ONE_JOB.replace("    steps:\n", '    env:\n      RUSTUP_AUTO_INSTALL: "0"\n    steps:\n') + CHECKOUT,
        }
        for label, text in cases.items():
            with self.subTest(case=label):
                self.assertTrue(_has(auto_install_findings(text), "does not set RUSTUP_AUTO_INSTALL"), label)

    # Rounds 8 and 9: anything past the top-level key that names the variable may set or remove it.
    def test_every_mention_past_the_top_level_key_is_refused(self) -> None:
        with_job_defaults = ONE_JOB.replace("    steps:\n", "    defaults:\n      run:\n        shell: env -u RUSTUP_AUTO_INSTALL bash -e {0}\n    steps:\n")
        cases = {
            "job env": AUTO_INSTALL_ENV + ONE_JOB.replace("    steps:\n", '    env:\n      RUSTUP_AUTO_INSTALL: "1"\n    steps:\n') + CHECKOUT,
            "job env set to 0": AUTO_INSTALL_ENV + ONE_JOB.replace("    steps:\n", '    env:\n      RUSTUP_AUTO_INSTALL: "0"\n    steps:\n') + CHECKOUT,
            "step env": AUTO_INSTALL_JOB + '      - env:\n          RUSTUP_AUTO_INSTALL: ""\n        run: cargo build\n',
            "GITHUB_ENV assignment": AUTO_INSTALL_JOB + '      - run: echo "RUSTUP_AUTO_INSTALL=1" >> "$GITHUB_ENV"\n',
            "rustup set": AUTO_INSTALL_JOB + "      - run: rustup set auto-install enable\n",
            "unset": AUTO_INSTALL_JOB + "      - run: unset RUSTUP_AUTO_INSTALL && cargo build\n",
            "env -u": AUTO_INSTALL_JOB + "      - run: env -u RUSTUP_AUTO_INSTALL cargo build\n",
            "pwsh Remove-Item": AUTO_INSTALL_JOB + "      - shell: pwsh\n        run: Remove-Item Env:RUSTUP_AUTO_INSTALL\n",
            "step shell": AUTO_INSTALL_JOB + "      - shell: env -u RUSTUP_AUTO_INSTALL bash -e {0}\n        run: cargo build\n",
            "job defaults shell": AUTO_INSTALL_ENV + with_job_defaults + CHECKOUT + "      - run: cargo build\n",
            "workflow defaults shell": (AUTO_INSTALL_ENV + "defaults:\n  run:\n    shell: env -u RUSTUP_AUTO_INSTALL bash -e {0}\n"
                                        + ONE_JOB + CHECKOUT),
            "multi-line GITHUB_ENV": (AUTO_INSTALL_JOB + '      - run: |\n          echo "RUSTUP_AUTO_INSTALL<<EOF" >> "$GITHUB_ENV"\n'
                                      '          echo 1 >> "$GITHUB_ENV"\n          echo EOF >> "$GITHUB_ENV"\n'),
            "printf into GITHUB_ENV": AUTO_INSTALL_JOB + "      - run: printf '%s=\\n' RUSTUP_AUTO_INSTALL >> \"$GITHUB_ENV\"\n",
        }
        for label, text in cases.items():
            with self.subTest(case=label):
                found = auto_install_findings(text)
                self.assertTrue(_has(found, OUTSIDE_TOP_LEVEL), found)

    def test_the_top_level_key_itself_is_not_a_mention(self) -> None:
        self.assertEqual([], auto_install_findings(AUTO_INSTALL_JOB + INSTALL_STEP + "      - run: cargo build\n"))

    def test_a_workflow_the_loader_rejects_or_a_bare_composite_action_is_refused(self) -> None:
        self.assertTrue(_has(auto_install_findings(AUTO_INSTALL_JOB + "  - [\n"), "not valid YAML"))
        composite = "runs:\n  using: composite\n  steps:\n    - run: cargo build\n      shell: bash\n"
        self.assertTrue(_has(auto_install_findings(composite), "of a composite action does not set RUSTUP_AUTO_INSTALL"))
        covered = composite.replace("      shell: bash\n", '      shell: bash\n      env:\n        RUSTUP_AUTO_INSTALL: "0"\n')
        self.assertEqual([], auto_install_findings(covered))

    def test_removing_it_from_a_real_workflow_is_refused(self) -> None:
        for workflow in ("ci.yml", "tag-release.yml"):
            with self.subTest(workflow=workflow):
                text = (WORKFLOW_DIR / workflow).read_text(encoding="utf-8")
                line = '  RUSTUP_AUTO_INSTALL: "0"\n'
                self.assertIn(line, text)
                self.assertTrue(_has(auto_install_findings(text.replace(line, "")), "does not set RUSTUP_AUTO_INSTALL"))

    # Round 7, finding 4: a call that can run after a failure is not covered by an install that cannot.
    def test_a_call_that_can_run_after_a_failure_needs_an_install_that_can_too(self) -> None:
        after_failure = ("always()", "failure()", "${{ !cancelled() }}", "cancelled() || github.ref == 'x'")
        for condition in after_failure:
            with self.subTest(build=condition):
                found = _in_repository(INSTALL_STEP + f"      - if: {condition}\n        run: cargo build\n")
                self.assertTrue(_has(found, f"j: step 2 {REACHES_THE_FILE}"), found)
                always = "      - if: always()\n        run: rustup toolchain install --no-self-update --profile minimal\n"
                self.assertEqual([], _in_repository(always + f"      - if: {condition}\n        run: cargo build\n"))
                same = f"      - if: {condition}\n        run: rustup toolchain install --no-self-update --profile minimal\n"
                self.assertEqual([], _in_repository(same + f"      - if: {condition}\n        run: cargo build\n"))
        self.assertEqual([], _in_repository(INSTALL_STEP + "      - if: github.event_name == 'push'\n        run: cargo build\n"))


class RealWorkflowTests(unittest.TestCase):
    # Rounds 3 and 4 (finding 7): every loom command a doc gives is one CI runs.
    def test_every_documented_loom_command_is_one_ci_runs(self) -> None:
        ci = [sorted(command) for command in _ci_loom_commands()]
        self.assertEqual(2, len(ci), ci)
        documented = _documented_loom_commands()
        self.assertLessEqual({"crates/velesdb-core/Cargo.toml", "crates/velesdb-core/tests/loom_tests.rs",
                              "crates/velesdb-core/src/storage/loom_tests.rs", "crates/velesdb-core/src/sync.rs",
                              "docs/CONCURRENCY_MODEL.md"}, {path for path, _, _ in documented})
        for path, number, command in documented:
            with self.subTest(doc=f"{path}:{number}"):
                self.assertIn(sorted(command), ci)

    # Round 4, finding 8: the Dockerfile says what its base image carries.
    def test_the_dockerfile_says_what_its_base_image_carries(self) -> None:
        text = " ".join(line.lstrip("# ").strip() for line in (REPO_ROOT / "Dockerfile").read_text(encoding="utf-8").splitlines())
        self.assertIn("carries rustup and a stable toolchain this build does not use", text)

    # Round 4, finding 9: the dx-timing Dockerfile writes no toolchain version, comments included.
    def test_dx_timing_writes_no_toolchain_version(self) -> None:
        channel, _ = toolchain_file()
        self.assertNotIn(channel, (REPO_ROOT / "scripts" / "dx-timing" / "Dockerfile.rust").read_text(encoding="utf-8"))

    # Round 4: every job that runs this guard installs PyYAML first, at one pin.
    def test_every_job_that_runs_this_guard_installs_pyyaml_at_one_pin(self) -> None:
        pins, runners = set(), set()
        for path in workflow_files():
            for name, job in parse(path.read_text(encoding="utf-8")).jobs.items():
                for index, step in enumerate(job.steps):
                    if not RUNS_THIS_GUARD_RE.search(step.run):
                        continue
                    runners.add(f"{path.name}::{name}")
                    found = [m.group(1) for s in job.steps[:index] if (m := PYYAML_RE.search(s.run))]
                    with self.subTest(job=f"{path.name}::{name}"):
                        self.assertTrue(found, "runs this guard without installing pyyaml==<pin> first")
                    pins.update(found)
        self.assertLessEqual({"gate-contracts.yml::script-gates", "ci.yml::mcp-doc-contract"}, runners)
        self.assertEqual(1, len(pins), pins)

    def test_the_toolchain_file_names_a_channel_and_components(self) -> None:
        channel, components = toolchain_file()
        self.assertRegex(channel, r"^\d+\.\d+(\.\d+)?$")
        self.assertTrue(components, "rust-toolchain.toml names no components")

    def test_every_workflow_parses_into_jobs_with_steps(self) -> None:
        workflows = workflow_files()
        self.assertTrue(workflows)
        for path in workflows:
            with self.subTest(workflow=path.name):
                jobs = parse(path.read_text(encoding="utf-8")).jobs
                self.assertTrue(jobs, "no job parsed")
                self.assertTrue(any(job.steps for job in jobs.values()), "no step parsed")

    def test_every_job_parses_as_many_steps_as_it_writes(self) -> None:
        for path in workflow_files():
            with self.subTest(workflow=path.name):
                text = path.read_text(encoding="utf-8")
                parsed = {name: len(job.steps) for name, job in parse(text).jobs.items()}
                self.assertEqual(_written_steps(text), parsed)

    def test_the_python_build_jobs_install_from_the_file_before_pip(self) -> None:
        jobs = parse((WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")).jobs
        for name in ("python-integrations", "python-sdk-tests"):
            with self.subTest(job=name):
                steps = jobs[name].steps
                build = next(i for i, s in enumerate(steps) if "pip install ./crates/" in s.run)
                install = next((i for i, s in enumerate(steps) if _installs_from_the_file(s.run)), None)
                self.assertIsNotNone(install, "no `rustup toolchain install` from the file")
                self.assertLess(install, build)

    def test_no_workflow_pins_a_toolchain_or_reaches_cargo_before_installing_it(self) -> None:
        report = real_findings()
        self.assertEqual(
            {},
            report,
            "\n".join(f"{wf}: {f}" for wf, found in report.items() for f in found),
        )

    def test_every_dockerfile_that_builds_rust_installs_from_the_file(self) -> None:
        channel, _ = toolchain_file()
        paths = dockerfiles()
        self.assertIn("Dockerfile", [path.relative_to(REPO_ROOT).as_posix() for path in paths])
        for path in paths:
            with self.subTest(dockerfile=path.relative_to(REPO_ROOT).as_posix()):
                self.assertEqual([], dockerfile_findings(path.read_text(encoding="utf-8"), channel))

    # Round 2, finding 3: loom no longer runs on nightly, and no line may say it does.
    def test_no_tracked_file_says_loom_runs_on_nightly(self) -> None:
        claims = loom_nightly_claims()
        self.assertEqual([], claims, "\n".join(claims))


if __name__ == "__main__":
    unittest.main()
