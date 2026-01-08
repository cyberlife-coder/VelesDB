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

__all__ = ["VelesDBVectorStore"]
__version__ = "0.8.10"
