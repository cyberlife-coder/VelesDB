/**
 * WASM Backend — sparse vectors
 *
 * A point's sparse vector lives in its collection's own velesdb-wasm store,
 * under the point's own id: `sparse_insert` replaces the document's postings
 * and `sparse_remove` deletes them. VelesQL's `SPARSE_NEAR` runs over that
 * same store (`wasm-velesql.ts` shares it with the executor), so it sees
 * exactly what the typed sparse and hybrid searches see.
 *
 * As in core (`dropped_sparse_entries`, #2834), an upsert replaces the whole
 * point: one without a sparse vector removes the point's current one. The
 * binding validates every sparse vector of an upsert before any point is
 * written ({@link validateSparse}), so a refused one leaves the point as it
 * was.
 */

import type { SparseVector } from '../types';
import type { WasmSparseResult, WasmVectorStore } from './wasm-types';
import { sparseVectorToArrays } from './wasm-helpers';

/** Creates an empty, metadata-only store. */
export type NewScratchStore = () => WasmVectorStore;

function toArrays(vector: SparseVector): [Uint32Array, Float32Array] {
  const { indices, values } = sparseVectorToArrays(vector);
  return [new Uint32Array(indices), new Float32Array(values)];
}

/**
 * Throw the binding's refusal of the first of `vectors` it would refuse
 * (`undefined` entries are skipped), touching no collection: each is
 * inserted into a scratch store, freed afterwards, so the check is the
 * binding's own and the SDK keeps no copy of it.
 */
export function validateSparse(
  newScratch: NewScratchStore,
  vectors: ReadonlyArray<SparseVector | undefined>
): void {
  if (vectors.every((vector) => vector === undefined)) {
    return;
  }
  const scratch = newScratch();
  try {
    for (const vector of vectors) {
      if (vector !== undefined) {
        scratch.sparse_insert(0n, ...toArrays(vector));
      }
    }
  } finally {
    scratch.free();
  }
}

/**
 * Give each point of `pointIds` the vector at the same position of
 * `vectors`, or none where that is `undefined`. A point listed twice keeps
 * the last.
 */
export function writeSparse(
  store: WasmVectorStore,
  pointIds: readonly bigint[],
  vectors: ReadonlyArray<SparseVector | undefined>
): void {
  pointIds.forEach((pointId, i) => {
    const vector = vectors[i];
    if (vector === undefined) {
      store.sparse_remove(pointId);
    } else {
      store.sparse_insert(pointId, ...toArrays(vector));
    }
  });
}

/** The top `k` sparse hits of `store` as `[pointId, score]`, best first. */
export function sparseHits(
  store: WasmVectorStore,
  indices: number[],
  values: number[],
  k: number
): Array<[bigint, number]> {
  const raw: WasmSparseResult[] = store.sparse_search(
    new Uint32Array(indices),
    new Float32Array(values),
    k
  );
  return raw.map(({ doc_id, score }) => [BigInt(doc_id), score]);
}
