# AGENTS.md — velesdb-migrate

Bulk-loads vectors from Pinecone, Qdrant, Milvus and other sources into a local VelesDB database. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies inside this crate.

- **Source adapters are read-only against the source and idempotent against the target:** a re-run of the same import must not duplicate points (upsert semantics, never blind insert).
- **Batch sizing is a measured constant, not a guess:** batch sizes live in one place per source and are registered in `docs/reference/promise-contract.json` if quoted in docs.
- **Credentials never hit argv or logs:** source API keys come from env/files, and errors are redacted before printing.
- **Tests:** `cargo test -p velesdb-migrate -- --test-threads=1` (CI leg `rest`). Network access in tests is forbidden — mock at the source-client boundary.
- Docs: crate [README.md](README.md); examples in `examples/`.
