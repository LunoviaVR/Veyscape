import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Tauri expects a fixed dev port; see src-tauri/tauri.conf.json `build.devUrl`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Only src/ and shared/ should trigger reloads of the shell during development.
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/docs/**', '**/tests/**', '**/assets/**'] } },
  build: { target: 'es2022', sourcemap: false, cssCodeSplit: false },
  css: { modules: { localsConvention: 'camelCaseOnly' } },
  test: { include: ['src/**/*.test.{ts,tsx}'] },
});
