import express, { type Request, type Response, type NextFunction } from "express";
import { z } from "zod";
import { embed } from "./embed.js";
import { initVelesDB, COLLECTION } from "./velesdb.js";

const PORT = Number(process.env.PORT ?? 3000);
const VELESDB_URL = process.env.VELESDB_URL ?? "http://localhost:8080";

const ingestSchema = z.object({
  id: z.string().min(1).max(256).optional(),
  text: z.string().min(1),
  metadata: z.record(z.unknown()).optional(),
});

const searchSchema = z.object({
  query: z.string().min(1),
  k: z.number().int().min(1).max(100).optional(),
});

async function main(): Promise<void> {
  const db = await initVelesDB(VELESDB_URL);
  const app = express();
  app.use(express.json({ limit: "1mb" }));

  app.get("/health", (_req, res) => {
    res.json({ status: "ok", collection: COLLECTION, velesdb: VELESDB_URL });
  });

  app.post("/ingest", async (req, res, next) => {
    try {
      const body = ingestSchema.parse(req.body);
      const id = body.id ?? crypto.randomUUID();
      await db.upsert(COLLECTION, {
        id,
        vector: embed(body.text),
        payload: { text: body.text, ...(body.metadata ?? {}) },
      });
      res.status(201).json({ id });
    } catch (err) {
      next(err);
    }
  });

  app.post("/search", async (req, res, next) => {
    try {
      const { query, k = 10 } = searchSchema.parse(req.body);
      const queryVec = embed(query);
      const t0 = performance.now();
      const results = await db.search(COLLECTION, queryVec, { k });
      const latencyMs = performance.now() - t0;
      res.json({ latencyMs, results });
    } catch (err) {
      next(err);
    }
  });

  app.use((err: unknown, _req: Request, res: Response, _next: NextFunction) => {
    if (err instanceof z.ZodError) {
      res.status(400).json({ error: "validation_failed", issues: err.issues });
      return;
    }
    console.error(err);
    const message = err instanceof Error ? err.message : String(err);
    res.status(500).json({ error: "internal_error", message });
  });

  const server = app.listen(PORT, () => {
    console.log(`[node-express-rag] listening on :${PORT} -> ${VELESDB_URL}`);
  });

  const shutdown = (signal: string) => {
    console.log(`[node-express-rag] ${signal} received, shutting down`);
    server.close(() => process.exit(0));
  };
  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));
}

main().catch((err) => {
  console.error("[node-express-rag] failed to start:", err);
  process.exit(1);
});
