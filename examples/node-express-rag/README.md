# Express.js — RAG Backend with VelesDB

A minimal Express + TypeScript backend that talks to a [VelesDB](https://github.com/cyberlife-coder/VelesDB)
server through the [`@wiscale/velesdb-sdk`](https://www.npmjs.com/package/@wiscale/velesdb-sdk) Node.js client.

Three endpoints:

| Method | Path | Body |
|---|---|---|
| `POST` | `/ingest` | `{ "id"?, "text", "metadata"? }` — embeds `text` and upserts into the collection |
| `POST` | `/search` | `{ "query", "k"? }` — embeds `query` and returns the top-K hits |
| `GET`  | `/health` | — — returns `{ status, collection, velesdb }` |

## Run with Docker (recommended)

```bash
cd examples/node-express-rag
docker compose up --build
```

This starts:

- `velesdb-server` (built from the repo root `Dockerfile`) on `:8080` with persistent volume `velesdb-data`
- `node-express-rag` (this app) on `:3000`, configured via `VELESDB_URL=http://velesdb:8080`

The app waits for the VelesDB healthcheck to go green before accepting traffic.

> **First build is slow**: `docker compose up --build` compiles the full
> Rust workspace (~5–15 minutes cold). Subsequent runs use the cached
> image. If you already built `velesdb/velesdb:latest` from the repo
> root, change the `velesdb` service in `docker-compose.yml` to
> `image: velesdb/velesdb:latest` (drop the `build:` block) to skip
> the rebuild. The named volume `velesdb-data` survives `docker compose
> down` but is wiped by `docker compose down -v`.

### Try it

```bash
# Ingest a few documents
curl -X POST http://localhost:3000/ingest \
  -H "content-type: application/json" \
  -d '{"id":"1","text":"VelesDB is a single-binary embedded vector + graph database in Rust."}'

curl -X POST http://localhost:3000/ingest \
  -H "content-type: application/json" \
  -d '{"id":"2","text":"Express is a minimal Node.js web framework for building HTTP APIs."}'

curl -X POST http://localhost:3000/ingest \
  -H "content-type: application/json" \
  -d '{"id":"3","text":"Cosine similarity ranks documents by the angle between their embedding vectors."}'

# Search
curl -X POST http://localhost:3000/search \
  -H "content-type: application/json" \
  -d '{"query":"how does similarity search work","k":3}'

# Health
curl http://localhost:3000/health
```

## Run locally without Docker

If you have a `velesdb-server` process running on the host (`cargo run --bin velesdb-server`):

```bash
cd examples/node-express-rag
cp .env.example .env       # adjust PORT / VELESDB_URL if needed
npm install
npm run dev                # tsx watch mode
```

## Embedding pipeline

`src/embed.ts` ships a deterministic 384-D **hashing-trick** projection (the
same trick used by `examples/react-wasm-search/`). It is intentionally a
placeholder so the example is self-contained — no model download, no
external API key — but it only captures lexical overlap, not semantics.

Production-ready alternatives:

- In-process: [`@xenova/transformers`](https://www.npmjs.com/package/@xenova/transformers) (ONNX runtime)
- Remote: any embeddings API that returns a fixed-size vector

Swap the `embed()` function and bump `EMBEDDING_DIM` to your model's output size; the rest of the pipeline is unchanged.

## File map

| Path | Purpose |
|---|---|
| `src/server.ts` | Express app — routes, validation (zod), error handler. |
| `src/velesdb.ts` | SDK init wrapper — opens the REST connection, creates the `documents` collection if missing. |
| `src/embed.ts` | Hashing-trick embedder used by both `/ingest` and `/search`. |
| `Dockerfile` | Multi-stage Node 20 alpine build for the app. |
| `docker-compose.yml` | Runs `velesdb-server` + this app together. |
