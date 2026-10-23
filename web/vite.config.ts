import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The API runs in `accessmod serve` (default port 5099); the dev server
// proxies to it, events included.
const api = process.env.ACCESSMOD_API ?? "http://127.0.0.1:5099";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { "/api": { target: api, changeOrigin: false } },
  },
  build: { outDir: "dist", sourcemap: true, chunkSizeWarningLimit: 2000 },
  test: { include: ["src/**/*.test.ts"] },
});
