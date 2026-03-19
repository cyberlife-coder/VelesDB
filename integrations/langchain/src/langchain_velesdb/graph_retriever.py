"""VelesDB Graph Retriever for LangChain.

Provides a retriever that combines vector search with graph traversal
for context expansion in RAG applications.

Example:
    >>> from langchain_velesdb import VelesDBVectorStore, GraphRetriever
    >>>
    >>> # Native mode (no server required)
    >>> vector_store = VelesDBVectorStore(path="./db", collection="docs")
    >>> retriever = GraphRetriever(
    ...     vector_store=vector_store,
    ...     mode="native",
    ...     graph_collection_name="my_graph",
    ...     max_depth=2,
    ...     expand_k=5
    ... )
    >>>
    >>> # REST mode (backward-compatible)
    >>> retriever = GraphRetriever(
    ...     vector_store=vector_store,
    ...     mode="rest",
    ...     server_url="http://localhost:8080",
    ...     max_depth=2,
    ...     expand_k=5
    ... )
    >>>
    >>> docs = retriever.get_relevant_documents("What is machine learning?")
"""

from typing import Any, List, Optional
from dataclasses import dataclass, field
import hashlib
import logging

logger = logging.getLogger(__name__)

try:
    from langchain_core.retrievers import BaseRetriever
    from langchain_core.documents import Document
    from langchain_core.callbacks import CallbackManagerForRetrieverRun
except ImportError:
    # Fallback for older langchain versions
    from langchain.schema.retriever import BaseRetriever
    from langchain.schema.document import Document
    from langchain.callbacks.manager import CallbackManagerForRetrieverRun


@dataclass
class TraversalResult:
    """Result from graph traversal."""
    target_id: int
    depth: int
    path: List[int] = field(default_factory=list)


def _open_native_graph(db_path: str, collection_name: str) -> Any:
    """Open a native graph collection from a VelesDB database.

    Args:
        db_path: Filesystem path to the VelesDB database directory.
        collection_name: Name of the graph collection to open.

    Returns:
        PyGraphCollection instance.

    Raises:
        ImportError: If the velesdb package is not installed.
        ValueError: If the graph collection cannot be found.
    """
    try:
        import velesdb  # type: ignore[import]
    except ImportError as exc:
        raise ImportError(
            "The 'velesdb' package is required for native mode. "
            "Install it with: pip install velesdb"
        ) from exc

    db = velesdb.Database(db_path)
    graph = db.get_graph_collection(collection_name)
    if graph is None:
        raise ValueError(
            f"Graph collection '{collection_name}' not found in database at '{db_path}'"
        )
    return graph


def _infer_db_path(vector_store: Any) -> Optional[str]:
    """Try to infer the database path from a vector store instance."""
    for attr in ("_db_path", "_path", "_data_path"):
        path = getattr(vector_store, attr, None)
        if path is not None:
            return str(path)
    return None


class GraphRetriever(BaseRetriever):
    """Retriever that uses graph traversal for context expansion.

    This retriever implements the "seed + expand" pattern:
    1. Vector search to find initial seed documents
    2. Graph traversal to expand context to related documents
    3. Return combined results for RAG

    Supports two modes:

    - ``"native"`` (default): uses the VelesDB Python bindings directly,
      no server required. Requires either ``db_path`` or a vector store with
      a ``_db_path`` / ``_path`` attribute, plus ``graph_collection_name``.
    - ``"rest"``: legacy HTTP mode — calls the VelesDB REST server.
      Requires ``server_url``.

    Args:
        vector_store: VelesDBVectorStore instance.
        mode: Operation mode, either ``"native"`` or ``"rest"`` (default: ``"native"``).
        db_path: Filesystem path to VelesDB data directory (native mode).
            If omitted, inferred from ``vector_store._db_path`` / ``_path``.
        graph_collection_name: Name of the graph collection (native mode).
        server_url: URL of VelesDB server (REST mode only).
        seed_k: Number of initial vector search results (default: 3).
        expand_k: Maximum documents to include after expansion (default: 10).
        max_depth: Maximum graph traversal depth (default: 2).
        rel_types: Relationship types to follow (default: all).
        score_threshold: Minimum score for seed documents (default: 0.0).
        low_latency: If True, skip graph expansion for minimal latency (default: False).
        timeout_ms: Timeout for REST graph operations in milliseconds (default: 1000).
        fallback_on_timeout: If True, return vector-only results on timeout (default: True).

    Example:
        >>> # Native mode — preferred
        >>> retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     mode="native",
        ...     graph_collection_name="my_graph",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>>
        >>> # REST mode — backward-compatible
        >>> retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     mode="rest",
        ...     server_url="http://localhost:8080",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>>
        >>> # Minimal latency mode (vector search only)
        >>> fast_retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     low_latency=True
        ... )
    """

    vector_store: Any  # VelesDBVectorStore
    mode: str = "native"
    db_path: Optional[str] = None
    graph_collection_name: Optional[str] = None
    server_url: str = "http://localhost:8080"
    seed_k: int = 3
    expand_k: int = 10
    max_depth: int = 2
    rel_types: Optional[List[str]] = None
    score_threshold: float = 0.0
    low_latency: bool = False
    timeout_ms: int = 1000
    fallback_on_timeout: bool = True

    class Config:
        arbitrary_types_allowed = True

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)

        # Store native graph collection as a plain instance attribute so it
        # stays outside Pydantic's field system (works for both v1 and v2).
        object.__setattr__(self, "_graph_collection", None)

        if self.mode == "rest":
            from langchain_velesdb.security import validate_url, validate_k
            validate_url(self.server_url)
            validate_k(self.seed_k, "seed_k")
            validate_k(self.expand_k, "expand_k")
        elif self.mode == "native":
            from langchain_velesdb.security import validate_k
            validate_k(self.seed_k, "seed_k")
            validate_k(self.expand_k, "expand_k")
            object.__setattr__(self, "_graph_collection", self._init_native_graph())
        else:
            raise ValueError(
                f"Invalid mode '{self.mode}'. Must be 'native' or 'rest'."
            )

    def _init_native_graph(self) -> Any:
        """Resolve db_path and open the native graph collection."""
        resolved_path = self.db_path or _infer_db_path(self.vector_store)
        if resolved_path is None:
            raise ValueError(
                "Native mode requires 'db_path' or a vector store with a "
                "'_db_path' / '_path' attribute."
            )
        if not self.graph_collection_name:
            raise ValueError(
                "Native mode requires 'graph_collection_name'."
            )
        return _open_native_graph(resolved_path, self.graph_collection_name)

    def _get_relevant_documents(
        self,
        query: str,
        *,
        run_manager: Optional[CallbackManagerForRetrieverRun] = None,
    ) -> List[Document]:
        """Get relevant documents using vector search + graph expansion.

        Args:
            query: The query string.
            run_manager: Callback manager (optional).

        Returns:
            List of relevant documents.
        """
        seed_results = self.vector_store.similarity_search_with_score(
            query, k=self.seed_k if self.low_latency else self.expand_k
        )

        seeds = [
            (doc, score) for doc, score in seed_results
            if score >= self.score_threshold
        ]

        if not seeds:
            return []

        if self.low_latency:
            return self._build_vector_only_results(seeds)

        return self._build_expanded_results(seeds)

    def _build_vector_only_results(
        self, seeds: List[Any]
    ) -> List[Document]:
        """Return seed documents annotated for vector-only mode."""
        result_docs = []
        for doc, score in seeds[: self.expand_k]:
            doc.metadata["graph_depth"] = 0
            doc.metadata["relevance_score"] = score
            doc.metadata["retrieval_mode"] = "vector_only"
            result_docs.append(doc)
        return result_docs

    def _build_expanded_results(self, seeds: List[Any]) -> List[Document]:
        """Run graph expansion and return combined document list."""
        expanded_ids: set = set()
        seed_docs: dict = {}
        graph_available = True

        for doc, score in seeds:
            doc_id = doc.metadata.get("id") or doc.metadata.get("doc_id")
            if doc_id is None:
                continue
            seed_docs[doc_id] = (doc, score)
            expanded_ids.add(doc_id)

            if graph_available:
                graph_available = self._expand_from_seed(
                    doc_id, expanded_ids, graph_available
                )

        return self._assemble_results(seed_docs, expanded_ids, graph_available)

    def _expand_from_seed(
        self, doc_id: int, expanded_ids: set, graph_available: bool
    ) -> bool:
        """Traverse graph from one seed node; return updated graph_available flag."""
        try:
            neighbors = self._traverse_graph(doc_id)
            for neighbor_id in neighbors:
                expanded_ids.add(neighbor_id)
        except Exception as exc:
            if self._is_timeout(exc):
                logger.warning(
                    "Graph traversal timeout for node %s, falling back to vector-only",
                    doc_id,
                )
                if self.fallback_on_timeout:
                    return False
                raise
            logger.debug("Graph traversal failed for node %s: %s", doc_id, exc)
        return graph_available

    def _is_timeout(self, exc: Exception) -> bool:
        """Return True if the exception represents a network/operation timeout."""
        try:
            import requests
            if isinstance(exc, requests.exceptions.Timeout):
                return True
        except ImportError:
            pass
        return isinstance(exc, TimeoutError)

    def _assemble_results(
        self,
        seed_docs: dict,
        expanded_ids: set,
        graph_available: bool,
    ) -> List[Document]:
        """Build the final document list from seeds and expanded neighbours."""
        retrieval_mode = "graph_expanded" if graph_available else "vector_fallback"
        result_docs: List[Document] = []

        for _doc_id, (doc, score) in seed_docs.items():
            doc.metadata["graph_depth"] = 0
            doc.metadata["relevance_score"] = score
            doc.metadata["retrieval_mode"] = retrieval_mode
            result_docs.append(doc)

        if graph_available:
            self._append_neighbor_docs(result_docs, expanded_ids, seed_docs)

        return result_docs[: self.expand_k]

    def _append_neighbor_docs(
        self, result_docs: List[Document], expanded_ids: set, seed_docs: dict
    ) -> None:
        """Fetch and append expanded neighbor documents to the result list."""
        remaining = self.expand_k - len(result_docs)
        neighbor_ids = [nid for nid in expanded_ids if nid not in seed_docs][:remaining]
        for neighbor_id in neighbor_ids:
            try:
                neighbor_doc = self._fetch_document(neighbor_id)
                if neighbor_doc:
                    neighbor_doc.metadata["graph_depth"] = 1
                    neighbor_doc.metadata["retrieval_mode"] = "graph_expanded"
                    result_docs.append(neighbor_doc)
            except Exception as exc:
                logger.debug("Failed to fetch neighbour document %s: %s", neighbor_id, exc)

    def _traverse_graph(self, source_id: int) -> List[int]:
        """Traverse graph from source node, dispatching to native or REST.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.
        """
        if self.mode == "native":
            return self._traverse_graph_native(source_id)
        return self._traverse_graph_rest(source_id)

    def _traverse_graph_native(self, source_id: int) -> List[int]:
        """Traverse graph using the native Python bindings.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.
        """
        results = self._graph_collection.traverse_bfs(
            source_id,
            max_depth=self.max_depth,
            limit=self.expand_k * 2,
            rel_types=self.rel_types or [],
        )
        return [r["target_id"] for r in results]

    def _traverse_graph_rest(self, source_id: int) -> List[int]:
        """Traverse graph via the VelesDB REST API.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.

        Raises:
            requests.exceptions.Timeout: If the request exceeds ``timeout_ms``.
        """
        import requests

        collection = self.vector_store._collection_name
        url = f"{self.server_url}/collections/{collection}/graph/traverse"

        payload = {
            "source": source_id,
            "strategy": "bfs",
            "max_depth": self.max_depth,
            "limit": self.expand_k * 2,
            "rel_types": self.rel_types or [],
        }

        timeout_sec = self.timeout_ms / 1000.0
        response = requests.post(url, json=payload, timeout=timeout_sec)

        if response.status_code == 200:
            data = response.json()
            return [r["target_id"] for r in data.get("results", [])]

        return []

    def _fetch_document(self, doc_id: int) -> Optional[Document]:
        """Fetch a document by ID from the vector store.

        Args:
            doc_id: Document ID.

        Returns:
            Document or None if not found.
        """
        try:
            results = self.vector_store.get_by_ids([doc_id])
            if results:
                return results[0]
        except Exception:
            pass
        return None


class GraphQARetriever(GraphRetriever):
    """Graph-enhanced retriever optimized for Q&A tasks.

    Extends GraphRetriever with:
    - Automatic re-ranking based on graph distance
    - Deduplication of similar documents
    - Configurable expansion strategies

    Example:
        >>> retriever = GraphQARetriever(
        ...     vector_store=vector_store,
        ...     mode="native",
        ...     graph_collection_name="my_graph",
        ...     expansion_strategy="breadth_first"
        ... )
    """

    expansion_strategy: str = "breadth_first"  # or "depth_first"
    deduplicate: bool = True

    def _get_relevant_documents(
        self,
        query: str,
        *,
        run_manager: Optional[CallbackManagerForRetrieverRun] = None,
    ) -> List[Document]:
        """Get relevant documents with re-ranking."""
        docs = super()._get_relevant_documents(query, run_manager=run_manager)

        if self.deduplicate:
            docs = self._deduplicate(docs)

        # Re-rank: seeds first, then by graph depth
        docs.sort(key=lambda d: (
            d.metadata.get("graph_depth", 999),
            -d.metadata.get("relevance_score", 0)
        ))

        return docs

    def _deduplicate(self, docs: List[Document]) -> List[Document]:
        """Remove duplicate documents based on stable content hash."""
        seen = set()
        unique = []
        for doc in docs:
            # Use SHA256 for deterministic, collision-resistant hashing
            content_hash = hashlib.sha256(
                doc.page_content[:200].encode("utf-8")
            ).hexdigest()
            if content_hash not in seen:
                seen.add(content_hash)
                unique.append(doc)
        return unique
