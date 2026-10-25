import { defineConfig } from 'vitest/config';

export default defineConfig({test: {
  include: ['**/*.performance.test.ts'],
  setupFiles: ['src/test/vitest.setup.ts'],
  fileParallelism: false
}});
