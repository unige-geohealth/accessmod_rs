import { defineConfig } from "@playwright/test";

// End-to-end tests against a real `accessmod serve` on a fresh data
// directory, serving the built front end (`npm run build` first). The
// server takes uploads only (no local paths), as a country server does.
// One worker: the spec files share the server and its demo project.
const data = `${process.env.TMPDIR ?? "/tmp"}/accessmod-e2e-${process.pid}`;
// ACCESSMOD_E2E_BIN: a packaged binary (scripts/package.sh), which serves the
// client it ships; by default the workspace build and web/dist.
const bin = process.env.ACCESSMOD_E2E_BIN;
const serve = bin
  ? `"${bin}" serve --data ${data} --port 5198 --no-local-paths`
  : `cargo run -q -p accessmod-cli -- serve --data ${data} --port 5198 --no-local-paths --web-dir web/dist`;

export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  workers: 1,
  use: { baseURL: "http://127.0.0.1:5198", viewport: { width: 1440, height: 1000 } },
  webServer: {
    // The Project Creator finds its sources in the cache: no network (e2e/fixtures/sources).
    command: `rm -rf ${data} && mkdir -p ${data}/cache && cp -R web/e2e/fixtures/sources ${data}/cache/sources && ${serve}`,
    cwd: "..",
    url: "http://127.0.0.1:5198/api/health",
    timeout: 300_000,
    reuseExistingServer: false,
    // A dummy key: the online basemaps are offered; their requests are answered by the tests.
    env: { MAPTILER_API_KEY: "e2e" },
  },
});
