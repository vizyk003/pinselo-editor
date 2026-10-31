import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const API = process.env.OC_API_PORT ?? '4000';
const COLLAB = process.env.OC_COLLAB_PORT ?? '4100';

export default defineConfig({
  plugins: [react()],
  server: {
    // Ports are overridable so a second checkout (git worktree) can run its own stack alongside.
    port: Number(process.env.OC_WEB_PORT ?? 3000),
    strictPort: true,
    proxy: {
      '/api': `http://localhost:${API}`,
      '/site': `http://localhost:${API}`,
      '/v1': `http://localhost:${API}`,
      // AI connector (MCP) and its OAuth discovery documents.
      '/mcp': `http://localhost:${API}`,
      '/.well-known/oauth-': `http://localhost:${API}`,
      '/collab': { target: `ws://localhost:${COLLAB}`, ws: true, rewrite: (p) => p.replace(/^\/collab/, '') },
    },
  },
  // @oc/engine (the Rust core: editor, text, renderer) loads its .wasm with `new URL(…, import.meta.url)`: Vite serves
  // it in dev and emits it as a hashed asset in the build. It must not be pre-bundled (that would break the URL), and
  // its top-level await needs the es2022 target below.
  optimizeDeps: { exclude: ['@oc/engine'] },
  build: { target: 'es2022', sourcemap: true },
});
