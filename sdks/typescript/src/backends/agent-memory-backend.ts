/**
 * Agent Memory Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * These functions implement the three memory types:
 * - Semantic (vector-backed facts)
 * - Episodic (temporal events)
 * - Procedural (learned patterns)
 */

import type {
  SearchResult,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
  EpisodicRecord,
} from '../types';
import type { BaseTransport, TransportResponse } from './shared';
import { throwOnError, collectionPath } from './shared';
import { parseRestPointId } from './crud-backend';

/** Minimal transport interface for agent memory operations. */
export interface AgentMemoryTransport extends BaseTransport {
  searchVectors(
    collection: string,
    embedding: number[],
    k: number,
    filter: Record<string, string>
  ): Promise<SearchResult[]>;
}

// ---------------------------------------------------------------------------
// Unique ID generator
// ---------------------------------------------------------------------------

/**
 * Monotonic unique ID generator.
 * Combines millisecond timestamp with a sub-ms counter to avoid
 * collisions when multiple IDs are generated within the same millisecond.
 *
 * Uses a 1000-slot counter per millisecond. When the counter exceeds 999,
 * the timestamp is artificially advanced to the next millisecond to prevent
 * ID collisions with future real-time IDs in the same ms bucket.
 */
let _idCounter = 0;
let _lastTimestamp = 0;

export function generateUniqueId(): number {
  const now = Date.now();
  if (now <= _lastTimestamp) {
    _idCounter++;
    if (_idCounter >= 1000) {
      _lastTimestamp++;
      _idCounter = 0;
    }
  } else {
    _lastTimestamp = now;
    _idCounter = 0;
  }
  return _lastTimestamp * 1000 + _idCounter;
}

/** @internal Reset state — only for tests. */
export function _resetIdState(): void {
  _idCounter = 0;
  _lastTimestamp = 0;
}

// ---------------------------------------------------------------------------
// String <-> u64 id boundary (single source of truth)
// ---------------------------------------------------------------------------

/**
 * Render a u64 point id as the canonical string boundary value.
 *
 * Ids are returned as decimal strings so values above
 * `Number.MAX_SAFE_INTEGER` (2^53-1) survive the JavaScript boundary without
 * precision loss — matching the project's documented string-id convention for
 * u64 (see `/search/scroll`, graph node/edge ids).
 */
export function memoryIdToString(id: number): string {
  return String(id);
}

/**
 * Normalise a caller-provided point id into the numeric u64 the REST wire
 * accepts. Accepts a `string | number`; a string must be a decimal integer.
 *
 * Delegates range/integer validation to {@link parseRestPointId} so the rule
 * ("non-negative integer within the JS safe-integer range") lives in exactly
 * one place. The REST server deserialises point ids as JSON numbers, so ids
 * above `Number.MAX_SAFE_INTEGER` cannot be transmitted and are rejected here
 * rather than silently corrupted.
 */
export function resolveWireId(id: string | number): number {
  const numeric = typeof id === 'string' ? Number(id) : id;
  return parseRestPointId(numeric);
}

/** Current unix time in **seconds** (floor of epoch-millis / 1000). */
function nowUnixSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

// ---------------------------------------------------------------------------
// Agent memory operations
// ---------------------------------------------------------------------------

export async function storeSemanticFact(
  transport: AgentMemoryTransport,
  collection: string,
  entry: SemanticEntry
): Promise<void> {
  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/points`,
    {
      points: [{
        id: resolveWireId(entry.id),
        vector: entry.embedding,
        payload: {
          // Caller metadata is spread first so the reserved keys below
          // (`_memory_type`, `content`) always win and cannot be clobbered.
          ...entry.metadata,
          _memory_type: 'semantic',
          // `content` matches the core semantic store and the server/Python
          // payload field (BREAKING: was `text` before this change).
          content: entry.text,
        },
      }],
    }
  );

  throwOnError(response);
}

export async function searchSemanticMemory(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'semantic' });
}

export async function recordEpisodicEvent(
  transport: AgentMemoryTransport,
  collection: string,
  event: EpisodicEvent
): Promise<string> {
  const id = event.id !== undefined ? resolveWireId(event.id) : generateUniqueId();
  const timestamp = event.timestamp ?? nowUnixSeconds();

  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/points`,
    {
      points: [{
        id,
        vector: event.embedding,
        payload: {
          // Caller-supplied data/metadata is spread first so the reserved
          // keys below (`_memory_type`, `event_type`, `timestamp`) always
          // win and cannot be clobbered.
          ...event.data,
          ...event.metadata,
          _memory_type: 'episodic',
          event_type: event.eventType,
          // NUMERIC unix-seconds, mirroring the core episodic store so
          // recallRecent/recallOlderThan can range-filter on it.
          timestamp,
        },
      }],
    }
  );

  throwOnError(response);
  return memoryIdToString(id);
}

export async function recallEpisodicEvents(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'episodic' });
}

export async function storeProceduralPattern(
  transport: AgentMemoryTransport,
  collection: string,
  pattern: ProceduralPattern
): Promise<string> {
  const id = pattern.id !== undefined ? resolveWireId(pattern.id) : generateUniqueId();

  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/points`,
    {
      points: [{
        id,
        vector: pattern.embedding,
        payload: {
          // Caller metadata is spread first so the reserved keys below
          // (`_memory_type`, `name`, `steps`) always win and cannot be
          // clobbered.
          ...pattern.metadata,
          _memory_type: 'procedural',
          name: pattern.name,
          steps: pattern.steps,
        },
      }],
    }
  );

  throwOnError(response);
  return memoryIdToString(id);
}

export async function matchProceduralPatterns(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'procedural' });
}

// ---------------------------------------------------------------------------
// Temporal recall (mirrors core EpisodicMemory::recent / older_than)
// ---------------------------------------------------------------------------

/** Raw scroll page shape (snake_case `next_cursor`). */
interface ScrollPage {
  points: Array<{ id: string | number; payload?: Record<string, unknown> }>;
  next_cursor: string | number | null;
}

/**
 * Map a scrolled point to an {@link EpisodicRecord} when it is an episodic
 * event carrying a numeric timestamp; otherwise `undefined` (filtered out).
 */
function toEpisodicRecord(point: ScrollPage['points'][number]): EpisodicRecord | undefined {
  const payload = point.payload ?? {};
  if (payload._memory_type !== 'episodic') return undefined;
  if (typeof payload.timestamp !== 'number') return undefined;
  return { id: String(point.id), timestamp: payload.timestamp, payload };
}

/** Scroll every episodic event in the collection into memory. */
async function scrollEpisodicRecords(
  transport: AgentMemoryTransport,
  collection: string
): Promise<EpisodicRecord[]> {
  const records: EpisodicRecord[] = [];
  let cursor: string | number | null = null;
  do {
    const response: TransportResponse<ScrollPage> = await transport.requestJson<ScrollPage>(
      'POST',
      `${collectionPath(collection)}/points/scroll`,
      { cursor: cursor ?? undefined, filter: { _memory_type: 'episodic' } }
    );
    throwOnError(response);
    const page: ScrollPage = response.data!;
    for (const point of page.points) {
      const record = toEpisodicRecord(point);
      if (record !== undefined) records.push(record);
    }
    cursor = page.next_cursor;
  } while (cursor !== null && cursor !== undefined);
  return records;
}

/**
 * Recall episodic events most-recent-first, optionally bounded below by
 * `since` (inclusive). Mirrors `EpisodicMemory::recent(since_timestamp)`.
 */
export async function recallRecentEvents(
  transport: AgentMemoryTransport,
  collection: string,
  since?: number
): Promise<EpisodicRecord[]> {
  const records = await scrollEpisodicRecords(transport, collection);
  return records
    .filter((r) => since === undefined || r.timestamp >= since)
    .sort((a, b) => b.timestamp - a.timestamp);
}

/**
 * Recall episodic events strictly older than `before`, most-recent-first.
 * Mirrors `EpisodicMemory::older_than(timestamp)`.
 */
export async function recallOlderThanEvents(
  transport: AgentMemoryTransport,
  collection: string,
  before: number
): Promise<EpisodicRecord[]> {
  const records = await scrollEpisodicRecords(transport, collection);
  return records
    .filter((r) => r.timestamp < before)
    .sort((a, b) => b.timestamp - a.timestamp);
}
