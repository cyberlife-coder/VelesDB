"""Haystack 2.x DocumentStore backed by VelesDB.

Implements the Haystack ``DocumentStore`` protocol so VelesDB can be used
as the vector backend in any Haystack 2.x indexing or retrieval pipeline.
"""
from __future__ import annotations

import hashlib
import logging
from typing import Any, Dict, List, Optional

from haystack import default_from_dict, default_to_dict
from haystack.dataclasses import Document
from haystack.document_stores.errors import DuplicateDocumentError
from haystack.document_stores.types import DuplicatePolicy

import velesdb
from velesdb_common.security import (
    validate_collection_name,
    validate_metric,
    validate_path,
)

logger = logging.getLogger(__name__)

__all__ = ["VelesDBDocumentStore"]

_DEFAULT_COLLECTION = "haystack_documents"
_DEFAULT_DIMENSION = 768
_DEFAULT_METRIC = "cosine"
_DEFAULT_SCROLL_LIMIT = 10_000
_INT63_MASK = (1 << 63) - 1
# Reserved keys stored by this integration in the VelesDB payload.
_RESERVED_PAYLOAD_KEYS = frozenset({"_doc_id", "content"})


def _str_id_to_int(doc_id: str) -> int:
    """Map a Haystack string document ID to a stable positive 63-bit integer.

    Uses the first 8 bytes of SHA-256, masked to 63 bits (~9.2 × 10¹⁸ slots).
    Collision probability for a 1 M-document collection is roughly 5 × 10⁻¹⁴ —
    negligible for typical RAG workloads but not zero.  If two distinct string
    IDs produce the same integer ID, :meth:`write_documents` raises
    :class:`ValueError` rather than silently overwriting the existing document.
    """
    return int.from_bytes(hashlib.sha256(doc_id.encode()).digest()[:8], "big") & _INT63_MASK


def _doc_to_point(doc: Document) -> dict:
    """Convert a Haystack Document to a VelesDB point dict.

    Reserved payload keys (``_doc_id``, ``content``) are always written from
    the document's canonical fields, not from ``doc.meta``.  Any meta entry
    that shares a reserved name is silently dropped from the payload to
    prevent round-trip corruption.
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
    point: dict = {"id": _str_id_to_int(doc.id), "payload": payload}
    if doc.embedding is not None:
        point["vector"] = list(doc.embedding)
    return point


def _result_to_doc(
    result: dict, *, scale_score: bool = False, metric: str = "cosine"
) -> Document:
    """Convert a VelesDB search or scroll result to a Haystack Document."""
    payload = result.get("payload", {})
    doc_id = payload.get("_doc_id", str(result.get("id", "")))
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


class VelesDBDocumentStore:
    """Haystack 2.x DocumentStore backed by a local VelesDB collection.

    Stores documents (with optional embeddings) in VelesDB and exposes the
    standard Haystack retrieval interface so this store works as a drop-in
    backend for ``EmbeddingRetriever`` and similar pipeline components.

    Args:
        path: Directory path where VelesDB persists data.
        collection_name: Name of the VelesDB collection to use.
        embedding_dim: Dimensionality of the embedding vectors.
        metric: Distance metric: ``"cosine"``, ``"euclidean"``, or ``"dot"``.
        scroll_limit: Maximum documents returned by :meth:`filter_documents`.
            Increase this value when your collection exceeds 10 000 documents.
    """

    def __init__(
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

        Passes *filters* directly to VelesDB's scroll operation.  At most
        ``self._scroll_limit`` documents are returned per call; set
        ``scroll_limit`` on the constructor for collections larger than the
        default 10 000.
        """
        raw: List[dict] = self._get_collection().scroll(
            filter=filters, limit=self._scroll_limit
        )
        return [_result_to_doc(r) for r in raw]

    def write_documents(
        self,
        documents: List[Document],
        policy: DuplicatePolicy = DuplicatePolicy.NONE,
    ) -> int:
        """Write *documents* to VelesDB and return the number written.

        VelesDB upsert semantics apply for policies other than ``FAIL``:
        an existing point with the same integer ID is overwritten.

        When *policy* is ``DuplicatePolicy.FAIL`` this method scans the
        collection before writing and raises :class:`DuplicateDocumentError`
        if any incoming document already exists.  For large collections
        prefer ``OVERWRITE`` or ``NONE`` to avoid the pre-scan cost.

        Raises:
            DuplicateDocumentError: When *policy* is ``FAIL`` and at least
                one document already exists in the store.
            ValueError: When a SHA-256 hash collision is detected — two
                distinct string IDs that map to the same integer ID.
        """
        if not documents:
            return 0

        # Build int_id → str_id map and detect in-batch hash collisions.
        int_id_map: Dict[int, str] = {}
        for doc in documents:
            iid = _str_id_to_int(doc.id)
            if iid in int_id_map and int_id_map[iid] != doc.id:
                raise ValueError(
                    f"SHA-256 collision in write batch: '{int_id_map[iid]}' and "
                    f"'{doc.id}' map to the same integer ID {iid}. "
                    f"Rename one of the documents."
                )
            int_id_map[iid] = doc.id

        col = self._get_collection()

        if policy == DuplicatePolicy.FAIL:
            # Point-by-point lookup: O(batch_size), not O(collection_size).
            # Avoids the scroll_limit blind spot of the old scroll approach.
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

        points = []
        for doc in documents:
            if doc.embedding is None:
                logger.warning("Document '%s' has no embedding; stored without vector.", doc.id)
            points.append(_doc_to_point(doc))
        result = col.upsert(points)
        return result if isinstance(result, int) else len(points)

    def delete_documents(
        self,
        document_ids: Optional[List[str]] = None,
    ) -> None:
        """Delete documents identified by their Haystack string IDs."""
        if not document_ids:
            return
        int_ids = [_str_id_to_int(did) for did in document_ids]
        self._get_collection().delete(int_ids)

    def embedding_retrieval(
        self,
        query_embedding: List[float],
        *,
        top_k: int = 10,
        filters: Optional[Dict[str, Any]] = None,
        scale_score: bool = True,
    ) -> List[Document]:
        """Return the *top_k* documents most similar to *query_embedding*.

        Args:
            query_embedding: Dense query vector.
            top_k: Maximum number of documents to return.
            filters: Optional VelesDB filter dict to restrict the search space.
            scale_score: When ``True`` and ``metric="cosine"``, scores are
                normalised from ``[-1, 1]`` to ``[0, 1]``. Ignored for other
                metrics, where raw scores are returned unchanged.
        """
        results: List[dict] = self._get_collection().search(
            vector=query_embedding,
            top_k=top_k,
            filter=filters,
        )
        return [_result_to_doc(r, scale_score=scale_score, metric=self._metric) for r in results]

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
