"""Haystack 2.x DocumentStore backed by VelesDB.

Implements the Haystack ``DocumentStore`` protocol so VelesDB can be used
as the vector backend in any Haystack 2.x indexing or retrieval pipeline.
"""
from __future__ import annotations

import logging
from typing import Any, Dict, List, Optional

from haystack import default_from_dict, default_to_dict
from haystack.dataclasses import Document
from haystack.document_stores.errors import DuplicateDocumentError
from haystack.document_stores.types import DuplicatePolicy

import velesdb
from velesdb_common.fusion import build_fusion_strategy
from velesdb_common.ids import stable_hash_id
from velesdb_common.security import (
    validate_collection_name,
    validate_metric,
    validate_named_sparse_vector,
    validate_path,
)

logger = logging.getLogger(__name__)

__all__ = ["VelesDBDocumentStore"]

_DEFAULT_COLLECTION = "haystack_documents"
_DEFAULT_DIMENSION = 768
_DEFAULT_METRIC = "cosine"
_DEFAULT_SCROLL_LIMIT = 10_000
# Reserved keys stored by this integration in the VelesDB payload.
_RESERVED_PAYLOAD_KEYS = frozenset({"_doc_id", "content"})

# Haystack 2.x comparison operator (string) -> VelesDB Condition type tag.
# Reference: https://docs.haystack.deepset.ai/docs/metadata-filtering
_HAYSTACK_COMPARISON_TO_VELES: Dict[str, str] = {
    "==": "eq",
    "!=": "neq",
    ">": "gt",
    ">=": "gte",
    "<": "lt",
    "<=": "lte",
    "in": "in",
    # `not in` is handled specially: wrapped in a top-level NOT around an `in`.
}

# Haystack 2.x logical operator -> VelesDB Condition type tag.
_HAYSTACK_LOGIC_TO_VELES: Dict[str, str] = {
    "AND": "and",
    "OR": "or",
    "NOT": "not",
}


def _strip_meta_prefix(field: str) -> str:
    """Drop the leading ``meta.`` namespace from a Haystack field path.

    Haystack stores user-supplied metadata under ``Document.meta`` and exposes
    it through the filter API as ``meta.<key>``. VelesDB stores the same data
    flat in the payload (alongside the reserved ``_doc_id`` and ``content``
    keys), so the prefix must be stripped before the filter can match.
    """
    if field.startswith("meta."):
        return field[len("meta."):]
    return field


def _translate_logical(operator: str, filters: Dict[str, Any]) -> Dict[str, Any]:
    """Translate a Haystack ``AND`` / ``OR`` / ``NOT`` combinator node.

    ``NOT`` is special: Haystack wraps ``conditions`` (list) just like AND/OR,
    but VelesDB ``Not`` wraps a single ``condition``. Reject anything other
    than a one-element list — Haystack itself rejects multi-element NOT.
    """
    veles_op = _HAYSTACK_LOGIC_TO_VELES[operator]
    conditions = filters.get("conditions") or []
    if veles_op == "not":
        if len(conditions) != 1:
            raise ValueError(
                "Haystack NOT must wrap exactly one condition; got "
                f"{len(conditions)}"
            )
        return {"type": "not", "condition": _translate_condition(conditions[0])}
    if not conditions:
        raise ValueError(
            f"Haystack {operator} requires non-empty 'conditions' list"
        )
    return {
        "type": veles_op,
        "conditions": [_translate_condition(c) for c in conditions],
    }


def _translate_comparison(operator: str, filters: Dict[str, Any]) -> Dict[str, Any]:
    """Translate a Haystack comparison leaf (``field``/``operator``/``value``)."""
    field = filters.get("field")
    if not isinstance(field, str) or not field:
        raise ValueError(
            f"Haystack comparison '{operator}' requires non-empty 'field'"
        )
    veles_field = _strip_meta_prefix(field)
    if operator == "in":
        values = filters.get("value")
        if not isinstance(values, list):
            raise ValueError(
                "Haystack 'in' operator requires 'value' to be a list"
            )
        return {"type": "in", "field": veles_field, "values": values}
    return {
        "type": _HAYSTACK_COMPARISON_TO_VELES[operator],
        "field": veles_field,
        "value": filters.get("value"),
    }


def _translate_not_in(filters: Dict[str, Any]) -> Dict[str, Any]:
    """Translate a Haystack ``not in`` leaf to VelesDB ``Not(In(...))``."""
    field = filters.get("field")
    if not isinstance(field, str) or not field:
        raise ValueError("Haystack 'not in' requires non-empty 'field'")
    values = filters.get("value")
    if not isinstance(values, list):
        raise ValueError("Haystack 'not in' requires 'value' to be a list")
    return {
        "type": "not",
        "condition": {
            "type": "in",
            "field": _strip_meta_prefix(field),
            "values": values,
        },
    }


def _translate_condition(filters: Dict[str, Any]) -> Dict[str, Any]:
    """Recursive helper: translate ANY Haystack filter node to a VelesDB
    ``Condition`` shape. The top-level :func:`_translate_haystack_filter`
    wraps the result in ``{"condition": ...}`` to produce a complete
    VelesDB ``Filter`` object.
    """
    if not isinstance(filters, dict):
        raise ValueError(
            f"Haystack filter must be a dict, got {type(filters).__name__}"
        )
    operator = filters.get("operator")
    if operator in _HAYSTACK_LOGIC_TO_VELES:
        return _translate_logical(operator, filters)
    if operator in _HAYSTACK_COMPARISON_TO_VELES:
        return _translate_comparison(operator, filters)
    if operator == "not in":
        return _translate_not_in(filters)
    supported = sorted(
        set(_HAYSTACK_COMPARISON_TO_VELES)
        | set(_HAYSTACK_LOGIC_TO_VELES)
        | {"not in"}
    )
    raise NotImplementedError(
        f"Unsupported Haystack filter operator: {operator!r}. Supported: {supported}"
    )


def _translate_haystack_filter(
    filters: Optional[Dict[str, Any]],
) -> Optional[Dict[str, Any]]:
    """Translate a Haystack 2.x filter dict to the VelesDB ``Filter`` JSON shape.

    Haystack 2.x uses two interchangeable filter shapes:

    1. Comparison leaf — ``{"field": "meta.x", "operator": "==", "value": v}``
    2. Logical combinator — ``{"operator": "AND", "conditions": [<leaf>, ...]}``

    VelesDB tags every node with a ``type`` discriminator and uses lowercase
    snake_case operator names (``eq``, ``and``, ``in``, ...). The ``meta.``
    namespace prefix is stripped because the DocumentStore stores metadata
    flat in the VelesDB payload. The full ``Filter`` JSON object always wraps
    the translated condition tree under a top-level ``"condition"`` key:

        Haystack:  {"field": "meta.x", "operator": "==", "value": 1}
        VelesDB:   {"condition": {"type": "eq", "field": "x", "value": 1}}

    Returns ``None`` when *filters* is ``None`` (pass-through), so callers can
    forward the result directly to ``Collection.scroll(filter=...)``.

    Raises:
        NotImplementedError: When the filter contains an operator VelesDB
            does not support (e.g. a misspelled operator).
        ValueError: When the filter dict is structurally invalid (missing
            required keys, conditions list empty for a logical combinator).
    """
    if filters is None:
        return None
    return {"condition": _translate_condition(filters)}


def _doc_to_point(doc: Document, sparse_vector: Optional[dict] = None) -> dict:
    """Convert a Haystack Document to a VelesDB point dict.

    Reserved payload keys (``_doc_id``, ``content``) are always written from
    the document's canonical fields, not from ``doc.meta``.  Any meta entry
    that shares a reserved name is silently dropped from the payload to
    prevent round-trip corruption.

    When *sparse_vector* is given (a flat ``dict[int, float]`` or a named
    ``dict[str, dict[int, float]]`` mapping) it is attached so the upsert
    creates the matching sparse index for hybrid retrieval.
    """
    payload: dict = {}
    # Merge meta first; reserved keys are excluded so they cannot
    # clobber the canonical doc identity written below.
    if doc.meta:
        for k, v in doc.meta.items():
            if k not in _RESERVED_PAYLOAD_KEYS:
                payload[k] = v
    payload["_doc_id"] = doc.id
    if doc.content is not None:
        payload["content"] = doc.content
    point: dict = {"id": stable_hash_id(doc.id), "payload": payload}
    if doc.embedding is not None:
        point["vector"] = list(doc.embedding)
    if sparse_vector is not None:
        point["sparse_vector"] = sparse_vector
    return point


def _result_to_doc(
    result: dict, *, scale_score: bool = False, metric: str = "cosine"
) -> Document:
    """Convert a VelesDB search or scroll result to a Haystack Document.

    Requires ``_doc_id`` to be present in the payload. Points written by
    :meth:`VelesDBDocumentStore.write_documents` always carry that key, so
    a missing ``_doc_id`` means the underlying VelesDB collection was
    populated by a different code path (raw ``col.upsert``, migration
    scripts, mixed tooling). Falling back to the stringified integer ID
    would silently corrupt :meth:`delete_documents`: the integer-as-string
    re-hashes through SHA-256 to a *different* integer, so the delete
    would no-op without raising. We fail fast instead.

    Raises:
        ValueError: When ``_doc_id`` is missing from the payload.
    """
    payload = result.get("payload", {})
    doc_id = payload.get("_doc_id")
    if doc_id is None:
        raise ValueError(
            f"VelesDB point id={result.get('id')} has no '_doc_id' field in "
            "its payload. VelesDBDocumentStore requires every point in the "
            "underlying collection to be written via write_documents(); "
            "points populated by raw col.upsert() or external migration "
            "scripts cannot be round-tripped because the stringified "
            "integer ID would re-hash to a different integer and break "
            "delete_documents()."
        )
    content = payload.get("content")
    meta = {k: v for k, v in payload.items() if k not in _RESERVED_PAYLOAD_KEYS}
    raw_score: Optional[float] = result.get("score")
    if scale_score and raw_score is not None and metric == "cosine":
        # Normalise cosine similarity from [-1, 1] to [0, 1].
        # Only meaningful for cosine; l2 and dot scores have different ranges.
        score: Optional[float] = (raw_score + 1.0) / 2.0
    else:
        score = raw_score
    return Document(id=doc_id, content=content, meta=meta, score=score)


def _build_int_id_map(documents: List[Document]) -> Dict[int, str]:
    """Map every document's integer ID back to its string ID, raising on
    in-batch SHA-256 collisions.

    Two distinct string IDs that hash to the same 63-bit integer would
    silently overwrite each other on upsert. This helper is the first
    line of defence: it detects collisions inside a single
    ``write_documents`` batch before any state hits the collection.
    """
    int_id_map: Dict[int, str] = {}
    for doc in documents:
        iid = stable_hash_id(doc.id)
        if iid in int_id_map and int_id_map[iid] != doc.id:
            raise ValueError(
                f"SHA-256 collision in write batch: '{int_id_map[iid]}' and "
                f"'{doc.id}' map to the same integer ID {iid}. "
                "Rename one of the documents."
            )
        int_id_map[iid] = doc.id
    return int_id_map


def _enforce_fail_policy(col: Any, int_id_map: Dict[int, str]) -> None:
    """For ``DuplicatePolicy.FAIL``, raise if any incoming integer ID
    already exists in the collection, or if a stored point points to a
    different string ID (cross-store SHA-256 collision).

    Uses point-by-point ``col.get(int_ids)`` — O(batch_size) — instead of
    a full scroll, so collections larger than ``scroll_limit`` are still
    correctly enforced.
    """
    existing_points: List[Any] = col.get(list(int_id_map.keys()))
    conflicts: List[str] = []
    for point in existing_points:
        if point is None:
            continue
        iid = point["id"]
        existing_str = point.get("payload", {}).get("_doc_id", str(iid))
        str_id = int_id_map[iid]
        if existing_str != str_id:
            raise ValueError(
                f"SHA-256 collision on write: incoming document '{str_id}' "
                f"maps to the same integer ID {iid} as existing document "
                f"'{existing_str}'. Rename one of the documents."
            )
        conflicts.append(str_id)
    if conflicts:
        raise DuplicateDocumentError(
            f"Documents already exist (policy=FAIL): {conflicts}"
        )


def _filter_skip_policy(
    col: Any, int_id_map: Dict[int, str], documents: List[Document]
) -> List[Document]:
    """For ``DuplicatePolicy.SKIP``: return only the documents whose
    integer ID does not already exist in the collection.

    Uses point-by-point ``col.get(int_ids)`` — same O(batch_size) shape as
    :func:`_enforce_fail_policy` — so collections larger than
    ``scroll_limit`` are still correctly handled.
    """
    existing_points: List[Any] = col.get(list(int_id_map.keys()))
    existing_int_ids: set[int] = {
        point["id"] for point in existing_points if point is not None
    }
    if not existing_int_ids:
        return documents
    str_to_int = {v: k for k, v in int_id_map.items()}
    return [doc for doc in documents if str_to_int[doc.id] not in existing_int_ids]


def _build_sparse_by_id(
    documents: List[Document],
    sparse_vectors: Optional[List[dict]],
) -> Dict[str, dict]:
    """Map each document id to its validated sparse vector.

    Keying by document id (rather than list position) keeps the sparse
    vectors aligned with their documents even when ``DuplicatePolicy.SKIP``
    drops a subset before upsert. Each entry is validated as a flat
    ``dict[int, float]`` or a named ``dict[str, dict[int, float]]`` mapping.
    """
    if sparse_vectors is None:
        return {}
    sparse_by_id: Dict[str, dict] = {}
    for idx, doc in enumerate(documents):
        if idx >= len(sparse_vectors):
            break
        sparse_by_id[doc.id] = validate_named_sparse_vector(sparse_vectors[idx])
    return sparse_by_id


def _documents_to_points(
    documents: List[Document],
    sparse_by_id: Optional[Dict[str, dict]] = None,
) -> List[dict]:
    """Convert each document to its VelesDB point dict, logging documents
    that lack an embedding so the caller still gets feedback when the
    underlying SDK accepts vector-less points.

    *sparse_by_id* (when given) maps document ids to their sparse vector dict;
    each is attached to its point so the upsert creates the corresponding
    sparse index.
    """
    sparse_by_id = sparse_by_id or {}
    points: List[dict] = []
    for doc in documents:
        if doc.embedding is None:
            logger.warning(
                "Document '%s' has no embedding; stored without vector.", doc.id
            )
        points.append(_doc_to_point(doc, sparse_vector=sparse_by_id.get(doc.id)))
    return points


class VelesDBDocumentStore:
    """Haystack 2.x DocumentStore backed by a local VelesDB collection.

    Stores documents (with optional embeddings) in VelesDB and exposes the
    standard Haystack retrieval interface so this store works as a drop-in
    backend for ``EmbeddingRetriever`` and similar pipeline components.

    Args:
        path: Directory path where VelesDB persists data.
        collection_name: Name of the VelesDB collection to use.
        embedding_dim: Dimensionality of the embedding vectors.
        metric: Distance metric: ``"cosine"``, ``"euclidean"``, ``"dot"``,
            ``"hamming"``, or ``"jaccard"``.
        scroll_limit: Maximum documents returned by :meth:`filter_documents`.
            Increase this value when your collection exceeds 10 000 documents.
    """

    def __init__(  # pylint: disable=too-many-arguments,too-many-positional-arguments
        self,
        path: str = "./velesdb_haystack",
        collection_name: str = _DEFAULT_COLLECTION,
        embedding_dim: int = _DEFAULT_DIMENSION,
        metric: str = _DEFAULT_METRIC,
        scroll_limit: int = _DEFAULT_SCROLL_LIMIT,
    ) -> None:
        self._path = validate_path(path)
        self._collection_name = validate_collection_name(collection_name)
        self._embedding_dim = embedding_dim
        self._metric = validate_metric(metric)
        self._scroll_limit = scroll_limit
        self._db: Optional[Any] = None
        self._collection: Optional[Any] = None

    # ------------------------------------------------------------------
    # Internal connection management
    # ------------------------------------------------------------------

    def _get_collection(self) -> Any:
        """Return the VelesDB collection, opening or creating it as needed."""
        if self._db is None:
            self._db = velesdb.Database(self._path)
        if self._collection is None:
            col: Optional[Any] = None
            try:
                col = self._db.get_collection(self._collection_name)
            except KeyError:
                pass
            if col is None:
                col = self._db.create_collection(
                    self._collection_name,
                    dimension=self._embedding_dim,
                    metric=self._metric,
                )
            self._collection = col
        return self._collection

    # ------------------------------------------------------------------
    # DocumentStore protocol
    # ------------------------------------------------------------------

    def count_documents(self) -> int:
        """Return the total number of documents in the store."""
        result = self._get_collection().count()
        return result if isinstance(result, int) else 0

    def filter_documents(
        self,
        filters: Optional[Dict[str, Any]] = None,
    ) -> List[Document]:
        """Return documents matching *filters*, or all documents when *None*.

        *filters* must follow the standard Haystack 2.x filter dict shape
        (see https://docs.haystack.deepset.ai/docs/metadata-filtering).
        It is translated to the VelesDB native filter format before being
        forwarded to ``Collection.scroll``. The ``meta.<key>`` namespace
        used by Haystack is stripped because this DocumentStore stores
        metadata flat in the VelesDB payload.

        The real SDK returns ``Iterator[List[Dict]]`` and has no ``limit``
        kwarg, so we drive the iterator ourselves and stop once
        ``self._scroll_limit`` documents have been collected. Increase
        ``scroll_limit`` on the constructor for collections larger than
        the default 10 000.

        Raises:
            NotImplementedError: When *filters* uses an operator VelesDB
                does not support.
            ValueError: When *filters* is structurally malformed.
        """
        veles_filter = _translate_haystack_filter(filters)
        col = self._get_collection()
        documents: List[Document] = []
        for batch in col.scroll(filter=veles_filter):
            for raw in batch:
                if len(documents) >= self._scroll_limit:
                    return documents
                documents.append(_result_to_doc(raw))
        return documents

    def write_documents(
        self,
        documents: List[Document],
        policy: DuplicatePolicy = DuplicatePolicy.NONE,
        sparse_vectors: Optional[List[dict]] = None,
    ) -> int:
        """Write *documents* to VelesDB and return the number written.

        Honours every Haystack ``DuplicatePolicy`` value:

        - ``NONE`` / ``OVERWRITE`` — VelesDB upsert semantics: an existing
          point with the same integer ID is overwritten by the incoming one.
        - ``SKIP`` — incoming documents whose integer ID already exists in
          the collection are dropped silently; only the genuinely-new
          subset is upserted. The return value reflects the number of new
          documents actually written.
        - ``FAIL`` — pre-scans the collection (point-by-point lookup,
          O(batch_size)) and raises :class:`DuplicateDocumentError` if any
          incoming document already exists. Prefer ``OVERWRITE`` or
          ``NONE`` for large batches to avoid the pre-scan cost.

        Args:
            documents: Documents to write.
            policy: Duplicate-handling policy (see above).
            sparse_vectors: Optional list aligned with *documents*; each entry
                is a flat ``dict[int, float]`` or a named
                ``dict[str, dict[int, float]]`` mapping (e.g.
                ``{"bge_m3": {0: 1.5}}``). A named mapping creates the named
                sparse index so it can later be queried with
                ``sparse_index_name="bge_m3"``.

        Raises:
            DuplicateDocumentError: When *policy* is ``FAIL`` and at least
                one document already exists in the store.
            ValueError: When a SHA-256 hash collision is detected — two
                distinct string IDs that map to the same integer ID.
        """
        if not documents:
            return 0
        sparse_by_id = _build_sparse_by_id(documents, sparse_vectors)
        int_id_map = _build_int_id_map(documents)
        col = self._get_collection()
        if policy == DuplicatePolicy.FAIL:
            _enforce_fail_policy(col, int_id_map)
            survivors = documents
        elif policy == DuplicatePolicy.SKIP:
            survivors = _filter_skip_policy(col, int_id_map, documents)
            if not survivors:
                return 0
        else:
            survivors = documents
        points = _documents_to_points(survivors, sparse_by_id)
        result = col.upsert(points)
        return result if isinstance(result, int) else len(points)

    def delete_documents(
        self,
        document_ids: Optional[List[str]] = None,
    ) -> None:
        """Delete documents identified by their Haystack string IDs."""
        if not document_ids:
            return
        int_ids = [stable_hash_id(did) for did in document_ids]
        self._get_collection().delete(int_ids)

    def embedding_retrieval(
        self,
        query_embedding: List[float],
        *,
        top_k: int = 10,
        filters: Optional[Dict[str, Any]] = None,
        scale_score: bool = True,
        fusion: Optional[str] = None,
        fusion_params: Optional[dict] = None,
    ) -> List[Document]:
        """Return the *top_k* documents most similar to *query_embedding*.

        Args:
            query_embedding: Dense query vector.
            top_k: Maximum number of documents to return.
            filters: Optional Haystack 2.x filter dict to restrict the search
                space. Translated to VelesDB native shape before being
                forwarded; ``meta.<key>`` is stripped to ``<key>``.
            scale_score: When ``True`` and ``metric="cosine"``, scores are
                normalised from ``[-1, 1]`` to ``[0, 1]``. Ignored for other
                metrics, where raw scores are returned unchanged. Score
                scaling does not apply to fused (``fusion``) results, whose
                scores come from the fusion strategy rather than the metric.
            fusion: Optional fusion strategy name applied to the ranking —
                one of ``"average"``, ``"maximum"``, ``"rrf"``,
                ``"weighted"``, ``"relative_score"`` / ``"rsf"``. When set,
                the query is ranked through the chosen
                :class:`velesdb.FusionStrategy`, which changes the result
                ordering relative to the default dense ranking. ``filters``
                are not supported together with ``fusion``.
            fusion_params: Optional parameters for *fusion* (see
                :func:`velesdb_common.fusion.build_fusion_strategy`).

        Raises:
            NotImplementedError: When *filters* uses an operator VelesDB
                does not support.
            ValueError: When *filters* is structurally malformed, or when
                *filters* is combined with *fusion*.
        """
        if fusion is not None:
            return self._fusion_retrieval(
                query_embedding, top_k, filters, fusion, fusion_params
            )
        veles_filter = _translate_haystack_filter(filters)
        results: List[dict] = self._get_collection().search_request(
            velesdb.SearchOptions(
                vector=query_embedding,
                top_k=top_k,
                filter=veles_filter,
            )
        )
        return [_result_to_doc(r, scale_score=scale_score, metric=self._metric) for r in results]

    def _fusion_retrieval(
        self,
        query_embedding: List[float],
        top_k: int,
        filters: Optional[Dict[str, Any]],
        fusion: str,
        fusion_params: Optional[dict],
    ) -> List[Document]:
        """Rank a single query through a :class:`velesdb.FusionStrategy`.

        Delegates to ``Collection.multi_query_search`` with a one-element
        query list so the chosen strategy decides the fused scores. The
        shared :func:`velesdb_common.fusion.build_fusion_strategy` builder is
        reused (same as the LangChain and LlamaIndex integrations).
        """
        if filters is not None:
            raise ValueError(
                "fusion cannot be combined with filters; apply filters in a "
                "separate dense embedding_retrieval call or omit fusion."
            )
        strategy = build_fusion_strategy(fusion, fusion_params)
        results: List[dict] = self._get_collection().multi_query_search(
            vectors=[query_embedding],
            top_k=top_k,
            fusion=strategy,
        )
        # Fused scores are strategy-derived, not metric similarities, so the
        # cosine [-1, 1] -> [0, 1] rescaling is intentionally not applied.
        return [_result_to_doc(r, metric=self._metric) for r in results]

    # ------------------------------------------------------------------
    # Haystack pipeline serialisation
    # ------------------------------------------------------------------

    def to_dict(self) -> Dict[str, Any]:
        """Serialise the store configuration for Haystack pipeline YAML."""
        return default_to_dict(
            self,
            path=self._path,
            collection_name=self._collection_name,
            embedding_dim=self._embedding_dim,
            metric=self._metric,
            scroll_limit=self._scroll_limit,
        )

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "VelesDBDocumentStore":
        """Restore a store instance from a Haystack pipeline config dict."""
        return default_from_dict(cls, data)
