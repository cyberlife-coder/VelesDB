# Workload Profile Catalog for `velesdb-core`

This catalog standardizes benchmark suites by workload family so performance claims are reproducible.

## Profiles

| Profile | Primary path | Dataset | KPI focus | Command |
|---|---|---|---|---|
| RAG Retrieval | vector + filter + payload | 10k/100k docs | p50/p95/p99, Recall@10 | `cargo bench -p velesdb-core --bench hnsw_benchmark` |
| Recommendations | vector + graph traversal | 5k products + graph edges | blended latency, throughput | `cargo test -p velesdb-core --test integration_scenarios` |
| Graph-heavy MATCH | MATCH + WHERE filters | synthetic relationship graph | planner determinism, latency | `cargo test -p velesdb-core velesql_planner_golden` |
| Durability stress | flush/restart/recovery | persistence fixtures | recovery success rate/time | `cargo test -p velesdb-core crash_recovery_corruption` |

## Reporting convention

- Publish p50/p95/p99 for each profile.
- Publish hardware + rustc + feature flags.
- Publish Recall@10 where ANN is involved.
- Keep profile outputs as release artifacts.
