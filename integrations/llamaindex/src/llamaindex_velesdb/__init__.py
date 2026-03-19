"""LlamaIndex VelesDB Vector Store integration.

This package provides a VelesDB-backed vector store for LlamaIndex,
enabling high-performance semantic search in RAG applications.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore
    >>> from llama_index.core import VectorStoreIndex
    >>>
    >>> vector_store = VelesDBVectorStore(path="./data")
    >>> index = VectorStoreIndex.from_vector_store(vector_store)
"""

from llamaindex_velesdb.vectorstore import VelesDBVectorStore
from llamaindex_velesdb.graph_loader import GraphLoader
from llamaindex_velesdb.graph_retriever import GraphRetriever, GraphQARetriever
from llamaindex_velesdb.security import SecurityError

# Memory classes require velesdb native extension - optional import
try:
    from llamaindex_velesdb.memory import (
        VelesDBSemanticMemory,
        VelesDBEpisodicMemory,
        VelesDBProceduralMemory,
    )
    _HAS_MEMORY = True
except ImportError:
    VelesDBSemanticMemory = None  # type: ignore
    VelesDBEpisodicMemory = None  # type: ignore
    VelesDBProceduralMemory = None  # type: ignore
    _HAS_MEMORY = False

__all__ = [
    "VelesDBVectorStore",
    "GraphLoader",
    "GraphRetriever",
    "GraphQARetriever",
    "SecurityError",
]

if _HAS_MEMORY:
    __all__.extend([
        "VelesDBSemanticMemory",
        "VelesDBEpisodicMemory",
        "VelesDBProceduralMemory",
    ])
__version__ = "1.6.0"
