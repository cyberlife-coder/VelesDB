/**
 * WASM Backend - Sprint 2 Wave 4 unsupported feature stubs
 *
 * These endpoints are server-only (REST) and are not available in the
 * WASM backend. Each stub throws `wasmNotSupported()`.
 * @packageDocumentation
 */

import type {
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  AggregateQueryOptions,
  AggregateResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  GraphEdge,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchRequest,
  GraphSearchResponse,
} from '../types';
import { wasmNotSupported } from './shared';

export function wasmRebuildIndex(_c: string): Promise<RebuildIndexResponse> {
  return Promise.resolve(wasmNotSupported('Index rebuild'));
}

export function wasmGetGuardrails(): Promise<GuardRailsConfigResponse> {
  return Promise.resolve(wasmNotSupported('Guardrails'));
}

export function wasmUpdateGuardrails(
  _r: GuardRailsUpdateRequest
): Promise<GuardRailsConfigResponse> {
  return Promise.resolve(wasmNotSupported('Guardrails'));
}

export function wasmAggregate(
  _q: string, _p?: Record<string, unknown>, _o?: AggregateQueryOptions
): Promise<AggregateResponse> {
  return Promise.resolve(wasmNotSupported('Aggregate queries'));
}

export function wasmMatchQuery(
  _c: string, _q: string, _p?: Record<string, unknown>, _o?: MatchQueryOptions
): Promise<MatchQueryResponse> {
  return Promise.resolve(wasmNotSupported('MATCH queries'));
}

export function wasmRemoveEdge(_c: string, _id: number): Promise<boolean> {
  return Promise.resolve(wasmNotSupported('Graph edge removal'));
}

export function wasmGetEdgeCount(_c: string): Promise<number> {
  return Promise.resolve(wasmNotSupported('Graph edge count'));
}

export function wasmListNodes(_c: string): Promise<ListNodesResponse> {
  return Promise.resolve(wasmNotSupported('Graph list nodes'));
}

export function wasmGetNodeEdges(
  _c: string, _id: number, _o?: GetNodeEdgesOptions
): Promise<GraphEdge[]> {
  return Promise.resolve(wasmNotSupported('Graph node edges'));
}

export function wasmGetNodePayload(
  _c: string, _id: number
): Promise<NodePayloadResponse> {
  return Promise.resolve(wasmNotSupported('Graph node payload (read)'));
}

export function wasmUpsertNodePayload(
  _c: string, _id: number, _p: Record<string, unknown>
): Promise<void> {
  return Promise.resolve(wasmNotSupported('Graph node payload (upsert)'));
}

export function wasmGraphSearch(
  _c: string, _r: GraphSearchRequest
): Promise<GraphSearchResponse> {
  return Promise.resolve(wasmNotSupported('Graph search'));
}
