import { defineConfig } from "@playwright/test";

// End-to-end tests against a real `accessmod serve` on a fresh data
// directory, serving the built front end (`npm run build` first).
const data = `${process.env.TMPDIR ?? "/tmp"}/accessmod-e2e-${process.pid}`;

export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  use: { baseURL: "http://127.0.0.1:5198", viewport: { width: 1400, height: 900 } },
  webServer: {
    command: `rm -rf ${data} && cargo run -q -p accessmod-cli -- serve --data ${data} --port 5198 --web-dir web/dist`,
    cwd: "..",
    url: "http://127.0.0.1:5198/api/health",
    timeout: 300_000,
    reuseExistingServer: false,
  },
});
