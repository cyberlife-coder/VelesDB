/**
 * WASM Backend — sparse vector bookkeeping
 *
 * velesdb-wasm's sparse index keys postings by a document id. A binding that
 * exposes `sparse_remove` (after 8.0.0, #2287) deletes a document's postings
 * outright; 8.0.0, the oldest release the SDK's dependency range admits, has
 * no way to delete them, nor to drop the terms a re-insert leaves out: an
 * insert there overwrites only the terms it repeats. Indexed under the
 * point's own id, a deleted point, or a term a point no longer has, would
 * keep matching sparse queries.
 *
 * Each sparse upsert is therefore indexed under a fresh sparse id on every
 * binding generation. Replacing or deleting the point retires that id, and
 * `retireId` does what the binding allows: on a `sparse_remove` binding it
 * deletes the id's postings — `dead` never grows, searches fetch exactly
 * `k`, and the index holds live entries only; on 8.0.0 the postings stay but
 * map to no point, searches skip them and over-fetch by the number of
 * retired ids so the dead cannot crowd live points out of the top `k`. As in
 * core (`dropped_sparse_entries`, #2834), an upsert replaces the whole point:
 * one without a sparse vector retires the point's current one.
 *
 * An upsert stages its sparse vectors before the point is written
 * ({@link stageSparse}): each goes into the index under a fresh id no point
 * maps to yet, so the binding validates it — and refuses a non-finite weight
 * — before the dense write, and no search sees it until {@link commitSparse}
 * hands it to its point. A refused stage, or a dense write that fails after
 * it, abandons what was staged and leaves the point as it was.
 *
 * Without `sparse_remove`, retired postings would pile up for good and every
 * search would over-fetch by their number, so the cost of a search would
 * grow with the replacements a collection has seen. The sparse index
 * therefore lives in a store of its own, a metadata-only velesdb-wasm store
 * beside the collection's vector store, and once retired ids outnumber live
 * ones it is rebuilt from the live sparse vectors the SDK keeps: the index
 * then never holds more than twice the live entries, and a rebuild costs
 * O(live) once per `live` retirements, O(1) amortized. The collection's
 * vectors and payloads are never touched. Once the dependency floor passes
 * the release carrying `sparse_remove`, the tombstone path — and the
 * fresh-id scheme itself — can go.
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

/**
 * A sparse vector inserted under a fresh sparse id that no point maps to yet:
 * the binding accepted it, and no search returns it before
 * {@link commitSparse}.
 */
export type StagedSparse = bigint;

/**
 * Stage each of `vectors` (`undefined` stages nothing). When the binding
 * refuses one, the ones staged before it are abandoned and its error is
 * rethrown, leaving the index as it was.
 */
export function stageSparse(
  ids: SparseIds,
  newStore: NewSparseStore,
  vectors: ReadonlyArray<SparseVector | undefined>
): Array<StagedSparse | undefined> {
  const staged: Array<StagedSparse | undefined> = [];
  try {
    for (const vector of vectors) {
      staged.push(vector === undefined ? undefined : stageOne(ids, newStore, vector));
    }
  } catch (error) {
    abandonSparse(ids, newStore, staged);
    throw error;
  }
  return staged;
}

/**
 * Give each point of `pointIds` the vector staged at the same position,
 * retiring the one it had; a point with none staged loses its sparse vector,
 * as its new version carries none. A point listed twice keeps the last.
 */
export function commitSparse(
  ids: SparseIds,
  newStore: NewSparseStore,
  pointIds: readonly bigint[],
  staged: ReadonlyArray<StagedSparse | undefined>
): void {
  pointIds.forEach((pointId, i) => {
    const sparseId = staged[i];
    if (sparseId === undefined) {
      retireSparse(ids, newStore, pointId);
      return;
    }
    const previous = ids.byPoint.get(pointId);
    ids.byPoint.set(pointId, sparseId);
    ids.byId.set(sparseId, pointId);
    if (previous !== undefined) {
      retireId(ids, newStore, previous);
    }
  });
}

/** Retire staged vectors whose point was never written. */
export function abandonSparse(
  ids: SparseIds,
  newStore: NewSparseStore,
  staged: ReadonlyArray<StagedSparse | undefined>
): void {
  for (const sparseId of staged) {
    if (sparseId !== undefined) {
      retireId(ids, newStore, sparseId);
    }
  }
}

function stageOne(ids: SparseIds, newStore: NewSparseStore, vector: SparseVector): StagedSparse {
  const { indices, values } = sparseVectorToArrays(vector);
  const entry = { indices: new Uint32Array(indices), values: new Float32Array(values) };
  const sparseId = ids.next;
  (ids.store ??= newStore()).sparse_insert(sparseId, entry.indices, entry.values);
  ids.next += 1n;
  // In `vectors` at once, so a rebuild before the commit keeps it; not in
  // `byId`, so no search returns it until then.
  ids.vectors.set(sparseId, entry);
  return sparseId;
}

/** Retire `pointId`'s sparse id, if it has one: its postings stop matching. */
export function retireSparse(ids: SparseIds, newStore: NewSparseStore, pointId: bigint): void {
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
 * The top `k` sparse hits as `[pointId, score]`, live points only, each
 * point id the exact `BigInt` it was upserted under. `k` must
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
): Array<[bigint, number]> {
  if (!Number.isInteger(k) || k <= 0 || ids.store === null) {
    return [];
  }
  const raw: WasmSparseResult[] = ids.store.sparse_search(
    new Uint32Array(indices),
    new Float32Array(values),
    k + ids.dead
  );
  const hits: Array<[bigint, number]> = [];
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
  const store = ids.store;
  if (store !== null && typeof store.sparse_remove === 'function') {
    store.sparse_remove(sparseId);
    return;
  }
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
