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
import type { Filter } from '../filter';
import { f } from '../filter';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';
import { parseRestPointId } from './crud-backend';
import { scroll } from './scroll-backend';

/** Minimal transport interface for agent memory operations. */
export interface AgentMemoryTransport extends BaseTransport {
  searchVectors(
    collection: string,
    embedding: number[],
    k: number,
    filter: Filter
  ): Promise<SearchResult[]>;
}

// ---------------------------------------------------------------------------
// Wire filters — every agent-memory query selects through these, so each one
// is a core `Filter` (`{ condition: ... }`) and nothing else. Their exact JSON
// is pinned by `conformance/filter_wire_cases.json`, which velesdb-core's
// `tests/filter_wire_conformance.rs` parses with `velesdb_core::Filter`.
// ---------------------------------------------------------------------------

/** The value written to `_memory_type` by each store operation. */
type MemoryType = 'semantic' | 'episodic' | 'procedural';

/** Points written as `memoryType` (`_memory_type == memoryType`). */
function memoryTypeFilter(memoryType: MemoryType): Filter {
  return f.eq('_memory_type', memoryType);
}

/**
 * Episodic events inside an optional timestamp window: `since` is an
 * inclusive lower bound (the SDK's documented `recallRecent` contract; core's
 * `TemporalIndex::recent` excludes `since`), `before` an exclusive upper
 * bound, as in core's `EpisodicMemory::older_than`.
 */
function episodicWindowFilter(window: { since?: number; before?: number }): Filter {
  const bounds: Filter[] = [];
  if (window.since !== undefined) bounds.push(f.gte('timestamp', window.since));
  if (window.before !== undefined) bounds.push(f.lt('timestamp', window.before));
  const episodic = memoryTypeFilter('episodic');
  return bounds.length === 0 ? episodic : f.and([episodic, ...bounds]);
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
// String <-> u64 id boundary (inbound validation lives in `parseRestPointId`,
// the single gate shared with the CRUD backend and the client layer)
// ---------------------------------------------------------------------------

/**
 * Render a u64 point id as the canonical string boundary value.
 *
 * Ids are returned as decimal strings so values above
 * `Number.MAX_SAFE_INTEGER` (2^53-1) survive the JavaScript boundary without
 * precision loss — matching the project's documented string-id convention for
 * u64 (see `/search/scroll`, graph node/edge ids).
 */
export function memoryIdToString(id: number | string): string {
  return String(id);
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
        id: parseRestPointId(entry.id),
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
  return transport.searchVectors(collection, embedding, k, memoryTypeFilter('semantic'));
}

export async function recordEpisodicEvent(
  transport: AgentMemoryTransport,
  collection: string,
  event: EpisodicEvent
): Promise<string> {
  const id = event.id !== undefined ? parseRestPointId(event.id) : generateUniqueId();
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
  return transport.searchVectors(collection, embedding, k, memoryTypeFilter('episodic'));
}

export async function storeProceduralPattern(
  transport: AgentMemoryTransport,
  collection: string,
  pattern: ProceduralPattern
): Promise<string> {
  const id = pattern.id !== undefined ? parseRestPointId(pattern.id) : generateUniqueId();

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
  return transport.searchVectors(collection, embedding, k, memoryTypeFilter('procedural'));
}

// ---------------------------------------------------------------------------
// Temporal recall (mirrors core EpisodicMemory::recent / older_than)
// ---------------------------------------------------------------------------

/**
 * Map a scrolled point to an {@link EpisodicRecord} when it is an episodic
 * event carrying a numeric timestamp; otherwise `undefined` (filtered out).
 */
function toEpisodicRecord(point: {
  id: string | number;
  payload?: Record<string, unknown>;
}): EpisodicRecord | undefined {
  const payload = point.payload ?? {};
  if (payload._memory_type !== 'episodic') return undefined;
  if (typeof payload.timestamp !== 'number') return undefined;
  return { id: String(point.id), timestamp: payload.timestamp, payload };
}

/**
 * Scroll every episodic event the server selects with `filter` into memory,
 * sorted most-recent-first. The timestamp window is applied by core, not here.
 */
async function scrollEpisodicRecords(
  transport: AgentMemoryTransport,
  collection: string,
  filter: Filter
): Promise<EpisodicRecord[]> {
  const records: EpisodicRecord[] = [];
  let cursor: string | number | null = null;
  do {
    const page = await scroll(transport, collection, {
      cursor: cursor ?? undefined,
      filter,
    });
    for (const point of page.points) {
      const record = toEpisodicRecord(point);
      if (record !== undefined) records.push(record);
    }
    cursor = page.nextCursor;
  } while (cursor !== null && cursor !== undefined);
  return records.sort((a, b) => b.timestamp - a.timestamp);
}

/**
 * Recall episodic events most-recent-first, optionally bounded below by
 * `since` (inclusive). Counterpart of `EpisodicMemory::recent(since_timestamp)`,
 * whose `since` bound is exclusive in core.
 */
export async function recallRecentEvents(
  transport: AgentMemoryTransport,
  collection: string,
  since?: number
): Promise<EpisodicRecord[]> {
  return scrollEpisodicRecords(transport, collection, episodicWindowFilter({ since }));
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
  return scrollEpisodicRecords(transport, collection, episodicWindowFilter({ before }));
}
