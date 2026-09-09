import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
    env: { MAIN_VITE_SPOTIFY_CLIENT_ID: 'test-client-id' },
  },
});
