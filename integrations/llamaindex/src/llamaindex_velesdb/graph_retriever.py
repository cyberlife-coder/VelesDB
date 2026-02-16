"""VelesDB Graph Retriever for LlamaIndex.

Provides a retriever that combines vector search with graph traversal
for context expansion in RAG applications.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore, GraphRetriever
    >>> from llama_index.core import VectorStoreIndex
    >>> 
    >>> vector_store = VelesDBVectorStore(path="./db", collection="docs")
    >>> index = VectorStoreIndex.from_vector_store(vector_store)
    >>> retriever = GraphRetriever(
    ...     index=index,
    ...     server_url="http://localhost:8080",
    ...     max_depth=2
    ... )
    >>> 
    >>> nodes = retriever.retrieve("What is machine learning?")
"""

from typing import Any, Dict, List, Optional
from dataclasses import dataclass, field
import logging
import requests

from velesdb_common import (
    validate_url,
    validate_k,
    validate_timeout,
    SecurityError,
)

logger = logging.getLogger(__name__)

try:
    from llama_index.core.retrievers import BaseRetriever
    from llama_index.core.schema import NodeWithScore, QueryBundle, TextNode
except ImportError:
    # Fallback for older llama-index versions
    from llama_index.retrievers import BaseRetriever
    from llama_index.schema import NodeWithScore, QueryBundle, TextNode


@dataclass
class TraversalResult:
    """Result from graph traversal."""
    target_id: int
    depth: int
    path: List[int] = field(default_factory=list)


class GraphRetriever(BaseRetriever):
    """Retriever that uses graph traversal for context expansion.
    
    This retriever implements the "seed + expand" pattern:
    1. Vector search to find initial seed nodes
    2. Graph traversal to expand context to related nodes
    3. Return combined results for RAG
    
    Args:
        index: VectorStoreIndex with VelesDBVectorStore
        server_url: URL of VelesDB server (for graph operations)
        seed_k: Number of initial vector search results (default: 3)
        expand_k: Maximum nodes to include after expansion (default: 10)
        max_depth: Maximum graph traversal depth (default: 2)
        rel_types: Relationship types to follow (default: all)
        low_latency: If True, skip graph expansion for minimal latency (default: False)
        timeout_ms: Timeout for graph operations in milliseconds (default: 1000)
        fallback_on_timeout: If True, return vector-only results on timeout (default: True)
        
    Example:
        >>> # Full graph expansion mode
        >>> retriever = GraphRetriever(
        ...     index=index,
        ...     server_url="http://localhost:8080",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>> 
        >>> # Minimal latency mode (vector search only)
        >>> fast_retriever = GraphRetriever(
        ...     index=index,
        ...     low_latency=True  # Skips graph expansion
        ... )
        >>> 
        >>> # Balanced mode with short timeout
        >>> balanced_retriever = GraphRetriever(
        ...     index=index,
        ...     timeout_ms=200,  # 200ms timeout
        ...     fallback_on_timeout=True
        ... )
    """
    
    def __init__(
        self,
        index: Any,  # VectorStoreIndex
        server_url: str = "http://localhost:8080",
        seed_k: int = 3,
        expand_k: int = 10,
        max_depth: int = 2,
        rel_types: Optional[List[str]] = None,
        collection_name: Optional[str] = None,
        low_latency: bool = False,
        timeout_ms: int = 1000,
        fallback_on_timeout: bool = True,
        **kwargs: Any,
    ):
        super().__init__(**kwargs)
        
        # Security: Validate inputs
        validate_url(server_url)
        validate_k(seed_k, "seed_k")
        validate_k(expand_k, "expand_k")
        
        self._index = index
        self._server_url = server_url
        self._seed_k = seed_k
        self._expand_k = expand_k
        self._max_depth = max_depth
        self._rel_types = rel_types or []
        self._collection_name = collection_name or self._infer_collection_name()
        self._low_latency = low_latency
        self._timeout_ms = timeout_ms
        self._fallback_on_timeout = fallback_on_timeout
    
    def _infer_collection_name(self) -> str:
        """Try to infer collection name from index's vector store."""
        try:
            vs = self._index._vector_store
            if hasattr(vs, "_collection_name"):
                return vs._collection_name
            if hasattr(vs, "collection_name"):
                return vs.collection_name
        except Exception:
            pass
        return "default"
    
    def _retrieve(self, query_bundle: QueryBundle) -> List[NodeWithScore]:
        """Retrieve nodes using vector search + graph expansion.
        
        Args:
            query_bundle: Query bundle with query string
            
        Returns:
            List of NodeWithScore objects
        """
        query_str = query_bundle.query_str
        
        # Step 1: Vector search for seed nodes
        k = self._seed_k if self._low_latency else self._expand_k
        base_retriever = self._index.as_retriever(similarity_top_k=k)
        seed_nodes = base_retriever.retrieve(query_str)
        
        if not seed_nodes:
            return []
        
        # LOW LATENCY MODE: Skip graph expansion entirely
        if self._low_latency:
            for node_with_score in seed_nodes[:self._expand_k]:
                node_with_score.node.metadata["graph_depth"] = 0
                node_with_score.node.metadata["retrieval_mode"] = "vector_only"
            return seed_nodes[:self._expand_k]
        
        # Step 2: Graph traversal for context expansion
        expanded_ids = set()
        seed_map = {}
        graph_available = True
        
        for node_with_score in seed_nodes:
            node = node_with_score.node
            node_id = self._extract_node_id(node)
            
            if node_id is not None:
                seed_map[node_id] = node_with_score
                expanded_ids.add(node_id)
                
                # Traverse graph from this seed (with timeout)
                if graph_available:
                    try:
                        neighbors = self._traverse_graph(node_id)
                        for neighbor_id in neighbors:
                            expanded_ids.add(neighbor_id)
                    except requests.exceptions.Timeout:
                        # Timeout: disable graph for remaining seeds
                        logger.warning(f"Graph traversal timeout for node {node_id}, falling back to vector-only")
                        if self._fallback_on_timeout:
                            graph_available = False
                        else:
                            raise
                    except Exception as e:
                        # Graph traversal is optional - continue without it
                        logger.debug(f"Graph traversal failed for node {node_id}: {e}")
        
        # Step 3: Build result list
        results = []
        
        # Add seed nodes first (highest relevance)
        for node_id, node_with_score in seed_map.items():
            node_with_score.node.metadata["graph_depth"] = 0
            node_with_score.node.metadata["retrieval_mode"] = "graph_expanded" if graph_available else "vector_fallback"
            results.append(node_with_score)
        
        # Fetch neighbor nodes (only if graph was used)
        if graph_available:
            remaining_slots = self._expand_k - len(results)
            neighbor_ids = [
                nid for nid in expanded_ids 
                if nid not in seed_map
            ][:remaining_slots]
            
            for neighbor_id in neighbor_ids:
                try:
                    neighbor_node = self._fetch_node(neighbor_id)
                    if neighbor_node:
                        neighbor_node.metadata["graph_depth"] = 1
                        neighbor_node.metadata["retrieval_mode"] = "graph_expanded"
                        results.append(NodeWithScore(
                            node=neighbor_node,
                            score=0.5  # Lower score for expanded nodes
                        ))
                except Exception as e:
                    logger.debug(f"Failed to fetch neighbor node {neighbor_id}: {e}")
        
        return results[:self._expand_k]
    
    def _extract_node_id(self, node: Any) -> Optional[int]:
        """Extract numeric node ID from a LlamaIndex node."""
        try:
            # Try metadata first
            if hasattr(node, "metadata"):
                for key in ["id", "doc_id", "node_id"]:
                    if key in node.metadata:
                        val = node.metadata[key]
                        return int(val) if isinstance(val, (int, str)) else None
            
            # Try node_id attribute
            if hasattr(node, "node_id"):
                try:
                    return int(node.node_id)
                except (ValueError, TypeError):
                    pass
        except Exception:
            pass
        return None
    
    def _traverse_graph(self, source_id: int) -> List[int]:
        """Traverse graph from source node.
        
        Args:
            source_id: Starting node ID
            
        Returns:
            List of neighbor node IDs
            
        Raises:
            requests.exceptions.Timeout: If request exceeds timeout_ms
        """
        url = f"{self._server_url}/collections/{self._collection_name}/graph/traverse"
        
        payload = {
            "source": source_id,
            "strategy": "bfs",
            "max_depth": self._max_depth,
            "limit": self._expand_k * 2,
            "rel_types": self._rel_types,
        }
        
        # Convert timeout_ms to seconds for requests
        timeout_sec = self._timeout_ms / 1000.0
        
        response = requests.post(url, json=payload, timeout=timeout_sec)
        
        if response.status_code == 200:
            data = response.json()
            return [r["target_id"] for r in data.get("results", [])]
        
        return []
    
    def _fetch_node(self, node_id: int) -> Optional[TextNode]:
        """Fetch a node by ID from the vector store.
        
        Args:
            node_id: Node ID
            
        Returns:
            TextNode or None if not found
        """
        try:
            # Reason: vectorstore exposes get_nodes([ids]), not get_by_id(id)
            vs = self._index._vector_store
            if hasattr(vs, "get_nodes"):
                nodes = vs.get_nodes([str(node_id)])
                if nodes:
                    return nodes[0]
        except Exception:
            pass
        return None


class GraphQARetriever(GraphRetriever):
    """Graph-enhanced retriever optimized for Q&A tasks.
    
    Extends GraphRetriever with:
    - Automatic re-ranking based on graph distance
    - Support for multi-hop reasoning
    """
    
    def __init__(
        self,
        index: Any,
        rerank_by_depth: bool = True,
        **kwargs: Any,
    ):
        super().__init__(index=index, **kwargs)
        self._rerank_by_depth = rerank_by_depth
    
    def _retrieve(self, query_bundle: QueryBundle) -> List[NodeWithScore]:
        """Retrieve with re-ranking."""
        nodes = super()._retrieve(query_bundle)
        
        if self._rerank_by_depth:
            # Sort by graph depth, then by score
            nodes.sort(key=lambda n: (
                n.node.metadata.get("graph_depth", 999),
                -n.score
            ))
        
        return nodes
