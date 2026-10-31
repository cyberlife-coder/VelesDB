"""The published GraphRAG example runs end to end offline, against the real binding.

`examples/python/graphrag_langchain.py --offline` uses the framework's own fake
embeddings and LLM, so it needs no API key; this keeps the published example
working, graph expansion included.
"""

import os
import subprocess
import sys
from pathlib import Path

import pytest

pytest.importorskip("velesdb")
pytest.importorskip("langchain_velesdb")

EXAMPLE = Path(__file__).resolve().parents[3] / "examples" / "python" / "graphrag_langchain.py"


def test_given_no_api_key_when_example_runs_offline_then_graph_expands():
    env = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY"}
    env["PYTHONPATH"] = os.pathsep.join(p for p in sys.path if p)

    run = subprocess.run(
        [sys.executable, str(EXAMPLE), "--offline"],
        env=env, capture_output=True, text=True, timeout=120, check=False,
    )

    assert run.returncode == 0, run.stderr
    assert "(offline)" in run.stdout
    assert "(graph, depth 1)" in run.stdout  # a neighbour reached through an edge
