"""End-to-end RAG example: PDF ingestion and semantic search with VelesDB.

Requires:
    pip install haystack-ai haystack-velesdb sentence-transformers

Usage:
    python rag_pipeline.py                  # uses built-in sample text
    python rag_pipeline.py --pdf paper.pdf  # ingest a real PDF
"""
from __future__ import annotations

import argparse

from haystack import Pipeline
from haystack.components.embedders import (
    SentenceTransformersDocumentEmbedder,
    SentenceTransformersTextEmbedder,
)
from haystack.components.preprocessors import DocumentSplitter
from haystack.components.writers import DocumentWriter
from haystack.dataclasses import Document
from haystack_velesdb import VelesDBDocumentStore

MODEL = "sentence-transformers/all-MiniLM-L6-v2"
STORE_PATH = "./rag_demo_store"


def build_index_pipeline(store: VelesDBDocumentStore) -> Pipeline:
    """Return a pipeline that embeds and writes documents into *store*."""
    pipeline = Pipeline()
    pipeline.add_component("splitter", DocumentSplitter(split_by="sentence", split_length=3))
    pipeline.add_component("embedder", SentenceTransformersDocumentEmbedder(model=MODEL))
    pipeline.add_component("writer", DocumentWriter(document_store=store))
    pipeline.connect("splitter", "embedder")
    pipeline.connect("embedder", "writer")
    return pipeline


def build_query_pipeline(store: VelesDBDocumentStore) -> Pipeline:
    """Return a pipeline that embeds a query and retrieves from *store*."""
    pipeline = Pipeline()
    pipeline.add_component("embedder", SentenceTransformersTextEmbedder(model=MODEL))
    pipeline.add_component("retriever", _VelesRetriever(store))
    pipeline.connect("embedder.embedding", "retriever.query_embedding")
    return pipeline


class _VelesRetriever:
    """Thin Haystack component wrapping VelesDBDocumentStore.embedding_retrieval."""

    def __init__(self, store: VelesDBDocumentStore, top_k: int = 5) -> None:
        self._store = store
        self._top_k = top_k

    def run(self, query_embedding: list) -> dict:
        docs = self._store.embedding_retrieval(query_embedding, top_k=self._top_k)
        return {"documents": docs}


SAMPLE_TEXTS = [
    "VelesDB is a local-first vector database written in Rust.",
    "It achieves microsecond retrieval latency through HNSW indexing and SIMD acceleration.",
    "The Python SDK exposes a simple collection interface for upsert, search, and scroll.",
    "Haystack is a framework for building production-grade NLP pipelines.",
    "Together, VelesDB and Haystack make a powerful local RAG stack.",
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pdf", help="Optional PDF file to ingest instead of sample texts")
    args = parser.parse_args()

    store = VelesDBDocumentStore(path=STORE_PATH, embedding_dim=384)

    # --- Indexing ---
    if args.pdf:
        from haystack.components.converters import PyPDFToDocument

        index_pipeline = Pipeline()
        index_pipeline.add_component("converter", PyPDFToDocument())
        index_pipeline.add_component("splitter", DocumentSplitter(split_by="sentence", split_length=3))
        index_pipeline.add_component("embedder", SentenceTransformersDocumentEmbedder(model=MODEL))
        index_pipeline.add_component("writer", DocumentWriter(document_store=store))
        index_pipeline.connect("converter", "splitter")
        index_pipeline.connect("splitter", "embedder")
        index_pipeline.connect("embedder", "writer")
        index_pipeline.run({"converter": {"sources": [args.pdf]}})
    else:
        docs = [Document(content=t) for t in SAMPLE_TEXTS]
        idx = build_index_pipeline(store)
        idx.run({"splitter": {"documents": docs}})

    print(f"Indexed {store.count_documents()} document chunks.")

    # --- Querying ---
    query = "How does VelesDB achieve fast retrieval?"
    q_pipeline = build_query_pipeline(store)
    result = q_pipeline.run({"embedder": {"text": query}})
    print(f"\nQuery: {query}")
    for doc in result["retriever"]["documents"]:
        print(f"  [{doc.score:.3f}] {doc.content}")


if __name__ == "__main__":
    main()
