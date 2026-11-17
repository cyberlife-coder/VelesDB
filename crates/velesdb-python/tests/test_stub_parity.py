"""Regression guard: the ``__init__.pyi`` type stub must keep up with the
runtime surface so typed callers (mypy/pyright) do not see false "attribute
does not exist" errors for things that exist at runtime.

Complementary checks:

* ``test_compatibility_stub_surface_matches_runtime`` is a *curated*, AST-only
  check (no import needed) for the compatibility aliases and call shapes that
  are easy to forget, so it still guards a docs-only checkout.
* ``test_every_exported_class_is_declared_in_stub`` imports the compiled module
  and is *fully automatic*: every class in ``velesdb.__all__`` must have a stub
  class, so a newly exported class can never silently miss the stub.
* ``test_stub_declares_every_runtime_member_and_keyword`` is the automatic
  member-level diff: every public method, property and keyword argument of
  every exported class, both ways. A facade class that forwards through
  ``__getattr__`` is read as its own members plus those of the compiled class
  it wraps, so delegated members are seen and inherited ones are not.
"""

import ast
import inspect
from pathlib import Path
from typing import Any

import pytest


STUB_PATH = Path(__file__).parents[1] / "python" / "velesdb" / "__init__.pyi"


def _classes(source: "str | None" = None) -> dict[str, ast.ClassDef]:
    module = ast.parse(STUB_PATH.read_text() if source is None else source)
    return {
        node.name: node
        for node in module.body
        if isinstance(node, ast.ClassDef)
    }


def _methods(classes: dict[str, ast.ClassDef], class_name: str) -> set[str]:
    """Methods a type checker sees on ``class_name``, inherited ones included."""
    return {
        name
        for name, nodes in _stub_members(classes, class_name).items()
        if any(isinstance(node, ast.FunctionDef) for node in nodes)
    }


def test_compatibility_stub_surface_matches_runtime() -> None:
    """Documented compatibility APIs must be declared in the type stub."""
    classes = _classes()

    expected = {
        "FusionStrategy": {"rsf", "from_name", "fused_higher_is_better"},
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


def test_every_exported_function_is_declared_in_stub() -> None:
    """Every module-level function in ``velesdb.__all__`` must have a stub
    ``def``, the function half of the class guard above."""
    import inspect

    velesdb = pytest.importorskip("velesdb")
    exported = getattr(velesdb, "__all__", None) or dir(velesdb)
    exported_functions = {
        name for name in exported if inspect.isroutine(getattr(velesdb, name, None))
    }
    stub_functions = {
        node.name
        for node in ast.parse(STUB_PATH.read_text()).body
        if isinstance(node, ast.FunctionDef)
    }
    missing = sorted(exported_functions - stub_functions)
    assert missing == [], f"exported functions missing from the type stub: {missing}"


@pytest.mark.parametrize("class_name", ["GraphStore", "GraphCollection"])
def test_add_edge_stub_declares_every_facade_call_shape(class_name):
    """Each `add_edge` form the facade accepts has a typed overload.

    `add_edge(source, target, label, id=n)` worked at runtime but no overload
    declared a keyword-only `id`, so a type checker refused the call; the
    `GraphStore` stub declared only the dict form (Devin, #2831).
    """
    overloads = [
        node
        for node in _classes()[class_name].body
        if isinstance(node, ast.FunctionDef) and node.name == "add_edge"
    ]
    shapes = set()
    for fn in overloads:
        positional = [a.arg for a in (fn.args.posonlyargs + fn.args.args)[1:]]
        keyword_only = [a.arg for a in fn.args.kwonlyargs]
        shapes.add((positional[0] if positional else None, "id" in keyword_only, "source" in keyword_only))
    assert ("edge", False, False) in shapes, "dict form"
    assert ("edge_id", False, True) in shapes, "explicit id positional, source keyword"
    assert ("source", True, False) in shapes, "source positional, id keyword"
    assert ("source", False, False) in shapes, "no-id positional form"


# ---------------------------------------------------------------------------
# Automatic runtime <-> stub diff, member by member and keyword by keyword.
#
# The runtime side is read from the built module: a compiled class's own
# members and their ``__text_signature__`` (``inspect.signature``), and, for a
# facade class of ``velesdb/__init__.py``, the facade's members plus those of
# the compiled class it forwards to through ``__getattr__``. The stub side is
# the ``.pyi`` AST. Where pyo3 provides no signature the member's name is
# still compared and its parameters are skipped.
# ---------------------------------------------------------------------------

#: Protocol methods a type checker resolves on the class, so the stub must
#: declare them; other dunders (``__repr__``, ``__eq__``, …) are not compared.
_PROTOCOL_DUNDERS = frozenset(
    {"__len__", "__contains__", "__iter__", "__next__", "__enter__", "__exit__", "__getitem__"}
)

#: The facade constructor parameter that receives the wrapped compiled
#: object: the wrapping seam the facade's own factories use, not caller API.
_FACADE_SEAM_PARAM = "inner"


def _facade_targets(velesdb) -> dict[str, type]:
    """Exported facade class name -> the compiled class it forwards to."""
    raw = velesdb.velesdb
    return {
        "Collection": raw.Collection,
        "Database": raw.Database,
        "GraphStore": raw.GraphStore,
        "GraphCollection": raw.GraphCollection,
    }


def _is_compared(name: str) -> bool:
    return not name.startswith("_") or name in _PROTOCOL_DUNDERS


def _runtime_members(cls: type, inner: "type | None") -> dict[str, Any]:
    """Public members (and protocol dunders) defined by ``cls`` itself, plus
    those of the compiled class a facade forwards to."""
    members: dict[str, Any] = {}
    for owner in ([inner] if inner is not None else []) + [cls]:
        for name in vars(owner):
            if _is_compared(name):
                members[name] = getattr(owner, name)
    return members


def _stub_members(classes: dict[str, ast.ClassDef], class_name: str) -> dict[str, list[ast.AST]]:
    """What a type checker sees on ``class_name``: its own declarations over
    those of the base classes it names that the stub defines, resolved like
    an MRO. Nothing else counts — a runtime ``__getattr__`` the stub does not
    declare is invisible to a type checker, so it is invisible here too."""
    members: dict[str, list[ast.AST]] = {}
    for base in reversed(classes[class_name].bases):
        if isinstance(base, ast.Name) and base.id in classes:
            members.update(_stub_members(classes, base.id))
    own: dict[str, list[ast.AST]] = {}
    for node in classes[class_name].body:
        if isinstance(node, ast.FunctionDef):
            own.setdefault(node.name, []).append(node)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            own.setdefault(node.target.id, []).append(node)
    members.update(own)
    return members


def _runtime_keywords(obj: Any) -> "tuple[set[str], bool] | None":
    """Names a caller can pass by keyword, and whether ``**kwargs`` takes
    any other; ``None`` when no signature is available."""
    try:
        params = inspect.signature(obj).parameters.values()
    except (TypeError, ValueError):
        return None
    if any(p.kind is p.VAR_POSITIONAL for p in params):
        return None
    names = {
        p.name
        for p in params
        if p.kind in (p.POSITIONAL_OR_KEYWORD, p.KEYWORD_ONLY) and p.name not in ("self", "cls")
    }
    return names, any(p.kind is p.VAR_KEYWORD for p in params)


def _stub_keywords(nodes: list[ast.AST]) -> "tuple[set[str], bool] | None":
    """The same for a stub member, across all of its overloads."""
    names: set[str] = set()
    open_ended = False
    for node in nodes:
        if not isinstance(node, ast.FunctionDef):
            return None
        args = node.args
        names |= {a.arg for a in args.args + args.kwonlyargs} - {"self", "cls"}
        open_ended |= args.kwarg is not None
    return names, open_ended


def _keyword_drift(where: str, runtime: Any, stub_nodes: list[ast.AST], exempt=frozenset()) -> list[str]:
    actual = _runtime_keywords(runtime)
    declared = _stub_keywords(stub_nodes)
    if actual is None or declared is None:
        return []
    runtime_names, runtime_open = actual
    stub_names, stub_open = declared
    runtime_names -= exempt
    drift = []
    if not stub_open:
        drift += [f"{where}({name}=) missing from stub" for name in sorted(runtime_names - stub_names)]
    if not runtime_open:
        drift += [f"{where}({name}=) not accepted at runtime" for name in sorted(stub_names - runtime_names)]
    return drift


def _class_drift(classes, name: str, cls: type, inner: "type | None") -> list[str]:
    """Drift between runtime class ``cls`` (forwarding to ``inner`` when it
    is a facade) and the stub class ``name``."""
    runtime = _runtime_members(cls, inner)
    stub = _stub_members(classes, name)
    drift = [f"{name}.{m} missing from stub" for m in sorted(set(runtime) - set(stub))]
    drift += [
        f"{name}.{m} declared in stub but absent at runtime"
        for m in sorted(set(stub) - set(runtime) - {"__init__", "__repr__"})
    ]
    for member in sorted(set(runtime) & set(stub)):
        if member.startswith("__"):
            continue  # protocol parameters are positional; their names are free
        drift += _keyword_drift(f"{name}.{member}", runtime[member], stub[member])
    if "__init__" in stub:
        exempt = frozenset({_FACADE_SEAM_PARAM}) if inner is not None else frozenset()
        drift += _keyword_drift(name, cls, stub["__init__"], exempt)
    elif inner is None:
        drift += _keyword_drift(name, cls, [ast.parse("def __init__(self): ...").body[0]])
    return drift


def test_every_facade_is_mapped_to_the_class_it_forwards_to() -> None:
    """A new ``__getattr__`` facade must join ``_facade_targets``, or the
    members it forwards would escape the diff below."""
    velesdb = pytest.importorskip("velesdb")
    forwarding = {
        name
        for name in velesdb.__all__
        if inspect.isclass(getattr(velesdb, name)) and "__getattr__" in vars(getattr(velesdb, name))
    }
    assert forwarding == set(_facade_targets(velesdb))


def test_stub_declares_every_runtime_member_and_keyword() -> None:
    """Every public method, property and keyword argument of every exported
    class exists in the stub, and the stub declares nothing the runtime lacks."""
    velesdb = pytest.importorskip("velesdb")
    classes = _classes()
    drift: list[str] = []
    for name in velesdb.__all__:
        obj = getattr(velesdb, name)
        if inspect.isclass(obj) and not issubclass(obj, BaseException):
            drift += _class_drift(classes, name, obj, _facade_targets(velesdb).get(name))
    assert drift == [], "type stub drifted from runtime:\n" + "\n".join(drift)


def test_stub_functions_take_the_runtime_keywords() -> None:
    velesdb = pytest.importorskip("velesdb")
    functions = {
        node.name: [node]
        for node in ast.parse(STUB_PATH.read_text()).body
        if isinstance(node, ast.FunctionDef)
    }
    drift: list[str] = []
    for name in velesdb.__all__:
        obj = getattr(velesdb, name)
        if inspect.isroutine(obj) and name in functions:
            drift += _keyword_drift(name, obj, functions[name])
    assert drift == [], "type stub drifted from runtime:\n" + "\n".join(drift)


def test_a_member_reached_only_through_runtime_delegation_is_drift() -> None:
    """The guard reads the stub as a type checker does: a facade that
    forwards a member at runtime but whose stub class neither declares it nor
    inherits it from a stub base is reported, and a stub base declaring it
    settles the drift."""
    runtime_only_delegation = """
class Raw:
    def query(self, q: str) -> None: ...
class Facade:
    def close(self) -> None: ...
"""
    shared_base = """
class _Shared:
    def query(self, q: str) -> None: ...
class Raw(_Shared): ...
class Facade(_Shared):
    def close(self) -> None: ...
"""

    class Raw:
        def query(self, q):
            return None

    class Facade:
        def __getattr__(self, name):
            return getattr(Raw(), name)

        def close(self):
            return None

    assert _class_drift(_classes(runtime_only_delegation), "Facade", Facade, Raw) == [
        "Facade.query missing from stub"
    ]
    assert _class_drift(_classes(shared_base), "Facade", Facade, Raw) == []
    assert _class_drift(_classes(shared_base), "Raw", Raw, None) == []
