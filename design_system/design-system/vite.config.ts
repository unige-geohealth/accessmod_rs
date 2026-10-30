import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Internal imports use the package's subpath imports (`#ui/*` in
// package.json), so the sources resolve the same from the web client.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { port: 5180 },
});
