/**
 * The memory wedge's FIRST binding catch: the dynamic
 * `import('@wiscale/velesdb-wasm')` itself failing (#2282).
 *
 * A file of its own because the only way to make that import fail is a
 * module factory that throws, and `vi.mock` applies to a whole module graph:
 * `memory.test.ts` needs the binding to resolve.
 *
 * **What this pins, honestly.** vitest replaces whatever a mock factory
 * throws with an error of its own, so this test cannot choose the reason's
 * text or its type — a bare string here would not survive to the SDK. What
 * it does pin is the mechanism the two catches below it share: the loader's
 * reason reaches the `ConnectionError`'s MESSAGE. `ConnectionError`'s
 * `cause` slot takes only an `Error`, so a reason left out of the message is
 * lost for every non-`Error` the loader can reject with. Dropping
 * `describeWasmThrow` from that catch leaves the bare prefix and fails here.
 */
import { describe, it, expect, vi } from 'vitest';
import { MemoryService } from '../src/memory';
import { ConnectionError } from '../src/types';

vi.mock('@wiscale/velesdb-wasm', () => {
  throw new Error('the resolved build cannot be loaded');
});

vi.mock('../src/backends/wasm-node-loader', () => ({
  isNodeRuntime: () => false,
  loadWasmBytesNode: async () => new Uint8Array(),
}));

describe('MemoryService.init() — the wasm module cannot be loaded at all (#2282)', () => {
  it("rejects with ConnectionError carrying the loader's reason in the message", async () => {
    const memory = new MemoryService();
    const outcome = await memory.init().then(
      () => null,
      (error: unknown) => error
    );

    expect(outcome).toBeInstanceOf(ConnectionError);
    const message = (outcome as ConnectionError).message;
    // Not the bare prefix: a reason follows the colon.
    expect(message).toMatch(/^Failed to load @wiscale\/velesdb-wasm: \S/);
    expect(message).not.toBe('Failed to load @wiscale/velesdb-wasm');
    expect(memory.isInitialized()).toBe(false);
  });
});
