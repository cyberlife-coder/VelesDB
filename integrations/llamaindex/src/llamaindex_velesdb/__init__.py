"""LlamaIndex VelesDB Vector Store integration.

This package provides a VelesDB-backed vector store for LlamaIndex,
enabling high-performance semantic search in RAG applications.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore
    >>> from llama_index.core import StorageContext, VectorStoreIndex
    >>>
    >>> # Index new documents into VelesDB (StorageContext is required —
    >>> # from_documents() ignores a bare vector_store= keyword):
    >>> vector_store = VelesDBVectorStore(path="./data")
    >>> storage_context = StorageContext.from_defaults(vector_store=vector_store)
    >>> index = VectorStoreIndex.from_documents(
    ...     documents, storage_context=storage_context
    ... )
    >>>
    >>> # Or attach to data already stored in VelesDB:
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
        VelesDBChatMemory,
        VelesDBProceduralMemory,
    )
    _HAS_MEMORY = True
except ImportError as e:
    import logging
    logging.getLogger(__name__).debug("Optional import failed: %s", e)
    VelesDBSemanticMemory = None  # type: ignore
    VelesDBEpisodicMemory = None  # type: ignore
    VelesDBChatMemory = None  # type: ignore
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
        "VelesDBChatMemory",
        "VelesDBProceduralMemory",
    ])
__version__ = "3.3.0"
