/**
 * SSE backend operations for the VelesDB REST API.
 *
 * Covers `GET /collections/{name}/graph/traverse/stream`: a Server-Sent
 * Events stream of traversal events (`node`, `stats`, `done`, `error`).
 * Like the NDJSON bulk upsert, this cannot go through `requestJson` (the
 * body is an event stream, not one JSON document), so it takes the raw
 * connection primitives directly.
 */

import type { StreamTraverseRequest, TraverseStreamEvent } from '../types';
import { VelesDBError, ConnectionError } from '../types';
import { collectionPath, safeJsonParse } from './shared';
import { parseRestPointId } from './crud-backend';
import { extractErrorPayload, mapStatusToErrorCode } from './rest-http';

/** Transport fields needed to open an SSE stream via `fetch`. */
export interface SseTransport {
  readonly baseUrl: string;
  readonly apiKey: string | undefined;
  readonly timeout: number;
}

/** Query-string wire shape of `StreamTraverseParams`. */
function streamTraverseQuery(request: StreamTraverseRequest): string {
  const params = new URLSearchParams();
  params.set('start_node', String(parseRestPointId(request.startNode)));
  params.set('algorithm', request.algorithm ?? 'bfs');
  if (request.maxDepth !== undefined) params.set('max_depth', String(request.maxDepth));
  if (request.limit !== undefined) params.set('limit', String(request.limit));
  if (request.relationshipTypes?.length) {
    params.set('relationship_types', request.relationshipTypes.join(','));
  }
  return params.toString();
}

/** Coerce a wire value to number when it is one, else keep the raw value. */
function asNumber(value: unknown): number {
  return typeof value === 'number' ? value : Number(value);
}

/** Map one SSE `data:` payload to a typed event. Unknown events are skipped. */
function parseSseEvent(event: string, data: string): TraverseStreamEvent | null {
  const payload = safeJsonParseLike(data);
  switch (event) {
    case 'node':
      return {
        type: 'node',
        id: (payload.id as string | number) ?? 0,
        depth: asNumber(payload.depth),
        path: Array.isArray(payload.path)
          ? (payload.path as Array<string | number>)
          : [],
      };
    case 'stats':
      return {
        type: 'stats',
        nodesVisited: asNumber(payload.nodes_visited),
        elapsedMs: asNumber(payload.elapsed_ms),
      };
    case 'done':
      return {
        type: 'done',
        totalNodes: asNumber(payload.total_nodes),
        maxDepthReached: asNumber(payload.max_depth_reached),
        elapsedMs: asNumber(payload.elapsed_ms),
      };
    case 'error':
      return { type: 'error', message: String(payload.error ?? data) };
    default:
      return null;
  }
}

/** Tolerant JSON parse that returns the raw string's fields or a stub. */
function safeJsonParseLike(data: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(data);
    if (parsed && typeof parsed === 'object') {
      return parsed as Record<string, unknown>;
    }
  } catch {
    // fall through
  }
  return {};
}

/**
 * Open the SSE traversal stream and yield its events as they arrive.
 *
 * The HTTP status is checked first: a 4xx/5xx answers a JSON error body and
 * throws the typed error (same mapping as the other raw-stream endpoints).
 * An `error` SSE event (the server sends it alone when the traversal worker
 * fails) is yielded as data, not thrown.
 */
export async function* streamTraverse(
  transport: SseTransport,
  collection: string,
  request: StreamTraverseRequest
): AsyncIterable<TraverseStreamEvent> {
  const url =
    `${transport.baseUrl}${collectionPath(collection)}/graph/traverse/stream` +
    `?${streamTraverseQuery(request)}`;
  const headers: Record<string, string> = { Accept: 'text/event-stream' };
  if (transport.apiKey) {
    headers['Authorization'] = `Bearer ${transport.apiKey}`;
  }

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), transport.timeout);
  let response: Response;
  try {
    response = await fetch(url, {
      method: 'GET',
      headers,
      signal: controller.signal,
    });
  } catch (error) {
    clearTimeout(timeoutId);
    if (error instanceof Error && error.name === 'AbortError') {
      throw new ConnectionError('Request timeout');
    }
    throw new ConnectionError(
      `Stream traverse failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
      error instanceof Error ? error : undefined
    );
  }

  try {
    if (!response.ok || !response.body) {
      const data = await safeJsonParse(response);
      const ep = extractErrorPayload(data);
      throw new VelesDBError(
        ep.message ?? `HTTP ${response.status}`,
        ep.code ?? mapStatusToErrorCode(response.status)
      );
    }

    // SSE framing: events separated by a blank line; each event has
    // `event: <name>` and one or more `data: <line>` fields.
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let currentEvent = 'message';
    let currentData: string[] = [];

    const flush = (): TraverseStreamEvent | null => {
      const event = currentEvent;
      const data = currentData.join('\n');
      currentEvent = 'message';
      currentData = [];
      if (!data) return null;
      return parseSseEvent(event, data);
    };

    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let sep: number;
      while ((sep = buffer.indexOf('\n\n')) !== -1) {
        const rawEvent = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        for (const line of rawEvent.split('\n')) {
          if (line.startsWith('event:')) {
            currentEvent = line.slice(6).trim();
          } else if (line.startsWith('data:')) {
            currentData.push(line.slice(5).trimStart());
          }
          // comment lines (`:`) and other fields are ignored
        }
        const parsed = flush();
        if (parsed) yield parsed;
      }
    }
    buffer += decoder.decode();
    if (buffer.trim()) {
      for (const line of buffer.split('\n')) {
        if (line.startsWith('event:')) currentEvent = line.slice(6).trim();
        else if (line.startsWith('data:')) currentData.push(line.slice(5).trimStart());
      }
      const parsed = flush();
      if (parsed) yield parsed;
    }
  } finally {
    clearTimeout(timeoutId);
  }
}
