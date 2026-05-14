// Deterministic hashing-trick embedder. Same projection runs at ingest and at
// query time so lexical overlap drives ranking. This is a placeholder — real
// applications should swap it for a sentence-transformer pipeline (e.g.
// `@xenova/transformers` for an in-process model, or a remote embedding API).

export const EMBEDDING_DIM = 384;

const WORD_RE = /[a-z0-9]+/g;

function hash(token: string, seed: number): number {
  let h = seed ^ 2166136261;
  for (let i = 0; i < token.length; i++) {
    h = Math.imul(h ^ token.charCodeAt(i), 16777619);
  }
  return h >>> 0;
}

export function embed(text: string): number[] {
  const vec = new Array<number>(EMBEDDING_DIM).fill(0);
  const tokens = text.toLowerCase().match(WORD_RE);
  if (!tokens) return vec;

  for (const token of tokens) {
    const idx = hash(token, 0) % EMBEDDING_DIM;
    const sign = (hash(token, 1) & 1) === 0 ? 1 : -1;
    vec[idx] += sign;
  }

  let norm = 0;
  for (let i = 0; i < EMBEDDING_DIM; i++) norm += vec[i] * vec[i];
  norm = Math.sqrt(norm);
  if (norm > 0) {
    for (let i = 0; i < EMBEDDING_DIM; i++) vec[i] /= norm;
  }
  return vec;
}
