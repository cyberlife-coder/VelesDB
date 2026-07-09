/**
 * REST Backend - HTTP infrastructure
 *
 * Core HTTP request handling, error mapping, and transport adapter
 * construction for the RestBackend class.
 * @packageDocumentation
 */

import type { SparseVector } from '../types';
import { ConnectionError } from '../types';
import type { TransportResponse, BaseTransport } from './shared';
import { safeJsonParse } from './shared';
import type { CrudTransport, RawBulkTransport } from './crud-backend';
import { parseRestPointId, sparseVectorToRestFormat } from './crud-backend';
import type { SearchTransport } from './search-backend';
import type { AgentMemoryTransport } from './agent-memory-backend';
import type { QueryTransport } from './query-backend';
import type { StreamingTransport } from './streaming-backend';
import type { SearchResult } from '../types';

/** Configuration for the REST HTTP client. */
export interface RestHttpConfig {
  baseUrl: string;
  apiKey?: string;
  timeout: number;
  /** Max automatic retries on 429/503 backpressure responses (default 2). */
  maxRetries?: number;
  /** Base delay in ms for exponential backoff between retries (default 200). */
  retryBaseDelayMs?: number;
}

/** Default number of retries on 429/503 when not configured. */
const DEFAULT_MAX_RETRIES = 2;
/** Default exponential-backoff base delay in ms. */
const DEFAULT_RETRY_BASE_MS = 200;

/** HTTP status → typed error code lookup. */
const STATUS_ERROR_CODES: Record<number, string> = {
  400: 'BAD_REQUEST',
  401: 'UNAUTHORIZED',
  403: 'FORBIDDEN',
  404: 'NOT_FOUND',
  409: 'CONFLICT',
  429: 'RATE_LIMITED',
  500: 'INTERNAL_ERROR',
  503: 'SERVICE_UNAVAILABLE',
};

/** Map an HTTP status code to a typed error code string. */
export function mapStatusToErrorCode(status: number): string {
  return STATUS_ERROR_CODES[status] ?? 'UNKNOWN_ERROR';
}

/** Safely extract a string field from an object, checking multiple keys. */
function stringField(obj: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const key of keys) {
    if (typeof obj[key] === 'string') return obj[key] as string;
  }
  return undefined;
}

/** Extract error code and message from an error response payload. */
export function extractErrorPayload(data: unknown): { code?: string; message?: string } {
  if (!data || typeof data !== 'object') return {};
  const payload = data as Record<string, unknown>;
  const nested = typeof payload.error === 'object' && payload.error
    ? payload.error as Record<string, unknown>
    : payload;
  return {
    code: stringField(nested, 'code') ?? stringField(payload, 'code'),
    message: stringField(nested, 'message') ?? stringField(payload, 'message', 'error'),
  };
}

/** Parse a node ID from an unknown value (bigint, number, or string). */
export function parseNodeId(value: unknown): bigint | number {
  if (value === null || value === undefined) { return 0; }
  if (typeof value === 'bigint') { return value; }
  if (typeof value === 'string') {
    const num = Number(value);
    return num > Number.MAX_SAFE_INTEGER ? BigInt(value) : num;
  }
  if (typeof value === 'number') { return value; }
  return 0;
}

/** Execute an HTTP request against the REST API. */
/** Build request headers from config. */
function buildHeaders(config: RestHttpConfig): Record<string, string> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (config.apiKey) headers['Authorization'] = `Bearer ${config.apiKey}`;
  return headers;
}

/** Wrap a caught error as a ConnectionError. */
function wrapCatchError(error: unknown): never {
  if (error instanceof Error && error.name === 'AbortError') {
    throw new ConnectionError('Request timeout');
  }
  const message = error instanceof Error ? error.message : 'Unknown error';
  const cause = error instanceof Error ? error : undefined;
  throw new ConnectionError(`Request failed: ${message}`, cause);
}

/** Promise-based delay used between retries. */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Compute the backoff delay before retrying a 429/503 response. Honors an
 * integer `Retry-After` header (seconds); otherwise exponential backoff with
 * light jitter.
 */
function retryDelayMs(response: Response, attempt: number, baseDelayMs: number): number {
  const retryAfter = response.headers.get('Retry-After');
  if (retryAfter !== null) {
    const secs = Number(retryAfter);
    if (Number.isFinite(secs) && secs >= 0) return secs * 1000;
  }
  return baseDelayMs * 2 ** attempt + Math.floor(Math.random() * baseDelayMs);
}

/** Perform a single HTTP attempt (own timeout); network errors throw. */
async function attemptFetch(
  config: RestHttpConfig,
  method: string,
  path: string,
  body?: unknown
): Promise<Response> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), config.timeout);
  try {
    return await fetch(`${config.baseUrl}${path}`, {
      method,
      headers: buildHeaders(config),
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
  } catch (error) {
    wrapCatchError(error);
  } finally {
    clearTimeout(timeoutId);
  }
}

/**
 * Execute an HTTP request against the REST API.
 *
 * Retries automatically on 429 (rate limited) and 503 (service unavailable),
 * which the server returns *before* processing the request, so a replay is
 * safe even for writes. Network errors and timeouts are NOT retried (a write
 * may already have been applied). Backoff is exponential, honoring `Retry-After`.
 */
export async function request<T>(
  config: RestHttpConfig,
  method: string,
  path: string,
  body?: unknown
): Promise<TransportResponse<T>> {
  const maxRetries = config.maxRetries ?? DEFAULT_MAX_RETRIES;
  const baseDelay = config.retryBaseDelayMs ?? DEFAULT_RETRY_BASE_MS;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    const response = await attemptFetch(config, method, path, body);
    const data = await safeJsonParse(response);
    if (response.ok) {
      return { data: data as unknown as T };
    }
    if ((response.status === 429 || response.status === 503) && attempt < maxRetries) {
      await sleep(retryDelayMs(response, attempt, baseDelay));
      continue;
    }
    const ep = extractErrorPayload(data);
    return { error: {
      code: ep.code ?? mapStatusToErrorCode(response.status),
      message: ep.message ?? `HTTP ${response.status}`,
    }};
  }
  // Unreachable: the final iteration always returns (retry guard is attempt < maxRetries).
  throw new ConnectionError('Request failed: retries exhausted');
}

// ============================================================================
// Transport adapter factories
// ============================================================================

/** Build a BaseTransport adapter. */
export function buildBaseTransport(config: RestHttpConfig): BaseTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
  };
}

/** Build a CrudTransport adapter. */
export function buildCrudTransport(config: RestHttpConfig): CrudTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
  };
}

/** Build a RawBulkTransport adapter (raw `fetch` primitives for binary bodies). */
export function buildRawBulkTransport(config: RestHttpConfig): RawBulkTransport {
  return {
    baseUrl: config.baseUrl,
    apiKey: config.apiKey,
    timeout: config.timeout,
  };
}

/** Build a SearchTransport adapter. */
export function buildSearchTransport(config: RestHttpConfig): SearchTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
    sparseToRest: (sv: SparseVector) => sparseVectorToRestFormat(sv),
  };
}

/** Build a QueryTransport adapter. */
export function buildQueryTransport(config: RestHttpConfig): QueryTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
    parseNodeId: (v: unknown) => parseNodeId(v),
  };
}

/** Build a StreamingTransport adapter. */
export function buildStreamingTransport(config: RestHttpConfig): StreamingTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
    baseUrl: config.baseUrl,
    apiKey: config.apiKey,
    timeout: config.timeout,
    parseRestPointId,
    sparseVectorToRestFormat,
    mapStatusToErrorCode: (s: number) => mapStatusToErrorCode(s),
    extractErrorPayload: (d: unknown) => extractErrorPayload(d),
  };
}

/** Build an AgentMemoryTransport adapter (requires a search function ref). */
export function buildAgentMemoryTransport(
  config: RestHttpConfig,
  searchFn: (c: string, e: number[], opts: { k: number; filter: Record<string, string> }) => Promise<SearchResult[]>
): AgentMemoryTransport {
  return {
    requestJson: <T>(m: string, p: string, b?: unknown) => request<T>(config, m, p, b),
    searchVectors: (c: string, e: number[], k: number, f: Record<string, string>) =>
      searchFn(c, e, { k, filter: f }),
  };
}
