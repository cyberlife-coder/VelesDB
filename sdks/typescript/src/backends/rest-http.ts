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
import type { CrudTransport } from './crud-backend';
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
}

/** Map an HTTP status code to a typed error code string. */
export function mapStatusToErrorCode(status: number): string {
  switch (status) {
    case 400: return 'BAD_REQUEST';
    case 401: return 'UNAUTHORIZED';
    case 403: return 'FORBIDDEN';
    case 404: return 'NOT_FOUND';
    case 409: return 'CONFLICT';
    case 429: return 'RATE_LIMITED';
    case 500: return 'INTERNAL_ERROR';
    case 503: return 'SERVICE_UNAVAILABLE';
    default:  return 'UNKNOWN_ERROR';
  }
}

/** Extract error code and message from an error response payload. */
export function extractErrorPayload(data: unknown): { code?: string; message?: string } {
  if (!data || typeof data !== 'object') {
    return {};
  }
  const payload = data as Record<string, unknown>;
  const nestedError =
    payload.error && typeof payload.error === 'object'
      ? (payload.error as Record<string, unknown>)
      : undefined;
  const codeField = nestedError?.code ?? payload.code;
  const code = typeof codeField === 'string' ? codeField : undefined;
  const messageField = nestedError?.message ?? payload.message ?? payload.error;
  const message = typeof messageField === 'string' ? messageField : undefined;
  return { code, message };
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
export async function request<T>(
  config: RestHttpConfig,
  method: string,
  path: string,
  body?: unknown
): Promise<TransportResponse<T>> {
  const url = `${config.baseUrl}${path}`;
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (config.apiKey) {
    headers['Authorization'] = `Bearer ${config.apiKey}`;
  }
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), config.timeout);

  try {
    const response = await fetch(url, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    clearTimeout(timeoutId);
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      const errorPayload = extractErrorPayload(data);
      return {
        error: {
          code: errorPayload.code ?? mapStatusToErrorCode(response.status),
          message: errorPayload.message ?? `HTTP ${response.status}`,
        },
      };
    }
    return { data };
  } catch (error) {
    clearTimeout(timeoutId);
    if (error instanceof Error && error.name === 'AbortError') {
      throw new ConnectionError('Request timeout');
    }
    throw new ConnectionError(
      `Request failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
      error instanceof Error ? error : undefined
    );
  }
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
