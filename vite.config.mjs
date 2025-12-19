import { defineConfig } from 'vite';
import os from 'os';
import path from 'path';

export default defineConfig({
  // Fix EPERM/locking issues on network drives by moving cache to local temp
  cacheDir: path.join(os.tmpdir(), 'slither-neuroevo-vite-cache'),
  root: '.',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    open: true
  }
});
