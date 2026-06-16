"""Regression guard: the ``__init__.pyi`` type stub must keep up with the
runtime surface so typed callers (mypy/pyright) do not see false "attribute
does not exist" errors for things that exist at runtime.

Two complementary checks:

* ``test_compatibility_stub_surface_matches_runtime`` is a *curated*, AST-only
  check (no import needed) for the compatibility aliases and call shapes that
  are easy to forget. Method coverage is curated on purpose: the facade classes
  (``GraphStore``, ``GraphCollection``, ``Collection``) delegate via
  ``__getattr__``, so an exhaustive ``dir()`` diff would be brittle — it cannot
  see delegated members and would flag inherited ones.
* ``test_every_exported_class_is_declared_in_stub`` imports the compiled module
  and is *fully automatic*: every class in ``velesdb.__all__`` must have a stub
  class, so a newly exported class can never silently miss the stub.
"""

import ast
from pathlib import Path

import pytest


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
        "GraphStore": {
            "get_outgoing_edges",
            "get_incoming_edges",
            "has_edge",
            "traverse_bfs",
            "traverse_dfs",
        },
        "PyGraphCollection": {"get_outgoing_edges", "get_incoming_edges", "upsert_node"},
        "GraphCollection": {"add_node", "bfs", "dfs", "has_edge", "close"},
        "ParsedStatement": {
            "is_ddl",
            "is_dml",
            "is_delete",
            "is_insert_edge",
            "has_having",
        },
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


def test_every_exported_class_is_declared_in_stub() -> None:
    """Every class in ``velesdb.__all__`` must have a matching stub class.

    This is the automatic half of the guard: it needs the compiled extension,
    so it is skipped when the module is not built (e.g. a docs-only checkout).
    """
    import inspect

    velesdb = pytest.importorskip("velesdb")
    exported = getattr(velesdb, "__all__", None) or dir(velesdb)
    exported_classes = {
        name for name in exported if inspect.isclass(getattr(velesdb, name, None))
    }
    missing = sorted(exported_classes - set(_classes()))
    assert missing == [], f"exported classes missing from the type stub: {missing}"
