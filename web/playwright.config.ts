import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { defineConfig, devices } from "@playwright/test";

// The tests run a real deck: the release binary with the built web app embedded (`pnpm build`, then
// `cargo build --release -p quietdeck-server`), on a fresh data directory, signed in by passkey only.
const port = Number(process.env.QUIETDECK_E2E_PORT ?? 18990);
const deckBin = process.env.QUIETDECK_SERVER_BIN ?? path.resolve(import.meta.dirname, "../target/release/quietdeck-server");
const dataDir = process.env.QUIETDECK_E2E_DATA ?? mkdtempSync(path.join(tmpdir(), "quietdeck-e2e-"));
process.env.QUIETDECK_E2E_DATA = dataDir;
const baseURL = `http://localhost:${port}`;

export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: process.env.CI ? "github" : "list",
  use: { baseURL, trace: "retain-on-failure" },
  projects: [{ name: "desktop", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `"${deckBin}" serve`,
    url: `${baseURL}/healthz`,
    reuseExistingServer: false,
    env: {
      QUIETDECK_DATA_DIR: dataDir,
      QUIETDECK_PUBLIC_URL: baseURL,
      QUIETDECK_LISTEN: `127.0.0.1:${port}`,
      QUIETDECK_HOOKS_LISTEN: `127.0.0.1:${port + 1}`,
      RUST_LOG: "warn",
    },
  },
});
