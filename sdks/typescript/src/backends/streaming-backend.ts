/**
 * Streaming Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: trainPq, streamInsert, streamUpsertPoints.
 */

import type {
  VectorDocument,
  PqTrainOptions,
  SparseVector,
  RestPointId,
  StreamUpsertResponse,
} from '../types';
import { BackpressureError, ConnectionError, VelesDBError } from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath, toNumberArray } from './shared';

/** Minimal transport interface for streaming operations. */
export interface StreamingTransport extends BaseTransport {
  readonly baseUrl: string;
  readonly apiKey: string | undefined;
  readonly timeout: number;

  parseRestPointId(id: string | number): RestPointId;
  sparseVectorToRestFormat(sv: SparseVector): Record<string, number>;
  mapStatusToErrorCode(status: number): string;
  extractErrorPayload(data: unknown): { code?: string; message?: string };
}

export async function trainPq(
  transport: StreamingTransport,
  collection: string,
  options?: PqTrainOptions
): Promise<string> {
  const m = options?.m ?? 8;
  const k = options?.k ?? 256;
  const withClause = options?.opq
    ? `WITH (m=${m}, k=${k}, opq=true)`
    : `WITH (m=${m}, k=${k})`;
  const queryString = `TRAIN QUANTIZER ON ${collection} ${withClause}`;

  const response = await transport.requestJson<{ message: string }>(
    'POST',
    '/query',
    { query: queryString }
  );

  throwOnError(response);

  return response.data?.message ?? 'PQ training initiated';
}

/**
 * Why streamInsert does NOT use transport.requestJson:
 *
 * 1. **Backpressure signalling** — the streaming endpoint returns HTTP 429 to
 *    signal backpressure (the ingestion channel is full), which is semantically
 *    different from a rate-limit 429.  requestJson maps every non-2xx status to
 *    a generic VelesDBError; here we must catch 429 *before* that mapping and
 *    raise BackpressureError so callers can react with back-off logic.
 *
 * 2. **Custom endpoint** — the target URL is
 *    `<collection>/stream/insert`, not the shared `/query` endpoint used by
 *    requestJson, so the helper's URL construction does not apply.
 *
 * 3. **202 Accepted** — the endpoint may return 202 (enqueued, not yet
 *    persisted) as a success code; requestJson treats only 2xx as success
 *    but does not special-case 202 vs 200 for streaming semantics.
 */
export async function streamInsert(
  transport: StreamingTransport,
  collection: string,
  docs: VectorDocument[]
): Promise<void> {
  for (const doc of docs) {
    const restId = transport.parseRestPointId(doc.id);
    const vector = toNumberArray(doc.vector);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const body: Record<string, any> = {
      id: restId,
      vector,
      payload: doc.payload ?? null,
    };

    if (doc.sparseVector) {
      body.sparse_vector = transport.sparseVectorToRestFormat(doc.sparseVector);
    }

    const url = `${transport.baseUrl}${collectionPath(collection)}/stream/insert`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    if (transport.apiKey) {
      headers['Authorization'] = `Bearer ${transport.apiKey}`;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), transport.timeout);

    try {
      const response = await fetch(url, {
        method: 'POST',
        headers,
        body: JSON.stringify(body),
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (response.status === 429) {
        throw new BackpressureError();
      }

      if (!response.ok && response.status !== 202) {
        const data = await response.json().catch(() => ({}));
        const errorPayload = transport.extractErrorPayload(data);
        throw new VelesDBError(
          errorPayload.message ?? `HTTP ${response.status}`,
          errorPayload.code ?? transport.mapStatusToErrorCode(response.status)
        );
      }
    } catch (error) {
      clearTimeout(timeoutId);

      if (error instanceof BackpressureError || error instanceof VelesDBError) {
        throw error;
      }

      if (error instanceof Error && error.name === 'AbortError') {
        throw new ConnectionError('Request timeout');
      }

      throw new ConnectionError(
        `Stream insert failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        error instanceof Error ? error : undefined
      );
    }
  }
}

/**
 * Batch upsert points via the NDJSON streaming endpoint.
 *
 * Sends all documents as a single `application/x-ndjson` POST to
 * `POST /collections/{name}/points/stream` (100 MB body limit on the server).
 * Each document is serialised as one JSON line (NDJSON format) matching the
 * server-side `Point` schema: `{ "id": <u64>, "vector": [...], "payload": {...} }`.
 *
 * Unlike `streamInsert` (which sends one point per HTTP request via the
 * bounded ingestion channel), this function sends the entire batch in a
 * single request, making it far more efficient for bulk loads.
 *
 * The server returns statistics about the processing (inserted, malformed,
 * failed, network errors). If any points failed, the function still resolves
 * with the response so the caller can inspect partial results. A 429 response
 * raises `BackpressureError` for caller-side retry logic.
 */
export async function streamUpsertPoints(
  transport: StreamingTransport,
  collection: string,
  docs: VectorDocument[]
): Promise<StreamUpsertResponse> {
  const ndjsonLines = docs.map(doc => {
    const restId = transport.parseRestPointId(doc.id);
    const vector = toNumberArray(doc.vector);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const point: Record<string, any> = {
      id: restId,
      vector,
      payload: doc.payload ?? null,
    };

    if (doc.sparseVector) {
      point.sparse_vector = transport.sparseVectorToRestFormat(doc.sparseVector);
    }

    return JSON.stringify(point);
  });

  const body = ndjsonLines.join('\n');
  const url = `${transport.baseUrl}${collectionPath(collection)}/points/stream`;
  const headers: Record<string, string> = {
    'Content-Type': 'application/x-ndjson',
  };

  if (transport.apiKey) {
    headers['Authorization'] = `Bearer ${transport.apiKey}`;
  }

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), transport.timeout);

  try {
    const response = await fetch(url, {
      method: 'POST',
      headers,
      body,
      signal: controller.signal,
    });

    clearTimeout(timeoutId);

    if (response.status === 429) {
      throw new BackpressureError();
    }

    if (!response.ok) {
      const data = await response.json().catch(() => ({}));
      const errorPayload = transport.extractErrorPayload(data);
      throw new VelesDBError(
        errorPayload.message ?? `HTTP ${response.status}`,
        errorPayload.code ?? transport.mapStatusToErrorCode(response.status)
      );
    }

    const data = await response.json().catch(() => ({}));
    return {
      message: typeof data.message === 'string' ? data.message : 'Stream processed',
      inserted: typeof data.inserted === 'number' ? data.inserted : 0,
      malformed: typeof data.malformed === 'number' ? data.malformed : 0,
      failedUpserts: typeof data.failed_upserts === 'number' ? data.failed_upserts : 0,
      networkErrors: typeof data.network_errors === 'number' ? data.network_errors : 0,
    };
  } catch (error) {
    clearTimeout(timeoutId);

    if (error instanceof BackpressureError || error instanceof VelesDBError) {
      throw error;
    }

    if (error instanceof Error && error.name === 'AbortError') {
      throw new ConnectionError('Request timeout');
    }

    throw new ConnectionError(
      `Stream upsert failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
      error instanceof Error ? error : undefined
    );
  }
}
