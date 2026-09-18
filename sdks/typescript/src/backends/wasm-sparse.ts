/**
 * WASM Backend — sparse vector bookkeeping
 *
 * velesdb-wasm's sparse index keys postings by a document id and has no way
 * to delete a document's postings, nor to drop the terms a re-insert leaves
 * out: an insert overwrites only the terms it repeats. Indexed under the
 * point's own id, a deleted point, or a term a point no longer has, would
 * keep matching sparse queries.
 *
 * Each sparse upsert is therefore indexed under a fresh sparse id. Replacing
 * or deleting the point retires that id: its postings stay in the index but
 * map to no point, and searches skip them, over-fetching by the number of
 * retired ids so the dead cannot crowd live points out of the top `k`. As in
 * core, an upsert without a sparse vector keeps the point's current one.
 *
 * Retired postings would otherwise pile up for good, and every search would
 * over-fetch by their number, so the cost of a search would grow with the
 * replacements a collection has seen. The sparse index therefore lives in a
 * store of its own, a metadata-only velesdb-wasm store beside the
 * collection's vector store, and once retired
 * ids outnumber live ones it is rebuilt from the live sparse vectors the SDK
 * keeps: the index then never holds more than twice the live entries, and a
 * rebuild costs O(live) once per `live` retirements, O(1) amortized. The
 * collection's vectors and payloads are never touched. velesdb-wasm deleting
 * postings itself (#2287) will make this unnecessary.
 */

import type { SparseVector } from '../types';
import type { SparseIds, WasmSparseResult, WasmVectorStore } from './wasm-types';
import { sparseVectorToArrays } from './wasm-helpers';

/** Creates an empty store to hold a collection's sparse index. */
export type NewSparseStore = () => WasmVectorStore;

/** Empty bookkeeping for a new collection: its sparse store is created on first use. */
export function newSparseIds(): SparseIds {
  return { store: null, byPoint: new Map(), byId: new Map(), vectors: new Map(), dead: 0, next: 1n };
}

/** Index `vector` as `pointId`'s sparse vector, retiring the one it had. */
export function indexSparse(
  ids: SparseIds,
  newStore: NewSparseStore,
  pointId: number,
  vector: SparseVector
): void {
  const { indices, values } = sparseVectorToArrays(vector);
  const entry = { indices: new Uint32Array(indices), values: new Float32Array(values) };
  const sparseId = ids.next;
  (ids.store ??= newStore()).sparse_insert(sparseId, entry.indices, entry.values);
  ids.next += 1n;
  const previous = ids.byPoint.get(pointId);
  ids.byPoint.set(pointId, sparseId);
  ids.byId.set(sparseId, pointId);
  ids.vectors.set(sparseId, entry);
  if (previous !== undefined) {
    retireId(ids, newStore, previous);
  }
}

/** Retire `pointId`'s sparse id, if it has one: its postings stop matching. */
export function retireSparse(ids: SparseIds, newStore: NewSparseStore, pointId: number): void {
  const current = ids.byPoint.get(pointId);
  if (current === undefined) {
    return;
  }
  ids.byPoint.delete(pointId);
  retireId(ids, newStore, current);
}

/** Release the collection's sparse store. */
export function freeSparse(ids: SparseIds): void {
  ids.store?.free();
  ids.store = null;
}

/**
 * The top `k` sparse hits as `[pointId, score]`, live points only. `k` must
 * be a positive integer, as the search boundary guarantees
 * (`validateSearchInputs`); anything else fetches nothing, since the
 * over-fetch by `dead` would otherwise return live hits the `k` cap never
 * trims.
 */
export function sparseHits(
  ids: SparseIds,
  indices: number[],
  values: number[],
  k: number
): Array<[number, number]> {
  if (!Number.isInteger(k) || k <= 0 || ids.store === null) {
    return [];
  }
  const raw: WasmSparseResult[] = ids.store.sparse_search(
    new Uint32Array(indices),
    new Float32Array(values),
    k + ids.dead
  );
  const hits: Array<[number, number]> = [];
  for (const { doc_id, score } of raw) {
    const pointId = ids.byId.get(BigInt(doc_id));
    if (pointId !== undefined) {
      hits.push([pointId, score]);
      if (hits.length === k) {
        break;
      }
    }
  }
  return hits;
}

function retireId(ids: SparseIds, newStore: NewSparseStore, sparseId: bigint): void {
  ids.byId.delete(sparseId);
  ids.vectors.delete(sparseId);
  ids.dead += 1;
  if (ids.dead > ids.byId.size) {
    rebuildSparse(ids, newStore);
  }
}

/** Re-index the live sparse vectors into a fresh store, dropping every retired id with the old one. */
function rebuildSparse(ids: SparseIds, newStore: NewSparseStore): void {
  let fresh: WasmVectorStore | null = null;
  if (ids.vectors.size > 0) {
    fresh = newStore();
    for (const [sparseId, entry] of ids.vectors) {
      fresh.sparse_insert(sparseId, entry.indices, entry.values);
    }
  }
  ids.store?.free();
  ids.store = fresh;
  ids.dead = 0;
}
