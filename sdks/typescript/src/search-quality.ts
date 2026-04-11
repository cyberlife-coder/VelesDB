/**
 * VelesDB SearchQuality → REST wire format
 *
 * Helper that converts the TypeScript `SearchQuality` type into the
 * `{ mode, ef_search }` fragment expected by `velesdb-server`'s
 * `SearchRequest` body. The server supports named presets
 * (`fast | balanced | accurate | perfect | autotune`) plus two
 * template-literal forms:
 * - `custom:<ef>`          — explicit HNSW `ef_search` override
 * - `adaptive:<min>:<max>` — recall-target adaptive loop
 *
 * The helper preserves the string verbatim and lets the server parse
 * it via `velesdb_core::api_types::mode_to_search_quality`. This
 * keeps the wire contract in one place (the Rust parser) so the TS
 * SDK does not duplicate the variant parsing logic.
 *
 * @packageDocumentation
 */

import type { SearchQuality } from './types';

/**
 * Fragment spliced into a `SearchRequest` body. Only `mode` is set
 * today — the Rust parser resolves `custom:<ef>` and
 * `adaptive:<min>:<max>` server-side and populates `ef_search` from
 * the template payload. Callers that need a raw `ef_search` override
 * should pass `SearchOptions.k` and use `'custom:<ef>'`.
 */
export interface SearchQualityWire {
  /** Search mode preset or template string (see module docs). */
  mode?: string;
}

/**
 * Convert a `SearchQuality` value into the REST wire fragment.
 *
 * Returns an empty object `{}` when the caller passes `undefined`,
 * so spreading the result into a request body is safe and produces
 * no `mode` key at all (leaving the server free to apply its
 * configured default quality).
 */
export function searchQualityToMode(
  quality: SearchQuality | undefined
): SearchQualityWire {
  if (quality === undefined) {
    return {};
  }
  return { mode: quality };
}
