/**
 * Optional embedding helpers for the VelesDB TypeScript SDK.
 *
 * Skeleton adapters around common providers. The OpenAI adapter is
 * fetch-based and ships with the SDK (no extra dependency); other
 * providers can be added by implementing the `Embedder` interface.
 *
 * @example
 * ```ts
 * import { VelesDB } from '@wiscale/velesdb-sdk';
 * import { OpenAIEmbedder } from '@wiscale/velesdb-sdk/embed';
 *
 * const embedder = new OpenAIEmbedder({ apiKey: process.env.OPENAI_API_KEY! });
 * const [vec] = await embedder.embed(['hello world']);
 *
 * const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
 * await db.init();
 * await db.upsert('docs', { id: '1', vector: vec });
 * ```
 *
 * @packageDocumentation
 */

export interface Embedder {
  readonly dimension: number;
  embed(texts: string[]): Promise<number[][]>;
}

export interface OpenAIEmbedderOptions {
  /** Model identifier, e.g. `text-embedding-3-small`. */
  model?: string;
  /** API key. Falls back to `process.env.OPENAI_API_KEY` in Node. */
  apiKey?: string;
  /** Override for Azure OpenAI, vLLM, or any OpenAI-compatible endpoint. */
  baseUrl?: string;
  /** Force a specific output dimension (supported by `text-embedding-3-*`). */
  dimensions?: number;
  /** Inject a custom fetch implementation (defaults to `globalThis.fetch`). */
  fetch?: typeof fetch;
}

interface OpenAIEmbeddingResponse {
  data: Array<{ embedding: number[] }>;
}

/**
 * OpenAI / Azure-OpenAI embedding adapter.
 *
 * Uses `fetch` directly so there's no runtime dependency on the official
 * `openai` package. Compatible with any OpenAI-shaped `/v1/embeddings`
 * endpoint via the `baseUrl` option.
 */
export class OpenAIEmbedder implements Embedder {
  readonly model: string;
  private readonly apiKey: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private _dimension: number;

  constructor(options: OpenAIEmbedderOptions = {}) {
    this.model = options.model ?? 'text-embedding-3-small';

    const envKey =
      typeof process !== 'undefined' && process.env
        ? process.env.OPENAI_API_KEY
        : undefined;
    const key = options.apiKey ?? envKey;
    if (!key) {
      throw new Error(
        'OpenAIEmbedder requires an `apiKey` option or OPENAI_API_KEY env var.',
      );
    }
    this.apiKey = key;

    this.baseUrl = (options.baseUrl ?? 'https://api.openai.com/v1').replace(
      /\/$/,
      '',
    );

    const fetchFn = options.fetch ?? globalThis.fetch;
    if (typeof fetchFn !== 'function') {
      throw new Error(
        'No fetch implementation available — pass `options.fetch` (Node < 18 has no global fetch).',
      );
    }
    this.fetchImpl = fetchFn;

    this._dimension = options.dimensions ?? 0;
  }

  get dimension(): number {
    if (this._dimension === 0) {
      throw new Error(
        'Dimension unknown — pass `dimensions` to the constructor or call `embed()` once first.',
      );
    }
    return this._dimension;
  }

  async embed(texts: string[]): Promise<number[][]> {
    if (texts.length === 0) return [];

    const body: Record<string, unknown> = {
      model: this.model,
      input: texts,
    };
    if (this._dimension > 0) {
      body.dimensions = this._dimension;
    }

    const response = await this.fetchImpl(`${this.baseUrl}/embeddings`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${this.apiKey}`,
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      const errorBody = await response.text();
      throw new Error(
        `OpenAI embeddings request failed: ${response.status} ${response.statusText} — ${errorBody}`,
      );
    }

    const payload = (await response.json()) as OpenAIEmbeddingResponse;
    const vectors = payload.data.map((item) => item.embedding);

    if (this._dimension === 0 && vectors.length > 0) {
      const first = vectors[0];
      if (first) {
        this._dimension = first.length;
      }
    }

    return vectors;
  }
}
