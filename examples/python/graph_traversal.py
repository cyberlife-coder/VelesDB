#!/usr/bin/env python3
# PSEUDOCODE: This file is not directly runnable.
# It requires the VelesDB Python SDK compiled via PyO3:
#   cd crates/velesdb-python && maturin develop
# For a runnable example using the real SDK, see examples/python/multimodel_notebook.py
"""
Graph Traversal Examples for VelesDB Python SDK

Demonstrates Knowledge Graph operations:
- BFS (Breadth-First Search) traversal
- DFS (Depth-First Search) traversal
- Edge operations
- Graph RAG patterns

EPIC-059 US-005: VelesQL & Graph examples
"""

# Note: VelesDB Python SDK uses PyO3 bindings
# Install: pip install velesdb-python (when published)
# For now, build from source: maturin develop


def example_add_edges():
    """
    Adding edges to build a Knowledge Graph.
    """
    print("\n=== Building a Knowledge Graph ===")
    print()
    
    print("Python SDK:")
    print("  # Create graph store")
    print("  graph = db.get_graph_store('knowledge')")
    print()
    print("  # Add edges (relationships)")
    print("  graph.add_edge({")
    print("      'edge_id': 1,")
    print("      'source': 100,      # Node: 'Python'")
    print("      'target': 200,      # Node: 'Programming Language'")
    print("      'label': 'is_a',")
    print("      'properties': {'confidence': 0.99}")
    print("  })")
    print()
    print("  graph.add_edge({")
    print("      'edge_id': 2,")
    print("      'source': 100,      # Node: 'Python'")
    print("      'target': 300,      # Node: 'Data Science'")
    print("      'label': 'used_for',")
    print("      'properties': {'popularity': 'high'}")
    print("  })")
    print()
    print("CLI:")
    print("  velesdb graph add-edge ./data knowledge 1 100 200 is_a")
    print("  velesdb graph add-edge ./data knowledge 2 100 300 used_for")


def example_bfs_traversal():
    """
    BFS (Breadth-First Search) traversal.
    
    Best for: Finding shortest paths, level-by-level exploration
    """
    print("\n=== BFS Traversal (Breadth-First Search) ===")
    print("Best for: Shortest paths, level-by-level exploration")
    print()
    
    print("Python SDK:")
    print("  from velesdb import StreamingConfig")
    print("  config = StreamingConfig(max_depth=3, max_visited=50, relationship_types=['is_a', 'used_for'])")
    print("  results = graph.traverse_bfs_streaming(100, config)  # Start node: 'Python'")
    print()
    print("  for result in results:")
    print("      print(f'Edge {result.edge_id}: {result.source} -[{result.label}]-> {result.target} (depth {result.depth})')")
    print()
    print("CLI:")
    print("  velesdb graph traverse ./data knowledge 100 --strategy bfs \\")
    print("    --max-depth 3 --limit 50 --rel-types is_a,used_for")


def example_dfs_traversal():
    """
    DFS (Depth-First Search) traversal.
    
    Best for: Deep exploration, finding all paths
    """
    print("\n=== DFS Traversal (Depth-First Search) ===")
    print("Best for: Deep exploration, complete path enumeration")
    print()
    
    print("Python SDK:")
    print("  config = StreamingConfig(max_depth=5, max_visited=100)")
    print("  results = graph.traverse_dfs(100, config)  # Start node")
    print()
    print("  for result in results:")
    print("      print(f'Edge {result.edge_id}: {result.source} -[{result.label}]-> {result.target} (depth {result.depth})')")
    print()
    print("CLI:")
    print("  velesdb graph traverse ./data knowledge 100 --strategy dfs \\")
    print("    --max-depth 5 --limit 100")


def example_graph_rag():
    """
    Graph RAG: Combining vector search with graph traversal.
    
    Pattern: Vector search → Graph expansion → LLM context
    """
    print("\n=== Graph RAG Pattern ===")
    print("Combines semantic search with knowledge graph context")
    print()
    
    print("Python SDK:")
    print("  # Step 1: Vector search for relevant entities")
    print("  query_vec = embed('What are Python libraries for ML?')")
    print("  entities = collection.search(vector=query_vec, top_k=5)")
    print()
    print("  # Step 2: Graph expansion (get related knowledge)")
    print("  context_nodes = []")
    print("  for entity in entities:")
    print("      config = StreamingConfig(max_depth=2, max_visited=10)")
    print("      neighbors = graph.traverse_bfs_streaming(entity.id, config)")
    print("      context_nodes.extend(neighbors)")
    print()
    print("  # Step 3: Build LLM context")
    print("  context = build_context(entities, context_nodes)")
    print("  response = llm.generate(query, context)")
    print()
    print("VelesQL (future):")
    print("  SELECT e.*, n.* FROM entities e")
    print("  JOIN GRAPH knowledge ON e.id = n.source")
    print("  WHERE vector NEAR $query")
    print("  TRAVERSE BFS(max_depth=2)")
    print("  LIMIT 20")


def example_node_degree():
    """
    Get node degree (number of connections).
    """
    print("\n=== Node Degree Analysis ===")
    print("Find most connected nodes (hubs)")
    print()
    
    print("Python SDK:")
    print("  in_deg = graph.in_degree(100)")
    print("  out_deg = graph.out_degree(100)")
    print("  print(f'Node 100 has {in_deg} incoming, {out_deg} outgoing')")
    print()
    print("CLI:")
    print("  velesdb graph degree ./data knowledge 100")


def main():
    """Run all graph traversal examples."""
    print("=" * 60)
    print("VelesDB Graph Traversal Examples - Python SDK")
    print("=" * 60)
    
    example_add_edges()
    example_bfs_traversal()
    example_dfs_traversal()
    example_graph_rag()
    example_node_degree()
    
    print("\n" + "=" * 60)
    print("Graph features enable Knowledge Graph + Vector RAG patterns")
    print("See: docs/guides/GRAPH_RAG.md")
    print("=" * 60)


if __name__ == "__main__":
    main()
