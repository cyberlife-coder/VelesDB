/**
 * Shared test helper for streaming-backend tests.
 *
 * Provides a `buildTransport()` factory that returns a stub
 * `StreamingTransport` with sensible defaults. Individual test files can
 * pass `overrides` to customize any field (e.g. `apiKey: undefined`).
 *
 * Extracted from the original `streaming-backend.test.ts` to avoid
 * duplicating the ~25-line factory across the 3 split test files
 * (streamUpsertPoints, trainPq, streamInsert).
 */

import { vi } from 'vitest';
import type { StreamingTransport } from '../../src/backends/streaming-backend';

export function buildTransport(
  overrides: Partial<StreamingTransport> = {}
): StreamingTransport {
  return {
    requestJson: vi.fn(),
    baseUrl: 'http://localhost:8080',
    apiKey: 'test-key',
    timeout: 5000,
    parseRestPointId: (id: string | number) => {
      if (typeof id === 'string') return Number(id);
      return id;
    },
    sparseVectorToRestFormat: (sv: Record<number, number>) => sv,
    mapStatusToErrorCode: (status: number) => {
      const map: Record<number, string> = {
        400: 'BAD_REQUEST',
        404: 'NOT_FOUND',
        500: 'INTERNAL_ERROR',
      };
      return map[status] ?? 'UNKNOWN_ERROR';
    },
    extractErrorPayload: (data: unknown) => {
      if (!data || typeof data !== 'object') return {};
      const d = data as Record<string, unknown>;
      return {
        code: typeof d.code === 'string' ? d.code : undefined,
        message:
          typeof d.message === 'string'
            ? d.message
            : typeof d.error === 'string'
              ? d.error
              : undefined,
      };
    },
    ...overrides,
  };
}
