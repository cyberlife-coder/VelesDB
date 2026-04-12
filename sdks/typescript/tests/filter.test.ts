/**
 * Filter DSL Tests (Sprint 2 Wave 4 — #19 PROP-FILTER-UNTYPED)
 *
 * Verifies that the TypeScript Filter type is a faithful mirror of
 * velesdb-core::Filter (20 operators), that the fluent `f.*` builders
 * emit the wire format expected by the server, and that backward-compat
 * with raw `Record<string, unknown>` filters is preserved.
 */

import { describe, it, expect } from 'vitest';
import type { Filter, Condition, CompareOp, JsonValue } from '../src/filter';
import { f, isTypedFilter, normalizeFilter } from '../src/filter';

// ============================================================================
// Nominal: each builder produces the exact wire shape
// ============================================================================

describe('Filter builders — comparison operators', () => {
  it('eq produces {type:"eq", field, value}', () => {
    const filter = f.eq('category', 'tech');
    expect(filter).toEqual({
      condition: { type: 'eq', field: 'category', value: 'tech' },
    });
  });

  it('neq produces {type:"neq", ...}', () => {
    expect(f.neq('status', 'archived')).toEqual({
      condition: { type: 'neq', field: 'status', value: 'archived' },
    });
  });

  it('gt, gte, lt, lte produce matching wire shapes', () => {
    expect(f.gt('price', 100).condition).toMatchObject({ type: 'gt', field: 'price', value: 100 });
    expect(f.gte('price', 100).condition).toMatchObject({ type: 'gte', field: 'price', value: 100 });
    expect(f.lt('price', 100).condition).toMatchObject({ type: 'lt', field: 'price', value: 100 });
    expect(f.lte('price', 100).condition).toMatchObject({ type: 'lte', field: 'price', value: 100 });
  });

  it('accepts number, string, boolean, null, and nested JSON values', () => {
    expect(f.eq('count', 42).condition).toMatchObject({ value: 42 });
    expect(f.eq('active', true).condition).toMatchObject({ value: true });
    expect(f.eq('deleted_at', null).condition).toMatchObject({ value: null });
    const nested: JsonValue = { nested: { depth: 2 } };
    expect(f.eq('meta', nested).condition).toMatchObject({ value: nested });
  });
});

describe('Filter builders — set / string / null', () => {
  it('in produces {type:"in", field, values}', () => {
    expect(f.in('category', ['tech', 'sports']).condition).toEqual({
      type: 'in',
      field: 'category',
      values: ['tech', 'sports'],
    });
  });

  it('contains produces {type:"contains", field, value}', () => {
    expect(f.contains('title', 'velesdb').condition).toEqual({
      type: 'contains',
      field: 'title',
      value: 'velesdb',
    });
  });

  it('isNull / isNotNull produce the snake_case wire type', () => {
    expect(f.isNull('deleted_at').condition).toEqual({ type: 'is_null', field: 'deleted_at' });
    expect(f.isNotNull('author').condition).toEqual({ type: 'is_not_null', field: 'author' });
  });
});

describe('Filter builders — SQL patterns', () => {
  it('like produces {type:"like", field, pattern}', () => {
    expect(f.like('title', 'Intro to %').condition).toEqual({
      type: 'like',
      field: 'title',
      pattern: 'Intro to %',
    });
  });

  it('ilike produces {type:"ilike", field, pattern} (case-insensitive)', () => {
    expect(f.ilike('title', '%SQL%').condition).toEqual({
      type: 'ilike',
      field: 'title',
      pattern: '%SQL%',
    });
  });
});

describe('Filter builders — array operators', () => {
  it('arrayContains matches a single value inside an array field', () => {
    expect(f.arrayContains('tags', 'rust').condition).toEqual({
      type: 'array_contains',
      field: 'tags',
      value: 'rust',
    });
  });

  it('arrayContainsAny matches at least one value', () => {
    expect(f.arrayContainsAny('tags', ['rust', 'go']).condition).toEqual({
      type: 'array_contains_any',
      field: 'tags',
      values: ['rust', 'go'],
    });
  });

  it('arrayContainsAll matches every value', () => {
    expect(f.arrayContainsAll('tags', ['rust', 'go']).condition).toEqual({
      type: 'array_contains_all',
      field: 'tags',
      values: ['rust', 'go'],
    });
  });
});

describe('Filter builders — geo operators', () => {
  it('geoDistance carries CompareOp and threshold', () => {
    const cond = f.geoDistance('location', 48.8566, 2.3522, 'Lte', 5000).condition;
    expect(cond).toEqual({
      type: 'geo_distance',
      field: 'location',
      lat: 48.8566,
      lng: 2.3522,
      operator: 'Lte',
      threshold: 5000,
    });
  });

  it('geoBbox packs the four corners into snake_case fields', () => {
    const cond = f.geoBbox('location', {
      lat_min: 48.8,
      lng_min: 2.3,
      lat_max: 48.9,
      lng_max: 2.4,
    }).condition;
    expect(cond).toEqual({
      type: 'geo_bbox',
      field: 'location',
      lat_min: 48.8,
      lng_min: 2.3,
      lat_max: 48.9,
      lng_max: 2.4,
    });
  });
});

describe('Filter builders — logical combinators', () => {
  it('and flattens child conditions under type:"and"', () => {
    const combined = f.and([f.eq('a', 1), f.gt('b', 2)]);
    expect(combined.condition).toEqual({
      type: 'and',
      conditions: [
        { type: 'eq', field: 'a', value: 1 },
        { type: 'gt', field: 'b', value: 2 },
      ],
    });
  });

  it('or mirrors and but with type:"or"', () => {
    expect(f.or([f.eq('a', 1), f.eq('a', 2)]).condition).toEqual({
      type: 'or',
      conditions: [
        { type: 'eq', field: 'a', value: 1 },
        { type: 'eq', field: 'a', value: 2 },
      ],
    });
  });

  it('not wraps a single condition', () => {
    expect(f.not(f.eq('a', 1)).condition).toEqual({
      type: 'not',
      condition: { type: 'eq', field: 'a', value: 1 },
    });
  });

  it('and + or + not compose into a nested tree', () => {
    const complex = f.and([
      f.or([f.eq('category', 'tech'), f.eq('category', 'news')]),
      f.not(f.isNull('author')),
      f.gte('score', 0.5),
    ]);
    expect(complex.condition).toEqual({
      type: 'and',
      conditions: [
        {
          type: 'or',
          conditions: [
            { type: 'eq', field: 'category', value: 'tech' },
            { type: 'eq', field: 'category', value: 'news' },
          ],
        },
        { type: 'not', condition: { type: 'is_null', field: 'author' } },
        { type: 'gte', field: 'score', value: 0.5 },
      ],
    });
  });
});

// ============================================================================
// Edge cases: boundaries, empties, large structures
// ============================================================================

describe('Filter builders — edge cases', () => {
  it('and([]) produces an empty conditions array (server decides semantics)', () => {
    expect(f.and([]).condition).toEqual({ type: 'and', conditions: [] });
  });

  it('or([]) is a valid zero-arity disjunction', () => {
    expect(f.or([]).condition).toEqual({ type: 'or', conditions: [] });
  });

  it('in([]) with empty values list is valid', () => {
    expect(f.in('field', []).condition).toEqual({ type: 'in', field: 'field', values: [] });
  });

  it('handles very large IN lists without mutation', () => {
    const values = Array.from({ length: 1000 }, (_, i) => i);
    const filter = f.in('id', values);
    expect(filter.condition).toMatchObject({ type: 'in', field: 'id', values });
    // Filter must own its values — mutating the original must not leak.
    values.push(9999);
    expect((filter.condition as { values: number[] }).values).toHaveLength(1000);
  });

  it('handles deeply nested logical trees (depth 5)', () => {
    const deep = f.and([
      f.or([
        f.and([
          f.not(f.or([f.eq('a', 1), f.eq('b', 2)])),
          f.gt('c', 3),
        ]),
      ]),
    ]);
    expect(deep.condition.type).toBe('and');
  });
});

// ============================================================================
// Type discrimination: isTypedFilter + normalizeFilter
// ============================================================================

describe('isTypedFilter', () => {
  it('returns true for typed filters built via f.*', () => {
    expect(isTypedFilter(f.eq('a', 1))).toBe(true);
    expect(isTypedFilter(f.and([f.eq('a', 1)]))).toBe(true);
  });

  it('returns false for raw Record<string, unknown>', () => {
    expect(isTypedFilter({ field: 'a', op: '=', value: 1 })).toBe(false);
    expect(isTypedFilter({})).toBe(false);
  });

  it('returns false for malformed inputs (condition not an object)', () => {
    expect(isTypedFilter({ condition: 'not-an-object' } as unknown as Filter)).toBe(false);
    expect(isTypedFilter({ condition: null } as unknown as Filter)).toBe(false);
  });
});

describe('normalizeFilter', () => {
  it('passes typed filters through unchanged', () => {
    const typed = f.eq('a', 1);
    expect(normalizeFilter(typed)).toBe(typed);
  });

  it('passes legacy Record<string, unknown> through unchanged', () => {
    const legacy: Record<string, unknown> = { custom: 'schema' };
    expect(normalizeFilter(legacy)).toBe(legacy);
  });

  it('handles undefined by returning undefined (backend skips filter)', () => {
    expect(normalizeFilter(undefined)).toBeUndefined();
  });
});

// ============================================================================
// Negative: type safety contracts (verified at compile time + runtime)
// ============================================================================

describe('Filter negative / contract tests', () => {
  it('Condition discriminated union carries distinct type tags', () => {
    // This test exists so that if a variant is accidentally removed,
    // TypeScript will fail to compile and the test file will break.
    const variants: Condition['type'][] = [
      'eq', 'neq', 'gt', 'gte', 'lt', 'lte',
      'in', 'contains', 'is_null', 'is_not_null',
      'and', 'or', 'not',
      'like', 'ilike',
      'array_contains', 'array_contains_any', 'array_contains_all',
      'geo_distance', 'geo_bbox',
    ];
    expect(variants).toHaveLength(20);
  });

  it('CompareOp mirrors the exact 6 Rust variants', () => {
    const ops: CompareOp[] = ['Eq', 'NotEq', 'Gt', 'Gte', 'Lt', 'Lte'];
    expect(ops).toHaveLength(6);
  });

  it('serializes to JSON losslessly (roundtrip)', () => {
    const filter = f.and([f.eq('a', 1), f.in('b', [1, 2, 3])]);
    const serialized = JSON.stringify(filter);
    const parsed = JSON.parse(serialized) as Filter;
    expect(parsed).toEqual(filter);
  });
});
