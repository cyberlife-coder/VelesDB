"""Haystack 2.x DocumentStore integration for VelesDB."""

from haystack_velesdb.document_store import VelesDBDocumentStore
from haystack_velesdb.retriever import VelesDBEmbeddingRetriever

__all__ = ["VelesDBDocumentStore", "VelesDBEmbeddingRetriever"]
__version__ = "3.12.0"
