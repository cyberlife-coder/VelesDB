import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html', 'lcov'],
      include: ['src/**/*.ts'],
      exclude: [
        'node_modules/**',
        'dist/**',
        '**/*.test.ts',
        'vitest.config.ts',

        // Pure type modules — no executable code, only interfaces and type aliases.
        // v8 reports 0% because no statements are instrumented at runtime.
        'src/types/**',
        'src/types.ts',
        'src/index.ts',
        'src/client/index.ts',
        'src/backends/wasm-types.ts',

        // TODO(US-S4-07): agent-memory.ts is the last module pending
        // dedicated tests — out of scope for #598 which targeted the
        // 12 backend + client modules.
        'src/agent-memory.ts',
      ],
      thresholds: {
        // Global floor — conservative baseline while the modules listed
        // above remain excluded. Once those are covered, raise these.
        lines: 80,
        functions: 80,
        branches: 75,
        statements: 80,

        // Per-file gates — strict for the four modules S4-07 targeted.
        // Note on streaming-backend: functions stays at 44% because v8
        // counts each anonymous .catch() / arrow callback as a function;
        // the public surface (trainPq, streamInsert, streamUpsertPoints)
        // is fully exercised and the line coverage is 100%.
        'src/backends/index-backend.ts': {
          lines: 95,
          functions: 100,
          branches: 85,
          statements: 95,
        },
        'src/backends/scroll-backend.ts': {
          lines: 95,
          functions: 100,
          branches: 85,
          statements: 95,
        },
        'src/backends/streaming-backend.ts': {
          lines: 90,
          functions: 40,
          branches: 80,
          statements: 90,
        },
        'src/backends/wasm-wave4-stubs.ts': {
          lines: 100,
          functions: 100,
          branches: 100,
          statements: 100,
        },
      },
    },
  },
});
