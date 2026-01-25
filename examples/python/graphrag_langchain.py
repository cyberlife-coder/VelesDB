#!/usr/bin/env python3
"""
VelesDB GraphRAG Example with LangChain

Demonstrates the "Seed + Expand" pattern for Graph-enhanced RAG:
1. Vector search to find initial seed documents
2. Graph traversal to expand context to related documents
3. Combined context for LLM generation

Requirements:
    pip install langchain-velesdb langchain-openai

Usage:
    export OPENAI_API_KEY=your-key
    python graphrag_langchain.py
"""

import os
from typing import List

# LangChain imports
from langchain_core.documents import Document
from langchain_core.prompts import ChatPromptTemplate
from langchain_openai import ChatOpenAI, OpenAIEmbeddings

# VelesDB LangChain integration
from langchain_velesdb import VelesDBVectorStore, GraphRetriever


def create_sample_knowledge_base() -> VelesDBVectorStore:
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
        path="./graphrag_demo",
        collection_name="knowledge_base",
    )
    
    # Add graph edges (concept relationships)
    collection = vectorstore._get_collection()
    
    # ML → DL (ML includes DL)
    collection.add_edge(id=1, source=1, target=2, label="INCLUDES")
    # ML → NLP (ML includes NLP)
    collection.add_edge(id=2, source=1, target=3, label="INCLUDES")
    # NLP → Transformers (NLP uses Transformers)
    collection.add_edge(id=3, source=3, target=4, label="USES")
    # Transformers → GPT (Transformers basis for GPT)
    collection.add_edge(id=4, source=4, target=5, label="BASIS_FOR")
    # Transformers → BERT (Transformers basis for BERT)
    collection.add_edge(id=5, source=4, target=6, label="BASIS_FOR")
    # VelesDB → VectorDB (VelesDB is a VectorDB)
    collection.add_edge(id=6, source=8, target=7, label="IS_A")
    
    print("✅ Knowledge base created with 8 documents and 6 relationships")
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
            vectorstore=vectorstore,
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
    
    print(f"\n📚 Retrieved {len(docs)} documents using {mode}:")
    for i, doc in enumerate(docs):
        depth = doc.metadata.get("graph_depth", 0)
        marker = "🎯" if depth == 0 else "🔗"
        print(f"  {marker} [{i+1}] {doc.page_content[:60]}...")
    
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
        print("⚠️  Set OPENAI_API_KEY environment variable")
        print("   For demo without LLM, the retrieval still works.")
        return
    
    # Create knowledge base
    vectorstore = create_sample_knowledge_base()
    
    # Query examples
    queries = [
        "What is the relationship between transformers and GPT?",
        "How does VelesDB differ from other vector databases?",
    ]
    
    for query in queries:
        print(f"\n{'=' * 60}")
        print(f"❓ Question: {query}")
        print("=" * 60)
        
        # Compare GraphRAG vs Standard RAG
        print("\n--- GraphRAG Mode ---")
        answer_graph = graphrag_query(vectorstore, query, use_graph=True)
        print(f"\n💡 Answer: {answer_graph}")
        
        print("\n--- Standard RAG Mode ---")
        answer_vector = graphrag_query(vectorstore, query, use_graph=False)
        print(f"\n💡 Answer: {answer_vector}")
    
    print("\n" + "=" * 60)
    print("✅ Demo complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
