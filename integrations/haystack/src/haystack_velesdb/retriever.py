"""Haystack retriever component backed by a :class:`VelesDBDocumentStore`."""

from typing import Any, Dict, List, Optional

from haystack import component, default_from_dict, default_to_dict
from haystack.dataclasses import Document

from haystack_velesdb.document_store import VelesDBDocumentStore


@component
class VelesDBEmbeddingRetriever:
    """Retrieves documents from a :class:`VelesDBDocumentStore` by dense
    embedding similarity, for use inside a Haystack ``Pipeline``.

    Ships the component the ecosystem expects (like ``QdrantEmbeddingRetriever``)
    so callers no longer have to hand-roll a ``@component`` wrapper: connect an
    embedder's ``embedding`` output to this component's ``query_embedding`` input.
    ``top_k`` / ``filters`` set at construction can be overridden per ``run``.
    """

    def __init__(
        self,
        document_store: VelesDBDocumentStore,
        top_k: int = 10,
        filters: Optional[Dict[str, Any]] = None,
        scale_score: bool = True,
    ) -> None:
        self._document_store = document_store
        self._top_k = top_k
        self._filters = filters
        self._scale_score = scale_score

    @component.output_types(documents=List[Document])
    def run(
        self,
        query_embedding: List[float],
        top_k: Optional[int] = None,
        filters: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, List[Document]]:
        """Run dense retrieval for ``query_embedding``."""
        documents = self._document_store.embedding_retrieval(
            query_embedding,
            top_k=self._top_k if top_k is None else top_k,
            filters=self._filters if filters is None else filters,
            scale_score=self._scale_score,
        )
        return {"documents": documents}

    def to_dict(self) -> Dict[str, Any]:
        """Serialize the component (and its store) for pipeline persistence."""
        return default_to_dict(
            self,
            document_store=self._document_store.to_dict(),
            top_k=self._top_k,
            filters=self._filters,
            scale_score=self._scale_score,
        )

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "VelesDBEmbeddingRetriever":
        """Rebuild the component (and its store) from a serialized pipeline."""
        store_data = data["init_parameters"]["document_store"]
        data["init_parameters"]["document_store"] = VelesDBDocumentStore.from_dict(store_data)
        return default_from_dict(cls, data)
