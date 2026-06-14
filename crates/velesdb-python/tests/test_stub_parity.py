"""Regression guard: the ``__init__.pyi`` type stub must declare the
documented compatibility surface so typed callers (mypy/pyright) do not see
false "attribute does not exist" errors for methods that exist at runtime.

This test parses the stub with :mod:`ast` only — it does not import the
compiled extension, so it runs anywhere pytest collects it.
"""

import ast
from pathlib import Path


STUB_PATH = Path(__file__).parents[1] / "python" / "velesdb" / "__init__.pyi"


def _classes() -> dict[str, ast.ClassDef]:
    module = ast.parse(STUB_PATH.read_text())
    return {
        node.name: node
        for node in module.body
        if isinstance(node, ast.ClassDef)
    }


def _methods(classes: dict[str, ast.ClassDef], class_name: str) -> set[str]:
    return {
        node.name
        for node in classes[class_name].body
        if isinstance(node, ast.FunctionDef)
    }


def test_compatibility_stub_surface_matches_runtime() -> None:
    """Documented compatibility APIs must be declared in the type stub."""
    classes = _classes()

    expected = {
        "FusionStrategy": {"rsf"},
        "Database": {"get_collections"},
        "GraphStore": {"get_outgoing_edges", "get_incoming_edges", "has_edge"},
        "PyGraphCollection": {"get_outgoing_edges", "get_incoming_edges", "upsert_node"},
        "GraphCollection": {"add_node", "bfs", "dfs", "has_edge"},
        "AgentMemory": {"rollback"},
    }

    missing: list[str] = []
    for class_name, method_names in expected.items():
        assert class_name in classes, f"type stub is missing class {class_name!r}"
        missing.extend(
            f"{class_name}.{name}"
            for name in sorted(method_names - _methods(classes, class_name))
        )

    weighted_overloads = [
        node
        for node in classes["FusionStrategy"].body
        if isinstance(node, ast.FunctionDef) and node.name == "weighted"
    ]
    if len(weighted_overloads) < 4:
        missing.append("FusionStrategy.weighted overloads")

    assert missing == [], f"type stub drifted from runtime: {missing}"
