// Tiny deterministic text → vector embedder used for both the seed corpus and
// live search queries. Hashes each token into the embedding space (the
// classic "hashing trick" / random-projection sketch) and L2-normalizes so the
// cosine metric in VelesDB returns scores in [-1, 1].
//
// Trade-off: this is a bag-of-words sketch with no semantics — it surfaces
// lexical overlap, not meaning. Real applications would swap this for a
// sentence-transformer embedding (server-side or via transformers.js). The
// goal here is to keep the demo self-contained: zero model downloads,
// zero backend, deterministic results.

export const EMBEDDING_DIM = 64;

const WORD_RE = /[a-z0-9]+/g;

function hash(token: string, seed: number): number {
  let h = seed ^ 2166136261;
  for (let i = 0; i < token.length; i++) {
    h = Math.imul(h ^ token.charCodeAt(i), 16777619);
  }
  return h >>> 0;
}

export function embed(text: string): Float32Array {
  const vec = new Float32Array(EMBEDDING_DIM);
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
