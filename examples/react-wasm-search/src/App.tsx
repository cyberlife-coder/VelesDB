import { useEffect, useMemo, useState } from "react";
import init, { VectorStore } from "@wiscale/velesdb-wasm";
import { embed, EMBEDDING_DIM } from "./embed";
import { PRODUCTS, type Product } from "./data/products";

type SearchHit = { product: Product; score: number };

const TOP_K = 8;
const PRODUCTS_BY_ID = new Map(PRODUCTS.map((p) => [p.id, p]));

async function buildStore(): Promise<VectorStore> {
  await init();
  const store = new VectorStore(EMBEDDING_DIM, "cosine");
  for (const p of PRODUCTS) {
    store.insert(BigInt(p.id), embed(`${p.title} ${p.description}`));
  }
  return store;
}

export function App() {
  const [store, setStore] = useState<VectorStore | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    let created: VectorStore | null = null;
    buildStore()
      .then((s) => {
        created = s;
        if (alive) setStore(s);
        else s.free();
      })
      .catch((e) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
      created?.free();
    };
  }, []);

  useEffect(() => {
    if (!store || !query.trim()) {
      setHits([]);
      setLatencyMs(null);
      return;
    }
    const queryVec = embed(query);
    const start = performance.now();
    const raw = store.search(queryVec, TOP_K) as Array<[bigint, number]>;
    const elapsed = performance.now() - start;

    const next: SearchHit[] = [];
    for (const [id, score] of raw) {
      const product = PRODUCTS_BY_ID.get(Number(id));
      if (product) next.push({ product, score });
    }
    setHits(next);
    setLatencyMs(elapsed);
  }, [query, store]);

  const status = useMemo(() => {
    if (error) return `Error: ${error}`;
    if (!store) return "Loading WASM module…";
    return `Ready · ${PRODUCTS.length} items indexed (${EMBEDDING_DIM}-D cosine)`;
  }, [store, error]);

  return (
    <main className="page">
      <header>
        <h1>VelesDB · in-browser vector search</h1>
        <p className="status">{status}</p>
      </header>

      <input
        className="query"
        type="search"
        placeholder="Try: noise cancelling, brewing coffee, hiking gear…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        autoFocus
        disabled={!store}
      />

      {latencyMs !== null && (
        <p className="latency">
          {hits.length} results · {latencyMs.toFixed(2)} ms
        </p>
      )}

      <ol className="hits">
        {hits.map(({ product, score }) => (
          <li key={product.id}>
            <div className="hit-header">
              <span className="hit-title">{product.title}</span>
              <span className="hit-score">{score.toFixed(3)}</span>
            </div>
            <p className="hit-desc">{product.description}</p>
          </li>
        ))}
      </ol>

      {query.trim() && hits.length === 0 && store && (
        <p className="empty">No results for that query.</p>
      )}

      <footer>
        <a href="https://github.com/cyberlife-coder/VelesDB">VelesDB on GitHub</a>
        {" · "}
        <a href="https://www.npmjs.com/package/@wiscale/velesdb-wasm">@wiscale/velesdb-wasm</a>
      </footer>
    </main>
  );
}
