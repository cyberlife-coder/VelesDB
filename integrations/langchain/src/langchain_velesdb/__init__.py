"""LangChain integration for VelesDB vector database.

This package provides a LangChain VectorStore implementation for VelesDB,
enabling seamless integration with LangChain's retrieval and RAG pipelines.

Example:
    >>> from langchain_velesdb import VelesDBVectorStore
    >>> from langchain_openai import OpenAIEmbeddings
    >>>
    >>> vectorstore = VelesDBVectorStore(
    ...     path="./my_data",
    ...     collection_name="documents",
    ...     embedding=OpenAIEmbeddings()
    ... )
    >>>
    >>> # Add documents
    >>> vectorstore.add_texts(["Hello world", "VelesDB is fast"])
    >>>
    >>> # Search
    >>> results = vectorstore.similarity_search("greeting", k=1)
"""

from langchain_velesdb.vectorstore import VelesDBVectorStore
from langchain_velesdb.graph_retriever import GraphRetriever, GraphQARetriever
from velesdb_common import SecurityError

# Memory classes require full langchain - optional import
try:
    from langchain_velesdb.memory import VelesDBChatMemory, VelesDBSemanticMemory
    _HAS_MEMORY = True
except ImportError:
    VelesDBChatMemory = None  # type: ignore
    VelesDBSemanticMemory = None  # type: ignore
    _HAS_MEMORY = False

__all__ = [
    "VelesDBVectorStore",
    "GraphRetriever",
    "GraphQARetriever",
    "SecurityError",
]

if _HAS_MEMORY:
    __all__.extend(["VelesDBChatMemory", "VelesDBSemanticMemory"])
__version__ = "0.8.10"
