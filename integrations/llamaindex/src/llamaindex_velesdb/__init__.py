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

__all__ = [
    "VelesDBVectorStore",
    "GraphLoader",
    "GraphRetriever",
    "GraphQARetriever",
    "SecurityError",
]
__version__ = "0.8.10"
