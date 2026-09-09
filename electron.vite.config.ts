import { defineConfig, loadEnv } from 'electron-vite';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), 'MAIN_VITE_');
  if (!env.MAIN_VITE_SPOTIFY_CLIENT_ID) {
    throw new Error(
      'MAIN_VITE_SPOTIFY_CLIENT_ID is not set. Copy .env.example to .env.local and paste the client id of your Spotify developer app (README, "Quick start").',
    );
  }
  return {
    main: {},
    preload: {
      // A sandboxed preload must be CommonJS; with "type": "module" this lands as out/preload/index.cjs.
      build: { rollupOptions: { output: { format: 'cjs' } } },
    },
    renderer: {},
  };
});
