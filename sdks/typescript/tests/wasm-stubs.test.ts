/**
 * WASM Stubs Tests (Sprint 2 Wave 4 — #23 F-BACK-001)
 *
 * Verifies that the WASM backend index-management stubs throw a
 * "not supported" error instead of returning misleading empty
 * values (`[]` / `false`) that made the capability boundary
 * invisible in pre-v1.13 callers.
 */

import { describe, it, expect } from 'vitest';
import {
  wasmCreateIndex,
  wasmListIndexes,
  wasmHasIndex,
  wasmDropIndex,
} from '../src/backends/wasm-stubs';

describe('WASM index management stubs — F-BACK-001', () => {
  it('wasmCreateIndex throws "not supported"', async () => {
    await expect(
      wasmCreateIndex('docs', { label: 'Person', property: 'email' })
    ).rejects.toThrow(/not supported/i);
  });

  it('wasmListIndexes throws instead of returning []', async () => {
    // Pre-v1.13 returned `[]` which callers interpreted as "no
    // indexes exist on this collection"; that hid the real
    // capability boundary. We now throw upfront.
    await expect(wasmListIndexes('docs')).rejects.toThrow(/not supported/i);
  });

  it('wasmHasIndex throws instead of returning false', async () => {
    await expect(
      wasmHasIndex('docs', 'Person', 'email')
    ).rejects.toThrow(/not supported/i);
  });

  it('wasmDropIndex throws instead of returning false', async () => {
    await expect(
      wasmDropIndex('docs', 'Person', 'email')
    ).rejects.toThrow(/not supported/i);
  });

  it('every stub error message mentions REST backend as the workaround', async () => {
    const errors: unknown[] = [];
    for (const op of [
      () => wasmCreateIndex('docs', { label: 'P', property: 'e' }),
      () => wasmListIndexes('docs'),
      () => wasmHasIndex('docs', 'P', 'e'),
      () => wasmDropIndex('docs', 'P', 'e'),
    ]) {
      try {
        await op();
      } catch (e) {
        errors.push(e);
      }
    }
    expect(errors).toHaveLength(4);
    for (const e of errors) {
      const msg = e instanceof Error ? e.message : String(e);
      expect(msg).toMatch(/REST backend/i);
    }
  });
});
