/**
 * MemoryService Unit Tests
 *
 * Tests the MemoryService class against a mocked `@wiscale/velesdb-wasm`
 * module, mirroring wasm-backend.test.ts's convention (a real-wasm smoke
 * run was used to verify the class against the actual compiled artifact
 * during development; this suite is the permanent, CI-safe one).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { MemoryService } from '../src/memory';
import { ConnectionError, NotFoundError, ValidationError } from '../src/types';

// Captures the most recently constructed mock instance so a test can
// override one of its methods for that specific instance. Overriding
// `MockWasmMemoryService.prototype.X` would NOT work here: each method is a
// class-field (`remember = vi.fn(...)`), which vitest/TS compiles into a
// per-instance own-property assigned in the constructor — an own-property
// always shadows a same-named prototype property, so a prototype patch
// applied after construction has no effect on an already-built instance.
let lastMockInstance: MockWasmMemoryService | null = null;

class MockWasmMemoryService {
  remember = vi.fn(() => '1');
  recall = vi.fn(() => [{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
  recallWhere = vi.fn(() => [{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
  recallFused = vi.fn(() => [{ id: '2', score: 0.0, content: 'EPIC-317' }]);
  relate = vi.fn(() => '5');
  forget = vi.fn();
  why = vi.fn(() => ({
    nodes: [{ id: '1', content: 'we chose parking_lot', hop: 0 }],
    edges: [],
  }));
  free = vi.fn();

  constructor(public dimension: number) {
    lastMockInstance = this;
  }
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  MemoryService: MockWasmMemoryService,
};

// Mock the dynamic import - must match the import path in memory.ts
vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

// Stub the Node-only loader so the unit suite does not touch the real
// filesystem (see wasm-backend.test.ts for the same rationale).
vi.mock('../src/backends/wasm-node-loader', () => ({
  isNodeRuntime: () => false,
  loadWasmBytesNode: vi.fn(() => Promise.resolve(new Uint8Array(0))),
}));

describe('MemoryService', () => {
  let memory: MemoryService;

  beforeEach(() => {
    vi.clearAllMocks();
    lastMockInstance = null;
    memory = new MemoryService({ dimension: 4 });
  });

  describe('lifecycle', () => {
    it('initializes successfully', async () => {
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
    });

    it('is idempotent', async () => {
      await memory.init();
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
      expect(mockWasmModule.default).toHaveBeenCalledTimes(1);
    });

    it('coalesces concurrent init() calls into one wasm-bindgen invocation', async () => {
      await Promise.all([memory.init(), memory.init(), memory.init()]);
      expect(mockWasmModule.default).toHaveBeenCalledTimes(1);
      expect(memory.isInitialized()).toBe(true);
    });

    it('supports re-init after close()', async () => {
      await memory.init();
      await memory.close();
      expect(memory.isInitialized()).toBe(false);
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
    });

    it('throws ConnectionError from any wedge method before init()', async () => {
      await expect(memory.recall('query')).rejects.toThrow(ConnectionError);
    });

    it('wraps a wasm-bindgen default() failure in ConnectionError', async () => {
      mockWasmModule.default.mockRejectedValueOnce(new Error('boom'));
      await expect(memory.init()).rejects.toThrow(ConnectionError);
      expect(memory.isInitialized()).toBe(false);
    });
  });

  describe('wedge operations', () => {
    beforeEach(async () => {
      await memory.init();
    });

    it('remember() passes fact/links/metadata/ttl through and returns the id', async () => {
      const id = await memory.remember('we chose parking_lot', {
        links: [{ target: '1', relation: 'decided_in' }],
        metadata: { project: 'veles' },
        ttlSeconds: 60,
      });
      expect(id).toBe('1');
      expect(lastMockInstance!.remember).toHaveBeenCalledWith(
        'we chose parking_lot',
        [{ target: '1', relation: 'decided_in' }],
        { project: 'veles' },
        60n
      );
    });

    it('remember() passes empty links and undefined metadata/ttl when not provided', async () => {
      await memory.remember('a fact');
      expect(lastMockInstance!.remember).toHaveBeenCalledWith('a fact', [], undefined, undefined);
    });

    it.each([1.5, -1, Number.NaN])(
      'remember() rejects ttlSeconds %p with ValidationError, not a raw RangeError',
      async (ttlSeconds) => {
        // Regression: BigInt(1.5) throws a codeless RangeError and a negative
        // value dies as an opaque wasm-bindgen u64 conversion — both escaped
        // the typed-error contract instead of surfacing as ValidationError.
        await expect(memory.remember('a fact', { ttlSeconds })).rejects.toBeInstanceOf(
          ValidationError
        );
        expect(lastMockInstance!.remember).not.toHaveBeenCalled();
      }
    );

    it('recall() returns the mocked recollections', async () => {
      const hits = await memory.recall('parking_lot', 5, { project: 'veles' });
      expect(hits).toEqual([{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
      expect(lastMockInstance!.recall).toHaveBeenCalledWith('parking_lot', 5, { project: 'veles' });
    });

    it('recallWhere() returns the mocked recollections', async () => {
      const hits = await memory.recallWhere('parking_lot', [
        { field: 'project', op: 'eq', value: 'veles' },
      ]);
      expect(hits).toEqual([{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
    });

    it('recallFused() returns the mocked recollections', async () => {
      const hits = await memory.recallFused('parking_lot', 3, undefined, { hops: 2 });
      expect(hits).toEqual([{ id: '2', score: 0.0, content: 'EPIC-317' }]);
      expect(lastMockInstance!.recallFused).toHaveBeenCalledWith('parking_lot', 3, undefined, {
        hops: 2,
      });
    });

    it('relate() returns the edge id', async () => {
      const edgeId = await memory.relate('1', '2', 'decided_in');
      expect(edgeId).toBe('5');
      expect(lastMockInstance!.relate).toHaveBeenCalledWith('1', '2', 'decided_in');
    });

    it('forget() resolves', async () => {
      await expect(memory.forget('1')).resolves.toBeUndefined();
    });

    it('why() returns the explanation subgraph', async () => {
      const explanation = await memory.why('parking_lot', 2);
      expect(explanation.nodes).toHaveLength(1);
      expect(explanation.edges).toEqual([]);
    });

    it('close() frees the underlying wasm instance', async () => {
      const inner = lastMockInstance!;
      await memory.close();
      expect(inner.free).toHaveBeenCalledTimes(1);
    });
  });

  describe('error translation', () => {
    beforeEach(async () => {
      await memory.init();
    });

    it('translates a NOT_FOUND wasm error into NotFoundError, preserving the original message', async () => {
      const err = new Error('memory 999 does not exist');
      (err as Error & { code: string }).code = 'NOT_FOUND';
      lastMockInstance!.relate.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.relate('999', '1', 'x')).rejects.toSatisfy((e: unknown) => {
        expect(e).toBeInstanceOf(NotFoundError);
        expect((e as Error).message).toBe('memory 999 does not exist');
        return true;
      });
    });

    it('translates an INVALID_INPUT wasm error into ValidationError', async () => {
      const err = new Error('fact text must not be empty');
      (err as Error & { code: string }).code = 'INVALID_INPUT';
      lastMockInstance!.remember.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.remember('')).rejects.toSatisfy((e: unknown) => {
        expect(e).toBeInstanceOf(ValidationError);
        expect((e as Error).message).toBe('fact text must not be empty');
        return true;
      });
    });

    it('rethrows an uncoded error unchanged', async () => {
      const err = new Error('unstructured failure');
      lastMockInstance!.forget.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.forget('1')).rejects.toBe(err);
    });

    it('wraps a thrown non-Error value in VelesDBError', async () => {
      lastMockInstance!.forget.mockImplementationOnce(() => {
        // eslint-disable-next-line @typescript-eslint/no-throw-literal
        throw 'a raw string throw';
      });

      await expect(memory.forget('1')).rejects.toThrow('a raw string throw');
    });
  });
});
