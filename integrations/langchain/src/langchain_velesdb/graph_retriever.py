"""VelesDB Graph Retriever for LangChain.

Provides a retriever that combines vector search with graph traversal
for context expansion in RAG applications.

Example:
    >>> from langchain_velesdb import VelesDBVectorStore, GraphRetriever
    >>> 
    >>> vector_store = VelesDBVectorStore(path="./db", collection="docs")
    >>> retriever = GraphRetriever(
    ...     vector_store=vector_store,
    ...     max_depth=2,
    ...     expand_k=5
    ... )
    >>> 
    >>> # Use in RAG chain
    >>> docs = retriever.get_relevant_documents("What is machine learning?")
"""

from typing import Any, Dict, List, Optional, Callable
from dataclasses import dataclass, field
import hashlib
import requests

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


class GraphRetriever(BaseRetriever):
    """Retriever that uses graph traversal for context expansion.
    
    This retriever implements the "seed + expand" pattern:
    1. Vector search to find initial seed documents
    2. Graph traversal to expand context to related documents
    3. Return combined results for RAG
    
    Args:
        vector_store: VelesDBVectorStore instance
        server_url: URL of VelesDB server (for graph operations)
        seed_k: Number of initial vector search results (default: 3)
        expand_k: Maximum documents to include after expansion (default: 10)
        max_depth: Maximum graph traversal depth (default: 2)
        rel_types: Relationship types to follow (default: all)
        score_threshold: Minimum score for seed documents (default: 0.0)
        low_latency: If True, skip graph expansion for minimal latency (default: False)
        timeout_ms: Timeout for graph operations in milliseconds (default: 1000)
        fallback_on_timeout: If True, return vector-only results on timeout (default: True)
        
    Example:
        >>> # Full graph expansion mode
        >>> retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     server_url="http://localhost:8080",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>> 
        >>> # Minimal latency mode (vector search only)
        >>> fast_retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     low_latency=True  # Skips graph expansion
        ... )
        >>> 
        >>> # Balanced mode with short timeout
        >>> balanced_retriever = GraphRetriever(
        ...     vector_store=vector_store,
        ...     timeout_ms=200,  # 200ms timeout
        ...     fallback_on_timeout=True  # Falls back to vector-only
        ... )
    """
    
    vector_store: Any  # VelesDBVectorStore
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
    
    def _get_relevant_documents(
        self,
        query: str,
        *,
        run_manager: Optional[CallbackManagerForRetrieverRun] = None,
    ) -> List[Document]:
        """Get relevant documents using vector search + graph expansion.
        
        Args:
            query: The query string
            run_manager: Callback manager (optional)
            
        Returns:
            List of relevant documents
        """
        # Step 1: Vector search for seed documents
        seed_results = self.vector_store.similarity_search_with_score(
            query, k=self.seed_k if self.low_latency else self.expand_k
        )
        
        # Filter by score threshold
        seeds = [
            (doc, score) for doc, score in seed_results 
            if score >= self.score_threshold
        ]
        
        if not seeds:
            return []
        
        # LOW LATENCY MODE: Skip graph expansion entirely
        if self.low_latency:
            result_docs = []
            for doc, score in seeds[:self.expand_k]:
                doc.metadata["graph_depth"] = 0
                doc.metadata["relevance_score"] = score
                doc.metadata["retrieval_mode"] = "vector_only"
                result_docs.append(doc)
            return result_docs
        
        # Step 2: Graph traversal for context expansion
        expanded_ids = set()
        seed_docs = {}
        graph_available = True
        
        for doc, score in seeds:
            doc_id = doc.metadata.get("id") or doc.metadata.get("doc_id")
            if doc_id is not None:
                seed_docs[doc_id] = (doc, score)
                expanded_ids.add(doc_id)
                
                # Traverse graph from this seed (with timeout)
                if graph_available:
                    try:
                        neighbors = self._traverse_graph(doc_id)
                        for neighbor_id in neighbors:
                            expanded_ids.add(neighbor_id)
                    except requests.exceptions.Timeout:
                        # Timeout: disable graph for remaining seeds
                        if self.fallback_on_timeout:
                            graph_available = False
                        else:
                            raise
                    except Exception:
                        # Graph traversal is optional - continue without it
                        pass
        
        # Step 3: Fetch expanded documents
        result_docs = []
        
        # Add seed documents first (highest relevance)
        for doc_id, (doc, score) in seed_docs.items():
            doc.metadata["graph_depth"] = 0
            doc.metadata["relevance_score"] = score
            doc.metadata["retrieval_mode"] = "graph_expanded" if graph_available else "vector_fallback"
            result_docs.append(doc)
        
        # Fetch neighbor documents (only if graph was used)
        if graph_available:
            remaining_slots = self.expand_k - len(result_docs)
            neighbor_ids = [
                nid for nid in expanded_ids 
                if nid not in seed_docs
            ][:remaining_slots]
            
            for neighbor_id in neighbor_ids:
                try:
                    neighbor_doc = self._fetch_document(neighbor_id)
                    if neighbor_doc:
                        neighbor_doc.metadata["graph_depth"] = 1
                        neighbor_doc.metadata["retrieval_mode"] = "graph_expanded"
                        result_docs.append(neighbor_doc)
                except Exception:
                    pass
        
        return result_docs[:self.expand_k]
    
    def _traverse_graph(self, source_id: int) -> List[int]:
        """Traverse graph from source node.
        
        Args:
            source_id: Starting node ID
            
        Returns:
            List of neighbor node IDs
            
        Raises:
            requests.exceptions.Timeout: If request exceeds timeout_ms
        """
        collection = self.vector_store._collection_name
        url = f"{self.server_url}/collections/{collection}/graph/traverse"
        
        payload = {
            "source": source_id,
            "strategy": "bfs",
            "max_depth": self.max_depth,
            "limit": self.expand_k * 2,
            "rel_types": self.rel_types or [],
        }
        
        # Convert timeout_ms to seconds for requests
        timeout_sec = self.timeout_ms / 1000.0
        
        response = requests.post(url, json=payload, timeout=timeout_sec)
        
        if response.status_code == 200:
            data = response.json()
            return [r["target_id"] for r in data.get("results", [])]
        
        return []
    
    def _fetch_document(self, doc_id: int) -> Optional[Document]:
        """Fetch a document by ID from the vector store.
        
        Args:
            doc_id: Document ID
            
        Returns:
            Document or None if not found
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
