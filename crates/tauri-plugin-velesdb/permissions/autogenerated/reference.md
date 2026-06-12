## Default Permission

Default permissions for VelesDB plugin - allows all database operations

#### This default permission set includes the following:

- `allow-create-collection`
- `allow-create-metadata-collection`
- `allow-delete-collection`
- `allow-list-collections`
- `allow-get-collection`
- `allow-is-empty`
- `allow-flush`
- `allow-scroll-collection`
- `allow-upsert`
- `allow-upsert-metadata`
- `allow-get-points`
- `allow-delete-points`
- `allow-search`
- `allow-batch-search`
- `allow-text-search`
- `allow-hybrid-search`
- `allow-multi-query-search`
- `allow-query`
- `allow-sparse-search`
- `allow-hybrid-sparse-search`
- `allow-sparse-upsert`
- `allow-train-pq`
- `allow-stream-insert`
- `allow-semantic-store`
- `allow-semantic-store-with-ttl`
- `allow-semantic-query`
- `allow-semantic-delete`
- `allow-semantic-dimension`
- `allow-semantic-serialize`
- `allow-semantic-deserialize`
- `allow-episodic-record`
- `allow-episodic-recent`
- `allow-episodic-recall-similar`
- `allow-episodic-older-than`
- `allow-episodic-delete`
- `allow-episodic-serialize`
- `allow-episodic-deserialize`
- `allow-procedural-learn`
- `allow-procedural-recall`
- `allow-procedural-reinforce`
- `allow-procedural-list-all`
- `allow-procedural-delete`
- `allow-procedural-serialize`
- `allow-procedural-deserialize`
- `allow-memory-set-ttl`
- `allow-memory-auto-expire`
- `allow-memory-evict-low-confidence`
- `allow-memory-snapshot`
- `allow-memory-load-latest-snapshot`
- `allow-memory-load-snapshot-version`
- `allow-memory-list-snapshot-versions`
- `allow-memory-query-semantic`
- `allow-memory-query-episodic`
- `allow-memory-query-procedural`
- `allow-create-graph-collection`
- `allow-add-edge`
- `allow-get-edges`
- `allow-traverse-graph`
- `allow-get-node-degree`
- `allow-traverse-graph-parallel`
- `allow-create-index`
- `allow-drop-index`
- `allow-list-indexes`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`velesdb:allow-add-edge`

</td>
<td>

Enables the add_edge command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-add-edge`

</td>
<td>

Denies the add_edge command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-batch-search`

</td>
<td>

Enables the batch_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-batch-search`

</td>
<td>

Denies the batch_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-create-collection`

</td>
<td>

Enables the create_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-create-collection`

</td>
<td>

Denies the create_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-create-graph-collection`

</td>
<td>

Enables the create_graph_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-create-graph-collection`

</td>
<td>

Denies the create_graph_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-create-index`

</td>
<td>

Enables the create_index command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-create-index`

</td>
<td>

Denies the create_index command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-create-metadata-collection`

</td>
<td>

Enables the create_metadata_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-create-metadata-collection`

</td>
<td>

Denies the create_metadata_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-delete-collection`

</td>
<td>

Enables the delete_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-delete-collection`

</td>
<td>

Denies the delete_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-delete-points`

</td>
<td>

Enables the delete_points command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-delete-points`

</td>
<td>

Denies the delete_points command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-drop-index`

</td>
<td>

Enables the drop_index command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-drop-index`

</td>
<td>

Denies the drop_index command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-delete`

</td>
<td>

Enables the episodic_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-delete`

</td>
<td>

Denies the episodic_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-deserialize`

</td>
<td>

Enables the episodic_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-deserialize`

</td>
<td>

Denies the episodic_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-older-than`

</td>
<td>

Enables the episodic_older_than command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-older-than`

</td>
<td>

Denies the episodic_older_than command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-recall-similar`

</td>
<td>

Enables the episodic_recall_similar command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-recall-similar`

</td>
<td>

Denies the episodic_recall_similar command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-recent`

</td>
<td>

Enables the episodic_recent command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-recent`

</td>
<td>

Denies the episodic_recent command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-record`

</td>
<td>

Enables the episodic_record command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-record`

</td>
<td>

Denies the episodic_record command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-episodic-serialize`

</td>
<td>

Enables the episodic_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-episodic-serialize`

</td>
<td>

Denies the episodic_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-flush`

</td>
<td>

Enables the flush command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-flush`

</td>
<td>

Denies the flush command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-get-collection`

</td>
<td>

Enables the get_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-get-collection`

</td>
<td>

Denies the get_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-get-edges`

</td>
<td>

Enables the get_edges command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-get-edges`

</td>
<td>

Denies the get_edges command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-get-node-degree`

</td>
<td>

Enables the get_node_degree command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-get-node-degree`

</td>
<td>

Denies the get_node_degree command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-get-points`

</td>
<td>

Enables the get_points command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-get-points`

</td>
<td>

Denies the get_points command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-hybrid-search`

</td>
<td>

Enables the hybrid_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-hybrid-search`

</td>
<td>

Denies the hybrid_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-hybrid-sparse-search`

</td>
<td>

Enables the hybrid_sparse_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-hybrid-sparse-search`

</td>
<td>

Denies the hybrid_sparse_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-is-empty`

</td>
<td>

Enables the is_empty command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-is-empty`

</td>
<td>

Denies the is_empty command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-list-collections`

</td>
<td>

Enables the list_collections command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-list-collections`

</td>
<td>

Denies the list_collections command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-list-indexes`

</td>
<td>

Enables the list_indexes command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-list-indexes`

</td>
<td>

Denies the list_indexes command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-auto-expire`

</td>
<td>

Enables the memory_auto_expire command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-auto-expire`

</td>
<td>

Denies the memory_auto_expire command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-evict-low-confidence`

</td>
<td>

Enables the memory_evict_low_confidence command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-evict-low-confidence`

</td>
<td>

Denies the memory_evict_low_confidence command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-list-snapshot-versions`

</td>
<td>

Enables the memory_list_snapshot_versions command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-list-snapshot-versions`

</td>
<td>

Denies the memory_list_snapshot_versions command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-load-latest-snapshot`

</td>
<td>

Enables the memory_load_latest_snapshot command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-load-latest-snapshot`

</td>
<td>

Denies the memory_load_latest_snapshot command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-load-snapshot-version`

</td>
<td>

Enables the memory_load_snapshot_version command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-load-snapshot-version`

</td>
<td>

Denies the memory_load_snapshot_version command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-query-episodic`

</td>
<td>

Enables the memory_query_episodic command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-query-episodic`

</td>
<td>

Denies the memory_query_episodic command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-query-procedural`

</td>
<td>

Enables the memory_query_procedural command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-query-procedural`

</td>
<td>

Denies the memory_query_procedural command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-query-semantic`

</td>
<td>

Enables the memory_query_semantic command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-query-semantic`

</td>
<td>

Denies the memory_query_semantic command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-set-ttl`

</td>
<td>

Enables the memory_set_ttl command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-set-ttl`

</td>
<td>

Denies the memory_set_ttl command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-memory-snapshot`

</td>
<td>

Enables the memory_snapshot command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-memory-snapshot`

</td>
<td>

Denies the memory_snapshot command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-multi-query-search`

</td>
<td>

Enables the multi_query_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-multi-query-search`

</td>
<td>

Denies the multi_query_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-delete`

</td>
<td>

Enables the procedural_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-delete`

</td>
<td>

Denies the procedural_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-deserialize`

</td>
<td>

Enables the procedural_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-deserialize`

</td>
<td>

Denies the procedural_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-learn`

</td>
<td>

Enables the procedural_learn command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-learn`

</td>
<td>

Denies the procedural_learn command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-list-all`

</td>
<td>

Enables the procedural_list_all command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-list-all`

</td>
<td>

Denies the procedural_list_all command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-recall`

</td>
<td>

Enables the procedural_recall command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-recall`

</td>
<td>

Denies the procedural_recall command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-reinforce`

</td>
<td>

Enables the procedural_reinforce command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-reinforce`

</td>
<td>

Denies the procedural_reinforce command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-procedural-serialize`

</td>
<td>

Enables the procedural_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-procedural-serialize`

</td>
<td>

Denies the procedural_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-query`

</td>
<td>

Enables the query command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-query`

</td>
<td>

Denies the query command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-scroll-collection`

</td>
<td>

Enables the scroll_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-scroll-collection`

</td>
<td>

Denies the scroll_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-search`

</td>
<td>

Enables the search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-search`

</td>
<td>

Denies the search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-delete`

</td>
<td>

Enables the semantic_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-delete`

</td>
<td>

Denies the semantic_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-deserialize`

</td>
<td>

Enables the semantic_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-deserialize`

</td>
<td>

Denies the semantic_deserialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-dimension`

</td>
<td>

Enables the semantic_dimension command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-dimension`

</td>
<td>

Denies the semantic_dimension command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-query`

</td>
<td>

Enables the semantic_query command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-query`

</td>
<td>

Denies the semantic_query command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-serialize`

</td>
<td>

Enables the semantic_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-serialize`

</td>
<td>

Denies the semantic_serialize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-store`

</td>
<td>

Enables the semantic_store command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-store`

</td>
<td>

Denies the semantic_store command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-semantic-store-with-ttl`

</td>
<td>

Enables the semantic_store_with_ttl command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-semantic-store-with-ttl`

</td>
<td>

Denies the semantic_store_with_ttl command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-sparse-search`

</td>
<td>

Enables the sparse_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-sparse-search`

</td>
<td>

Denies the sparse_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-sparse-upsert`

</td>
<td>

Enables the sparse_upsert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-sparse-upsert`

</td>
<td>

Denies the sparse_upsert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-stream-insert`

</td>
<td>

Enables the stream_insert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-stream-insert`

</td>
<td>

Denies the stream_insert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-text-search`

</td>
<td>

Enables the text_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-text-search`

</td>
<td>

Denies the text_search command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-train-pq`

</td>
<td>

Enables the train_pq command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-train-pq`

</td>
<td>

Denies the train_pq command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-traverse-graph`

</td>
<td>

Enables the traverse_graph command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-traverse-graph`

</td>
<td>

Denies the traverse_graph command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-traverse-graph-parallel`

</td>
<td>

Enables the traverse_graph_parallel command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-traverse-graph-parallel`

</td>
<td>

Denies the traverse_graph_parallel command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-upsert`

</td>
<td>

Enables the upsert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-upsert`

</td>
<td>

Denies the upsert command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:allow-upsert-metadata`

</td>
<td>

Enables the upsert_metadata command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`velesdb:deny-upsert-metadata`

</td>
<td>

Denies the upsert_metadata command without any pre-configured scope.

</td>
</tr>
</table>
