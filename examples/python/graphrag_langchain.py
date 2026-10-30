#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LangChain

Demonstrates the "Seed + Expand" pattern for Graph-enhanced RAG:
1. Vector search to find initial seed documents
2. Graph traversal to expand context to related documents
3. Combined context for LLM generation

Requirements:
    pip install velesdb langchain-core langchain-openai httpx

Usage:
    export OPENAI_API_KEY=your-key
    python graphrag_langchain.py
"""

import os
import shutil
import sys
from pathlib import Path
from typing import List

import httpx

# LangChain imports
from langchain_core.documents import Document
from langchain_core.prompts import ChatPromptTemplate
from langchain_openai import ChatOpenAI, OpenAIEmbeddings

# VelesDB LangChain integration — use the local example implementation.
# For production, install: pip install langchain-velesdb
sys.path.insert(0, str(Path(__file__).parent.parent / "langchain"))
from hybrid_search import VelesDBVectorStore  # noqa: E402


class GraphRetriever:
    """Graph-enhanced retriever that expands seed results via VelesDB graph API."""

    def __init__(
        self,
        vector_store: VelesDBVectorStore,
        server_url: str,
        seed_k: int = 2,
        expand_k: int = 5,
        max_depth: int = 2,
        collection: str = "knowledge_base",
    ) -> None:
        self._vector_store = vector_store
        self._server_url = server_url.rstrip("/")
        self._collection = collection
        self._seed_k = seed_k
        self._expand_k = expand_k
        self._max_depth = max_depth

    def invoke(self, query: str) -> List[Document]:
        """Retrieve seed documents by vector search then expand via graph traversal."""
        seed_docs = self._vector_store.similarity_search(query, k=self._seed_k)
        expanded: List[Document] = list(seed_docs)
        seen_texts = {d.page_content for d in seed_docs}

        for doc in seed_docs:
            doc_id = doc.metadata.get("id")
            if doc_id is None:
                continue
            try:
                response = httpx.post(
                    f"{self._server_url}/collections/{self._collection}/graph/traverse",
                    json={
                        "source": int(doc_id),
                        "max_depth": self._max_depth,
                        "limit": self._expand_k,
                    },
                    timeout=5.0,
                )
                response.raise_for_status()
                # TraverseResponse: each hit has target_id (a u64 serialised
                # as a string) and depth (hops from the seed).
                for hit in response.json().get("results", []):
                    neighbor_id = hit["target_id"]
                    # Fetch the neighbor document by its u64 ID directly,
                    # not via similarity search on the string representation
                    # of the integer (which would match arbitrary text instead
                    # of retrieving the correct node).
                    neighbor_points = self._vector_store._collection.get(
                        [int(neighbor_id)]
                    )
                    for point in neighbor_points:
                        payload = point.get("payload", {})
                        text = payload.get("text", "")
                        if text and text not in seen_texts:
                            seen_texts.add(text)
                            original_id = payload.get(
                                "_original_id", str(point.get("id", ""))
                            )
                            meta = {
                                k: v
                                for k, v in payload.items()
                                if k not in ("text", "_original_id")
                            }
                            meta["id"] = original_id
                            meta["graph_depth"] = hit["depth"]
                            expanded.append(
                                Document(page_content=text, metadata=meta)
                            )
            except httpx.HTTPError:
                # Graph expansion is best-effort; fall back gracefully.
                pass

        return expanded[: self._expand_k]


VELESDB_SERVER = "http://localhost:8080"


def add_graph_edge(collection: str, edge_id: int, source: int, target: int, label: str):
    """Add a graph edge via VelesDB HTTP API."""
    url = f"{VELESDB_SERVER}/collections/{collection}/graph/edges"
    payload = {
        "id": edge_id,
        "source": source,
        "target": target,
        "label": label,
    }
    try:
        response = httpx.post(url, json=payload, timeout=5.0)
        response.raise_for_status()
    except httpx.HTTPError as e:
        print(f"Warning: Could not add edge {edge_id}: {e}")


def create_sample_knowledge_base(db_path: str) -> VelesDBVectorStore:
    """Create a sample knowledge base with documents and relations."""

    embeddings = OpenAIEmbeddings()

    # Initialize VelesDB with local storage
    vectorstore = VelesDBVectorStore.from_texts(
        texts=[
            "Machine learning is a subset of artificial intelligence.",
            "Deep learning uses neural networks with many layers.",
            "Natural language processing enables computers to understand text.",
            "Transformers revolutionized NLP with attention mechanisms.",
            "GPT models are based on the transformer architecture.",
            "BERT is a bidirectional transformer for language understanding.",
            "Vector databases store embeddings for similarity search.",
            "VelesDB combines vector search with graph traversal.",
        ],
        embedding=embeddings,
        metadatas=[
            {"id": 1, "topic": "ML", "category": "fundamentals"},
            {"id": 2, "topic": "DL", "category": "fundamentals"},
            {"id": 3, "topic": "NLP", "category": "fundamentals"},
            {"id": 4, "topic": "Transformers", "category": "architecture"},
            {"id": 5, "topic": "GPT", "category": "models"},
            {"id": 6, "topic": "BERT", "category": "models"},
            {"id": 7, "topic": "VectorDB", "category": "infrastructure"},
            {"id": 8, "topic": "VelesDB", "category": "infrastructure"},
        ],
        db_path=db_path,
        collection_name="knowledge_base",
        dimension=1536,
    )

    # Add graph edges via HTTP API (concept relationships)
    collection_name = "knowledge_base"

    # ML -> DL (ML includes DL)
    add_graph_edge(collection_name, 1, source=1, target=2, label="INCLUDES")
    # ML -> NLP (ML includes NLP)
    add_graph_edge(collection_name, 2, source=1, target=3, label="INCLUDES")
    # NLP -> Transformers (NLP uses Transformers)
    add_graph_edge(collection_name, 3, source=3, target=4, label="USES")
    # Transformers -> GPT (Transformers basis for GPT)
    add_graph_edge(collection_name, 4, source=4, target=5, label="BASIS_FOR")
    # Transformers -> BERT (Transformers basis for BERT)
    add_graph_edge(collection_name, 5, source=4, target=6, label="BASIS_FOR")
    # VelesDB -> VectorDB (VelesDB is a VectorDB)
    add_graph_edge(collection_name, 6, source=8, target=7, label="IS_A")

    print("Knowledge base created with 8 documents and 6 relationships")
    return vectorstore


def graphrag_query(
    vectorstore: VelesDBVectorStore,
    query: str,
    use_graph: bool = True,
) -> str:
    """
    Execute a GraphRAG query.

    Args:
        vectorstore: VelesDB vector store with graph layer
        query: User question
        use_graph: Whether to use graph expansion (True) or vector-only (False)

    Returns:
        LLM-generated answer
    """

    # Create retriever (with or without graph expansion)
    if use_graph:
        retriever = GraphRetriever(
            vector_store=vectorstore,
            server_url=VELESDB_SERVER,
            seed_k=2,      # Initial vector search results
            expand_k=5,    # Max results after graph expansion
            max_depth=2,   # Graph traversal depth
        )
        mode = "GraphRAG (vector + graph)"
    else:
        retriever = vectorstore.as_retriever(search_kwargs={"k": 5})
        mode = "Standard RAG (vector only)"

    # Retrieve relevant documents
    docs: List[Document] = retriever.invoke(query)

    print(f"\nRetrieved {len(docs)} documents using {mode}:")
    for i, doc in enumerate(docs):
        depth = doc.metadata.get("graph_depth", 0)
        marker = "seed" if depth == 0 else "graph"
        print(f"  [{marker}] [{i+1}] {doc.page_content[:60]}...")

    # Create prompt with retrieved context
    context = "\n".join([doc.page_content for doc in docs])

    prompt = ChatPromptTemplate.from_messages([
        ("system", "You are a helpful AI assistant. Answer based on the context provided."),
        ("human", "Context:\n{context}\n\nQuestion: {question}\n\nAnswer:"),
    ])

    # Generate answer with LLM
    llm = ChatOpenAI(model="gpt-4o-mini", temperature=0)
    chain = prompt | llm

    response = chain.invoke({"context": context, "question": query})
    return response.content


def main():
    """Run GraphRAG demonstration."""

    print("=" * 60)
    print("VelesDB GraphRAG Demo")
    print("=" * 60)

    # Check for API key
    if not os.getenv("OPENAI_API_KEY"):
        print("Warning: Set OPENAI_API_KEY environment variable")
        print("   For demo without LLM, the retrieval still works.")
        return

    DB_PATH = "./graphrag_demo"
    try:
        # Create knowledge base
        vectorstore = create_sample_knowledge_base(DB_PATH)

        # Query examples
        queries = [
            "What is the relationship between transformers and GPT?",
            "How does VelesDB differ from other vector databases?",
        ]

        for query in queries:
            print(f"\n{'=' * 60}")
            print(f"Question: {query}")
            print("=" * 60)

            # Compare GraphRAG vs Standard RAG
            print("\n--- GraphRAG Mode ---")
            answer_graph = graphrag_query(vectorstore, query, use_graph=True)
            print(f"\nAnswer: {answer_graph}")

            print("\n--- Standard RAG Mode ---")
            answer_vector = graphrag_query(vectorstore, query, use_graph=False)
            print(f"\nAnswer: {answer_vector}")

        print("\n" + "=" * 60)
        print("Demo complete!")
        print("=" * 60)
    finally:
        shutil.rmtree(DB_PATH, ignore_errors=True)


if __name__ == "__main__":
    main()
