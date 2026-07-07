import { defineConfig, devices } from '@playwright/test'

/**
 * Browser E2E for the CANDLE chart (subscription-driven price history).
 *
 * `globalSetup` boots surfpool (:8964, ws :8965), seeds an ACTIVE market with a
 * live pool, runs the real `kassandra-indexer` against surfpool + an ephemeral
 * Postgres with `SOLANA_WS_URL` set, fires swaps that move the price, and waits
 * until the `/candles` API reflects them. `webServer` starts a Vite dev server
 * (:5177) whose `/api/*` proxy points at the indexer, so the app's MarketDetail
 * chart renders from real indexed data.
 *
 * Run via `scripts/e2e-playwright-candles.sh` (needs the indexer binary built +
 * postgres available). Distinct ports so it never collides with the default
 * (:8899/:5173), forked (:8940/:5174), or indexer (:8960/:5175) projects.
 */
export default defineConfig({
  testDir: './e2e/candles',
  timeout: 180_000,
  expect: { timeout: 30_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  globalSetup: './e2e/candles/global-setup.ts',
  use: {
    baseURL: 'http://localhost:5177',
    headless: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'pnpm exec vite --port 5177 --strictPort',
    url: 'http://localhost:5177',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: {
      VITE_RPC_URL: 'http://127.0.0.1:8964',
      VITE_INDEXER_URL: 'http://127.0.0.1:3113',
      INDEXER_URL: 'http://127.0.0.1:3113',
      VITE_E2E: '1',
    },
  },
})
