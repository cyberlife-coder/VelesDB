"""Tests for the OSS direct-search governance gate.

The Python SDK's direct-search API (``Collection.search`` /
``search_request`` / ``text_search`` / ``hybrid_search`` and their variants)
routes every read through the control-plane read gate (the same
``query_request`` observer hook that VelesQL ``SELECT``/``MATCH`` use). This
module proves three properties for those paths:

* **Deny fails closed** — an observer that vetoes ``query_request`` makes the
  matching search raise, so no results leak (mirrors the core
  ``test_gated_search_deny_fails_closed_with_zero_results``). Before the gate
  the Python ``.search()`` bypassed the observer entirely, so a deny had no
  effect — these tests are the regression guard.
* **Scope narrows** — an observer returning a scope ``dict`` narrows results
  versus the ungated run.
* **Backward compatible** — with no observer, results are unchanged and the
  ``.search()`` deprecation warning still fires (zero-overhead allow).
"""

import shutil
import tempfile
import warnings

import pytest

try:
    from velesdb import Database, SearchOptions

    VELESDB_AVAILABLE = True
except (ImportError, AttributeError):
    VELESDB_AVAILABLE = False
    Database = None  # type: ignore[assignment,misc]
    SearchOptions = None  # type: ignore[assignment,misc]

pytestmark = pytest.mark.skipif(
    not VELESDB_AVAILABLE,
    reason="VelesDB Python bindings not installed. Run: maturin develop",
)

DIM = 4
QUERY = [1.0, 0.0, 0.0, 0.0]

# Search operations emitted by the direct-search gate (per GatedRead variant).
READ_OPS = {"vector_search", "text_search", "hybrid_search"}


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


def _raise_on_read(event, **fields):
    """A buggy policy that raises while deciding a gated read."""
    if event == "query_request":
        raise RuntimeError("policy bug")
    return None


def _return_bad_type(event, **fields):
    """A buggy policy that returns an uninterpretable value (a list)."""
    if event == "query_request":
        return ["not", "a", "decision"]
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
    """Create ``name`` and upsert two tenant-tagged, text-bearing points."""
    collection = db.create_collection(name, dimension=DIM)
    collection.upsert(
        [
            {
                "id": 1,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "payload": {"tenant": "acme", "text": "alpha machine learning"},
            },
            {
                "id": 2,
                "vector": [0.9, 0.1, 0.0, 0.0],
                "payload": {"tenant": "other", "text": "beta machine learning"},
            },
        ]
    )
    return collection


def _ids(results):
    return {r["id"] for r in results}


# ---------------------------------------------------------------------------
# Deny fails closed (mirrors core test_gated_search_deny_fails_closed_*).
# ---------------------------------------------------------------------------


def test_search_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                collection.search(QUERY, top_k=2)


def test_search_request_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=2))


def test_text_search_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.text_search("learning", top_k=2)


def test_hybrid_search_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.hybrid_search(QUERY, "learning", top_k=2)


def test_batch_search_deny_fails_closed():
    """A non-GatedRead path (batch) is gated via authorize_read → deny raises."""
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.batch_search([{"vector": QUERY, "top_k": 2}])


def test_search_ids_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_ids(QUERY, top_k=2)


# ---------------------------------------------------------------------------
# Strict (fail-closed) mode, audit F-5.3: a policy *error* denies under strict
# and allows (fail-open) by default. Explicit decisions are unaffected.
# ---------------------------------------------------------------------------


def test_policy_exception_fails_open_by_default():
    """A raising policy must NOT break the read in the default (fail-open) mode."""
    with _Db(observer=_raise_on_read) as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=2))
        assert _ids(results) == {1, 2}


def test_policy_exception_fails_closed_under_strict():
    """The same raising policy denies the read when observer_strict=True."""
    with _Db(observer=_raise_on_read, observer_strict=True) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=2))


def test_bad_return_type_fails_open_by_default():
    """An uninterpretable return value is treated as allow by default."""
    with _Db(observer=_return_bad_type) as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=2))
        assert _ids(results) == {1, 2}


def test_bad_return_type_fails_closed_under_strict():
    """An uninterpretable return value denies the read under strict mode."""
    with _Db(observer=_return_bad_type, observer_strict=True) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=2))


def test_explicit_allow_unaffected_by_strict():
    """None/allow from the policy still allows even under strict mode."""
    with _Db(observer=_scope_to_acme, observer_strict=True) as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
        # Scope still narrows to tenant 'acme' (id 1), proving strict didn't
        # turn an explicit allow-with-scope into a denial.
        assert _ids(results) == {1}


# ---------------------------------------------------------------------------
# Scope narrows results versus the ungated run.
# ---------------------------------------------------------------------------


def test_search_scope_narrows_results():
    # Ungated: both points are returned.
    with _Db() as db:
        collection = _seed(db)
        ungated = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
    assert _ids(ungated) == {1, 2}

    # Scoped to tenant == 'acme': only point 1 survives.
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
    assert _ids(scoped) == {1}


def test_search_with_filter_scope_narrows_results():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        # Caller filter is AND-composed with the observer scope; both allow id 1.
        scoped = collection.search_with_filter(
            QUERY,
            top_k=10,
            filter={"condition": {"type": "eq", "field": "tenant", "value": "acme"}},
        )
    assert _ids(scoped) == {1}


def test_text_search_scope_narrows_results():
    with _Db() as db:
        collection = _seed(db)
        ungated = collection.text_search("learning", top_k=10)
    assert _ids(ungated) == {1, 2}

    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.text_search("learning", top_k=10)
    assert _ids(scoped) == {1}


def test_batch_search_scope_narrows_results():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.batch_search([{"vector": QUERY, "top_k": 10}])
    assert len(scoped) == 1
    assert _ids(scoped[0]) == {1}


def test_search_ids_scope_fails_closed():
    """IDs-only results carry no payload, so a scope filter cannot be applied:
    the read must fail closed rather than return unscoped ids."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_ids(QUERY, top_k=10)


def test_malformed_scope_filter_denies():
    """A scope dict whose VelesQL predicate does not parse fails closed."""

    def bad_scope(event, **fields):
        if event == "query_request":
            return {"filter": "this is not valid velesql !!!"}
        return None

    with _Db(observer=bad_scope) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=10))


# ---------------------------------------------------------------------------
# Fail closed on a scope dict that carries no enforceable filter. A dict MUST
# yield a real predicate; OSS does not narrow by "tenant" alone, so an
# empty/tenant-only dict must NOT masquerade as scoping.
# ---------------------------------------------------------------------------


def test_empty_scope_dict_denies():
    """An empty ``{}`` scope dict fails closed, not allow-unscoped."""

    def empty_scope(event, **fields):
        if event == "query_request":
            return {}
        return None

    with _Db(observer=empty_scope) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=10))


def test_tenant_only_scope_dict_denies():
    """A ``{"tenant": ...}``-only dict fails closed (OSS ignores tenant)."""

    def tenant_only(event, **fields):
        if event == "query_request":
            return {"tenant": "acme"}
        return None

    with _Db(observer=tenant_only) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(SearchOptions(vector=QUERY, top_k=10))


def test_tenant_plus_filter_scope_narrows():
    """A dict with both tenant hint and a real filter still narrows correctly."""

    def scoped(event, **fields):
        if event == "query_request":
            return {"tenant": "acme", "filter": "tenant = 'acme'"}
        return None

    with _Db(observer=scoped) as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
    assert _ids(results) == {1}


# ---------------------------------------------------------------------------
# Additional non-GatedRead coverage: sparse deny, and dense-backed parallel /
# multi-query deny + scope (multi_query_search_ids fails closed on scope).
# ---------------------------------------------------------------------------


def test_sparse_search_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_request(
                SearchOptions(sparse_vector={0: 1.0}, top_k=2)
            )


def test_search_batch_parallel_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.search_batch_parallel([QUERY], top_k=2)


def test_search_batch_parallel_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.search_batch_parallel([QUERY], top_k=10)
    assert len(scoped) == 1
    assert _ids(scoped[0]) == {1}


def test_multi_query_search_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.multi_query_search([QUERY], top_k=2)


def test_multi_query_search_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        scoped = collection.multi_query_search([QUERY], top_k=10)
    assert _ids(scoped) == {1}


def test_multi_query_search_ids_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.multi_query_search_ids([QUERY], top_k=2)


def test_multi_query_search_ids_scope_fails_closed():
    """IDs-only fused results carry no payload → scope fails closed."""
    with _Db(observer=_scope_to_acme) as db:
        collection = _seed(db)
        with pytest.raises(Exception):
            collection.multi_query_search_ids([QUERY], top_k=10)


# ---------------------------------------------------------------------------
# Graph collection: PyGraphCollection.search_by_embedding is now gated too.
# ---------------------------------------------------------------------------


def _seed_graph(db, name="kg"):
    graph = db.create_graph_collection(name, dimension=DIM)
    graph.upsert_node(1, {"tenant": "acme"}, [1.0, 0.0, 0.0, 0.0])
    graph.upsert_node(2, {"tenant": "other"}, [0.9, 0.1, 0.0, 0.0])
    return graph


def test_graph_search_by_embedding_no_observer_unchanged():
    with _Db() as db:
        graph = _seed_graph(db)
        results = graph.search_by_embedding(QUERY, 10)
    assert _ids(results) == {1, 2}


def test_graph_search_by_embedding_deny_fails_closed():
    with _Db(observer=_deny_reads) as db:
        graph = _seed_graph(db)
        with pytest.raises(Exception):
            graph.search_by_embedding(QUERY, 10)


def test_graph_search_by_embedding_scope_narrows():
    with _Db(observer=_scope_to_acme) as db:
        graph = _seed_graph(db)
        scoped = graph.search_by_embedding(QUERY, 10)
    assert _ids(scoped) == {1}


# ---------------------------------------------------------------------------
# Backward compatibility: no observer ⇒ unchanged results + deprecation warning.
# ---------------------------------------------------------------------------


def test_no_observer_returns_all_results():
    with _Db() as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
    assert _ids(results) == {1, 2}
    # Nearest neighbour to QUERY is point 1 (exact match).
    assert results[0]["id"] == 1


def test_deprecated_search_still_warns_and_works():
    with _Db() as db:
        collection = _seed(db)
        with pytest.warns(DeprecationWarning):
            results = collection.search(QUERY, top_k=10)
    assert _ids(results) == {1, 2}


def test_allow_observer_permits_read():
    def notify_only(event, **fields):
        return None

    with _Db(observer=notify_only) as db:
        collection = _seed(db)
        results = collection.search_request(SearchOptions(vector=QUERY, top_k=10))
    assert _ids(results) == {1, 2}


def test_query_request_fields_carry_search_operation():
    """The veto callback sees the correct operation label per search kind."""
    seen = []

    def observe(event, **fields):
        if event == "query_request":
            seen.append(fields["operation"])
        return None

    with _Db(observer=observe) as db:
        collection = _seed(db)
        collection.search_request(SearchOptions(vector=QUERY, top_k=2))
        collection.text_search("learning", top_k=2)
        collection.hybrid_search(QUERY, "learning", top_k=2)

    assert READ_OPS.issubset(set(seen))
