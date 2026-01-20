"""VelesDB Graph Toolkit for entity extraction and knowledge graph construction.

This module provides tools for building knowledge graphs from unstructured text,
with support for local LLM extraction (Ollama) and semantic chunking.

Example:
    >>> from langchain_velesdb.graph_toolkit import GraphExtractor, GraphLoader
    >>> from velesdb import Database
    >>>
    >>> # Extract entities and relations from text
    >>> extractor = GraphExtractor(model="llama3")
    >>> result = extractor.extract("John works at Acme Corp in Paris.")
    >>>
    >>> # Load into VelesDB
    >>> db = Database("./my_graph")
    >>> loader = GraphLoader(db)
    >>> loader.load(result.nodes, result.edges)
"""

from langchain_velesdb.graph_toolkit.extractor import (
    GraphExtractor,
    ExtractionResult,
    Entity,
    Relation,
)
from langchain_velesdb.graph_toolkit.loader import GraphLoader
from langchain_velesdb.graph_toolkit.chunker import SemanticChunker

__all__ = [
    "GraphExtractor",
    "ExtractionResult",
    "Entity",
    "Relation",
    "GraphLoader",
    "SemanticChunker",
]
