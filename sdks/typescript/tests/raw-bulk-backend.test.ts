/**
 * Raw-bulk Backend Tests — upsertBatchRaw (binary wire format)
 *
 * Pins the VRB1 binary encoder used by the REST backend's `upsertBatchRaw`
 * against a known batch, and verifies the WASM backend throws a typed
 * "not supported" error.
 *
 * The wire format (little-endian) is:
 *   magic b"VRB1" (4) | count u32 (4) | dim u32 (4) | id_width u8 (1) |
 *   reserved (3) | ids [u64; count] | vectors [f32; count*dim]
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { encodeRawBulk, upsertBatchRaw, type RawBulkTransport } from '../src/backends/crud-backend';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError, ValidationError } from '../src/types';

const mockFetch = vi.fn();
global.fetch = mockFetch;

function buildRawTransport(overrides: Partial<RawBulkTransport> = {}): RawBulkTransport {
  return {
    baseUrl: 'http://localhost:8080',
    apiKey: 'test-key',
    timeout: 5000,
    ...overrides,
  };
}

describe('encodeRawBulk', () => {
  it('encodes a known batch to the exact bytes', () => {
    const ids = [10, 20];
    const vectors = [
      new Float32Array([1.0, 0.0]),
      new Float32Array([0.0, 1.0]),
    ];
    const bytes = encodeRawBulk(ids, vectors, 2);

    // Header: 16 bytes; ids: 2*8 = 16; vectors: 2*2*4 = 16 → total 48.
    expect(bytes.byteLength).toBe(48);

    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    // Magic "VRB1"
    expect(bytes[0]).toBe(0x56); // V
    expect(bytes[1]).toBe(0x52); // R
    expect(bytes[2]).toBe(0x42); // B
    expect(bytes[3]).toBe(0x31); // 1
    // count = 2 (LE u32)
    expect(view.getUint32(4, true)).toBe(2);
    // dim = 2 (LE u32)
    expect(view.getUint32(8, true)).toBe(2);
    // id_width = 8
    expect(bytes[12]).toBe(8);
    // reserved
    expect(bytes[13]).toBe(0);
    expect(bytes[14]).toBe(0);
    expect(bytes[15]).toBe(0);
    // ids
    expect(view.getBigUint64(16, true)).toBe(10n);
    expect(view.getBigUint64(24, true)).toBe(20n);
    // vectors
    expect(view.getFloat32(32, true)).toBeCloseTo(1.0);
    expect(view.getFloat32(36, true)).toBeCloseTo(0.0);
    expect(view.getFloat32(40, true)).toBeCloseTo(0.0);
    expect(view.getFloat32(44, true)).toBeCloseTo(1.0);
  });

  it('is deterministic for the same input', () => {
    const ids = [7, 42];
    const vectors = [new Float32Array([1, 2]), new Float32Array([3, 4])];
    const a = encodeRawBulk(ids, vectors, 2);
    const b = encodeRawBulk(ids, vectors, 2);
    expect(Array.from(a)).toEqual(Array.from(b));
  });

  it('accepts number[] vectors as well as Float32Array', () => {
    const bytes = encodeRawBulk([1], [[0.5, 0.25]], 2);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    expect(view.getFloat32(24, true)).toBeCloseTo(0.5);
    expect(view.getFloat32(28, true)).toBeCloseTo(0.25);
  });

  it('rejects a vector whose length differs from dim', () => {
    expect(() => encodeRawBulk([1], [new Float32Array([1, 2, 3])], 2)).toThrow(ValidationError);
  });

  it('rejects mismatched ids/vectors lengths', () => {
    expect(() => encodeRawBulk([1, 2], [new Float32Array([1, 2])], 2)).toThrow(ValidationError);
  });
});

describe('upsertBatchRaw (REST)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('POSTs octet-stream to /collections/{name}/points/raw and returns count', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'Points upserted', count: 2 }),
    });

    const transport = buildRawTransport();
    const docs = [
      { id: 10, vector: new Float32Array([1, 0]) },
      { id: 20, vector: new Float32Array([0, 1]) },
    ];

    const inserted = await upsertBatchRaw(transport, 'test-col', docs, 2);
    expect(inserted).toBe(2);

    expect(mockFetch).toHaveBeenCalledTimes(1);
    const [url, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:8080/collections/test-col/points/raw');
    expect(opts.method).toBe('POST');
    expect((opts.headers as Record<string, string>)['Content-Type']).toBe(
      'application/octet-stream'
    );
    expect((opts.headers as Record<string, string>)['Authorization']).toBe('Bearer test-key');
    // Body is the binary encoding (48 bytes for this batch).
    expect((opts.body as Uint8Array).byteLength).toBe(48);
  });

  it('throws a VelesDBError on a non-OK response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 400,
      json: () => Promise.resolve({ error: 'dimension mismatch', code: 'VELES-001' }),
    });
    const transport = buildRawTransport();
    const docs = [{ id: 1, vector: new Float32Array([1, 2]) }];
    await expect(upsertBatchRaw(transport, 'test-col', docs, 2)).rejects.toThrow(VelesDBError);
  });
});

describe('WasmBackend.upsertBatchRaw', () => {
  it('throws a not-supported error', async () => {
    const backend = new WasmBackend();
    await expect(
      backend.upsertBatchRaw('c', [{ id: 1, vector: new Float32Array([1, 2]) }])
    ).rejects.toThrow(VelesDBError);
  });
});
