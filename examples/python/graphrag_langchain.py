#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LangChain

Demonstrates the "Seed + Expand" pattern for Graph-enhanced RAG, natively
(no server):
1. Vector search finds seed documents (``VelesDBVectorStore``)
2. ``GraphRetriever`` expands them through a VelesDB graph collection
3. The combined context goes to the LLM

The graph collection lives in the vector store's database; its node ids are
the documents' VelesDB point ids, which ``GraphRetriever`` expands from.

Requirements:
    pip install velesdb langchain-velesdb langchain-core
    pip install langchain-openai  # for the OpenAI LLM and embeddings

Usage:
    export OPENAI_API_KEY=your-key
    python graphrag_langchain.py

    # Offline: deterministic fake embeddings and a canned LLM, no API key.
    # Used automatically when OPENAI_API_KEY is unset.
    python graphrag_langchain.py --offline
"""

import argparse
import os
import shutil
import tempfile
from typing import Dict, List, Tuple

from langchain_core.documents import Document
from langchain_core.prompts import ChatPromptTemplate

from langchain_velesdb import GraphRetriever, VelesDBVectorStore

# (document id, text) — the id is the LangChain document id.
DOCUMENTS: List[Tuple[str, str]] = [
    ("ml", "Machine learning is a subset of artificial intelligence."),
    ("dl", "Deep learning uses neural networks with many layers."),
    ("nlp", "Natural language processing enables computers to understand text."),
    ("transformers", "Transformers revolutionized NLP with attention mechanisms."),
    ("gpt", "GPT models are based on the transformer architecture."),
    ("bert", "BERT is a bidirectional transformer for language understanding."),
    ("vectordb", "Vector databases store embeddings for similarity search."),
    ("velesdb", "VelesDB combines vector search with graph traversal."),
]

# (source, target, label) between document ids.
RELATIONS: List[Tuple[str, str, str]] = [
    ("ml", "dl", "INCLUDES"),
    ("ml", "nlp", "INCLUDES"),
    ("nlp", "transformers", "USES"),
    ("transformers", "gpt", "BASIS_FOR"),
    ("transformers", "bert", "BASIS_FOR"),
    ("velesdb", "vectordb", "IS_A"),
]

GRAPH_COLLECTION = "knowledge_graph"


def make_models(offline: bool):
    """OpenAI embeddings and chat model, or deterministic offline stand-ins."""
    if offline:
        from langchain_core.embeddings import DeterministicFakeEmbedding
        from langchain_core.language_models.fake_chat_models import FakeListChatModel

        canned = "(offline) An answer would be generated from the context above."
        return DeterministicFakeEmbedding(size=64), FakeListChatModel(responses=[canned])

    from langchain_openai import ChatOpenAI, OpenAIEmbeddings

    return OpenAIEmbeddings(), ChatOpenAI(model="gpt-4o-mini", temperature=0)


def create_knowledge_base(db_path: str, embeddings) -> VelesDBVectorStore:
    """Store the documents, then link them in a graph collection."""
    vector_store = VelesDBVectorStore(
        embedding=embeddings, path=db_path, collection_name="knowledge_base"
    )
    ids = [doc_id for doc_id, _ in DOCUMENTS]
    vector_store.add_texts([text for _, text in DOCUMENTS], ids=ids)

    # GraphRetriever expands from each hit's VelesDB point id, which every
    # returned document carries as metadata["_int_id"].
    point_ids: Dict[str, int] = {
        doc_id: vector_store.get_by_ids([doc_id])[0].metadata["_int_id"] for doc_id in ids
    }

    # The graph collection shares the vector store's database (one handle:
    # core refuses a second Database on the same path).
    graph = vector_store._get_db().create_graph_collection(GRAPH_COLLECTION)
    for doc_id, point_id in point_ids.items():
        graph.upsert_node_payload(point_id, {"doc_id": doc_id})  # edges need both ends
    for edge_id, (source, target, label) in enumerate(RELATIONS, start=1):
        graph.add_edge({
            "id": edge_id,
            "source": point_ids[source],
            "target": point_ids[target],
            "label": label,
        })

    print(f"Knowledge base: {len(DOCUMENTS)} documents, {len(RELATIONS)} relations")
    return vector_store


def retrieve(vector_store: VelesDBVectorStore, query: str, use_graph: bool) -> List[Document]:
    """Seed + expand with GraphRetriever, or vector hits only."""
    retriever = GraphRetriever(
        vector_store=vector_store,
        mode="native",
        graph_collection_name=GRAPH_COLLECTION,
        seed_k=2,      # vector hits the graph is expanded from
        expand_k=5,    # max documents returned, seeds included
        max_depth=2,   # BFS traversal depth
        low_latency=not use_graph,
    )
    docs = retriever.invoke(query)
    for i, doc in enumerate(docs, start=1):
        depth = doc.metadata["graph_depth"]
        marker = "seed" if depth == 0 else f"graph, depth {depth}"
        print(f"  [{i}] ({marker}) {doc.page_content}")
    return docs


def answer(llm, docs: List[Document], query: str) -> str:
    """Generate an answer from the retrieved context."""
    prompt = ChatPromptTemplate.from_messages([
        ("system", "You are a helpful AI assistant. Answer based on the context provided."),
        ("human", "Context:\n{context}\n\nQuestion: {question}\n\nAnswer:"),
    ])
    context = "\n".join(doc.page_content for doc in docs)
    return (prompt | llm).invoke({"context": context, "question": query}).content


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    parser.add_argument(
        "--offline", action="store_true",
        help="fake embeddings and LLM, no API key (default when OPENAI_API_KEY is unset)",
    )
    offline = parser.parse_args().offline or not os.getenv("OPENAI_API_KEY")
    embeddings, llm = make_models(offline)
    print("VelesDB GraphRAG with LangChain" + (" (offline)" if offline else ""))

    db_path = tempfile.mkdtemp(prefix="graphrag_langchain_")
    try:
        vector_store = create_knowledge_base(db_path, embeddings)
        for query in (
            "What is the relationship between transformers and GPT?",
            "How does VelesDB differ from other vector databases?",
        ):
            for use_graph in (True, False):
                mode = "GraphRAG (vector + graph)" if use_graph else "vector only"
                print(f"\nQuestion: {query}\nRetrieved, {mode}:")
                docs = retrieve(vector_store, query, use_graph)
                print(f"Answer: {answer(llm, docs, query)}")
    finally:
        shutil.rmtree(db_path, ignore_errors=True)


if __name__ == "__main__":
    main()
