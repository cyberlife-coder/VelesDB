/**
 * Knowledge Graph operations for REST backend (EPIC-016)
 */

import type {
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseResponse,
  DegreeResponse,
  StreamTraverseOptions,
  StreamTraverseCallbacks,
} from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';

export async function addEdge(
  client: HttpClient, collection: string, edge: AddEdgeRequest
): Promise<void> {
  client.ensureInitialized();

  const response = await client.request(
    'POST',
    `/collections/${encodeURIComponent(collection)}/graph/edges`,
    {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      label: edge.label,
      properties: edge.properties ?? {},
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function getEdges(
  client: HttpClient, collection: string, options: GetEdgesOptions
): Promise<GraphEdge[]> {
  client.ensureInitialized();

  const queryParams = `?label=${encodeURIComponent(options.label)}`;

  const response = await client.request<{ edges: GraphEdge[]; count: number }>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/graph/edges${queryParams}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.edges ?? [];
}

export async function traverseGraph(
  client: HttpClient, collection: string, request: TraverseRequest
): Promise<TraverseResponse> {
  client.ensureInitialized();

  const response = await client.request<{
    results: Array<{ target_id: number; depth: number; path: number[] }>;
    next_cursor: string | null;
    has_more: boolean;
    stats: { visited: number; depth_reached: number };
  }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/graph/traverse`,
    {
      source: request.source,
      strategy: request.strategy ?? 'bfs',
      max_depth: request.maxDepth ?? 3,
      limit: request.limit ?? 100,
      cursor: request.cursor,
      rel_types: request.relTypes ?? [],
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  const data = response.data;
  return {
    results: (data?.results ?? []).map(r => ({
      targetId: r.target_id,
      depth: r.depth,
      path: r.path,
    })),
    nextCursor: data?.next_cursor ?? undefined,
    hasMore: data?.has_more ?? false,
    stats: {
      visited: data?.stats?.visited ?? 0,
      depthReached: data?.stats?.depth_reached ?? 0,
    },
  };
}

export async function getNodeDegree(
  client: HttpClient, collection: string, nodeId: number
): Promise<DegreeResponse> {
  client.ensureInitialized();

  const response = await client.request<{ in_degree: number; out_degree: number }>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/graph/nodes/${nodeId}/degree`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return {
    inDegree: response.data?.in_degree ?? 0,
    outDegree: response.data?.out_degree ?? 0,
  };
}

/** Dispatch a parsed SSE event to the appropriate callback */
function dispatchSseEvent(
  eventType: string,
  data: Record<string, unknown>,
  callbacks: StreamTraverseCallbacks
): void {
  switch (eventType) {
    case 'node':
      callbacks.onNode({
        id: data.id as number,
        depth: data.depth as number,
        path: (data.path as number[]) ?? [],
      });
      break;
    case 'stats':
      callbacks.onStats?.({
        nodesVisited: data.nodes_visited as number,
        elapsedMs: data.elapsed_ms as number,
      });
      break;
    case 'done':
      callbacks.onDone?.({
        totalNodes: data.total_nodes as number,
        maxDepthReached: data.max_depth_reached as number,
        elapsedMs: data.elapsed_ms as number,
      });
      break;
    case 'error':
      callbacks.onError?.(
        new VelesDBError((data.error as string) ?? 'Unknown stream error', 'STREAM_ERROR')
      );
      break;
  }
}

/** Build the SSE URL with query parameters for streaming traversal */
function buildStreamUrl(baseUrl: string, collection: string, options: StreamTraverseOptions): string {
  const params = new URLSearchParams();
  params.set('start_node', String(options.source));
  params.set('algorithm', options.strategy ?? 'bfs');
  params.set('max_depth', String(options.maxDepth ?? 5));
  params.set('limit', String(options.limit ?? 1000));
  if (options.relTypes && options.relTypes.length > 0) {
    params.set('relationship_types', options.relTypes.join(','));
  }
  return `${baseUrl}/collections/${encodeURIComponent(collection)}/graph/traverse/stream?${params.toString()}`;
}

/** Mutable state for SSE line parser */
interface SseParserState {
  buffer: string;
  currentEventType: string;
}

/** Process a single SSE line and dispatch events via callbacks */
function processSseLine(line: string, state: SseParserState, callbacks: StreamTraverseCallbacks): void {
  if (line.startsWith('event:')) {
    state.currentEventType = line.slice(6).trim();
    return;
  }

  if (!line.startsWith('data:')) return;

  const dataStr = line.slice(5).trim();
  if (!dataStr) return;

  try {
    dispatchSseEvent(state.currentEventType, JSON.parse(dataStr), callbacks);
  } catch {
    callbacks.onError?.(new VelesDBError(`Failed to parse SSE data: ${dataStr}`, 'PARSE_ERROR'));
  }
  state.currentEventType = '';
}

/** Validate the SSE fetch response, throwing on HTTP errors */
function validateStreamResponse(response: Response, collection: string): void {
  if (response.ok) return;
  if (response.status === 404) {
    throw new NotFoundError(`Collection '${collection}'`);
  }
  throw new VelesDBError(`Stream request failed: HTTP ${response.status}`, 'STREAM_ERROR');
}

/**
 * Stream graph traversal results via Server-Sent Events.
 *
 * Connects to `GET /collections/{name}/graph/traverse/stream` and parses
 * SSE events: `node`, `stats`, `done`, `error`.
 */
export async function streamTraverseGraph(
  client: HttpClient,
  collection: string,
  options: StreamTraverseOptions,
  callbacks: StreamTraverseCallbacks
): Promise<void> {
  client.ensureInitialized();

  const url = buildStreamUrl(client.getBaseUrl(), collection, options);
  const headers: Record<string, string> = { 'Accept': 'text/event-stream', ...client.getHeaders() };
  const response = await fetch(url, { headers });

  validateStreamResponse(response, collection);

  if (!response.body) {
    throw new VelesDBError('No response body for SSE stream', 'STREAM_ERROR');
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const state: SseParserState = { buffer: '', currentEventType: '' };

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      state.buffer += decoder.decode(value, { stream: true });
      const lines = state.buffer.split('\n');
      state.buffer = lines.pop() ?? '';

      for (const line of lines) {
        processSseLine(line, state, callbacks);
      }
    }
  } finally {
    reader.releaseLock();
  }
}
