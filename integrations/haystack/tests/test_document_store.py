"""Unit tests for VelesDBDocumentStore.

All external dependencies (haystack, velesdb) are replaced with lightweight
stubs so no server or framework install is required to run the suite.
"""
from __future__ import annotations

import importlib.util
import sys
import types
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any, Dict, List, Optional

# ---------------------------------------------------------------------------
# Haystack 2.x stubs — mirror the public API surface used by document_store.py
# ---------------------------------------------------------------------------


@dataclass
class Document:
    id: str = ""
    content: Optional[str] = None
    embedding: Optional[List[float]] = None
    meta: Dict[str, Any] = field(default_factory=dict)
    score: Optional[float] = None


class DuplicatePolicy(Enum):
    NONE = "none"
    SKIP = "skip"
    OVERWRITE = "overwrite"
    FAIL = "fail"


class DuplicateDocumentError(Exception):
    pass


# ---------------------------------------------------------------------------
# Fake VelesDB objects — deterministic, no I/O
# ---------------------------------------------------------------------------


class _FakeFusionStrategy:
    """Minimal stand-in for velesdb.FusionStrategy.

    Records the strategy name so a fake ``multi_query_search`` can vary its
    result ordering by strategy (mirrors the real binding, where different
    strategies produce different fused scores).
    """

    def __init__(self, name: str, params: Optional[dict] = None) -> None:
        self.name = name
        self.params = params or {}

    @staticmethod
    def average() -> "_FakeFusionStrategy":
        return _FakeFusionStrategy("average")

    @staticmethod
    def maximum() -> "_FakeFusionStrategy":
        return _FakeFusionStrategy("maximum")

    @staticmethod
    def rrf(k: int = 60) -> "_FakeFusionStrategy":
        return _FakeFusionStrategy("rrf", {"k": k})

    @staticmethod
    def weighted(
        avg_weight: float = 0.6,
        max_weight: float = 0.3,
        hit_weight: float = 0.1,
    ) -> "_FakeFusionStrategy":
        return _FakeFusionStrategy(
            "weighted",
            {
                "avg_weight": avg_weight,
                "max_weight": max_weight,
                "hit_weight": hit_weight,
            },
        )

    @staticmethod
    def relative_score(
        dense_weight: float, sparse_weight: float
    ) -> "_FakeFusionStrategy":
        return _FakeFusionStrategy(
            "relative_score",
            {"dense_weight": dense_weight, "sparse_weight": sparse_weight},
        )


def _build_fake_fusion(
    fusion: str, fusion_params: Optional[dict] = None
) -> _FakeFusionStrategy:
    """Stand-in for velesdb_common.fusion.build_fusion_strategy.

    Maps the strategy name to the matching FusionStrategy factory so the
    document store's fusion routing can be exercised without the real
    velesdb_common package.
    """
    params = fusion_params or {}
    if fusion in ("relative_score", "rsf"):
        return _FakeFusionStrategy.relative_score(
            params.get("dense_weight", 0.5), params.get("sparse_weight", 0.5)
        )
    if fusion == "weighted":
        return _FakeFusionStrategy.weighted()
    if fusion == "maximum":
        return _FakeFusionStrategy.maximum()
    if fusion == "average":
        return _FakeFusionStrategy.average()
    return _FakeFusionStrategy.rrf(params.get("k", 60))


class _FakeSearchOptions:
    """Minimal stand-in for velesdb.SearchOptions used by search_request."""

    def __init__(
        self,
        vector: Any = None,
        *,
        sparse_vector: Any = None,
        top_k: int = 10,
        filter: Any = None,  # pylint: disable=redefined-builtin
        sparse_index_name: Any = None,
        include_vectors: bool = False,
    ) -> None:
        self.vector = vector
        self.sparse_vector = sparse_vector
        self.top_k = top_k
        self.filter = filter
        self.sparse_index_name = sparse_index_name
        self.include_vectors = include_vectors


class _FakeCollection:
    def __init__(self) -> None:
        self._points: dict = {}  # int_id -> point dict

    def upsert(self, points: list) -> int:
        for p in points:
            self._points[p["id"]] = p
        return len(points)

    def get(self, int_ids: list) -> list:
        return [
            {"id": iid, "payload": self._points[iid].get("payload", {})}
            if iid in self._points else None
            for iid in int_ids
        ]

    # `filter=` mirrors the public velesdb SDK kwarg name on Collection.search /
    # Collection.scroll; renaming it would break the kwargs contract under test.
    def search(  # pylint: disable=redefined-builtin
        self, vector: list, top_k: int = 10, filter: Any = None
    ) -> list:
        del vector, filter  # the fake ignores these
        return [
            {"id": p["id"], "score": 0.9, "payload": p.get("payload", {})}
            for p in list(self._points.values())[:top_k]
        ]

    def search_request(self, opts: Any) -> list:
        """Canonical search entry point — delegate to the legacy `search`."""
        return self.search(opts.vector, top_k=opts.top_k, filter=opts.filter)

    def multi_query_search(
        self,
        vectors: list,
        top_k: int = 10,
        fusion: Any = None,
        filter: Any = None,  # pylint: disable=redefined-builtin
    ) -> list:
        """Fused multi-query search whose ordering depends on the strategy.

        The real binding produces strategy-dependent fused scores. This fake
        reproduces that observable behaviour: the points are sorted by a
        per-strategy key so callers can assert that ``fusion='rsf'`` and
        ``fusion='weighted'`` yield different orderings.
        """
        del vectors, filter  # the fake ignores these
        points = list(self._points.values())
        name = getattr(fusion, "name", "rrf")
        # Reverse the order for relative_score so the resulting ranking
        # differs from the default (rrf) and from weighted.
        reverse = name in ("relative_score", "rsf")
        ordered = points[::-1] if reverse else points
        results = [
            {"id": p["id"], "score": 0.9, "payload": p.get("payload", {})}
            for p in ordered[:top_k]
        ]
        return results

    def scroll(  # pylint: disable=redefined-builtin
        self,
        *,
        batch_size: int = 100,
        filter: Any = None,
        as_dataframe: bool = False,
        backend: str = "pandas",
    ) -> Any:
        """Match the real velesdb SDK signature: kwargs-only, returns
        Iterator[List[Dict]]. The real SDK has no ``limit`` kwarg — callers
        drive the iterator and stop themselves.
        """
        del filter, as_dataframe, backend  # the fake ignores these
        all_points = [
            {"id": p["id"], "score": None, "payload": p.get("payload", {})}
            for p in self._points.values()
        ]
        for offset in range(0, len(all_points), batch_size):
            yield all_points[offset : offset + batch_size]

    def delete(self, int_ids: list) -> None:
        for iid in int_ids:
            self._points.pop(iid, None)

    def count(self) -> int:
        return len(self._points)


class _FakeDatabase:
    def __init__(self, path: str) -> None:
        self._collections: dict = {}

    def get_collection(self, name: str) -> _FakeCollection:
        if name not in self._collections:
            raise KeyError(name)
        return self._collections[name]

    def create_collection(
        self, name: str, dimension: int, metric: str
    ) -> _FakeCollection:
        col = _FakeCollection()
        self._collections[name] = col
        return col


# ---------------------------------------------------------------------------
# Module loader — inject stubs, load document_store from source
# ---------------------------------------------------------------------------


def _load_module() -> types.ModuleType:
    root = Path(__file__).resolve().parents[1] / "src" / "haystack_velesdb"

    haystack_pkg = types.ModuleType("haystack")
    haystack_pkg.default_to_dict = lambda obj, **kw: {  # type: ignore[attr-defined]
        "type": type(obj).__name__,
        "init_parameters": kw,
    }
    haystack_pkg.default_from_dict = lambda cls, d: cls(  # type: ignore[attr-defined]
        **d.get("init_parameters", {})
    )
    sys.modules["haystack"] = haystack_pkg

    dc_mod = types.ModuleType("haystack.dataclasses")
    dc_mod.Document = Document  # type: ignore[attr-defined]
    sys.modules["haystack.dataclasses"] = dc_mod

    ds_pkg = types.ModuleType("haystack.document_stores")
    sys.modules["haystack.document_stores"] = ds_pkg
    types_mod = types.ModuleType("haystack.document_stores.types")
    types_mod.DuplicatePolicy = DuplicatePolicy  # type: ignore[attr-defined]
    sys.modules["haystack.document_stores.types"] = types_mod
    errors_mod = types.ModuleType("haystack.document_stores.errors")
    errors_mod.DuplicateDocumentError = DuplicateDocumentError  # type: ignore[attr-defined]
    sys.modules["haystack.document_stores.errors"] = errors_mod

    sys.modules["velesdb"] = types.SimpleNamespace(  # type: ignore
        Database=_FakeDatabase,
        SearchOptions=_FakeSearchOptions,
        FusionStrategy=_FakeFusionStrategy,
    )

    # Stub velesdb_common.security with no-op validators (real package has its own tests).
    def _passthrough(value: Any, *args: Any, **kwargs: Any) -> Any:
        return value

    vc_mod = types.ModuleType("velesdb_common")
    sys.modules["velesdb_common"] = vc_mod
    vc_sec = types.ModuleType("velesdb_common.security")
    vc_sec.validate_path = _passthrough  # type: ignore[attr-defined]
    vc_sec.validate_collection_name = _passthrough  # type: ignore[attr-defined]
    vc_sec.validate_metric = _passthrough  # type: ignore[attr-defined]
    vc_sec.validate_named_sparse_vector = _passthrough  # type: ignore[attr-defined]
    vc_sec.SecurityError = ValueError  # type: ignore[attr-defined]
    sys.modules["velesdb_common.security"] = vc_sec

    vc_fusion = types.ModuleType("velesdb_common.fusion")
    vc_fusion.build_fusion_strategy = _build_fake_fusion  # type: ignore[attr-defined]
    sys.modules["velesdb_common.fusion"] = vc_fusion

    pkg = types.ModuleType("haystack_velesdb")
    pkg.__path__ = [str(root)]  # type: ignore[attr-defined]
    sys.modules["haystack_velesdb"] = pkg

    spec = importlib.util.spec_from_file_location(
        "haystack_velesdb.document_store", root / "document_store.py"
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["haystack_velesdb.document_store"] = mod
    spec.loader.exec_module(mod)  # type: ignore[union-attr]
    return mod


_MOD = _load_module()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_write_and_count() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_write")
    docs = [
        Document(id="a", content="alpha", embedding=[0.1, 0.2, 0.3]),
        Document(id="b", content="beta", embedding=[0.4, 0.5, 0.6]),
    ]
    assert store.write_documents(docs) == 2
    assert store.count_documents() == 2


def test_write_empty_returns_zero() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_empty")
    assert store.write_documents([]) == 0


def test_embedding_retrieval_returns_documents() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_retrieval")
    store.write_documents([Document(id="x", content="hello", embedding=[0.1, 0.2, 0.3])])
    results = store.embedding_retrieval([0.1, 0.2, 0.3], top_k=5)
    assert len(results) >= 1
    assert results[0].id == "x"
    assert results[0].content == "hello"


def test_scale_score_normalises_cosine() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_score")
    store.write_documents([Document(id="y", content="world", embedding=[1.0, 0.0])])
    scaled = store.embedding_retrieval([1.0, 0.0], scale_score=True)
    raw = store.embedding_retrieval([1.0, 0.0], scale_score=False)
    assert scaled[0].score == (0.9 + 1.0) / 2.0
    assert raw[0].score == 0.9


def test_filter_documents_returns_all_when_none() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_filter")
    store.write_documents([
        Document(id="p", content="foo", embedding=[0.1, 0.2]),
        Document(id="q", content="bar", embedding=[0.7, 0.8]),
    ])
    assert len(store.filter_documents()) == 2


def test_filter_documents_passes_filter_to_scroll() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_filter_arg")
    store.write_documents([
        Document(id="fa", content="alpha", embedding=[0.1]),
    ])
    # A real Haystack 2.x filter shape — the fake scroll ignores the
    # translated VelesDB filter, but this exercises the translator end-to-end.
    results = store.filter_documents(
        filters={"field": "meta.source", "operator": "==", "value": "wiki"}
    )
    assert len(results) == 1


def test_scale_score_not_applied_for_non_cosine_metric() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_score_nc", metric="euclidean")
    store.write_documents([Document(id="z", content="raw", embedding=[1.0])])
    scaled = store.embedding_retrieval([1.0], scale_score=True)
    # For euclidean metric scale_score should be a no-op — raw score returned.
    assert scaled[0].score == 0.9


def test_scroll_limit_is_respected() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_limit", scroll_limit=1)
    store.write_documents([
        Document(id="r", content="one", embedding=[0.1]),
        Document(id="s", content="two", embedding=[0.2]),
    ])
    # With scroll_limit=1 the fake scroll caps at 1 result.
    assert len(store.filter_documents()) == 1


def test_delete_documents() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_delete")
    store.write_documents([
        Document(id="del1", content="remove me", embedding=[0.1, 0.2]),
        Document(id="keep1", content="keep me", embedding=[0.3, 0.4]),
    ])
    assert store.count_documents() == 2
    store.delete_documents(["del1"])
    assert store.count_documents() == 1
    remaining = store.filter_documents()
    assert remaining[0].id == "keep1"


def test_document_metadata_round_trips() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_meta")
    store.write_documents([
        Document(id="m1", content="meta test", embedding=[0.5], meta={"source": "wiki"})
    ])
    docs = store.filter_documents()
    assert docs[0].meta.get("source") == "wiki"


def test_reserved_meta_keys_do_not_corrupt_payload() -> None:
    """doc.meta containing reserved keys must not overwrite canonical fields."""
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_reserved")
    # A user accidentally sets meta keys that clash with our reserved names.
    store.write_documents([
        Document(
            id="safe",
            content="real content",
            embedding=[0.1],
            meta={"_doc_id": "evil_id", "content": "evil content"},
        )
    ])
    docs = store.filter_documents()
    assert docs[0].id == "safe", "_doc_id must come from doc.id, not meta"
    assert docs[0].content == "real content", "content must come from doc.content, not meta"
    # Reserved keys should not leak back into meta on retrieval.
    assert "_doc_id" not in docs[0].meta
    assert "content" not in docs[0].meta


def test_get_collection_catches_key_error_and_creates_collection() -> None:
    """_get_collection catches KeyError from get_collection and falls back to create_collection."""
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_key_error_path")
    # The fake raises KeyError for unknown collections; _get_collection should
    # catch it and call create_collection instead of letting the error propagate.
    assert store.count_documents() == 0
    assert store._collection is not None


def test_write_documents_skip_policy_does_not_overwrite_existing() -> None:
    """Regression: DuplicatePolicy.SKIP must leave existing documents untouched
    and only write the genuinely-new subset. Earlier behaviour (v1.14.0/v1.14.1)
    silently fell through to col.upsert(...) for SKIP, violating the Haystack
    DocumentStore contract.
    """
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_skip")
    original = Document(id="dup", content="original content", embedding=[0.1, 0.2])
    store.write_documents([original])
    # Re-write with SKIP — the new document has different content/embedding,
    # but the contract says SKIP should NOT overwrite the existing one.
    new = Document(id="dup", content="REPLACED CONTENT", embedding=[0.9, 0.9])
    fresh = Document(id="brand_new", content="fresh", embedding=[0.5, 0.5])
    written = store.write_documents([new, fresh], policy=DuplicatePolicy.SKIP)
    assert written == 1, "SKIP should report only the genuinely-new doc as written"
    assert store.count_documents() == 2, "collection should now have 2 docs"
    docs = {d.id: d for d in store.filter_documents()}
    assert docs["dup"].content == "original content", \
        "SKIP must NOT overwrite the existing 'dup' document"
    assert docs["brand_new"].content == "fresh", \
        "SKIP must still upsert the genuinely-new 'brand_new' document"


def test_write_documents_skip_policy_all_existing_returns_zero() -> None:
    """Regression: when every incoming document is already in the store, SKIP
    must return 0 and not call col.upsert at all (no spurious side-effects).
    """
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_skip_all")
    docs = [Document(id="a", content="1", embedding=[0.1]),
            Document(id="b", content="2", embedding=[0.2])]
    store.write_documents(docs)
    written = store.write_documents(docs, policy=DuplicatePolicy.SKIP)
    assert written == 0, "SKIP with all-existing batch must report 0 written"


def test_write_documents_fail_policy_raises_on_duplicate() -> None:
    """DuplicatePolicy.FAIL raises DuplicateDocumentError when a document already exists."""
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_fail_dup")
    doc = Document(id="dup1", content="original", embedding=[0.1, 0.2])
    store.write_documents([doc])

    import pytest
    with pytest.raises(DuplicateDocumentError):
        store.write_documents([doc], policy=DuplicatePolicy.FAIL)


def test_write_documents_fail_policy_succeeds_for_new_docs() -> None:
    """DuplicatePolicy.FAIL succeeds when none of the documents already exist."""
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_fail_new")
    doc = Document(id="new_only", content="fresh", embedding=[0.5])
    result = store.write_documents([doc], policy=DuplicatePolicy.FAIL)
    assert result == 1
    assert store.count_documents() == 1


def test_serialisation_round_trip() -> None:
    store = _MOD.VelesDBDocumentStore(
        path="/tmp/hs_serial",
        collection_name="serial",
        embedding_dim=384,
        metric="euclidean",
        scroll_limit=5_000,
    )
    d = store.to_dict()
    assert d["init_parameters"]["embedding_dim"] == 384
    assert d["init_parameters"]["metric"] == "euclidean"
    assert d["init_parameters"]["scroll_limit"] == 5_000
    restored = _MOD.VelesDBDocumentStore.from_dict(d)
    assert restored._embedding_dim == 384
    assert restored._metric == "euclidean"
    assert restored._scroll_limit == 5_000


def test_filter_documents_drives_scroll_iterator_across_batches() -> None:
    """Regression: filter_documents must drive the Iterator returned by
    Collection.scroll() (the real SDK returns Iterator[List[Dict]], it does
    not return a flat list nor accept a 'limit' kwarg). With batch_size=100
    in the fake, a 2-document collection yields a single 2-element batch,
    and the helper must collect both.
    """
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_iter_drive")
    store.write_documents(
        [
            Document(id="i1", content="one", embedding=[0.1]),
            Document(id="i2", content="two", embedding=[0.2]),
        ]
    )
    docs = store.filter_documents()
    assert {d.id for d in docs} == {"i1", "i2"}


def test_result_to_doc_raises_on_missing_doc_id() -> None:
    """Regression: a VelesDB point with no `_doc_id` payload key must raise
    rather than silently fall back to str(int_id). The previous fallback
    corrupted delete_documents() because str(int_id) re-hashes via SHA-256
    to a different integer, so the delete would no-op without raising.
    """
    import pytest

    raw = {"id": 12345, "score": 0.9, "payload": {"content": "orphan"}}
    with pytest.raises(ValueError, match="no '_doc_id'"):
        _MOD._result_to_doc(raw)


def test_get_collection_returns_none_and_creates_collection() -> None:
    """_get_collection handles SDK returning None (the production SDK behavior)."""
    class _FakeDatabaseReturnsNone:
        def __init__(self, path: str) -> None:
            self._collections: dict = {}

        def get_collection(self, name: str) -> Optional[_FakeCollection]:
            return None  # Real VelesDB SDK returns None for unknown collections.

        def create_collection(
            self, name: str, dimension: int, metric: str
        ) -> _FakeCollection:
            col = _FakeCollection()
            self._collections[name] = col
            return col

    original_velesdb = _MOD.velesdb
    try:
        _MOD.velesdb = types.SimpleNamespace(Database=_FakeDatabaseReturnsNone)  # type: ignore
        store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_none_path")
        assert store.count_documents() == 0
        assert store._collection is not None
    finally:
        _MOD.velesdb = original_velesdb


# ---------------------------------------------------------------------------
# Haystack-filter -> VelesDB-filter translator tests
# ---------------------------------------------------------------------------


def test_translate_filter_none_passes_through() -> None:
    assert _MOD._translate_haystack_filter(None) is None


def test_translate_filter_wraps_top_level_in_condition() -> None:
    """The entry point wraps the translated condition under the
    VelesDB ``Filter`` shape (``{"condition": ...}``); recursive
    ``_translate_condition()`` returns the inner shape directly.
    """
    out = _MOD._translate_haystack_filter(
        {"field": "meta.x", "operator": "==", "value": 1}
    )
    assert out == {"condition": {"type": "eq", "field": "x", "value": 1}}


def test_translate_filter_simple_eq_strips_meta_prefix() -> None:
    out = _MOD._translate_haystack_filter(
        {"field": "meta.source", "operator": "==", "value": "wiki"}
    )
    assert out == {
        "condition": {"type": "eq", "field": "source", "value": "wiki"}
    }


def test_translate_filter_keeps_field_when_no_meta_prefix() -> None:
    out = _MOD._translate_haystack_filter(
        {"field": "_doc_id", "operator": "==", "value": "doc1"}
    )
    assert out == {
        "condition": {"type": "eq", "field": "_doc_id", "value": "doc1"}
    }


def test_translate_filter_all_comparison_operators() -> None:
    cases = [
        ("==", "eq"),
        ("!=", "neq"),
        (">", "gt"),
        (">=", "gte"),
        ("<", "lt"),
        ("<=", "lte"),
    ]
    for hs_op, veles_op in cases:
        out = _MOD._translate_haystack_filter(
            {"field": "meta.x", "operator": hs_op, "value": 42}
        )
        assert out == {
            "condition": {"type": veles_op, "field": "x", "value": 42}
        }, hs_op


def test_translate_filter_in_remaps_value_to_values() -> None:
    out = _MOD._translate_haystack_filter(
        {"field": "meta.tag", "operator": "in", "value": ["a", "b", "c"]}
    )
    assert out == {
        "condition": {"type": "in", "field": "tag", "values": ["a", "b", "c"]}
    }


def test_translate_filter_in_rejects_scalar_value() -> None:
    import pytest

    with pytest.raises(ValueError, match="'in' operator requires"):
        _MOD._translate_haystack_filter(
            {"field": "meta.tag", "operator": "in", "value": "scalar"}
        )


def test_translate_filter_not_in_wraps_in_with_not() -> None:
    out = _MOD._translate_haystack_filter(
        {"field": "meta.tag", "operator": "not in", "value": ["x", "y"]}
    )
    assert out == {
        "condition": {
            "type": "not",
            "condition": {"type": "in", "field": "tag", "values": ["x", "y"]},
        }
    }


def test_translate_filter_logical_and() -> None:
    out = _MOD._translate_haystack_filter({
        "operator": "AND",
        "conditions": [
            {"field": "meta.source", "operator": "==", "value": "wiki"},
            {"field": "meta.score", "operator": ">", "value": 0.5},
        ],
    })
    assert out == {
        "condition": {
            "type": "and",
            "conditions": [
                {"type": "eq", "field": "source", "value": "wiki"},
                {"type": "gt", "field": "score", "value": 0.5},
            ],
        }
    }


def test_translate_filter_logical_or() -> None:
    out = _MOD._translate_haystack_filter({
        "operator": "OR",
        "conditions": [
            {"field": "meta.lang", "operator": "==", "value": "en"},
            {"field": "meta.lang", "operator": "==", "value": "fr"},
        ],
    })
    assert out == {
        "condition": {
            "type": "or",
            "conditions": [
                {"type": "eq", "field": "lang", "value": "en"},
                {"type": "eq", "field": "lang", "value": "fr"},
            ],
        }
    }


def test_translate_filter_nested_and_inside_or() -> None:
    out = _MOD._translate_haystack_filter({
        "operator": "OR",
        "conditions": [
            {"field": "meta.a", "operator": "==", "value": 1},
            {
                "operator": "AND",
                "conditions": [
                    {"field": "meta.b", "operator": ">", "value": 0},
                    {"field": "meta.c", "operator": "<", "value": 10},
                ],
            },
        ],
    })
    assert out == {
        "condition": {
            "type": "or",
            "conditions": [
                {"type": "eq", "field": "a", "value": 1},
                {
                    "type": "and",
                    "conditions": [
                        {"type": "gt", "field": "b", "value": 0},
                        {"type": "lt", "field": "c", "value": 10},
                    ],
                },
            ],
        }
    }


def test_translate_filter_not_wraps_single_condition() -> None:
    out = _MOD._translate_haystack_filter({
        "operator": "NOT",
        "conditions": [{"field": "meta.x", "operator": "==", "value": 1}],
    })
    assert out == {
        "condition": {
            "type": "not",
            "condition": {"type": "eq", "field": "x", "value": 1},
        }
    }


def test_translate_filter_not_rejects_multi_condition() -> None:
    import pytest

    with pytest.raises(ValueError, match="NOT must wrap exactly one"):
        _MOD._translate_haystack_filter({
            "operator": "NOT",
            "conditions": [
                {"field": "meta.x", "operator": "==", "value": 1},
                {"field": "meta.y", "operator": "==", "value": 2},
            ],
        })


def test_translate_filter_unknown_operator_raises() -> None:
    import pytest

    with pytest.raises(NotImplementedError, match="Unsupported Haystack filter operator"):
        _MOD._translate_haystack_filter(
            {"field": "meta.x", "operator": "regex", "value": ".*"}
        )


def test_translate_filter_non_dict_raises() -> None:
    import pytest

    with pytest.raises(ValueError, match="must be a dict"):
        _MOD._translate_haystack_filter("not a dict")  # type: ignore[arg-type]


def test_translate_filter_logical_empty_conditions_raises() -> None:
    import pytest

    with pytest.raises(ValueError, match="non-empty 'conditions'"):
        _MOD._translate_haystack_filter({"operator": "AND", "conditions": []})


def test_translate_filter_comparison_missing_field_raises() -> None:
    import pytest

    with pytest.raises(ValueError, match="non-empty 'field'"):
        _MOD._translate_haystack_filter({"operator": "==", "value": 1})


def test_filter_documents_translates_haystack_filter_to_veles_shape() -> None:
    """End-to-end: filter_documents accepts a Haystack-shaped filter and
    forwards a VelesDB-shaped filter to Collection.scroll(...).
    """
    captured: dict = {}

    class _CapturingCollection(_FakeCollection):
        def scroll(  # pylint: disable=redefined-builtin
            self,
            *,
            batch_size: int = 100,
            filter: Any = None,
            as_dataframe: bool = False,
            backend: str = "pandas",
        ) -> Any:
            captured["filter"] = filter
            return super().scroll(
                batch_size=batch_size,
                filter=filter,
                as_dataframe=as_dataframe,
                backend=backend,
            )

    class _CapturingDatabase:
        def __init__(self, path: str) -> None:
            self._col = _CapturingCollection()

        def get_collection(self, name: str) -> _CapturingCollection:
            return self._col

        def create_collection(
            self, name: str, dimension: int, metric: str
        ) -> _CapturingCollection:
            return self._col

    original_velesdb = _MOD.velesdb
    try:
        _MOD.velesdb = types.SimpleNamespace(Database=_CapturingDatabase)  # type: ignore
        store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_translate")
        store.write_documents([
            Document(id="x", content="hello", embedding=[0.1], meta={"source": "wiki"})
        ])
        store.filter_documents(
            {"field": "meta.source", "operator": "==", "value": "wiki"}
        )
        assert captured["filter"] == {
            "condition": {"type": "eq", "field": "source", "value": "wiki"},
        }, "Haystack filter must be translated to VelesDB Filter shape before scroll"
    finally:
        _MOD.velesdb = original_velesdb


def test_embedding_retrieval_translates_haystack_filter_to_veles_shape() -> None:
    """End-to-end: embedding_retrieval accepts a Haystack-shaped filter and
    forwards a VelesDB-shaped filter to Collection.search(...).
    """
    captured: dict = {}

    class _CapturingCollection(_FakeCollection):
        def search(  # pylint: disable=redefined-builtin
            self, vector: list, top_k: int = 10, filter: Any = None
        ) -> list:
            captured["filter"] = filter
            return super().search(vector=vector, top_k=top_k, filter=filter)

    class _CapturingDatabase:
        def __init__(self, path: str) -> None:
            self._col = _CapturingCollection()

        def get_collection(self, name: str) -> _CapturingCollection:
            return self._col

        def create_collection(
            self, name: str, dimension: int, metric: str
        ) -> _CapturingCollection:
            return self._col

    original_velesdb = _MOD.velesdb
    try:
        _MOD.velesdb = types.SimpleNamespace(  # type: ignore
            Database=_CapturingDatabase, SearchOptions=_FakeSearchOptions
        )
        store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_search_translate")
        store.write_documents([
            Document(id="y", content="world", embedding=[0.5])
        ])
        store.embedding_retrieval(
            [0.5],
            filters={
                "operator": "AND",
                "conditions": [
                    {"field": "meta.lang", "operator": "==", "value": "en"},
                    {"field": "meta.score", "operator": ">", "value": 0.5},
                ],
            },
        )
        assert captured["filter"] == {
            "condition": {
                "type": "and",
                "conditions": [
                    {"type": "eq", "field": "lang", "value": "en"},
                    {"type": "gt", "field": "score", "value": 0.5},
                ],
            },
        }, "embedding_retrieval must translate Haystack filter to VelesDB Filter shape"
    finally:
        _MOD.velesdb = original_velesdb


# ---------------------------------------------------------------------------
# I1: fusion (RSF / Weighted) on embedding_retrieval
# ---------------------------------------------------------------------------


def _store_with_three_docs(name: str) -> Any:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name=name)
    store.write_documents([
        Document(id="d1", content="one", embedding=[0.1]),
        Document(id="d2", content="two", embedding=[0.2]),
        Document(id="d3", content="three", embedding=[0.3]),
    ])
    return store


def test_embedding_retrieval_fusion_changes_ordering_vs_default() -> None:
    """fusion='rsf' must reorder results relative to the default ranking."""
    store = _store_with_three_docs("t_fusion_rsf")
    default_ids = [d.id for d in store.embedding_retrieval([0.1], top_k=3)]
    rsf_ids = [
        d.id for d in store.embedding_retrieval([0.1], top_k=3, fusion="rsf")
    ]
    assert rsf_ids != default_ids, "fusion='rsf' must change result ordering"
    assert sorted(rsf_ids) == sorted(default_ids), "same doc set, different order"


def test_embedding_retrieval_rsf_and_weighted_differ() -> None:
    """rsf and weighted fusion must produce different orderings."""
    store = _store_with_three_docs("t_fusion_pair")
    rsf_ids = [
        d.id for d in store.embedding_retrieval([0.1], top_k=3, fusion="rsf")
    ]
    weighted_ids = [
        d.id
        for d in store.embedding_retrieval([0.1], top_k=3, fusion="weighted")
    ]
    assert rsf_ids != weighted_ids, "rsf and weighted must differ in ordering"


def test_embedding_retrieval_fusion_passes_params() -> None:
    """fusion_params must reach build_fusion_strategy and the collection."""
    captured: dict = {}

    class _CapturingCollection(_FakeCollection):
        def multi_query_search(
            self,
            vectors: list,
            top_k: int = 10,
            fusion: Any = None,
            filter: Any = None,  # pylint: disable=redefined-builtin
        ) -> list:
            captured["fusion_name"] = getattr(fusion, "name", None)
            captured["fusion_params"] = getattr(fusion, "params", None)
            return super().multi_query_search(
                vectors, top_k=top_k, fusion=fusion, filter=filter
            )

    class _CapturingDatabase:
        def __init__(self, path: str) -> None:
            self._col = _CapturingCollection()

        def get_collection(self, name: str) -> _CapturingCollection:
            return self._col

        def create_collection(
            self, name: str, dimension: int, metric: str
        ) -> _CapturingCollection:
            return self._col

    original_velesdb = _MOD.velesdb
    try:
        _MOD.velesdb = types.SimpleNamespace(  # type: ignore
            Database=_CapturingDatabase,
            SearchOptions=_FakeSearchOptions,
            FusionStrategy=_FakeFusionStrategy,
        )
        store = _MOD.VelesDBDocumentStore(
            path="/tmp/hs", collection_name="t_fusion_params"
        )
        store.write_documents([Document(id="p", content="x", embedding=[0.5])])
        store.embedding_retrieval(
            [0.5],
            top_k=3,
            fusion="rsf",
            fusion_params={"dense_weight": 0.7, "sparse_weight": 0.3},
        )
        assert captured["fusion_name"] == "relative_score"
        assert captured["fusion_params"]["dense_weight"] == 0.7
    finally:
        _MOD.velesdb = original_velesdb


# ---------------------------------------------------------------------------
# I2: named-sparse-index creation on write_documents
# ---------------------------------------------------------------------------


def test_write_documents_forwards_named_sparse_vectors() -> None:
    """A named sparse vector dict must reach the upserted point so the
    underlying named sparse index is created.
    """
    captured: dict = {}

    class _CapturingCollection(_FakeCollection):
        def upsert(self, points: list) -> int:
            captured["points"] = points
            return super().upsert(points)

    class _CapturingDatabase:
        def __init__(self, path: str) -> None:
            self._col = _CapturingCollection()

        def get_collection(self, name: str) -> _CapturingCollection:
            return self._col

        def create_collection(
            self, name: str, dimension: int, metric: str
        ) -> _CapturingCollection:
            return self._col

    original_velesdb = _MOD.velesdb
    try:
        _MOD.velesdb = types.SimpleNamespace(  # type: ignore
            Database=_CapturingDatabase,
            SearchOptions=_FakeSearchOptions,
            FusionStrategy=_FakeFusionStrategy,
        )
        store = _MOD.VelesDBDocumentStore(
            path="/tmp/hs", collection_name="t_named_sparse"
        )
        store.write_documents(
            [Document(id="s1", content="hi", embedding=[0.5])],
            sparse_vectors=[{"bge_m3": {0: 1.5, 7: 0.8}}],
        )
        point = captured["points"][0]
        assert point["sparse_vector"] == {"bge_m3": {0: 1.5, 7: 0.8}}
    finally:
        _MOD.velesdb = original_velesdb
