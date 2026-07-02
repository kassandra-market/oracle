import { configDefaults, defineConfig } from "vitest/config";

/**
 * The default `pnpm --filter app test` is fast + offline (the mock-Connection
 * unit tests over the pure data layer). The gated surfpool integration test
 * (`test/*.e2e.test.ts`) spawns a real validator + deploys the program and is
 * OPT-IN: only included when `KASSANDRA_E2E=1`. This keeps the default suite
 * green/offline and never spawns surfpool.
 */
const e2e = process.env.KASSANDRA_E2E === "1";

export default defineConfig({
  // Collapse the two workspace copies of web3.js to ONE instance, so a Keypair's
  // Address created here is recognized by the SDK's `new Address(...)` guard (the
  // branded-type check throws across duplicate module instances otherwise).
  resolve: { dedupe: ["@solana/web3.js"] },
  test: {
    include: ["test/**/*.test.ts", "test/**/*.test.tsx"],
    exclude: e2e
      ? configDefaults.exclude
      : [...configDefaults.exclude, "test/**/*.e2e.test.ts"],
    // The surfpool suite owns a single validator on a fixed port — never run in parallel.
    ...(e2e ? { fileParallelism: false } : {}),
  },
});
