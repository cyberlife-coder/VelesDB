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

/** One SSE event name → how to build its typed payload from the `data:` JSON. */
type EventBuilder = (payload: Record<string, unknown>, data: string) => TraverseStreamEvent;

const EVENT_BUILDERS: Record<string, EventBuilder> = {
  node: (p) => ({
    type: 'node',
    id: (p.id as string | number) ?? 0,
    depth: asNumber(p.depth),
    path: Array.isArray(p.path) ? (p.path as Array<string | number>) : [],
  }),
  stats: (p) => ({
    type: 'stats',
    nodesVisited: asNumber(p.nodes_visited),
    elapsedMs: asNumber(p.elapsed_ms),
  }),
  done: (p) => ({
    type: 'done',
    totalNodes: asNumber(p.total_nodes),
    maxDepthReached: asNumber(p.max_depth_reached),
    elapsedMs: asNumber(p.elapsed_ms),
  }),
  error: (p, data) => ({ type: 'error', message: String(p.error ?? data) }),
};

/** Map one SSE `data:` payload to a typed event. Unknown events are skipped. */
function parseSseEvent(event: string, data: string): TraverseStreamEvent | null {
  const builder = EVENT_BUILDERS[event];
  return builder ? builder(safeJsonParseLike(data), data) : null;
}

/**
 * Incremental SSE framing: accumulates the `event:`/`data:` lines of one
 * event block, then turns the completed block into a typed event.
 */
class SseEventParser {
  private event = 'message';
  private data: string[] = [];

  pushLine(line: string): void {
    if (line.startsWith('event:')) {
      this.event = line.slice(6).trim();
    } else if (line.startsWith('data:')) {
      this.data.push(line.slice(5).trimStart());
    }
    // comment lines (`:`) and other fields are ignored
  }

  flush(): TraverseStreamEvent | null {
    const event = this.event;
    const data = this.data.join('\n');
    this.event = 'message';
    this.data = [];
    return data ? parseSseEvent(event, data) : null;
  }
}

/** Feed one raw event block (no blank line) through the parser, yielding its event. */
function* eventsInBlock(parser: SseEventParser, block: string): Generator<TraverseStreamEvent> {
  for (const line of block.split('\n')) {
    parser.pushLine(line);
  }
  const parsed = parser.flush();
  if (parsed) yield parsed;
}

function buildStreamUrl(
  transport: SseTransport,
  collection: string,
  request: StreamTraverseRequest
): string {
  return (
    `${transport.baseUrl}${collectionPath(collection)}/graph/traverse/stream` +
    `?${streamTraverseQuery(request)}`
  );
}

function streamHeaders(transport: SseTransport): Record<string, string> {
  const headers: Record<string, string> = { Accept: 'text/event-stream' };
  if (transport.apiKey) {
    headers['Authorization'] = `Bearer ${transport.apiKey}`;
  }
  return headers;
}

async function fetchStream(
  url: string,
  headers: Record<string, string>,
  signal: AbortSignal
): Promise<Response> {
  try {
    return await fetch(url, { method: 'GET', headers, signal });
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') {
      throw new ConnectionError('Request timeout');
    }
    throw new ConnectionError(
      `Stream traverse failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
      error instanceof Error ? error : undefined
    );
  }
}

/**
 * A 4xx/5xx answers a JSON error body, not a stream — throw the typed error
 * (same mapping as the other raw-stream endpoints).
 */
async function throwUnlessStreamOk(response: Response): Promise<void> {
  if (response.ok && response.body) return;
  const data = await safeJsonParse(response);
  const ep = extractErrorPayload(data);
  throw new VelesDBError(
    ep.message ?? `HTTP ${response.status}`,
    ep.code ?? mapStatusToErrorCode(response.status)
  );
}

/** Read the response body as SSE frames (events separated by a blank line) and yield them. */
async function* readStreamEvents(response: Response): AsyncIterable<TraverseStreamEvent> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  const parser = new SseEventParser();
  let buffer = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let sep: number;
    while ((sep = buffer.indexOf('\n\n')) !== -1) {
      const block = buffer.slice(0, sep);
      buffer = buffer.slice(sep + 2);
      yield* eventsInBlock(parser, block);
    }
  }
  buffer += decoder.decode();
  if (buffer.trim()) {
    yield* eventsInBlock(parser, buffer);
  }
}

/**
 * Open the SSE traversal stream and yield its events as they arrive.
 *
 * The HTTP status is checked first: a 4xx/5xx throws the typed error. An
 * `error` SSE event (the server sends it alone when the traversal worker
 * fails) is yielded as data, not thrown.
 */
export async function* streamTraverse(
  transport: SseTransport,
  collection: string,
  request: StreamTraverseRequest
): AsyncIterable<TraverseStreamEvent> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), transport.timeout);
  try {
    const response = await fetchStream(
      buildStreamUrl(transport, collection, request),
      streamHeaders(transport),
      controller.signal
    );
    await throwUnlessStreamOk(response);
    yield* readStreamEvents(response);
  } finally {
    clearTimeout(timeoutId);
  }
}
