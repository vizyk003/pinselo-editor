import { defineConfig } from 'vitest/config';

// Package-local config: these tests need the built wasm (`pnpm --filter @oc/engine build`), so they are not part
// of the root `pnpm test` include pattern (packages/*/src/**/*.test.ts).
export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    testTimeout: 120_000,
    hookTimeout: 120_000,
  },
});
