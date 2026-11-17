/**
 * A stand-in for velesdb-wasm's sparse index
 * (`crates/velesdb-wasm/src/sparse.rs`) that keeps its limits: postings are
 * keyed by term, then document; an insert overwrites the (term, document)
 * weights it repeats and leaves that document's other terms in place;
 * an insert carrying a non-finite weight is refused whole, with the
 * binding's message thrown as a string, as wasm-bindgen throws it;
 * `remove` deletes a document's postings, as `sparse_remove` does on the
 * bindings that carry it (8.0.0 does not — tests mock its absence by not
 * exposing `sparse_remove`); a search sums query × document weight per
 * document and returns the top `k` by score, each `doc_id` its decimal
 * string, as the binding writes every id it returns.
 *
 * Tests use it so the SDK's sparse bookkeeping is checked against what the
 * binding actually does, not against an index that forgets on its behalf.
 */
export class FakeSparseIndex {
  private readonly postings = new Map<number, Map<bigint, number>>();

  insert(docId: bigint, indices: Uint32Array, values: Float32Array): void {
    indices.forEach((term, i) => {
      if (!Number.isFinite(values[i]!)) {
        throw `sparse weight of term ${term} is not finite: ${values[i]}`;
      }
    });
    indices.forEach((term, i) => {
      const list = this.postings.get(term) ?? new Map<bigint, number>();
      list.set(BigInt(docId), values[i]!);
      this.postings.set(term, list);
    });
  }

  /** Delete `docId`'s postings. `true` when it had any, as the binding reports. */
  remove(docId: bigint): boolean {
    let found = false;
    for (const [term, list] of this.postings) {
      found = list.delete(BigInt(docId)) || found;
      if (list.size === 0) {
        this.postings.delete(term);
      }
    }
    return found;
  }

  /** Live (term, document) entries — what the binding's memory holds. */
  get size(): number {
    let n = 0;
    for (const list of this.postings.values()) {
      n += list.size;
    }
    return n;
  }

  search(
    indices: Uint32Array,
    values: Float32Array,
    k: number
  ): Array<{ doc_id: string; score: number }> {
    const scores = new Map<bigint, number>();
    indices.forEach((term, i) => {
      for (const [doc, weight] of this.postings.get(term) ?? []) {
        scores.set(doc, (scores.get(doc) ?? 0) + values[i]! * weight);
      }
    });
    return [...scores]
      .map(([doc, score]) => ({ doc_id: String(doc), score }))
      .sort((a, b) => b.score - a.score)
      .slice(0, k);
  }
}
