#!/usr/bin/env python3
"""Run the Python examples against the installed `velesdb` wheel.

The examples were only compiled (`py_compile`) in CI, so the root README's
"three commands that work" shipped on 8.0.0 calling `SearchOptions(top_k=)`,
which 8.0.0 removed: every name resolved, and every run failed. This runs each
example that needs no network, server or API key, in its own temporary
directory, and fails on the first non-zero exit.

Examples that embed text get the deterministic stand-in encoder in
`scripts/fixtures/stub_sentence_transformers`, so no model is downloaded.

Usage: python scripts/run-python-examples.py [--python PATH]
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
STUB_ENCODER = REPO_ROOT / "scripts" / "fixtures" / "stub_sentence_transformers"

#: Every example that runs offline against the wheel alone. Left out, each for
#: a stated reason: `examples/python_example.py` talks to a running server; the
#: `graphrag_*` examples call OpenAI; the CLIP tutorial downloads a CLIP model.
EXAMPLES = (
    "examples/python/hello_velesdb.py",
    "examples/python/hello_velesdb_text.py",
    "examples/python/hybrid_queries.py",
    "examples/python/multimodel_notebook.py",
    "examples/python/fusion_strategies.py",
    "examples/python/graph_traversal.py",
    "examples/tutorials/distance_metrics_demo.py",
    "examples/tutorials/multi_query_fusion_demo.py",
    "examples/tutorials/velesql_tutorial_complete.py",
)

TIMEOUT_SECONDS = 300


def run_example(python: str, example: str) -> "tuple[int, str]":
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(STUB_ENCODER), env.get("PYTHONPATH")]))
    with tempfile.TemporaryDirectory() as workdir:
        done = subprocess.run(
            [python, str(REPO_ROOT / example)],
            cwd=workdir,
            env=env,
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    return done.returncode, (done.stdout + done.stderr)[-2000:]


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--python", default=sys.executable, help="interpreter with velesdb installed")
    args = parser.parse_args(argv)

    failures = 0
    for example in EXAMPLES:
        code, tail = run_example(args.python, example)
        if code == 0:
            print(f"  OK    {example}")
            continue
        failures += 1
        print(f"  FAIL  {example} (exit {code})\n{tail}")
        print(f"::error file={example}::{example} exited with {code} against the installed velesdb wheel")
    print(f"\n{len(EXAMPLES) - failures}/{len(EXAMPLES)} examples ran.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
