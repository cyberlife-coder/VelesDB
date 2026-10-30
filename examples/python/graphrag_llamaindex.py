#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LlamaIndex

Demonstrates Graph-enhanced RAG natively (no server):
1. Index chunked nodes in ``VelesDBVectorStore``
2. ``GraphLoader.load_from_nodes(extract_relations=True)`` turns the nodes'
   ``relationships`` (``NEXT``/``PREVIOUS`` between chunks, ``PARENT``/``CHILD``
   with their section) into edges of a VelesDB graph collection
3. ``GraphRetriever`` seeds by vector search and expands through those edges
4. A query engine answers from the combined context

Requirements:
    pip install velesdb llama-index-vector-stores-velesdb llama-index-core
    pip install llama-index-llms-openai llama-index-embeddings-openai  # for OpenAI

Usage:
    export OPENAI_API_KEY=your-key
    python graphrag_llamaindex.py

    # Offline: LlamaIndex's MockEmbedding and MockLLM, no API key.
    # Used automatically when OPENAI_API_KEY is unset.
    python graphrag_llamaindex.py --offline
"""

import argparse
import os
import shutil
import tempfile
from typing import List

from llama_index.core import StorageContext, VectorStoreIndex
from llama_index.core.query_engine import RetrieverQueryEngine
from llama_index.core.schema import NodeRelationship, RelatedNodeInfo, TextNode

from llamaindex_velesdb import GraphLoader, GraphRetriever, VelesDBVectorStore

GRAPH_COLLECTION = "knowledge_graph"

SECTION = ("guide", "VelesDB in one page: a local-first vector, graph and column database.")
CHUNKS = [
    ("chunk-1", "VelesDB stores vectors in HNSW indexes and searches them with SIMD kernels."),
    ("chunk-2", "Its graph collections hold nodes and edges, traversed by BFS or DFS."),
    ("chunk-3", "VelesQL queries combine NEAR vector search with MATCH graph patterns."),
    ("chunk-4", "Everything runs in-process from a single binary, with no server needed."),
]


def make_models(offline: bool):
    """OpenAI embedding model and LLM, or LlamaIndex's mock ones."""
    if offline:
        from llama_index.core.embeddings import MockEmbedding
        from llama_index.core.llms import MockLLM

        return MockEmbedding(embed_dim=64), MockLLM(max_tokens=16)

    from llama_index.embeddings.openai import OpenAIEmbedding
    from llama_index.llms.openai import OpenAI

    return OpenAIEmbedding(), OpenAI(model="gpt-4o-mini", temperature=0)


def build_nodes() -> List[TextNode]:
    """A section and its chunks, linked the way a LlamaIndex node parser links them."""
    section = TextNode(id_=SECTION[0], text=SECTION[1])
    chunks = [TextNode(id_=node_id, text=text) for node_id, text in CHUNKS]
    section.relationships[NodeRelationship.CHILD] = [
        RelatedNodeInfo(node_id=chunk.node_id) for chunk in chunks
    ]
    for previous, chunk in zip([None] + chunks, chunks):
        chunk.relationships[NodeRelationship.PARENT] = RelatedNodeInfo(node_id=section.node_id)
        if previous is not None:
            chunk.relationships[NodeRelationship.PREVIOUS] = RelatedNodeInfo(
                node_id=previous.node_id
            )
            previous.relationships[NodeRelationship.NEXT] = RelatedNodeInfo(
                node_id=chunk.node_id
            )
    return [section] + chunks


def create_knowledge_graph(db_path: str, embed_model):
    """Index the nodes, then load their relationships as graph edges."""
    nodes = build_nodes()
    vector_store = VelesDBVectorStore(path=db_path, collection_name="guide")
    index = VectorStoreIndex(
        nodes,
        storage_context=StorageContext.from_defaults(vector_store=vector_store),
        embed_model=embed_model,
    )

    # The graph collection shares the vector store's database. Its node ids
    # are the nodes' VelesDB point ids, which GraphRetriever expands from.
    vector_store.client.create_graph_collection(GRAPH_COLLECTION)
    loader = GraphLoader(vector_store, graph_collection_name=GRAPH_COLLECTION)
    counts = loader.load_from_nodes(nodes, extract_relations=True)
    print(f"Knowledge graph: {counts['nodes']} nodes, {counts['edges']} edges")
    return index


def query(index: VectorStoreIndex, llm, question: str, use_graph: bool) -> None:
    """Retrieve (seed + expand, or vector hits only), then answer."""
    retriever = GraphRetriever(
        index=index,
        mode="native",
        graph_collection_name=GRAPH_COLLECTION,
        seed_k=1,      # vector hits the graph is expanded from
        expand_k=4,    # max nodes returned, seeds included
        max_depth=1,   # neighbours one edge away
        low_latency=not use_graph,
    )
    print(f"\nQuestion: {question}")
    print("Retrieved, " + ("GraphRAG (vector + graph):" if use_graph else "vector only:"))
    for i, hit in enumerate(retriever.retrieve(question), start=1):
        depth = hit.node.metadata["graph_depth"]
        marker = "seed" if depth == 0 else f"graph, depth {depth}"
        print(f"  [{i}] ({marker}) {hit.node.get_content()}")

    engine = RetrieverQueryEngine.from_args(retriever, llm=llm)
    print(f"Answer: {engine.query(question)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    parser.add_argument(
        "--offline", action="store_true",
        help="mock embeddings and LLM, no API key (default when OPENAI_API_KEY is unset)",
    )
    offline = parser.parse_args().offline or not os.getenv("OPENAI_API_KEY")
    embed_model, llm = make_models(offline)
    print("VelesDB GraphRAG with LlamaIndex" + (" (offline)" if offline else ""))

    db_path = tempfile.mkdtemp(prefix="graphrag_llamaindex_")
    try:
        index = create_knowledge_graph(db_path, embed_model)
        for question in (
            "How does VelesDB traverse graphs?",
            "Does VelesDB need a server?",
        ):
            for use_graph in (True, False):
                query(index, llm, question, use_graph)
    finally:
        shutil.rmtree(db_path, ignore_errors=True)


if __name__ == "__main__":
    main()
