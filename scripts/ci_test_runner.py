"""Shared process runner for CI-only Node and WASM functional gates.

The binding crates are outside ``cargo test --workspace`` or contain tests
that only exist on wasm32.  Their workflow steps therefore are guards in
their own right.  Keeping the command construction here gives those guards a
real CLI surface: the workflow and the refusal-vector harness execute the
same sequencing and exit-code propagation.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent


@dataclass(frozen=True)
class Stage:
    """One named subprocess in a blocking CI gate."""

    label: str
    command: tuple[str, ...]


def run_stages(stages: Sequence[Stage], cwd: Path) -> int:
    """Run stages in order, normalising a tool refusal to exit code 1.

    A child that ran and returned non-zero is a gate refusal.  Failure to
    start the child is operational breakage and remains distinguishable as
    exit 2, matching the repository refusal-vector contract.
    """
    for stage in stages:
        print(f"[{stage.label}] {shlex.join(stage.command)}", flush=True)
        try:
            result = subprocess.run(  # noqa: S603 - fixed, CLI-visible tool path
                list(stage.command), cwd=cwd, check=False
            )
        except OSError as error:
            print(
                f"ERROR: could not start {stage.command[0]!r}: {error}",
                file=sys.stderr,
            )
            return 2
        if result.returncode != 0:
            print(
                f"ERROR: {stage.label} failed with exit code {result.returncode}.",
                file=sys.stderr,
            )
            return 1
    return 0


def node_stages(npm: str, npx: str) -> tuple[Stage, ...]:
    """The Linux N-API sequence formerly written inline in ci.yml."""
    return (
        Stage("Install Node dependencies", (npm, "install", "--no-audit", "--no-fund")),
        Stage("Build the napi addon", (npx, "napi", "build", "--platform")),
        Stage("Run the Node suites", (npm, "test")),
    )


def windows_node_stages(npm: str, npx: str, node: str) -> tuple[Stage, ...]:
    """The Windows N-API sequence, including its deliberately explicit spec."""
    return (
        Stage("Install Node dependencies", (npm, "install", "--no-audit", "--no-fund")),
        Stage("Build the napi addon", (npx, "napi", "build", "--platform")),
        Stage(
            "Run the Windows Node suite",
            (node, "--test", "__test__/index.spec.mjs"),
        ),
    )


def wasm_stages(wasm_pack: str) -> tuple[Stage, ...]:
    """The wasm-bindgen suite and the Node runner it requires."""
    return (
        Stage(
            "Run wasm-bindgen tests under Node",
            (wasm_pack, "test", "--node", "crates/velesdb-wasm"),
        ),
    )


def _tool_default(name: str) -> str:
    """Use the command shim suffix CreateProcess requires on Windows."""
    if os.name == "nt" and name in {"npm", "npx"}:
        return f"{name}.cmd"
    return name


def _parser(description: str) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="repository root (defaults to the checkout containing this script)",
    )
    return parser


def _require_directory(parser: argparse.ArgumentParser, path: Path, label: str) -> Path:
    resolved = path.resolve()
    if not resolved.is_dir():
        parser.error(f"{label} does not exist: {resolved}")
    return resolved


def run_node_binding(argv: Sequence[str] | None = None) -> int:
    """Run the Linux N-API build and complete Node suite."""
    parser = _parser("Build the N-API addon and run its Linux Node suites.")
    parser.add_argument("--npm", default=_tool_default("npm"), help="npm executable")
    parser.add_argument("--npx", default=_tool_default("npx"), help="npx executable")
    args = parser.parse_args(argv)
    package = _require_directory(
        parser, args.root / "crates" / "velesdb-node", "Node package directory"
    )
    return run_stages(node_stages(args.npm, args.npx), package)


def run_windows_node_binding(argv: Sequence[str] | None = None) -> int:
    """Run the Windows N-API build and its portable regression spec."""
    parser = _parser("Build the N-API addon and run its Windows Node suite.")
    parser.add_argument("--npm", default=_tool_default("npm"), help="npm executable")
    parser.add_argument("--npx", default=_tool_default("npx"), help="npx executable")
    parser.add_argument("--node", default="node", help="Node executable")
    args = parser.parse_args(argv)
    package = _require_directory(
        parser, args.root / "crates" / "velesdb-node", "Node package directory"
    )
    return run_stages(
        windows_node_stages(args.npm, args.npx, args.node), package
    )


def run_wasm_bindgen(argv: Sequence[str] | None = None) -> int:
    """Run the wasm32-only wasm-bindgen suites under Node."""
    parser = _parser("Run the wasm-bindgen functional suites under Node.")
    parser.add_argument("--wasm-pack", default="wasm-pack", help="wasm-pack executable")
    args = parser.parse_args(argv)
    root = _require_directory(parser, args.root, "repository root")
    _require_directory(parser, root / "crates" / "velesdb-wasm", "WASM crate directory")
    return run_stages(wasm_stages(args.wasm_pack), root)
