/**
 * Optional embedding helpers for the VelesDB TypeScript SDK.
 *
 * A thin {@link Embedder} interface plus an adapter for OpenAI-compatible
 * endpoints. The adapter uses the global `fetch` API (Node ≥ 18, browsers,
 * Deno) and has no additional runtime dependencies.
 *
 * @example
 * ```typescript
 * import { VelesDB } from '@wiscale/velesdb-sdk';
 * import { OpenAIEmbedder } from '@wiscale/velesdb-sdk/embed';
 *
 * const embedder = new OpenAIEmbedder({ apiKey: process.env.OPENAI_API_KEY! });
 * const db = new VelesDB({ backend: 'wasm' });
 * await db.init();
 * await db.createCollection('docs', { dimension: embedder.dimension ?? 1536 });
 * const vectors = await embedder.embed(['hello world', 'vector search']);
 * ```
 */

export interface Embedder {
  /** Embedding dimension, or `0` if not yet known (determined after first call). */
  readonly dimension: number;
  embed(texts: string[]): Promise<number[][]>;
}

export interface OpenAIEmbedderOptions {
  model?: string;
  apiKey: string;
  /** Override the base URL for Azure OpenAI, vLLM, or any compatible endpoint. */
  baseUrl?: string;
  /** Request a specific output dimension (requires a model that supports it). */
  dimensions?: number;
}

interface OpenAIEmbeddingResponse {
  data: Array<{ embedding: number[] }>;
}

export class OpenAIEmbedder implements Embedder {
  private readonly model: string;
  private readonly apiKey: string;
  private readonly baseUrl: string;
  private readonly requestedDimensions: number | undefined;
  dimension: number;

  constructor(options: OpenAIEmbedderOptions) {
    this.model = options.model ?? 'text-embedding-3-small';
    this.apiKey = options.apiKey;
    this.baseUrl = options.baseUrl?.replace(/\/$/, '') ?? 'https://api.openai.com/v1';
    this.requestedDimensions = options.dimensions;
    this.dimension = options.dimensions ?? 0;
  }

  async embed(texts: string[]): Promise<number[][]> {
    if (texts.length === 0) return [];

    const body: Record<string, unknown> = { model: this.model, input: texts };
    if (this.requestedDimensions !== undefined) {
      body['dimensions'] = this.requestedDimensions;
    }

    const response = await fetch(`${this.baseUrl}/embeddings`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${this.apiKey}`,
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      const text = await response.text().catch(() => '');
      throw new Error(
        `OpenAI embeddings request failed: ${response.status} ${response.statusText} — ${text.slice(0, 500)}`,
      );
    }

    const json = (await response.json()) as OpenAIEmbeddingResponse;
    const vectors = json.data.map((item) => item.embedding);
    if (this.dimension === 0 && vectors.length > 0) {
      this.dimension = vectors[0].length;
    }
    return vectors;
  }
}
