#!/usr/bin/env python3
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
    print("  graph.add_edge(")
    print("      edge_id=1,")
    print("      source=100,      # Node: 'Python'")
    print("      target=200,      # Node: 'Programming Language'")
    print("      label='is_a',")
    print("      properties={'confidence': 0.99}")
    print("  )")
    print()
    print("  graph.add_edge(")
    print("      edge_id=2,")
    print("      source=100,      # Node: 'Python'")
    print("      target=300,      # Node: 'Data Science'")
    print("      label='used_for',")
    print("      properties={'popularity': 'high'}")
    print("  )")
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
    print("  results = graph.traverse_bfs(")
    print("      source=100,           # Start node: 'Python'")
    print("      max_depth=3,          # Max hops")
    print("      limit=50,             # Max results")
    print("      rel_types=['is_a', 'used_for']  # Filter by edge types")
    print("  )")
    print()
    print("  for node in results:")
    print("      print(f'Node {node.id} at depth {node.depth}')")
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
    print("  results = graph.traverse_dfs(")
    print("      source=100,           # Start node")
    print("      max_depth=5,          # Go deeper")
    print("      limit=100,")
    print("      rel_types=None        # All relationship types")
    print("  )")
    print()
    print("  for node in results:")
    print("      path = ' -> '.join(str(p) for p in node.path)")
    print("      print(f'Path: {path}')")
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
    print("  entities = collection.search(query_vec, top_k=5)")
    print()
    print("  # Step 2: Graph expansion (get related knowledge)")
    print("  context_nodes = []")
    print("  for entity in entities:")
    print("      neighbors = graph.traverse_bfs(")
    print("          source=entity.id,")
    print("          max_depth=2,")
    print("          limit=10")
    print("      )")
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
    print("  degree = graph.get_node_degree(node_id=100)")
    print("  print(f'Node 100 has {degree.in_degree} incoming, {degree.out_degree} outgoing')")
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
