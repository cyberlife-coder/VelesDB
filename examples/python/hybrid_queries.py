#!/usr/bin/env python3
"""
Hybrid Query Examples for VelesDB Python SDK

Demonstrates vector similarity search combined with metadata filtering,
aggregations, and multi-model query patterns.

See docs/guides/USE_CASES.md for the 10 documented use cases.
"""

import numpy as np

# Note: VelesDB Python SDK uses PyO3 bindings
# Install: pip install velesdb-python (when published)
# For now, build from source: maturin develop

def generate_embedding(seed: int, dim: int = 128) -> list[float]:
    """Generate a deterministic mock embedding for demo purposes."""
    np.random.seed(seed)
    vec = np.random.randn(dim).astype(np.float32)
    vec = vec / np.linalg.norm(vec)  # Normalize
    return vec.tolist()


def example_1_contextual_rag():
    """
    Use Case 1: Contextual RAG
    
    Find documents similar to a query with metadata filtering.
    VelesQL: SELECT * FROM docs WHERE similarity(embedding, $q) > 0.75 LIMIT 20
    """
    print("\n=== Use Case 1: Contextual RAG ===")
    
    # Mock data - replace with real VelesDB calls
    documents = [
        {"id": 1, "title": "Quantum Computing Basics", "category": "physics"},
        {"id": 2, "title": "Machine Learning Guide", "category": "ai"},
        {"id": 3, "title": "Neural Networks Deep Dive", "category": "ai"},
    ]
    
    query_embedding = generate_embedding(42)
    
    # VelesQL query (when parser supports execution)
    velesql = """
        SELECT id, title, category 
        FROM documents 
        WHERE similarity(embedding, $query) > 0.75 
          AND category = 'ai'
        ORDER BY similarity(embedding, $query) DESC
        LIMIT 10
    """
    print(f"VelesQL: {velesql.strip()}")
    
    # Programmatic equivalent
    print("\nProgrammatic API:")
    print("  results = collection.search(query_embedding, top_k=10)")
    print("  filtered = [r for r in results if r.payload['category'] == 'ai']")


def example_2_semantic_search_with_filters():
    """
    Use Case 5: Semantic Search with Filters
    
    Combine vector NEAR with multiple metadata filters.
    VelesQL: SELECT * FROM articles WHERE vector NEAR $v AND category IN (...) LIMIT 20
    """
    print("\n=== Use Case 5: Semantic Search with Filters ===")
    
    velesql = """
        SELECT id, title, price 
        FROM articles 
        WHERE vector NEAR $query 
          AND category IN ('technology', 'science') 
          AND published_date >= '2024-01-01'
          AND access_level = 'public'
        LIMIT 20
        WITH (mode = 'balanced')
    """
    print(f"VelesQL: {velesql.strip()}")
    
    print("\nProgrammatic API:")
    print("  results = collection.search(query_vec, top_k=50)")
    print("  filtered = [r for r in results")
    print("              if r.payload['category'] in ['technology', 'science']")
    print("              and r.payload['access_level'] == 'public'][:20]")


def example_3_aggregations():
    """
    Use Case 4: Document Clustering with Aggregations
    
    Group similar documents by category.
    VelesQL: SELECT category, COUNT(*) FROM docs GROUP BY category
    """
    print("\n=== Use Case 4: Document Clustering ===")
    
    velesql = """
        SELECT category, COUNT(*) 
        FROM documents 
        WHERE similarity(embedding, $query) > 0.6 
        GROUP BY category 
        ORDER BY COUNT(*) DESC 
        LIMIT 10
    """
    print(f"VelesQL: {velesql.strip()}")
    
    print("\nProgrammatic API:")
    print("  from collections import Counter")
    print("  results = collection.search(query_vec, top_k=100)")
    print("  filtered = [r for r in results if r.score > 0.6]")
    print("  counts = Counter(r.payload['category'] for r in filtered)")


def example_4_recommendation_engine():
    """
    Use Case 6: Recommendation Engine
    
    Find similar items based on user preferences.
    """
    print("\n=== Use Case 6: Recommendation Engine ===")
    
    velesql = """
        SELECT id, name, category, price 
        FROM items 
        WHERE similarity(embedding, $user_preference) > 0.7 
          AND category = 'electronics' 
          AND price < 100
        ORDER BY similarity(embedding, $user_preference) DESC
        LIMIT 10
    """
    print(f"VelesQL: {velesql.strip()}")
    
    print("\nProgrammatic API:")
    print("  user_pref = get_user_preference_embedding(user_id)")
    print("  results = collection.search(user_pref, top_k=20)")
    print("  recommendations = [r for r in results")
    print("                     if r.payload['price'] < 100][:10]")


def example_5_entity_resolution():
    """
    Use Case 7: Entity Resolution (Deduplication)
    
    Find near-duplicates using high similarity threshold.
    """
    print("\n=== Use Case 7: Entity Resolution ===")
    
    velesql = """
        SELECT id, name 
        FROM companies 
        WHERE similarity(embedding, $new_entity) > 0.95 
        LIMIT 5
    """
    print(f"VelesQL: {velesql.strip()}")
    
    print("\nProgrammatic API:")
    print("  new_entity_emb = embed(new_company_name)")
    print("  matches = collection.search(new_entity_emb, top_k=5)")
    print("  duplicates = [m for m in matches if m.score > 0.95]")
    print("  if duplicates:")
    print("      print(f'Possible duplicate: {duplicates[0].payload[\"name\"]}')")


def example_6_conversational_memory():
    """
    Use Case 10: Conversational Memory for AI Agents
    
    Retrieve relevant context from conversation history.
    """
    print("\n=== Use Case 10: Conversational Memory ===")
    
    velesql = """
        SELECT content, role, timestamp 
        FROM messages 
        WHERE conversation_id = $conv_id 
          AND similarity(embedding, $current_query) > 0.6 
        ORDER BY timestamp DESC 
        LIMIT 10
    """
    print(f"VelesQL: {velesql.strip()}")
    
    print("\nProgrammatic API:")
    print("  current_query_emb = embed(user_message)")
    print("  context = collection.search(current_query_emb, top_k=20)")
    print("  relevant = [m for m in context")
    print("              if m.payload['conversation_id'] == conv_id][:10]")
    print("  # Pass to LLM as context")


def main():
    """Run all hybrid query examples."""
    print("=" * 60)
    print("VelesDB Hybrid Query Examples - Python SDK")
    print("=" * 60)
    
    example_1_contextual_rag()
    example_2_semantic_search_with_filters()
    example_3_aggregations()
    example_4_recommendation_engine()
    example_5_entity_resolution()
    example_6_conversational_memory()
    
    print("\n" + "=" * 60)
    print("See docs/guides/USE_CASES.md for all 10 use cases")
    print("See docs/VELESQL_SPEC.md for complete VelesQL reference")
    print("=" * 60)


if __name__ == "__main__":
    main()
