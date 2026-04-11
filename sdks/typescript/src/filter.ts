/**
 * VelesDB Filter DSL
 *
 * Typed mirror of `velesdb_core::filter::Condition` (20 operators).
 * Provides a fluent builder API (`f.*`) for ergonomic filter construction
 * and accepts raw `Record<string, unknown>` objects for backward compatibility
 * with pre-v1.13 code.
 *
 * @example Typed builder
 * ```typescript
 * import { f } from '@wiscale/velesdb-sdk';
 *
 * const filter = f.and([
 *   f.eq('category', 'tech'),
 *   f.gte('price', 100),
 *   f.not(f.isNull('author')),
 * ]);
 * const results = await db.search('docs', query, { filter });
 * ```
 *
 * @example Legacy JSON (backward-compat, no compile-time checking)
 * ```typescript
 * const filter = {
 *   condition: { type: 'eq', field: 'category', value: 'tech' }
 * };
 * const results = await db.search('docs', query, { filter });
 * ```
 *
 * @packageDocumentation
 */

// ============================================================================
// Core types — exact mirror of velesdb-core::filter::Condition
// ============================================================================

/** JSON value accepted by filter operators (mirror of `serde_json::Value`). */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

/**
 * Comparison operators used by `GeoDistance`.
 *
 * Mirrors `velesdb_core::velesql::ast::condition::CompareOp` which
 * serializes with default (PascalCase) serde representation.
 */
export type CompareOp = 'Eq' | 'NotEq' | 'Gt' | 'Gte' | 'Lt' | 'Lte';

/**
 * Discriminated union matching `velesdb_core::filter::Condition`.
 *
 * The wire format uses `{"type": "<snake_case_variant>", ...}` as produced
 * by `#[serde(tag = "type", rename_all = "snake_case")]` on the Rust enum.
 *
 * 20 variants total — any change must stay in lock-step with the Rust source.
 */
export type Condition =
  // Comparison (6)
  | { type: 'eq'; field: string; value: JsonValue }
  | { type: 'neq'; field: string; value: JsonValue }
  | { type: 'gt'; field: string; value: JsonValue }
  | { type: 'gte'; field: string; value: JsonValue }
  | { type: 'lt'; field: string; value: JsonValue }
  | { type: 'lte'; field: string; value: JsonValue }
  // Set / string / null (4)
  | { type: 'in'; field: string; values: JsonValue[] }
  | { type: 'contains'; field: string; value: string }
  | { type: 'is_null'; field: string }
  | { type: 'is_not_null'; field: string }
  // Logical (3)
  | { type: 'and'; conditions: Condition[] }
  | { type: 'or'; conditions: Condition[] }
  | { type: 'not'; condition: Condition }
  // SQL patterns (2)
  | { type: 'like'; field: string; pattern: string }
  | { type: 'ilike'; field: string; pattern: string }
  // Array (3)
  | { type: 'array_contains'; field: string; value: JsonValue }
  | { type: 'array_contains_any'; field: string; values: JsonValue[] }
  | { type: 'array_contains_all'; field: string; values: JsonValue[] }
  // Geo (2)
  | {
      type: 'geo_distance';
      field: string;
      lat: number;
      lng: number;
      operator: CompareOp;
      threshold: number;
    }
  | {
      type: 'geo_bbox';
      field: string;
      lat_min: number;
      lng_min: number;
      lat_max: number;
      lng_max: number;
    };

/**
 * A filter for metadata-based search refinement.
 *
 * Mirrors `velesdb_core::filter::Filter` which wraps a root `Condition`.
 */
export interface Filter {
  condition: Condition;
}

/**
 * Filter parameter type accepted by SDK methods.
 *
 * - `Filter`: typed filter produced by the `f.*` builders. **Recommended.**
 * - `Record<string, unknown>`: raw JSON object for backward compatibility
 *   with pre-v1.13 code. The payload is forwarded verbatim to the server.
 */
export type FilterInput = Filter | Record<string, unknown>;

// ============================================================================
// Runtime helpers
// ============================================================================

/**
 * Type guard narrowing a `FilterInput` to the typed `Filter` shape.
 *
 * Returns `true` only when the value has a `condition` property that is
 * itself a non-null object. Does NOT validate the inner condition shape
 * — use TypeScript's compile-time checking for that.
 */
export function isTypedFilter(input: FilterInput): input is Filter {
  if (typeof input !== 'object' || input === null) {
    return false;
  }
  if (!('condition' in input)) {
    return false;
  }
  const cond = (input as { condition: unknown }).condition;
  return typeof cond === 'object' && cond !== null;
}

/**
 * Normalizes a filter input to the wire format expected by velesdb-server.
 *
 * The SDK never rewrites filter payloads — it forwards them verbatim. This
 * helper exists to keep backend code agnostic of whether the caller passed
 * a typed `Filter` or a legacy `Record<string, unknown>`.
 *
 * Passing `undefined` returns `undefined`, signalling the server should
 * apply no filter.
 */
export function normalizeFilter(input: FilterInput): Record<string, unknown>;
export function normalizeFilter(input: undefined): undefined;
export function normalizeFilter(
  input: FilterInput | undefined
): Record<string, unknown> | undefined;
export function normalizeFilter(
  input: FilterInput | undefined
): Record<string, unknown> | undefined {
  if (input === undefined) {
    return undefined;
  }
  return input as Record<string, unknown>;
}

// ============================================================================
// Fluent builder — `f.*` produces typed Filter values
// ============================================================================

/**
 * Fluent filter builder.
 *
 * Each method returns a new `Filter` whose root `condition` matches the
 * wire format expected by `velesdb-server`. Builders do not mutate inputs:
 * arrays passed to `in`, `arrayContainsAny`, `arrayContainsAll`, `and`, `or`
 * are copied before being wrapped.
 */
export const f = {
  // --- Comparison -----------------------------------------------------------

  /** `field == value` */
  eq(field: string, value: JsonValue): Filter {
    return { condition: { type: 'eq', field, value } };
  },

  /** `field != value` */
  neq(field: string, value: JsonValue): Filter {
    return { condition: { type: 'neq', field, value } };
  },

  /** `field > value` */
  gt(field: string, value: JsonValue): Filter {
    return { condition: { type: 'gt', field, value } };
  },

  /** `field >= value` */
  gte(field: string, value: JsonValue): Filter {
    return { condition: { type: 'gte', field, value } };
  },

  /** `field < value` */
  lt(field: string, value: JsonValue): Filter {
    return { condition: { type: 'lt', field, value } };
  },

  /** `field <= value` */
  lte(field: string, value: JsonValue): Filter {
    return { condition: { type: 'lte', field, value } };
  },

  // --- Set / string / null --------------------------------------------------

  /** `field IN (values...)` — the values list is copied. */
  in(field: string, values: JsonValue[]): Filter {
    return { condition: { type: 'in', field, values: [...values] } };
  },

  /** Substring containment: `field LIKE '%value%'` (case-sensitive). */
  contains(field: string, value: string): Filter {
    return { condition: { type: 'contains', field, value } };
  },

  /** `field IS NULL` */
  isNull(field: string): Filter {
    return { condition: { type: 'is_null', field } };
  },

  /** `field IS NOT NULL` */
  isNotNull(field: string): Filter {
    return { condition: { type: 'is_not_null', field } };
  },

  // --- SQL patterns ---------------------------------------------------------

  /** SQL LIKE pattern matching (case-sensitive). Supports `%` and `_`. */
  like(field: string, pattern: string): Filter {
    return { condition: { type: 'like', field, pattern } };
  },

  /** SQL ILIKE pattern matching (case-insensitive). */
  ilike(field: string, pattern: string): Filter {
    return { condition: { type: 'ilike', field, pattern } };
  },

  // --- Array ----------------------------------------------------------------

  /** `value IN field` (field must be an array). */
  arrayContains(field: string, value: JsonValue): Filter {
    return { condition: { type: 'array_contains', field, value } };
  },

  /** At least one of `values` is present in the array field. */
  arrayContainsAny(field: string, values: JsonValue[]): Filter {
    return { condition: { type: 'array_contains_any', field, values: [...values] } };
  },

  /** Every value in `values` is present in the array field. */
  arrayContainsAll(field: string, values: JsonValue[]): Filter {
    return { condition: { type: 'array_contains_all', field, values: [...values] } };
  },

  // --- Geo ------------------------------------------------------------------

  /** Haversine distance comparison: `distance(field, (lat, lng)) <op> threshold`. */
  geoDistance(
    field: string,
    lat: number,
    lng: number,
    operator: CompareOp,
    threshold: number
  ): Filter {
    return {
      condition: { type: 'geo_distance', field, lat, lng, operator, threshold },
    };
  },

  /** Bounding-box containment: point field falls inside `[lat_min, lat_max] x [lng_min, lng_max]`. */
  geoBbox(
    field: string,
    bounds: { lat_min: number; lng_min: number; lat_max: number; lng_max: number }
  ): Filter {
    return {
      condition: {
        type: 'geo_bbox',
        field,
        lat_min: bounds.lat_min,
        lng_min: bounds.lng_min,
        lat_max: bounds.lat_max,
        lng_max: bounds.lng_max,
      },
    };
  },

  // --- Logical --------------------------------------------------------------

  /** Logical AND — the filters list is copied and flattened to root conditions. */
  and(filters: Filter[]): Filter {
    return {
      condition: {
        type: 'and',
        conditions: filters.map((item) => item.condition),
      },
    };
  },

  /** Logical OR — the filters list is copied and flattened to root conditions. */
  or(filters: Filter[]): Filter {
    return {
      condition: {
        type: 'or',
        conditions: filters.map((item) => item.condition),
      },
    };
  },

  /** Logical NOT — wraps a single filter. */
  not(filter: Filter): Filter {
    return { condition: { type: 'not', condition: filter.condition } };
  },
} as const;
