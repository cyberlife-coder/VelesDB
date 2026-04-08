/**
 * Scroll Backend operations for VelesDB REST API.
 *
 * Implements cursor-based scroll pagination for iterating over collection points.
 */

import type { ScrollRequest, ScrollResponse } from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';

/** Minimal transport interface for scroll operations. */
export type ScrollTransport = BaseTransport;

/** Raw API response shape (snake_case). */
interface ScrollApiResponse {
  points: Array<{
    id: string | number;
    vector?: number[];
    payload?: Record<string, unknown>;
  }>;
  next_cursor: string | number | null;
}

/**
 * Scroll through collection points with cursor-based pagination.
 *
 * @param transport - Transport for HTTP requests
 * @param collection - Collection name
 * @param request - Optional scroll parameters (cursor, batchSize, filter)
 * @returns Scroll response with points and next cursor
 */
export async function scroll(
  transport: ScrollTransport,
  collection: string,
  request?: ScrollRequest
): Promise<ScrollResponse> {
  // Build request body, mapping camelCase → snake_case
  const body: Record<string, unknown> = {};
  if (request?.cursor !== undefined) {
    body.cursor = request.cursor;
  }
  if (request?.batchSize !== undefined) {
    body.batch_size = request.batchSize;
  }
  if (request?.filter !== undefined) {
    body.filter = request.filter;
  }

  const response = await transport.requestJson<ScrollApiResponse>(
    'POST',
    `${collectionPath(collection)}/points/scroll`,
    body
  );

  throwOnError(response, `Collection '${collection}'`);

  const data = response.data!;
  return {
    points: data.points,
    nextCursor: data.next_cursor,
  };
}
