/**
 * A stand-in for velesdb-wasm's sparse index
 * (`crates/velesdb-wasm/src/sparse.rs`) that keeps its limits: postings are
 * keyed by term, then document; an insert overwrites the (term, document)
 * weights it repeats and leaves that document's other terms in place;
 * nothing is ever removed; a search sums query × document weight per
 * document and returns the top `k` by score.
 *
 * Tests use it so the SDK's sparse bookkeeping is checked against what the
 * binding actually does, not against an index that forgets on its behalf.
 */
export class FakeSparseIndex {
  private readonly postings = new Map<number, Map<bigint, number>>();

  insert(docId: bigint, indices: Uint32Array, values: Float32Array): void {
    indices.forEach((term, i) => {
      const list = this.postings.get(term) ?? new Map<bigint, number>();
      list.set(BigInt(docId), values[i]!);
      this.postings.set(term, list);
    });
  }

  search(
    indices: Uint32Array,
    values: Float32Array,
    k: number
  ): Array<{ doc_id: bigint; score: number }> {
    const scores = new Map<bigint, number>();
    indices.forEach((term, i) => {
      for (const [doc, weight] of this.postings.get(term) ?? []) {
        scores.set(doc, (scores.get(doc) ?? 0) + values[i]! * weight);
      }
    });
    return [...scores]
      .map(([doc_id, score]) => ({ doc_id, score }))
      .sort((a, b) => b.score - a.score)
      .slice(0, k);
  }
}
