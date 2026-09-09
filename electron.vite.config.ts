import { defineConfig } from 'electron-vite';

export default defineConfig({
  main: {},
  preload: {
    // A sandboxed preload must be CommonJS; with "type": "module" this lands as out/preload/index.cjs.
    build: { rollupOptions: { output: { format: 'cjs' } } },
  },
  renderer: {},
});
