#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LlamaIndex

Demonstrates Graph-enhanced RAG using LlamaIndex's query engine:
1. Build a knowledge graph from documents
2. Use VelesDBGraphLoader to store citation edges via the VelesDB REST graph API
3. Use VelesDBGraphRetriever for context expansion
4. Generate answers with expanded context

Requirements:
    pip install llama-index-vector-stores-velesdb llama-index-llms-openai requests

Usage:
    # Start VelesDB server first: velesdb-server --port 8080
    export OPENAI_API_KEY=your-key
    python graphrag_llamaindex.py
"""

import os
from typing import List, Optional

import requests

# LlamaIndex imports
from llama_index.core import VectorStoreIndex, Settings
from llama_index.core.schema import TextNode, NodeRelationship, RelatedNodeInfo
from llama_index.llms.openai import OpenAI
from llama_index.embeddings.openai import OpenAIEmbedding

# VelesDB LlamaIndex vector store integration
# Install: pip install llama-index-vector-stores-velesdb
from llama_index.vector_stores.velesdb import VelesDBVectorStore


class VelesDBGraphLoader:
    """Loads graph edges into VelesDB via its REST graph API."""

    def __init__(self, server_url: str, collection: str = "research_papers"):
        self.server_url = server_url.rstrip("/")
        self.collection = collection

    def add_edge(self, source_id: str, target_id: str, label: str,
                 properties: Optional[dict] = None) -> None:
        """Add a directed edge between two nodes."""
        payload = {
            "collection": self.collection,
            "source_id": source_id,
            "target_id": target_id,
            "label": label,
            "properties": properties or {},
        }
        resp = requests.post(
            f"{self.server_url}/api/graph/edges",
            json=payload,
            timeout=5,
        )
        resp.raise_for_status()


class VelesDBGraphRetriever:
    """Retrieves nodes with graph expansion via the VelesDB REST API."""

    def __init__(self, index: VectorStoreIndex, server_url: str,
                 collection: str = "research_papers",
                 seed_k: int = 2, expand_k: int = 4, max_depth: int = 2,
                 low_latency: bool = False):
        self.index = index
        self.server_url = server_url.rstrip("/")
        self.collection = collection
        self.seed_k = seed_k
        self.expand_k = expand_k
        self.max_depth = max_depth
        self.low_latency = low_latency

    def retrieve(self, query: str) -> list:
        """Retrieve seed nodes by vector search then expand via graph traversal."""
        # Step 1: seed retrieval via standard vector search
        retriever = self.index.as_retriever(similarity_top_k=self.seed_k)
        seed_nodes = retriever.retrieve(query)

        if self.low_latency:
            return seed_nodes

        # Step 2: graph expansion for each seed node
        expanded = list(seed_nodes)
        seen_ids = {n.node.node_id for n in seed_nodes}

        for seed in seed_nodes:
            seed_id = seed.node.node_id
            try:
                resp = requests.post(
                    f"{self.server_url}/api/graph/traverse",
                    json={
                        "collection": self.collection,
                        "start_node": seed_id,
                        "max_depth": self.max_depth,
                        "limit": self.expand_k,
                    },
                    timeout=5,
                )
                resp.raise_for_status()
                neighbor_ids = resp.json().get("node_ids", [])

                for nid in neighbor_ids:
                    if nid not in seen_ids:
                        seen_ids.add(nid)
                        # Fetch the node from the index by ID
                        node_retriever = self.index.as_retriever(similarity_top_k=1)
                        neighbors = node_retriever.retrieve(nid)
                        for n in neighbors:
                            n.node.metadata["graph_depth"] = 1
                            n.node.metadata["retrieval_mode"] = "graph"
                            expanded.append(n)

            except requests.RequestException:
                # Graph expansion is best-effort; fall back gracefully
                pass

        return expanded


def create_knowledge_graph() -> tuple[VelesDBVectorStore, VectorStoreIndex]:
    """Create a knowledge graph with documents and relationships."""
    
    # Configure LlamaIndex settings
    Settings.llm = OpenAI(model="gpt-4o-mini", temperature=0)
    Settings.embed_model = OpenAIEmbedding()
    
    # Create VelesDB vector store
    vector_store = VelesDBVectorStore(
        path="./graphrag_llamaindex_demo",
        collection_name="research_papers",
        dimension=1536,
    )
    
    # Create sample research paper nodes
    nodes = [
        TextNode(
            text="Attention Is All You Need introduced the Transformer architecture, "
                 "replacing recurrence with self-attention mechanisms.",
            id_="paper_1",
            metadata={"title": "Attention Is All You Need", "year": 2017, "topic": "transformers"},
        ),
        TextNode(
            text="BERT: Pre-training of Deep Bidirectional Transformers showed that "
                 "bidirectional pre-training improves language understanding.",
            id_="paper_2",
            metadata={"title": "BERT", "year": 2018, "topic": "nlp"},
        ),
        TextNode(
            text="GPT-3: Language Models are Few-Shot Learners demonstrated that "
                 "scaling up language models enables few-shot learning.",
            id_="paper_3",
            metadata={"title": "GPT-3", "year": 2020, "topic": "llm"},
        ),
        TextNode(
            text="Retrieval-Augmented Generation combines retrieval with generation "
                 "to reduce hallucinations and improve factuality.",
            id_="paper_4",
            metadata={"title": "RAG", "year": 2020, "topic": "rag"},
        ),
        TextNode(
            text="GraphRAG: Unlocking LLM discovery on narrative private data uses "
                 "knowledge graphs to enhance retrieval for complex queries.",
            id_="paper_5",
            metadata={"title": "GraphRAG", "year": 2024, "topic": "graphrag"},
        ),
    ]
    
    # Add relationships between papers (citations)
    nodes[1].relationships[NodeRelationship.PARENT] = RelatedNodeInfo(
        node_id="paper_1", metadata={"relation": "cites"}
    )
    nodes[2].relationships[NodeRelationship.PARENT] = RelatedNodeInfo(
        node_id="paper_1", metadata={"relation": "cites"}
    )
    nodes[3].relationships[NodeRelationship.PARENT] = RelatedNodeInfo(
        node_id="paper_2", metadata={"relation": "extends"}
    )
    nodes[4].relationships[NodeRelationship.PARENT] = RelatedNodeInfo(
        node_id="paper_4", metadata={"relation": "extends"}
    )
    
    # Build index from nodes
    index = VectorStoreIndex.from_vector_store(vector_store)
    
    # Add nodes to index
    for node in nodes:
        vector_store.add([node])
    
    # Load graph relationships via VelesDB REST graph API
    loader = VelesDBGraphLoader(server_url="http://localhost:8080")

    # Add edges based on citations (use node IDs matching TextNode id_ fields)
    loader.add_edge(source_id="paper_2", target_id="paper_1", label="CITES")    # BERT cites Transformer
    loader.add_edge(source_id="paper_3", target_id="paper_1", label="CITES")    # GPT-3 cites Transformer
    loader.add_edge(source_id="paper_4", target_id="paper_2", label="EXTENDS")  # RAG extends BERT ideas
    loader.add_edge(source_id="paper_5", target_id="paper_4", label="EXTENDS")  # GraphRAG extends RAG
    
    print("✅ Knowledge graph created: 5 papers, 4 citation relationships")
    return vector_store, index


def query_with_graph_expansion(
    index: VectorStoreIndex,
    vector_store: VelesDBVectorStore,
    query: str,
) -> str:
    """Query using GraphRetriever for context expansion."""
    
    # Create graph-enhanced retriever
    retriever = VelesDBGraphRetriever(
        index=index,
        server_url="http://localhost:8080",  # VelesDB server for graph ops
        seed_k=2,
        expand_k=4,
        max_depth=2,
        low_latency=False,  # Enable graph expansion
    )
    
    # Retrieve with graph expansion
    nodes = retriever.retrieve(query)
    
    print(f"\n📚 Retrieved {len(nodes)} nodes:")
    for i, node_with_score in enumerate(nodes):
        node = node_with_score.node
        depth = node.metadata.get("graph_depth", 0)
        mode = node.metadata.get("retrieval_mode", "unknown")
        marker = "🎯" if depth == 0 else "🔗"
        title = node.metadata.get("title", "Unknown")
        print(f"  {marker} [{i+1}] {title} (depth={depth}, mode={mode})")
    
    # Create query engine and generate answer
    query_engine = index.as_query_engine(
        retriever=retriever,
        response_mode="compact",
    )
    
    response = query_engine.query(query)
    return str(response)


def query_vector_only(index: VectorStoreIndex, query: str) -> str:
    """Query using standard vector search only."""
    
    query_engine = index.as_query_engine(
        similarity_top_k=4,
        response_mode="compact",
    )
    
    response = query_engine.query(query)
    return str(response)


def main():
    """Run LlamaIndex GraphRAG demonstration."""
    
    print("=" * 60)
    print("VelesDB GraphRAG Demo (LlamaIndex)")
    print("=" * 60)
    
    # Check for API key
    if not os.getenv("OPENAI_API_KEY"):
        print("⚠️  Set OPENAI_API_KEY environment variable")
        return
    
    # Create knowledge graph
    vector_store, index = create_knowledge_graph()
    
    # Example queries
    queries = [
        "What papers built upon the Transformer architecture?",
        "How does GraphRAG improve upon traditional RAG?",
    ]
    
    for query in queries:
        print(f"\n{'=' * 60}")
        print(f"❓ Question: {query}")
        print("=" * 60)
        
        print("\n--- GraphRAG Mode (with citation graph) ---")
        try:
            answer = query_with_graph_expansion(index, vector_store, query)
            print(f"\n💡 Answer: {answer}")
        except Exception as e:
            print(f"⚠️  Graph expansion requires VelesDB server: {e}")
            print("   Falling back to vector-only mode...")
            answer = query_vector_only(index, query)
            print(f"\n💡 Answer: {answer}")
    
    print("\n" + "=" * 60)
    print("✅ Demo complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
