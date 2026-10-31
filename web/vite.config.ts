import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The API runs in `accessmod serve` (`npm run dev:server`, port
// ACCESSMOD_DEV_PORT, default 5099); the dev server proxies to it, events
// included. ACCESSMOD_API names another server.
const api = process.env.ACCESSMOD_API ?? `http://127.0.0.1:${process.env.ACCESSMOD_DEV_PORT ?? 5099}`;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // The design system (@accessmod/ui) is compiled from its sources: one React.
  resolve: { dedupe: ["react", "react-dom"] },
  server: {
    port: 5173,
    proxy: { "/api": { target: api, changeOrigin: false } },
  },
  build: { outDir: "dist", sourcemap: true, chunkSizeWarningLimit: 4000 },
  test: { include: ["src/**/*.test.ts"] },
});
