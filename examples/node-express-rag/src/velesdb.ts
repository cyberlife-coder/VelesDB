import { VelesDB } from "@wiscale/velesdb-sdk";
import { EMBEDDING_DIM } from "./embed.js";

export const COLLECTION = "documents";

export async function initVelesDB(url: string): Promise<VelesDB> {
  const db = new VelesDB({ backend: "rest", url });
  await db.init();

  const existing = await db.getCollection(COLLECTION);
  if (!existing) {
    await db.createCollection(COLLECTION, {
      dimension: EMBEDDING_DIM,
      metric: "cosine",
    });
  }
  return db;
}
