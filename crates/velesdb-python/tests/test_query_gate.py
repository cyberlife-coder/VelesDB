"""Tests for the VelesQL / point-read governance gate (issues #1405, #1392).

``test_search_gate.py`` proves the *search* surface honors the control-plane
read gate. This module proves the rest of the Python read surface does too:

* ``Collection.query`` / ``query_ids`` / ``explain_analyze`` — routed through
  ``Database::execute_query`` / ``explain_analyze_query`` (the gated facade),
  not the detached collection leaf (audit F-5.4 / #1392: the leaf has no
  observer reference, so a deny was silently ignored).
* ``Collection.match_query`` — no gated twin returns ``MatchResult``, so the
  binding consults ``authorize_read`` itself (deny ⇒ raise; scope ⇒ fail
  closed, the MATCH leaf takes no metadata filter).
* ``Collection.scroll`` / ``scroll_batch`` / ``get`` — payload-shaped reads:
  deny ⇒ raise, scope ⇒ narrow (scroll AND-composes the scope filter, ``get``
  masks out-of-scope points to ``None``).
* ``Collection.all_ids`` / ``__contains__`` — id-only shapes: deny ⇒ raise,
  scope ⇒ fail closed (no payload to filter on).
* ``PyGraphCollection.query`` / ``match_query`` / ``query_ids`` /
  ``explain_analyze`` — same mechanisms.
* ``explain()`` stays ungated by design: it builds a plan and reads no data.
* With no observer registered, every path behaves exactly as before
  (zero-overhead allow).
"""

import shutil
import tempfile

import pytest

try:
    from velesdb import Database

    VELESDB_AVAILABLE = True
except (ImportError, AttributeError):
    VELESDB_AVAILABLE = False
    Database = None  # type: ignore[assignment,misc]

pytestmark = pytest.mark.skipif(
    not VELESDB_AVAILABLE,
    reason="VelesDB Python bindings not installed. Run: maturin develop",
)

DIM = 4
QUERY = [1.0, 0.0, 0.0, 0.0]


def _deny_reads(event, **fields):
    """Veto every gated *read*; allow lifecycle/notify events through."""
    if event == "query_request":
        return False
    return None


def _scope_to_acme(event, **fields):
    """Allow reads but narrow them to ``tenant == 'acme'``."""
    if event == "query_request":
        return {"filter": "tenant = 'acme'"}
    return None


class _Db:
    """Context-managed temp database, optionally with an observer."""

    def __init__(self, observer=None, observer_strict=False):
        self.observer = observer
        self.observer_strict = observer_strict

    def __enter__(self):
        self.dir = tempfile.mkdtemp()
        self.db = Database(
            self.dir,
            observer=self.observer,
            observer_strict=self.observer_strict,
        )
        return self.db

    def __exit__(self, *exc):
        shutil.rmtree(self.dir, ignore_errors=True)


def _seed(db, name="docs"):
    """Create ``name`` and upsert two tenant-tagged points."""
    collection = db.create_collection(name, dimension=DIM)
    collection.upsert(
        [
            {
                "id": 1,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "payload": {"tenant": "acme", "text": "alpha"},
            },
            {
                "id": 2,
                "vector": [0.9, 0.1, 0.0, 0.0],
                "payload": {"tenant": "other", "text": "beta"},
            },
        ]
    )
    return collection


def _seed_graph(db, name="kg"):
    """Create a graph collection with two tenant-tagged nodes and one edge."""
    graph = db.create_graph_collection(name, dimension=DIM)
    graph.upsert_node(1, {"tenant": "acme", "_labels": ["Doc"]}, [1.0, 0.0, 0.0, 0.0])
    graph.upsert_node(2, {"tenant": "other", "_labels": ["Doc"]}, [0.9, 0.1, 0.0, 0.0])
    graph.add_edge({"id": 1, "source": 1, "target": 2, "label": "REL"})
    return graph


def _ids(results):
    return {r["id"] for r in results}


# ---------------------------------------------------------------------------
# Deny fails closed — Collection VelesQL surface.
# Regression guard for #1392: before the fix these paths hit the detached
# collection leaf, so a deny observer was silently bypassed.
# ---------------------------------------------------------------------------


def test_query_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.query("SELECT * FROM docs LIMIT 10")


def test_query_ids_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.query_ids("SELECT * FROM docs LIMIT 10")


def test_match_query_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.match_query("MATCH (n) RETURN n LIMIT 10")


def test_match_query_with_vector_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.match_query(
                "MATCH (n) RETURN n LIMIT 10", vector=QUERY, threshold=0.1
            )


def test_explain_analyze_deny_fails_closed():
    """EXPLAIN ANALYZE *executes* the query, so it must be gated like one."""
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.explain_analyze("SELECT * FROM docs LIMIT 10")


def test_explain_stays_ungated_plan_only():
    """``explain()`` builds a plan without reading data — allowed by design."""
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        plan = collection.explain("SELECT * FROM docs LIMIT 10")
    assert "tree" in plan


# ---------------------------------------------------------------------------
# Deny fails closed — point-read surface (scroll / get / all_ids / contains).
# ---------------------------------------------------------------------------


def test_scroll_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            list(collection.scroll(batch_size=10))


def test_scroll_batch_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.scroll_batch(batch_size=10)


def test_get_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.get([1, 2])


def test_all_ids_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.all_ids()


def test_contains_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            1 in collection


# ---------------------------------------------------------------------------
# Deny fails closed — PyGraphCollection VelesQL surface.
# ---------------------------------------------------------------------------


def test_graph_query_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.query("SELECT * FROM kg LIMIT 10")


def test_graph_match_query_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.match_query("MATCH (a)-[:REL]->(b) RETURN a, b LIMIT 10")


def test_graph_query_ids_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.query_ids("SELECT * FROM kg LIMIT 10")


def test_graph_explain_analyze_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.explain_analyze("SELECT * FROM kg LIMIT 10")


# ---------------------------------------------------------------------------
# Scope narrows (payload shapes) or fails closed (shapes that cannot carry a
# metadata filter).
# ---------------------------------------------------------------------------


def test_query_scope_narrows():
    with _Db() as db:
        collection = _seed(db)
        ungated = collection.query("SELECT * FROM docs LIMIT 10")
    assert {r["node_id"] for r in ungated} == {1, 2}

    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.query("SELECT * FROM docs LIMIT 10")
    assert {r["node_id"] for r in scoped} == {1}


def test_query_ids_scope_narrows():
    """The scope filter is AND-composed into the query AST in core, so the
    ids returned are already narrowed — no unscoped id can leak."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.query_ids("SELECT * FROM docs LIMIT 10")
    assert _ids(scoped) == {1}


def test_match_query_scope_fails_closed():
    """The MATCH leaf takes no metadata filter and ``MatchResult`` has no
    payload to post-filter, so a scoped read must fail closed."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.match_query("MATCH (n) RETURN n LIMIT 10")


def test_explain_analyze_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        out = collection.explain_analyze("SELECT * FROM docs LIMIT 10")
    assert out["actual_stats"]["actual_rows"] == 1


def test_scroll_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        batches = list(collection.scroll(batch_size=10))
    points = [p for batch in batches for p in batch]
    assert {p["id"] for p in points} == {1}


def test_scroll_batch_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        points, _cursor = collection.scroll_batch(batch_size=10)
    assert {p["id"] for p in points} == {1}


def test_scroll_scope_composes_with_caller_filter():
    """Caller filter AND observer scope: a caller filter matching the
    out-of-scope tenant must yield nothing."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        points, _cursor = collection.scroll_batch(
            batch_size=10,
            filter={"condition": {"type": "eq", "field": "tenant", "value": "other"}},
        )
    assert points == []


def test_get_scope_masks_out_of_scope_points():
    """``get`` keeps id-alignment: out-of-scope points come back ``None``."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        got = collection.get([1, 2])
    assert got[0] is not None and got[0]["id"] == 1
    assert got[1] is None


def test_all_ids_scope_fails_closed():
    """Ids-only shape cannot carry a scope filter — fail closed."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.all_ids()


def test_contains_scope_fails_closed():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            1 in collection


# ---------------------------------------------------------------------------
# No observer ⇒ strictly unchanged behavior (zero-overhead allow contract).
# ---------------------------------------------------------------------------


def test_query_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        results = collection.query("SELECT * FROM docs LIMIT 10")
    assert {r["node_id"] for r in results} == {1, 2}


def test_query_ids_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        results = collection.query_ids("SELECT * FROM docs LIMIT 10")
    assert _ids(results) == {1, 2}


def test_match_query_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        results = collection.match_query("MATCH (n) RETURN n LIMIT 10")
    assert {r["node_id"] for r in results} == {1, 2}


def test_explain_analyze_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        out = collection.explain_analyze("SELECT * FROM docs LIMIT 10")
    assert out["actual_stats"]["actual_rows"] == 2
    assert "plan" in out


def test_scroll_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        batches = list(collection.scroll(batch_size=1))
    points = [p for batch in batches for p in batch]
    assert {p["id"] for p in points} == {1, 2}


def test_point_reads_no_observer_unchanged():
    with _Db() as db:
        collection = _seed(db)
        got = collection.get([1, 2, 99])
        assert got[0]["id"] == 1 and got[1]["id"] == 2 and got[2] is None
        assert sorted(collection.all_ids()) == [1, 2]
        assert 1 in collection
        assert 99 not in collection


def test_graph_no_observer_unchanged():
    with _Db() as db:
        graph = _seed_graph(db)
        rows = graph.query("SELECT * FROM kg LIMIT 10")
        assert len(rows) == 2
        matches = graph.match_query("MATCH (a)-[:REL]->(b) RETURN a, b LIMIT 10")
        assert len(matches) > 0
        ids = graph.query_ids("SELECT * FROM kg LIMIT 10")
        assert _ids(ids) == {1, 2}
        out = graph.explain_analyze("SELECT * FROM kg LIMIT 10")
        assert "plan" in out


def test_graph_match_query_scope_fails_closed():
    with _Db(observer=_scope_to_acme) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.match_query("MATCH (n) RETURN n LIMIT 10")


# ---------------------------------------------------------------------------
# FROM is nominal: the collection-level VelesQL methods always execute against
# the wrapped collection, whatever the FROM clause names. The detached leaf
# historically ignored FROM, and the LangChain / LlamaIndex adapters rely on
# it (e.g. ``SELECT * FROM vectors`` on a collection with another name), so
# gated routing must preserve this contract — with and without an observer —
# and key the gate on the collection actually read.
# ---------------------------------------------------------------------------


def test_query_from_name_is_nominal():
    with _Db() as db:
        collection = _seed(db)  # collection is named "docs"
        results = collection.query("SELECT * FROM vectors LIMIT 10")
    assert {r["node_id"] for r in results} == {1, 2}


def test_query_ids_from_name_is_nominal():
    with _Db() as db:
        collection = _seed(db)
        results = collection.query_ids("SELECT * FROM vectors LIMIT 10")
    assert _ids(results) == {1, 2}


def test_explain_analyze_from_name_is_nominal():
    with _Db() as db:
        collection = _seed(db)
        out = collection.explain_analyze("SELECT * FROM vectors LIMIT 10")
    assert out["actual_stats"]["actual_rows"] == 2


def test_query_compound_operands_execute_on_wrapped_collection():
    """UNION operands also carried nominal FROM names on the leaf."""
    with _Db() as db:
        collection = _seed(db)
        results = collection.query(
            "SELECT * FROM a WHERE tenant = 'acme' "
            "UNION SELECT * FROM b WHERE tenant = 'other'"
        )
    assert {r["node_id"] for r in results} == {1, 2}


def test_graph_query_from_name_is_nominal():
    with _Db() as db:
        graph = _seed_graph(db)
        rows = graph.query("SELECT * FROM vectors LIMIT 10")
    assert len(rows) == 2


def test_query_deny_fails_closed_with_nominal_from():
    """The gate is keyed on the wrapped collection (the one actually read),
    not on whatever name the FROM clause carries."""
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.query("SELECT * FROM vectors LIMIT 10")


def test_query_scope_narrows_with_nominal_from():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.query("SELECT * FROM vectors LIMIT 10")
    assert {r["node_id"] for r in scoped} == {1}
