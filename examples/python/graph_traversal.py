#!/usr/bin/env python3
"""
Graph Traversal Examples for VelesDB Python SDK

Demonstrates persistent GraphCollection operations:
- Creating a graph collection with node embeddings
- Adding edges and node payloads
- BFS (Breadth-First Search) traversal
- DFS (Depth-First Search) traversal
- Node degree analysis
- Embedding-based node search

Requirements:
    pip install velesdb numpy
    # Or build from source: cd crates/velesdb-python && maturin develop

EPIC-059 US-005: VelesQL & Graph examples
"""

import shutil
import tempfile

import numpy as np

import velesdb


def generate_embedding(seed: int, dim: int = 128) -> list[float]:
    """Generate a deterministic normalized embedding for demo purposes."""
    rng = np.random.RandomState(seed)
    vec = rng.randn(dim).astype(np.float32)
    vec = vec / np.linalg.norm(vec)
    return vec.tolist()


def build_knowledge_graph(db: velesdb.Database, dim: int = 128):
    """
    Build a small knowledge graph about programming languages.

    Graph structure:
        Python(100) --[IS_A]--> ProgrammingLanguage(200)
        Python(100) --[USED_FOR]--> DataScience(300)
        Python(100) --[USED_FOR]--> WebDev(400)
        Rust(101) --[IS_A]--> ProgrammingLanguage(200)
        Rust(101) --[USED_FOR]--> SystemsProgramming(301)
        DataScience(300) --[REQUIRES]--> Statistics(500)
    """
    print("\n=== Building Knowledge Graph ===")

    graph = db.create_graph_collection("knowledge", dimension=dim, metric="cosine")

    # Store node payloads with embeddings
    nodes = [
        (100, "Python", {"type": "language", "year": 1991}),
        (101, "Rust", {"type": "language", "year": 2010}),
        (200, "Programming Language", {"type": "category"}),
        (300, "Data Science", {"type": "field"}),
        (301, "Systems Programming", {"type": "field"}),
        (400, "Web Development", {"type": "field"}),
        (500, "Statistics", {"type": "field"}),
    ]

    for node_id, name, props in nodes:
        payload = {"name": name, **props}
        graph.store_node_payload(node_id, payload)

    # Add edges (relationships)
    edges = [
        (1, 100, 200, "IS_A"),
        (2, 100, 300, "USED_FOR"),
        (3, 100, 400, "USED_FOR"),
        (4, 101, 200, "IS_A"),
        (5, 101, 301, "USED_FOR"),
        (6, 300, 500, "REQUIRES"),
    ]

    for edge_id, source, target, label in edges:
        graph.add_edge({
            "id": edge_id,
            "source": source,
            "target": target,
            "label": label,
        })

    print(f"Created graph with {len(nodes)} nodes and {graph.edge_count()} edges")
    return graph


def example_get_edges(graph):
    """Retrieve edges, optionally filtered by label."""
    print("\n=== Edge Retrieval ===")

    all_edges = graph.get_edges()
    print(f"Total edges: {len(all_edges)}")

    used_for_edges = graph.get_edges(label="USED_FOR")
    print(f"USED_FOR edges: {len(used_for_edges)}")
    for e in used_for_edges:
        print(f"  Edge {e['id']}: {e['source']} --[{e['label']}]--> {e['target']}")


def example_outgoing_incoming(graph):
    """Get outgoing and incoming edges for a node."""
    print("\n=== Outgoing / Incoming Edges ===")

    # Outgoing from Python (node 100)
    outgoing = graph.get_outgoing(100)
    print(f"Outgoing edges from Python (100): {len(outgoing)}")
    for e in outgoing:
        print(f"  {e['source']} --[{e['label']}]--> {e['target']}")

    # Incoming to ProgrammingLanguage (node 200)
    incoming = graph.get_incoming(200)
    print(f"Incoming edges to ProgrammingLanguage (200): {len(incoming)}")
    for e in incoming:
        print(f"  {e['source']} --[{e['label']}]--> {e['target']}")


def example_bfs_traversal(graph):
    """
    BFS (Breadth-First Search) traversal.

    Best for: Finding shortest paths, level-by-level exploration.
    """
    print("\n=== BFS Traversal from Python (node 100) ===")

    results = graph.traverse_bfs(
        source_id=100,
        max_depth=3,
        limit=20,
    )

    print(f"Reachable nodes: {len(results)}")
    for r in results:
        print(f"  Target: {r['target_id']}, Depth: {r['depth']}, "
              f"Path: {r['path']}")


def example_dfs_traversal(graph):
    """
    DFS (Depth-First Search) traversal.

    Best for: Deep exploration, finding all paths.
    """
    print("\n=== DFS Traversal from Python (node 100) ===")

    results = graph.traverse_dfs(
        source_id=100,
        max_depth=5,
        limit=20,
    )

    print(f"Reachable nodes: {len(results)}")
    for r in results:
        print(f"  Target: {r['target_id']}, Depth: {r['depth']}, "
              f"Path: {r['path']}")


def example_filtered_traversal(graph):
    """Traverse following only specific relationship types."""
    print("\n=== Filtered Traversal (USED_FOR only) ===")

    results = graph.traverse_bfs(
        source_id=100,
        max_depth=3,
        limit=20,
        rel_types=["USED_FOR"],
    )

    print(f"Nodes reachable via USED_FOR from Python: {len(results)}")
    for r in results:
        print(f"  Target: {r['target_id']}, Depth: {r['depth']}")


def example_node_degree(graph):
    """Analyze node connectivity (in-degree and out-degree)."""
    print("\n=== Node Degree Analysis ===")

    node_ids = [100, 101, 200, 300]
    for nid in node_ids:
        in_deg, out_deg = graph.node_degree(nid)
        payload = graph.get_node_payload(nid)
        name = payload.get("name", "Unknown") if payload else "Unknown"
        print(f"  Node {nid} ({name}): in={in_deg}, out={out_deg}")


def example_node_payload(graph):
    """Read and write node payloads."""
    print("\n=== Node Payload Operations ===")

    # Read existing payload
    payload = graph.get_node_payload(100)
    print(f"Python payload: {payload}")

    # List all node IDs with payloads
    all_ids = graph.all_node_ids()
    print(f"All nodes with payloads: {sorted(all_ids)}")


def main():
    """Run all graph traversal examples."""
    print("=" * 60)
    print("VelesDB Graph Traversal Examples")
    print("=" * 60)

    tmp_dir = tempfile.mkdtemp(prefix="velesdb_graph_")
    try:
        db = velesdb.Database(tmp_dir)
        graph = build_knowledge_graph(db)

        example_get_edges(graph)
        example_outgoing_incoming(graph)
        example_bfs_traversal(graph)
        example_dfs_traversal(graph)
        example_filtered_traversal(graph)
        example_node_degree(graph)
        example_node_payload(graph)

        # Flush to disk
        graph.flush()

        print("\n" + "=" * 60)
        print("All graph traversal examples completed successfully.")
        print("=" * 60)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
