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
from haystack.document_stores.types import DuplicatePolicy

import velesdb

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
    """Map a Haystack string document ID to a stable positive 63-bit integer."""
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


def _result_to_doc(result: dict, *, scale_score: bool = False) -> Document:
    """Convert a VelesDB search or scroll result to a Haystack Document."""
    payload = result.get("payload", {})
    doc_id = payload.get("_doc_id", str(result.get("id", "")))
    content = payload.get("content")
    meta = {k: v for k, v in payload.items() if k not in _RESERVED_PAYLOAD_KEYS}
    raw_score: Optional[float] = result.get("score")
    if scale_score and raw_score is not None:
        # Normalise cosine similarity from [-1, 1] to [0, 1].
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
        metric: Distance metric: ``"cosine"``, ``"dot"``, or ``"l2"``.
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
        self._path = path
        self._collection_name = collection_name
        self._embedding_dim = embedding_dim
        self._metric = metric
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
            try:
                self._collection = self._db.get_collection(self._collection_name)
            except Exception:
                self._collection = self._db.create_collection(
                    self._collection_name,
                    dimension=self._embedding_dim,
                    metric=self._metric,
                )
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

        VelesDB upsert semantics apply regardless of *policy*: an existing
        point with the same integer ID is always overwritten.
        """
        if not documents:
            return 0
        points = []
        for doc in documents:
            if doc.embedding is None:
                logger.warning("Document '%s' has no embedding; stored without vector.", doc.id)
            points.append(_doc_to_point(doc))
        result = self._get_collection().upsert(points)
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
            scale_score: When ``True`` cosine scores are normalised from
                ``[-1, 1]`` to ``[0, 1]``.
        """
        results: List[dict] = self._get_collection().search(
            vector=query_embedding,
            top_k=top_k,
            filter=filters,
        )
        return [_result_to_doc(r, scale_score=scale_score) for r in results]

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
