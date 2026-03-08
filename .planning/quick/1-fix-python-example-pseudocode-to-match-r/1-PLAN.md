---
phase: quick-1
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - examples/python/graph_traversal.py
  - examples/python/hybrid_queries.py
  - examples/python/multimodel_notebook.py
autonomous: true
requirements: []

must_haves:
  truths:
    - "All graph API calls in examples match real PyO3 SDK signatures"
    - "All collection.search calls use keyword vector= argument"
    - "Traversal examples use StreamingConfig, not positional args"
    - "TraversalResult field access matches real .source, .target, .label, .edge_id, .depth"
    - "Node degree uses separate in_degree/out_degree methods"
  artifacts:
    - path: "examples/python/graph_traversal.py"
      provides: "Corrected graph traversal pseudocode"
    - path: "examples/python/hybrid_queries.py"
      provides: "Corrected search call signatures"
    - path: "examples/python/multimodel_notebook.py"
      provides: "Corrected search call signature"
  key_links: []
---

<objective>
Fix Python example pseudocode to match real PyO3 SDK API signatures.

Purpose: Examples currently show incorrect API calls (wrong arg styles for add_edge, wrong traversal function signatures, nonexistent get_node_degree method, positional instead of keyword args for search). Users copying these examples would get runtime errors.

Output: Three corrected example files with accurate API signatures.
</objective>

<execution_context>
@C:/Users/Nicolas/.claude/get-shit-done/workflows/execute-plan.md
@C:/Users/Nicolas/.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@examples/python/graph_traversal.py
@examples/python/hybrid_queries.py
@examples/python/multimodel_notebook.py

Files NOT needing changes (verified correct):
- examples/python/fusion_strategies.py — FusionStrategy calls all match real API
- examples/python/graphrag_langchain.py — Uses HTTP REST API, not Python SDK directly
- examples/python/graphrag_llamaindex.py — Uses HTTP REST API, not Python SDK directly
- crates/velesdb-python/README.md — Already uses keyword `vector=` form (line 105)
</context>

<tasks>

<task type="auto">
  <name>Task 1: Fix graph_traversal.py — major API mismatches</name>
  <files>examples/python/graph_traversal.py</files>
  <action>
Fix ALL five API mismatches in this pseudocode file. The file uses print() statements to show pseudocode, so edit the print string contents:

1. **add_edge takes a dict, not named args** (lines 35-49):
   Change from:
   ```
   graph.add_edge(
       edge_id=1,
       source=100,
       target=200,
       label='is_a',
       properties={'confidence': 0.99}
   )
   ```
   To:
   ```
   graph.add_edge({
       'edge_id': 1,
       'source': 100,
       'target': 200,
       'label': 'is_a',
       'properties': {'confidence': 0.99}
   })
   ```
   Apply same fix to second add_edge call (edge_id=2).

2. **traverse_bfs uses StreamingConfig** (lines 67-72):
   Change from:
   ```
   results = graph.traverse_bfs(
       source=100,
       max_depth=3,
       limit=50,
       rel_types=['is_a', 'used_for']
   )
   ```
   To:
   ```
   from velesdb import StreamingConfig
   config = StreamingConfig(max_depth=3, max_visited=50, relationship_types=['is_a', 'used_for'])
   results = graph.traverse_bfs_streaming(100, config)
   ```
   Note: method name is `traverse_bfs_streaming`, not `traverse_bfs`.

3. **traverse_dfs uses StreamingConfig** (lines 93-98):
   Change from positional args to:
   ```
   config = StreamingConfig(max_depth=5, max_visited=100)
   results = graph.traverse_dfs(100, config)
   ```

4. **TraversalResult fields** (lines 74-76 and 100-102):
   - Change `node.id` and `node.depth` to show real fields: `result.source`, `result.target`, `result.label`, `result.edge_id`, `result.depth`
   - Change `node.path` to show real field access pattern (no .path exists)
   - For BFS example: `print(f'Edge {result.edge_id}: {result.source} -[{result.label}]-> {result.target} (depth {result.depth})')`
   - For DFS example: `print(f'Edge {result.edge_id}: {result.source} -[{result.label}]-> {result.target} (depth {result.depth})')`

5. **get_node_degree does not exist** (lines 155-156):
   Change from:
   ```
   degree = graph.get_node_degree(node_id=100)
   print(f'Node 100 has {degree.in_degree} incoming, {degree.out_degree} outgoing')
   ```
   To:
   ```
   in_deg = graph.in_degree(100)
   out_deg = graph.out_degree(100)
   print(f'Node 100 has {in_deg} incoming, {out_deg} outgoing')
   ```

6. **Graph RAG section** (lines 122, 127-131):
   - Fix `collection.search(query_vec, top_k=5)` to `collection.search(vector=query_vec, top_k=5)`
   - Fix the traverse_bfs call in the loop to use StreamingConfig pattern

Keep the PSEUDOCODE header comment at top. Keep CLI examples unchanged (those are separate from Python SDK).
  </action>
  <verify>python -c "exec(open('examples/python/graph_traversal.py').read())" runs without error (file only contains print statements)</verify>
  <done>All graph API pseudocode matches real PyO3 signatures: add_edge takes dict, traverse_bfs_streaming/traverse_dfs use StreamingConfig, TraversalResult uses .source/.target/.label/.edge_id/.depth, degree uses separate in_degree/out_degree methods</done>
</task>

<task type="auto">
  <name>Task 2: Fix search keyword args in hybrid_queries.py and multimodel_notebook.py</name>
  <files>examples/python/hybrid_queries.py, examples/python/multimodel_notebook.py</files>
  <action>
Fix positional-to-keyword arg for collection.search calls.

**hybrid_queries.py** — 6 occurrences in print() strings:
- Line 60: `collection.search(query_embedding, top_k=10)` -> `collection.search(vector=query_embedding, top_k=10)`
- Line 86: `collection.search(query_vec, top_k=50)` -> `collection.search(vector=query_vec, top_k=50)`
- Line 113: `collection.search(query_vec, top_k=100)` -> `collection.search(vector=query_vec, top_k=100)`
- Line 139: `collection.search(user_pref, top_k=20)` -> `collection.search(vector=user_pref, top_k=20)`
- Line 162: `collection.search(new_entity_emb, top_k=5)` -> `collection.search(vector=new_entity_emb, top_k=5)`
- Line 188: `collection.search(current_query_emb, top_k=20)` -> `collection.search(vector=current_query_emb, top_k=20)`

**multimodel_notebook.py** — 1 occurrence of actual code (not print):
- Line 87: `results = collection.search(query_vector, top_k=3)` -> `results = collection.search(vector=query_vector, top_k=3)`

Do NOT change fusion_strategies.py (already correct).
  </action>
  <verify>python -c "exec(open('examples/python/hybrid_queries.py').read())" runs without error AND python -c "import ast; ast.parse(open('examples/python/multimodel_notebook.py').read())" parses without error</verify>
  <done>All collection.search calls use keyword vector= argument matching real SDK signature: search(vector=None, *, sparse_vector=None, top_k=10)</done>
</task>

</tasks>

<verification>
- grep -n "collection.search(" examples/python/graph_traversal.py examples/python/hybrid_queries.py examples/python/multimodel_notebook.py shows all calls use vector= keyword
- grep -n "traverse_bfs\b" examples/python/graph_traversal.py returns zero matches (should be traverse_bfs_streaming)
- grep -n "get_node_degree" examples/python/graph_traversal.py returns zero matches
- grep -n "StreamingConfig" examples/python/graph_traversal.py returns matches
- grep -n "add_edge({" examples/python/graph_traversal.py shows dict-style calls
</verification>

<success_criteria>
- Zero instances of old positional search(vec, top_k=N) pattern in modified files
- Zero instances of traverse_bfs (must be traverse_bfs_streaming) in graph_traversal.py
- Zero instances of get_node_degree in graph_traversal.py
- All add_edge calls pass a dict argument
- All TraversalResult access uses .source/.target/.label/.edge_id/.depth (not .id/.path)
- All three files still execute/parse without syntax errors
</success_criteria>

<output>
After completion, create `.planning/quick/1-fix-python-example-pseudocode-to-match-r/1-SUMMARY.md`
</output>
