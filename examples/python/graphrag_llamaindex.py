#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LlamaIndex

Demonstrates Graph-enhanced RAG using LlamaIndex's query engine:
1. Build a knowledge graph from documents
2. Use GraphRetriever for context expansion
3. Generate answers with expanded context

Requirements:
    pip install llama-index-vector-stores-velesdb llama-index-llms-openai

Usage:
    export OPENAI_API_KEY=your-key
    python graphrag_llamaindex.py
"""

import os
from typing import List

# LlamaIndex imports
from llama_index.core import VectorStoreIndex, Settings
from llama_index.core.schema import TextNode, NodeRelationship, RelatedNodeInfo
from llama_index.llms.openai import OpenAI
from llama_index.embeddings.openai import OpenAIEmbedding

# VelesDB LlamaIndex integration
from llamaindex_velesdb import VelesDBVectorStore, GraphLoader, GraphRetriever


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
    
    # Load graph relationships
    loader = GraphLoader(vector_store)
    
    # Add edges based on citations
    loader.add_edge(id=1, source=2, target=1, label="CITES")  # BERT cites Transformer
    loader.add_edge(id=2, source=3, target=1, label="CITES")  # GPT-3 cites Transformer
    loader.add_edge(id=3, source=4, target=2, label="EXTENDS")  # RAG extends BERT ideas
    loader.add_edge(id=4, source=5, target=4, label="EXTENDS")  # GraphRAG extends RAG
    
    print("✅ Knowledge graph created: 5 papers, 4 citation relationships")
    return vector_store, index


def query_with_graph_expansion(
    index: VectorStoreIndex,
    vector_store: VelesDBVectorStore,
    query: str,
) -> str:
    """Query using GraphRetriever for context expansion."""
    
    # Create graph-enhanced retriever
    retriever = GraphRetriever(
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
