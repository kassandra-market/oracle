import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const rootDir = dirname(fileURLToPath(import.meta.url))

/**
 * Dev-only: under `VITE_E2E=1` (what `make dev` / dev-up.sh set), inject the
 * funded local-dev keypair `dev-seed.ts` wrote to `e2e/.wallet.json` as
 * `window.__E2E_WALLET_SECRET__` — the exact global the Playwright specs set via
 * `addInitScript`, and the one `E2eWalletProvider` reads. Without this,
 * interactive `make dev` (no Playwright) has no way to reach the funded wallet,
 * so the app falls back to the empty wallet-adapter modal ("You'll need a wallet
 * on Solana to continue"). Read per page-load, so it works even though the chain
 * seeder writes the file concurrently with the dev server starting. `serve`-only
 * (never runs in a production build) and gated on VITE_E2E; the injected secret
 * is a throwaway keypair funded only on the local surfpool chain.
 */
function injectE2eWallet(): Plugin {
  return {
    name: 'inject-e2e-wallet',
    apply: 'serve',
    transformIndexHtml() {
      if (process.env.VITE_E2E !== '1') return
      const walletFile = resolve(rootDir, 'e2e/.wallet.json')
      if (!existsSync(walletFile)) return
      const { secretKey } = JSON.parse(readFileSync(walletFile, 'utf8')) as {
        secretKey: number[]
      }
      if (!Array.isArray(secretKey) || secretKey.length === 0) return
      return [
        {
          tag: 'script',
          injectTo: 'head-prepend' as const,
          children: `window.__E2E_WALLET_SECRET__ = ${JSON.stringify(secretKey)};`,
        },
      ]
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [injectE2eWallet(), react(), tailwindcss()],
  // Both `@kassandra/sdk` and `@kassandra-market/sdk` (and the app) resolve
  // `@solana/web3.js` — dedupe to ONE copy so `Address`/`instanceof` checks pass
  // across the app + both SDKs.
  resolve: { dedupe: ['@solana/web3.js'] },
  // Dev: the market client talks to the indexer over same-origin `/api/*`; proxy
  // it to the local indexer (the oracle side uses a direct VITE_RPC_URL in dev).
  server: {
    proxy: {
      '/api': {
        target:
          process.env.INDEXER_URL ?? process.env.VITE_INDEXER_URL ?? 'http://127.0.0.1:3111',
        changeOrigin: true,
      },
    },
  },
  build: {
    rollupOptions: {
      output: {
        // Vendor chunking: split the heavy libs into separate, independently
        // cacheable chunks so the entry chunk stays small and the big deps
        // (wallet-adapter / web3.js / the SDK) cache across route navigations
        // and deploys. Purely a bundling detail — no runtime behavior change.
        manualChunks(id) {
          // The two workspace SDKs resolve to their dist/ (NOT node_modules), so
          // key on either the package name or the dist path.
          if (
            id.includes('@kassandra/sdk') ||
            id.includes('/sdk/dist/') ||
            id.includes('@kassandra-market/sdk') ||
            id.includes('/sdk-market/dist/')
          ) {
            return 'sdk'
          }
          // Solana: wallet-adapter, web3.js, and their low-level deps (ox,
          // @noble/*, etc.) — the bulk of the third-party weight.
          if (
            id.includes('@solana/') ||
            id.includes('node_modules/ox/') ||
            id.includes('node_modules/@noble/') ||
            id.includes('node_modules/@solana-mobile/') ||
            id.includes('node_modules/@wallet-standard/') ||
            id.includes('node_modules/jayson/') ||
            id.includes('node_modules/rpc-websockets/')
          ) {
            return 'solana'
          }
          // React runtime + router — small, stable, cache-friendly vendor chunk.
          if (
            id.includes('node_modules/react/') ||
            id.includes('node_modules/react-dom/') ||
            id.includes('node_modules/react-router-dom/') ||
            id.includes('node_modules/react-router/') ||
            id.includes('node_modules/scheduler/')
          ) {
            return 'react-vendor'
          }
        },
      },
    },
  },
})
