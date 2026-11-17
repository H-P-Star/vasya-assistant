import { defineConfig } from 'vite';

export default defineConfig({
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    watch: { ignored: ['**/.tooling/**', '**/src-tauri/**'] },
  },
  build: { target: 'es2021' },
});
