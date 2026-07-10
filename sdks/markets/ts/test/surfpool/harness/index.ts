/**
 * `surfpool/harness` folder module — re-exports the same public surface the
 * original single-file `surfpool/harness.ts` exposed. Grouped by concern:
 *   - `config.ts`   — paths, constants, binary discovery, HarnessOptions
 *   - `harness.ts`  — the `MarketSurfpoolHarness` class
 *   - `encoding.ts` — `toHex` + `splTransfer`
 */

// Re-export the shared SPL / oracle byte-layout fabrication (used by the harness
// helpers, and by the surfpool lifecycle e2e via this module).
export { mintBytes, oracleBytes, tokenAccountAmount, tokenAccountBytes } from "../../spl-layout.js";

// Re-export the web3.js primitives from THIS package's `@solana/web3.js` copy, so
// consumers (e.g. the app Playwright e2e) build Keypairs/Addresses in the SAME
// realm the harness + SDK use. pnpm can install a second physically-distinct
// web3.js copy for a differently-peered consumer, and cross-realm `Address`
// values fail the SDK's `instanceof Address` checks ("Invalid public key input").
export { Address, Keypair } from "@solana/web3.js";

export {
  type HarnessOptions,
  KASSANDRA_PROGRAM_ID,
  SO_PATH,
  surfpoolBinary,
  surfpoolReady,
} from "./config.js";
export { MarketSurfpoolHarness } from "./harness.js";
export { splTransfer, toHex } from "./encoding.js";
